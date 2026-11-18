//! fetchbox.exe — the window. It is a client of the background service, so the queue it shows is
//! the same one the browser extension uses.
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Button, Color32, ComboBox, CornerRadius, Frame, Key, Layout, Margin, ProgressBar, RichText, ScrollArea,
    Stroke, TextEdit, ThemePreference, Ui, Vec2,
};
use serde_json::{json, Value};

use fetchbox::settings::{self, Settings};
use fetchbox::update::{self, Release};
use fetchbox::{cobalt, install, paths, service, util, win, APP_NAME, VERSION};

// status colours, readable on both dark and light backgrounds
fn green(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(63, 185, 80) } else { Color32::from_rgb(26, 127, 55) }
}
fn red(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(248, 81, 73) } else { Color32::from_rgb(207, 34, 46) }
}
fn amber(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(210, 153, 34) } else { Color32::from_rgb(154, 103, 0) }
}

fn main() -> eframe::Result {
    if !win::claim_single_instance("Local\\FetchboxWindow") {
        win::focus_window(APP_NAME); // already open: bring that window forward
        return Ok(());
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../icon.png")).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([780.0, 700.0])
            .with_min_inner_size([600.0, 480.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Downloads,
    Settings,
}

/// Things background threads hand back to the UI.
#[derive(Default)]
struct Shared {
    status: Option<Value>,
    service_down: bool,
    toast: Option<(String, bool, Instant)>,
    picked_dir: Option<String>,
    cobalt_test: Option<(String, bool)>,
    connect_result: Option<(String, bool)>,
    /// a newer release on GitHub
    update: Option<Release>,
    /// result of "Check for updates", or the update's download progress
    update_note: Option<(String, bool)>,
    updating: bool,
}

struct App {
    view: View,
    settings: Settings,
    saved: Settings,
    url: String,
    focus_url: bool,
    show_options: bool,
    cobalt_key: String,
    helper_ok: bool,
    /// "Later" on the update banner (until the window is reopened)
    update_dismissed: bool,
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
}

impl App {
    fn new(ctx: &egui::Context) -> App {
        let settings = Settings::load();
        let _ = settings.save(); // persist any migration (e.g. an old plain-text API key is now encrypted)
        fonts(ctx);
        style(ctx);
        apply_theme(ctx, &settings.appearance);
        let shared = Arc::new(Mutex::new(Shared::default()));
        // keep the queue and tool status fresh
        {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let reply = service::call(&json!({"cmd": "status"}));
                let ok = reply["ok"] == true;
                let busy = reply["jobs"].as_array().is_some_and(|j| j.iter().any(|x| matches!(x["state"].as_str(), Some("queued" | "running"))))
                    || matches!(reply["cobalt"]["state"].as_str(), Some("setup" | "starting"));
                {
                    let mut s = shared.lock().unwrap();
                    s.service_down = !ok;
                    if ok {
                        s.status = Some(reply);
                    }
                }
                ctx.request_repaint();
                std::thread::sleep(Duration::from_millis(if busy { 250 } else { 700 }));
            });
        }
        let app = App {
            view: if std::env::args().any(|a| a == "--settings") { View::Settings } else { View::Downloads },
            cobalt_key: settings.cobalt_key(),
            saved: settings.clone(),
            settings,
            url: String::new(),
            focus_url: true,
            show_options: false,
            helper_ok: install::is_current(),
            update_dismissed: false,
            shared,
            ctx: ctx.clone(),
        };
        if app.settings.check_updates {
            app.check_updates(false);
        }
        app
    }

    /// Ask GitHub for a newer release. Quiet unless `manual` (then the result shows under About).
    fn check_updates(&self, manual: bool) {
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        if manual {
            shared.lock().unwrap().update_note = Some(("Checking…".into(), true));
        }
        std::thread::spawn(move || {
            let result = update::latest();
            let mut s = shared.lock().unwrap();
            match result {
                Ok(r) if update::newer(&r.version, VERSION) => {
                    s.update_note = manual.then(|| (format!("Fetchbox {} is available", r.version), true));
                    s.update = Some(r);
                }
                Ok(_) => s.update_note = manual.then(|| ("You have the latest version".into(), true)),
                Err(e) => s.update_note = manual.then(|| (format!("Couldn't check for updates: {e}"), false)),
            }
            ctx.request_repaint();
        });
    }

    /// Installed copy: download the installer, check it, run it and close. Portable copy: open the release page.
    fn install_update(&self, release: Release) {
        let Some(asset) = release.installer.clone().filter(|_| paths::is_installed()) else {
            win::open_web(&release.page);
            return;
        };
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        shared.lock().unwrap().updating = true;
        std::thread::spawn(move || {
            let note = |text: String, ok: bool| {
                shared.lock().unwrap().update_note = Some((text, ok));
                ctx.request_repaint();
            };
            let mut last = 0;
            let result = update::download(&asset, &mut |done, total| {
                let pct = total.filter(|t| *t > 0).map(|t| done * 100 / t).unwrap_or(0);
                if pct != last {
                    last = pct;
                    note(format!("Downloading Fetchbox {}… {pct}%", release.version), true);
                }
                true
            });
            match result.and_then(|exe| {
                // /RELAUNCH: the installer opens Fetchbox again when it's done
                win::spawn_detached(&exe, &["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/RELAUNCH"]).map_err(|e| e.to_string())
            }) {
                Ok(()) => {
                    note(format!("Installing Fetchbox {}…", release.version), true);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => {
                    shared.lock().unwrap().updating = false;
                    note(format!("Update failed: {e}"), false);
                }
            }
        });
    }

    fn toast(&self, text: impl Into<String>, ok: bool) {
        self.shared.lock().unwrap().toast = Some((text.into(), ok, Instant::now()));
        self.ctx.request_repaint();
    }

    /// Send a command to the service without blocking the window.
    fn send(&self, msg: Value) {
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let r = service::call(&msg);
            if r["ok"] != true {
                let err = r["error"].as_str().unwrap_or("something went wrong").to_string();
                shared.lock().unwrap().toast = Some((err, false, Instant::now()));
            }
            ctx.request_repaint();
        });
    }

    fn start_downloads(&mut self) {
        let urls = util::find_urls(&self.url);
        if urls.is_empty() {
            self.toast("That doesn't look like a link", false);
            return;
        }
        let _ = self.settings.save();
        self.saved = self.settings.clone();
        for url in urls {
            self.send(json!({"cmd": "download", "url": url, "mode": self.settings.mode, "playlist": self.settings.playlist}));
        }
        self.url.clear();
        self.focus_url = true;
    }

    fn paste(&mut self) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) if !text.trim().is_empty() => {
                self.url = text.trim().to_string();
                self.focus_url = true;
            }
            _ => self.toast("The clipboard has no text", false),
        }
    }

    fn pick_folder(&self) {
        let (shared, ctx, start) = (self.shared.clone(), self.ctx.clone(), self.settings.out_dir.clone());
        std::thread::spawn(move || {
            if let Some(dir) = rfd::FileDialog::new().set_directory(start).pick_folder() {
                shared.lock().unwrap().picked_dir = Some(dir.to_string_lossy().into_owned());
                ctx.request_repaint();
            }
        });
    }

    // ── downloads view ──
    fn downloads(&mut self, ui: &mut Ui, status: &Option<Value>) {
        // a newer release: one line, with Update / What's new / Later
        let (release, note, updating) = {
            let s = self.shared.lock().unwrap();
            (s.update.clone(), s.update_note.clone(), s.updating)
        };
        if let Some(r) = release.filter(|_| !self.update_dismissed) {
            card(ui).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if updating {
                        ui.label(note.map(|n| n.0).unwrap_or_default());
                        return;
                    }
                    ui.label(RichText::new(format!("Fetchbox {} is available", r.version)).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let auto = r.installer.is_some() && paths::is_installed();
                        let go = primary_button(ui, if auto { "Update" } else { "Download" })
                            .on_hover_text(if auto { "Download, install and reopen Fetchbox" } else { "Open the release page" });
                        if go.clicked() {
                            self.install_update(r.clone());
                        }
                        if ui.button("What's new").clicked() {
                            win::open_web(&r.page);
                        }
                        if ui.button("Later").clicked() {
                            self.update_dismissed = true;
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }

        // link box
        card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                let go = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let go = primary_button(ui, "Download").clicked();
                    if ui.add(Button::new("Paste").min_size(Vec2::new(0.0, 32.0))).on_hover_text("Paste from the clipboard").clicked() {
                        self.paste();
                    }
                    let edit = ui.add_sized(
                        [ui.available_width(), 32.0],
                        TextEdit::singleline(&mut self.url).hint_text("Paste one or more links").frame(Frame::NONE).align(egui::Align2::LEFT_CENTER),
                    );
                    if self.focus_url {
                        edit.request_focus();
                        self.focus_url = false;
                    }
                    go || (edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                });
                if go.inner {
                    self.start_downloads();
                }
            });
        });
        ui.add_space(8.0);

        // what to save
        ui.horizontal(|ui| {
            let s = &mut self.settings;
            for (value, label) in [("auto", "Video"), ("audio", "Audio"), ("mute", "Video, no sound")] {
                if ui.add(Button::new(label).selected(s.mode == value).min_size(Vec2::new(0.0, 28.0))).clicked() {
                    s.mode = value.into();
                }
            }
            ui.add_space(10.0);
            if s.mode == "audio" {
                combo(ui, "fmt", &mut s.audio_format, settings::AUDIO_FORMATS, |v| v.to_string(), 76.0);
                combo(ui, "kbps", &mut s.audio_bitrate, settings::BITRATES, |v| format!("{v} kbps"), 96.0);
            } else {
                combo(ui, "q", &mut s.quality, settings::QUALITIES, |v| if v == "best" { "Best quality".into() } else { format!("{v}p") }, 110.0);
                combo(ui, "c", &mut s.container, settings::CONTAINERS, |v| v.to_uppercase(), 76.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(Button::new(if self.show_options { "Fewer options" } else { "More options" }).frame(false)).clicked() {
                    self.show_options = !self.show_options;
                }
            });
        });
        if self.show_options {
            ui.add_space(6.0);
            card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let s = &mut self.settings;
                egui::Grid::new("opts").num_columns(4).spacing([14.0, 8.0]).show(ui, |ui| {
                    ui.label(muted(ui, "Engine"));
                    combo(ui, "eng", &mut s.engine, settings::ENGINES, |v| match v { "auto" => "Automatic".into(), o => o.to_string() }, 130.0)
                        .on_hover_text("Automatic: yt-dlp first, cobalt if it fails.\nYouTube always uses yt-dlp.");
                    ui.label(muted(ui, "File name"));
                    combo(ui, "fn", &mut s.filename, settings::FILENAMES, |v| v.to_string(), 150.0);
                    ui.end_row();
                    ui.label(muted(ui, "YouTube codec"));
                    combo(ui, "codec", &mut s.codec, settings::CODECS, |v| v.to_uppercase(), 130.0);
                    ui.label(muted(ui, "Use login from"));
                    combo(ui, "ck", &mut s.cookies, settings::COOKIE_SOURCES, |v| if v == "none" { "Nothing".into() } else { "Firefox".into() }, 150.0)
                        .on_hover_text("For videos that need you signed in.\nFirefox: Fetchbox reads its login directly.\n\
                                        Chrome, Edge, Brave and others lock theirs: turn on \"use my login for this site\"\n\
                                        in the browser extension instead.");
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.playlist, "Whole playlist");
                    ui.add_space(12.0);
                    ui.checkbox(&mut s.metadata, "Add title, cover art and chapters to the file");
                });
            });
        }
        ui.add_space(10.0);

        // queue
        let jobs: Vec<Value> = status.as_ref().and_then(|s| s["jobs"].as_array().cloned()).unwrap_or_default();
        let finished = jobs.iter().any(|j| !matches!(j["state"].as_str(), Some("queued" | "running")));
        egui::Panel::bottom("footer").frame(Frame::NONE.inner_margin(Margin::symmetric(0, 8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(muted(ui, "Save to"));
                let dir = self.settings.out_dir.clone();
                if ui.add(Button::new(RichText::new(shorten(&dir, 52))).frame(false)).on_hover_text("Open this folder").clicked() {
                    win::reveal(&PathBuf::from(&dir));
                }
                if ui.small_button("Change").clicked() {
                    self.pick_folder();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(finished, Button::new("Clear finished").small()).clicked() {
                        self.send(json!({"cmd": "clear"}));
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
            if jobs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    ui.label(RichText::new("Nothing here yet").size(16.0));
                    ui.label(muted(ui, "Paste a link above, or use the Fetchbox button in your browser.\nYouTube, TikTok, Instagram, X, Reddit, SoundCloud, Twitch, Vimeo and 1,700+ more."));
                });
                return;
            }
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for job in &jobs {
                    self.job_row(ui, job);
                    ui.add_space(6.0);
                }
            });
        });
    }

    fn job_row(&self, ui: &mut Ui, job: &Value) {
        let id = job["id"].clone();
        let state = job["state"].as_str().unwrap_or("");
        let active = matches!(state, "queued" | "running");
        card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if active {
                        if ui.button("Cancel").clicked() {
                            self.send(json!({"cmd": "cancel", "job": id}));
                        }
                    } else {
                        if ui.add(Button::new("✕").frame(false)).on_hover_text("Remove from the list").clicked() {
                            self.send(json!({"cmd": "remove", "job": id}));
                        }
                        if state == "done" && job["path"].is_string() {
                            if ui.button("Show in folder").clicked() {
                                self.send(json!({"cmd": "reveal", "job": id}));
                            }
                        } else if state != "done" && ui.button("Retry").clicked() {
                            self.send(json!({"cmd": "retry", "job": id}));
                        }
                    }
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        let title = job["file"].as_str().or(job["title"].as_str()).unwrap_or("");
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate()).on_hover_text(job["url"].as_str().unwrap_or(""));
                        let text = job["text"].as_str().unwrap_or("");
                        let color = match state {
                            "done" => green(ui),
                            "failed" => red(ui),
                            _ => ui.visuals().weak_text_color(),
                        };
                        let detail = if state == "done" { text.split_once("  ·  ").map(|(_, rest)| format!("✓ {rest}")).unwrap_or(text.to_string()) } else { text.to_string() };
                        ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(color)).truncate()).on_hover_text(text);
                        if active {
                            ui.add_space(2.0);
                            let bar = match job["progress"].as_f64() {
                                Some(p) => ProgressBar::new(p as f32),
                                None => ProgressBar::new(0.0).animate(true),
                            };
                            ui.add(bar.desired_height(4.0).fill(green(ui)));
                        }
                    });
                });
            });
        });
    }

    // ── settings view ──
    fn settings_view(&mut self, ui: &mut Ui, status: &Option<Value>) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            section(ui, "Browser extension", |ui| {
                ui.horizontal(|ui| {
                    if self.helper_ok {
                        ui.colored_label(green(ui), "● Connected to your browsers");
                    } else {
                        ui.colored_label(amber(ui), "● Not connected yet");
                    }
                    if ui.button(if self.helper_ok { "Reconnect" } else { "Connect browsers" }).clicked() {
                        let r = install::install();
                        self.helper_ok = install::is_current();
                        self.shared.lock().unwrap().connect_result = Some(match r {
                            Ok(b) => (format!("Registered for {}. Restart your browser.", b.join(", ")), true),
                            Err(e) => (e, false),
                        });
                    }
                });
                if let Some((text, ok)) = self.shared.lock().unwrap().connect_result.clone() {
                    ui.colored_label(if ok { green(ui) } else { red(ui) }, text);
                }
                ui.label(muted(ui, "Chrome, Edge, Brave, Helium…: open the extensions page, turn on Developer mode, choose “Load unpacked” and pick this folder:"));
                ui.horizontal(|ui| {
                    let dir = install::extension_dir();
                    ui.monospace(dir.to_string_lossy());
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(dir.to_string_lossy().into_owned());
                    }
                    if ui.small_button("Open").clicked() {
                        win::reveal(&dir);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(muted(ui, "Firefox: download the signed add-on (…-firefox.xpi) and drag it into a Firefox window."));
                    if ui.small_button("Get it").on_hover_text("Opens the latest release on GitHub").clicked() {
                        win::open_web(&format!("https://github.com/{}/releases/latest", update::REPO));
                    }
                });
            });

            section(ui, "cobalt", |ui| {
                ui.label(muted(ui, "Used when yt-dlp can't download a link (never for YouTube)."));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings.cobalt_local, true, "Run it on this PC (automatic)");
                    ui.radio_value(&mut self.settings.cobalt_local, false, "Use another server");
                });
                if self.settings.cobalt_local {
                    let c = status.as_ref().map(|s| &s["cobalt"]);
                    let state = c.and_then(|c| c["state"].as_str()).unwrap_or("off");
                    let text = c.and_then(|c| c["text"].as_str()).unwrap_or("");
                    let (color, label) = match state {
                        "running" => (green(ui), format!("● {text}")),
                        "error" => (red(ui), format!("● {text}")),
                        "setup" | "starting" => (amber(ui), format!("● {text}")),
                        _ => (ui.visuals().weak_text_color(), "○ Starts by itself when a download needs it".to_string()),
                    };
                    ui.colored_label(color, label);
                    ui.label(muted(ui, "A private server only this PC can reach. First use downloads Node.js and cobalt (about 150 MB)."));
                    ui.horizontal(|ui| {
                        if ui.button(if state == "running" { "Restart" } else { "Start now" }).clicked() {
                            self.send(json!({"cmd": "cobalt", "action": "restart"}));
                        }
                        if ui.button("Update cobalt").clicked() {
                            self.send(json!({"cmd": "cobalt", "action": "update"}));
                        }
                        if ui.button("Open log").clicked() {
                            win::reveal(&cobalt::log_file());
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Address");
                        ui.add(TextEdit::singleline(&mut self.settings.cobalt_url).hint_text("https://…").desired_width(320.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("API key");
                        if ui.add(TextEdit::singleline(&mut self.cobalt_key).password(true).hint_text("optional").desired_width(320.0)).changed() {
                            self.settings.set_cobalt_key(&self.cobalt_key);
                        }
                    });
                    ui.label(muted(ui, "The key is stored encrypted for your Windows account."));
                    ui.horizontal(|ui| {
                        if ui.button("Test connection").clicked() {
                            let (shared, ctx, url, key) = (self.shared.clone(), self.ctx.clone(), self.settings.cobalt_url.clone(), self.cobalt_key.clone());
                            shared.lock().unwrap().cobalt_test = Some(("Checking…".into(), true));
                            std::thread::spawn(move || {
                                let result = cobalt::check_instance_url(&url).and_then(|base| cobalt::api_info(&base, &key)).and_then(|v| {
                                    let c = &v["cobalt"];
                                    let n = c["services"].as_array().map(|a| a.len()).unwrap_or(0);
                                    c["version"].as_str().map(|ver| format!("✓ cobalt {ver}, {n} services")).ok_or_else(|| "that address isn't a cobalt server".to_string())
                                });
                                shared.lock().unwrap().cobalt_test = Some(match result {
                                    Ok(t) => (t, true),
                                    Err(e) => (format!("✕ {e}"), false),
                                });
                                ctx.request_repaint();
                            });
                        }
                        if let Some((text, ok)) = self.shared.lock().unwrap().cobalt_test.clone() {
                            ui.colored_label(if ok { green(ui) } else { red(ui) }, text);
                        }
                    });
                }
            });

            section(ui, "Tools", |ui| {
                let ytdlp = status.as_ref().and_then(|s| s["ytdlp"].as_str()).unwrap_or("…").to_string();
                ui.horizontal(|ui| {
                    ui.label(format!("yt-dlp  {ytdlp}"));
                    if ui.button("Update yt-dlp").on_hover_text("Get the latest release (checked against its published checksum)").clicked() {
                        self.send(json!({"cmd": "update_ytdlp"}));
                    }
                });
                match status.as_ref().and_then(|s| s["ffmpeg"].as_bool()) {
                    Some(false) => ui.colored_label(amber(ui), "ffmpeg  not found — it is downloaded automatically with the first download"),
                    _ => ui.label("ffmpeg  ready"),
                };
            });

            section(ui, "Appearance", |ui| {
                ui.horizontal(|ui| {
                    for (value, label) in [("dark", "Dark"), ("light", "Light"), ("system", "Match Windows")] {
                        if ui.add(Button::new(label).selected(self.settings.appearance == value)).clicked() {
                            self.settings.appearance = value.into();
                            apply_theme(ui.ctx(), value);
                        }
                    }
                });
            });

            section(ui, "About", |ui| {
                ui.label(format!("Fetchbox {VERSION}"));
                ui.label(muted(ui, "Only download what you have the right to keep."));
                ui.horizontal(|ui| {
                    if ui.button("Check for updates").clicked() {
                        self.update_dismissed = false;
                        self.check_updates(true);
                    }
                    ui.checkbox(&mut self.settings.check_updates, "Check when Fetchbox opens");
                });
                if let Some((text, ok)) = self.shared.lock().unwrap().update_note.clone() {
                    ui.colored_label(if ok { ui.visuals().text_color() } else { red(ui) }, text);
                }
                ui.horizontal(|ui| {
                    if ui.button("Open logs folder").clicked() {
                        let _ = std::fs::create_dir_all(paths::log_dir());
                        win::reveal(&paths::log_dir());
                    }
                    if ui.button("Open settings file").clicked() {
                        win::reveal(&paths::settings_file());
                    }
                });
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (status, service_down, toast) = {
            let mut s = self.shared.lock().unwrap();
            if let Some(dir) = s.picked_dir.take() {
                self.settings.out_dir = dir;
            }
            if s.toast.as_ref().is_some_and(|(_, ok, at)| at.elapsed() > Duration::from_secs(if *ok { 3 } else { 6 })) {
                s.toast = None;
            }
            (s.status.clone(), s.service_down, s.toast.clone())
        };

        // keyboard: Ctrl+V anywhere pastes into the link box; Esc leaves Settings
        if ctx.memory(|m| m.focused().is_none()) && self.view == View::Downloads {
            if let Some(text) = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None })) {
                self.url = text.trim().to_string();
                self.focus_url = true;
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.view == View::Settings {
            self.view = View::Downloads;
        }

        egui::CentralPanel::default().frame(Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(22, 16))).show(ui, |ui| {
            // header: name, status, navigation
            ui.horizontal(|ui| {
                ui.label(RichText::new("fetchbox").size(24.0).strong());
                ui.add_space(10.0);
                if service_down {
                    ui.colored_label(red(ui), "● background service isn't running");
                } else if let Some(s) = &status {
                    let (color, text) = match s["cobalt"]["state"].as_str() {
                        Some("running") => (green(ui), "cobalt ready".to_string()),
                        Some("setup" | "starting") => (amber(ui), s["cobalt"]["text"].as_str().unwrap_or("cobalt starting").to_string()),
                        Some("error") => (red(ui), "cobalt failed — see Settings".to_string()),
                        _ => (ui.visuals().weak_text_color(), "cobalt on standby".to_string()),
                    };
                    ui.label(muted(ui, &format!("yt-dlp {}", s["ytdlp"].as_str().unwrap_or("…"))));
                    ui.label(muted(ui, "·"));
                    ui.label(RichText::new(text).size(12.0).color(color));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (view, label) in [(View::Settings, "Settings"), (View::Downloads, "Downloads")] {
                        if ui.add(Button::new(label).selected(self.view == view).min_size(Vec2::new(0.0, 28.0))).clicked() {
                            self.view = view;
                        }
                    }
                });
            });
            ui.add_space(12.0);
            match self.view {
                View::Downloads => self.downloads(ui, &status),
                View::Settings => self.settings_view(ui, &status),
            }
        });

        if let Some((text, ok, _)) = toast {
            egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -18.0]).show(&ctx, |ui| {
                Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(Stroke::new(1.0, if ok { green(ui) } else { red(ui) }))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 9))
                    .show(ui, |ui| ui.label(text));
            });
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        // remember changes straight away (the service reads this file for every download)
        if self.settings != self.saved {
            self.settings.cobalt_url = self.settings.cobalt_url.trim().to_string();
            if self.settings.save().is_ok() {
                self.saved = self.settings.clone();
            }
        }
    }
}

