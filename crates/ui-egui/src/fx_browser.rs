//! Track FX Chain & Plugin Browser Window (REAPER-style intuitive FX Browser).
//!
//! Provides a dedicated, rich, multi-format plugin browser with category filters,
//! search, format badges (VST3, CLAP, Native), and an interactive track FX chain
//! inspector allowing instant insertion, reordering, bypass, and GUI opening.

use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Color32, Rect, Sense, Stroke, Ui, vec2};
use serde_json::json;
use soundcraft_dsp::Category;
use soundcraft_model::{Track, TrackKind};
use std::io::BufRead as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static SCAN_BUSY: AtomicBool = AtomicBool::new(false);
static SCAN_STATUS: Mutex<Option<String>> = Mutex::new(None);
/// Requests that arrived while a scan was running: (folders to register, bridge sync wanted).
static PENDING: Mutex<(Vec<String>, bool)> = Mutex::new((Vec::new(), false));
/// Set when a request arrived mid-scan, so the worker runs one more pass with it.
static SCAN_WAKE: AtomicBool = AtomicBool::new(false);

pub fn is_scanning() -> bool {
    SCAN_BUSY.load(Ordering::SeqCst)
}

pub fn take_scan_status() -> Option<String> {
    if let Ok(mut lock) = SCAN_STATUS.lock() { lock.take() } else { None }
}

/// A line printed by the `--scan-plugins` child process.
#[derive(Debug, serde::Deserialize)]
enum ScannedLine {
    #[serde(rename = "vst3")]
    Vst3(soundcraft_vst3_host::Vst3Descriptor),
    #[serde(rename = "clap")]
    Clap(soundcraft_clap_host::ClapDescriptor),
    /// About to load this bundle: if the child dies now, this is the culprit.
    #[serde(rename = "scanning")]
    Scanning(String),
}

/// What a scan produced. `skipped` names the bundles that crashed the scanner.
struct ScanOutcome {
    vst3: Vec<soundcraft_vst3_host::Vst3Descriptor>,
    clap: Vec<soundcraft_clap_host::ClapDescriptor>,
    note: Option<String>,
    /// Bundles whose plugin aborted the scanner process (typically a broken bridged Windows
    /// plugin). They are kept out of the catalogue: loading them in-process would abort the app.
    skipped: Vec<String>,
}

/// Bundles that crashed the scanner. Yabridge aborts its host process when a Windows plugin fails
/// to initialise, and in-process that host is the editor itself.
static BROKEN_BUNDLES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `~/.config/soundcraft/broken_plugins.json`: bundles that abort the scanner. Persisted so the
/// next scan skips them up front instead of paying a minute of Wine startup per broken plugin.
fn broken_file() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("SOUNDCRAFT_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config").join("soundcraft")))?;
    Some(dir.join("broken_plugins.json"))
}

fn load_broken() -> Vec<String> {
    let Some(path) = broken_file() else { return Vec::new() };
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok()).unwrap_or_default()
}

fn save_broken(list: &[String]) {
    let Some(path) = broken_file() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(path, text);
    }
}

/// Persistent plugin catalogue cache (~/.config/soundcraft/plugins_cache.json).
/// Avoids scanning 500+ plugins on every launch, enabling instant startup like Ardour.
fn plugin_cache_file() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("SOUNDCRAFT_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config").join("soundcraft")))?;
    Some(dir.join("plugins_cache.json"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PluginsCache {
    vst3: Vec<soundcraft_vst3_host::Vst3Descriptor>,
    clap: Vec<soundcraft_clap_host::ClapDescriptor>,
}

pub fn load_plugin_cache_from(path: &std::path::Path) -> bool {
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(cache) = serde_json::from_str::<PluginsCache>(&text)
    {
        let (nv, nc) = (cache.vst3.len(), cache.clap.len());
        soundcraft_vst3_host::adopt_scan(cache.vst3);
        soundcraft_clap_host::adopt_scan(cache.clap);
        log::info!("Loaded plugin cache from disk: {nv} VST3 and {nc} CLAP plugins ready");
        return true;
    }
    false
}

pub fn load_plugin_cache() {
    if let Some(path) = plugin_cache_file() {
        load_plugin_cache_from(&path);
    }
}

pub fn save_plugin_cache_to(path: &std::path::Path, vst3: &[soundcraft_vst3_host::Vst3Descriptor], clap: &[soundcraft_clap_host::ClapDescriptor]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cache = PluginsCache { vst3: vst3.to_vec(), clap: clap.to_vec() };
    if let Ok(text) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(path, text);
    }
}

pub fn save_plugin_cache(vst3: &[soundcraft_vst3_host::Vst3Descriptor], clap: &[soundcraft_clap_host::ClapDescriptor]) {
    if let Some(path) = plugin_cache_file() {
        save_plugin_cache_to(&path, vst3, clap);
    }
}

// ---- learning a hosted plugin's parameters -----------------------------------------------------

/// Ids whose parameter list the UI would like to show, and whether a worker is fetching one now.
/// Reading a hosted plugin's parameters means instantiating it — seconds for a bridged Windows
/// plugin under Wine, and forever if Wine is stuck — so it never happens on the UI thread. Controls
/// draw what [`crate::mix_window::plugin_info`] already knows (the catalogue entry) and ask here for
/// the rest; the table fills itself in once the worker has read it.
static LEARN_PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LEARN_BUSY: AtomicBool = AtomicBool::new(false);

/// Asks (once) for a hosted plugin's parameter list. A built-in always has its table, so this is
/// only about `vst3:`/`clap:`/`au:` ids; calling it with one of those is harmless either way.
pub fn learn_plugin_params(id: &str) {
    if !soundcraft_mix::is_third_party(id) || id.contains('/') || id.len() > 512 {
        return;
    }
    let Some(mut pending) = LEARN_PENDING.lock().ok() else { return };
    if pending.iter().any(|p| p == id) || pending.len() >= 64 {
        return;
    }
    pending.push(id.to_string());
    drop(pending);
    // One worker at a time: instantiating plugins is heavy, and under Wine it must not multiply.
    if LEARN_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                LEARN_BUSY.store(false, Ordering::SeqCst);
            }
        }
        let _clear = Clear;
        loop {
            let Some(mut queue) = LEARN_PENDING.lock().ok() else { return };
            let Some(id) = queue.pop() else { return };
            drop(queue);
            let info = if id.starts_with("clap:") {
                soundcraft_clap_host::plugin_info(&id)
            } else if id.starts_with("vst3:") {
                soundcraft_vst3_host::plugin_info(&id)
            } else {
                soundcraft_au_host::plugin_info(&id)
            };
            match info {
                Some(info) => log::info!("parameters read for {id}: {} ({})", info.params.len(), info.name),
                None => log::debug!("no parameter list for {id}"),
            }
        }
    });
}

/// True when a plugin's bundle crashed the scanner and must not be loaded in-process.
pub fn is_broken_bundle(path: &std::path::Path) -> bool {
    let p = path.to_string_lossy();
    BROKEN_BUNDLES.lock().map(|b| b.iter().any(|x| x == p.as_ref())).unwrap_or(false)
}

/// How long one scanner pass may run before it is killed (a hung Wine host must not pin the
/// worker), and how many crashing plugins it may skip before giving up.
const SCAN_CHILD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_SCAN_SKIPS: usize = 24;

struct ChildPass {
    vst3: Vec<soundcraft_vst3_host::Vst3Descriptor>,
    clap: Vec<soundcraft_clap_host::ClapDescriptor>,
    last_scanning: Option<String>,
    ok: bool,
    status: String,
}

