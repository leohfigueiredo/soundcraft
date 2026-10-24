//! The menu bar, built from the incumbent's menu catalog so every item has its place. Items
//! that map to a command are live; the rest are shown disabled (and listed in the parity report).

use crate::{MainWindow, SoundApp};
use serde_json::{Value, json};
use soundcraft_engine::catalog;

/// UI-layer commands: (id, label, catalog path or "", shortcut).
pub const UI_COMMANDS: &[(&str, &str, &str, Option<&str>)] = &[
    ("ui.dock", "Arrange Panel", "", None),
    ("ui.dock.reset", "Reset Panel Layout", "Window > Reset Panel Layout", None),
    ("window.mix", "Mix", "Window > Mix", Some("Cmd+=")),
    ("window.edit", "Edit", "Window > Edit", Some("Cmd+=")),
    ("window.toggle_mix_edit", "Toggle Mix/Edit", "", Some("Cmd+=")),
    ("window.mix_separate", "Mix in Separate Window", "", None),
    ("window.transport", "Transport", "Window > Transport", Some("Cmd+1")),
    ("window.big_counter", "Big Counter", "Window > Big Counter", Some("Cmd+3")),
    ("window.memory_locations", "Memory Locations", "Window > Memory Locations", Some("Cmd+5")),
    ("window.undo_history", "Undo History", "Window > Undo History", None),
    ("window.clip_list", "Clip List", "Window > Clip List", None),
    ("window.track_list", "Track List", "View > Other Displays > Track List", None),
    ("window.clip_list_view", "Clip List", "View > Other Displays > Clip List", None),
    ("window.narrow_mix", "Narrow Mix", "View > Narrow Mix", Some("Cmd+Alt+M")),
    ("window.session_info", "Session Info", "", None),
    ("window.audio_health", "Session Audio Health", "", None),
    ("window.about", "About SoundCraft", "", None),
    ("window.close", "Close Window", "Window > Close Window", Some("Cmd+W")),
    ("window.hide_floating", "Hide All Floating Windows", "Window > Hide All Floating Windows", Some("Cmd+Ctrl+W")),
    ("window.midi_editor", "MIDI Editor", "Window > MIDI Editor", None),
    ("window.universe", "Universe", "View > Other Displays > Universe", Some("Alt+7")),
    ("window.video", "Video", "Window > Video", Some("Cmd+9")),
    ("window.video_universe", "Video Universe", "Window > Video Universe", None),
    ("window.automation", "Automation", "Window > Automation", Some("Cmd+4")),
    ("window.color_palette", "Color Palette", "Window > Color Palette", None),
    ("window.disk_usage", "Disk Usage", "Window > Disk Usage", None),
    ("window.system_usage", "System Usage", "Window > System Usage", None),
    ("window.task_manager", "Task Manager", "Window > Task Manager", Some("Alt+'")),
    ("window.metadata", "Metadata Inspector", "Window > Metadata Inspector", None),
    ("window.event_list", "MIDI Event List", "Window > MIDI Event List", Some("Alt+=")),
    ("window.midi_keyboard", "MIDI Keyboard", "Window > MIDI Keyboard", Some("Shift+K")),
    ("window.mixer_dock", "Mixer Dock", "Window > Mixer Dock", None),
    ("window.midi_monitor", "MIDI Monitor", "Window > MIDI Monitor", None),
    ("window.midi_devices", "MIDI Devices", "Window > MIDI Devices", None),
    ("window.fx_browser", "Track FX Browser", "Window > Track FX Browser", Some("Shift+F")),
    ("window.workspace", "Workspace", "Window > New Workspace > Default", Some("Alt+I")),
    ("window.workspace_presets", "Workspace", "Window > New Workspace > Track Presets", None),
    ("window.workspace_sounds", "Workspace", "Window > New Workspace > Soundbase", None),
    ("window.workspace_front", "Workspaces to Front", "Window > Workspaces > Bring to Front", Some("Alt+J")),
    ("window.workspace_close", "Close All Workspaces", "Window > Workspaces > Close All Workspaces", None),
    ("window.configurations", "Window Configuration List", "Window > Configurations > Window Configuration List", Some("Cmd+Alt+J")),
    ("window.config_new", "New Configuration", "Window > Configurations > New Configuration...", None),
    ("window.config_update", "Update Active Configuration", "Window > Configurations > Update Active Configuration", None),
    ("window.config_auto_update", "Auto-Update Active Configuration", "Window > Configurations > Auto-Update Active Configuration", None),
    ("window.renderer", "Renderer", "Window > Renderer", None),
    ("window.ui_customization", "UI Customization", "Window > UI Customization", None),
    ("window.arrange_tile", "Tile", "Window > Arrange > Tile", None),
    ("window.arrange_tile_h", "Tile Horizontal", "Window > Arrange > Tile Horizontal", None),
    ("window.arrange_tile_v", "Tile Vertical", "Window > Arrange > Tile Vertical", None),
    ("window.arrange_cascade", "Cascade", "Window > Arrange > Cascade", None),
    ("window.midi_front", "MIDI Editors to Front", "Window > MIDI Editors > Bring to Front", None),
    ("window.midi_back", "MIDI Editors to Back", "Window > MIDI Editors > Send to Back", None),
    ("window.clip_effects", "Clip Effects", "Window > Clip Effects", None),
    ("window.clip_effects_dock", "Clip Effects", "View > Other Displays > Lower Dock > Clip Effects", Some("Alt+6")),
    ("window.search", "Search", "Window > Pro Tools Search", Some("Cmd+Ctrl+S")),
    ("window.score", "Score Editor", "Window > Score Editor", Some("Cmd+Ctrl+=")),
    ("window.beat_detective", "Beat Detective", "", None),
    ("window.tempo_ops", "Tempo Operations", "", None),
    ("window.time_ops", "Time Operations", "", None),
    ("window.midi_ops", "MIDI Operations", "", None),
    ("window.rtp", "MIDI Real-Time Properties", "", None),
    ("window.playback_engine", "Playback Engine", "", None),
    ("ui.audio_device", "Audio Device", "", None),
    ("window.io_setup", "I/O Setup", "", None),
    ("window.msc_hardware", "DAW Controllers & Mackie Setup", "Setup > Hardware > DAW Controller Setup (iCON / M-Vave)...", None),
    ("window.console.track", "Alternative GUI for This Track", "Window > Alternative GUI > This Track", Some("Shift+Alt+P")),
    ("window.console", "Instrument Console (Alternative GUI)", "Window > Instrument Console (Alternative GUI)", Some("Shift+P")),
    ("window.console.pianoteq", "Pianoteq Console", "", None),
    ("window.console.pigments", "Pigments Console", "", None),
    ("window.console.omnisphere", "Omnisphere Console", "", None),
    ("window.console.kontakt", "Kontakt Console", "", None),
    ("window.console.analoglab", "Analog Lab Console", "", None),
    ("window.console.labs", "LABS Console", "", None),
    ("controller.rescan", "Rescan MIDI Ports", "Setup > Hardware > Rescan MIDI Ports", None),
    ("controller.midi_ports", "MIDI Input Ports", "", None),
    ("controller.keyboard", "Connect MIDI Keyboard", "", None),
    ("controller.keyboard_auto", "Auto-Connect MIDI Keyboard", "", None),
    ("controller.ble", "Bluetooth (BLE-MIDI) Link", "", None),
    ("controller.surface", "Use MIDI Port as Control Surface", "", None),
    ("controller.mode", "Control Surface Mode (Global / Linked)", "", None),
    ("controller.soft_takeover", "Soft Takeover (Fader Pickup)", "", None),
    ("controller.learn", "MIDI Learn…", "", None),
    ("controller.cancel_learn", "Cancel MIDI Learn", "", None),
    ("controller.clear_learn", "Clear Learned Controls", "", None),
    ("controller.status", "Control Surface Status", "", None),
    ("window.vst_folders", "VST / CLAP Plugin Folders", "Setup > Hardware > VST & CLAP Plugin Folders...", None),
    ("window.shortcuts", "Keyboard Shortcuts", "", None),
    ("view.mix_section", "Mix Window View", "", None),
    ("ui.new_tracks_dialog", "New Tracks…", "", None),
    ("ui.bounce_dialog", "Bounce Mix…", "", None),
    ("ui.set", "Set UI State", "", None),
    ("ui.theme", "Appearance", "", None),
];

