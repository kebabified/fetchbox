//! The download engine: picks yt-dlp or cobalt for each link and runs the job.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::cobalt::{self, LocalCobalt};
use crate::settings::Settings;
use crate::tools;
use crate::util::{clean_err, human_bytes, safe_name, unique_path};
use crate::win::{kill_tree, quiet_command};

pub const MAX_PARALLEL: usize = 3;

// ── job ──
#[derive(Clone, Serialize, Debug)]
pub struct JobView {
    pub id: u64,
    pub url: String,
    pub mode: String,
    pub title: String,
    /// queued | running | done | failed | cancelled
    pub state: String,
    pub progress: Option<f32>,
    pub text: String,
    pub path: Option<String>,
    pub file: Option<String>,
}

pub struct Job {
    pub id: u64,
    pub url: String,
    pub opts: Settings,
    /// one-off cookie file from the browser extension (deleted when the job ends)
    pub cookie_file: Option<PathBuf>,
    pub cancel: AtomicBool,
    view: Mutex<JobView>,
    files: Mutex<Vec<PathBuf>>,
}

impl Job {
    pub fn new(id: u64, url: String, opts: Settings, cookie_file: Option<PathBuf>) -> Job {
        let view = JobView {
            id,
            url: url.clone(),
            mode: opts.mode.clone(),
            title: url.clone(),
            state: "queued".into(),
            progress: None,
            text: "queued".into(),
            path: None,
            file: None,
        };
        Job { id, url, opts, cookie_file, cancel: AtomicBool::new(false), view: Mutex::new(view), files: Mutex::new(Vec::new()) }
    }
    pub fn view(&self) -> JobView {
        self.view.lock().unwrap().clone()
    }
    pub fn is_active(&self) -> bool {
        matches!(self.view.lock().unwrap().state.as_str(), "queued" | "running")
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn set_title(&self, t: &str) {
        self.view.lock().unwrap().title = t.chars().take(200).collect();
    }
    fn set_progress(&self, frac: Option<f32>, text: impl Into<String>) {
        let mut v = self.view.lock().unwrap();
        v.state = "running".into();
        v.progress = frac.map(|f| f.clamp(0.0, 1.0));
        v.text = text.into();
    }
    fn finish(&self, state: &str, text: impl Into<String>, path: Option<&Path>) {
        let mut v = self.view.lock().unwrap();
        v.state = state.into();
        v.text = text.into();
        v.progress = (state == "done").then_some(1.0);
        v.path = path.map(|p| p.to_string_lossy().into_owned());
        v.file = path.and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
    }
}

/// Counting semaphore: at most `MAX_PARALLEL` jobs run at once.
pub struct Slots {
    free: Mutex<usize>,
    cv: Condvar,
}
impl Slots {
    pub fn new(n: usize) -> Self {
        Slots { free: Mutex::new(n), cv: Condvar::new() }
    }
    fn acquire(&self) {
        let mut free = self.free.lock().unwrap();
        while *free == 0 {
            free = self.cv.wait(free).unwrap();
        }
        *free -= 1;
    }
    fn release(&self) {
        *self.free.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

// ── routing ──
/// YouTube always goes through yt-dlp only — never cobalt, whatever the engine setting.
pub fn is_youtube(url: &str) -> bool {
    let host = url.split_once("://").map(|(_, r)| r).unwrap_or("").split(['/', '?', '#', ':']).next().unwrap_or("").to_ascii_lowercase();
    ["youtube.com", "youtube-nocookie.com", "youtu.be"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Which engines to try for this link, in order.
pub fn route(url: &str, engine: &str) -> Vec<&'static str> {
    if is_youtube(url) || engine == "yt-dlp" {
        vec!["yt-dlp"]
    } else if engine == "cobalt" {
        vec!["cobalt"]
    } else {
        vec!["yt-dlp", "cobalt"] // auto: yt-dlp first, cobalt as fallback
    }
}

enum Fail {
    Cancelled,
    Error(String),
}
impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Error(s)
    }
}
impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Fail::Error(s.to_string())
    }
}

