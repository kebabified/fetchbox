//! Registers the browser helper with Chrome-family browsers and Firefox (current user only, no admin).
//!
//! Each browser looks under its own registry key for a manifest that says which program to start
//! and which extension IDs may start it.

use serde_json::json;

use crate::paths;
use crate::win::{reg_delete, reg_set_default, reg_subkeys};

pub const HOST_NAME: &str = "com.fetchbox.host";
/// ID of the Fetchbox extension (fixed by the public key in extension/manifest.json).
pub const EXTENSION_ID: &str = "eolmboejalmfnjifabinocoalmgamgip";
/// ID of the Fetchbox extension in Firefox (browser_specific_settings.gecko.id in the manifest).
pub const FIREFOX_ID: &str = "fetchbox@fetchbox";
/// Firefox looks for native-messaging hosts here.
const FIREFOX_KEY: &str = r"Software\Mozilla\NativeMessagingHosts";

const KNOWN_BROWSERS: &[(&str, &str)] = &[
    ("Chrome", r"Software\Google\Chrome"),
    ("Chromium", r"Software\Chromium"),
    ("Edge", r"Software\Microsoft\Edge"),
    ("Brave", r"Software\BraveSoftware\Brave-Browser"),
    ("Vivaldi", r"Software\Vivaldi"),
    ("Helium", r"Software\imput\Helium"),
];

fn valid_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// Firefox IDs look like an email address (`name@domain`) or a `{GUID}`.
fn valid_firefox_id(id: &str) -> bool {
    let ok = |s: &str, extra: &[u8]| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b));
    if let Some(guid) = id.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        return guid.len() == 36 && ok(guid, b"-");
    }
    id.len() <= 80 && id.split_once('@').is_some_and(|(name, domain)| ok(name, b"._+-") && ok(domain, b".-"))
}

