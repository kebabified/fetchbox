//! Registers the browser helper with Chrome-family browsers (current user only, no admin).
//!
//! Each browser looks under its own registry key for a manifest that says which program to start
//! and which extension IDs may start it.

use serde_json::json;

use crate::paths;
use crate::win::{reg_delete, reg_set_default, reg_subkeys};

pub const HOST_NAME: &str = "com.fetchbox.host";
/// ID of the Fetchbox extension (fixed by the public key in extension/manifest.json).
pub const EXTENSION_ID: &str = "eolmboejalmfnjifabinocoalmgamgip";

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

/// Extension IDs allowed to use the helper: ours, plus any listed in native/allowed_extensions.txt
/// (one ID per line — for a store build or your own fork of the extension).
pub fn allowed_ids() -> Vec<String> {
    let mut ids = vec![EXTENSION_ID.to_string()];
    if let Ok(extra) = std::fs::read_to_string(paths::native_dir().join("allowed_extensions.txt")) {
        for id in extra.lines().map(str::trim).filter(|l| valid_extension_id(l)) {
            if !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}
pub fn allowed_origins() -> Vec<String> {
    allowed_ids().iter().map(|id| format!("chrome-extension://{id}/")).collect()
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

/// Register the helper for every browser. Returns the browser names.
pub fn install() -> Result<Vec<String>, String> {
    let host = paths::sibling_exe("fetchbox-host.exe");
    if !host.is_file() {
        return Err(format!("{} is missing", host.display()));
    }
    std::fs::create_dir_all(paths::native_dir()).map_err(|e| e.to_string())?;
    let manifest = json!({
        "name": HOST_NAME,
        "description": "Fetchbox downloader helper",
        "path": host,
        "type": "stdio",
        "allowed_origins": allowed_origins(),
    });
    std::fs::write(manifest_path(), serde_json::to_string_pretty(&manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
    let manifest_str = manifest_path().to_string_lossy().into_owned();
    let mut done = Vec::new();
    for (name, base) in browsers() {
        if reg_set_default(&format!(r"{base}\NativeMessagingHosts\{HOST_NAME}"), &manifest_str).is_ok() {
            done.push(name);
        }
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
    let _ = std::fs::remove_file(manifest_path());
    removed
}

/// Is the manifest in place and pointing at this copy of the helper?
pub fn is_current() -> bool {
    let host = paths::sibling_exe("fetchbox-host.exe");
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|p| std::path::Path::new(p) == host))
        .unwrap_or(false)
}

/// The folder to pick in the browser's "Load unpacked" dialog.
pub fn extension_dir() -> std::path::PathBuf {
    paths::exe_dir().join("extension")
}