/// Run one job start to finish: route it, fall back if needed, report through the job's view.
pub fn execute(job: &Job, local: &LocalCobalt, slots: &Slots) {
    slots.acquire();
    run(job, local);
    slots.release();
    if let Some(cf) = &job.cookie_file {
        let _ = std::fs::remove_file(cf); // one-off cookies: never left on disk
    }
}

fn run(job: &Job, local: &LocalCobalt) {
    if job.cancelled() {
        return job.finish("cancelled", "cancelled", None);
    }
    let mut order = route(&job.url, &job.opts.engine);
    if order.len() == 2 && !job.opts.cobalt_local && job.opts.cobalt_url.is_empty() {
        order.truncate(1); // no cobalt configured: nothing to fall back to
    }
    let mut errors: Vec<String> = Vec::new();
    for (i, name) in order.iter().enumerate() {
        job.files.lock().unwrap().clear();
        let result = if *name == "yt-dlp" { run_ytdlp(job) } else { run_cobalt(job, local) };
        match result {
            Ok(()) => {
                let files = job.files.lock().unwrap().clone();
                return match files.last() {
                    None => job.finish("done", format!("done via {name} (already downloaded?)"), None),
                    Some(last) => {
                        let size: u64 = files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
                        let label = if files.len() == 1 {
                            last.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
                        } else {
                            format!("{} files", files.len())
                        };
                        job.finish("done", format!("✓ {label}  ·  {}  ·  via {name}", human_bytes(size as f64)), Some(last))
                    }
                };
            }
            Err(Fail::Cancelled) => return job.finish("cancelled", "cancelled", None),
            Err(Fail::Error(_)) if job.cancelled() => return job.finish("cancelled", "cancelled", None),
            Err(Fail::Error(e)) => {
                errors.push(explain(clean_err(&e), job.cookie_file.is_some()));
                if i + 1 < order.len() {
                    job.set_progress(None, "yt-dlp failed, trying cobalt…");
                }
            }
        }
    }
    job.finish("failed", errors.join("  |  "), None);
}

/// Turns the errors people can fix themselves into what to do about them.
fn explain(err: String, sent_login: bool) -> String {
    let e = err.to_ascii_lowercase();
    if e.contains("dpapi") || e.contains("cookie database") || e.contains("decrypt") && e.contains("cookie") {
        return "Fetchbox can't read that browser's login (the browser locks it). In Settings set \"Use login from\" to \
                Nothing, and turn on \"use my login for this site\" in the browser extension instead."
            .into();
    }
    let wants_login = e.contains("sign in to confirm") || e.contains("--cookies") || e.contains("login required")
        || e.contains("requires authentication") || e.contains("use --username");
    if wants_login && sent_login {
        return "This site still wants you signed in: sign in to it in your browser, then download again.".into();
    }
    if wants_login {
        return "This video needs you signed in: click the Fetchbox button in your browser's toolbar, turn on \
                \"use my login for this site\", then download again."
            .into();
    }
    err
}

