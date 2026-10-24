//! SoundCraft desktop app.
//!
//! Usage: `soundcraft [--demo] [--control <port>] [--no-audio] [session.scraft | audio files…]`
//!
//! `--control <port>` (or `SOUNDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server
//! (`0` picks a free port and prints it): `{"id":1,"method":"ui.inspect","params":{}}` →
//! `{"id":1,"ok":true,"result":…}`. See `soundcraft_ui_egui::control` for the methods.
//!
//! `log` records go to standard error and `<settings dir>/logs/soundcraft.log` (`RUST_LOG` sets the
//! levels); see [`logging`].
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod config;
mod control_server;
#[cfg(any(target_os = "windows", test))]
mod graphics;
mod logging;
#[cfg(target_os = "macos")]
mod native_menu;

use soundcraft_engine::Engine;
use soundcraft_ui_egui::{Services, SoundApp, UiState, dialogs};
use std::sync::Arc;

struct App(SoundApp, Option<&'static logging::AppLogger>, #[cfg(target_os = "macos")] Option<native_menu::NativeMenu>);

/// Sends the title only when it changes, remembered here as X11 and Wayland can't read it back.
fn set_title(ctx: &egui::Context, title: String) {
    let id = egui::Id::new("window_title");
    if ctx.data(|d| d.get_temp::<String>(id)).as_deref() != Some(title.as_str()) {
        ctx.data_mut(|d| d.insert_temp(id, title.clone()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // What the realtime audio thread logged is written here, off that thread.
        if let Some(logger) = self.1 {
            logger.report_audio_thread();
        }
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.2 {
            menu.process(&mut self.0);
        }
        self.0.guarded(|app| app.logic(ctx));
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.2 {
            menu.sync(&self.0);
        }
        // Files dropped on the window: audio on an instrument track or a Sampler window loads
        // the sampler; otherwise sessions open and audio/MIDI import.
        let dropped: Vec<String> =
            self.0.take_dropped_files(ctx).iter().map(|f| f.path().to_string_lossy().into_owned()).filter(|s| !s.is_empty()).collect();
        for p in dropped {
            open_path(&mut self.0, &p);
        }
        if self.0.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) && dialogs::ask_to_save(&mut self.0, "app.quit", &serde_json::json!({})) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        soundcraft_ui_egui::fit_window_to_screen(ctx);
        let (inner, maximized) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().maximized.unwrap_or(false)));
        self.0.ui.window_maximized = maximized;
        if let Some(r) = inner.filter(|_| !maximized) {
            self.0.ui.window_size = (r.size() * ctx.zoom_factor()).round().into();
        }
        self.0.ui.ui_scale = self.0.extra.ui_scale;
        // Saved now rather than at exit: the recent list matters most after a crash.
        if self.0.take_recent_changed() || self.0.take_prefs_dirty() {
            save_prefs(&mut self.0);
        }
        // With the system menu bar there is no egui bar showing the window and dirty state, so the
        // title carries them; elsewhere the egui menu bar already does.
        let title = if self.0.native_menu_bar {
            format!(
                "{}{} — SoundCraft — {}",
                self.0.engine.session().name,
                if self.0.engine.is_dirty() { " *" } else { "" },
                if self.0.ui.window == soundcraft_ui_egui::MainWindow::Edit { "Edit" } else { "Mix" }
            )
        } else {
            format!("{} — SoundCraft", self.0.engine.session().name)
        };
        set_title(ctx, title);
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.guarded(|app| app.ui(ui));
    }
    fn on_exit(&mut self) {
        self.0.close_autosave();
        save_prefs(&mut self.0);
        if let Some(logger) = self.1 {
            logger.report_audio_thread();
        }
        // The system Quit (⌘Q) ends the process from inside the event loop, so `run_native` never
        // returns: release the player's plugin instances and shut plugin hosting down here.
        self.0.player = None;
        soundcraft_engine::shutdown_plugin_hosts();
    }
}

fn open_path(app: &mut SoundApp, p: &str) {
    let lower = p.to_ascii_lowercase();
    let r = if lower.ends_with(".scraft") {
        dialogs::run_or_ask(app, "session.open", serde_json::json!({"path": p}))
    } else if lower.ends_with(".mid") || lower.ends_with(".midi") || lower.ends_with(".smf") {
        app.run("file.import_midi", serde_json::json!({"path": p}))
    } else {
        let at = app.engine.session().edit.selection.start;
        app.run("file.import_audio", serde_json::json!({"path": p, "at": at}))
    };
    if let Err(e) = r {
        log::warn!("{p}: {e}");
    }
}

