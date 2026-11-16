//! Updates from GitHub releases.
//!
//! The window asks GitHub for the latest release. An installed copy can download that release's
//! installer, check it against the SHA-256 GitHub publishes for it, and run it; a portable copy
//! opens the release page instead (it is updated by replacing its files).

use std::path::PathBuf;

use serde_json::Value;

use crate::tools::{api_agent, download_file};
use crate::{paths, util};

/// Where releases are published.
pub const REPO: &str = "kebabified/fetchbox";

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "2.0.2" (the tag without its "v")
    pub version: String,
    /// the release page, for "What's new" and for portable copies
    pub page: String,
    /// `Fetchbox-<version>-setup.exe`, when the release has one with a published checksum
    pub installer: Option<Asset>,
}

/// Is version `a` newer than `b`? Compares the numbers of "major.minor.patch"; anything after a "-" is ignored.
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v').split('-').next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Only release files on GitHub itself are ever downloaded.
fn github_url(url: &str) -> bool {
    url.starts_with("https://github.com/") && util::valid_url(url)
}

fn parse(v: &Value) -> Result<Release, String> {
    let tag = v["tag_name"].as_str().ok_or("the release has no version")?;
    let version = tag.trim_start_matches('v').to_string();
    if !version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') || version.is_empty() {
        return Err("the release has an odd version".into());
    }
    let page = v["html_url"].as_str().filter(|u| github_url(u)).unwrap_or("").to_string();
    let wanted = format!("Fetchbox-{version}-setup.exe");
    let installer = v["assets"].as_array().into_iter().flatten().find_map(|a| {
        let name = a["name"].as_str()?;
        let url = a["browser_download_url"].as_str()?;
        let sha256 = a["digest"].as_str()?.strip_prefix("sha256:")?;
        (name == wanted && github_url(url) && sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Asset { name: name.into(), url: url.into(), sha256: sha256.to_ascii_lowercase() })
    });
    Ok(Release { version, page, installer })
}

/// The latest published release of `repo` ("owner/name").
pub fn latest_of(repo: &str) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut resp = api_agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("can't reach GitHub: {e}"))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Err("no public releases found".into()),
        403 | 429 => return Err("GitHub is limiting requests, try again later".into()),
        code => return Err(format!("GitHub answered HTTP {code}")),
    }
    let text = resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    parse(&serde_json::from_str(&text).map_err(|e| e.to_string())?)
}

/// The latest Fetchbox release.
pub fn latest() -> Result<Release, String> {
    latest_of(REPO)
}

/// Download a release file and check it against its published SHA-256. Returns where it was saved.
pub fn download(asset: &Asset, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<PathBuf, String> {
    let dir = paths::tmp_dir().join("update");
    let _ = std::fs::remove_dir_all(&dir); // older downloads
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(util::safe_name(&asset.name));
    let got = download_file(&asset.url, &dest, progress)?;
    if got != asset.sha256 {
        let _ = std::fs::remove_file(&dest);
        return Err("the download doesn't match its published checksum, so it wasn't run".into());
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert!(newer("2.0.2", "2.0.1"));
        assert!(newer("v2.1.0", "2.0.9"));
        assert!(newer("10.0.0", "9.9.9"));
        assert!(newer("2.0.1.1", "2.0.1"));
        assert!(!newer("2.0.1", "2.0.1"));
        assert!(!newer("2.0.0", "2.0.1"));
        assert!(!newer("2.0.1-beta", "2.0.1"));
    }

    #[test]
    fn parses_release() {
        let sha = "4c11b0312c2e8d303ef3eb7fb50a210d03edceedf3ff2e0cfe2d4ab91fd51041";
        let v = json!({
            "tag_name": "v2.0.2",
            "html_url": "https://github.com/kebabified/fetchbox/releases/tag/v2.0.2",
            "assets": [
                {"name": "Fetchbox-2.0.2-portable.zip", "browser_download_url": "https://github.com/x/y/z.zip", "digest": format!("sha256:{sha}")},
                {"name": "Fetchbox-2.0.2-setup.exe", "browser_download_url": "https://github.com/kebabified/fetchbox/releases/download/v2.0.2/Fetchbox-2.0.2-setup.exe", "digest": format!("sha256:{sha}")},
            ]
        });
        let r = parse(&v).unwrap();
        assert_eq!(r.version, "2.0.2");
        assert_eq!(r.installer.unwrap().name, "Fetchbox-2.0.2-setup.exe");

        // no checksum, or not hosted on GitHub: no automatic install
        let mut bad = v.clone();
        bad["assets"][1]["digest"] = Value::Null;
        assert!(parse(&bad).unwrap().installer.is_none());
        let mut bad = v.clone();
        bad["assets"][1]["browser_download_url"] = json!("https://evil.example/Fetchbox-2.0.2-setup.exe");
        assert!(parse(&bad).unwrap().installer.is_none());
        assert!(parse(&json!({"tag_name": "v2.0.2 & calc"})).is_err());
    }

    /// Needs the network: cargo test -- --ignored
    #[test]
    #[ignore]
    fn live_github() {
        let r = latest_of("yt-dlp/yt-dlp").unwrap();
        assert!(!r.version.is_empty() && r.page.starts_with("https://github.com/"));
        assert!(latest_of("kebabified/this-repo-does-not-exist").is_err());

        // download + checksum check, with a small file from yt-dlp's latest release
        let text = crate::tools::http_text("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest").unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let a = v["assets"].as_array().unwrap().iter().find(|a| a["name"] == "SHA2-256SUMS").unwrap();
        let mut asset = Asset {
            name: "SHA2-256SUMS".into(),
            url: a["browser_download_url"].as_str().unwrap().into(),
            sha256: a["digest"].as_str().unwrap().trim_start_matches("sha256:").into(),
        };
        let path = download(&asset, &mut |_, _| true).unwrap();
        assert!(path.is_file());
        asset.sha256 = "0".repeat(64);
        assert!(download(&asset, &mut |_, _| true).unwrap_err().contains("checksum"));
        assert!(!path.is_file(), "a file that fails the check is deleted");
    }
}
