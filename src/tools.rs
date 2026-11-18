//! The external tools Fetchbox drives: yt-dlp and ffmpeg.
//!
//! Both are fetched from their official GitHub releases and checked against the release's
//! published SHA-256 before anything is unpacked or run.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::Agent;

use crate::paths::runtime_dir;
use crate::util::hex;
use crate::win::quiet_command;

const YTDLP_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const YTDLP_ZIP: &str = "yt-dlp_win.zip";
const FFMPEG_BASE: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download";
const FFMPEG_ZIP: &str = "ffmpeg-master-latest-win64-gpl.zip";

pub type Report<'a> = &'a (dyn Fn(&str) + Sync);

fn agent_with(global_timeout: Option<Duration>) -> Agent {
    Agent::new_with_config(
        Agent::config_builder()
            // Windows' own TLS and certificate store. ureq defaults to a built-in root list even with native-tls,
            // which lacks newer CAs (nodejs.org's failed with "unable to find any user-specified roots").
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
            .user_agent(crate::USER_AGENT)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_global(global_timeout)
            .build(),
    )
}

/// For small API calls.
pub fn api_agent() -> &'static Agent {
    static A: OnceLock<Agent> = OnceLock::new();
    A.get_or_init(|| agent_with(Some(Duration::from_secs(45))))
}
/// For large downloads (no overall time limit).
pub fn download_agent() -> &'static Agent {
    static A: OnceLock<Agent> = OnceLock::new();
    A.get_or_init(|| agent_with(None))
}

pub fn http_text(url: &str) -> Result<String, String> {
    let mut resp = api_agent().get(url).call().map_err(|e| format!("{url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status().as_u16()));
    }
    resp.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

/// Stream a URL to a file, returning its SHA-256. `progress(done, total)`; return false to abort.
pub fn download_file(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String, String> {
    let resp = download_agent().get(url).call().map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status().as_u16()));
    }
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_body().into_reader();
    let mut file = File::create(dest).map_err(|e| format!("can't write {}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("disk error: {e}"))?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if !progress(done, total) {
            drop(file);
            let _ = std::fs::remove_file(dest);
            return Err("cancelled".into());
        }
    }
    Ok(hex(&hasher.finalize()))
}

/// Find `name` in a checksum list ("<sha256>  <name>" per line).
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

