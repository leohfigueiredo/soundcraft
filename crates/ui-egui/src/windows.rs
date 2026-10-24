//! Secondary floating windows (Window menu).

use crate::SoundApp;
use crate::theme::{Tokens, mono, rgb};
use egui::{Sense, vec2};
use serde_json::json;
use soundcraft_model::{AutomationMode, ClipContent};
use soundcraft_playback::AudioConfig;
use soundcraft_playback::devices::{AudioSystem, DeviceInfo};
use std::sync::Arc;

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    audio_health(app, ctx);
    automation(app, ctx);
    color_palette(app, ctx);
    disk_usage(app, ctx);
    system_usage(app, ctx);
    task_manager(app, ctx);
    metadata(app, ctx);
    event_list(app, ctx);
    midi_keyboard(app, ctx);
    workspace(app, ctx);
    configurations(app, ctx);
    playback_engine(app, ctx);
    io_setup(app, ctx);
    shortcuts_window(app, ctx);
    clip_effects(app, ctx);
    msc_hardware(app, ctx);
    midi_monitor(app, ctx);
    midi_devices(app, ctx);
    crate::fx_browser::show(app, ctx);
    crate::fx_browser::show_vst_paths_window(app, ctx);
}

fn audio_health(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_audio_health;
    win(ctx, &mut open, "Session Audio Health", vec2(620.0, 360.0), |ui| {
        let refresh = ui.button("Refresh Report").clicked();
        if app.audio_health_report.is_none() || refresh {
            app.audio_health_report = Some(app.engine.execute("session.audio_health", &json!({})).map_err(|error| error.to_string()));
        }
        let Some(result) = &app.audio_health_report else { return };
        match result {
            Ok(report) => {
                ui.label(format!("{} audio clips checked · {} issues", report["audio_clips"], report["issue_count"]));
                ui.label(match report["active_audio_end_seconds"].as_f64() {
                    Some(seconds) => format!("Active audio ends at {seconds:.2} seconds"),
                    None => "Active audio end time is unknown (invalid session sample rate)".into(),
                });
                ui.label("Includes alternate takes and muted clips. Refresh after editing.");
                ui.separator();
                if report["healthy"] == true {
                    ui.label("All audio clips have loaded media within source bounds.");
                }
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    if let Some(issues) = report["issues"].as_array() {
                        for issue in issues {
                            ui.group(|ui| {
                                let text = |key: &str| issue[key].as_str().unwrap_or("");
                                ui.label(format!(
                                    "{} / {} / {}{}",
                                    text("track"),
                                    text("playlist"),
                                    text("clip"),
                                    if issue["active"] == true { "" } else { " (alternate playlist)" }
                                ));
                                ui.label(text("message"));
                                if let Some(path) = issue["path"].as_str() {
                                    ui.label(path);
                                }
                            });
                        }
                    }
                });
                ui.separator();
                ui.label("Checks loaded media only, not files on disk, signal levels, plugins or routing.");
            }
            Err(error) => {
                ui.label(error.to_string());
            }
        }
    });
    app.ui.show_audio_health = open;
    if !open {
        app.audio_health_report = None;
    }
}

fn win(ctx: &egui::Context, open: &mut bool, title: &str, size: egui::Vec2, body: impl FnOnce(&mut egui::Ui)) {
    if !*open {
        return;
    }
    egui::Window::new(title).open(open).default_size(size).show(ctx, body);
}

fn automation(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_automation;
    win(ctx, &mut open, "Automation", vec2(260.0, 260.0), |ui| {
        ui.label("Write automation for the selected tracks:");
        let tracks = app.engine.session().edit.selected_tracks.clone();
        ui.horizontal_wrapped(|ui| {
            for m in [
                AutomationMode::Off,
                AutomationMode::Read,
                AutomationMode::Touch,
                AutomationMode::Latch,
                AutomationMode::TouchLatch,
                AutomationMode::Write,
                AutomationMode::Trim,
            ] {
                if ui.button(m.label()).clicked() {
                    let ids: Vec<u64> = tracks.iter().map(|t| t.0).collect();
                    let _ = app.run("mix.automation_mode", json!({"tracks": ids, "mode": m.label()}));
                }
            }
        });
        ui.separator();
        ui.label("Enabled parameters");
        for f in ["volume", "pan", "mute", "send level", "send pan", "send mute", "plugin"] {
            let id = format!("automation.enable.{}", f.replace(' ', "_"));
            let mut on = !app.engine.session().edit.flag(&format!("{id}.off"));
            if crate::widgets::checkbox(ui, &mut on, f).changed() {
                app.engine.session_mut().edit.set_flag(&format!("{id}.off"), !on);
            }
        }
        ui.separator();
        if ui.button("Write to Current").clicked() {
            let _ = app.run("automation.write_to_current", json!({}));
        }
        if ui.button("Thin All").clicked() {
            let _ = app.run("automation.thin_all", json!({}));
        }
    });
    app.ui.show_automation = open;
}

fn color_palette(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_color_palette;
    win(ctx, &mut open, "Color Palette", vec2(300.0, 140.0), |ui| {
        ui.label("Apply to selected tracks (Shift: selected clips)");
        ui.horizontal_wrapped(|ui| {
            for (i, c) in soundcraft_model::TRACK_COLORS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                ui.painter().rect_filled(r, 3.0, rgb(*c));
                if resp.clicked() {
                    if ui.input(|x| x.modifiers.shift) {
                        let _ = app.run("clip.color", json!({"color": c}));
                    } else {
                        let _ = app.run("track.color", json!({"color": i}));
                    }
                }
            }
        });
        ui.separator();
        ui.add(egui::Slider::new(&mut app.ui.track_tint, 0.0..=1.0).show_value(false).text("Track tint in the Edit window"));
    });
    app.ui.show_color_palette = open;
}

fn disk_usage(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_disk_usage;
    win(ctx, &mut open, "Disk Usage", vec2(360.0, 160.0), |ui| {
        let s = app.engine.session();
        let bytes: u64 = s.sources.iter().map(|x| x.frames * u64::from(x.channels) * 4).sum();
        ui.label(format!("Session media: {} files, {:.1} MB in memory (32-bit float)", s.sources.len(), bytes as f64 / 1_048_576.0));
        let per_min = f64::from(s.sample_rate.hz()) * 60.0 * 3.0 / 1_048_576.0;
        ui.label(format!("Recording at 24-bit uses {per_min:.1} MB per mono track-minute"));
        if let Some(p) = &app.engine.path {
            ui.label(format!("Session file: {p}"));
        }
    });
    app.ui.show_disk_usage = open;
}