/// The per-user settings directory: `ui.json`, `Autosave/`, `Presets/` and `logs/` live here.
fn config_dir() -> Option<std::path::PathBuf> {
    config::config_dir()
}

/// The preferences file, `config.json` (craftrules' settings convention). Older builds wrote
/// `ui.json`; it is read once so existing preferences survive the rename.
fn prefs_path() -> Option<std::path::PathBuf> {
    config_dir().map(|b| b.join("config.json"))
}

fn legacy_prefs_path() -> Option<std::path::PathBuf> {
    config_dir().map(|b| b.join("ui.json"))
}

/// Runs without preferences (`SOUNDCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
fn prefs_enabled() -> bool {
    std::env::var_os("SOUNDCRAFT_NO_PREFS").is_none()
}

fn load_prefs() -> Option<UiState> {
    if !prefs_enabled() {
        return None;
    }
    let path = prefs_path()?;
    match config::load_ui(&path) {
        Ok(Some(state)) => return Some(state),
        Ok(None) => {
            // Migrate `ui.json` from an older build; the next save writes `config.json`.
            let legacy = legacy_prefs_path()?;
            match config::load_ui(&legacy) {
                Ok(Some(state)) => return Some(state),
                Ok(None) => {}
                Err(error) => log::warn!("{legacy:?}: {error}"),
            }
        }
        Err(error) => log::warn!("{error}"),
    }
    None
}

fn save_prefs(app: &mut soundcraft_ui_egui::SoundApp) {
    if !prefs_enabled() {
        return;
    }
    // The controller setup and the keyboard range live in the app; copy them into the persisted
    // state before writing (they are not command-driven).
    app.sync_prefs();
    if let Some(path) = prefs_path()
        && let Err(error) = config::save_ui(&path, &app.ui)
    {
        log::warn!("{error}");
    }
}

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|purpose: &str, _exts: &[&str]| {
            let d = rfd::FileDialog::new();
            let d = match purpose {
                "session.open" => d.add_filter("SoundCraft Session", &["scraft"]),
                "file.import_midi" => d.add_filter("MIDI", &["mid", "midi", "smf"]),
                _ => d.add_filter(
                    "Audio",
                    &["wav", "wave", "bwf", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "m4a", "m4b", "aac", "mp4", "caf", "mkv", "webm"],
                ),
            };
            d.pick_file().map(|p| p.to_string_lossy().into_owned())
        })),
        pick_save: Some(Box::new(|purpose: &str, default: &str| {
            let path = std::path::Path::new(default);
            let mut d = match purpose {
                "file.bounce_mix" => rfd::FileDialog::new().add_filter("Audio", &["wav", "aif", "aiff", "flac"]),
                _ => rfd::FileDialog::new().add_filter("SoundCraft Session", &["scraft"]),
            };
            if let Some(dir) = path.parent().filter(|dir| dir.is_dir()) {
                d = d.set_directory(dir);
            }
            if let Some(n) = path.file_name().and_then(|n| n.to_str()) {
                d = d.set_file_name(n);
            }
            d.save_file().map(|p| p.to_string_lossy().into_owned())
        })),
        pick_folder: Some(Box::new(|start: &str| {
            rfd::FileDialog::new().set_directory(start).pick_folder().map(|p| p.to_string_lossy().into_owned())
        })),
    }
}