// ── yt-dlp ──
fn ytdlp_args(job: &Job, ffmpeg: Option<&Path>, node: Option<&Path>, temp: &Path) -> Vec<String> {
    let o = &job.opts;
    let mut tmpl = match o.filename.as_str() {
        "title [id]" => "%(title).150B [%(id)s].%(ext)s",
        "uploader - title" => "%(uploader,channel|Unknown).60B - %(title).120B.%(ext)s",
        _ => "%(title).150B.%(ext)s",
    }
    .to_string();
    if o.mode == "mute" {
        tmpl = tmpl.replace(".%(ext)s", " (mute).%(ext)s");
    }
    if o.playlist {
        tmpl = format!("%(playlist_title,playlist_id|.)s/{tmpl}");
    }

    let mut sort: Vec<String> = Vec::new();
    if o.quality != "best" {
        sort.push(format!("res:{}", o.quality));
    }
    sort.push(match o.codec.as_str() { "vp9" => "vcodec:vp9", "av1" => "vcodec:av01", _ => "vcodec:h264" }.into());
    if o.container == "mp4" {
        sort.push("ext:mp4:m4a".into());
    }

    let s = |x: &str| x.to_string();
    let mut a: Vec<String> = vec![
        // our settings only: ignore any yt-dlp config files and plugins on this PC
        s("--ignore-config"), s("--no-plugin-dirs"),
        s("--no-simulate"), s("--no-warnings"), s("--color"), s("never"), s("--encoding"), s("utf-8"),
        s("--newline"), s("--progress"),
        s("--progress-template"),
        s("download:FBX|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s"),
        s("--progress-template"), s("postprocess:FBP|%(progress.postprocessor)s|%(progress.status)s"),
        s("--print"), s("before_dl:FBT|%(playlist_index|0)s|%(n_entries|0)s|%(title)s"),
        s("--print"), s("after_move:FBF|%(filepath)s"),
        s("-P"), format!("home:{}", o.out_dir),
        s("-P"), format!("temp:{}", temp.display()),
        s("-o"), tmpl,
        s("--windows-filenames"), s("--no-mtime"),
        s(if o.playlist { "--yes-playlist" } else { "--no-playlist" }),
        s("-S"), sort.join(","),
        // MP4 is strict: if it truly can't be made the job fails here and auto routing hands it to cobalt.
        s("--merge-output-format"), if o.container == "mp4" { s("mp4") } else { format!("{}/mkv", o.container) },
        s("-N"), s("4"), s("--retries"), s("5"),
    ];
    if let Some(ff) = ffmpeg {
        a.extend([s("--ffmpeg-location"), ff.to_string_lossy().into_owned()]);
    }
    if let Some(node) = node {
        // YouTube's signature / "n" challenges: the solver scripts ship inside yt-dlp, this runs them
        a.extend([s("--js-runtimes"), format!("node:{}", node.display())]);
    }
    let mut final_ext = o.container.as_str();
    match o.mode.as_str() {
        "audio" => {
            a.extend([s("-f"), s("ba/b"), s("-x"), s("--audio-format"), o.audio_format.clone(), s("--audio-quality"), format!("{}K", o.audio_bitrate)]);
            final_ext = o.audio_format.as_str();
        }
        "mute" => a.extend([s("-f"), s("bv/b")]),
        _ => a.extend([s("-f"), s("bv*+ba/b")]),
    }
    if o.mode != "audio" && o.container == "mp4" && ffmpeg.is_some() {
        a.extend([s("--remux-video"), s("mp4")]); // single-file formats (.webm, .flv…) become MP4 too
    }
    if o.metadata && ffmpeg.is_some() {
        a.extend([s("--embed-metadata"), s("--embed-chapters")]);
        if !matches!(final_ext, "webm" | "wav" | "opus" | "best") {
            a.extend([s("--embed-thumbnail"), s("--convert-thumbnails"), s("jpg")]);
        }
    }
    if let Some(cf) = &job.cookie_file {
        a.extend([s("--cookies"), cf.to_string_lossy().into_owned()]);
    } else if o.cookies != "none" {
        a.extend([s("--cookies-from-browser"), o.cookies.clone()]);
    }
    a.extend([s("--"), job.url.clone()]); // after "--" nothing can be read as an option
    a
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn run_ytdlp(job: &Job) -> Result<(), Fail> {
    let o = &job.opts;
    let status = |t: &str| job.set_progress(None, format!("yt-dlp  ·  {t}"));
    let exe = tools::ensure_ytdlp(&status)?;
    let ffmpeg = match tools::ensure_ffmpeg(&status) {
        Ok(p) => Some(p),
        Err(e) => {
            crate::util::log("service", &format!("ffmpeg unavailable: {e}"));
            None
        }
    };
    if job.cancelled() {
        return Err(Fail::Cancelled);
    }
    let out_dir = PathBuf::from(&o.out_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("can't use the download folder: {e}"))?;
    let temp = out_dir.join(format!(".fetchbox-{}", job.id));
    // only YouTube needs it, so other sites never wait for the Node.js download
    let node = if is_youtube(&job.url) {
        match crate::cobalt::ensure_node(&status) {
            Ok(p) => Some(p),
            Err(e) => {
                crate::util::log("service", &format!("Node.js unavailable, YouTube formats may be missing: {e}"));
                None
            }
        }
    } else {
        None
    };
    if job.cancelled() {
        return Err(Fail::Cancelled);
    }
    let args = ytdlp_args(job, ffmpeg.as_deref().and_then(Path::parent), node.as_deref(), &temp);

    job.set_progress(None, "yt-dlp  ·  fetching info…");
    let mut child = quiet_command(&exe)
        .args(&args)
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start yt-dlp: {e}"))?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel::<(bool, String)>();
    for (is_err, stream) in [(false, child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>)), (true, child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>))] {
        let tx = tx.clone();
        if let Some(stream) = stream {
            std::thread::spawn(move || {
                let mut r = BufReader::new(stream);
                let mut buf = Vec::new();
                while r.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
                    let _ = tx.send((is_err, String::from_utf8_lossy(&buf).trim_end().to_string()));
                    buf.clear();
                }
            });
        }
    }
    drop(tx);

    let mut last_error = String::new();
    let mut cancelled = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((true, line)) => {
                if line.starts_with("ERROR:") {
                    last_error = line;
                }
            }
            Ok((false, line)) => handle_ytdlp_line(job, &line),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if job.cancelled() && !cancelled {
            cancelled = true;
            kill_tree(pid);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&temp); // half-downloaded pieces never linger

    if cancelled {
        return Err(Fail::Cancelled);
    }
    if !status.success() {
        return Err(Fail::Error(if last_error.is_empty() { format!("yt-dlp exited with {status}") } else { last_error }));
    }
    let files = job.files.lock().unwrap().clone();
    if o.container == "mp4" && o.mode != "audio" {
        let wrong: Vec<&PathBuf> = files.iter().filter(|f| !f.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4"))).collect();
        if let Some(first) = wrong.first() {
            let ext = first.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            for f in &wrong {
                let _ = std::fs::remove_file(f);
            }
            return Err(Fail::Error(format!("yt-dlp couldn't make an MP4 (got .{ext})")));
        }
    }
    if o.mode == "mute" {
        if let Some(ff) = &ffmpeg {
            job.set_progress(None, "yt-dlp  ·  removing sound…");
            for f in &files {
                strip_audio(f, ff);
            }
        }
    }
    Ok(())
}