fn system_usage(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_system_usage;
    win(ctx, &mut open, "System Usage", vec2(320.0, 140.0), |ui| {
        ui.label(format!("UI frame time: {:.1} ms", app.frame_ms));
        ui.label(format!("Audio: {}", app.player.as_ref().map_or("no engine".to_string(), |p| format!("{} @ {} Hz", p.device_name, p.device_rate))));
        let plugins: usize = app.engine.session().tracks.iter().map(|t| t.mixer.inserts.iter().flatten().count()).sum();
        ui.label(format!("Active plugin instances: {plugins}"));
        ui.label(format!("Tracks: {}", app.engine.session().tracks.len()));
        ui.label(format!("Undo steps: {}", app.engine.undo_history().len()));
    });
    app.ui.show_system_usage = open;
}

fn task_manager(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_task_manager;
    win(ctx, &mut open, "Task Manager", vec2(320.0, 120.0), |ui| {
        match &app.bounce {
            Some(b) => {
                ui.label(format!("Bouncing {} ({:.0}%)", b.path().display(), b.progress() * 100.0));
                ui.add(egui::ProgressBar::new(b.progress()));
            }
            None => {
                ui.label("No background tasks are running.");
            }
        }
        ui.label(
            egui::RichText::new("Renders and AudioSuite processes run to completion before returning.").small().color(Tokens::current().text_dim),
        );
    });
    app.ui.show_task_manager = open;
}

fn metadata(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_metadata;
    win(ctx, &mut open, "Metadata Inspector", vec2(380.0, 260.0), |ui| {
        let s = app.engine.session();
        ui.label(egui::RichText::new(&s.name).strong());
        egui::Grid::new("meta").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in [
                ("Sample rate", format!("{} Hz", s.sample_rate.hz())),
                ("Bit depth", format!("{:?}", s.bit_depth)),
                ("Timecode rate", s.frame_rate.label().to_string()),
                ("Tracks", s.tracks.len().to_string()),
                ("Clips", s.tracks.iter().map(|t| t.clips().len()).sum::<usize>().to_string()),
                ("Audio files", s.sources.len().to_string()),
                ("Markers", s.markers.len().to_string()),
                ("Length", s.format_position(s.content_end(), soundcraft_time::TimeFormat::MinSecs)),
            ] {
                ui.label(k);
                ui.label(v);
                ui.end_row();
            }
        });
        ui.separator();
        let mut c = s.comments.clone();
        ui.label("Session comments");
        if ui.text_edit_multiline(&mut c).changed() {
            app.engine.session_mut().comments = c;
        }
    });
    app.ui.show_metadata = open;
}

fn event_list(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_event_list;
    win(ctx, &mut open, "MIDI Event List", vec2(420.0, 360.0), |ui| {
        let Some(cid) = crate::midi_editor::target_clip(app) else {
            ui.label("Select a MIDI clip.");
            return;
        };
        let notes = match app.engine.session().find_clip(cid).map(|(_, c)| c.content.clone()) {
            Some(ClipContent::Midi { sequence }) => sequence.notes,
            _ => Vec::new(),
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("evl").num_columns(5).striped(true).show(ui, |ui| {
                for h in ["Start (ticks)", "Note", "Vel", "Length", ""] {
                    ui.label(egui::RichText::new(h).strong());
                }
                ui.end_row();
                for (i, n) in notes.iter().enumerate() {
                    ui.label(egui::RichText::new(n.start.to_string()).font(mono(11.0)));
                    ui.label(soundcraft_midi::note_name(n.pitch));
                    let mut v = i64::from(n.velocity);
                    if ui.add(egui::DragValue::new(&mut v).range(1..=127)).changed() {
                        let _ = app.engine.execute("midi.note_edit", &json!({"clip": cid.0, "index": i, "velocity": v}));
                    }
                    ui.label(n.length.to_string());
                    if ui.small_button("✕").clicked() {
                        let _ = app.run("midi.note_delete", json!({"clip": cid.0, "indices": [i]}));
                    }
                    ui.end_row();
                }
            });
        });
    });
    app.ui.show_event_list = open;
}

fn midi_keyboard(_app: &mut SoundApp, _ctx: &egui::Context) {
    // The horizontal virtual piano keyboard is integrated into the bottom dock of the Edit window.
}

fn workspace(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_workspace;
    if !open {
        return;
    }
    egui::Window::new("Workspace").open(&mut open).default_size(vec2(460.0, 420.0)).show(ctx, |ui| {
        let dir = if app.ui.workspace_dir.is_empty() { std::env::var("HOME").unwrap_or_else(|_| ".".into()) } else { app.ui.workspace_dir.clone() };
        ui.horizontal(|ui| {
            if ui.button("⬆").on_hover_text("Parent folder").clicked()
                && let Some(p) = std::path::Path::new(&dir).parent()
            {
                app.ui.workspace_dir = p.to_string_lossy().into_owned();
            }
            ui.label(egui::RichText::new(&dir).small());
        });
        ui.separator();
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
                        let ext = std::path::Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                        (is_dir || ["wav", "aif", "aiff", "flac", "mp3", "ogg", "m4a", "m4b", "caf", "mid", "midi", "scraft"].contains(&ext.as_str()))
                            .then_some((name, is_dir))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (name, is_dir) in entries.into_iter().take(2000) {
                let path = std::path::Path::new(&dir).join(&name).to_string_lossy().into_owned();
                let label = if is_dir { format!("📁 {name}") } else { format!("🔊 {name}") };
                let r = ui.selectable_label(false, label);
                if r.double_clicked() {
                    if is_dir {
                        app.ui.workspace_dir = path;
                    } else if name.ends_with(".scraft") {
                        let _ = crate::dialogs::run_or_ask(app, "session.open", json!({"path": path}));
                    } else if name.ends_with(".mid") || name.ends_with(".midi") {
                        let _ = app.run("file.import_midi", json!({"path": path}));
                    } else {
                        let at = app.engine.session().edit.selection.start;
                        let _ = app.run("file.import_audio", json!({"path": path, "at": at}));
                    }
                }
            }
        });
        ui.label(egui::RichText::new("Double-click to import audio/MIDI at the insertion point or open a session.").small());
    });
    app.ui.show_workspace = open;
}

fn configurations(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_configurations;
    win(ctx, &mut open, "Window Configurations", vec2(320.0, 240.0), |ui| {
        if ui.button("New Configuration…").clicked() {
            let n = app.ui.configurations.len() + 1;
            let mut snap = app.ui.clone();
            snap.configurations.clear();
            if let Ok(v) = serde_json::to_value(&snap) {
                app.ui.configurations.push((format!("Configuration {n}"), v));
            }
        }
        ui.separator();
        let list = app.ui.configurations.clone();
        for (i, (name, v)) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(format!("{}  {name}", i + 1)).clicked()
                    && let Ok(mut st) = serde_json::from_value::<crate::UiState>(v.clone())
                {
                    if let Err(error) = st.docking.validate().and_then(|()| st.mix_docking.validate_mix()) {
                        app.ui.status = format!("Window configuration could not be restored: {error}");
                    } else {
                        st.configurations = app.ui.configurations.clone();
                        st.theme = app.ui.theme;
                        st.show_configurations = true;
                        crate::docking::cancel_replaced_state(app, &st);
                        app.ui = st;
                    }
                }
                if ui.small_button("✕").clicked() {
                    app.ui.configurations.remove(i);
                }
            });
        }
    });
    app.ui.show_configurations = open;
}