/// `--probe-plugin <bundle> <id>`: exercises one plugin in a throwaway process. A plugin is only
/// loaded in-process once this child proved it survives being prepared, run and saved. Yabridge aborts the host process on a Wine
/// failure, and that host is the editor, so a crash here has to mean "refuse", not "try and see".
fn probe_plugin(path: &str, id: &str) -> bool {
    let bundle = std::path::Path::new(path);
    let made = if id.starts_with("vst3:") {
        if let Ok(descriptors) = soundcraft_vst3_host::load_bundle(bundle) {
            soundcraft_vst3_host::adopt_scan(descriptors);
        }
        soundcraft_vst3_host::instantiate(id).map_err(|e| e.to_string())
    } else if id.starts_with("clap:") {
        if let Ok(descriptors) = soundcraft_clap_host::load_bundle(bundle) {
            soundcraft_clap_host::adopt_scan(descriptors);
        }
        soundcraft_clap_host::instantiate(id).map_err(|e| e.to_string())
    } else {
        return false;
    };
    let Ok(mut plugin) = made else { return false };
    plugin.prepare(48_000.0, 512, 2);
    plugin.reset();
    let mut io = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
    plugin.process(&mut io, 512);
    if plugin.info().is_instrument {
        plugin.note_on(0, 60, 100);
    } else {
        for (i, sample) in io[0].iter_mut().enumerate() {
            *sample = (i as f32 * 0.05).sin() * 0.25;
        }
        let left = io[0].clone();
        if let Some(right) = io.get_mut(1) {
            right.copy_from_slice(&left);
        }
    }
    plugin.process(&mut io, 512);
    plugin.all_notes_off();
    if !io.iter().all(|channel| channel.iter().all(|s| s.is_finite())) {
        log::warn!("plugin {id}: produced NaN or infinity while being validated");
    }
    let _ = plugin.save_state();
    // A session normally holds several instances of one plugin (the same piano on two tracks, say).
    // Some builds refuse or abort on the second instance, and that abort would kill the editor, so
    // the probe loads a second one while the first is still alive.
    let made = if id.starts_with("vst3:") {
        soundcraft_vst3_host::instantiate(id).map_err(|e| e.to_string())
    } else {
        soundcraft_clap_host::instantiate(id).map_err(|e| e.to_string())
    };
    let Ok(mut second) = made else { return false };
    second.prepare(48_000.0, 512, 2);
    let mut io = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
    second.process(&mut io, 512);
    let _ = second.save_state();
    true
}

