//! The browser helper: Chrome-family browsers start this ("native messaging") when the Fetchbox
//! extension connects. Messages are a 4-byte length followed by JSON, on stdin/stdout.
//!
//!   check     is this link downloadable?  (answered here, offline)
//!   download  hand a link to the background service (started if needed)
//!   status / cancel / reveal / retry      follow background downloads
//!   open      show the Fetchbox window
//!   ping      health check

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::util::{log, valid_url};
use crate::{install, ipc, paths, service, sites, win};

const MAX_MESSAGE: usize = 8 * 1024 * 1024; // cookies for one site fit easily
const MAX_CHECK: usize = 200;

fn read_message(input: &mut impl Read) -> Option<Value> {
    let mut len = [0u8; 4];
    input.read_exact(&mut len).ok()?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return None; // not a browser, or a broken stream: stop
    }
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_message(out: &Mutex<std::io::Stdout>, msg: &Value) {
    let data = serde_json::to_vec(msg).unwrap_or_default();
    let mut out = out.lock().unwrap();
    let _ = out.write_all(&(data.len() as u32).to_le_bytes());
    let _ = out.write_all(&data);
    let _ = out.flush();
}

fn fail(msg: &str) -> Value {
    json!({"ok": false, "error": msg})
}

fn handle(msg: &Value, cache: &Mutex<HashMap<String, sites::Verdict>>) -> Value {
    let job = msg.get("job").cloned().unwrap_or(Value::Null);
    match msg.get("type").and_then(Value::as_str).unwrap_or("") {
        "ping" => json!({"ok": true, "version": crate::VERSION, "service": ipc::request(&json!({"cmd": "ping"})).is_some()}),
        "check" => {
            let mut results = serde_json::Map::new();
            for url in msg.get("urls").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(MAX_CHECK) {
                let cached = cache.lock().unwrap().get(url).cloned();
                let verdict = cached.unwrap_or_else(|| {
                    let v = sites::check(url);
                    let mut c = cache.lock().unwrap();
                    if c.len() > 5000 {
                        c.clear();
                    }
                    c.insert(url.to_string(), v.clone());
                    v
                });
                results.insert(url.to_string(), serde_json::to_value(verdict).unwrap_or(Value::Null));
            }
            json!({"ok": true, "results": results})
        }
        "download" => {
            let url = msg.get("url").and_then(Value::as_str).unwrap_or("");
            if !valid_url(url) {
                return fail("not a web link");
            }
            service::call(&json!({
                "cmd": "download", "url": url,
                "mode": msg.get("mode").cloned().unwrap_or(Value::Null),
                "playlist": msg.get("playlist").and_then(Value::as_bool).unwrap_or(false),
                "cookies": msg.get("cookies").cloned().unwrap_or(Value::Null),
            }))
        }
        // these never start the service: nothing running means nothing to show
        "status" => ipc::request(&json!({"cmd": "status"})).unwrap_or_else(|| json!({"ok": true, "jobs": []})),
        t @ ("cancel" | "reveal" | "retry") => ipc::request(&json!({"cmd": t, "job": job})).unwrap_or_else(|| fail("nothing is running")),
        "open" => {
            if win::focus_window(crate::APP_NAME) {
                return json!({"ok": true});
            }
            match win::spawn_detached(&paths::sibling_exe("fetchbox.exe"), &[]) {
                Ok(()) => json!({"ok": true}),
                Err(e) => fail(&format!("couldn't open Fetchbox: {e}")),
            }
        }
        other => fail(&format!("unknown request '{other}'")),
    }
}

/// Entry point when a browser starts us: `fetchbox-host.exe chrome-extension://<id>/ …`.
pub fn run(origin: &str) -> i32 {
    // The browser already enforces this through the helper's manifest; refuse anything else too.
    if !install::allowed_origins().iter().any(|o| o == origin) {
        log("host", &format!("refused: {origin} is not an allowed extension"));
        return 2;
    }
    log("host", &format!("started by {origin}"));
    sites::warm_up();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let cache = Arc::new(Mutex::new(HashMap::new()));
    let mut input = std::io::stdin().lock();
    while let Some(msg) = read_message(&mut input) {
        let (out, cache) = (out.clone(), cache.clone());
        std::thread::spawn(move || {
            let mut reply = handle(&msg, &cache);
            let kind = msg.get("type").and_then(Value::as_str).unwrap_or("?").to_string();
            if !matches!(kind.as_str(), "check" | "status") {
                log("host", &format!("{kind} -> ok={} {}", reply["ok"], reply.get("error").and_then(Value::as_str).unwrap_or("")));
            }
            if let Some(obj) = reply.as_object_mut() {
                obj.insert("id".into(), msg.get("id").cloned().unwrap_or(Value::Null)); // the extension's request number
            }
            write_message(&out, &reply);
        });
    }
    log("host", "browser closed the connection");
    0
}
