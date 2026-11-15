//! Small helpers shared by everything.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Only plain web links are ever handed to a downloader.
pub fn valid_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 4096
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && url.split_once("://").is_some_and(|(_, rest)| !rest.is_empty() && !rest.starts_with('/'))
}

/// Web links found in free text (the paste box accepts several at once).
pub fn find_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let w = word.trim_matches(|c| matches!(c, '<' | '>' | '"' | '\'' | ',' | ';'));
        if valid_url(w) && !out.iter().any(|u| u == w) {
            out.push(w.to_string());
        }
    }
    out
}

pub fn human_bytes(n: f64) -> String {
    if !(n > 0.0) {
        return "?".into();
    }
    let mut n = n;
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 {
            return if unit == "B" { format!("{n:.0} {unit}") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    format!("{n:.1} TB")
}

/// A file name that is safe on Windows and cannot escape its folder.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == ' ' || c == '.');
    let mut s: String = cleaned.chars().take(180).collect();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    if s.is_empty() || reserved.contains(&stem.as_str()) || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))) {
        s = format!("download {s}").trim().to_string();
    }
    s
}

pub fn unique_path(folder: &Path, name: &str) -> PathBuf {
    let p = folder.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (1..).map(|i| folder.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap()
}

/// First line of an error, without yt-dlp's "ERROR: " prefix or colour codes.
pub fn clean_err(msg: &str) -> String {
    let mut out = String::new();
    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    let line = out.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error");
    line.trim_start_matches("ERROR: ").chars().take(300).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An unguessable token for temp file names.
pub fn random_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seed = format!("{nanos}-{}-{}-{:p}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed), &COUNTER);
    sha256_hex(seed.as_bytes())[..24].to_string()
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let days = secs / 86400;
    // civil-from-days (UTC)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Append a line to logs/<name>.log (kept small; never contains cookies or page contents).
pub fn log(name: &str, msg: &str) {
    let dir = crate::paths::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.log"));
    if std::fs::metadata(&path).map(|m| m.len() > 300_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join(format!("{name}.old.log")));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {msg}", timestamp());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert!(valid_url("https://x.com/a/status/1"));
        assert!(!valid_url("file:///c:/x"));
        assert!(!valid_url("https://"));
        assert!(!valid_url("https://a b"));
        assert!(!valid_url("--exec=calc"));
        assert_eq!(find_urls("see https://a.b/c, and <https://d.e/f> x"), vec!["https://a.b/c", "https://d.e/f"]);
    }

    #[test]
    fn names() {
        assert_eq!(safe_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(safe_name("..\\..\\evil.exe"), "_.._evil.exe");
        assert_eq!(safe_name("CON.mp4"), "download CON.mp4");
        assert_eq!(safe_name("  .. "), "download");
        assert_eq!(clean_err("\x1b[0;31mERROR:\x1b[0m boom\nmore"), "boom");
    }
}