/// Our ID plus any listed one per line in native/allowed_extensions.txt (a store build or your own fork).
fn with_extras(base: &str, valid: fn(&str) -> bool) -> Vec<String> {
    let mut ids = vec![base.to_string()];
    if let Ok(extra) = std::fs::read_to_string(paths::native_dir().join("allowed_extensions.txt")) {
        for id in extra.lines().map(str::trim).filter(|l| valid(l)) {
            if !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// Chrome-family extensions allowed to use the helper, as the origins the browser passes us.
pub fn allowed_origins() -> Vec<String> {
    with_extras(EXTENSION_ID, valid_extension_id).iter().map(|id| format!("chrome-extension://{id}/")).collect()
}
/// Firefox extensions allowed to use the helper.
pub fn allowed_firefox_ids() -> Vec<String> {
    with_extras(FIREFOX_ID, valid_firefox_id)
}
/// Whoever the browser says started us: a Chrome origin or a Firefox extension ID.
pub fn caller_allowed(caller: &str) -> bool {
    allowed_origins().iter().chain(allowed_firefox_ids().iter()).any(|c| c == caller)
}

fn firefox_installed() -> bool {
    reg_subkeys(r"Software\Mozilla").iter().any(|k| k.contains("Firefox"))
        || [r"C:\Program Files\Mozilla Firefox\firefox.exe", r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe"]
            .iter()
            .any(|p| std::path::Path::new(p).is_file())
}

/// Every Chromium-based browser we know of or can detect (they keep "BLBeacon"/"PreferenceMACs" keys).
fn browsers() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = KNOWN_BROWSERS.iter().map(|(n, k)| (n.to_string(), k.to_string())).collect();
    for company in reg_subkeys("Software") {
        for product in reg_subkeys(&format!(r"Software\{company}")) {
            let base = format!(r"Software\{company}\{product}");
            if product.to_ascii_lowercase().contains("webview") || found.iter().any(|(_, k)| k.eq_ignore_ascii_case(&base)) {
                continue;
            }
            let kids = reg_subkeys(&base);
            if kids.iter().any(|k| k == "BLBeacon" || k == "PreferenceMACs") {
                found.push((format!("{company} {product}"), base));
            }
        }
    }
    found
}

fn manifest_path() -> std::path::PathBuf {
    paths::native_dir().join(format!("{HOST_NAME}.json"))
}
/// Firefox wants `allowed_extensions` instead of `allowed_origins`, so it gets its own manifest.
fn firefox_manifest_path() -> std::path::PathBuf {
    paths::native_dir().join(format!("{HOST_NAME}.firefox.json"))
}

fn write_manifest(path: &std::path::Path, host: &std::path::Path, allow: (&str, Vec<String>)) -> Result<String, String> {
    let mut manifest = json!({
        "name": HOST_NAME,
        "description": "Fetchbox downloader helper",
        "path": host,
        "type": "stdio",
    });
    manifest[allow.0] = json!(allow.1);
    std::fs::write(path, serde_json::to_string_pretty(&manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Register the helper for every browser. Returns the browser names.
pub fn install() -> Result<Vec<String>, String> {
    let host = paths::sibling_exe("fetchbox-host.exe");
    if !host.is_file() {
        return Err(format!("{} is missing", host.display()));
    }
    std::fs::create_dir_all(paths::native_dir()).map_err(|e| e.to_string())?;
    let chrome = write_manifest(&manifest_path(), &host, ("allowed_origins", allowed_origins()))?;
    let firefox = write_manifest(&firefox_manifest_path(), &host, ("allowed_extensions", allowed_firefox_ids()))?;
    let mut done = Vec::new();
    for (name, base) in browsers() {
        if reg_set_default(&format!(r"{base}\NativeMessagingHosts\{HOST_NAME}"), &chrome).is_ok() {
            done.push(name);
        }
    }
    // registered even when Firefox isn't installed yet, so it works once it is
    if reg_set_default(&format!(r"{FIREFOX_KEY}\{HOST_NAME}"), &firefox).is_ok() && firefox_installed() {
        done.push("Firefox".into());
    }
    if done.is_empty() {
        return Err("couldn't write to the registry".into());
    }
    Ok(done)
}

pub fn uninstall() -> Vec<String> {
    let mut removed = Vec::new();
    for (name, base) in browsers() {
        if reg_delete(&format!(r"{base}\NativeMessagingHosts\{HOST_NAME}")) {
            removed.push(name);
        }
    }
    if reg_delete(&format!(r"{FIREFOX_KEY}\{HOST_NAME}")) {
        removed.push("Firefox".into());
    }
    let _ = std::fs::remove_file(manifest_path());
    let _ = std::fs::remove_file(firefox_manifest_path());
    removed
}

/// Are both manifests in place and pointing at this copy of the helper?
pub fn is_current() -> bool {
    let host = paths::sibling_exe("fetchbox-host.exe");
    [manifest_path(), firefox_manifest_path()].iter().all(|m| {
        std::fs::read_to_string(m)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|p| std::path::Path::new(p) == host))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_ids() {
        assert!(valid_extension_id(EXTENSION_ID));
        assert!(valid_firefox_id(FIREFOX_ID));
        assert!(valid_firefox_id("{12345678-1234-1234-1234-123456789abc}"));
        assert!(!valid_firefox_id(EXTENSION_ID));
        assert!(!valid_firefox_id("a b@c"));
        assert!(!valid_firefox_id("@c"));
        assert!(!valid_extension_id(FIREFOX_ID));
        assert!(caller_allowed("chrome-extension://eolmboejalmfnjifabinocoalmgamgip/"));
        assert!(caller_allowed(FIREFOX_ID));
        assert!(!caller_allowed("evil@example.com"));
    }
}

/// The folder to pick in the browser's "Load unpacked" dialog.
pub fn extension_dir() -> std::path::PathBuf {
    paths::exe_dir().join("extension")
}
