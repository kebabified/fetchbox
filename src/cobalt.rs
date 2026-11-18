//! cobalt: the API client, and the manager for Fetchbox's own private cobalt server.
//!
//! The local server is set up on first use (portable Node.js, checksum-verified, plus cobalt's source
//! from GitHub), listens on 127.0.0.1 only, and is stopped when the background service exits.

use std::fs::File;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::paths::{app_dir, log_dir, runtime_dir};
use crate::tools::{self, Report};
use crate::util::{log, valid_url};
use crate::win::{kill_tree, process_is, quiet_command};

const NODE_MAJOR: &str = "v24."; // what cobalt's own Dockerfile uses
const COBALT_ZIP: &str = "https://github.com/imputnet/cobalt/archive/refs/heads/main.zip";
const COBALT_REPO: &str = "https://github.com/imputnet/cobalt";

// ── API client ──
/// A cobalt instance URL we are willing to talk to: https, or plain http only on this machine.
pub fn check_instance_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !valid_url(url) {
        return Err("not a valid web address".into());
    }
    let host = url.split_once("://").map(|(_, r)| r).unwrap_or("").split(['/', ':']).next().unwrap_or("");
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if url.starts_with("http://") && !local {
        return Err("use https:// for a remote cobalt instance (plain http would expose your API key)".into());
    }
    Ok(format!("{url}/"))
}

fn call(base: &str, key: &str, body: Option<&Value>) -> Result<Value, String> {
    let agent = tools::api_agent();
    let auth = format!("Api-Key {key}");
    let result = match body {
        Some(b) => {
            let mut r = agent.post(base).header("Accept", "application/json");
            if !key.is_empty() {
                r = r.header("Authorization", &auth);
            }
            r.send_json(b)
        }
        None => {
            let mut r = agent.get(base).header("Accept", "application/json");
            if !key.is_empty() {
                r = r.header("Authorization", &auth);
            }
            r.call()
        }
    };
    let mut resp = result.map_err(|e| format!("can't reach cobalt: {e}"))?;
    let status = resp.status().as_u16();
    resp.body_mut().read_json::<Value>().map_err(|_| format!("cobalt returned HTTP {status}"))
}

pub fn api_post(base: &str, key: &str, body: &Value) -> Result<Value, String> {
    call(base, key, Some(body))
}
/// Instance info (`{"cobalt": {"version", "services", …}}`).
pub fn api_info(base: &str, key: &str) -> Result<Value, String> {
    call(base, key, None)
}

// ── local server ──
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Off,
    Setup,
    Starting,
    Running,
    Error,
}
impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Setup => "setup",
            State::Starting => "starting",
            State::Running => "running",
            State::Error => "error",
        }
    }
}

struct Inner {
    state: State,
    text: String,
    port: u16,
    child: Option<Child>,
}

pub struct LocalCobalt {
    inner: Mutex<Inner>,
    /// serialises install/start/stop
    op: Mutex<()>,
}

fn node_dir() -> PathBuf {
    runtime_dir().join("node")
}
fn node_exe() -> PathBuf {
    node_dir().join("node.exe")
}
fn cobalt_dir() -> PathBuf {
    runtime_dir().join("cobalt")
}
fn api_dir() -> PathBuf {
    cobalt_dir().join("api")
}
fn pid_file() -> PathBuf {
    runtime_dir().join("cobalt.pid")
}
pub fn log_file() -> PathBuf {
    log_dir().join("cobalt.log")
}

fn free_port() -> Result<u16, String> {
    (9000..9100).find(|p| TcpListener::bind(("127.0.0.1", *p)).is_ok()).ok_or_else(|| "no free port for cobalt".to_string())
}

fn ping(port: u16) -> Option<Value> {
    api_info(&format!("http://127.0.0.1:{port}/"), "").ok()?.get("cobalt").cloned()
}

impl Default for LocalCobalt {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalCobalt {
    pub fn new() -> Self {
        LocalCobalt { inner: Mutex::new(Inner { state: State::Off, text: "not started".into(), port: 0, child: None }), op: Mutex::new(()) }
    }

    fn set(&self, state: State, text: &str) {
        let mut i = self.inner.lock().unwrap();
        i.state = state;
        i.text = text.to_string();
    }

    pub fn status(&self) -> (State, String) {
        let i = self.inner.lock().unwrap();
        (i.state, i.text.clone())
    }

    fn alive(&self) -> Option<u16> {
        let mut i = self.inner.lock().unwrap();
        let exited = match i.child.as_mut() {
            Some(c) => c.try_wait().map(|s| s.is_some()).unwrap_or(true),
            None => true,
        };
        (i.state == State::Running && !exited).then_some(i.port)
    }