/// One scanner child run. `Err` means the child could not start at all (no process); a crash is
/// reported in `ok`/`status` with whatever it printed before dying.
fn run_scanner_child(exe: &std::path::Path, folders: &[String], skipped: &[String]) -> Result<ChildPass, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--scan-plugins");
    for f in folders {
        cmd.arg("--dir").arg(f);
    }
    for s in skipped {
        cmd.arg("--skip").arg(s);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("the scanner could not start: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "the scanner has no output pipe".to_string())?;
    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    // Keep the last lines of stderr for the log: that is where the crash or Wine error shows.
    let err_tail = std::sync::Arc::new(Mutex::new(String::new()));
    let err_slot = std::sync::Arc::clone(&err_tail);
    let err_reader = stderr.map(|err| {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                if let Ok(mut tail) = err_slot.lock() {
                    tail.push_str(&line);
                    tail.push('\n');
                    if tail.len() > 4000 {
                        let cut = tail.len() - 4000;
                        *tail = tail.split_off(cut);
                    }
                }
            }
        })
    });
    let deadline = Instant::now() + SCAN_CHILD_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                if let Some(t) = err_reader {
                    let _ = t.join();
                }
                return Err(format!("the scanner could not be waited for: {e}"));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            if let Some(t) = err_reader {
                let _ = t.join();
            }
            return Ok(ChildPass { vst3: Vec::new(), clap: Vec::new(), last_scanning: None, ok: false, status: "timed out".into() });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = reader.join();
    if let Some(t) = err_reader {
        let _ = t.join();
    }
    let mut pass = ChildPass { vst3: Vec::new(), clap: Vec::new(), last_scanning: None, ok: status.success(), status: status.to_string() };
    for line in rx.try_iter() {
        match serde_json::from_str::<ScannedLine>(&line) {
            Ok(ScannedLine::Vst3(d)) => pass.vst3.push(d),
            Ok(ScannedLine::Clap(d)) => pass.clap.push(d),
            Ok(ScannedLine::Scanning(p)) => pass.last_scanning = Some(p),
            Err(_) => {}
        }
    }
    if let Ok(tail) = err_tail.lock()
        && !tail.is_empty()
    {
        log::warn!("plugin scanner: {tail}");
    }
    Ok(pass)
}

/// Scans through the child process, restarting past plugins that crash it. Each crash names the
/// bundle being loaded, which is skipped on the next pass, so the scan always completes: a broken
/// plugin costs one skipped bundle, never the editor.
fn scan_with_child(folders: &[String]) -> Result<ScanOutcome, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the editor binary: {e}"))?;
    let mut vst3 = Vec::new();
    let mut clap = Vec::new();
    let mut skipped: Vec<String> = load_broken();
    let mut note = None;
    for _ in 0..=MAX_SCAN_SKIPS {
        let pass = run_scanner_child(&exe, folders, &skipped)?;
        vst3.extend(pass.vst3);
        clap.extend(pass.clap);
        if pass.ok {
            if let Ok(mut broken) = BROKEN_BUNDLES.lock() {
                broken.extend(skipped.iter().cloned());
                broken.dedup();
            }
            save_broken(&skipped);
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        }
        let Some(culprit) = pass.last_scanning else {
            note = Some(format!("the scanner stopped ({})", pass.status));
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        };
        if skipped.contains(&culprit) {
            note = Some(format!("the scanner kept crashing on {}", culprit));
            return Ok(ScanOutcome { vst3, clap, note, skipped });
        }
        log::warn!("the plugin scanner crashed while loading {culprit} ({}); skipping that bundle", pass.status);
        skipped.push(culprit);
        note = Some(format!("{} bundle(s) skipped after crashing the scanner", skipped.len()));
    }
    Ok(ScanOutcome { vst3, clap, note, skipped })
}