/// Extra catalog mappings handled by the UI layer.
pub fn ui_aliases() -> Vec<(&'static str, &'static str)> {
    let mut v: Vec<(&str, &str)> = UI_COMMANDS.iter().filter(|c| !c.2.is_empty()).map(|c| (c.2, c.0)).collect();
    v.extend(mix_view_aliases().iter().copied());
    v.extend(AUDIOSUITE.iter().map(|(p, _)| (*p, "audiosuite.process")));
    v.extend(MENU_WINDOWS.iter().copied());
    v
}

/// Mix-view menu paths, allocated once. Rebuilding them on every menu draw leaked a copy per frame.
fn mix_view_aliases() -> &'static [(&'static str, &'static str)] {
    static ALIASES: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
    ALIASES.get_or_init(|| {
        [
            "Inserts A-E",
            "Inserts F-J",
            "Sends A-E",
            "Sends F-J",
            "I/O",
            "Track Color",
            "Comments",
            "EQ Curve",
            "Meters and Faders",
            "All",
            "Minimal",
            "Mic Preamps",
            "Instruments",
            "Object",
        ]
        .into_iter()
        .map(|sec| {
            let path: &'static str = Box::leak(format!("View > Mix Window Views > {sec}").into_boxed_str());
            (path, "view.mix_section")
        })
        .collect()
    })
}

/// AudioSuite catalog entries → our processes (functional equivalents, our own plugins).
pub const AUDIOSUITE: &[(&str, &str)] = &[
    ("AudioSuite > EQ > Channel Strip", "channel_strip"),
    ("AudioSuite > EQ > EQ3 1-Band", "eq_1band"),
    ("AudioSuite > EQ > EQ3 7-Band", "eq_7band"),
    ("AudioSuite > Dynamics > BF-76", "compressor"),
    ("AudioSuite > Dynamics > Channel Strip", "channel_strip"),
    ("AudioSuite > Dynamics > Dyn3 Compressor/Limiter", "compressor"),
    ("AudioSuite > Dynamics > Dyn3 De-Esser", "de_esser"),
    ("AudioSuite > Dynamics > Dyn3 Expander/Gate", "expander_gate"),
    ("AudioSuite > Dynamics > Maxim", "maximizer"),
    ("AudioSuite > Pitch Shift > Pitch II", "pitch_shift"),
    ("AudioSuite > Pitch Shift > Pitch Shift Legacy", "pitch_shift"),
    ("AudioSuite > Pitch Shift > Time Shift", "time_stretch"),
    ("AudioSuite > Pitch Shift > Vari-Fi", "varispeed"),
    ("AudioSuite > Reverb > D-Verb", "room_reverb"),
    ("AudioSuite > Delay > Mod Delay III", "mod_delay"),
    ("AudioSuite > Modulation > Sci-Fi", "flanger"),
    ("AudioSuite > Harmonic > Eleven Lite", "saturator"),
    ("AudioSuite > Harmonic > Lo-Fi", "lofi"),
    ("AudioSuite > Harmonic > Recti-Fi", "rectifier"),
    ("AudioSuite > Harmonic > SansAmp PSA-1", "saturator"),
    ("AudioSuite > Other > DC Offset Removal", "dc_offset_removal"),
    ("AudioSuite > Other > Duplicate", "duplicate"),
    ("AudioSuite > Other > Gain", "gain"),
    ("AudioSuite > Other > Invert", "invert"),
    ("AudioSuite > Other > Normalize", "normalize"),
    ("AudioSuite > Other > Reverse", "reverse"),
    ("AudioSuite > Other > Signal Generator", "signal_generator"),
    ("AudioSuite > Other > Time Compression Expansion", "time_stretch"),
];

/// Commands that open a dialog when invoked from a menu (programmatic calls never do).
const DIALOG_COMMANDS: &[&str] = &[
    "clip.clear",
    "track.new",
    "file.bounce_mix",
    "session.new",
    "file.import_audio",
    "file.import_midi",
    "session.open",
    "session.save",
    "session.save_as",
    "session.save_copy",
    "track.group",
    "edit.repeat",
    "edit.shift",
    "edit.fades_create",
    "setup.session",
    "event.quantize",
    "event.transpose",
    "edit.strip_silence",
    "clip.rename",
];