/// The audio systems as last enumerated (enumerating probes devices: done when the window opens
/// and on Refresh, not every frame).
fn audio_systems(ctx: &egui::Context, refresh: bool) -> Arc<Vec<AudioSystem>> {
    let id = egui::Id::new("audio_systems");
    if !refresh && let Some(s) = ctx.data(|d| d.get_temp::<Arc<Vec<AudioSystem>>>(id)) {
        return s;
    }
    let s = Arc::new(soundcraft_playback::devices::audio_systems());
    ctx.data_mut(|d| d.insert_temp(id, Arc::clone(&s)));
    s
}

/// The system a config means (its own, else the default one) in an enumeration.
fn chosen_system<'a>(systems: &'a [AudioSystem], cfg: &AudioConfig) -> Option<&'a AudioSystem> {
    cfg.system
        .as_deref()
        .and_then(|n| systems.iter().find(|s| s.name.eq_ignore_ascii_case(n)))
        .or_else(|| systems.iter().find(|s| s.is_default))
        .or(systems.first())
}

/// The device a config means (by name, else the system default).
fn chosen_device<'a>(devices: &'a [DeviceInfo], name: Option<&str>) -> Option<&'a DeviceInfo> {
    name.and_then(|n| devices.iter().find(|d| d.name == n)).or_else(|| devices.iter().find(|d| d.is_default))
}

fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<String>, devices: &[DeviceInfo]) {
    let default = devices.iter().find(|d| d.is_default).map_or("none".to_string(), |d| d.name.clone());
    let shown = value.clone().unwrap_or_else(|| format!("Default ({default})"));
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(value, None, format!("Default ({default})"));
        for d in devices {
            ui.selectable_value(value, Some(d.name.clone()), format!("{} ({} ch)", d.name, d.max_channels));
        }
    });
}

fn number_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<u32>, options: &[u32], unit: &str) {
    let shown = value.map_or("Device default".to_string(), |v| format!("{v} {unit}"));
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Device default");
        for v in options {
            ui.selectable_value(value, Some(*v), format!("{v} {unit}"));
        }
    });
}

/// Setup › Playback Engine / Hardware: choose the audio system, devices, rate and buffer size.
pub fn playback_engine(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_playback_engine;
    if !open {
        return;
    }
    let draft_id = egui::Id::new("audio_draft");
    let mut draft = ctx.data(|d| d.get_temp::<AudioConfig>(draft_id)).unwrap_or_else(|| app.ui.audio.clone());
    let mut refresh = false;
    let mut apply = false;
    let systems = audio_systems(ctx, false);
    win(ctx, &mut open, "Playback Engine", vec2(460.0, 360.0), |ui| {
        let t = Tokens::current();
        egui::Grid::new("pe").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label("Audio system");
            let def = systems.iter().find(|s| s.is_default).map_or("none".to_string(), |s| s.name.clone());
            let shown = draft.system.clone().unwrap_or_else(|| format!("Default ({def})"));
            egui::ComboBox::from_id_salt("pe-system").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                let before = draft.system.clone();
                ui.selectable_value(&mut draft.system, None, format!("Default ({def})"));
                for s in systems.iter() {
                    ui.selectable_value(&mut draft.system, Some(s.name.clone()), &s.name);
                }
                if draft.system != before {
                    // Device names belong to a system.
                    draft.output = None;
                    draft.input = None;
                }
            });
            ui.end_row();
            let sys = chosen_system(&systems, &draft);
            ui.label("Output device");
            device_combo(ui, "pe-out", &mut draft.output, sys.map_or(&[][..], |s| &s.outputs));
            ui.end_row();
            ui.label("Input device");
            device_combo(ui, "pe-in", &mut draft.input, sys.map_or(&[][..], |s| &s.inputs));
            ui.end_row();
            let out = sys.and_then(|s| chosen_device(&s.outputs, draft.output.as_deref()));
            ui.label("Sample rate");
            number_combo(ui, "pe-rate", &mut draft.sample_rate, out.map_or(&[][..], |d| &d.sample_rates), "Hz");
            ui.end_row();
            ui.label("Buffer size");
            let buffers = out.map(DeviceInfo::buffer_sizes).unwrap_or_else(|| soundcraft_playback::devices::STANDARD_BUFFERS.to_vec());
            number_combo(ui, "pe-buffer", &mut draft.buffer_size, &buffers, "samples");
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh devices").clicked() {
                refresh = true;
            }
            let changed = draft != app.ui.audio;
            if ui.add_enabled(changed, egui::Button::new("Apply")).on_hover_text("Reopen the audio devices with these settings").clicked() {
                apply = true;
            }
            if changed && ui.button("Revert").clicked() {
                draft = app.ui.audio.clone();
            }
        });
        ui.separator();
        egui::Grid::new("pe-state").num_columns(2).show(ui, |ui| {
            ui.label("Output");
            ui.label(app.player.as_ref().map_or("none".to_string(), |p| {
                if p.silent {
                    "no device (silent clock)".to_string()
                } else {
                    let buf = p.device_buffer.map_or("default buffer".to_string(), |b| format!("{b} samples"));
                    format!("{} · {} Hz · {} ch · {buf}", p.device_name, p.device_rate, p.device_channels)
                }
            }));
            ui.end_row();
            ui.label("Audio thread");
            ui.label(app.player.as_ref().map_or("none".to_string(), |p| p.realtime().describe()));
            ui.end_row();
            ui.label("Input");
            ui.label(app.recorder.as_ref().map_or("opened on first record or monitor".to_string(), |r| {
                format!("{} · {} Hz · {} ch", r.device_name, r.sample_rate, r.channels)
            }));
            ui.end_row();
            ui.label("Session rate");
            let sr = app.engine.session().sample_rate.hz();
            let note = app
                .player
                .as_ref()
                .filter(|p| !p.silent && p.device_rate != sr)
                .map_or(String::new(), |_| " (resampled to the device rate)".to_string());
            ui.label(format!("{sr} Hz{note}"));
            ui.end_row();
        });
        ui.label(egui::RichText::new("The mix runs in 512-sample blocks; the buffer size sets the device's own latency.").small().color(t.text_dim));
        let mut dc = app.engine.session().edit.delay_compensation;
        if crate::widgets::checkbox(ui, &mut dc, "Delay compensation").changed() {
            let _ = app.run("options.delay_compensation", json!({"value": dc}));
        }
        if ui.button("Reconnect audio device").clicked() {
            app.reconnect_audio();
        }
    });
    if refresh {
        audio_systems(ctx, true);
    }
    if apply && let Err(e) = app.apply_audio_config(draft.clone()) {
        app.ui.status = e;
    }
    if open {
        ctx.data_mut(|d| d.insert_temp(draft_id, draft));
    } else {
        // Closing discards unapplied edits and the device list (re-read on the next open).
        ctx.data_mut(|d| {
            d.remove_temp::<AudioConfig>(draft_id);
            d.remove_temp::<Arc<Vec<AudioSystem>>>(egui::Id::new("audio_systems"));
        });
    }
    app.ui.show_playback_engine = open;
}