pub fn spawn_async_sync(custom_path_to_add: Option<String>, run_yabridge_sync: bool) {
    // A request while a scan runs is queued (and its folder registered now), never dropped: the
    // worker honours it as one more pass. The UI thread only touches counters and channels here.
    if let Some(path) = custom_path_to_add {
        let pb = std::path::PathBuf::from(&path);
        if !pb.exists() {
            let _ = std::fs::create_dir_all(&pb);
        }
        soundcraft_vst3_host::register_search_dir(&pb);
        soundcraft_clap_host::register_search_dir(&pb);
        if let Ok(mut pending) = PENDING.lock() {
            pending.0.push(path);
        }
    }
    if run_yabridge_sync && let Ok(mut pending) = PENDING.lock() {
        pending.1 = true;
    }
    if SCAN_BUSY.swap(true, Ordering::SeqCst) {
        SCAN_WAKE.store(true, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        // Clears the busy flag even if the worker panics (a broken bridge or plugin).
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                SCAN_BUSY.store(false, Ordering::SeqCst);
            }
        }
        let _clear = Clear;
        // A folder full of Windows `.dll`/`.vst3` needs yabridge to become loadable. If the
        // bridge tool is missing, say so instead of scanning silently to nothing.
        let have_bridge = std::process::Command::new("yabridgectl").arg("--version").output().is_ok();
        loop {
            let (folders, sync) = PENDING.lock().map(|mut p| (std::mem::take(&mut p.0), std::mem::replace(&mut p.1, false))).unwrap_or_default();
            let mut added_note = String::new();
            let mut bridge_note = String::new();
            if !folders.is_empty() {
                added_note = format!("Folders registered: {}. ", folders.join(", "));
                if have_bridge {
                    for f in &folders {
                        let _ = std::process::Command::new("yabridgectl").args(["add", f]).output();
                    }
                }
            }
            if sync {
                if have_bridge {
                    let _ = std::process::Command::new("yabridgectl").arg("sync").output();
                } else {
                    bridge_note = " (yabridgectl not found: Windows plugins were not bridged)".to_string();
                }
            }
            // Scan in a child process: one plugin that settles its own memory or aborts while
            // being inspected kills the scanner, not the editor. The child prints one plugin per
            // line, so a crash mid-scan keeps everything found so far.
            let mut dirs: Vec<String> = folders.clone();
            for d in soundcraft_vst3_host::extra_dirs() {
                let d = d.to_string_lossy().into_owned();
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
            let msg = match scan_with_child(&dirs) {
                Ok(outcome) => {
                    let broken = outcome.skipped.clone();
                    let kept_v3: Vec<_> = outcome.vst3.into_iter().filter(|d| !broken.iter().any(|b| d.path == std::path::Path::new(b))).collect();
                    let kept_cl: Vec<_> = outcome.clap.into_iter().filter(|d| !broken.iter().any(|b| d.path == std::path::Path::new(b))).collect();
                    let (nv, nc) = (kept_v3.len(), kept_cl.len());
                    soundcraft_vst3_host::adopt_scan(kept_v3.clone());
                    soundcraft_clap_host::adopt_scan(kept_cl.clone());
                    save_plugin_cache(&kept_v3, &kept_cl);
                    let skipped = match &outcome.note {
                        Some(n) => format!(" ({n})"),
                        None => String::new(),
                    };
                    format!("✅ {added_note}Scan complete: {nv} VST3 and {nc} CLAP plugins ready.{skipped}{bridge_note}")
                }
                Err(e) => {
                    // The child could not start (odd packaging). Scan in-process as a last resort:
                    // still off the UI thread, but a crashing plugin could take the app down.
                    let v3 = soundcraft_vst3_host::rescan();
                    let cl = soundcraft_clap_host::rescan();
                    save_plugin_cache(&v3, &cl);
                    format!("⚠️ Scanner helper failed ({e}); scanned in-process: {} VST3, {} CLAP.{bridge_note}", v3.len(), cl.len())
                }
            };
            if let Ok(mut lock) = SCAN_STATUS.lock() {
                *lock = Some(msg);
            }
            if !SCAN_WAKE.swap(false, Ordering::SeqCst) {
                break;
            }
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryFilter {
    All,
    Instruments,
    Effects,
    Vst3,
    Clap,
    BuiltIn,
    Specific(Category),
}

#[derive(Debug, Clone)]
pub struct PluginItem {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub format: &'static str,
    pub category: Category,
    pub is_instrument: bool,
}

/// Collect all currently scanned plugins (Built-in, VST3, CLAP, AU).
pub fn collect_plugins() -> Vec<PluginItem> {
    let mut list = Vec::new();

    // 1. Built-in Native DSP
    for p in soundcraft_dsp::plugins() {
        list.push(PluginItem {
            id: p.id.to_string(),
            name: p.name.to_string(),
            vendor: "SoundCraft".to_string(),
            format: "Native",
            category: p.category,
            is_instrument: p.is_instrument,
        });
    }

    // 2. VST3 Host: the cache only. Scanning happens on a worker thread (`init_vst_paths`,
    // the Scan button): a bridged Windows plugin takes seconds to answer, and the UI must never
    // wait for one.
    let vst3_list = soundcraft_vst3_host::cached().unwrap_or_default();
    for v in vst3_list {
        list.push(PluginItem {
            id: v.id,
            name: v.name,
            vendor: if v.vendor.is_empty() { "Third-Party".to_string() } else { v.vendor },
            format: "VST3",
            category: v.category,
            is_instrument: v.is_instrument,
        });
    }

    // 3. CLAP Host: the cache only, for the same reason.
    let clap_list = soundcraft_clap_host::cached().unwrap_or_default();
    for c in clap_list {
        list.push(PluginItem {
            id: c.id,
            name: c.name,
            vendor: if c.vendor.is_empty() { "Third-Party".to_string() } else { c.vendor },
            format: "CLAP",
            category: c.category,
            is_instrument: c.is_instrument,
        });
    }

    // 4. Audio Units (macOS)
    for au in soundcraft_au_host::scan() {
        list.push(PluginItem {
            id: au.id,
            name: au.name,
            vendor: if au.vendor.is_empty() { "Apple/AU".to_string() } else { au.vendor },
            format: "AU",
            category: Category::Other,
            is_instrument: au.is_instrument,
        });
    }

    list
}

/// Floating FX Browser Window.
pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(target_track_id) = app.ui.show_fx_browser else {
        return;
    };

    let session = app.engine.session();
    let track = match session.track(target_track_id) {
        Some(t) => t.clone(),
        None => {
            app.ui.show_fx_browser = None;
            return;
        }
    };

    let mut open = true;
    let title = format!(
        "🎛 FX: {} — {}",
        track.name,
        match track.kind {
            TrackKind::Instrument => "Instrument Track",
            TrackKind::Audio => "Audio Track",
            TrackKind::Aux => "Aux / Bus Track",
            TrackKind::Master => "Master Fader",
            _ => "Track",
        }
    );

    egui::Window::new(title)
        .id(egui::Id::new("fx_browser_modal_window_v2"))
        .open(&mut open)
        .default_size(vec2(1080.0, 720.0))
        .min_size(vec2(880.0, 600.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            render_fx_browser(app, ui, &track);
        });

    if !open {
        app.ui.show_fx_browser = None;
    }
}

fn render_fx_browser(app: &mut SoundApp, ui: &mut Ui, initial_track: &Track) {
    let t = Tokens::current();
    let track_id = initial_track.id;
    let track_live = app.engine.session().track(track_id).cloned().unwrap_or_else(|| initial_track.clone());
    let track = &track_live;

    // Ephemeral state stored in egui temp memory
    let search_id = egui::Id::new(("fx_browser_search", track_id.0));
    let cat_id = egui::Id::new(("fx_browser_cat", track_id.0));
    let sel_plug_id = egui::Id::new(("fx_browser_sel_plug", track_id.0));
    let target_slot_id = egui::Id::new(("fx_browser_target_slot", track_id.0));

    let mut search_query = ui.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
    let mut selected_cat = ui.data(|d| d.get_temp::<CategoryFilter>(cat_id)).unwrap_or(CategoryFilter::All);
    let mut selected_plugin = ui.data(|d| d.get_temp::<Option<String>>(sel_plug_id)).flatten();
    let mut target_slot =
        ui.data(|d| d.get_temp::<usize>(target_slot_id)).unwrap_or_else(|| track.mixer.inserts.iter().position(Option::is_none).unwrap_or(0));

    let plugins = collect_plugins();

    // Top Header Bar
    ui.horizontal(|ui| {
        let dot_color = egui::Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
        ui.painter().circle_filled(ui.cursor().min + vec2(8.0, 10.0), 5.0, dot_color);
        ui.add_space(18.0);

        let track_list: Vec<(soundcraft_model::TrackId, String, soundcraft_model::TrackKind)> =
            app.engine.session().tracks.iter().filter(|t| !t.hidden).map(|t| (t.id, t.name.clone(), t.kind)).collect();
        egui::ComboBox::from_id_salt("fx_browser_track_selector")
            .selected_text(egui::RichText::new(format!("{} ({:?})", track.name, track.kind)).font(bold(13.0)).color(t.text))
            .show_ui(ui, |ui| {
                for (tid, tname, tkind) in track_list {
                    let icon = match tkind {
                        soundcraft_model::TrackKind::Instrument => "🎹",
                        soundcraft_model::TrackKind::Audio => "🎤",
                        soundcraft_model::TrackKind::Aux => "🎛",
                        soundcraft_model::TrackKind::Master => "🎚",
                        _ => "📁",
                    };
                    if ui.selectable_label(tid == track.id, format!("{icon} {tname} ({tkind:?})")).clicked() {
                        app.ui.show_fx_browser = Some(tid);
                    }
                }
            });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("📁 Plugin Folders…").on_hover_text("Add or remove VST3 and CLAP scan folders (including yabridge's)").clicked() {
                app.ui.show_vst_folders = true;
            }
            if ui.button("🔄 Scan VST3 & CLAP").on_hover_text("Rescan the standard folders in the background").clicked() {
                // Never scan on the UI thread: a bridged plugin takes seconds to answer.
                spawn_async_sync(None, false);
            }

            let num_inserts = track.mixer.inserts.iter().flatten().count() + usize::from(track.instrument.is_some());
            let badge_bg = if num_inserts > 0 { egui::Color32::from_rgb(0, 200, 120) } else { t.text_dim };
            ui.label(egui::RichText::new(format!(" {num_inserts} Active FX ")).background_color(badge_bg).color(Color32::BLACK).font(bold(10.5)));
        });
    });

    ui.separator();

    // Main 3-Column Layout: [Track FX Chain (Left)] | [Categories (Center-Left)] | [Plugin Catalog (Right)]
    let full_rect = ui.available_rect_before_wrap();
    let chain_width = 290.0;
    let cat_width = 165.0;
    let catalog_width = (full_rect.width() - chain_width - cat_width - 24.0).max(300.0);

    ui.horizontal(|ui| {
        // COLUMN 1: Track FX Chain
        ui.vertical(|ui| {
            ui.set_width(chain_width);
            let live_track = app.engine.session().track(track_id).cloned().unwrap_or_else(|| track.clone());
            render_track_chain(app, ui, &live_track, &mut target_slot);
        });

        ui.separator();

        // COLUMN 2: Categories / Formats Filter
        ui.vertical(|ui| {
            ui.set_width(cat_width);
            ui.label(egui::RichText::new("CATEGORIES").font(bold(11.0)).color(t.accent));
            ui.add_space(4.0);

            let all_count = plugins.len();
            let inst_count = plugins.iter().filter(|p| p.is_instrument).count();
            let fx_count = plugins.iter().filter(|p| !p.is_instrument).count();
            let vst3_count = plugins.iter().filter(|p| p.format == "VST3").count();
            let clap_count = plugins.iter().filter(|p| p.format == "CLAP").count();
            let native_count = plugins.iter().filter(|p| p.format == "Native").count();

            for (label, filter, count) in [
                ("🌟 All Plugins", CategoryFilter::All, all_count),
                ("🎹 Instruments", CategoryFilter::Instruments, inst_count),
                ("🎛 Audio FX", CategoryFilter::Effects, fx_count),
                ("⚡ VST3", CategoryFilter::Vst3, vst3_count),
                ("🔧 CLAP", CategoryFilter::Clap, clap_count),
                ("📦 Native DSP", CategoryFilter::BuiltIn, native_count),
            ] {
                let sel = selected_cat == filter;
                if ui.selectable_label(sel, format!("{label} ({count})")).clicked() {
                    selected_cat = filter;
                }
            }

            ui.separator();
            ui.label(egui::RichText::new("FX TYPES").font(bold(10.0)).color(t.text_dim));

            for c in [
                Category::Dynamics,
                Category::Eq,
                Category::Reverb,
                Category::Delay,
                Category::Modulation,
                Category::Harmonic,
                Category::PitchShift,
                Category::Other,
            ] {
                let count = plugins.iter().filter(|p| p.category == c && !p.is_instrument).count();
                let filter = CategoryFilter::Specific(c);
                let sel = selected_cat == filter;
                if ui.selectable_label(sel, format!("{c:?} ({count})")).clicked() {
                    selected_cat = filter;
                }
            }
        });

        ui.separator();

        // COLUMN 3: Plugin Catalog & Search Table
        ui.vertical(|ui| {
            ui.set_width(catalog_width);

            // Search Bar
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add(
                    egui::TextEdit::singleline(&mut search_query)
                        .hint_text("Search by plugin name, vendor, format...")
                        .desired_width(catalog_width - 30.0),
                );
            });

            ui.add_space(4.0);

            // Filter plugins list
            let filtered: Vec<&PluginItem> = plugins
                .iter()
                .filter(|p| {
                    // Category filter
                    let cat_match = match selected_cat {
                        CategoryFilter::All => true,
                        CategoryFilter::Instruments => p.is_instrument,
                        CategoryFilter::Effects => !p.is_instrument,
                        CategoryFilter::Vst3 => p.format == "VST3",
                        CategoryFilter::Clap => p.format == "CLAP",
                        CategoryFilter::BuiltIn => p.format == "Native",
                        CategoryFilter::Specific(cat) => p.category == cat && !p.is_instrument,
                    };
                    if !cat_match {
                        return false;
                    }
                    // Search query match
                    if search_query.is_empty() {
                        return true;
                    }
                    let q = search_query.to_lowercase();
                    p.name.to_lowercase().contains(&q)
                        || p.vendor.to_lowercase().contains(&q)
                        || p.format.to_lowercase().contains(&q)
                        || format!("{:?}", p.category).to_lowercase().contains(&q)
                })
                .collect();

            retain_visible_selection(&mut selected_plugin, &filtered);
            ui.label(egui::RichText::new(format!("Showing {} plugins", filtered.len())).color(t.text_dim).font(regular(9.5)));

            // Table / List of plugins
            let scroll_height = (ui.available_height() - 55.0).max(180.0);
            egui::ScrollArea::vertical().max_height(scroll_height).show(ui, |ui| {
                for plug in &filtered {
                    let is_sel = selected_plugin.as_deref() == Some(&plug.id);
                    let mut load_clicked = false;
                    let row_resp = ui.horizontal(|ui| {
                        let icon = if plug.is_instrument { "🎹" } else { "🎛" };
                        ui.label(icon);

                        // Format Badge
                        let (badge_bg, badge_fg) = match plug.format {
                            "VST3" => (Color32::from_rgb(20, 60, 120), Color32::from_rgb(100, 200, 255)),
                            "CLAP" => (Color32::from_rgb(100, 50, 10), Color32::from_rgb(255, 170, 70)),
                            _ => (Color32::from_rgb(20, 70, 50), Color32::from_rgb(80, 240, 180)),
                        };
                        ui.painter().rect_filled(Rect::from_min_size(ui.cursor().min + vec2(0.0, 2.0), vec2(42.0, 14.0)), 2.0, badge_bg);
                        ui.label(egui::RichText::new(format!(" {} ", plug.format)).color(badge_fg).font(bold(9.0)));

                        let name_txt = egui::RichText::new(&plug.name).font(bold(11.0)).color(if is_sel { t.accent } else { t.text });
                        ui.label(name_txt);

                        ui.label(egui::RichText::new(format!("· {}", plug.vendor)).color(t.text_dim).font(regular(10.0)));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let btn_label = if plug.is_instrument { "➕ Carregar" } else { "➕ Inserir" };
                            let btn_col = if is_sel { Color32::from_rgb(0, 220, 130) } else { t.accent };
                            if ui.button(egui::RichText::new(btn_label).font(bold(9.5)).color(btn_col)).clicked() {
                                load_clicked = true;
                            }
                        });
                    });

                    let r = ui.interact(row_resp.response.rect, ui.id().with(("fx_browser_plugin", &plug.id)), Sense::click());
                    if r.clicked() {
                        selected_plugin = Some(plug.id.clone());
                    }
                    if r.double_clicked() || load_clicked {
                        selected_plugin = Some(plug.id.clone());
                        if let Some(next) = add_plugin_to_track(app, track, plug, target_slot) {
                            target_slot = next;
                        }
                    }
                    ui.separator();
                }
            });

            // Action footer for catalog
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(sel_id) = &selected_plugin
                    && let Some(p) = filtered.iter().find(|p| p.id == *sel_id)
                {
                    if p.is_instrument {
                        if track.kind == TrackKind::Instrument {
                            let action_label = format!("➕ Load Instrument [{}] on [{}]", p.name, track.name);
                            if ui.button(egui::RichText::new(action_label).font(bold(11.5)).color(Color32::WHITE)).clicked()
                                && let Some(next) = add_plugin_to_track(app, track, p, target_slot)
                            {
                                target_slot = next;
                            }
                        } else {
                            let action_label = format!("➕ Convert [{}] to Instrument & Load [{}]", track.name, p.name);
                            if ui.button(egui::RichText::new(action_label).font(bold(11.5)).color(Color32::from_rgb(0, 220, 130))).clicked()
                                && let Some(next) = add_plugin_to_track(app, track, p, target_slot)
                            {
                                target_slot = next;
                            }
                            let other_inst = app
                                .engine
                                .session()
                                .tracks
                                .iter()
                                .find(|t| t.kind == TrackKind::Instrument && !t.hidden && t.id != track.id)
                                .cloned();
                            if let Some(it) = other_inst {
                                let quick_label = format!("🎹 Load on [{}]", it.name);
                                if ui.button(egui::RichText::new(quick_label).font(bold(11.0)).color(t.accent)).clicked() {
                                    app.ui.show_fx_browser = Some(it.id);
                                    add_plugin_to_track(app, &it, p, 0);
                                }
                            }
                        }
                    } else {
                        let letter = (b'A' + target_slot.min(9) as u8) as char;
                        let action_label = format!("➕ Insert [{}] into Slot {}", p.name, letter);
                        if ui.button(egui::RichText::new(action_label).font(bold(11.5)).color(Color32::WHITE)).clicked()
                            && let Some(next) = add_plugin_to_track(app, track, p, target_slot)
                        {
                            target_slot = next;
                        }
                    }
                } else {
                    ui.label(egui::RichText::new("Select a plugin or double-click to insert").italics().color(t.text_dim));
                }
            });
        });
    });

    // Save ephemeral state
    ui.data_mut(|d| {
        d.insert_temp(search_id, search_query);
        d.insert_temp(cat_id, selected_cat);
        d.insert_temp(sel_plug_id, selected_plugin);
        d.insert_temp(target_slot_id, target_slot);
    });
}

