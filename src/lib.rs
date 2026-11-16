//! Fetchbox — a media downloader that drives yt-dlp and cobalt.
//!
//! Two programs share this library:
//!   * `fetchbox.exe`       the window (a client of the background service)
//!   * `fetchbox-host.exe`  the background service, the browser helper and the installer

pub mod cobalt;
pub mod engine;
pub mod host;
pub mod install;
pub mod ipc;
pub mod paths;
pub mod service;
pub mod settings;
pub mod sites;
pub mod tools;
pub mod update;
pub mod util;
pub mod win;

pub const APP_NAME: &str = "Fetchbox";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const USER_AGENT: &str = concat!("Fetchbox/", env!("CARGO_PKG_VERSION"));