/// Shared menu hierarchy for egui and native desktop hosts.
pub struct MenuNode {
    pub label: String,
    pub path: String,
    pub children: Vec<MenuNode>,
}

fn insert_path(roots: &mut Vec<MenuNode>, line: &str) {
    let parts: Vec<&str> = line.split(" > ").collect();
    let mut level = roots;
    let mut path = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            path.push_str(" > ");
        }
        path.push_str(p);
        let idx = match level.iter().position(|n| n.label == *p) {
            Some(i) => i,
            None => {
                level.push(MenuNode { label: p.to_string(), path: path.clone(), children: Vec::new() });
                level.len() - 1
            }
        };
        let Some(node) = level.get_mut(idx) else { break };
        level = &mut node.children;
    }
}

pub fn tree() -> Vec<MenuNode> {
    let mut roots: Vec<MenuNode> = Vec::new();
    for line in catalog::catalog() {
        insert_path(&mut roots, line);
    }
    insert_path(&mut roots, "Window > Reset Panel Layout");
    insert_path(&mut roots, "Window > MIDI Monitor");
    insert_path(&mut roots, "Window > MIDI Devices");
    // Not in the incumbent catalog, but the only UI path to an empty MIDI clip (#282).
    // It creates a clip rather than acting on the selected ones, so it leads the Clip menu.
    insert_path(&mut roots, "Clip > New MIDI Clip");
    if let Some(clip) = roots.iter_mut().find(|n| n.label == "Clip") {
        clip.children.rotate_right(1);
    }
    for (path, _) in MENU_WINDOWS {
        insert_path(&mut roots, path);
    }
    roots
}

pub fn menu_bar(app: &mut SoundApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::current();
    egui::Panel::top("menu_bar").exact_size(24.0).frame(egui::Frame::NONE.fill(t.menu_bar).inner_margin(egui::Margin::symmetric(8, 2))).show(
        ui,
        |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("SoundCraft", |ui| {
                    if ui.button("About SoundCraft").clicked() {
                        app.ui.show_about = true;
                    }
                    if ui.button("Session Info").clicked() {
                        app.ui.show_session_info = true;
                    }
                    if ui.button("Session Audio Health").clicked() {
                        let _ = app.run("window.audio_health", json!({"value": true}));
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        let _ = crate::dialogs::run_or_ask(app, "app.quit", json!({}));
                    }
                });
                let extra = ui_aliases();
                for root in tree() {
                    ui.menu_button(&root.label, |ui| {
                        for c in &root.children {
                            menu_node(app, ui, c, &extra);
                            if c.path == "File > Open" {
                                crate::recent::menu(app, ui);
                            }
                        }
                        if root.label == "AudioSuite" {
                            ui.separator();
                            ui.label(egui::RichText::new("Processes selected clips offline").small().color(t.text_dim));
                        }
                        if root.label == "Window" {
                            ui.separator();
                            let mut separate = app.ui.mix_separate;
                            if ui
                                .checkbox(&mut separate, "Mix in Separate Window")
                                .on_hover_text("Show Edit and Mix at once, e.g. one per monitor")
                                .changed()
                            {
                                let _ = run_ui_command(app, "window.mix_separate", &json!({"value": separate}));
                                ui.close();
                            }
                        }
                    });
                }
                let title = format!(
                    "{}{}  —  {}",
                    app.engine.session().name,
                    if app.engine.is_dirty() { " *" } else { "" },
                    if app.ui.window == MainWindow::Edit || app.ui.mix_separate { "Edit" } else { "Mix" }
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(title).color(t.text_dim));
                    ui.separator();
                    let cur_name = match app.ui.theme {
                        crate::theme::ThemeMode::Dark => "Dark Studio",
                        crate::theme::ThemeMode::FLStudio => "FL Studio (Charcoal & Neon)",
                        crate::theme::ThemeMode::StudioPro => "Studio Pro (Black Piano)",
                        crate::theme::ThemeMode::Light => "Clean Light",
                        crate::theme::ThemeMode::Cyberpunk => "Cyberpunk / Neon",
                        crate::theme::ThemeMode::Vintage => "Vintage Console",
                        crate::theme::ThemeMode::System => "System",
                    };
                    egui::ComboBox::from_id_salt("theme_quick_select").selected_text(cur_name).show_ui(ui, |ui| {
                        for (mode, label) in [
                            (crate::theme::ThemeMode::FLStudio, "FL Studio (Charcoal & Neon)"),
                            (crate::theme::ThemeMode::Dark, "Dark Studio"),
                            (crate::theme::ThemeMode::StudioPro, "Studio Pro (Black Piano)"),
                            (crate::theme::ThemeMode::Light, "Clean Light"),
                            (crate::theme::ThemeMode::Cyberpunk, "Cyberpunk / Neon"),
                            (crate::theme::ThemeMode::Vintage, "Vintage Console"),
                            (crate::theme::ThemeMode::System, "System"),
                        ] {
                            if ui.selectable_label(app.ui.theme == mode, label).clicked() {
                                app.ui.theme = mode;
                                let _ = app.run("ui.theme", json!({"mode": mode.id()}));
                            }
                        }
                    });
                    ui.label(egui::RichText::new("Theme:").small().color(t.text_dim));
                    ui.separator();
                    // Controller state, always visible: a dot that goes bright on input, plus the
                    // keyboard. Clicking opens the setup window.
                    let now = ui.input(|i| i.time);
                    use crate::midi_io::LinkState;
                    let (icon, label, col) = match app.msc.link.state {
                        LinkState::Connected => {
                            let port = app.msc.link.port_name.replace("SoundCraft ", "");
                            let name = if port.is_empty() { "MCU".to_string() } else { port };
                            ("🎛", name, egui::Color32::from_rgb(0, 220, 130))
                        }
                        LinkState::Searching => ("🎛", "MCU: no port".to_string(), t.meter_yellow),
                        LinkState::Unavailable => ("🎛", "MCU: no MIDI".to_string(), t.meter_red),
                    };
                    let flash = app.msc.input_active(now, 0.6);
                    if ui
                        .small_button(egui::RichText::new(format!("{}{icon} {label}", if flash { "● " } else { "" })).color(col))
                        .on_hover_text("MIDI controller: click for setup, faders, MIDI Learn and the input monitor")
                        .clicked()
                    {
                        app.ui.show_msc_hardware = !app.ui.show_msc_hardware;
                    }
                    let (kb_icon, kb_label, kb_col) = match app.msc.link.keyboard_state {
                        LinkState::Connected => {
                            let name = app.msc.keyboard_port.clone().unwrap_or_else(|| "MIDI keyboard".to_string());
                            ("🎹", name, egui::Color32::from_rgb(0, 220, 130))
                        }
                        LinkState::Searching => ("🎹", "no keyboard".to_string(), t.text_dim),
                        LinkState::Unavailable => ("🎹", "no MIDI".to_string(), t.meter_red),
                    };
                    ui.small_button(egui::RichText::new(format!("{kb_icon} {kb_label}")).color(kb_col))
                        .on_hover_text("MIDI keyboard: notes play the selected instrument track");
                    ui.separator();
                    // FL Studio style quick-dock buttons: Piano (F7), Plugins (F8), Mixer (F9)
                    let piano_btn = egui::Button::new(egui::RichText::new("🎹 Piano (F7)").color(if app.ui.show_midi_keyboard {
                        egui::Color32::BLACK
                    } else {
                        t.text
                    }))
                    .fill(if app.ui.show_midi_keyboard { t.accent } else { t.button });
                    if ui.add(piano_btn).on_hover_text("Show or hide the virtual master keyboard (F7)").clicked() {
                        app.ui.show_midi_keyboard = !app.ui.show_midi_keyboard;
                    }

                    let plug_btn = egui::Button::new(egui::RichText::new("🎛 Plugins (F8)").color(if app.ui.show_vst_folders {
                        egui::Color32::BLACK
                    } else {
                        t.text
                    }))
                    .fill(if app.ui.show_vst_folders { t.accent } else { t.button });
                    if ui.add(plug_btn).on_hover_text("Open the VST3/CLAP plugin manager and search folders (F8)").clicked() {
                        app.ui.show_vst_folders = !app.ui.show_vst_folders;
                    }

                    // The alternative console: one window, an instrument tab row inside, always
                    // one click away (and Shift+P) so it is found without opening the dock.
                    let any_open = app.ui.show_console;
                    ui.menu_button(egui::RichText::new("🎹 ALT GUI ▾").color(if any_open { t.accent } else { t.text }), |ui| {
                        ui.set_min_width(260.0);
                        let current = app.ui.console_profile.clone();
                        for profile in crate::console_kit::profiles() {
                            let mark = if app.ui.show_console && current == profile.id { "✔ " } else { "   " };
                            if ui.button(format!("{mark}{} console", profile.name)).clicked() {
                                crate::console_kit::open(app, profile.id);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Close the console").clicked() {
                            app.ui.show_console = false;
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Alternative rack-style GUI for the hosted instruments (Shift+P)");

                    // The mixer lives in the dock under the timeline now; F9 still opens the full
                    // console, but there is no "switch windows" button in the way.
                    let _ = MainWindow::Mix;
                });
            });
        },
    );
}