pub fn remove_dir(p: &Path) {
    if p.exists() {
        // node_modules and .git contain read-only files
        let _ = quiet_command("cmd").args(["/c", "rmdir", "/s", "/q"]).arg(p).status();
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Download `<base>/<zip>`, verify it against `<base>/<sums>`, unpack into a fresh folder and return it.
pub fn fetch_verified_zip(base: &str, zip_name: &str, sums_name: &str, label: &str, report: Report) -> Result<PathBuf, String> {
    let rt = runtime_dir();
    std::fs::create_dir_all(&rt).map_err(|e| e.to_string())?;
    report(&format!("downloading {label}…"));
    let sums = http_text(&format!("{base}/{sums_name}"))?;
    let want = expected_hash(&sums, zip_name).ok_or_else(|| format!("no published checksum for {zip_name}"))?;
    let zip_path = rt.join(format!("{zip_name}.download"));
    let mut last = 0u64;
    let got = download_file(&format!("{base}/{zip_name}"), &zip_path, &mut |done, total| {
        if done - last > 4 * 1024 * 1024 {
            last = done;
            match total {
                Some(t) if t > 0 => report(&format!("downloading {label}… {}%", done * 100 / t)),
                _ => report(&format!("downloading {label}… {} MB", done / 1_048_576)),
            }
        }
        true
    })?;
    if got != want {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!("{label} download failed its checksum — not installed"));
    }
    report(&format!("unpacking {label}…"));
    let tmp = rt.join(format!("_unpack-{}", crate::util::random_token()));
    let result = unzip(&zip_path, &tmp);
    let _ = std::fs::remove_file(&zip_path);
    if let Err(e) = result {
        remove_dir(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

pub fn unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("bad archive: {e}"))?;
    archive.extract(dest).map_err(|e| format!("unpack failed: {e}")) // refuses paths that escape `dest`
}

fn install_lock() -> &'static Mutex<()> {
    static L: Mutex<()> = Mutex::new(());
    &L
}

// ── yt-dlp ──
pub fn ytdlp_exe() -> PathBuf {
    runtime_dir().join("yt-dlp").join("yt-dlp.exe")
}

pub fn ytdlp_version() -> Option<String> {
    let out = quiet_command(ytdlp_exe()).arg("--version").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

fn install_ytdlp(report: Report) -> Result<(), String> {
    let unpacked = fetch_verified_zip(YTDLP_BASE, YTDLP_ZIP, "SHA2-256SUMS", "yt-dlp", report)?;
    if !unpacked.join("yt-dlp.exe").exists() {
        remove_dir(&unpacked);
        return Err("yt-dlp archive didn't contain yt-dlp.exe".into());
    }
    let dir = runtime_dir().join("yt-dlp");
    remove_dir(&dir);
    std::fs::rename(&unpacked, &dir).map_err(|e| format!("couldn't install yt-dlp: {e}"))
}

/// Make sure yt-dlp is installed; returns its path.
pub fn ensure_ytdlp(report: Report) -> Result<PathBuf, String> {
    let _guard = install_lock().lock().unwrap_or_else(|e| e.into_inner());
    if !ytdlp_exe().exists() {
        install_ytdlp(report)?;
    }
    Ok(ytdlp_exe())
}

/// Replace yt-dlp with the latest release. Returns the new version.
pub fn update_ytdlp(report: Report) -> Result<String, String> {
    let _guard = install_lock().lock().unwrap_or_else(|e| e.into_inner());
    install_ytdlp(report)?;
    ytdlp_version().ok_or_else(|| "yt-dlp was installed but doesn't run".into())
}

// ── ffmpeg ──
fn bundled_ffmpeg() -> PathBuf {
    runtime_dir().join("ffmpeg").join("ffmpeg.exe")
}

/// ffmpeg from our runtime folder, or else from PATH.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let ours = bundled_ffmpeg();
    if ours.exists() {
        return Some(ours);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join("ffmpeg.exe")).find(|p| p.is_file()))
}

/// Make sure ffmpeg is available (downloads a build into the runtime folder if the PC has none).
pub fn ensure_ffmpeg(report: Report) -> Result<PathBuf, String> {
    if let Some(p) = find_ffmpeg() {
        return Ok(p);
    }
    let _guard = install_lock().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = find_ffmpeg() {
        return Ok(p);
    }
    let unpacked = fetch_verified_zip(FFMPEG_BASE, FFMPEG_ZIP, "checksums.sha256", "ffmpeg", report)?;
    let bin = unpacked.join(FFMPEG_ZIP.trim_end_matches(".zip")).join("bin");
    let dir = runtime_dir().join("ffmpeg");
    remove_dir(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut ok = false;
    for exe in ["ffmpeg.exe", "ffprobe.exe"] {
        ok |= std::fs::rename(bin.join(exe), dir.join(exe)).is_ok() && exe == "ffmpeg.exe";
    }
    remove_dir(&unpacked);
    if ok { Ok(bundled_ffmpeg()) } else { Err("ffmpeg archive had an unexpected layout".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_lookup() {
        let h = "a".repeat(64);
        let sums = format!("{}  other.zip\n{h}  yt-dlp_win.zip\n{h} *star.zip\n", "b".repeat(64));
        assert_eq!(expected_hash(&sums, "yt-dlp_win.zip"), Some(h.clone()));
        assert_eq!(expected_hash(&sums, "star.zip"), Some(h));
        assert_eq!(expected_hash(&sums, "missing.zip"), None);
    }
}