    /// Install if needed, start if needed; returns the server's URL.
    pub fn ensure_running(&self) -> Result<String, String> {
        let _op = self.op.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(port) = self.alive() {
            return Ok(format!("http://127.0.0.1:{port}/"));
        }
        match self.start() {
            Ok(url) => Ok(url),
            Err(e) => {
                self.kill_child();
                log("service", &format!("local cobalt failed: {e}"));
                self.set(State::Error, &e);
                Err(format!("local cobalt: {e}"))
            }
        }
    }

    fn start(&self) -> Result<String, String> {
        let report = |t: &str| self.set(State::Setup, t);
        ensure_node(&report)?;
        fetch_cobalt(&report)?;
        install_deps(&report)?;

        self.set(State::Starting, "starting cobalt…");
        self.kill_child();
        kill_stale();
        let port = free_port()?;
        let url = format!("http://127.0.0.1:{port}/");
        std::fs::create_dir_all(log_dir()).map_err(|e| e.to_string())?;
        let out = File::create(log_file()).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let mut cmd = quiet_command(node_exe());
        cmd.arg("src/cobalt")
            .current_dir(api_dir())
            .env("PATH", node_path_env())
            .env("API_URL", &url)
            .env("API_PORT", port.to_string())
            .env("API_LISTEN_ADDRESS", "127.0.0.1") // this PC only
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        let cookies = app_dir().join("cookies.json");
        if cookies.is_file() {
            cmd.env("COOKIE_PATH", cookies);
        }
        let child = cmd.spawn().map_err(|e| format!("couldn't start node: {e}"))?;
        let _ = std::fs::write(pid_file(), child.id().to_string());
        {
            let mut i = self.inner.lock().unwrap();
            i.port = port;
            i.child = Some(child);
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let exited = self.inner.lock().unwrap().child.as_mut().map(|c| c.try_wait().map(|s| s.is_some()).unwrap_or(true)).unwrap_or(true);
            if exited {
                return Err("cobalt exited on startup — see logs/cobalt.log".into());
            }
            if let Some(info) = ping(port) {
                let version = info.get("version").and_then(Value::as_str).unwrap_or("?");
                self.set(State::Running, &format!("cobalt {version} running on this PC"));
                return Ok(url);
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        Err("cobalt didn't come up within 60 seconds — see logs/cobalt.log".into())
    }

    fn kill_child(&self) {
        let child = self.inner.lock().unwrap().child.take();
        if let Some(mut c) = child {
            if c.try_wait().map(|s| s.is_none()).unwrap_or(false) {
                kill_tree(c.id());
            }
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(pid_file());
    }

    pub fn stop(&self) {
        let _op = self.op.lock().unwrap_or_else(|e| e.into_inner());
        self.kill_child();
        self.set(State::Off, "stopped");
    }

    /// Get the latest cobalt and reinstall its dependencies, then start it.
    pub fn update(&self) -> Result<String, String> {
        {
            let _op = self.op.lock().unwrap_or_else(|e| e.into_inner());
            self.kill_child();
            self.set(State::Setup, "removing the old cobalt…");
            tools::remove_dir(&cobalt_dir());
        }
        self.ensure_running()
    }
}

fn node_path_env() -> std::ffi::OsString {
    let mut paths = vec![node_dir()];
    if let Some(p) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(paths).unwrap_or_default()
}

/// If a previous run was force-closed, its node.exe may still be running — stop it.
fn kill_stale() {
    if let Some(pid) = std::fs::read_to_string(pid_file()).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
        if process_is(pid, "node.exe") {
            kill_tree(pid);
        }
    }
    let _ = std::fs::remove_file(pid_file());
}

/// Make sure the portable Node.js is installed; returns node.exe. cobalt runs on it, and yt-dlp uses it
/// to solve YouTube's JavaScript challenges (without one, signed-in requests get no video formats).
pub fn ensure_node(report: Report) -> Result<PathBuf, String> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    install_node(report)?;
    Ok(node_exe())
}

fn install_node(report: Report) -> Result<(), String> {
    if node_exe().exists() {
        return Ok(());
    }
    report("looking up Node.js…");
    let index: Value = serde_json::from_str(&tools::http_text("https://nodejs.org/dist/index.json")?).map_err(|e| e.to_string())?;
    let version = index
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| {
            r["version"].as_str().is_some_and(|v| v.starts_with(NODE_MAJOR))
                && r["files"].as_array().is_some_and(|f| f.iter().any(|x| x == "win-x64-zip"))
        })
        .and_then(|r| r["version"].as_str())
        .ok_or("no suitable Node.js release found")?
        .to_string();
    if !version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
        return Err("unexpected Node.js version string".into());
    }
    let name = format!("node-{version}-win-x64");
    let unpacked = tools::fetch_verified_zip(&format!("https://nodejs.org/dist/{version}"), &format!("{name}.zip"), "SHASUMS256.txt", "Node.js", report)?;
    tools::remove_dir(&node_dir());
    let r = std::fs::rename(unpacked.join(&name), node_dir()).map_err(|e| format!("couldn't install Node.js: {e}"));
    tools::remove_dir(&unpacked);
    r
}