fn menu_node(app: &mut SoundApp, ui: &mut egui::Ui, n: &MenuNode, extra: &[(&str, &str)]) {
    if !n.children.is_empty() {
        ui.menu_button(&n.label, |ui| {
            for c in &n.children {
                menu_node(app, ui, c, extra);
            }
        });
        return;
    }
    let id = command_for_path(&n.path, extra);
    let (enabled, checked) = item_state(app, &n.path, id.as_deref());
    let label = live_label(app, id.as_deref()).unwrap_or_else(|| n.label.clone());
    let label = if checked { format!("✔ {label}") } else { label };
    let shortcut = id.as_deref().and_then(shortcut_for).unwrap_or("");
    let btn = egui::Button::new(label).shortcut_text(crate::shortcuts::shortcut_label(shortcut, ui.ctx().os().is_mac()));
    if ui.add_enabled(enabled, btn).clicked()
        && let Some(id) = id
    {
        invoke_menu(app, &id, &n.path);
        ui.close();
    }
}

/// Resolve the command behind a menu leaf, including UI dialog overrides.
pub fn command_for_path(path: &str, extra: &[(&str, &str)]) -> Option<String> {
    catalog::implemented_by(path, extra).or_else(|| MENU_WINDOWS.iter().find(|(p, _)| *p == path).map(|(_, w)| w.to_string()))
}

/// Enabled and checked state shared by all menu presentations.
pub fn item_state(app: &SoundApp, path: &str, id: Option<&str>) -> (bool, bool) {
    let enabled = match id {
        Some(i)
            if i.starts_with("window.") || i.starts_with("view.mix_section") || matches!(i, "audiosuite.process" | "ui.dock" | "ui.dock.reset") =>
        {
            true
        }
        Some(i) => soundcraft_engine::find_command(i).is_some_and(|c| (c.enabled)(&app.engine).is_ok() && !app.engine.blocked_while_recording(i)),
        None => false,
    };
    (enabled, checked_state(app, path, id))
}