fn retain_visible_selection(selected_plugin: &mut Option<String>, filtered: &[&PluginItem]) {
    if selected_plugin.as_deref().is_some_and(|selected| !filtered.iter().any(|plugin| plugin.id == selected)) {
        *selected_plugin = None;
    }
}

fn plugin_title(id: &str) -> String {
    if let Some(info) = soundcraft_dsp::plugin_info(id) {
        return info.name.to_string();
    }
    if let Some(v) = soundcraft_vst3_host::cached()
        && let Some(d) = v.iter().find(|x| x.id == id)
    {
        return d.name.clone();
    }
    if let Some(c) = soundcraft_clap_host::cached()
        && let Some(d) = c.iter().find(|x| x.id == id)
    {
        return d.name.clone();
    }
    id.to_string()
}

fn render_track_chain(app: &mut SoundApp, ui: &mut Ui, track: &Track, target_slot: &mut usize) {
    let t = Tokens::current();
    let track_id = track.id;
    // Third-party plugins the audio side has not loaded (yet, or refuses to), by slot: without
    // this a bridged Windows plugin that never came up just left an empty-looking slot.
    let load_status: Vec<(usize, soundcraft_playback::ProbeState)> = app
        .player
        .as_ref()
        .map(|p| p.plugin_load_status().into_iter().filter(|(tr, _, _, _)| *tr == track_id).map(|(_, slot, _, state)| (slot, state)).collect())
        .unwrap_or_default();

    ui.label(egui::RichText::new("TRACK FX CHAIN").font(bold(11.0)).color(t.accent));
    ui.add_space(4.0);

    let scroll_h = ui.available_height();
    egui::ScrollArea::vertical().id_salt("track_fx_chain_scroll").auto_shrink([false, false]).max_height(scroll_h).show(ui, |ui| {
        // 1. Instrument Slot (if Instrument Track or has an instrument)
        if track.kind == TrackKind::Instrument || track.instrument.is_some() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🎹 INSTRUMENT").font(bold(9.5)).color(t.accent));
                    if track.instrument.is_some() {
                        if ui.small_button("GUI").clicked() {
                            crate::mix_window::open_native_gui(app, track_id, soundcraft_mix::INSTRUMENT_SLOT);
                            if let Some(inst) = &track.instrument
                                && !soundcraft_mix::is_third_party(&inst.plugin)
                                && !app.ui.plugin_windows.contains(&(track_id, soundcraft_mix::INSTRUMENT_SLOT))
                            {
                                app.ui.plugin_windows.push((track_id, soundcraft_mix::INSTRUMENT_SLOT));
                            }
                        }
                        if ui.small_button("✕").on_hover_text("Remove Instrument").clicked()
                            && let Some(tr) = app.engine.session_mut().track_mut(track_id)
                        {
                            tr.instrument = None;
                        }
                    }
                });

                if let Some(inst) = &track.instrument {
                    let name = plugin_title(&inst.plugin);
                    ui.label(egui::RichText::new(&name).font(bold(11.0)).color(Color32::WHITE));
                    if let Some((_, state)) = load_status
                        .iter()
                        .find(|(slot, state)| *slot == soundcraft_mix::INSTRUMENT_SLOT && *state != soundcraft_playback::ProbeState::Loadable)
                    {
                        let color = match state {
                            soundcraft_playback::ProbeState::Refused => t.rec,
                            _ => t.text_dim,
                        };
                        ui.label(egui::RichText::new(state.describe()).italics().color(color).font(regular(9.5)));
                    }
                } else {
                    ui.label(egui::RichText::new("(Empty — select an instrument from list)").italics().color(t.text_dim));
                }
            });
            ui.add_space(4.0);
        }

        // 2. Inserts A through J
        ui.label(egui::RichText::new("INSERTS (A–J)").font(bold(10.0)).color(t.text_dim));

        for slot in 0..10 {
            let letter = (b'A' + slot as u8) as char;
            let is_targeted = *target_slot == slot;
            let ins = track.mixer.inserts.get(slot).cloned().flatten();

            let border_color = if is_targeted { t.accent } else { t.border };
            let bg_color = if is_targeted { t.toolbar_group } else { t.panel_bg };

            egui::Frame::new()
                .fill(bg_color)
                .stroke(Stroke::new(if is_targeted { 1.5 } else { 1.0 }, border_color))
                .corner_radius(3.0)
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Slot letter indicator
                        let l_color = if ins.is_some() { egui::Color32::from_rgb(0, 220, 130) } else { t.text_dim };
                        ui.label(egui::RichText::new(format!("[{letter}]")).font(bold(10.0)).color(l_color));

                        match &ins {
                            Some(insert) => {
                                let name = plugin_title(&insert.plugin);
                                let name_color = if insert.bypass { t.text_dim } else { t.text };

                                if ui.selectable_label(is_targeted, egui::RichText::new(&name).font(bold(10.5)).color(name_color)).clicked() {
                                    *target_slot = slot;
                                }
                                if let Some((_, state)) =
                                    load_status.iter().find(|(s, state)| *s == slot && *state != soundcraft_playback::ProbeState::Loadable)
                                {
                                    let color = match state {
                                        soundcraft_playback::ProbeState::Refused => t.rec,
                                        _ => t.text_dim,
                                    };
                                    ui.label(egui::RichText::new(state.describe()).italics().color(color).font(regular(9.5)));
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Remove Insert").clicked() {
                                        let _ = app.run("mix.insert_remove", json!({"track": track_id.0, "slot": slot}));
                                    }
                                    if ui.small_button("GUI").on_hover_text("Open Parameters Window").clicked() {
                                        crate::mix_window::open_native_gui(app, track_id, slot);
                                        if !soundcraft_mix::is_third_party(&insert.plugin) && !app.ui.plugin_windows.contains(&(track_id, slot)) {
                                            app.ui.plugin_windows.push((track_id, slot));
                                        }
                                    }
                                    let bp_txt = if insert.bypass { "ON" } else { "BYP" };
                                    if ui.small_button(bp_txt).on_hover_text("Toggle Bypass").clicked() {
                                        let _ = app.run("mix.insert_bypass", json!({"track": track_id.0, "slot": slot}));
                                    }
                                });
                            }
                            None => {
                                if ui.selectable_label(is_targeted, egui::RichText::new("(Empty)").italics().color(t.text_dim)).clicked() {
                                    *target_slot = slot;
                                }
                            }
                        }
                    });
                });

            ui.add_space(2.0);
        }
    });
}

