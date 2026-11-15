//! fetchbox-host.exe — background service, browser helper and installer (no window).
//!
//!   fetchbox-host chrome-extension://<id>/   started by the browser (native messaging)
//!   fetchbox-host service                    the background download service
//!   fetchbox-host install | uninstall        register / unregister the browser helper
//!   fetchbox-host check <url>…               is this link downloadable?
//!   fetchbox-host download <url> [auto|audio|mute]
//!   fetchbox-host status | tools             show the queue / install and locate yt-dlp and ffmpeg

use fetchbox::{host, install, service, sites};
use serde_json::json;

fn main() {
    // a crash is logged instead of vanishing (these programs have no console of their own)
    std::panic::set_hook(Box::new(|info| fetchbox::util::log("crash", &info.to_string())));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some(origin) if origin.starts_with("chrome-extension://") => host::run(origin),
        Some("service") => service::run(),
        Some("install") => match install::install() {
            Ok(browsers) => {
                println!("Browser helper registered for: {}", browsers.join(", "));
                println!("Extension folder (Load unpacked): {}", install::extension_dir().display());
                0
            }
            Err(e) => {
                eprintln!("Install failed: {e}");
                1
            }
        },
        Some("uninstall") => {
            println!("Removed registration for: {}", install::uninstall().join(", "));
            0
        }
        Some("check") => {
            for url in &args[1..] {
                println!("{url} -> {}", serde_json::to_string(&sites::check(url)).unwrap_or_default());
            }
            0
        }
        Some("download") if args.len() >= 2 => {
            let r = service::call(&json!({"cmd": "download", "url": args[1], "mode": args.get(2)}));
            println!("{r}");
            if r["ok"] == true { 0 } else { 1 }
        }
        Some("status") => {
            println!("{}", serde_json::to_string_pretty(&service::call(&json!({"cmd": "status"}))).unwrap_or_default());
            0
        }
        Some("tools") => {
            let say = |t: &str| println!("{t}");
            match fetchbox::tools::ensure_ytdlp(&say) {
                Ok(p) => println!("yt-dlp {} at {}", fetchbox::tools::ytdlp_version().unwrap_or_default(), p.display()),
                Err(e) => println!("yt-dlp: {e}"),
            }
            match fetchbox::tools::ensure_ffmpeg(&say) {
                Ok(p) => println!("ffmpeg at {}", p.display()),
                Err(e) => println!("ffmpeg: {e}"),
            }
            0
        }
        Some("--version") => {
            println!("fetchbox-host {}", fetchbox::VERSION);
            0
        }
        _ => {
            eprintln!("fetchbox-host {} — see the top of src/bin/host.rs for commands", fetchbox::VERSION);
            64
        }
    };
    std::process::exit(code);
}