/// `--verify-plugin <bundle> <id> [--editor]`: exercises one plugin through the same calls the
/// audio side and the UI make, prints one JSON line and exits. Everything runs in this child
/// process, so a plugin that segfaults or hangs costs one plugin of a batch run, not the run.
///
/// Stages are printed to stderr as they start, so a crash tells which call killed the child.
fn verify_plugin(path: &str, id: &str, with_editor: bool, editor_hold_ms: u64, program: Option<usize>, state_file: Option<&str>) {
    let stage = |name: &str| eprintln!("stage: {name}");
    let started = std::time::Instant::now();
    let bundle = std::path::Path::new(path);
    let mut report = serde_json::Map::new();
    report.insert("id".into(), serde_json::Value::String(id.to_string()));
    report.insert("bundle".into(), serde_json::Value::String(path.to_string()));
    fn fail(report: &mut serde_json::Map<String, serde_json::Value>, stage: &str, error: String) -> ! {
        report.insert("ok".into(), serde_json::Value::Bool(false));
        report.insert("stage".into(), serde_json::Value::String(stage.to_string()));
        report.insert("error".into(), serde_json::Value::String(error));
        println!("{}", serde_json::Value::Object(report.clone()));
        std::process::exit(2);
    }

    stage("load_bundle");
    let loaded = if id.starts_with("vst3:") {
        soundcraft_vst3_host::load_bundle(bundle).map_err(|e| e.to_string()).map(|d| {
            soundcraft_vst3_host::adopt_scan(d);
        })
    } else if id.starts_with("clap:") {
        soundcraft_clap_host::load_bundle(bundle).map_err(|e| e.to_string()).map(|d| {
            soundcraft_clap_host::adopt_scan(d);
        })
    } else {
        Err(format!("not a hosted plugin id: {id}"))
    };
    if let Err(e) = loaded {
        fail(&mut report, "load_bundle", e);
    }

    if id.starts_with("vst3:")
        && let Ok(classes) = soundcraft_vst3_host::bundle_classes(bundle)
    {
        report.insert("classes".into(), serde_json::to_value(classes).unwrap_or(serde_json::Value::Null));
    }

    stage("instantiate");
    let made = if id.starts_with("vst3:") {
        soundcraft_vst3_host::instantiate(id).map_err(|e| e.to_string())
    } else {
        soundcraft_clap_host::instantiate(id).map_err(|e| e.to_string())
    };
    let Ok(mut plugin) = made else {
        fail(&mut report, "instantiate", made.err().unwrap_or_default());
    };
    let info = plugin.info();
    report.insert("name".into(), serde_json::Value::String(info.name.to_string()));
    report.insert("instrument".into(), serde_json::Value::Bool(info.is_instrument));
    report.insert("params".into(), serde_json::Value::from(info.params.len()));

    stage("programs");
    // Preset banks the plugin publishes (VST3 `IUnitInfo`): what a console can switch an
    // instrument to without opening the plugin's own browser.
    let lists = plugin.program_lists();
    report.insert(
        "programs".into(),
        serde_json::Value::from(lists.iter().map(|l| serde_json::json!({"id": l.id, "name": l.name, "count": l.programs.len(), "first": l.programs.first().cloned().unwrap_or_default()})).collect::<Vec<_>>()),
    );

    // `--state-file PATH`: does this plugin take this blob as its state? That is how a preset
    // file (a .fxp, a .vstpreset chunk) turns into a real sound change from outside the plugin.
    if let Some(path) = state_file {
        let raw = std::fs::read(path).unwrap_or_default();
        let fingerprint = |p: &dyn soundcraft_dsp::Plugin| -> String {
            use std::fmt::Write as _;
            let mut h = String::new();
            for param in p.info().params.iter().take(24) {
                if let Some(v) = p.param(param.id) {
                    let _ = write!(h, "{:.4},", v);
                }
            }
            h
        };
        // A Pianoteq `.fxp` wraps its payload in file headers; try the payload at each marker.
        let markers = [b"FPCh".as_slice(), b"Pt6q".as_slice(), b"VST3".as_slice()];
        let mut tried = Vec::new();
        for (name, blob) in std::iter::once(("whole".to_string(), raw.clone()))
            .chain(markers.iter().flat_map(|m| {
                raw.windows(m.len()).position(|w| w == *m).map(|at| (format!("{}", String::from_utf8_lossy(m)), raw[at..].to_vec())).into_iter()
            }))
            .chain(raw.windows(4).position(|w| w == b"FPCh").map(|at| ("after-headers".to_string(), raw[at..].to_vec())))
        {
            let before = fingerprint(plugin.as_ref());
            let accepted = plugin.load_state(&blob);
            let changed = fingerprint(plugin.as_ref()) != before;
            tried.push(serde_json::json!({"from": name, "offset_bytes": blob.len(), "accepted": accepted, "values_changed": changed}));
        }
        report.insert("state_file".into(), serde_json::Value::String(path.to_string()));
        report.insert("state_file_tries".into(), serde_json::Value::from(tried));
    }

    // `--program N`: select a program and report whether the plugin's own values moved, which is
    // the difference between "the bank exists" and "the host can actually switch it".
    if let Some(index) = program {
        let fingerprint = |p: &dyn soundcraft_dsp::Plugin| -> String {
            use std::fmt::Write as _;
            let mut h = String::new();
            for param in p.info().params.iter().take(24) {
                if let Some(v) = p.param(param.id) {
                    let _ = write!(h, "{:.4},", v);
                }
            }
            h
        };
        let before = fingerprint(plugin.as_ref());
        let selected = plugin.select_program(0, index);
        let after = fingerprint(plugin.as_ref());
        report.insert("program".into(), serde_json::Value::from(index));
        report.insert("program_selected".into(), serde_json::Value::Bool(selected));
        report.insert("program_changed_values".into(), serde_json::Value::Bool(before != after));
    }

    stage("prepare");
    plugin.prepare(48_000.0, 512, 2);
    plugin.reset();
    report.insert("latency".into(), serde_json::Value::from(plugin.latency()));

    stage("process");
    // Two blocks: one of silence and one with a test tone or a note, so a plugin that only
    // misbehaves once running (or that returns garbage) is caught rather than assumed fine.
    let mut io = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
    plugin.process(&mut io, 512);
    if info.is_instrument {
        plugin.note_on(0, 60, 100);
    } else {
        for (i, sample) in io[0].iter_mut().enumerate() {
            *sample = (i as f32 * 0.05).sin() * 0.25;
        }
        let left: Vec<f32> = io[0].clone();
        if let Some(right) = io.get_mut(1) {
            right.copy_from_slice(&left);
        }
    }
    plugin.process(&mut io, 512);
    plugin.all_notes_off();
    let finite = io.iter().all(|ch| ch.iter().all(|s| s.is_finite()));
    let peak = io.iter().flat_map(|ch| ch.iter()).fold(0.0f32, |a, s| a.max(s.abs()));
    report.insert("finite".into(), serde_json::Value::Bool(finite));
    report.insert("peak".into(), serde_json::Value::from(peak));

    if id.starts_with("vst3:")
        && let Some((before, after)) = soundcraft_vst3_host::param_count_timeline(id)
    {
        report.insert("params_before_configure".into(), serde_json::Value::from(before));
        report.insert("params_after_configure".into(), serde_json::Value::from(after));
    }

    stage("state");
    let saved = plugin.save_state();
    let state = match &saved {
        None => "none".to_string(),
        Some(bytes) => {
            report.insert("state_bytes".into(), serde_json::Value::from(bytes.len()));
            // A fresh instance must accept its own save; a refusal is the session's state being
            // silently lost, which the editor has to say out loud.
            if id.starts_with("vst3:") {
                match soundcraft_vst3_host::instantiate(id) {
                    Ok(mut other) => {
                        if other.load_state(bytes) {
                            "round_trip".to_string()
                        } else {
                            "refused".to_string()
                        }
                    }
                    Err(e) => format!("reload_failed: {e}"),
                }
            } else {
                match soundcraft_clap_host::instantiate(id) {
                    Ok(mut other) => {
                        if other.load_state(bytes) {
                            "round_trip".to_string()
                        } else {
                            "refused".to_string()
                        }
                    }
                    Err(e) => format!("reload_failed: {e}"),
                }
            }
        }
    };
    report.insert("state".into(), serde_json::Value::String(state));

    stage("editor");
    let editor = if with_editor {
        // The app drives editors through the handle (`Plugin::editor`) and services them every
        // frame (`idle`); opening the view without pumping it leaves the plugin waiting for events
        // that never come, so the check has to use the same path the app does.
        match plugin.editor() {
            None => "none".to_string(),
            Some(mut handle) => match handle.open() {
                Err(e) => format!("error: {e}"),
                Ok(()) => {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(editor_hold_ms);
                    let mut edits = 0usize;
                    while std::time::Instant::now() < deadline {
                        edits += handle.idle().len();
                        std::thread::sleep(std::time::Duration::from_millis(16));
                    }
                    handle.close();
                    format!("ok (edits {edits})")
                }
            },
        }
    } else {
        "skipped".to_string()
    };
    report.insert("editor".into(), serde_json::Value::String(editor));
    report.insert("ok".into(), serde_json::Value::Bool(true));
    if !finite {
        report.insert("nan".into(), serde_json::Value::Bool(true));
        report.insert("stage".into(), serde_json::Value::String("process (NaN)".into()));
        report.insert("error".into(), serde_json::Value::String("the plugin filled its output with NaN or infinity".into()));
    }
    report.insert("ms".into(), serde_json::Value::from(started.elapsed().as_millis() as u64));
    println!("{}", serde_json::Value::Object(report));
}