fn add_plugin_to_track(app: &mut SoundApp, track: &Track, plug: &PluginItem, target_slot: usize) -> Option<usize> {
    let track_id = track.id;
    let third_party = soundcraft_mix::is_third_party(&plug.id);

    let should_rename = plug.is_instrument
        || track.name.starts_with("Track")
        || track.name.starts_with("Audio")
        || track.name.starts_with("Inst")
        || track.name.is_empty();

    let mut next_slot = None;
    let result = if plug.is_instrument {
        if track.kind != TrackKind::Instrument
            && let Some(tr) = app.engine.session_mut().track_mut(track_id)
        {
            tr.kind = TrackKind::Instrument;
            tr.view = "notes".to_string();
        }
        if should_rename && let Some(tr) = app.engine.session_mut().track_mut(track_id) {
            tr.name = plug.name.clone();
        }
        app.run("mix.instrument", json!({"track": track_id.0, "plugin": plug.id})).map(|_| {
            if let Some(p) = &app.player {
                p.update_session(app.engine.session_arc());
            }
            if third_party && app.player.is_some() {
                app.pending_gui = Some((track_id, soundcraft_mix::INSTRUMENT_SLOT));
            }
            crate::mix_window::open_native_gui(app, track_id, soundcraft_mix::INSTRUMENT_SLOT);
            format!("Loaded instrument `{}` on {}", plug.name, track.name)
        })
    } else {
        let slot = target_slot.min(9);
        if should_rename && let Some(tr) = app.engine.session_mut().track_mut(track_id) {
            tr.name = plug.name.clone();
        }
        app.run("mix.insert", json!({"track": track_id.0, "slot": slot, "plugin": plug.id})).map(|_| {
            if let Some(p) = &app.player {
                p.update_session(app.engine.session_arc());
            }
            if third_party && app.player.is_some() {
                app.pending_gui = Some((track_id, slot));
            }
            crate::mix_window::open_native_gui(app, track_id, slot);
            let letter = (b'A' + slot as u8) as char;
            // Auto-advance target_slot to next empty slot
            let next_empty =
                app.engine.session().track(track_id).and_then(|t| t.mixer.inserts.iter().position(Option::is_none)).unwrap_or(slot + 1).min(9);
            next_slot = Some(next_empty);

            format!("Inserted `{}` on {} [Slot {}]", plug.name, track.name, letter)
        })
    };
    match result {
        Ok(message) => app.ui.status = message,
        Err(error) => app.ui.status = error,
    }
    next_slot
}

/// Persistent VST & CLAP search paths configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct VstPathsConfig {
    pub custom_paths: Vec<String>,
}

impl VstPathsConfig {
    pub fn config_path() -> Option<std::path::PathBuf> {
        if let Some(home) = std::env::var_os("HOME") {
            let p = std::path::PathBuf::from(home).join(".config/soundcraft/vst_paths.json");
            return Some(p);
        }
        Some(std::path::PathBuf::from("vst_paths.json"))
    }