/// `ui.audio_device`: `{list?: bool}` returns the audio systems and their devices;
/// `{system?, output?, input?, sample_rate?, buffer_size?}` (a `null` value means "default") sets
/// the choice and reopens the devices. Always returns the choice and the open device.
pub fn audio_device_command(app: &mut SoundApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    let mut changed = false;
    let mut cfg = app.ui.audio.clone();
    for (key, slot) in [("system", &mut cfg.system), ("output", &mut cfg.output), ("input", &mut cfg.input)] {
        if let Some(v) = p.get(key) {
            *slot = v.as_str().map(str::to_string).filter(|s| !s.is_empty());
            changed = true;
        }
    }
    for (key, slot, range) in [("sample_rate", &mut cfg.sample_rate, 8_000..=768_000u64), ("buffer_size", &mut cfg.buffer_size, 16..=8192u64)] {
        if let Some(v) = p.get(key) {
            *slot = match v {
                Value::Null => None,
                v => Some(v.as_u64().filter(|n| range.contains(n)).ok_or_else(|| format!("`{key}` must be a number in {range:?} or null"))? as u32),
            };
            changed = true;
        }
    }
    if changed {
        app.apply_audio_config(cfg)?;
    }
    let device = app.player.as_ref().map(|pl| {
        json!({"name": pl.device_name, "sample_rate": pl.device_rate, "channels": pl.device_channels, "buffer_size": pl.device_buffer, "silent": pl.silent, "audio_thread": pl.realtime().describe()})
    });
    let mut out = json!({"config": app.ui.audio, "device": device});
    if p.get("list").and_then(Value::as_bool).unwrap_or(false) {
        out["systems"] = json!(soundcraft_playback::devices::audio_systems());
    }
    Ok(out)
}

/// Setup › I/O: busses and output paths.
pub fn io_setup(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_io_setup;
    win(ctx, &mut open, "I/O Setup", vec2(420.0, 300.0), |ui| {
        ui.label(egui::RichText::new("Busses").strong());
        let busses: Vec<(String, String)> = app.engine.session().busses.iter().map(|b| (b.name.clone(), b.format.label().to_string())).collect();
        egui::Grid::new("busses").num_columns(3).striped(true).show(ui, |ui| {
            for (name, fmt) in busses {
                let key = egui::Id::new(("bus_name", name.clone()));
                let mut edit: String = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_else(|| name.clone());
                let r = ui.text_edit_singleline(&mut edit);
                ui.ctx().memory_mut(|m| m.data.insert_temp(key, edit.clone()));
                if r.lost_focus() && edit != name {
                    let _ = app.run("setup.io", json!({"action": "rename_bus", "name": name, "new_name": edit}));
                }
                ui.label(fmt);
                if ui.small_button("Delete").clicked() {
                    let _ = app.run("setup.io", json!({"action": "delete_bus", "name": name}));
                }
                ui.end_row();
            }
        });
        if ui.button("New Bus").clicked() {
            let n = app.engine.session().busses.len() + 1;
            let _ = app.run("mix.new_bus", json!({"name": format!("Bus {n}")}));
        }
        ui.separator();
        ui.label(egui::RichText::new("Outputs").strong());
        // Main output format: stereo, or a surround format (the mix and master faders follow).
        let main = app.engine.session().main_format();
        ui.horizontal(|ui| {
            ui.label("Main output");
            egui::ComboBox::from_id_salt("main_output_format").selected_text(main.label()).show_ui(ui, |ui| {
                for f in soundcraft_model::ChannelFormat::ALL.into_iter().filter(|f| (2..=16).contains(&f.channels())) {
                    if ui.selectable_label(f == main, f.label()).clicked() && f != main {
                        let _ = app.run("setup.main_format", json!({"format": f.label()}));
                    }
                }
            });
            ui.label(egui::RichText::new(format!("{} ch", main.channels())).small());
        });
        for o in app.engine.session().outputs.clone() {
            ui.label(format!("{} — {} from channel {}", o.name, o.format.label(), o.first_channel + 1));
        }
    });
    app.ui.show_io_setup = open;
}

/// Setup › Keyboard Shortcuts.
pub fn shortcuts_window(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_shortcuts;
    let mac = ctx.os().is_mac();
    win(ctx, &mut open, "Keyboard Shortcuts", vec2(520.0, 480.0), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sc").num_columns(3).striped(true).show(ui, |ui| {
                let engine = soundcraft_engine::command_specs().iter().map(|c| (c.shortcut, c.label, c.menu.join(" › ")));
                let windows = crate::menus::UI_COMMANDS.iter().map(|c| (c.3, c.1, c.2.replace(" > ", " › ")));
                for (shortcut, label, menu) in engine.chain(windows) {
                    let Some(shortcut) = shortcut else { continue };
                    ui.label(egui::RichText::new(crate::shortcuts::shortcut_label(shortcut, mac)).font(mono(11.0)));
                    ui.label(label);
                    ui.label(egui::RichText::new(menu).small());
                    ui.end_row();
                }
            });
        });
    });
    app.ui.show_shortcuts = open;
}

