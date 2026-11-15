//! The background service: owns the download queue for both the window and the browser extension.
//!
//! Started on demand, it exits by itself after a few idle minutes (stopping its local cobalt too).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cobalt::LocalCobalt;
use crate::engine::{execute, Job, Slots, MAX_PARALLEL};
use crate::settings::{Settings, MODES};
use crate::util::{log, valid_url};
use crate::{ipc, paths, tools, win};

const IDLE_EXIT: Duration = Duration::from_secs(180);
const KEEP_FINISHED: usize = 50;
const MAX_ACTIVE: usize = 200;
const MAX_COOKIES: usize = 5000;

pub struct Service {
    jobs: Mutex<Vec<Arc<Job>>>,
    next_id: AtomicU64,
    slots: Arc<Slots>,
    cobalt: Arc<LocalCobalt>,
    last_activity: Mutex<Instant>,
    /// yt-dlp version, or what is happening to it ("downloading yt-dlp… 40%")
    ytdlp: Mutex<String>,
    busy_tools: AtomicU64,
}

fn ok() -> Value {
    json!({"ok": true})
}
fn fail(msg: impl Into<String>) -> Value {
    json!({"ok": false, "error": msg.into()})
}

/// Cookies the extension sent for one site → a Netscape cookie file yt-dlp can read.
fn write_cookie_file(cookies: &[Value]) -> Option<PathBuf> {
    if cookies.is_empty() || cookies.len() > MAX_COOKIES {
        return None;
    }
    let clean = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").replace(['\t', '\r', '\n'], "");
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let host_only = c.get("hostOnly").and_then(Value::as_bool).unwrap_or(false);
        let mut domain = clean(c, "domain");
        if domain.is_empty() {
            continue;
        }
        if !host_only && !domain.starts_with('.') {
            domain.insert(0, '.');
        }
        let path = Some(clean(c, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| "/".into());
        let secure = c.get("secure").and_then(Value::as_bool).unwrap_or(false);
        let expires = c.get("expirationDate").and_then(Value::as_f64).unwrap_or(0.0) as u64;
        let flag = |b: bool| if b { "TRUE" } else { "FALSE" };
        text += &format!("{domain}\t{}\t{path}\t{}\t{expires}\t{}\t{}\n", flag(!host_only), flag(secure), clean(c, "name"), clean(c, "value"));
    }
    let dir = paths::tmp_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("cookies-{}.txt", crate::util::random_token()));
    std::fs::write(&file, text).ok()?;
    Some(file)
}

impl Service {
    fn new() -> Arc<Service> {
        Arc::new(Service {
            jobs: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            slots: Arc::new(Slots::new(MAX_PARALLEL)),
            cobalt: Arc::new(LocalCobalt::new()),
            last_activity: Mutex::new(Instant::now()),
            ytdlp: Mutex::new("checking…".into()),
            busy_tools: AtomicU64::new(0),
        })
    }

    fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    fn find(&self, msg: &Value) -> Option<Arc<Job>> {
        let id = msg.get("job").and_then(Value::as_u64)?;
        self.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned()
    }

    fn active(&self) -> usize {
        self.jobs.lock().unwrap().iter().filter(|j| j.is_active()).count()
    }