    pub fn load() -> Self {
        if let Some(path) = Self::config_path()
            && path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(cfg) = serde_json::from_str::<Self>(&content)
        {
            return cfg;
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// Registers the saved folders with the plugin hosts. It only records the paths: the scan
    /// itself walks the folders (and may dlopen bridged plugins), so it must never run on the UI
    /// thread — the caller starts `spawn_async_sync`, which scans on a worker.
    pub fn apply_to_hosts(&self) {
        for p in &self.custom_paths {
            let pb = std::path::PathBuf::from(p);
            if pb.exists() {
                soundcraft_vst3_host::register_search_dir(&pb);
                soundcraft_clap_host::register_search_dir(&pb);
            }
        }
    }
}

/// Initialises the plugin search paths at launch. The scan itself runs on a worker thread: a
/// Wine-hosted plugin can take seconds to answer, and startup must not wait for it.
pub fn init_vst_paths() {
    let cfg = VstPathsConfig::load();
    cfg.apply_to_hosts();
    if let Some(home) = std::env::var_os("HOME") {
        let h = std::path::PathBuf::from(home);
        let _ = std::fs::create_dir_all(h.join(".vst3"));
        let _ = std::fs::create_dir_all(h.join(".clap"));
    }
    load_plugin_cache();
    spawn_async_sync(None, false);
}

/// Helper to pick folder via app service or fallback to zenity CLI on Linux
pub fn pick_folder_dialog(app: &SoundApp) -> Option<String> {
    if let Some(pick) = &app.services.pick_folder {
        let start = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        if let Some(path) = pick(&start) {
            return Some(path);
        }
    }
    // Fallback: try zenity on Linux (always available on standard GTK/GNOME desktop)
    if let Ok(output) =
        std::process::Command::new("zenity").args(["--file-selection", "--directory", "--title=Choose a VST3 / CLAP plugin folder"]).output()
        && output.status.success()
    {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

/// Dedicated VST & CLAP Plugin Folders Management Window (FL Studio Plugin Manager style).
pub fn show_vst_paths_window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_vst_folders {
        return;
    }

    let mut open = app.ui.show_vst_folders;

    egui::Window::new("🎛 Gerenciador de Plugins (FL Studio Plugin Manager)")
        .open(&mut open)
        .default_size(vec2(980.0, 620.0))
        .min_size(vec2(760.0, 480.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            render_vst_paths_ui(app, ui);
        });

    app.ui.show_vst_folders = open;
}

fn render_vst_paths_ui(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();

    let new_path_id = egui::Id::new("vst_paths_new_input");
    let status_id = egui::Id::new("vst_paths_status_msg");
    let search_id = egui::Id::new("vst_mgr_search_query");
    let cat_id = egui::Id::new("vst_mgr_cat_filter");

    let mut new_path = ui.data(|d| d.get_temp::<String>(new_path_id)).unwrap_or_default();
    let status_msg = ui.data(|d| d.get_temp::<String>(status_id));
    let mut search_query = ui.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
    let mut selected_cat = ui.data(|d| d.get_temp::<CategoryFilter>(cat_id)).unwrap_or(CategoryFilter::All);

    let mut cfg = VstPathsConfig::load();
    let mut changed = false;

    let is_busy = is_scanning();
    if is_busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if let Some(fresh_msg) = take_scan_status() {
        ui.data_mut(|d| d.insert_temp(status_id, fresh_msg));
    }

    // Top Action Bar
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("FL Studio Plugin Manager").font(bold(16.0)).color(t.accent));
        ui.label(egui::RichText::new("— scan and catalogue of VST3, CLAP and built-in DSP").color(t.text_dim));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let scan_btn = egui::Button::new(egui::RichText::new("🔍 Find Plugins").strong().color(Color32::BLACK)).fill(t.accent);
            if ui.add_enabled(!is_busy, scan_btn).on_hover_text("Rescan every standard and custom folder for VST3 and CLAP plugins").clicked() {
                cfg.apply_to_hosts();
                spawn_async_sync(None, false);
                ui.data_mut(|d| d.insert_temp(status_id, "⏳ Scanning for plugins in the background…".to_string()));
            }

            ui.add_space(4.0);

            let sync_btn =
                egui::Button::new(egui::RichText::new("🌉 Sync Windows VSTs (yabridge)").strong().color(Color32::BLACK)).fill(t.meter_yellow);
            let sync_hint = "Runs yabridgectl sync to refresh the Windows plugins (Kontakt, Omnisphere...) and reloads the catalogue";
            if ui.add_enabled(!is_busy, sync_btn).on_hover_text(sync_hint).clicked() {
                cfg.apply_to_hosts();
                spawn_async_sync(None, true);
                ui.data_mut(|d| d.insert_temp(status_id, "⏳ Syncing Windows plugins (Wine + yabridge) in the background…".to_string()));
            }
        });
    });

    if is_busy {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spinner();
            let busy = egui::RichText::new("⏳ Scanning and syncing plugins in the background (Wine / yabridge)…");
            ui.label(busy.color(Color32::from_rgb(255, 200, 50)).strong());
        });
    }

    if let Some(msg) = &status_msg {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(msg).color(egui::Color32::from_rgb(0, 220, 130)).strong());
    }

    ui.separator();

    // Two-Column FL Studio Layout: [Search Paths (Left 380px)] | [Installed Plugins (Right)]
    let avail = ui.available_size();
    let left_w = 380.0;
    let right_w = (avail.x - left_w - 20.0).max(360.0);
    let body_h = (avail.y - 35.0).max(380.0);

    ui.horizontal(|ui| {
        // COLUMN 1: Search Paths
        ui.vertical(|ui| {
            ui.set_width(left_w);
            ui.set_height(body_h);

            ui.label(egui::RichText::new("PASTAS DE BUSCA (Plugin search paths)").font(bold(12.0)).color(t.accent));
            ui.label(egui::RichText::new("O SoundCraft varre estes diretórios em busca de plugins:").small().color(t.text_dim));
            ui.add_space(4.0);

            // Action Buttons
            ui.horizontal(|ui| {
                let pick_btn = egui::Button::new(egui::RichText::new("📂 Choose Folder…").strong());
                if ui.add_enabled(!is_busy, pick_btn).on_hover_text("Opens the system file dialog to pick any plugin folder").clicked()
                    && let Some(chosen) = pick_folder_dialog(app)
                {
                    if !cfg.custom_paths.contains(&chosen) {
                        cfg.custom_paths.push(chosen.clone());
                        changed = true;
                        cfg.save();
                        spawn_async_sync(Some(chosen), true);
                        ui.data_mut(|d| d.insert_temp(status_id, "⏳ Registering the folder and syncing in the background…".to_string()));
                    } else {
                        ui.data_mut(|d| d.insert_temp(status_id, format!("ℹ️ `{chosen}` is already in the list.")));
                    }
                }
            });

            ui.add_space(2.0);
            let home_str = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            ui.horizontal(|ui| {
                if ui.add_enabled(!is_busy, egui::Button::new("➕ ~/.vst3")).clicked() {
                    let p = format!("{home_str}/.vst3");
                    if !cfg.custom_paths.contains(&p) {
                        cfg.custom_paths.push(p.clone());
                        changed = true;
                        cfg.save();
                        spawn_async_sync(Some(p), false);
                        ui.data_mut(|d| d.insert_temp(status_id, "⏳ Varrendo ~/.vst3 em segundo plano...".to_string()));
                    }
                }
                if ui.add_enabled(!is_busy, egui::Button::new("➕ /usr/lib/vst3")).clicked() {
                    let p = "/usr/lib/vst3".to_string();
                    if !cfg.custom_paths.contains(&p) {
                        cfg.custom_paths.push(p.clone());
                        changed = true;
                        cfg.save();
                        spawn_async_sync(Some(p), false);
                        ui.data_mut(|d| d.insert_temp(status_id, "⏳ Varrendo /usr/lib/vst3 em segundo plano...".to_string()));
                    }
                }
                if ui.add_enabled(!is_busy, egui::Button::new("➕ ~/.clap")).clicked() {
                    let p = format!("{home_str}/.clap");
                    if !cfg.custom_paths.contains(&p) {
                        cfg.custom_paths.push(p.clone());
                        changed = true;
                        cfg.save();
                        spawn_async_sync(Some(p), false);
                        ui.data_mut(|d| d.insert_temp(status_id, "⏳ Varrendo ~/.clap em segundo plano...".to_string()));
                    }
                }
            });

            ui.add_space(4.0);

            // Manual Text Entry
            ui.horizontal(|ui| {
                ui.add_sized(vec2(left_w - 90.0, 22.0), egui::TextEdit::singleline(&mut new_path).hint_text("Or type a folder path…"));
                let add_btn = egui::Button::new("➕ Add");
                if ui.add_enabled(!is_busy, add_btn).clicked() {
                    let trimmed = new_path.trim().to_string();
                    if trimmed.is_empty() {
                        ui.data_mut(|d| d.insert_temp(status_id, "⚠️ Type a folder path, or use \"📂 Choose Folder…\"".to_string()));
                    } else if cfg.custom_paths.contains(&trimmed) {
                        ui.data_mut(|d| d.insert_temp(status_id, format!("ℹ️ `{trimmed}` is already in the list.")));
                    } else {
                        cfg.custom_paths.push(trimmed.clone());
                        changed = true;
                        cfg.save();
                        spawn_async_sync(Some(trimmed), true);
                        let note = "⏳ Registering the folder and syncing Windows plugins (Wine / yabridge) in the background…";
                        ui.data_mut(|d| d.insert_temp(status_id, note.to_string()));
                        new_path.clear();
                    }
                }
            });

            ui.add_space(6.0);

            // List of All Active Search Paths
            let mut remove_idx: Option<usize> = None;
            egui::ScrollArea::vertical().id_salt("vst_paths_scroll").max_height(body_h - 140.0).show(ui, |ui| {
                let sys_paths = [
                    (format!("{home_str}/.vst3/yabridge"), "Windows Bridge (Yabridge)"),
                    (format!("{home_str}/.vst3"), "VST3 (Usuário)"),
                    ("/usr/lib/vst3".to_string(), "VST3 (Sistema)"),
                    ("/usr/local/lib/vst3".to_string(), "VST3 (Local)"),
                    (format!("{home_str}/.clap"), "CLAP (Usuário)"),
                    ("/usr/lib/clap".to_string(), "CLAP (Sistema)"),
                ];

                for (p, tag) in sys_paths {
                    let exists = std::path::Path::new(&p).exists();
                    ui.horizontal(|ui| {
                        let badge_bg = if exists { egui::Color32::from_rgb(0, 180, 100) } else { t.text_dim };
                        ui.label(egui::RichText::new(format!(" [{tag}] ")).background_color(badge_bg).color(Color32::BLACK).font(bold(9.0)));
                        ui.label(egui::RichText::new(&p).monospace().color(if exists { t.text } else { t.text_dim }));
                        if exists {
                            ui.label(egui::RichText::new("✓").color(egui::Color32::from_rgb(0, 220, 130)).strong());
                        }
                    });
                    ui.add_space(2.0);
                }

                if !cfg.custom_paths.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("Custom folders:").font(bold(10.5)).color(t.accent));
                    for (idx, p) in cfg.custom_paths.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let exists = std::path::Path::new(p).exists();
                            ui.label(egui::RichText::new(" [CUSTOM] ").background_color(t.accent).color(Color32::BLACK).font(bold(9.0)));
                            ui.label(egui::RichText::new(p).monospace().color(if exists { t.text } else { t.text_dim }));
                            if !exists {
                                ui.label(egui::RichText::new("⚠️").color(t.meter_yellow));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("✕").on_hover_text("Remove folder").clicked() {
                                    remove_idx = Some(idx);
                                }
                            });
                        });
                        ui.add_space(2.0);
                    }
                }
            });

            if let Some(idx) = remove_idx
                && idx < cfg.custom_paths.len()
            {
                cfg.custom_paths.remove(idx);
                changed = true;
                cfg.save();
                ui.data_mut(|d| d.insert_temp(status_id, "Folder removed from the scan list.".to_string()));
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.small_button("📂 Open ~/.vst3").clicked() {
                    let p = format!("{home_str}/.vst3");
                    let _ = std::fs::create_dir_all(&p);
                    let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
                }
                if ui.small_button("📂 Open ~/.clap").clicked() {
                    let p = format!("{home_str}/.clap");
                    let _ = std::fs::create_dir_all(&p);
                    let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
                }
            });
        });

        ui.separator();

        // COLUMN 2: Installed Plugins Catalog
        ui.vertical(|ui| {
            ui.set_width(right_w);
            ui.set_height(body_h);

            ui.label(egui::RichText::new("PLUGINS INSTALADOS (Installed plugins)").font(bold(12.0)).color(t.accent));

            // Search Bar & Filters
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add_sized(
                    vec2(right_w - 30.0, 24.0),
                    egui::TextEdit::singleline(&mut search_query).hint_text("Filtrar por nome do plugin, desenvolvedor ou formato..."),
                );
            });

            ui.add_space(4.0);

            let all_plugins = collect_plugins();
            let all_count = all_plugins.len();
            let inst_count = all_plugins.iter().filter(|p| p.is_instrument).count();
            let fx_count = all_plugins.iter().filter(|p| !p.is_instrument).count();
            let v3_count = all_plugins.iter().filter(|p| p.format == "VST3").count();
            let cl_count = all_plugins.iter().filter(|p| p.format == "CLAP").count();
            let nat_count = all_plugins.iter().filter(|p| p.format == "Native").count();

            // Filter Chips
            ui.horizontal_wrapped(|ui| {
                for (lbl, filter, count) in [
                    ("🌟 Todos", CategoryFilter::All, all_count),
                    ("🎹 Instrumentos", CategoryFilter::Instruments, inst_count),
                    ("🎛 Efeitos", CategoryFilter::Effects, fx_count),
                    ("⚡ VST3", CategoryFilter::Vst3, v3_count),
                    ("🔧 CLAP", CategoryFilter::Clap, cl_count),
                    ("📦 DSP Nativo", CategoryFilter::BuiltIn, nat_count),
                ] {
                    let sel = selected_cat == filter;
                    if ui.selectable_label(sel, format!("{lbl} ({count})")).clicked() {
                        selected_cat = filter;
                    }
                }
            });

            ui.add_space(6.0);

            // Filtered list
            let query = search_query.trim().to_lowercase();
            let filtered: Vec<&PluginItem> = all_plugins
                .iter()
                .filter(|p| {
                    if !query.is_empty() {
                        let matches_name = p.name.to_lowercase().contains(&query);
                        let matches_vendor = p.vendor.to_lowercase().contains(&query);
                        let matches_fmt = p.format.to_lowercase().contains(&query);
                        if !matches_name && !matches_vendor && !matches_fmt {
                            return false;
                        }
                    }
                    match &selected_cat {
                        CategoryFilter::All => true,
                        CategoryFilter::Instruments => p.is_instrument,
                        CategoryFilter::Effects => !p.is_instrument,
                        CategoryFilter::Vst3 => p.format == "VST3",
                        CategoryFilter::Clap => p.format == "CLAP",
                        CategoryFilter::BuiltIn => p.format == "Native",
                        CategoryFilter::Specific(c) => p.category == *c,
                    }
                })
                .collect();

            // Table Header
            egui::Frame::new().fill(t.panel_bg2).stroke(Stroke::new(1.0, t.group_border)).inner_margin(4.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("STATUS").font(bold(10.0)).color(t.text_dim));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("NOME DO PLUGIN").font(bold(10.0)).color(t.text_dim));
                    ui.add_space(160.0);
                    ui.label(egui::RichText::new("CATEGORIA").font(bold(10.0)).color(t.text_dim));
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("FORMATO").font(bold(10.0)).color(t.text_dim));
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new("FABRICANTE / VENDOR").font(bold(10.0)).color(t.text_dim));
                });
            });

            // Table Body
            egui::ScrollArea::vertical().id_salt("installed_plugins_scroll").max_height(body_h - 110.0).show(ui, |ui| {
                if filtered.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No plugin matches this filter.").color(t.text_dim).italics());
                        let tip = egui::RichText::new("Tip: use the \"📂 Choose Folder…\" button on the left to point at your VST folder.");
                        ui.label(tip.color(t.accent).small());
                    });
                } else {
                    for p in &filtered {
                        ui.horizontal(|ui| {
                            // Status
                            ui.label(egui::RichText::new("✓ OK").color(egui::Color32::from_rgb(0, 220, 130)).font(bold(9.5)));
                            ui.add_space(12.0);

                            // Name
                            let name_lbl = ui.label(egui::RichText::new(&p.name).font(bold(11.0)).color(t.text));
                            if name_lbl.hovered() {
                                name_lbl.on_hover_text(format!("ID: {}\nVendor: {}\nFormat: {}", p.id, p.vendor, p.format));
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Vendor
                                ui.label(egui::RichText::new(&p.vendor).small().color(t.text_dim));
                                ui.add_space(8.0);

                                // Format Badge
                                let fmt_col = match p.format {
                                    "VST3" => t.accent,
                                    "CLAP" => t.meter_yellow,
                                    _ => egui::Color32::from_rgb(0, 220, 130),
                                };
                                let badge = egui::RichText::new(format!(" [{}] ", p.format));
                                ui.label(badge.background_color(fmt_col).color(Color32::BLACK).font(bold(9.0)));
                                ui.add_space(8.0);

                                // Category
                                let cat_text = if p.is_instrument {
                                    "Synth/Inst"
                                } else {
                                    match p.category {
                                        Category::Dynamics => "Dynamics",
                                        Category::Eq => "EQ",
                                        Category::Reverb => "Reverb",
                                        Category::Delay => "Delay",
                                        Category::Modulation => "Modulation",
                                        Category::Harmonic => "Harmonic",
                                        Category::PitchShift => "Pitch",
                                        _ => "FX",
                                    }
                                };
                                ui.label(egui::RichText::new(cat_text).color(t.text_dim).small());
                            });
                        });
                        ui.separator();
                    }
                }
            });
        });
    });

    ui.separator();

    // Footer Summary Bar
    let plugins = collect_plugins();
    let v3_count = plugins.iter().filter(|p| p.format == "VST3").count();
    let cl_count = plugins.iter().filter(|p| p.format == "CLAP").count();
    let nat_count = plugins.iter().filter(|p| p.format == "Native").count();
    let inst_count = plugins.iter().filter(|p| p.is_instrument).count();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Total: {} plugins verificados", plugins.len())).strong().color(t.text));
        ui.separator();
        ui.label(egui::RichText::new(format!("⚡ VST3: {v3_count}")).color(t.accent).strong());
        ui.separator();
        ui.label(egui::RichText::new(format!("🔧 CLAP: {cl_count}")).color(t.meter_yellow).strong());
        ui.separator();
        ui.label(egui::RichText::new(format!("📦 DSP Nativo: {nat_count}")).color(egui::Color32::from_rgb(0, 220, 130)).strong());
        ui.separator();
        ui.label(egui::RichText::new(format!("🎹 Instrumentos/Sintetizadores: {inst_count}")).color(t.text));
    });

    if changed {
        cfg.save();
    }
    ui.data_mut(|d| d.insert_temp(new_path_id, new_path));
    ui.data_mut(|d| d.insert_temp(search_id, search_query));
    ui.data_mut(|d| d.insert_temp(cat_id, selected_cat));
}