// ── look ──
/// Windows' own fonts: Segoe UI for the interface, plus symbol and CJK fallbacks so any video title is readable.
fn fonts(ctx: &egui::Context) {
    let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:/Windows".into())).join("Fonts");
    let mut defs = egui::FontDefinitions::default();
    let mut add = |file: &str, first: bool| {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            defs.font_data.insert(file.to_string(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let list = defs.families.entry(family.clone()).or_default();
                if first && family == egui::FontFamily::Proportional {
                    list.insert(0, file.to_string());
                } else {
                    list.push(file.to_string());
                }
            }
        }
    };
    add("segoeui.ttf", true);
    for fallback in ["seguisym.ttf", "msyh.ttc", "YuGothM.ttc", "malgun.ttf"] {
        add(fallback, false);
    }
    ctx.set_fonts(defs);
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(11.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        for w in [&mut s.visuals.widgets.inactive, &mut s.visuals.widgets.hovered, &mut s.visuals.widgets.active, &mut s.visuals.widgets.open, &mut s.visuals.widgets.noninteractive] {
            w.corner_radius = CornerRadius::same(8);
        }
        s.visuals.selection.bg_fill = Color32::from_rgb(46, 130, 62);
        s.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        for (_, font) in s.text_styles.iter_mut() {
            font.size *= 1.08;
        }
    });
}