fn handle_ytdlp_line(job: &Job, line: &str) {
    let mut parts = line.split('|');
    match parts.next() {
        Some("FBX") => {
            let v: Vec<Option<f64>> = parts.map(num).collect();
            let done = v.first().copied().flatten().unwrap_or(0.0);
            let total = v.get(1).copied().flatten().or(v.get(2).copied().flatten());
            let mut text = format!("{} / {}", human_bytes(done), human_bytes(total.unwrap_or(0.0)));
            if let Some(speed) = v.get(3).copied().flatten() {
                text += &format!("  ·  {}/s", human_bytes(speed));
            }
            if let Some(eta) = v.get(4).copied().flatten() {
                text += &format!("  ·  {}:{:02} left", eta as u64 / 60, eta as u64 % 60);
            }
            job.set_progress(total.filter(|t| *t > 0.0).map(|t| (done / t) as f32), format!("yt-dlp  ·  {text}"));
        }
        Some("FBP") => {
            let name = parts.next().unwrap_or("");
            if parts.next() == Some("started") && name != "MoveFiles" {
                job.set_progress(None, format!("yt-dlp  ·  {name}…"));
            }
        }
        Some("FBT") => {
            let idx = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let n = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let title = parts.collect::<Vec<_>>().join("|");
            job.set_title(&if idx > 0 && n > 1 { format!("{title}  ({idx}/{n})") } else { title });
        }
        Some("FBF") => {
            let path = parts.collect::<Vec<_>>().join("|");
            if !path.is_empty() {
                job.files.lock().unwrap().push(PathBuf::from(path));
            }
        }
        _ => {}
    }
}