#[cfg(test)]
mod scanner_tests {
    #[test]
    fn plugin_cache_saves_and_loads_from_disk() {
        let path = std::env::temp_dir().join("soundcraft_test_cache.json");

        let vst = soundcraft_vst3_host::Vst3Descriptor {
            id: "vst3:test1234567890123456789012345678".into(),
            class_id: "test1234567890123456789012345678".into(),
            name: "Cached Synth".into(),
            vendor: "TestCorp".into(),
            version: "1.0.0".into(),
            sdk_version: "VST 3.7.4".into(),
            sub_categories: vec!["Instrument".into()],
            category: Category::Instrument,
            is_instrument: true,
            path: std::path::PathBuf::from("/tmp/test.vst3"),
        };

        super::save_plugin_cache_to(&path, std::slice::from_ref(&vst), &[]);
        assert!(super::load_plugin_cache_from(&path));

        let cached = soundcraft_vst3_host::cached().expect("cached descriptors should exist");
        assert!(cached.iter().any(|d| d.id == vst.id && d.name == "Cached Synth"));

        let _ = std::fs::remove_file(&path);
    }

    use super::{Category, PluginItem, ScannedLine, add_plugin_to_track, retain_visible_selection};
    use crate::{Services, SoundApp};
    use soundcraft_model::{ChannelFormat, TrackKind};