    /// Run a tool task (install/update) in the background, keeping the service alive meanwhile.
    fn tool_task(self: &Arc<Self>, f: impl FnOnce(&Service) + Send + 'static) {
        let me = self.clone();
        self.busy_tools.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            f(&me);
            me.busy_tools.fetch_sub(1, Ordering::SeqCst);
            me.touch();
        });
    }

    fn refresh_ytdlp(&self) {
        let set = |t: &str| *self.ytdlp.lock().unwrap() = t.to_string();
        match tools::ensure_ytdlp(&set) {
            Ok(_) => set(&tools::ytdlp_version().unwrap_or_else(|| "installed (version unknown)".into())),
            Err(e) => {
                log("service", &format!("yt-dlp install failed: {e}"));
                set(&format!("not installed — {e}"));
            }
        }
    }

    fn enqueue(self: &Arc<Self>, url: String, mut opts: Settings, cookie_file: Option<PathBuf>) -> u64 {
        if let Err(e) = std::fs::create_dir_all(&opts.out_dir) {
            log("service", &format!("download folder unusable ({e}), using the default"));
            opts.out_dir = Settings::default().out_dir;
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let job = Arc::new(Job::new(id, url, opts, cookie_file));
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.push(job.clone());
            // forget the oldest finished jobs
            let finished: Vec<u64> = jobs.iter().filter(|j| !j.is_active()).map(|j| j.id).collect();
            if finished.len() > KEEP_FINISHED {
                let drop: Vec<u64> = finished[..finished.len() - KEEP_FINISHED].to_vec();
                jobs.retain(|j| !drop.contains(&j.id));
            }
        }
        let me = self.clone();
        std::thread::spawn(move || {
            log("service", &format!("job {id} start: {} {}", job.opts.mode, job.url));
            execute(&job, &me.cobalt, &me.slots);
            let v = job.view();
            log("service", &format!("job {id} {}: {}", v.state, v.text));
            me.touch();
        });
        id
    }

    fn download(self: &Arc<Self>, msg: &Value) -> Value {
        let url = msg.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if !valid_url(&url) {
            return fail("not a web link");
        }
        if self.active() >= MAX_ACTIVE {
            return fail("too many downloads queued");
        }
        let mut opts = Settings::load(); // always the window's current settings
        if let Some(mode) = msg.get("mode").and_then(Value::as_str).filter(|m| MODES.contains(m)) {
            opts.mode = mode.to_string();
        }
        if let Some(p) = msg.get("playlist").and_then(Value::as_bool) {
            opts.playlist = p;
        }
        let cookie_file = msg.get("cookies").and_then(Value::as_array).and_then(|c| write_cookie_file(c));
        json!({"ok": true, "job": self.enqueue(url, opts, cookie_file)})
    }

    fn status(&self) -> Value {
        let mut jobs: Vec<_> = self.jobs.lock().unwrap().iter().map(|j| j.view()).collect();
        jobs.sort_by(|a, b| b.id.cmp(&a.id));
        let (state, text) = self.cobalt.status();
        json!({
            "ok": true,
            "version": crate::VERSION,
            "jobs": jobs,
            "ytdlp": *self.ytdlp.lock().unwrap(),
            "ffmpeg": tools::find_ffmpeg().is_some(),
            "cobalt": {"state": state.as_str(), "text": text},
        })
    }

    pub fn handle(self: &Arc<Self>, msg: Value) -> Value {
        self.touch();
        match msg.get("cmd").and_then(Value::as_str).unwrap_or("") {
            "ping" => json!({"ok": true, "service": true, "version": crate::VERSION, "active": self.active()}),
            "status" => self.status(),
            "download" => self.download(&msg),
            "cancel" => match self.find(&msg) {
                Some(j) => {
                    j.cancel.store(true, Ordering::SeqCst);
                    ok()
                }
                None => fail("no such download"),
            },
            "retry" => match self.find(&msg) {
                Some(j) if !j.is_active() => {
                    let mut opts = Settings::load();
                    opts.mode = j.opts.mode.clone();
                    opts.playlist = j.opts.playlist;
                    let id = j.id;
                    self.jobs.lock().unwrap().retain(|x| x.id != id);
                    json!({"ok": true, "job": self.enqueue(j.url.clone(), opts, None)})
                }
                _ => fail("no such finished download"),
            },
            "remove" => {
                if let Some(j) = self.find(&msg).filter(|j| !j.is_active()) {
                    self.jobs.lock().unwrap().retain(|x| x.id != j.id);
                }
                ok()
            }
            "clear" => {
                self.jobs.lock().unwrap().retain(|j| j.is_active());
                ok()
            }
            "reveal" => {
                let path = self.find(&msg).and_then(|j| j.view().path).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(Settings::load().out_dir));
                win::reveal(&path);
                ok()
            }
            "cobalt" => {
                let action = msg.get("action").and_then(Value::as_str).unwrap_or("").to_string();
                match action.as_str() {
                    "stop" => self.tool_task(|s| s.cobalt.stop()),
                    "start" => self.tool_task(|s| drop(s.cobalt.ensure_running())),
                    "restart" => self.tool_task(|s| {
                        s.cobalt.stop();
                        let _ = s.cobalt.ensure_running();
                    }),
                    "update" => self.tool_task(|s| drop(s.cobalt.update())),
                    _ => return fail("unknown cobalt action"),
                }
                ok()
            }
            "update_ytdlp" => {
                self.tool_task(|s| {
                    let set = |t: &str| *s.ytdlp.lock().unwrap() = t.to_string();
                    match tools::update_ytdlp(&set) {
                        Ok(v) => set(&v),
                        Err(e) => set(&format!("update failed — {e}")),
                    }
                });
                ok()
            }
            other => fail(format!("unknown command '{other}'")),
        }
    }
}

/// Entry point of `fetchbox-host.exe service`.
pub fn run() -> i32 {
    let svc = Service::new();
    let handler = svc.clone();
    match ipc::serve(move |msg| handler.handle(msg)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return 0, // one is already running
        Err(e) => {
            log("service", &format!("can't open the service pipe: {e}"));
            return 1;
        }
    }
    log("service", &format!("started (pid {}, v{})", std::process::id(), crate::VERSION));
    // leftovers from a previous crash
    let _ = std::fs::remove_dir_all(paths::tmp_dir());
    svc.tool_task(|s| s.refresh_ytdlp());

    loop {
        std::thread::sleep(Duration::from_secs(2));
        let idle = svc.last_activity.lock().unwrap().elapsed();
        if idle > IDLE_EXIT && svc.active() == 0 && svc.busy_tools.load(Ordering::SeqCst) == 0 {
            break;
        }
    }
    log("service", "idle — shutting down");
    svc.cobalt.stop();
    0
}

/// Make sure a service is running (used by the window and the browser helper). True when reachable.
pub fn ensure_running() -> bool {
    let ping = json!({"cmd": "ping"});
    if ipc::request(&ping).is_some() {
        return true;
    }
    let exe = paths::sibling_exe("fetchbox-host.exe");
    if win::spawn_detached(&exe, &["service"]).is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(60));
        if ipc::request(&ping).is_some() {
            return true;
        }
    }
    false
}

/// Send a command to the service, starting it first if needed.
pub fn call(msg: &Value) -> Value {
    if let Some(r) = ipc::request(msg) {
        return r;
    }
    if !ensure_running() {
        return fail("Fetchbox's background service didn't start");
    }
    ipc::request(msg).unwrap_or_else(|| fail("Fetchbox's background service didn't answer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_file_format() {
        std::env::set_var("FETCHBOX_HOME", std::env::temp_dir().join("fetchbox-test-home"));
        let f = write_cookie_file(&[json!({"domain": "x.com", "name": "a\tb", "value": "v\n", "secure": true, "path": "/", "expirationDate": 99.5})]).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        let _ = std::fs::remove_file(&f);
        assert!(text.ends_with(".x.com\tTRUE\t/\tTRUE\t99\tab\tv\n"), "{text:?}");
        assert!(write_cookie_file(&[]).is_none());
    }
}