/// Clip Effects: per-clip EQ, dynamics and gain (stored as `clip_fx.<clip>.*` values).
pub fn clip_effects(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_clip_effects;
    win(ctx, &mut open, "Clip Effects", vec2(560.0, 420.0), |ui| {
        let clip = app.engine.session().edit.selected_clips.first().copied().or_else(|| {
            let s = app.engine.session();
            s.tracks
                .iter()
                .filter(|t| s.edit.selected_tracks.contains(&t.id))
                .flat_map(|t| t.clips().iter())
                .find(|c| c.range().contains(s.edit.selection.start) && c.is_audio())
                .map(|c| c.id)
        });
        let Some(cid) = clip else {
            ui.label("Select an audio clip.");
            return;
        };
        let name = app.engine.session().find_clip(cid).map(|(_, c)| c.name.clone()).unwrap_or_default();
        let bypass = app.engine.session().edit.flag(&format!("clip_fx.bypass.{}", cid.0));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).strong());
            if ui.selectable_label(bypass, "Bypass").clicked() {
                let _ = app.run("clip.effects_bypass", json!({"clips": [cid.0]}));
            }
            if ui.button("Render").clicked() {
                let _ = app.run("clip.effects_render", json!({"clips": [cid.0]}));
            }
            if ui.button("Clear").clicked() {
                let _ = app.run("edit.clear_clip_effects", json!({"clips": [cid.0]}));
            }
        });
        let get = |app: &SoundApp, k: &str, d: f32| app.engine.session().edit.values.get(&format!("clip_fx.{}.{k}", cid.0)).map_or(d, |v| *v as f32);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |cols| {
                for (col, (title, plugin, prefix)) in cols.iter_mut().zip([("EQ", "eq_7band", "eq."), ("Dynamics", "compressor", "comp.")]) {
                    col.label(egui::RichText::new(title).strong());
                    let Some(info) = soundcraft_dsp::plugin_info(plugin) else { continue };
                    for p in info.params {
                        let key = format!("{prefix}{}", p.id);
                        let mut v = get(app, &key, p.default);
                        let before = v;
                        col.add(egui::Slider::new(&mut v, p.min..=p.max).text(p.name));
                        if (v - before).abs() > f32::EPSILON {
                            let mut m = serde_json::Map::new();
                            m.insert(key.clone(), json!(v));
                            let _ = app.engine.execute_merged(
                                "clip.effects_set",
                                &json!({"clips": [cid.0], "params": m}),
                                &format!("cfx:{}:{key}", cid.0),
                            );
                        }
                    }
                }
            });
            let mut g = get(app, "gain", 0.0);
            let before = g;
            ui.add(egui::Slider::new(&mut g, -36.0..=24.0).text("Clip effects gain (dB)"));
            if (g - before).abs() > f32::EPSILON {
                let _ =
                    app.engine.execute_merged("clip.effects_set", &json!({"clips": [cid.0], "params": {"gain": g}}), &format!("cfx:{}:gain", cid.0));
            }
        });
    });
    app.ui.show_clip_effects = open;
}

/// MIDI Devices: the Reaper-like device list. Every readable MIDI port with two assignments —
/// `Input` (notes to the selected instrument track) and `Control` (faders/buttons as the control
/// surface) — plus a live activity light.
pub fn midi_devices(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_midi_devices;
    let t = Tokens::current();
    win(ctx, &mut open, "MIDI Devices", vec2(640.0, 430.0), |ui| {
        use crate::midi_io::LinkState;
        ui.horizontal(|ui| {
            let (dot, col) = match app.msc.link.state {
                LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                LinkState::Searching => ("◐", t.meter_yellow),
                LinkState::Unavailable => ("○", t.meter_red),
            };
            ui.label(egui::RichText::new(format!("{dot} sequencer {}", app.msc.link.state.label())).color(col));
            ui.label(egui::RichText::new(&app.msc.link.backend).small().color(t.text_dim));
            if ui.small_button("Rescan").clicked() {
                app.control_surface_rescan();
            }
            if ui.small_button("MIDI Monitor").clicked() {
                app.ui.show_midi_monitor = true;
            }
            if ui.small_button("Bluetooth (BLE)").clicked() {
                let on = !app.msc.ble;
                app.control_surface_ble(on);
            }
            if app.msc.ble {
                ui.label(egui::RichText::new("BLE on").small().color(t.accent));
            }
        });
        ui.separator();
        ui.label(egui::RichText::new("Enable each device the way Reaper does: Input = notes, Control = faders/buttons.").small().color(t.text_dim));
        let ports = app.msc.link.ports.clone();
        let now = ui.input(|i| i.time);
        let flash = app.msc.input_active(now, 0.6);
        let keyboard = app.msc.keyboard_port.clone();
        let surface = app.msc.surface_port();
        if ports.is_empty() {
            ui.label(
                egui::RichText::new("No MIDI ports. Check: /dev/snd/seq exists, user in the 'audio' group, device on USB.").color(t.meter_yellow),
            );
        }
        egui::ScrollArea::vertical().id_salt("midi_devices_list").show(ui, |ui| {
            for port in &ports {
                ui.horizontal(|ui| {
                    let dot = if flash { "●" } else { "○" };
                    ui.label(egui::RichText::new(dot).color(if flash { t.accent } else { t.text_dim }));
                    let name_r = egui::RichText::new(&port.name).monospace();
                    ui.label(if port.matched { name_r.color(t.accent) } else { name_r });
                    ui.label(egui::RichText::new(&port.address).small().color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let is_surface = surface.as_deref() == Some(port.name.as_str());
                        let is_keyboard = keyboard.as_deref() == Some(port.name.as_str());
                        if ui.selectable_label(is_surface, "Control").on_hover_text("Use this port for the faders/buttons").clicked() {
                            if is_surface {
                                app.control_surface_port(None);
                            } else {
                                app.control_surface_port(Some(port.name.clone()));
                            }
                            app.control_surface_rescan();
                        }
                        if ui
                            .selectable_label(is_keyboard, "Input")
                            .on_hover_text("Use this port for notes (the selected instrument track)")
                            .clicked()
                        {
                            if is_keyboard {
                                app.control_surface_keyboard(None);
                            } else {
                                app.control_surface_keyboard(Some(port.name.clone()));
                            }
                            app.control_surface_rescan();
                        }
                    });
                });
            }
        });
        ui.separator();
        if !app.msc.link.ports.is_empty() && app.msc.link.received == 0 {
            ui.label(egui::RichText::new("Ports found but no message received yet: is the device in DAW/MCU mode?").small().color(t.meter_yellow));
        }
    });
    app.ui.show_midi_devices = open;
}

