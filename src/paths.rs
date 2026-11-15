//! Where Fetchbox keeps things.
//!
//! * settings:  %APPDATA%\Fetchbox\settings.json
//! * app data (downloaded tools, logs, browser-helper manifest): the program's own folder when it is
//!   writable ("portable"), otherwise %LOCALAPPDATA%\Fetchbox (e.g. when installed in Program Files).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::APP_NAME;

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir())
}

/// Portable copy: a `portable.txt` next to the programs keeps settings in the folder too,
/// so nothing is left on the PC (and the folder can live on a USB stick).
pub fn is_portable() -> bool {
    exe_dir().join("portable.txt").is_file()
}

pub fn settings_dir() -> PathBuf {
    if is_portable() {
        exe_dir().join("settings")
    } else {
        env_dir("APPDATA").join(APP_NAME)
    }
}
pub fn settings_file() -> PathBuf {
    settings_dir().join("settings.json")
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".fetchbox-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn app_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(d) = std::env::var_os("FETCHBOX_HOME") {
            return PathBuf::from(d);
        }
        let exe = exe_dir();
        if writable(&exe) {
            exe
        } else {
            env_dir("LOCALAPPDATA").join(APP_NAME)
        }
    })
}

pub fn runtime_dir() -> PathBuf {
    app_dir().join("runtime")
}
pub fn log_dir() -> PathBuf {
    app_dir().join("logs")
}
pub fn tmp_dir() -> PathBuf {
    app_dir().join("tmp")
}
pub fn native_dir() -> PathBuf {
    app_dir().join("native")
}

/// The other Fetchbox program, next to the running one.
pub fn sibling_exe(name: &str) -> PathBuf {
    exe_dir().join(name)
}

pub fn default_download_dir() -> PathBuf {
    env_dir("USERPROFILE").join("Downloads")
}
