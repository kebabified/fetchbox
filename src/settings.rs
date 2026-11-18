//! User settings (one JSON file, shared by the window and the background service).

use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;

use crate::paths;

pub const MODES: &[&str] = &["auto", "audio", "mute"];
pub const ENGINES: &[&str] = &["auto", "yt-dlp", "cobalt"];
pub const QUALITIES: &[&str] = &["best", "2160", "1440", "1080", "720", "480", "360", "240", "144"];
pub const CONTAINERS: &[&str] = &["mp4", "mkv", "webm"];
pub const CODECS: &[&str] = &["h264", "vp9", "av1"];
pub const AUDIO_FORMATS: &[&str] = &["mp3", "m4a", "opus", "flac", "wav", "best"];
pub const BITRATES: &[&str] = &["320", "256", "192", "128", "96", "64"];
pub const FILENAMES: &[&str] = &["title", "title [id]", "uploader - title"];
// Only Firefox: Chrome-family browsers lock their cookies on Windows so yt-dlp can't read them (the
// extension's "use my login" sends them instead). An old saved "chrome" etc. falls back to "none".
pub const COOKIE_SOURCES: &[&str] = &["none", "firefox"];
pub const APPEARANCES: &[&str] = &["dark", "light", "system"];
const VERSION: u64 = 3;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Settings {
    pub out_dir: String,
    /// auto (video) | audio | mute
    pub mode: String,
    /// auto (yt-dlp, then cobalt) | yt-dlp | cobalt — YouTube always uses yt-dlp
    pub engine: String,
    pub quality: String,
    pub container: String,
    pub codec: String,
    pub audio_format: String,
    pub audio_bitrate: String,
    pub filename: String,
    /// read cookies from this browser for yt-dlp ("none" = don't)
    pub cookies: String,
    pub playlist: bool,
    pub metadata: bool,
    /// true: Fetchbox runs its own cobalt; false: use `cobalt_url`
    pub cobalt_local: bool,
    pub cobalt_url: String,
    /// API key for a custom cobalt instance, encrypted for this Windows user (DPAPI), base64
    pub cobalt_key_enc: String,
    pub appearance: String,
    /// look for a new Fetchbox release on GitHub when the window opens
    pub check_updates: bool,
    pub settings_version: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            out_dir: paths::default_download_dir().to_string_lossy().into_owned(),
            mode: "auto".into(),
            engine: "auto".into(),
            quality: "1080".into(),
            container: "mp4".into(),
            codec: "h264".into(),
            audio_format: "mp3".into(),
            audio_bitrate: "320".into(),
            filename: "title".into(),
            cookies: "none".into(),
            playlist: false,
            metadata: true,
            cobalt_local: true,
            cobalt_url: String::new(),
            cobalt_key_enc: String::new(),
            appearance: "dark".into(),
            check_updates: true,
            settings_version: VERSION,
        }
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
fn flag(v: &Value, key: &str) -> Option<bool> {
    match v.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => Some(matches!(s.as_str(), "true" | "True" | "1")),
        Value::Number(n) => Some(n.as_i64() != Some(0)),
        _ => None,
    }
}
/// Keep a stored choice only if it is one of the allowed values.
fn pick(v: &Value, key: &str, allowed: &[&str], out: &mut String) {
    if let Some(s) = text(v, key) {
        if allowed.contains(&s.as_str()) {
            *out = s;
        }
    }
}

impl Settings {
    /// Reads the file leniently: unknown or invalid values fall back to defaults, older files are migrated.
    pub fn load() -> Settings {
        let raw = std::fs::read_to_string(paths::settings_file()).unwrap_or_default();
        Self::from_json(&raw)
    }

    pub fn from_json(raw: &str) -> Settings {
        let mut s = Settings::default();
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return s };
        if !v.is_object() {
            return s;
        }
        if let Some(d) = text(&v, "out_dir").filter(|d| !d.trim().is_empty()) {
            s.out_dir = d;
        }
        pick(&v, "mode", MODES, &mut s.mode);
        pick(&v, "engine", ENGINES, &mut s.engine);
        pick(&v, "quality", QUALITIES, &mut s.quality);
        pick(&v, "container", CONTAINERS, &mut s.container);
        pick(&v, "codec", CODECS, &mut s.codec);
        pick(&v, "audio_format", AUDIO_FORMATS, &mut s.audio_format);
        pick(&v, "audio_bitrate", BITRATES, &mut s.audio_bitrate);
        pick(&v, "filename", FILENAMES, &mut s.filename);
        pick(&v, "cookies", COOKIE_SOURCES, &mut s.cookies);
        pick(&v, "appearance", APPEARANCES, &mut s.appearance);
        s.playlist = flag(&v, "playlist").unwrap_or(s.playlist);
        s.metadata = flag(&v, "metadata").unwrap_or(s.metadata);
        s.check_updates = flag(&v, "check_updates").unwrap_or(s.check_updates);
        s.cobalt_url = text(&v, "cobalt_url").unwrap_or_default().trim().to_string();
        s.cobalt_key_enc = text(&v, "cobalt_key_enc").unwrap_or_default();
        s.cobalt_local = match flag(&v, "cobalt_local") {
            Some(b) => b,
            // v1/v2 stored "local (automatic)" / "custom instance"
            None => text(&v, "cobalt_source").map(|c| c.starts_with("local")).unwrap_or(true),
        };
        // v1/v2 stored the API key in plain text: encrypt it now.
        if s.cobalt_key_enc.is_empty() {
            if let Some(plain) = text(&v, "cobalt_key").filter(|k| !k.is_empty()) {
                s.set_cobalt_key(&plain);
            }
        }
        let version = text(&v, "settings_version").and_then(|t| t.parse::<u64>().ok()).unwrap_or(1);
        if version < 2 {
            s.engine = "auto".into(); // automatic per-site routing replaced the manual engine pick
        }
        s.settings_version = VERSION;
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let file = paths::settings_file();
        std::fs::create_dir_all(paths::settings_dir())?;
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, file) // atomic: the service never sees a half-written file
    }

    pub fn cobalt_key(&self) -> String {
        base64::engine::general_purpose::STANDARD
            .decode(&self.cobalt_key_enc)
            .ok()
            .and_then(|c| crate::win::unprotect(&c))
            .and_then(|p| String::from_utf8(p).ok())
            .unwrap_or_default()
    }

    pub fn set_cobalt_key(&mut self, plain: &str) {
        self.cobalt_key_enc = if plain.is_empty() {
            String::new()
        } else {
            crate::win::protect(plain.as_bytes())
                .map(|c| base64::engine::general_purpose::STANDARD.encode(c))
                .unwrap_or_default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_and_migrating() {
        let s = Settings::from_json(r#"{"quality":"9999","mode":"audio","playlist":"True","settings_version":"2",
            "cobalt_source":"custom instance","cobalt_url":" https://c.example/ ","engine":"cobalt","cobalt_key":"secret"}"#);
        assert_eq!(s.quality, "1080"); // invalid → default
        assert_eq!(s.mode, "audio");
        assert!(s.playlist);
        assert!(!s.cobalt_local);
        assert_eq!(s.cobalt_url, "https://c.example/");
        assert_eq!(s.engine, "cobalt");
        assert_eq!(s.cobalt_key(), "secret");
        assert!(!s.cobalt_key_enc.contains("secret"));
        assert_eq!(Settings::from_json("not json"), Settings::default());
        assert_eq!(Settings::from_json(r#"{"engine":"yt-dlp → cobalt"}"#).engine, "auto");
    }
}