fn apply_theme(ctx: &egui::Context, appearance: &str) {
    ctx.set_theme(match appearance {
        "light" => ThemePreference::Light,
        "system" => ThemePreference::System,
        _ => ThemePreference::Dark,
    });
}

fn card(ui: &Ui) -> Frame {
    Frame::NONE.fill(ui.visuals().faint_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(12, 9))
}

fn muted(ui: &Ui, text: &str) -> RichText {
    RichText::new(text).size(12.0).color(ui.visuals().weak_text_color())
}

fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (fill, fg) = if dark { (Color32::from_gray(240), Color32::from_gray(20)) } else { (Color32::from_gray(28), Color32::WHITE) };
    ui.add(Button::new(RichText::new(text).strong().color(fg)).fill(fill).min_size(Vec2::new(104.0, 32.0)))
}

fn combo(ui: &mut Ui, id: &str, value: &mut String, options: &[&str], label: impl Fn(&str) -> String, width: f32) -> egui::Response {
    ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(label(value))
        .show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, o.to_string(), label(o));
            }
        })
        .response
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(title).size(15.0).strong());
    ui.add_space(2.0);
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    ui.add_space(12.0);
}

fn shorten(path: &str, max: usize) -> String {
    let n = path.chars().count();
    if n <= max {
        path.to_string()
    } else {
        format!("…{}", path.chars().skip(n - max + 1).collect::<String>())
    }
}