/// MIDI Monitor: every port the worker sees, what is subscribed, and the message log. This is
/// the window that answers "is the controller sending anything?".
pub fn midi_monitor(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_midi_monitor;
    let t = Tokens::current();
    win(ctx, &mut open, "MIDI Monitor", vec2(620.0, 460.0), |ui| {
        use crate::midi_io::LinkState;
        let link = app.msc.link.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Sequencer:").strong());
            let (dot, col) = match link.state {
                LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                LinkState::Searching => ("◐", t.meter_yellow),
                LinkState::Unavailable => ("○", t.meter_red),
            };
            ui.label(egui::RichText::new(format!("{dot} {}", link.state.label())).color(col));
            ui.label(egui::RichText::new(&link.backend).color(t.text_dim));
            if !link.message.is_empty() {
                ui.label(egui::RichText::new(&link.message).italics().color(t.meter_yellow));
            }
        });
        ui.horizontal(|ui| {
            ui.label(format!("Surface: {}", if link.port_name.is_empty() { "—" } else { &link.port_name }));
            ui.separator();
            ui.label(format!("Keyboard: {}", app.msc.keyboard_port.as_deref().unwrap_or("—")));
            ui.separator();
            let (kdot, kcol) = match link.keyboard_state {
                LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                LinkState::Searching => ("◐", t.meter_yellow),
                LinkState::Unavailable => ("○", t.text_dim),
            };
            ui.label(egui::RichText::new(format!("{kdot} {}", link.keyboard_state.label())).color(kcol));
            ui.separator();
            let (bdot, bcol) = match link.ble_state {
                LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                LinkState::Searching => ("◐", t.meter_yellow),
                LinkState::Unavailable => ("○", t.text_dim),
            };
            ui.label(egui::RichText::new(format!("{bdot} BLE: {}", if link.ble_device.is_empty() { "—" } else { &link.ble_device })).color(bcol));
            ui.separator();
            ui.label(egui::RichText::new(format!("rx {} · tx {}", link.received, link.sent)).monospace().color(t.text_dim));
        });
        ui.separator();
        ui.label(egui::RichText::new("Input ports (click one to use it as the keyboard)").small().color(t.text_dim));
        let ports = link.ports.clone();
        let current = app.msc.keyboard_port.clone();
        egui::ScrollArea::vertical().max_height(120.0).id_salt("midi_monitor_ports").show(ui, |ui| {
            if ports.is_empty() {
                ui.label(egui::RichText::new("No readable MIDI ports. Is the sequencer running? (ls /dev/snd/seq)").color(t.meter_yellow));
            }
            for port in &ports {
                ui.horizontal(|ui| {
                    let is_keyboard = current.as_deref() == Some(port.name.as_str());
                    let label = if port.matched { format!("🎛 {}", port.name) } else { format!("🎹 {}", port.name) };
                    if ui.selectable_label(is_keyboard, egui::RichText::new(label).small()).on_hover_text(&port.address).clicked() {
                        if is_keyboard {
                            app.control_surface_keyboard(None);
                        } else {
                            app.control_surface_keyboard(Some(port.name.clone()));
                        }
                        app.control_surface_rescan();
                    }
                    if ui.small_button("as surface").on_hover_text("Use this port for the faders/buttons").clicked() {
                        app.control_surface_port(Some(port.name.clone()));
                        app.control_surface_rescan();
                    }
                });
            }
        });
        ui.separator();
        ui.label(egui::RichText::new("Last messages").small().color(t.text_dim));
        egui::ScrollArea::vertical().max_height(180.0).id_salt("midi_monitor_log").show(ui, |ui| {
            let mut lines: Vec<(String, String)> = app
                .msc
                .recent_input
                .iter()
                .map(|(source, m)| (source.clone(), format!("{:02X} {:02X} {:02X}  {}", m[0], m[1], m[2], crate::midi_io::describe(*m))))
                .collect();
            lines.reverse();
            if lines.is_empty() {
                ui.label(egui::RichText::new("Nothing received yet. Move a fader or press a button on the controller.").italics().color(t.text_dim));
            }
            for (source, text) in lines {
                let col = if source == "KEYBOARD" { t.meter_yellow } else { t.accent };
                ui.label(egui::RichText::new(format!("{source:8} {text}")).monospace().small().color(col));
            }
        });
        if !link.ports.is_empty() && link.received == 0 {
            ui.label(
                egui::RichText::new(
                    "Ports exist but no message arrived: check that the controller is in MCU mode and its OUT port is the one subscribed.",
                )
                .small()
                .color(t.meter_yellow),
            );
        }
    });
    app.ui.show_midi_monitor = open;
}