/// `--stress-plugin BUNDLE ID [--rounds N]`: opens each instance's editor and then drops the
/// instance **without closing the editor first**, `N` times. That is what the editor does when a
/// plugin scan finishes and the session's instances are rebuilt: the detached thread that closes the
/// view must finish before the plugin objects are released, or the plugin's own code runs on freed
/// memory and the process dies with nothing in the log.
fn stress_plugin(path: &str, id: &str, rounds: u32) {
    let bundle = std::path::Path::new(path);
    let loaded = if id.starts_with("vst3:") {
        soundcraft_vst3_host::load_bundle(bundle).map(soundcraft_vst3_host::adopt_scan).map_err(|e| e.to_string())
    } else if id.starts_with("clap:") {
        soundcraft_clap_host::load_bundle(bundle).map(soundcraft_clap_host::adopt_scan).map_err(|e| e.to_string())
    } else {
        Err(format!("not a hosted plugin id: {id}"))
    };
    if let Err(e) = loaded {
        println!("{}", serde_json::json!({"ok": false, "stage": "load_bundle", "error": e}));
        return;
    }
    for round in 0..rounds {
        let made: Result<Box<dyn soundcraft_dsp::Plugin>, String> = if id.starts_with("vst3:") {
            soundcraft_vst3_host::instantiate(id).map_err(|e| e.to_string())
        } else {
            soundcraft_clap_host::instantiate(id).map_err(|e| e.to_string())
        };
        let Ok(mut plugin) = made else {
            println!("{}", serde_json::json!({"ok": false, "stage": "instantiate", "round": round}));
            return;
        };
        plugin.prepare(48_000.0, 512, 2);
        let mut opened = false;
        if let Some(mut handle) = plugin.editor()
            && handle.open().is_ok()
        {
            opened = true;
            // A few frames of the real pump, so the window is mapped and busy when it goes.
            for _ in 0..8 {
                handle.idle();
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        }
        eprintln!("round {round}: dropping the instance with its editor still open (opened: {opened})");
        drop(plugin);
        println!("{}", serde_json::json!({"ok": true, "round": round, "editor": opened}));
    }
}

/// `--scan-plugins [--dir PATH]… [--skip BUNDLE]…`: scans VST3 and CLAP plugins and prints one
/// JSON object per line. It is the app's own scanner child process: a plugin that segfaults or
/// aborts while being inspected kills only this child, never the editor.
///
/// Protocol (one object per line, flushed immediately):
/// * `{"scanning": "<bundle path>"}` — about to load this bundle (a crash points at it);
/// * `{"vst3": {…}}` / `{"clap": {…}}` — one plugin found.
fn scan_plugins(args: &[String]) -> eframe::Result {
    let mut dirs = soundcraft_vst3_host::scan::default_search_paths();
    let mut clap_dirs = soundcraft_clap_host::scan::default_search_paths();
    let mut skips: Vec<std::path::PathBuf> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                if let Some(p) = args.get(i + 1) {
                    let path = std::path::PathBuf::from(p);
                    if !dirs.contains(&path) {
                        dirs.push(path.clone());
                    }
                    if !clap_dirs.contains(&path) {
                        clap_dirs.push(path);
                    }
                }
                i += 2;
            }
            "--skip" => {
                if let Some(p) = args.get(i + 1) {
                    skips.push(std::path::PathBuf::from(p));
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    for path in soundcraft_vst3_host::scan::find_bundles(&dirs) {
        if skips.iter().any(|s| s == &path) {
            eprintln!("skipping {}", path.display());
            continue;
        }
        // Flushed per line: if this process dies, the editor keeps what was already found and
        // knows which bundle was being loaded.
        let _ = writeln!(out, "{}", serde_json::json!({"scanning": path.to_string_lossy()}));
        let _ = out.flush();
        match soundcraft_vst3_host::load_bundle(&path) {
            Ok(ds) => {
                for d in ds {
                    if let Ok(line) = serde_json::to_string(&serde_json::json!({ "vst3": d })) {
                        let _ = writeln!(out, "{line}");
                        let _ = out.flush();
                    }
                }
            }
            Err(e) => eprintln!("{e}"),
        }
    }
    for path in soundcraft_clap_host::scan::find_bundles(&clap_dirs) {
        if skips.iter().any(|s| s == &path) {
            eprintln!("skipping {}", path.display());
            continue;
        }
        let _ = writeln!(out, "{}", serde_json::json!({"scanning": path.to_string_lossy()}));
        let _ = out.flush();
        match soundcraft_clap_host::load_bundle(&path) {
            Ok(ds) => {
                for d in ds {
                    if let Ok(line) = serde_json::to_string(&serde_json::json!({ "clap": d })) {
                        let _ = writeln!(out, "{line}");
                        let _ = out.flush();
                    }
                }
            }
            Err(e) => eprintln!("{e}"),
        }
    }
    let _ = out.flush();
    Ok(())
}

/// The window, Dock, taskbar and Alt-Tab icon. macOS gets Apple's icon grid (transparent margin);
/// elsewhere the full tile. Regenerate with `packaging/icons.sh`.
fn app_icon() -> Option<egui::IconData> {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/soundcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.soundcraft.png");
    eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("app icon: {e}")).ok()
}

fn main() -> eframe::Result {
    // First, so every start-up record (and the engine's panic hook, installed with the first
    // Engine) is captured; see `logging`.
    let logger = logging::install();
    // A bridged Windows plugin may import DLLs that sit in its own folder (Pianoteq's engine and
    // many synths do). Wine only searches its own directories and `WINEPATH`, so put every
    // bridged plugin's folder there — once, before any worker thread can read the environment.
    let bridged = soundcraft_vst3_host::bridged_plugin_dirs();
    if !bridged.is_empty() {
        log::info!("bridged plugins: {} DLL folder(s) added to WINEPATH: {}", bridged.len(), bridged.join(" | "));
        soundcraft_vst3_host::add_wine_dll_dirs(&bridged);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("SoundCraft {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Plugin scanning runs in this same binary, in a child process, so a plugin that crashes the
    // scanner cannot take the editor down with it (see `spawn_async_sync`). It prints one JSON
    // object per line and exits; no display, no audio, no preferences are touched.
    if let Some(i) = args.iter().position(|a| a == "--scan-plugins") {
        return scan_plugins(&args[i + 1..]);
    }
    // Plugins are validated in this same binary, in a child process, before the editor loads them
    // in-process: yabridge aborts its host when a Wine plugin fails, and the host is the editor.
    if let Some(i) = args.iter().position(|a| a == "--verify-plugin") {
        let path = args.get(i + 1).cloned().unwrap_or_default();
        let id = args.get(i + 2).cloned().unwrap_or_default();
        let with_editor = args.iter().any(|a| a == "--editor");
        let hold = args.iter().position(|a| a == "--editor-hold").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(400u64);
        let program = args.iter().position(|a| a == "--program").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
        let state = args.iter().position(|a| a == "--state-file").and_then(|i| args.get(i + 1)).cloned();
        verify_plugin(&path, &id, with_editor, hold, program, state.as_deref());
        return Ok(());
    }
    // Drops instances with their editors still open, over and over: the case that closed the app
    // when a plugin scan finished and the session's instances were rebuilt under open editors.
    if let Some(i) = args.iter().position(|a| a == "--stress-plugin") {
        let path = args.get(i + 1).cloned().unwrap_or_default();
        let id = args.get(i + 2).cloned().unwrap_or_default();
        let rounds = args.iter().position(|a| a == "--rounds").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(6u32);
        stress_plugin(&path, &id, rounds);
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--probe-plugin") {
        let path = args.get(i + 1).cloned().unwrap_or_default();
        let id = args.get(i + 2).cloned().unwrap_or_default();
        let ok = probe_plugin(&path, &id);
        std::process::exit(if ok { 0 } else { 2 });
    }
    let demo = args.iter().any(|a| a == "--demo" || a == "--sample");
    let no_audio = args.iter().any(|a| a == "--no-audio");
    let control_port: Option<u16> = args
        .iter()
        .position(|a| a == "--control")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .or_else(|| std::env::var("SOUNDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok()));
    let files: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !(a.starts_with("--") || (i > &0 && args.get(i - 1).is_some_and(|p| p == "--control"))))
        .map(|(_, a)| a.clone())
        .collect();
    log::info!("SoundCraft {} ({} {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    // The log file lives in the settings directory; opened after the arguments, so `--version`
    // leaves no file behind. Records logged until now are written to it first. Runs without
    // preferences (agents' test runs) log to standard error only, so they don't rotate away the
    // user's own logs.
    if let Some(logger) = logger {
        match config_dir() {
            _ if !prefs_enabled() => logger.no_file(),
            Some(dir) => match logger.attach_dir(&dir.join(logging::LOG_DIR)) {
                Ok(path) => log::info!("log file {}", path.display()),
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => {
                logger.no_file();
                log::warn!("no log file: no settings directory (profile override is empty or platform configuration is unavailable)");
            }
        }
    }

    let engine = if demo { soundcraft_engine::demo::demo_engine() } else { Engine::default() };
    let prefs = load_prefs().unwrap_or_default();
    soundcraft_ui_egui::fx_browser::init_vst_paths();
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SoundCraft")
            .with_inner_size(sane_window_size(prefs.window_size))
            .with_maximized(prefs.window_maximized)
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_app_id("ai.storyteller.soundcraft"),
        ..Default::default()
    };
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    #[cfg(target_os = "linux")]
    if prefer_x11(std::env::var_os("DISPLAY").is_some(), std::env::var("SOUNDCRAFT_WAYLAND").ok().as_deref()) {
        log::info!("window system: X11 (XWayland when on Wayland) so plugin editors can be transient for the main window");
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }));
    }
    // Before eframe creates the wgpu instance: default Windows to DirectX 12 with FXC (see graphics.rs).
    #[cfg(target_os = "windows")]
    graphics::configure(&mut options, eframe::wgpu::Backends::from_env(), eframe::wgpu::Dx12Compiler::from_env());
    let result = eframe::run_native(
        "SoundCraft",
        options,
        Box::new(move |cc| {
            soundcraft_ui_egui::fx_browser::load_plugin_cache();
            soundcraft_playback::reset_probes();
            let player =
                if no_audio { None } else { Some(soundcraft_playback::Player::with_config(Arc::new(engine.session().clone()), &prefs.audio)) };
            let mut app = SoundApp::new(engine, player, services());
            app.autosave_dir = prefs_path().and_then(|p| p.parent().map(|d| d.join("Autosave")));
            app.leftovers = app.autosave_dir.as_deref().map(soundcraft_engine::autosave::leftovers).unwrap_or_default();
            app.preset_dir = prefs_path().and_then(|p| p.parent().map(|d| d.join("Presets")));
            app.extra.ui_scale = prefs.ui_scale;
            app.ui = prefs;
            app.apply_runtime_prefs();
            // Hardware control-surface I/O: a worker thread, absent in tests and agents' runs.
            if std::env::var_os("SOUNDCRAFT_NO_MIDI").is_none() {
                app.start_control_surface(app.msc.config.bluetooth_device_name_filter.clone());
                // Re-attach the MIDI keyboard the user picked last time (a piano, a controller).
                if let Some(port) = app.msc.keyboard_port.clone() {
                    app.control_surface_keyboard(Some(port));
                }
                if app.msc.ble {
                    app.control_surface_ble(true);
                }
                if let Some(port) = app.msc.surface_port.clone() {
                    app.control_surface_port(Some(port));
                }
            }
            if let Some(port) = control_port {
                let rx = control_server::start(port, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            // With no file on the command line, reopen the last session: starting on an empty
            // screen looks like the work vanished (and an empty session shows no tracks at all).
            let mut initial_files = files.clone();
            if initial_files.is_empty()
                && let Some(last) = app.ui.recent_sessions.first().cloned()
                && std::path::Path::new(&last).is_file()
            {
                log::info!("reopening the last session {last}");
                initial_files.push(last);
            }
            for f in &initial_files {
                open_path(&mut app, f);
            }
            // If the system menu bar cannot be built, keep the egui menu bar rather than fail to start.
            #[cfg(target_os = "macos")]
            let menu = match native_menu::NativeMenu::new(&app, &cc.egui_ctx) {
                Ok(m) => {
                    app.native_menu_bar = true;
                    Some(m)
                }
                Err(e) => {
                    log::warn!("system menu bar unavailable, using the in-window menu bar: {e}");
                    None
                }
            };
            Ok(Box::new(App(
                app,
                logger,
                #[cfg(target_os = "macos")]
                menu,
            )))
        }),
    );
    match &result {
        Ok(()) => log::info!("SoundCraft exited normally (event loop returned)"),
        Err(e) => log::error!("SoundCraft exited with an error: {e}"),
    }
    result
}

/// A saved window size, or the default when the prefs file holds nonsense (NaN, negative, tiny).
/// Linux plugin editors are X11 windows (VST3 `kPlatformTypeX11EmbedWindowID`, CLAP `x11`); a
/// window can only be transient for another X11 window. So with an X server around (XWayland on
/// a Wayland session) the main window uses X11 too, unless `SOUNDCRAFT_WAYLAND=1` asks for native
/// Wayland (editors then still open, kept above, but not owned by the main window).
#[cfg(any(target_os = "linux", test))]
fn prefer_x11(has_display: bool, wayland_override: Option<&str>) -> bool {
    has_display && !matches!(wayland_override.map(str::trim), Some("1" | "true" | "yes"))
}

fn sane_window_size([w, h]: [f32; 2]) -> [f32; 2] {
    if w.is_finite() && h.is_finite() && w >= 960.0 && h >= 600.0 { [w.min(16_384.0), h.min(16_384.0)] } else { [1600.0, 1000.0] }
}

#[cfg(test)]
mod tests {
    use super::prefer_x11;

    #[test]
    fn the_main_window_uses_x11_when_an_x_server_is_there_unless_wayland_is_asked_for() {
        assert!(prefer_x11(true, None));
        assert!(prefer_x11(true, Some("0")));
        assert!(!prefer_x11(true, Some("1")));
        assert!(!prefer_x11(true, Some(" yes ")));
        // No X server (pure Wayland without XWayland): native Wayland, whatever is asked.
        assert!(!prefer_x11(false, None));
    }

    use super::set_title;

    #[test]
    fn the_title_is_sent_only_when_it_changes() {
        let ctx = egui::Context::default();
        let titles = |title: &str| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| set_title(ui.ctx(), title.to_string()));
            out.textures_delta.clear();
            out.viewport_output.values().flat_map(|v| &v.commands).filter(|c| matches!(c, egui::ViewportCommand::Title(_))).count()
        };
        assert_eq!(titles("Song — SoundCraft"), 1);
        assert_eq!(titles("Song — SoundCraft"), 0);
        assert_eq!(titles("Song * — SoundCraft"), 1);
    }
}
