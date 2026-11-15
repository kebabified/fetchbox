//! Private local channel between the window, the browser helper and the background service.
//!
//! A Windows named pipe (never a network port) that only the current Windows user can open.
//! One request and one reply per connection, each a single line of JSON.

use std::io::{BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::win::PipeListener;

const MAX_MESSAGE: u64 = 8 * 1024 * 1024;

pub fn service_pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let user: String = user.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    format!("fetchbox-service-v2-{user}")
}

/// Send one request to the service. `None` means it is not running.
pub fn request(msg: &Value) -> Option<Value> {
    let path = format!(r"\\.\pipe\{}", service_pipe_name());
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut pipe = loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(f) => break f,
            // 231 = ERROR_PIPE_BUSY: the server is between instances, try again
            Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Err(_) => return None,
        }
    };
    let mut line = serde_json::to_vec(msg).ok()?;
    line.push(b'\n');
    pipe.write_all(&line).ok()?;
    let mut reply = String::new();
    BufReader::new(pipe.take(MAX_MESSAGE)).read_line(&mut reply).ok()?;
    serde_json::from_str(&reply).ok()
}

/// Serve requests forever on background threads. Errors with `AlreadyExists` if a service is already running.
pub fn serve<F>(handler: F) -> std::io::Result<()>
where
    F: Fn(Value) -> Value + Send + Sync + 'static,
{
    let mut listener = PipeListener::bind(&service_pipe_name())?;
    let handler = std::sync::Arc::new(handler);
    std::thread::spawn(move || loop {
        let Ok(conn) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let handler = handler.clone();
        std::thread::spawn(move || {
            let mut line = String::new();
            let mut reader = BufReader::new((&conn).take(MAX_MESSAGE));
            if reader.read_line(&mut line).is_err() {
                return;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) if msg.is_object() => handler(msg),
                _ => json!({"ok": false, "error": "bad request"}),
            };
            let mut out = serde_json::to_vec(&reply).unwrap_or_default();
            out.push(b'\n');
            let mut w = &conn;
            let _ = w.write_all(&out);
            let _ = conn.sync_all(); // wait until the client has read the reply before closing
        });
    });
    Ok(())
}