fn checked_state(app: &SoundApp, path: &str, id: Option<&str>) -> bool {
    let e = &app.engine.session().edit;
    match id {
        Some("options.loop_playback") => e.loop_playback,
        Some("options.loop_record") => e.loop_record,
        Some("options.quickpunch") => e.quickpunch,
        Some("options.pre_post_roll") => e.pre_post_roll,
        Some("options.link_timeline_edit") => e.link_timeline_edit,
        Some("options.link_track_edit") => e.link_track_edit,
        Some("options.insertion_follows_playback") => e.insertion_follows_playback,
        Some("options.tab_to_transient") => e.tab_to_transient,
        Some("options.automation_follows_edit") => e.automation_follows_edit,
        Some("options.markers_follow_edit") => e.markers_follow_edit,
        Some("options.click") => e.click,
        Some("options.delay_compensation") => e.delay_compensation,
        Some("options.pre_fader_metering") => e.pre_fader_metering,
        Some("view.ruler") => path.rsplit(" > ").next().map(ruler_id).is_some_and(|r| e.rulers.iter().any(|x| x == r)),
        Some("view.main_counter") => path.rsplit(" > ").next().map(ruler_id).is_some_and(|r| e.main_counter.id() == r),
        Some("window.mix") => app.ui.window == MainWindow::Mix,
        Some("window.edit") => app.ui.window == MainWindow::Edit,
        Some("window.track_list") => app.ui.show_tracks_list,
        Some("window.clip_list_view") | Some("window.clip_list") => app.ui.show_clip_list,
        Some("window.narrow_mix") => app.ui.narrow_mix,
        Some("window.renderer") => app.extra.show_renderer,
        Some("window.ui_customization") => app.extra.show_ui_customization,
        Some("window.config_auto_update") => app.extra.auto_update_config,
        Some("view.mix_section") => path.rsplit(" > ").next().map(mix_section_id).is_some_and(|s| app.ui.mix_views.iter().any(|v| v == s)),
        _ => false,
    }
}

fn ruler_id(label: &str) -> &'static str {
    match label {
        "Bars|Beats" => "bars_beats",
        "Min:Secs" => "min_secs",
        "Timecode" => "timecode",
        "Feet+Frames" => "feet_frames",
        "Samples" => "samples",
        "Tempo" => "tempo",
        "Meter" => "meter",
        "Markers" => "markers",
        "Key Signature" => "key",
        "Chord Symbols" => "chords",
        _ => "",
    }
}

fn mix_section_id(label: &str) -> &'static str {
    match label {
        "Inserts A-E" => "inserts_ae",
        "Inserts F-J" => "inserts_fj",
        "Sends A-E" => "sends_ae",
        "Sends F-J" => "sends_fj",
        "I/O" => "io",
        "Track Color" => "color",
        "Comments" => "comments",
        "EQ Curve" => "eq_curve",
        "Meters and Faders" => "meters",
        "All" => "all",
        "Minimal" => "minimal",
        "Mic Preamps" => "mic_preamps",
        "Instruments" => "instruments",
        "Object" => "object",
        _ => "",
    }
}

/// Parameters implied by the menu item a command was reached through.
fn params_for(path: &str, id: &str) -> Value {
    let leaf = path.rsplit(" > ").next().unwrap_or("");
    match id {
        "view.ruler" => json!({"ruler": ruler_id(leaf)}),
        "view.main_counter" => json!({"format": ruler_id(leaf)}),
        "options.scrolling" => {
            json!({"mode": match leaf { "No Scrolling" => "none", "After Playback" => "after_playback", "Continuous" => "continuous", "Center Playhead" => "center", _ => "page" }})
        }
        "options.solo_mode" => {
            json!({"mode": if leaf.starts_with("AFL") { "afl" } else if leaf.starts_with("PFL") { "pfl" } else { "sip" }, "xor": leaf.starts_with("X-OR")})
        }
        "clip.rating" => json!({"rating": leaf.parse::<u8>().unwrap_or(0)}),
        "track.change_width" => json!({"format": leaf}),
        "view.mix_section" => json!({"section": mix_section_id(leaf)}),
        "audiosuite.process" => json!({"process": AUDIOSUITE.iter().find(|(p, _)| *p == path).map_or("", |(_, x)| *x)}),
        _ => json!({}),
    }
}

/// Menu items that open a SoundCraft window when clicked (programmatic calls still run the command).
const MENU_WINDOWS: &[(&str, &str)] = &[
    ("Setup > Appearance", "window.ui_customization"),
    ("Setup > Hardware...", "window.playback_engine"),
    ("Setup > Playback Engine...", "window.playback_engine"),
    ("Setup > I/O...", "window.io_setup"),
    ("Setup > Keyboard Shortcuts...", "window.shortcuts"),
    ("Setup > Session", "setup.session"),
    ("Setup > Preferences...", "window.vst_folders"),
    ("Event > Beat Detective", "window.beat_detective"),
    ("Event > Tempo Operations > Tempo Operations Window", "window.tempo_ops"),
    ("Event > Time Operations > Time Operations Window", "window.time_ops"),
    ("Event > MIDI Operations > MIDI Operations Window", "window.midi_ops"),
    ("Event > MIDI Real-Time Properties", "window.rtp"),
];

/// The shortcut of an engine or window command.
pub fn shortcut_for(id: &str) -> Option<&'static str> {
    soundcraft_engine::find_command(id).map_or_else(|| UI_COMMANDS.iter().find(|c| c.0 == id).and_then(|c| c.3), |c| c.shortcut)
}

/// Undo and Redo name the step they would undo or redo; other items keep their catalog label.
pub fn live_label(app: &SoundApp, id: Option<&str>) -> Option<String> {
    let e = &app.engine;
    match id? {
        "edit.undo" => Some(e.undo_label().map_or_else(|| "Can't Undo".into(), |l| format!("Undo {l}"))),
        "edit.redo" => Some(e.redo_label().map_or_else(|| "Can't Redo".into(), |l| format!("Redo {l}"))),
        _ => None,
    }
}

/// Menu-style invocation: may open a dialog.
pub fn invoke_menu(app: &mut SoundApp, id: &str, path: &str) {
    if let Some((_, w)) = MENU_WINDOWS.iter().find(|(p, _)| *p == path)
        && w.starts_with("window.")
    {
        let _ = app.run(w, json!({"value": true}));
        return;
    }
    if id == "audiosuite.process" {
        let p = params_for(path, id);
        app.ui.audiosuite = p.get("process").and_then(Value::as_str).map(str::to_string);
        return;
    }
    if wants_dialog(id) && app.dialogs.open_for_command(&app.engine, id) {
        return;
    }
    // These ids only exist to open a dialog from the menu. Programmatic `app.run` refuses them.
    if id == "ui.new_tracks_dialog" {
        let _ = app.dialogs.open_for_command(&app.engine, "track.new");
        return;
    }
    if id == "ui.bounce_dialog" {
        let _ = app.dialogs.open_for_command(&app.engine, "file.bounce_mix");
        return;
    }
    let params = params_for(path, id);
    let _ = crate::dialogs::run_or_ask(app, id, params);
}