    #[test]
    fn changing_catalogue_filter_clears_a_stale_plugin_selection() {
        let vst = PluginItem {
            id: "vst3:pianoteq".into(),
            name: "Pianoteq".into(),
            vendor: "Modartt".into(),
            format: "VST3",
            category: Category::Instrument,
            is_instrument: true,
        };
        let clap = PluginItem {
            id: "clap:other".into(),
            name: "Other".into(),
            vendor: "Vendor".into(),
            format: "CLAP",
            category: Category::Instrument,
            is_instrument: true,
        };
        let filtered = [&clap];
        let mut selected = Some(vst.id.clone());

        retain_visible_selection(&mut selected, &filtered);

        assert_eq!(selected, None);
    }

    #[test]
    fn adding_instrument_to_audio_track_converts_to_instrument_track_and_loads() {
        let mut engine = soundcraft_engine::Engine::default();
        let track_id = engine.session_mut().add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Kick"));
        let Some(track) = engine.session().track(track_id).cloned() else {
            panic!("new audio track should exist");
        };
        let mut app = SoundApp::new(engine, None, Services::default());
        let plugin = PluginItem {
            id: "drum_synth".into(),
            name: "Drum Synth".into(),
            vendor: "SoundCraft".into(),
            format: "VST3",
            category: Category::Instrument,
            is_instrument: true,
        };

        add_plugin_to_track(&mut app, &track, &plugin, 0);

        assert_eq!(app.ui.status, "Loaded instrument `Drum Synth` on Kick");
        let t = app.engine.session().track(track_id).expect("track must exist");
        assert_eq!(t.kind, TrackKind::Instrument);
        assert_eq!(t.instrument.as_ref().map(|i| i.plugin.as_str()), Some("drum_synth"));
    }

    #[test]
    fn choosing_an_instrument_reports_success_only_after_it_is_set() {
        let mut engine = soundcraft_engine::Engine::default();
        let track_id = engine.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
        let Some(track) = engine.session().track(track_id).cloned() else {
            panic!("new instrument track should exist");
        };
        let mut app = SoundApp::new(engine, None, Services::default());
        let plugin = PluginItem {
            id: "drum_synth".into(),
            name: "Drum Synth".into(),
            vendor: "SoundCraft".into(),
            format: "VST3",
            category: Category::Instrument,
            is_instrument: true,
        };

        add_plugin_to_track(&mut app, &track, &plugin, 0);

        assert_eq!(app.ui.status, "Loaded instrument `Drum Synth` on Keys");
        assert_eq!(app.engine.session().track(track_id).and_then(|t| t.instrument.as_ref()).map(|i| i.plugin.as_str()), Some("drum_synth"));
    }

    #[test]
    fn choosing_an_unavailable_vst_does_not_claim_it_was_loaded() {
        let mut engine = soundcraft_engine::Engine::default();
        let track_id = engine.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
        let Some(track) = engine.session().track(track_id).cloned() else {
            panic!("new instrument track should exist");
        };
        let mut app = SoundApp::new(engine, None, Services::default());
        let plugin = PluginItem {
            id: "vst3:missing".into(),
            name: "Missing VST".into(),
            vendor: "Test".into(),
            format: "VST3",
            category: Category::Instrument,
            is_instrument: true,
        };

        add_plugin_to_track(&mut app, &track, &plugin, 0);

        assert!(app.ui.status.contains("unknown plugin `vst3:missing`"), "{}", app.ui.status);
        assert!(app.engine.session().track(track_id).is_some_and(|t| t.instrument.is_none()));
    }

    #[test]
    fn double_clicking_a_plugin_loads_it_on_the_instrument_track() {
        use egui::{Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, Shape, pos2, vec2};

        let mut engine = soundcraft_engine::Engine::default();
        let track_id = engine.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
        let mut app = SoundApp::new(engine, None, Services::default());
        app.ui.show_fx_browser = Some(track_id);
        let ctx = egui::Context::default();
        let search_id = egui::Id::new(("fx_browser_search", track_id.0));
        ctx.data_mut(|data| data.insert_temp(search_id, "Drum Synth".to_string()));
        let mut frame_time = 0.0;

        let mut frame = |ctx: &egui::Context, app: &mut SoundApp, events| {
            frame_time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))),
                time: Some(frame_time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            // The test never uploads textures; an unapplied delta panics when the output drops.
            out.textures_delta.clear();
            out
        };
        let mut output: FullOutput = frame(&ctx, &mut app, Vec::new());
        for _ in 0..3 {
            output = frame(&ctx, &mut app, Vec::new());
        }
        let plugin_center = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.text() == "Drum Synth" => Some(text.visual_bounding_rect().center()),
                _ => None,
            })
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .expect("the filtered plugin row should be visible");

        for pressed in [true, false] {
            let _ = frame(
                &ctx,
                &mut app,
                vec![
                    Event::PointerMoved(plugin_center),
                    Event::PointerButton { pos: plugin_center, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ],
            );
        }
        let selected_id = egui::Id::new(("fx_browser_sel_plug", track_id.0));
        assert_eq!(ctx.data(|data| data.get_temp::<Option<String>>(selected_id)).flatten().as_deref(), Some("drum_synth"));
        for pressed in [true, false] {
            let _ = frame(
                &ctx,
                &mut app,
                vec![
                    Event::PointerMoved(plugin_center),
                    Event::PointerButton { pos: plugin_center, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ],
            );
        }
        for _ in 0..2 {
            let _ = frame(&ctx, &mut app, Vec::new());
        }

        assert_eq!(app.ui.status, "Loaded instrument `Drum Synth` on Keys");
        assert_eq!(
            app.engine.session().track(track_id).and_then(|t| t.instrument.as_ref()).map(|i| i.plugin.as_str()),
            Some("drum_synth"),
            "status: {}; tracks: {:?}",
            app.ui.status,
            app.engine.session().tracks.iter().map(|t| (&t.name, t.instrument.as_ref().map(|i| &i.plugin))).collect::<Vec<_>>()
        );
    }

    #[test]
    fn scanner_lines_are_parsed_per_line() {
        let vst3 = r#"{"vst3":{"category":"Instrument","class_id":"ABCD","id":"vst3:ABCD","is_instrument":true,"name":"Synth","path":"/x/Synth.vst3","sdk_version":"VST 3.7.4","sub_categories":["Instrument"],"vendor":"V","version":"1"}}"#;
        match serde_json::from_str::<ScannedLine>(vst3) {
            Ok(ScannedLine::Vst3(d)) => {
                assert_eq!(d.id, "vst3:ABCD");
                assert_eq!(d.name, "Synth");
                assert!(d.is_instrument);
            }
            other => panic!("not a VST3 line: {other:?}"),
        }
        let clap = r#"{"clap":{"category":"Other","description":"","features":[],"id":"clap:org.test.gain","is_instrument":false,"name":"Gain","path":"/x/Gain.clap","plugin_id":"org.test.gain","vendor":"T","version":"1"}}"#;
        match serde_json::from_str::<ScannedLine>(clap) {
            Ok(ScannedLine::Clap(d)) => assert_eq!(d.id, "clap:org.test.gain"),
            other => panic!("not a CLAP line: {other:?}"),
        }
        // A truncated line (the child crashed mid-write) is skipped, not fatal.
        assert!(serde_json::from_str::<ScannedLine>("{\"vst3\":{\"id\":").is_err());
    }
}