fn fetch_cobalt(report: Report) -> Result<(), String> {
    if cobalt_dir().join(".git").exists() && api_dir().join("package.json").exists() {
        return Ok(());
    }
    report("downloading cobalt…");
    tools::remove_dir(&cobalt_dir());
    let rt = runtime_dir();
    std::fs::create_dir_all(&rt).map_err(|e| e.to_string())?;
    let zip = rt.join("cobalt.zip.download");
    tools::download_file(COBALT_ZIP, &zip, &mut |_, _| true)?;
    let tmp = rt.join(format!("_unpack-{}", crate::util::random_token()));
    let unzipped = tools::unzip(&zip, &tmp);
    let _ = std::fs::remove_file(&zip);
    unzipped?;
    let top = std::fs::read_dir(&tmp).ok().and_then(|mut d| d.next()).and_then(|e| e.ok()).map(|e| e.path()).ok_or("cobalt archive was empty")?;
    let moved = std::fs::rename(&top, cobalt_dir()).map_err(|e| format!("couldn't install cobalt: {e}"));
    tools::remove_dir(&tmp);
    moved?;
    // cobalt reads its version info from .git; a zip has none, so provide the three files it looks at.
    let git = cobalt_dir().join(".git");
    std::fs::create_dir_all(git.join("logs")).map_err(|e| e.to_string())?;
    let _ = std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n");
    let _ = std::fs::write(git.join("logs").join("HEAD"), "0 0000000000000000000000000000000000000000 x <x> 0 +0000\n");
    let _ = std::fs::write(git.join("config"), format!("[remote \"origin\"]\n\turl = {COBALT_REPO}\n"));
    Ok(())
}

fn install_deps(report: Report) -> Result<(), String> {
    if api_dir().join("node_modules").exists() {
        return Ok(());
    }
    report("installing cobalt's dependencies (one-time, about a minute)…");
    let corepack = node_dir().join("node_modules").join("corepack").join("dist").join("corepack.js");
    let out = quiet_command(node_exe())
        .arg(corepack)
        .args(["pnpm", "install", "--prod", "--frozen-lockfile", "--filter", "@imput/cobalt-api..."])
        .current_dir(cobalt_dir())
        .env("PATH", node_path_env())
        .env("COREPACK_HOME", runtime_dir().join("corepack"))
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run pnpm: {e}"))?;
    let _ = std::fs::create_dir_all(log_dir());
    let _ = std::fs::write(log_dir().join("cobalt-install.log"), [out.stdout.as_slice(), b"\n", out.stderr.as_slice()].concat());
    if !out.status.success() {
        tools::remove_dir(&api_dir().join("node_modules"));
        return Err("installing cobalt's dependencies failed — see logs/cobalt-install.log".into());
    }
    Ok(())
}

/// Build the request cobalt expects from our settings.
pub fn request_body(url: &str, s: &crate::settings::Settings) -> Value {
    json!({
        "url": url,
        "downloadMode": s.mode,
        "videoQuality": if s.quality == "best" { "max" } else { s.quality.as_str() },
        "youtubeVideoCodec": s.codec,
        "audioFormat": if matches!(s.audio_format.as_str(), "mp3" | "opus" | "wav") { s.audio_format.as_str() } else { "best" },
        // cobalt has no 192 kbps option: use the next one up
        "audioBitrate": if s.audio_bitrate == "192" { "256" } else { s.audio_bitrate.as_str() },
        "filenameStyle": match s.filename.as_str() { "title [id]" => "classic", "uploader - title" => "pretty", _ => "basic" },
        "disableMetadata": !s.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_urls() {
        assert_eq!(check_instance_url("https://c.example/").unwrap(), "https://c.example/");
        assert_eq!(check_instance_url(" http://localhost:9000 ").unwrap(), "http://localhost:9000/");
        assert!(check_instance_url("http://c.example/").is_err());
        assert!(check_instance_url("ftp://x").is_err());
    }
}