/// Hardware Controller & Mackie Control Universal Setup window (iCON P1-M & M-Vave SMC).
pub fn msc_hardware(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_msc_hardware;
    let t = Tokens::current();
    win(ctx, &mut open, "DAW Controller & Mackie Control Setup (iCON P1-M)", vec2(660.0, 520.0), |ui| {
        // Hardware Profile Switcher
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Hardware Profile:").strong());
            let is_p1m = app.msc.config.profile == crate::msc_mixer::ControllerProfile::IconP1M;
            if ui.selectable_label(is_p1m, "🎛 iCON P1-M (motorized 100 mm, display, 12-LED meters)").clicked() {
                app.msc.config = crate::msc_mixer::MscConfig::icon_p1m();
                app.msc.reset_pickups();
            }
            let is_smc = app.msc.config.profile == crate::msc_mixer::ControllerProfile::MVaveSmc;
            if ui.selectable_label(is_smc, "🎛 M-Vave SMC-Mixer (USB-MIDI / Bluetooth BLE)").clicked() {
                app.msc.config = crate::msc_mixer::MscConfig::mvave_smc();
                app.msc.reset_pickups();
            }
        });

        ui.separator();

        // MIDI keyboard (a piano, a controller): notes go to the selected instrument track.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MIDI keyboard:").strong());
            let state = app.msc.link.keyboard_state;
            let (dot, color) = match state {
                crate::midi_io::LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                crate::midi_io::LinkState::Searching => ("◐", t.meter_yellow),
                crate::midi_io::LinkState::Unavailable => ("○", t.meter_red),
            };
            let current = app.msc.keyboard_port.clone().unwrap_or_else(|| "not connected".to_string());
            ui.label(egui::RichText::new(format!("{dot} {current}")).color(color));
            if ui.small_button("Auto-connect piano").on_hover_text("Pick the first piano-like MIDI port (Casio, Yamaha, Roland…)").clicked() {
                let _ = app.run("controller.keyboard_auto", json!({}));
            }
            if app.msc.keyboard_port.is_some() && ui.small_button("Disconnect").clicked() {
                app.control_surface_keyboard(None);
                app.control_surface_rescan();
            }
        });
        // Every readable MIDI port: click one to use it as the keyboard.
        if !app.msc.link.ports.is_empty() {
            let ports = app.msc.link.ports.clone();
            let current = app.msc.keyboard_port.clone();
            ui.horizontal_wrapped(|ui| {
                for p in &ports {
                    let is_current = current.as_deref() == Some(p.name.as_str());
                    let label = if p.matched { format!("🎛 {}", p.name) } else { format!("🎹 {}", p.name) };
                    if ui.selectable_label(is_current, egui::RichText::new(label).small()).on_hover_text(&p.address).clicked() {
                        if is_current {
                            app.control_surface_keyboard(None);
                        } else {
                            app.control_surface_keyboard(Some(p.name.clone()));
                        }
                        app.control_surface_rescan();
                    }
                }
            });
        }
        if let Some(target) = crate::virtual_keyboard::active_target_track(app) {
            if let Some(track) = app.engine.session().track(target) {
                if track.instrument.is_some() {
                    ui.label(egui::RichText::new(format!("Notes play the selected track: {}", track.name)).small().color(t.text_dim));
                } else {
                    ui.label(
                        egui::RichText::new(format!("{} has no instrument loaded; add an instrument to hear MIDI notes.", track.name))
                            .small()
                            .color(t.meter_yellow),
                    );
                }
            }
        } else {
            ui.label(egui::RichText::new("Create an instrument track to play incoming MIDI notes.").small().color(t.meter_yellow));
        }
        // Input monitor: exactly what arrives from the hardware, so a dead fader or keyboard is
        // visible instead of guessed.
        if !app.msc.recent_input.is_empty() {
            let lines: Vec<String> =
                app.msc.recent_input.iter().map(|(source, d)| format!("{source:8} {:02X} {:02X} {:02X}", d[0], d[1], d[2])).collect();
            ui.group(|ui| {
                ui.label(egui::RichText::new("Last MIDI input").small().color(t.text_dim));
                for line in lines {
                    ui.label(egui::RichText::new(line).monospace().small());
                }
            });
        }

        // Live MIDI link: what the ALSA sequencer worker actually sees (hot-plug aware).
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MIDI link:").strong());
            let (dot, color) = match app.msc.link.state {
                crate::midi_io::LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                crate::midi_io::LinkState::Searching => ("◐", t.meter_yellow),
                crate::midi_io::LinkState::Unavailable => ("○", t.meter_red),
            };
            ui.label(egui::RichText::new(format!("{dot} {}", app.msc.link.state.label())).color(color).strong());
            ui.separator();
            ui.label(egui::RichText::new(&app.msc.link.backend).color(t.text_dim));
            if !app.msc.link.port_name.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(&app.msc.link.port_name).monospace());
            }
            if ui.small_button("🔄 Rescan").on_hover_text("Re-read the MIDI port list now (hot-plug)").clicked() {
                app.control_surface_rescan();
            }
            if ui.small_button("📡 MIDI Monitor").on_hover_text("Every port, what is subscribed, and the live message log").clicked() {
                app.ui.show_midi_monitor = true;
            }
            if ui.small_button("🎹 MIDI Devices").on_hover_text("Enable each device for notes or for faders/buttons (Reaper-style)").clicked() {
                app.ui.show_midi_devices = true;
            }
            if ui.small_button("🎛 Connect").on_hover_text("Start the control-surface link if it is not running").clicked() {
                app.start_control_surface(app.msc.config.bluetooth_device_name_filter.clone());
                app.control_surface_rescan();
            }
        });
        if !app.msc.link.message.is_empty() {
            ui.label(egui::RichText::new(&app.msc.link.message).italics().color(t.text_dim));
        }
        if app.msc.link.state == crate::midi_io::LinkState::Unavailable && app.msc.link.ble_state != crate::midi_io::LinkState::Connected {
            let hint = "No ALSA MIDI link: check /dev/snd/seq and its permissions. Pair the SMC-Mixer, then enable \
                        Bluetooth (BLE-MIDI) for the built-in BlueZ connection. The SMC-Mixer controls the mixer; \
                        use a MIDI keyboard and an instrument track to play notes.";
            ui.label(egui::RichText::new(hint).small().color(t.meter_yellow));
        }
        ui.horizontal(|ui| {
            ui.label("Device filter:");
            let mut filter = app.msc.link.filter.clone();
            if ui.add(egui::TextEdit::singleline(&mut filter).desired_width(160.0)).changed() {
                app.control_surface_filter(filter.clone());
                app.msc.config.bluetooth_device_name_filter = filter;
            }
            ui.separator();
            let counters = format!("rx {} · tx {} · resyncs {}", app.msc.link.received, app.msc.link.sent, app.msc.link.reconnects);
            let counters = egui::RichText::new(counters);
            ui.label(counters.monospace().color(t.text_dim));
        });
        if !app.msc.link.ports.is_empty() {
            let ports = app.msc.link.ports.clone();
            ui.group(|ui| {
                egui::ScrollArea::vertical().max_height(70.0).show(ui, |ui| {
                    for port in &ports {
                        ui.horizontal(|ui| {
                            let mark = if port.matched { "★" } else { "·" };
                            ui.label(egui::RichText::new(format!("{mark} {}", port.name)).monospace());
                            ui.label(egui::RichText::new(&port.address).monospace().color(t.text_dim));
                        });
                    }
                });
            });
        }

        ui.horizontal(|ui| {
            let mut ble = app.msc.ble;
            if ui
                .checkbox(&mut ble, "Bluetooth (BLE-MIDI)")
                .on_hover_text("Use the paired SMC-Mixer over Bluetooth LE: BlueZ GATT + BLE-MIDI packets")
                .changed()
            {
                app.control_surface_ble(ble);
            }
            let (dot, colour) = match app.msc.link.ble_state {
                crate::midi_io::LinkState::Connected => ("●", egui::Color32::from_rgb(0, 220, 130)),
                crate::midi_io::LinkState::Searching => ("◐", t.meter_yellow),
                crate::midi_io::LinkState::Unavailable => ("○", t.text_dim),
            };
            let name = if app.msc.link.ble_device.is_empty() { "no BLE device".to_string() } else { app.msc.link.ble_device.clone() };
            ui.label(egui::RichText::new(format!("{dot} BLE: {name}")).small().color(colour));
            ui.separator();
            ui.label(egui::RichText::new("Control mode:").strong());
            for mode in [crate::msc_mixer::MscMode::Global, crate::msc_mixer::MscMode::Linked] {
                if ui.selectable_label(app.msc.mode == mode, mode.label()).clicked() {
                    app.msc.mode = mode;
                    app.msc.reset_pickups();
                }
            }
            ui.separator();
            ui.checkbox(&mut app.msc.config.soft_takeover, "Soft takeover (fader pickup)")
                .on_hover_text("A physical fader only moves its track after reaching the track's current value");
            if ui.small_button("Release pickups").on_hover_text("Make every fader re-acquire its track").clicked() {
                app.msc.reset_pickups();
            }
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MIDI Learn:").strong());
            match &app.msc.learn {
                Some(target) => {
                    ui.label(egui::RichText::new(format!("armed for {} — move a knob/fader/press a button", target.describe())).color(t.accent));
                    if ui.small_button("Cancel").clicked() {
                        app.msc.learn_cancel();
                    }
                }
                None => {
                    let sel = app.engine.session().edit.selected_tracks.first().copied();
                    if let Some(tid) = sel {
                        if ui.small_button("Learn volume").clicked() {
                            app.msc.learn_start(crate::msc_mixer::LearnTarget::Volume { track: tid.0 });
                        }
                        if ui.small_button("Learn pan").clicked() {
                            app.msc.learn_start(crate::msc_mixer::LearnTarget::Pan { track: tid.0, index: 0 });
                        }
                    }
                    let n = app.msc.bindings.len();
                    ui.label(egui::RichText::new(format!("{n} learned control(s)")).color(t.text_dim));
                    if n > 0 && ui.small_button("Clear all").clicked() {
                        app.msc.learn_clear(None);
                    }
                }
            }
        });
        if !app.msc.bindings.is_empty() {
            let bindings = app.msc.bindings.clone();
            ui.group(|ui| {
                egui::ScrollArea::vertical().max_height(64.0).show(ui, |ui| {
                    for (i, b) in bindings.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{} → {}", b.describe(), b.target.describe())).monospace());
                            if ui.small_button("✖").on_hover_text("Unlearn").clicked() {
                                app.msc.bindings.remove(i);
                            }
                        });
                    }
                });
            });
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Protocol:").strong());
            ui.label(egui::RichText::new(&app.msc.config.protocol).color(t.accent));
            ui.separator();
            ui.label(format!("Bank: {} (tracks {}–{})", app.msc.active_bank + 1, app.msc.active_bank * 8 + 1, (app.msc.active_bank + 1) * 8));
            if ui.small_button("◀ Prev Bank").clicked() && app.msc.active_bank > 0 {
                app.msc.active_bank -= 1;
            }
            if ui.small_button("Next Bank ▶").clicked() && app.msc.active_bank < 8 {
                app.msc.active_bank += 1;
            }
            ui.separator();
            let flip_txt = if app.msc.faders_flipped { "FLIPPED (Faders=Pan)" } else { "NORMAL (Faders=Vol)" };
            let flip_col = if app.msc.faders_flipped { t.meter_yellow } else { t.text_dim };
            ui.label(egui::RichText::new(format!("Flip: {flip_txt}")).color(flip_col));
            ui.separator();
            let scrub_txt = if app.msc.scrub_active { "SCRUB ON" } else { "JOG" };
            let scrub_col = if app.msc.scrub_active { t.accent } else { t.text_dim };
            ui.label(egui::RichText::new(format!("Wheel: {scrub_txt}")).color(scrub_col));
        });

        // 8-Channel Hardware Strip Monitor
        ui.group(|ui| {
            ui.horizontal(|ui| {
                for ch in 0..app.msc.config.channels {
                    let tid = app.msc.track_for_channel(app.engine.session(), ch);
                    let tr = tid.and_then(|id| app.engine.session().track(id));
                    let is_touched = app.msc.fader_touched[ch];
                    ui.vertical(|ui| {
                        ui.set_width(70.0);
                        let title = tr.map(|t| t.name.as_str()).unwrap_or("<empty>");
                        ui.label(egui::RichText::new(format!("CH {}", ch + 1)).strong());
                        ui.add(egui::Label::new(egui::RichText::new(title).small()).truncate());
                        let db = tr.map(|t| t.mixer.volume_db).unwrap_or(-60.0);
                        ui.label(egui::RichText::new(format!("{:.1} dB", db)).monospace());
                        if is_touched {
                            ui.label(egui::RichText::new("[TOUCH]").color(t.accent).strong());
                        } else {
                            ui.label(egui::RichText::new("Motor").color(t.text_dim));
                        }
                    });
                }
            });
        });

        ui.separator();

        // Bluetooth BLE Controller Section
        ui.label(egui::RichText::new("Bluetooth BLE-MIDI Manager (Wireless Controllers)").strong());
        ui.horizontal(|ui| {
            if app.msc.bluetooth_available {
                let state_txt = if app.msc.bluetooth_powered { "POWERED ON" } else { "OFF" };
                let col = if app.msc.bluetooth_powered { egui::Color32::from_rgb(0, 220, 130) } else { t.text_dim };
                ui.label(egui::RichText::new(format!("Host Adapter: {state_txt}")).color(col));
            } else {
                ui.label(egui::RichText::new("Bluetooth Adapter: Not Detected").color(t.text_dim));
            }
            if ui.button("🔄 Refresh").clicked() {
                app.msc.refresh_bluetooth();
            }
            if ui.button("🔍 Scan BLE").clicked() {
                let _ = app.msc.scan_bluetooth();
            }
            if ui.button(egui::RichText::new("⚡ Auto-Connect M-Vave").strong()).clicked() {
                let _ = app.msc.auto_connect_ble();
            }
        });

        ui.label(egui::RichText::new(&app.msc.ble_status).italics().color(t.text_dim));

        ui.group(|ui| {
            ui.set_max_height(100.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                if app.msc.ble_devices.is_empty() {
                    let none = "No Bluetooth devices found. Click 'Scan BLE' for wireless operation,";
                    ui.label(format!("{none} or enable Bluetooth pairing on the mixer."));
                } else {
                    let devices = app.msc.ble_devices.clone();
                    let mut action: Option<(&'static str, String)> = None;
                    for dev in &devices {
                        ui.horizontal(|ui| {
                            let icon = if dev.is_mvave { "🎛" } else { "📱" };
                            ui.label(format!("{icon} {}", dev.name));
                            ui.label(egui::RichText::new(&dev.mac).monospace().color(t.text_dim));
                            if dev.is_mvave {
                                ui.label(egui::RichText::new("[M-VAVE]").color(t.accent).strong());
                            }
                            if dev.connected {
                                ui.label(egui::RichText::new("CONNECTED").color(egui::Color32::from_rgb(0, 220, 130)).strong());
                                if ui.small_button("Disconnect").clicked() {
                                    action = Some(("disconnect", dev.mac.clone()));
                                }
                            } else if ui.small_button("Connect").clicked() {
                                action = Some(("connect", dev.mac.clone()));
                            }
                        });
                    }
                    if let Some((act, mac)) = action {
                        if act == "connect" {
                            let _ = app.msc.connect_ble(&mac);
                        } else {
                            let _ = app.msc.disconnect_ble(&mac);
                        }
                    }
                }
            });
        });

        ui.separator();

        // Diagnostics and Testing
        ui.label(egui::RichText::new("MCU Diagnostics & Testing").strong());
        ui.horizontal(|ui| {
            if ui.button("🚨 Flash LEDs & Exercise Faders").clicked() {
                app.msc.trigger_diagnostic_test();
            }
            if ui.button("📁 Reload mapping from config.json").on_hover_text("Re-read the mapping presets stored beside config.json").clicked() {
                // The presets live beside the settings file, never in the process's working directory.
                let dir = app.preset_dir.as_deref().and_then(std::path::Path::parent).map(std::path::Path::to_path_buf);
                if let Some(dir) = dir {
                    app.msc.ensure_config_loaded(Some(&dir));
                } else {
                    app.ui.status = "No settings directory: mapping presets cannot be read".to_string();
                }
            }
            if ui.button("🔍 Auto-Detect Controller").clicked() {
                app.msc.auto_detect();
            }
        });

        if let Some(msg) = &app.msc.last_log_message {
            ui.label(egui::RichText::new(format!("Activity: {msg}")).color(t.accent));
        }
    });
    app.ui.show_msc_hardware = open;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod audio_health_tests {
    use super::*;

    #[test]
    fn window_command_renders_report_and_close_clears_snapshot() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, crate::Services::default());
        app.run("window.audio_health", json!({"value": true})).unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(app.audio_health_report.as_ref().unwrap().as_ref().unwrap()["healthy"], true);
        assert!(!app.engine.is_dirty());
        app.run("window.hide_floating", json!({})).unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.ui.show_audio_health);
        assert!(app.audio_health_report.is_none());
        app.run("window.audio_health", json!({"value": true})).unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(app.audio_health_report.is_some());
    }
}