/// Keyboard shortcut for a dialog command, or one that would close a modified session: open its
/// dialog. Returns true when handled.
pub fn invoke_shortcut_dialog(app: &mut SoundApp, id: &str) -> bool {
    (wants_dialog(id) && app.dialogs.open_for_command(&app.engine, id)) || crate::dialogs::ask_to_save(app, id, &json!({}))
}

/// Commands that open a dialog first: the listed ones, Score Setup, and anything that needs a path.
fn wants_dialog(id: &str) -> bool {
    DIALOG_COMMANDS.contains(&id) || id == "file.score_setup" || crate::dialogs::takes_path(id)
}

/// Handle UI-layer commands. Returns None when `id` is not a UI command.
pub fn run_ui_command(app: &mut SoundApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if matches!(id, "ui.dock" | "ui.dock.reset") {
        return Some(crate::docking::command(app, id == "ui.dock.reset", p));
    }
    if let Some(v) = crate::extra_windows::run(app, id, p) {
        return Some(Ok(v));
    }
    if id == "ui.audio_device" {
        return Some(crate::windows::audio_device_command(app, p));
    }
    let toggle = |v: &mut bool| {
        *v = p.get("value").and_then(Value::as_bool).unwrap_or(!*v);
        json!({"value": *v})
    };
    let r = match id {
        "window.mix" => {
            app.ui.window = MainWindow::Mix;
            app.focus_mix = app.ui.mix_separate;
            json!({"window": "mix"})
        }
        "window.mix_separate" => {
            let v = toggle(&mut app.ui.mix_separate);
            app.focus_mix = app.ui.mix_separate;
            v
        }
        "window.edit" => {
            app.ui.window = MainWindow::Edit;
            json!({"window": "edit"})
        }
        "window.toggle_mix_edit" => {
            if app.ui.mix_separate {
                // Both are on screen: bring the Mix window forward instead.
                app.focus_mix = true;
            } else {
                app.ui.window = if app.ui.window == MainWindow::Edit { MainWindow::Mix } else { MainWindow::Edit };
            }
            json!({"window": app.ui.window})
        }
        "window.transport" => toggle(&mut app.ui.show_transport),
        "window.big_counter" => toggle(&mut app.ui.show_big_counter),
        "window.memory_locations" => toggle(&mut app.ui.show_memory_locations),
        "window.undo_history" => toggle(&mut app.ui.show_undo_history),
        "window.clip_list" | "window.clip_list_view" => toggle(&mut app.ui.show_clip_list),
        "window.track_list" => toggle(&mut app.ui.show_tracks_list),
        "window.narrow_mix" => toggle(&mut app.ui.narrow_mix),
        "window.session_info" => toggle(&mut app.ui.show_session_info),
        "window.audio_health" => {
            app.audio_health_report = None;
            toggle(&mut app.ui.show_audio_health)
        }
        "window.about" => toggle(&mut app.ui.show_about),
        "window.midi_editor" => toggle(&mut app.ui.show_midi_editor),
        "window.midi_front" => {
            app.ui.show_midi_editor = true;
            json!({"value": true})
        }
        "window.midi_back" => {
            app.ui.show_midi_editor = false;
            json!({})
        }
        "window.automation" => toggle(&mut app.ui.show_automation),
        "window.clip_effects" | "window.clip_effects_dock" => toggle(&mut app.ui.show_clip_effects),
        "window.search" => toggle(&mut app.ui.show_search),
        "window.score" => toggle(&mut app.ui.show_score),
        "window.beat_detective" => toggle(&mut app.ui.show_beat_detective),
        "window.tempo_ops" => toggle(&mut app.ui.show_tempo_ops),
        "window.time_ops" => toggle(&mut app.ui.show_time_ops),
        "window.midi_ops" => toggle(&mut app.ui.show_midi_ops),
        "window.rtp" => toggle(&mut app.ui.show_rtp),
        "window.playback_engine" => toggle(&mut app.ui.show_playback_engine),
        "window.io_setup" => toggle(&mut app.ui.show_io_setup),
        "window.msc_hardware" => toggle(&mut app.ui.show_msc_hardware),
        "window.console" => toggle(&mut app.ui.show_console),
        "window.console.track" => {
            // The console of whatever instrument the selected track holds.
            let track = app.engine.session().edit.selected_tracks.first().copied().or_else(|| crate::virtual_keyboard::active_target_track(app));
            match track {
                Some(track) => {
                    crate::console_kit::open_for_track(app, track);
                    json!({"console": app.ui.console_profile, "track": track.0})
                }
                None => json!({"console": null, "track": null}),
            }
        }
        id if id.starts_with("window.console.") => {
            // One console window, each command picks the instrument it shows.
            let which = id.trim_start_matches("window.console.");
            if crate::console_kit::profile(which).is_some() {
                crate::console_kit::open(app, which);
                json!({"console": which})
            } else {
                json!({"console": null})
            }
        }
        "controller.rescan" => {
            app.control_surface_rescan();
            json!({"link": app.msc.link.state.label()})
        }
        "controller.midi_ports" => {
            let ports: Vec<Value> = app
                .msc
                .link
                .ports
                .iter()
                .map(|p| json!({"name": p.name, "address": p.address, "surface": p.matched, "keyboard": app.msc.keyboard_port.as_deref() == Some(p.name.as_str())}))
                .collect();
            json!({
                "ports": ports,
                "keyboard": app.msc.keyboard_port,
                "keyboard_state": app.msc.link.keyboard_state.label(),
                "surface": app.msc.link.state.label(),
            })
        }
        "controller.keyboard" => {
            let port = p.get("port").and_then(Value::as_str).map(str::to_string);
            if !p.as_object().is_some_and(|o| o.contains_key("port")) {
                return Some(Err("controller.keyboard: `port` required (use {\"port\": null} to disconnect)".to_string()));
            }
            app.control_surface_keyboard(port.clone());
            app.control_surface_rescan();
            json!({"keyboard": port})
        }
        "controller.surface" => {
            let port = p.get("port").and_then(Value::as_str).map(str::to_string);
            app.control_surface_port(port.clone());
            app.control_surface_rescan();
            json!({"surface": port})
        }
        "controller.ble" => {
            let value = p.get("value").and_then(Value::as_bool).unwrap_or(!app.msc.ble);
            app.control_surface_ble(value);
            json!({"ble": value})
        }
        "controller.keyboard_auto" => {
            // The usual piano names: a Casio CDP-S160, a Yamaha, a Roland... but never the
            // control surface itself.
            let pick = app
                .msc
                .link
                .ports
                .iter()
                .find(|p| {
                    !p.matched
                        && ["casio", "cdp", "piano", "keyboard", "yamaha", "roland", "kawai", "nord", "arturia", "midi"]
                            .iter()
                            .any(|a| p.name.to_ascii_lowercase().contains(a))
                })
                .map(|p| p.name.clone());
            match pick {
                Some(name) => {
                    app.control_surface_keyboard(Some(name.clone()));
                    app.control_surface_rescan();
                    json!({"keyboard": name})
                }
                None => return Some(Err("controller.keyboard_auto: no MIDI keyboard port found (plug the piano in, run Rescan)".to_string())),
            }
        }
        "controller.mode" => {
            use crate::msc_mixer::MscMode;
            let mode = match p.get("mode").and_then(Value::as_str).and_then(|m| match m.to_ascii_lowercase().as_str() {
                "global" => Some(MscMode::Global),
                "linked" | "link" => Some(MscMode::Linked),
                _ => None,
            }) {
                Some(m) => m,
                None => match app.msc.mode {
                    MscMode::Global => MscMode::Linked,
                    MscMode::Linked => MscMode::Global,
                },
            };
            app.msc.mode = mode;
            app.msc.reset_pickups();
            json!({"mode": format!("{:?}", mode).to_ascii_lowercase()})
        }
        "controller.soft_takeover" => {
            let value = p.get("value").and_then(Value::as_bool).unwrap_or(!app.msc.config.soft_takeover);
            app.msc.config.soft_takeover = value;
            app.msc.reset_pickups();
            json!({"value": value})
        }
        "controller.learn" => {
            use crate::msc_mixer::LearnTarget;
            let track = p.get("track").and_then(Value::as_u64).or_else(|| app.engine.session().edit.selected_tracks.first().map(|t| t.0));
            let Some(track) = track else {
                return Some(Err("controller.learn: no track selected".to_string()));
            };
            let target = match p.get("target").and_then(Value::as_str).unwrap_or("volume").to_ascii_lowercase().as_str() {
                "pan" => LearnTarget::Pan { track, index: 0 },
                "volume" | "fader" => LearnTarget::Volume { track },
                other => return Some(Err(format!("controller.learn: unknown target `{other}` (volume, pan)"))),
            };
            let described = target.describe();
            app.msc.learn_start(target);
            json!({"armed": true, "target": described})
        }
        "controller.cancel_learn" => {
            let was = app.msc.learn.take().is_some();
            json!({"cancelled": was})
        }
        "controller.clear_learn" => {
            let n = app.msc.bindings.len();
            app.msc.learn_clear(None);
            json!({"removed": n})
        }
        "controller.status" => {
            let bindings: Vec<Value> = app.msc.bindings.iter().map(|b| json!({"control": b.describe(), "target": b.target.describe()})).collect();
            json!({
                "link": app.msc.link.state.label(),
                "backend": app.msc.link.backend,
                "port": app.msc.link.port_name,
                "mode": app.msc.mode.label(),
                "soft_takeover": app.msc.config.soft_takeover,
                "learn_armed": app.msc.learn.as_ref().map(crate::msc_mixer::LearnTarget::describe),
                "bindings": bindings,
                "received": app.msc.link.received,
                "sent": app.msc.link.sent,
                "reconnects": app.msc.link.reconnects,
            })
        }
        "window.vst_folders" => toggle(&mut app.ui.show_vst_folders),
        "window.shortcuts" => toggle(&mut app.ui.show_shortcuts),
        "window.color_palette" => toggle(&mut app.ui.show_color_palette),
        "window.disk_usage" => toggle(&mut app.ui.show_disk_usage),
        "window.system_usage" => toggle(&mut app.ui.show_system_usage),
        "window.task_manager" => toggle(&mut app.ui.show_task_manager),
        "window.metadata" => toggle(&mut app.ui.show_metadata),
        "window.event_list" => toggle(&mut app.ui.show_event_list),
        "window.midi_keyboard" => toggle(&mut app.ui.show_midi_keyboard),
        "window.mixer_dock" => toggle(&mut app.ui.show_mixer_dock),
        "window.midi_monitor" => toggle(&mut app.ui.show_midi_monitor),
        "window.midi_devices" => toggle(&mut app.ui.show_midi_devices),
        "window.fx_browser" => {
            if app.ui.show_fx_browser.is_some() {
                app.ui.show_fx_browser = None;
                json!({"value": false})
            } else {
                let target = app.engine.session().edit.selected_tracks.first().copied().or_else(|| app.engine.session().tracks.first().map(|t| t.id));
                app.ui.show_fx_browser = target;
                json!({"value": target.is_some()})
            }
        }
        "window.workspace" | "window.workspace_presets" | "window.workspace_sounds" | "window.workspace_front" => {
            app.ui.show_workspace = true;
            json!({"value": true})
        }
        "window.workspace_close" => {
            app.ui.show_workspace = false;
            json!({"value": false})
        }
        "window.configurations" => toggle(&mut app.ui.show_configurations),
        "window.config_new" | "window.config_update" => {
            if let Err(error) = app.ui.docking.validate().and_then(|()| app.ui.mix_docking.validate_mix()) {
                return Some(Err(error));
            }
            let mut snap = app.ui.clone();
            snap.configurations.clear();
            let v = serde_json::to_value(&snap).unwrap_or_default();
            if id == "window.config_update" && !app.ui.configurations.is_empty() {
                if let Some(last) = app.ui.configurations.last_mut() {
                    last.1 = v;
                }
            } else {
                let n = app.ui.configurations.len() + 1;
                app.ui.configurations.push((format!("Configuration {n}"), v));
            }
            json!({"configurations": app.ui.configurations.len()})
        }
        "window.arrange_tile" | "window.arrange_tile_h" | "window.arrange_tile_v" | "window.arrange_cascade" => {
            if let Err(error) = crate::docking::arrange(app) {
                app.ui.status = format!("Panel windows could not be arranged: {error}");
            }
            app.arrange_request = true;
            json!({})
        }
        "window.universe" => toggle(&mut app.ui.show_universe),
        "window.video" => toggle(&mut app.ui.show_video),
        "window.video_universe" => toggle(&mut app.ui.show_video_universe),
        "window.close" | "window.hide_floating" => {
            app.ui.show_transport = false;
            app.ui.show_big_counter = false;
            app.ui.show_memory_locations = false;
            app.ui.show_undo_history = false;
            app.ui.show_session_info = false;
            app.ui.show_audio_health = false;
            app.ui.show_automation = false;
            app.ui.plugin_windows.clear();
            app.ui.audiosuite = None;
            json!({})
        }
        "view.mix_section" => {
            let sec = p.get("section").and_then(Value::as_str).unwrap_or("");
            match sec {
                "all" => {
                    app.ui.mix_views =
                        ["inserts_ae", "inserts_fj", "sends_ae", "sends_fj", "io", "comments", "color"].iter().map(|s| s.to_string()).collect()
                }
                "minimal" => app.ui.mix_views.clear(),
                "" => return Some(Err("`section` required".into())),
                s => {
                    if app.ui.mix_views.iter().any(|v| v == s) {
                        app.ui.mix_views.retain(|v| v != s);
                    } else {
                        app.ui.mix_views.push(s.to_string());
                    }
                }
            }
            json!({"mix_views": app.ui.mix_views})
        }
        "ui.new_tracks_dialog" => {
            return Some(Err("ui.new_tracks_dialog opens a dialog from the menu only; call track.new with parameters".into()));
        }
        "ui.bounce_dialog" => {
            return Some(Err("ui.bounce_dialog opens a dialog from the menu only; call file.bounce_mix with a path".into()));
        }
        "ui.theme" => {
            let Some(mode) = p.get("mode").and_then(Value::as_str) else {
                return Some(Ok(json!({"mode": app.ui.theme.id()})));
            };
            let Some(mode) = crate::theme::ThemeMode::parse(mode) else {
                return Some(Err(format!("unknown theme `{mode}` (system, light, dark, studiopro, cyberpunk, vintage)")));
            };
            app.ui.theme = mode;
            json!({"mode": mode.id()})
        }
        "ui.set" => match serde_json::to_value(&app.ui) {
            Ok(mut cur) => {
                if let (Some(obj), Some(patch)) = (cur.as_object_mut(), p.as_object()) {
                    for (k, v) in patch {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                match serde_json::from_value::<crate::UiState>(cur) {
                    Ok(ns) => {
                        if let Err(error) = ns.docking.validate().and_then(|()| ns.mix_docking.validate_mix()) {
                            return Some(Err(error));
                        }
                        crate::docking::cancel_replaced_state(app, &ns);
                        app.ui = ns;
                        json!({"ui": app.ui})
                    }
                    Err(e) => return Some(Err(format!("bad ui state: {e}"))),
                }
            }
            Err(e) => return Some(Err(e.to_string())),
        },
        _ => return None,
    };
    Some(Ok(r))
}

/// Parity including UI-layer commands.
pub fn parity() -> Value {
    let extra = ui_aliases();
    catalog::parity_with(&extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, SoundApp};

    #[test]
    fn mix_view_aliases_are_allocated_once() {
        let first = ui_aliases();
        let second = ui_aliases();
        let a = first.iter().find(|(path, _)| path.contains("Mix Window Views > All")).map(|(path, _)| *path).unwrap();
        let b = second.iter().find(|(path, _)| path.contains("Mix Window Views > All")).map(|(path, _)| *path).unwrap();
        assert!(std::ptr::eq(a, b));
        assert_eq!(first.iter().filter(|(path, _)| path.contains("Mix Window Views")).count(), 14);
    }

    #[test]
    fn midi_monitor_and_devices_are_available_in_the_window_menu() {
        let roots = tree();
        let window = roots.iter().find(|node| node.label == "Window").unwrap();
        for (label, path, command) in
            [("MIDI Monitor", "Window > MIDI Monitor", "window.midi_monitor"), ("MIDI Devices", "Window > MIDI Devices", "window.midi_devices")]
        {
            assert!(window.children.iter().any(|node| node.label == label), "{label} is missing");
            assert_eq!(command_for_path(path, &ui_aliases()).as_deref(), Some(command));
        }
    }

    #[test]
    fn programmatic_dialog_commands_stay_closed_and_the_menu_opens_them() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        let err = app.run("ui.new_tracks_dialog", json!({})).unwrap_err();
        assert!(err.contains("track.new"), "{err}");
        let err = app.run("ui.bounce_dialog", json!({})).unwrap_err();
        assert!(err.contains("file.bounce_mix"), "{err}");
        assert!(app.dialogs.open_name().is_none());

        invoke_menu(&mut app, "ui.new_tracks_dialog", "Track > New...");
        assert_eq!(app.dialogs.open_name(), Some("new_tracks"));
        app.dialogs.open = None;
        invoke_menu(&mut app, "ui.bounce_dialog", "File > Bounce Mix...");
        assert_eq!(app.dialogs.open_name(), Some("bounce"));
    }

    #[test]
    fn save_asks_for_a_path_until_the_session_has_one() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        invoke_menu(&mut app, "session.save", "File > Save Session");
        assert!(matches!(&app.dialogs.open, Some(crate::dialogs::Dialog::PathPrompt { cmd, .. }) if cmd == "session.save_as"));
        app.dialogs.open = None;
        assert!(invoke_shortcut_dialog(&mut app, "session.save"));
        app.dialogs.open = None;

        let dir = std::env::temp_dir().join(format!("soundcraft-save-{}", std::process::id()));
        let path = dir.join("Song.scraft");
        app.run("session.save_as", json!({"path": path.to_string_lossy()})).unwrap();
        app.run("track.new", json!({})).unwrap();
        assert!(!invoke_shortcut_dialog(&mut app, "session.save"));
        invoke_menu(&mut app, "session.save", "File > Save Session");
        assert!(app.dialogs.open.is_none());
        assert!(!app.engine.is_dirty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