fn strip_audio(path: &Path, ffmpeg: &Path) {
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_extension(format!("nosound.{ext}"));
    let ok = quiet_command(ffmpeg)
        .args(["-y", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0", "-an", "-c", "copy"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && tmp.exists() {
        let _ = std::fs::rename(&tmp, path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

// ── cobalt ──
fn run_cobalt(job: &Job, local: &LocalCobalt) -> Result<(), Fail> {
    let o = &job.opts;
    let (base, key) = if o.cobalt_local {
        job.set_progress(None, "cobalt  ·  starting the local server…");
        // show setup progress while a first-time install runs
        let done = AtomicBool::new(false);
        let url = std::thread::scope(|s| {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let (state, text) = local.status();
                    if matches!(state, cobalt::State::Setup | cobalt::State::Starting) {
                        job.set_progress(None, format!("cobalt  ·  {text}"));
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
            });
            let r = local.ensure_running();
            done.store(true, Ordering::Relaxed);
            r
        })?;
        (url, String::new())
    } else {
        if o.cobalt_url.is_empty() {
            return Err("no cobalt instance set — add one in Settings".into());
        }
        (cobalt::check_instance_url(&o.cobalt_url).map_err(|e| format!("cobalt instance: {e}"))?, o.cobalt_key())
    };
    if job.cancelled() {
        return Err(Fail::Cancelled);
    }
    job.set_progress(None, "cobalt  ·  asking the server…");
    let res = cobalt::api_post(&base, &key, &cobalt::request_body(&job.url, o))?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);

    let mut items: Vec<(String, Option<String>)> = Vec::new();
    match res.get("status").and_then(Value::as_str).unwrap_or("") {
        "error" => {
            let code = res.pointer("/error/code").and_then(Value::as_str).unwrap_or("unknown error");
            return Err(format!("cobalt: {code}").into());
        }
        "tunnel" | "redirect" => items.push((text(&res, "url").ok_or("cobalt: reply had no link")?, text(&res, "filename"))),
        "picker" => {
            match text(&res, "audio").filter(|_| o.mode == "audio") {
                Some(audio) => items.push((audio, text(&res, "audioFilename"))),
                None => items.extend(res.get("picker").and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((text(p, "url")?, None)))),
            }
        }
        "local-processing" => return Err("cobalt: this server needs local processing, which isn't supported".into()),
        other => return Err(format!("cobalt: unexpected reply '{other}'").into()),
    }
    if items.is_empty() {
        return Err("cobalt: nothing to download".into());
    }
    let out = PathBuf::from(&o.out_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("can't use the download folder: {e}"))?;
    let n = items.len();
    for (i, (url, name)) in items.into_iter().enumerate() {
        if !crate::util::valid_url(&url) {
            return Err("cobalt: reply contained an invalid link".into());
        }
        let prefix = if n > 1 { format!("cobalt  ·  {}/{n}  ·  ", i + 1) } else { "cobalt  ·  ".to_string() };
        let file = stream_to_file(job, &url, name, &out, &prefix)?;
        job.files.lock().unwrap().push(file);
    }
    Ok(())
}

fn stream_to_file(job: &Job, url: &str, name: Option<String>, out: &Path, prefix: &str) -> Result<PathBuf, Fail> {
    let resp = tools::download_agent().get(url).call().map_err(|e| format!("cobalt download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("cobalt download failed: HTTP {}", resp.status().as_u16()).into());
    }
    let header = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let name = name
        .or_else(|| header("content-disposition").and_then(|cd| cd.split("filename=").nth(1).map(|f| f.trim_matches(|c| c == '"' || c == ';' || c == ' ').to_string())))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            let ext = match header("content-type").unwrap_or_default().split(';').next().unwrap_or("") {
                "video/mp4" => ".mp4",
                "video/webm" => ".webm",
                "audio/mpeg" => ".mp3",
                "image/jpeg" => ".jpg",
                "image/png" => ".png",
                _ => "",
            };
            format!("download{ext}")
        });
    let dest = unique_path(out, &safe_name(&name)); // never outside the download folder
    let part = dest.with_file_name(format!("{}.part", dest.file_name().unwrap_or_default().to_string_lossy()));
    job.set_title(&dest.file_name().unwrap_or_default().to_string_lossy());
    let total = header("content-length").and_then(|v| v.parse::<f64>().ok()).filter(|t| *t > 0.0);

    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(&part).map_err(|e| format!("can't write to the download folder: {e}"))?;
    let (mut done, start, mut last) = (0f64, Instant::now(), Instant::now() - Duration::from_secs(1));
    let mut buf = vec![0u8; 256 * 1024];
    let result: Result<(), Fail> = loop {
        if job.cancelled() {
            break Err(Fail::Cancelled);
        }
        match reader.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                if let Err(e) = file.write_all(&buf[..n]) {
                    break Err(format!("disk error: {e}").into());
                }
                done += n as f64;
                if last.elapsed() > Duration::from_millis(150) {
                    last = Instant::now();
                    let speed = done / start.elapsed().as_secs_f64().max(0.001);
                    job.set_progress(total.map(|t| (done / t) as f32), format!("{prefix}{} / {}  ·  {}/s", human_bytes(done), human_bytes(total.unwrap_or(0.0)), human_bytes(speed)));
                }
            }
            Err(e) => break Err(format!("cobalt download interrupted: {e}").into()),
        }
    };
    drop(file);
    match result {
        Ok(()) => {
            std::fs::rename(&part, &dest).map_err(|e| format!("couldn't save the file: {e}"))?;
            Ok(dest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing() {
        for u in ["https://www.youtube.com/watch?v=x", "https://youtu.be/x", "https://music.youtube.com/watch?v=x", "https://www.youtube-nocookie.com/embed/x", "https://M.YouTube.com/shorts/x"] {
            assert_eq!(route(u, "cobalt"), vec!["yt-dlp"], "{u}");
            assert_eq!(route(u, "auto"), vec!["yt-dlp"], "{u}");
        }
        assert_eq!(route("https://x.com/a/status/1", "auto"), vec!["yt-dlp", "cobalt"]);
        assert_eq!(route("https://x.com/a/status/1", "cobalt"), vec!["cobalt"]);
        assert_eq!(route("https://notyoutube.com/watch?v=x", "auto"), vec!["yt-dlp", "cobalt"]);
        assert_eq!(route("https://evil.example/?u=youtube.com", "cobalt"), vec!["cobalt"]);
    }

    #[test]
    fn explains_login_errors() {
        let bot = "[youtube] ZfRYswD8mQk: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies".to_string();
        assert!(explain(bot.clone(), false).contains("use my login for this site"));
        assert!(explain(bot, true).contains("sign in to it in your browser"));
        assert!(explain("Failed to decrypt with DPAPI. See https://x".into(), false).contains("Use login from"));
        assert!(explain("Could not copy Chrome cookie database".into(), false).contains("Use login from"));
        assert_eq!(explain("HTTP Error 404".into(), false), "HTTP Error 404");
    }

    #[test]
    fn url_is_never_an_option() {
        let job = Job::new(1, "--exec=calc".into(), Settings::default(), None);
        let args = ytdlp_args(&job, None, Some(Path::new("n.exe")), Path::new("t"));
        assert!(args.windows(2).any(|w| w[0] == "--js-runtimes" && w[1] == "node:n.exe"));
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args.len(), dashdash + 2);
        assert!(args.contains(&"--ignore-config".to_string()));
    }
}
