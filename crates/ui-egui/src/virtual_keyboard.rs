//! Professional Horizontal Virtual Master Keyboard.
//!
//! Features high-fidelity grand piano aesthetics with red velvet felt rail,
//! ivory & ebony 3D keys, interactive pitch-bend and modulation wheels,
//! computer QWERTY keyboard live playback, velocity dynamics, octave range
//! selectors, and real-time live MIDI routing to session instrument tracks.

use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, CornerRadius, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_model::TrackId;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct KeyboardState {
    /// Notes currently held (illuminated).
    pub held_notes: HashSet<u8>,
    /// Pitches the on-screen keyboard (mouse) and the computer keys started. They are watched so a
    /// lost key/pointer release can never leave a note droning forever (see
    /// [`release_orphan_notes`]).
    pub typed_notes: HashSet<u8>,
    /// Last note pressed via mouse/touch.
    pub last_clicked: Option<u8>,
    /// Track that received the last note pressed via mouse/touch.
    pub last_clicked_track: Option<TrackId>,
    /// Octave offset (-3 to +3).
    pub base_octave: i8,
    /// Configured range: 25, 37, 49, 61, or 88 keys.
    pub num_keys: u8,
    /// Default velocity for notes triggered from UI (1..127).
    pub velocity: u8,
    /// Whether clicking also step-enters notes into an active MIDI clip.
    pub step_rec: bool,
    /// Pitch bend wheel position (-1.0 to +1.0, springs to 0.0).
    pub pitch_bend: f32,
    /// Modulation wheel position (0.0 to 1.0, CC 1).
    pub mod_wheel: f32,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The persisted subset of the keyboard setup.
    pub fn prefs(&self) -> KeyboardPrefs {
        KeyboardPrefs { num_keys: self.num_keys, base_octave: self.base_octave, velocity: self.velocity, step_rec: self.step_rec }
    }

    /// Applies persisted preferences (the rest of the state is runtime-only).
    pub fn apply_prefs(&mut self, prefs: &KeyboardPrefs) {
        self.num_keys = match prefs.num_keys {
            25 | 37 | 49 | 61 | 88 => prefs.num_keys,
            _ => 61,
        };
        self.base_octave = prefs.base_octave.clamp(-3, 3);
        self.velocity = prefs.velocity.clamp(1, 127);
        self.step_rec = prefs.step_rec;
    }
}

/// The persisted keyboard setup (written to `config.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeyboardPrefs {
    pub num_keys: u8,
    pub base_octave: i8,
    pub velocity: u8,
    pub step_rec: bool,
}

impl Default for KeyboardPrefs {
    fn default() -> Self {
        KeyboardPrefs { num_keys: 61, base_octave: 0, velocity: 100, step_rec: false }
    }
}

impl Default for KeyboardState {
    fn default() -> Self {
        KeyboardState {
            held_notes: HashSet::new(),
            typed_notes: HashSet::new(),
            last_clicked: None,
            last_clicked_track: None,
            base_octave: 0,
            num_keys: 61,
            velocity: 100,
            step_rec: false,
            pitch_bend: 0.0,
            mod_wheel: 0.0,
        }
    }
}

/// Note pitch name (e.g. C3, D#3).
pub fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (pitch as i32 / 12) - 1;
    let name = NAMES[(pitch % 12) as usize];
    format!("{name}{octave}")
}

/// Whether a MIDI pitch is a black key.
pub const fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

/// Find the target active track for virtual keyboard input. A selected track with an instrument
/// wins; otherwise use the first instrument-bearing MIDI track, even when an empty MIDI track or
/// audio track is selected.
pub fn active_target_track(app: &SoundApp) -> Option<TrackId> {
    let s = app.engine.session();
    let any_solo = s.tracks.iter().any(|t| t.mixer.solo && !t.inactive);

    // While an instrument console is open, its selected profile owns live input. Never send
    // Pianoteq controls or keyboard notes to a different plugin just because that track is
    // selected or is the first instrument in the session.
    if app.ui.show_console
        && let Some(profile) = crate::console_kit::profile(&app.ui.console_profile)
    {
        let matches_profile = |track: &soundcraft_model::Track| {
            track.kind.is_midi()
                && !track.hidden
                && track.instrument.as_ref().is_some_and(|instrument| {
                    let loaded = crate::console_kit::profile_for_plugin(&instrument.plugin);
                    loaded.is_some_and(|loaded| loaded.id == profile.id)
                })
        };
        // Try soloed matching track first.
        if any_solo {
            let maybe = s
                .edit
                .selected_tracks
                .iter()
                .filter_map(|id| s.track(*id))
                .find(|track| matches_profile(track) && track.mixer.solo)
                .or_else(|| s.tracks.iter().find(|track| matches_profile(track) && track.mixer.solo))
                .map(|track| track.id);
            if let Some(t) = maybe {
                return Some(t);
            }
        }
        // Any matching instrument (selected first, then record-armed, then any matching).
        let maybe = s
            .edit
            .selected_tracks
            .iter()
            .filter_map(|id| s.track(*id))
            .find(|track| matches_profile(track))
            .map(|track| track.id)
            .or_else(|| s.tracks.iter().find(|track| matches_profile(track) && track.mixer.record_arm).map(|t| t.id))
            .or_else(|| s.tracks.iter().find(|track| matches_profile(track)).map(|t| t.id));
        if let Some(t) = maybe {
            return Some(t);
        }
        // No matching track – fall back to the normal selection logic below.
        // CORREÇÃO: Agora vamos tentar usar o track selecionado ou um track instrument
        // mesmo que não corresponda ao perfil do console, para evitar silenciar MIDI
    }

    // 1. If any track is in Solo, only soloed tracks can sound through the mix bus.
    // Prioritize an instrument track that is in Solo:
    if any_solo {
        if let Some(&sel) = s
            .edit
            .selected_tracks
            .iter()
            .find(|&&id| s.track(id).is_some_and(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.solo))
        {
            return Some(sel);
        }
        if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.solo) {
            return Some(t.id);
        }
    }

    // 2. A selected track that has an instrument:
    if let Some(&sel) =
        s.edit.selected_tracks.iter().find(|&&id| s.track(id).is_some_and(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some()))
    {
        return Some(sel);
    }

    // 3. Any record-armed instrument track:
    if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some() && t.mixer.record_arm) {
        return Some(t.id);
    }

    // 4. First instrument track in the session:
    if let Some(t) = s.tracks.iter().find(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some()) {
        return Some(t.id);
    }

    // Keep the selected track as the UI fallback so MIDI Monitor can explain why no note sounds.
    // CORREÇÃO: Agora retornamos o primeiro track selecionado, mesmo que não tenha instrumento
    // para evitar silenciar completamente a entrada MIDI
    s.edit.selected_tracks.first().copied()
}

/// Computes the pitch range `(start_pitch, end_pitch)` inclusive based on configuration.
pub fn calculate_range(num_keys: u8, octave_offset: i8) -> (u8, u8) {
    if num_keys == 88 {
        // Standard 88-key piano: A0 (21) to C8 (108)
        let shift = (octave_offset.clamp(-1, 1) * 12) as i32;
        let start = (21 + shift).clamp(0, 40) as u8;
        let end = (start as usize + 87).min(127) as u8;
        return (start, end);
    }
    let base = ((3 + octave_offset as i32) * 12).clamp(0, 127 - num_keys as i32) as u8;
    (base, base + num_keys - 1)
}

/// Draw the responsive horizontal virtual keyboard bottom dock.
/// The plugin name of the instrument on the active track, when there is one ("pigments:…",
/// "Pigments", "vst3:…").
fn active_instrument_name(app: &SoundApp) -> Option<String> {
    let s = app.engine.session();
    if let Some(&sel) = s.edit.selected_tracks.iter().find(|&&id| s.track(id).is_some_and(|t| t.kind.is_midi() && !t.hidden && t.instrument.is_some())) {
        return s.track(sel).and_then(|t| t.instrument.as_ref()).map(|ins| ins.plugin.clone());
    }
    let track = active_target_track(app)?;
    let t = s.track(track)?;
    let ins = t.instrument.as_ref()?;
    Some(ins.plugin.clone())
}

pub fn show_dock(app: &mut SoundApp, ui: &mut Ui) {
    let r = ui.max_rect();

    // Piano chassis background: Luxury dark piano lacquer
    ui.painter().rect_filled(r, 0.0, Color32::from_rgb(15, 16, 20));
    ui.painter().line_segment([pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y)], Stroke::new(1.0, Color32::from_rgb(45, 48, 56)));

    // 1. Sleek brushed metal control bar (top 28px)
    let top_bar = Rect::from_min_size(r.min, vec2(r.width(), 28.0));
    draw_top_bar(app, ui, top_bar);

    // 2. Main Playable Area: Left Wheel Bay + Grand Piano Keyboard Bed
    let content_area = Rect::from_min_max(pos2(r.min.x + 4.0, r.min.y + 30.0), pos2(r.max.x - 4.0, r.max.y - 4.0));

    let wheels_w = 90.0;
    let wheels_rect = Rect::from_min_size(content_area.min, vec2(wheels_w, content_area.height()));
    let keys_rect = Rect::from_min_max(pos2(content_area.min.x + wheels_w + 6.0, content_area.min.y), content_area.max);

    // Handle computer keyboard live input. While a plugin's own window (a native, detached one
    // for Kontakt under yabridge) has focus, the OS sends it the typing and egui sees nothing,
    // which is the isolation we want.
    handle_computer_keyboard_input(app, ui);

    // Draw Wheel Bay (Pitch Bend & Modulation Wheels)
    draw_controller_wheels(app, ui, wheels_rect);

    // Draw Piano Keybed with Grand Piano Red Velvet Rail
    draw_piano_keybed(app, ui, keys_rect);
}

fn draw_top_bar(app: &mut SoundApp, ui: &mut Ui, r: Rect) {
    let t = Tokens::current();

    // Brushed top strip, in the active theme's panel colour.
    ui.painter().rect_filled(r, 0.0, t.panel_bg2);
    ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));

    let mut x = r.min.x + 12.0;
    let cy = r.center().y;

    // Title & Glowing Badge
    let title = "🎹 VIRTUAL MASTER KEYBOARD";
    let tg = ui.painter().layout_no_wrap(title.to_string(), bold(11.0), t.accent);
    ui.painter().galley(pos2(x, cy - tg.size().y * 0.5), tg.clone(), Color32::WHITE);
    x += tg.size().x + 16.0;

    // Octave Shift Section
    let btn_w = 24.0;
    let oct_down = Rect::from_center_size(pos2(x + btn_w * 0.5, cy), vec2(btn_w, 20.0));
    let oct_down_resp = ui.interact(oct_down, ui.id().with("oct_down"), Sense::click());
    let oct_down_hovered = oct_down_resp.hovered();
    if oct_down_resp.on_hover_text("Octave Down").clicked() {
        app.keyboard.base_octave = (app.keyboard.base_octave - 1).max(-3);
    }
    ui.painter().rect_filled(oct_down, 3.0, if oct_down_hovered { Color32::from_rgb(50, 54, 64) } else { Color32::from_rgb(32, 35, 42) });
    ui.painter().rect_stroke(oct_down, 3.0, Stroke::new(1.0, Color32::from_rgb(55, 60, 72)), StrokeKind::Inside);
    ui.painter().text(oct_down.center(), Align2::CENTER_CENTER, "◀", bold(10.0), Color32::WHITE);
    x += btn_w + 6.0;

    let (p_start, p_end) = calculate_range(app.keyboard.num_keys, app.keyboard.base_octave);
    let oct_label = format!("{} – {}", pitch_name(p_start), pitch_name(p_end));
    let oct_rect = Rect::from_min_size(pos2(x, cy - 10.0), vec2(78.0, 20.0));
    ui.painter().rect_filled(oct_rect, 3.0, t.counter_bg);
    ui.painter().rect_stroke(oct_rect, 3.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().text(oct_rect.center(), Align2::CENTER_CENTER, oct_label, bold(10.0), t.accent);
    x += 84.0;

    let oct_up = Rect::from_center_size(pos2(x + btn_w * 0.5, cy), vec2(btn_w, 20.0));
    let oct_up_resp = ui.interact(oct_up, ui.id().with("oct_up"), Sense::click());
    let oct_up_hovered = oct_up_resp.hovered();
    if oct_up_resp.on_hover_text("Octave Up").clicked() {
        app.keyboard.base_octave = (app.keyboard.base_octave + 1).min(3);
    }
    ui.painter().rect_filled(oct_up, 3.0, if oct_up_hovered { Color32::from_rgb(50, 54, 64) } else { Color32::from_rgb(32, 35, 42) });
    ui.painter().rect_stroke(oct_up, 3.0, Stroke::new(1.0, Color32::from_rgb(55, 60, 72)), StrokeKind::Inside);
    ui.painter().text(oct_up.center(), Align2::CENTER_CENTER, "▶", bold(10.0), Color32::WHITE);
    x += btn_w + 18.0;

    // Key Count Selector Buttons
    let range_lbl = ui.painter().layout_no_wrap("Keys:".to_string(), regular(10.0), t.text_dim);
    ui.painter().galley(pos2(x, cy - range_lbl.size().y * 0.5), range_lbl.clone(), Color32::WHITE);
    x += range_lbl.size().x + 6.0;

    for k in [25u8, 37, 49, 61, 88] {
        let is_sel = app.keyboard.num_keys == k;
        let kr = Rect::from_center_size(pos2(x + 13.0, cy), vec2(26.0, 19.0));
        let kresp = ui.interact(kr, ui.id().with(("krange", k)), Sense::click());
        if kresp.clicked() {
            app.keyboard.num_keys = k;
        }
        let bg = if is_sel {
            Color32::from_rgb(0, 150, 180)
        } else if kresp.hovered() {
            Color32::from_rgb(46, 50, 60)
        } else {
            Color32::from_rgb(30, 32, 38)
        };
        ui.painter().rect_filled(kr, 2.0, bg);
        ui.painter().rect_stroke(kr, 2.0, Stroke::new(1.0, if is_sel { t.accent } else { Color32::from_rgb(50, 54, 64) }), StrokeKind::Inside);
        ui.painter().text(kr.center(), Align2::CENTER_CENTER, format!("{k}"), bold(9.5), if is_sel { Color32::WHITE } else { t.text });
        x += 29.0;
    }
    x += 16.0;

    // Velocity Control with Dynamic Visual Meter
    let vel_lbl = ui.painter().layout_no_wrap(format!("Vel: {:3}", app.keyboard.velocity), regular(10.0), t.text);
    ui.painter().galley(pos2(x, cy - vel_lbl.size().y * 0.5), vel_lbl.clone(), Color32::WHITE);
    x += vel_lbl.size().x + 6.0;

    let vel_slider_rect = Rect::from_min_size(pos2(x, cy - 6.0), vec2(70.0, 12.0));
    ui.painter().rect_filled(vel_slider_rect, 3.0, Color32::from_rgb(14, 15, 18));
    let vel_pct = (app.keyboard.velocity.clamp(1, 127) as f32 - 1.0) / 126.0;
    let fill_rect = Rect::from_min_size(vel_slider_rect.min, vec2(vel_slider_rect.width() * vel_pct, vel_slider_rect.height()));
    ui.painter().rect_filled(fill_rect, 3.0, Color32::from_rgb(0, 210, 160));
    ui.painter().rect_stroke(vel_slider_rect, 3.0, Stroke::new(1.0, Color32::from_rgb(40, 44, 52)), StrokeKind::Inside);

    let vresp = ui.interact(vel_slider_rect, ui.id().with("vel_drag"), Sense::click_and_drag());
    if (vresp.clicked() || vresp.dragged())
        && let Some(pos) = vresp.interact_pointer_pos()
    {
        let pct = ((pos.x - vel_slider_rect.min.x) / vel_slider_rect.width()).clamp(0.0, 1.0);
        app.keyboard.velocity = (1.0 + pct * 126.0).round() as u8;
    }
    x += 82.0;

    // Target Track Indicator
    let target = active_target_track(app);
    let target_str = match target.and_then(|tid| app.engine.session().track(tid)) {
        Some(tr) => {
            let inst_str = tr.instrument.as_ref().map(|i| i.plugin.as_str()).unwrap_or("No Inst");
            format!("🎯 {} ({})", tr.name, inst_str)
        }
        None => "🎯 No Track Selected".to_string(),
    };
    let tg_galley = ui.painter().layout_no_wrap(target_str, bold(10.0), Color32::from_rgb(0, 240, 160));
    ui.painter().galley(pos2(x, cy - tg_galley.size().y * 0.5), tg_galley, Color32::WHITE);

    // QWERTY Computer Keyboard Guide Badge
    let qwerty_guide = "[QWERTY: A-K keys play notes]";
    let qg = ui.painter().layout_no_wrap(qwerty_guide.to_string(), regular(9.5), t.text_dim);
    // Left of the ALT GUI button (which sits at max.x - 144 … max.x - 40) so they never overlap.
    ui.painter().galley(pos2(r.max.x - 156.0 - qg.size().x, cy - qg.size().y * 0.5), qg, Color32::WHITE);

    // Alternative GUI for the hosted piano: its own console window, next to the plugin's editor.
    let alt_rect = Rect::from_center_size(pos2(r.max.x - 92.0, cy), vec2(104.0, 20.0));
    let alt = ui.interact(alt_rect, ui.id().with("alt_gui_button"), Sense::click());
    let alt_on = app.ui.show_console;
    let alt_bg = if alt_on {
        Color32::from_rgb(240, 180, 74)
    } else if alt.hovered() {
        Color32::from_rgb(60, 64, 74)
    } else {
        Color32::from_rgb(34, 36, 44)
    };
    ui.painter().rect_filled(alt_rect, 3.0, alt_bg);
    ui.painter().rect_stroke(
        alt_rect,
        3.0,
        Stroke::new(1.0, if alt_on { Color32::BLACK } else { Color32::from_rgb(60, 64, 74) }),
        StrokeKind::Inside,
    );
    ui.painter().text(alt_rect.center(), Align2::CENTER_CENTER, "ALT GUI", bold(9.5), if alt_on { Color32::BLACK } else { Color32::WHITE });
    if alt.on_hover_text("Alternative console for the hosted instrument").clicked() {
        // Open the console on the instrument that matches the active track's plugin; with nothing
        // loaded it opens on the piano, whose racks are the generic ones.
        let plugin = active_instrument_name(app).unwrap_or_default();
        let profile = crate::console_kit::profile_for_plugin(&plugin).map(|p| p.id).unwrap_or("pianoteq");
        if app.ui.show_console {
            app.ui.show_console = false;
        } else {
            crate::console_kit::open(app, profile);
        }
    }

    // Close button
    let close_rect = Rect::from_center_size(pos2(r.max.x - 16.0, cy), vec2(22.0, 20.0));
    let cresp = ui.interact(close_rect, ui.id().with("close_kbd_dock"), Sense::click());
    let close_hovered = cresp.hovered();
    if cresp.on_hover_text("Close Virtual Keyboard (Shift+K)").clicked() {
        app.ui.show_midi_keyboard = false;
    }
    ui.painter().rect_filled(close_rect, 3.0, if close_hovered { Color32::from_rgb(180, 40, 50) } else { Color32::from_rgb(34, 36, 44) });
    ui.painter().text(close_rect.center(), Align2::CENTER_CENTER, "✕", bold(11.0), Color32::WHITE);
}

#[derive(Clone, Copy, PartialEq)]
enum WheelAction {
    Drag(f32),
    Release,
}

fn draw_controller_wheels(app: &mut SoundApp, ui: &mut Ui, r: Rect) {
    let target = active_target_track(app);

    // Left cheek background
    ui.painter().rect_filled(r, 4.0, Color32::from_rgb(20, 22, 26));
    ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, Color32::from_rgb(38, 42, 50)), StrokeKind::Inside);

    let w_w = (r.width() - 20.0) * 0.5;
    let h = r.height() - 28.0;

    let pb_rect = Rect::from_min_size(pos2(r.min.x + 7.0, r.min.y + 6.0), vec2(w_w, h));
    let mod_rect = Rect::from_min_size(pos2(r.min.x + w_w + 13.0, r.min.y + 6.0), vec2(w_w, h));

    // 1. Pitch Bend Wheel (Center Spring)
    if let Some(act) = render_wheel(ui, pb_rect, "PITCH", app.keyboard.pitch_bend * 0.5 + 0.5, true) {
        match act {
            WheelAction::Drag(val_norm) => {
                let pb = (val_norm - 0.5) * 2.0;
                app.keyboard.pitch_bend = pb;
                if let Some(tid) = target {
                    let pb_int = ((pb + 1.0) * 8191.5).clamp(0.0, 16383.0).round() as u16;
                    let lsb = (pb_int & 0x7F) as u8;
                    let msb = ((pb_int >> 7) & 0x7F) as u8;
                    app.send_live_midi(tid, [0xE0, lsb, msb]);
                }
            }
            WheelAction::Release => {
                app.keyboard.pitch_bend = 0.0;
                if let Some(tid) = target {
                    app.send_live_midi(tid, [0xE0, 0x00, 0x40]);
                }
            }
        }
    }

    // 2. Mod Wheel (Friction loaded CC 1)
    if let Some(WheelAction::Drag(val_norm)) = render_wheel(ui, mod_rect, "MOD", app.keyboard.mod_wheel, false) {
        app.keyboard.mod_wheel = val_norm;
        if let Some(tid) = target {
            let cc_val = (val_norm * 127.0).clamp(0.0, 127.0).round() as u8;
            app.send_live_midi(tid, [0xB0, 1, cc_val]);
        }
    }
}

fn render_wheel(ui: &mut Ui, r: Rect, label: &str, val_norm: f32, is_spring: bool) -> Option<WheelAction> {
    // Wheel well / bezel
    ui.painter().rect_filled(r, 3.0, Color32::from_rgb(10, 11, 14));
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_rgb(30, 32, 38)), StrokeKind::Inside);

    // Ribbed wheel body
    let active_h = r.height() - 18.0;
    let thumb_y = r.min.y + 4.0 + (1.0 - val_norm.clamp(0.0, 1.0)) * (active_h - 18.0);
    let thumb_rect = Rect::from_min_size(pos2(r.min.x + 3.0, thumb_y), vec2(r.width() - 6.0, 18.0));

    // Wheel drum shading
    ui.painter().rect_filled(thumb_rect, 2.0, Color32::from_rgb(60, 64, 75));
    // Center notch or ribs
    for i in 0..3 {
        let ly = thumb_rect.min.y + 4.0 + i as f32 * 5.0;
        ui.painter()
            .line_segment([pos2(thumb_rect.min.x + 3.0, ly), pos2(thumb_rect.max.x - 3.0, ly)], Stroke::new(1.0, Color32::from_rgb(20, 22, 26)));
    }

    let resp = ui.interact(r, ui.id().with(("wheel", label)), Sense::click_and_drag());
    let mut action = None;
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let pct = 1.0 - ((p.y - (r.min.y + 12.0)) / (active_h - 12.0)).clamp(0.0, 1.0);
            action = Some(WheelAction::Drag(pct));
        }
    } else if is_spring && resp.drag_stopped() {
        action = Some(WheelAction::Release);
    }

    // Label below wheel
    ui.painter().text(pos2(r.center().x, r.max.y + 10.0), Align2::CENTER_CENTER, label, bold(8.5), Color32::from_rgb(140, 145, 160));

    action
}

fn draw_piano_cheek(ui: &mut Ui, r: Rect, is_left: bool) {
    if r.width() < 2.0 {
        return;
    }
    // High-gloss deep mahogany/rosewood lacquer
    ui.painter().rect_filled(r, 0.0, Color32::from_rgb(24, 18, 16));

    // Subtle rich wood tone gradient / grain line
    let mid_y = r.center().y;
    ui.painter().line_segment([pos2(r.min.x, mid_y), pos2(r.max.x, mid_y)], Stroke::new(1.0, Color32::from_rgb(38, 26, 22)));

    // Brass accent trim line and red felt cushion touching the keys
    if is_left {
        // Red felt buffer strip against the lowest key
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.max.x - 3.0, r.min.y), pos2(r.max.x, r.max.y)), 0.0, Color32::from_rgb(128, 18, 26));
        // Brass vertical inlay
        ui.painter().line_segment([pos2(r.max.x - 4.0, r.min.y), pos2(r.max.x - 4.0, r.max.y)], Stroke::new(1.0, Color32::from_rgb(190, 150, 60)));
    } else {
        // Red felt buffer strip against the highest key
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(r.min.x + 3.0, r.max.y)), 0.0, Color32::from_rgb(128, 18, 26));
        // Brass vertical inlay
        ui.painter().line_segment([pos2(r.min.x + 4.0, r.min.y), pos2(r.min.x + 4.0, r.max.y)], Stroke::new(1.0, Color32::from_rgb(190, 150, 60)));
    }
}

fn draw_piano_keybed(app: &mut SoundApp, ui: &mut Ui, r: Rect) {
    let (start_p, end_p) = calculate_range(app.keyboard.num_keys, app.keyboard.base_octave);

    // Count white keys
    let white_pitches: Vec<u8> = (start_p..=end_p).filter(|&p| !is_black_key(p)).collect();
    let num_white = white_pitches.len().max(1);

    // Realistic acoustic piano proportions:
    // A real grand piano key has ~21-23mm width per white key with ~120-150mm length (~6:1 ratio).
    // Keys should NEVER be gigantic 50px-60px blocks, and must touch seamlessly without gaps.
    let max_white_w = r.width() / num_white as f32;
    let key_w = max_white_w.clamp(14.0, 22.0);
    let bed_w = key_w * num_white as f32;
    let bed_left = r.min.x + ((r.width() - bed_w).max(0.0) * 0.5).floor();

    let felt_height = 6.0;

    // Draw luxury acoustic piano side cheeks (polished dark mahogany/rosewood with brass trim)
    if bed_left > r.min.x + 1.0 {
        let left_cheek = Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(bed_left, r.max.y));
        draw_piano_cheek(ui, left_cheek, true);
    }
    if bed_left + bed_w < r.max.x - 1.0 {
        let right_cheek = Rect::from_min_max(pos2(bed_left + bed_w, r.min.y), pos2(r.max.x, r.max.y));
        draw_piano_cheek(ui, right_cheek, false);
    }

    // 1. Grand Piano Red Velvet Felt Rail (crimson velvet over the playable keybed)
    let felt_rect = Rect::from_min_max(pos2(bed_left, r.min.y), pos2(bed_left + bed_w, r.min.y + felt_height));
    ui.painter().rect_filled(felt_rect, 0.0, Color32::from_rgb(148, 22, 33));
    // Gold pin / brass accent trim line
    ui.painter().line_segment(
        [pos2(felt_rect.min.x, felt_rect.max.y), pos2(felt_rect.max.x, felt_rect.max.y)],
        Stroke::new(1.2, Color32::from_rgb(215, 175, 75)),
    );

    // Key area under the felt
    let keys_rect = Rect::from_min_max(pos2(bed_left, r.min.y + felt_height), pos2(bed_left + bed_w, r.max.y));
    let key_h = keys_rect.height();
    let black_w = (key_w * 0.58).clamp(8.0, 14.0).round();
    let black_h = (key_h * 0.64).round();

    // Interaction response for touch / mouse glissando
    let kbd_resp = ui.interact(keys_rect, ui.id().with("kbd_touch_bed"), Sense::click_and_drag());
    let pointer_pos = kbd_resp.interact_pointer_pos();
    // Press and hold (and a drag, for glissando): the note sounds while the button is down,
    // which is also what a touchscreen produces. A released button is "up" in the same frame,
    // so the note-off below is not skipped.
    let is_down = kbd_resp.is_pointer_button_down_on() || kbd_resp.dragged();

    // Map pitches to their bounding rects
    let mut white_rects: Vec<(u8, Rect)> = Vec::with_capacity(num_white);
    let mut black_rects: Vec<(u8, Rect)> = Vec::with_capacity((end_p - start_p) as usize);

    for (i, &p) in white_pitches.iter().enumerate() {
        let x = bed_left + i as f32 * key_w;
        let kr = Rect::from_min_size(pos2(x.round(), keys_rect.min.y), vec2(key_w.round(), key_h));
        white_rects.push((p, kr));
    }

    let mut wi = 0;
    for p in start_p..=end_p {
        if is_black_key(p) {
            // Natural acoustic piano black key offsets:
            // C# and F# shift slightly left; D# and A# shift slightly right; G# is centered
            let offset = match p % 12 {
                1 => -key_w * 0.065, // C#
                3 => key_w * 0.065,  // D#
                6 => -key_w * 0.075, // F#
                8 => 0.0,            // G#
                10 => key_w * 0.075, // A#
                _ => 0.0,
            };
            let prev_x = bed_left + wi as f32 * key_w;
            let kr = Rect::from_min_size(pos2((prev_x - black_w * 0.5 + offset).round(), keys_rect.min.y), vec2(black_w, black_h));
            black_rects.push((p, kr));
        } else {
            wi += 1;
        }
    }

    // Determine currently hovered key (black keys first for top priority)
    let mut current_hovered_pitch: Option<u8> = None;
    if let Some(pos) = pointer_pos {
        for &(p, br) in &black_rects {
            if br.contains(pos) {
                current_hovered_pitch = Some(p);
                break;
            }
        }
        if current_hovered_pitch.is_none() {
            for &(p, wr) in &white_rects {
                if wr.contains(pos) {
                    current_hovered_pitch = Some(p);
                    break;
                }
            }
        }
    }

    let target = active_target_track(app);

    // Mouse Press / Drag Glissando Note Routing
    if is_down && let Some(pitch) = current_hovered_pitch {
        if app.keyboard.last_clicked != Some(pitch) {
            if let Some(prev) = app.keyboard.last_clicked {
                let release_target = app.keyboard.last_clicked_track.or(target);
                if let Some(tid) = release_target {
                    app.send_live_midi(tid, [0x80, prev, 0]);
                }
                app.keyboard.held_notes.remove(&prev);
                app.keyboard.typed_notes.remove(&prev);
            }
            if let Some(tid) = target {
                app.send_live_midi(tid, [0x90, pitch, app.keyboard.velocity]);
                if app.keyboard.step_rec {
                    step_record_note(app, tid, pitch);
                }
            }
            app.keyboard.held_notes.insert(pitch);
            app.keyboard.typed_notes.insert(pitch);
            app.keyboard.last_clicked = Some(pitch);
            app.keyboard.last_clicked_track = target;
        }
    } else if let Some(last) = app.keyboard.last_clicked.take() {
        let release_target = app.keyboard.last_clicked_track.take().or(target);
        if let Some(tid) = release_target {
            app.send_live_midi(tid, [0x80, last, 0]);
        }
        app.keyboard.held_notes.remove(&last);
        app.keyboard.typed_notes.remove(&last);
    }

    // 1. DRAW WHITE KEYS (Natural Ivory Realism, touching side-by-side with hairline divider)
    for &(p, kr) in &white_rects {
        let is_held = app.keyboard.held_notes.contains(&p);
        let is_hovered = is_down && current_hovered_pitch == Some(p);
        let active = is_held || is_hovered;

        let bg = if active {
            Color32::from_rgb(0, 210, 255) // Glowing cyan on key press
        } else {
            Color32::from_rgb(250, 250, 248) // High-grade ivory white
        };

        let draw_rect = if active {
            // Depressed key perspective shift
            Rect::from_min_max(pos2(kr.min.x, kr.min.y + 2.0), kr.max)
        } else {
            kr // Flush and touching adjacent keys with no gap!
        };

        // Keycap body with rounded bottom corners and hairline dark dividing seam
        ui.painter().rect(
            draw_rect,
            CornerRadius { nw: 0, ne: 0, sw: 2, se: 2 },
            bg,
            Stroke::new(1.0, Color32::from_rgb(45, 48, 56)),
            StrokeKind::Inside,
        );

        // Ivory bottom 3D shadow lip
        if !active {
            ui.painter().line_segment(
                [pos2(draw_rect.min.x + 1.0, draw_rect.max.y - 2.5), pos2(draw_rect.max.x - 1.0, draw_rect.max.y - 2.5)],
                Stroke::new(2.0, Color32::from_rgb(195, 200, 210)),
            );
        }

        // Note labels: ONLY on C keys (C2, C3, C4...) for authentic acoustic piano appearance
        let is_c = p % 12 == 0;
        if is_c {
            let label = pitch_name(p);
            let color = if active { Color32::BLACK } else { Color32::from_rgb(70, 75, 90) };
            ui.painter().text(pos2(draw_rect.center().x, draw_rect.max.y - 12.0), Align2::CENTER_CENTER, label, bold(9.5), color);
        }

        // QWERTY shortcut letter preview (e.g. A, S, D, F, G, H, J, K)
        if let Some(letter) = qwerty_label_for_pitch(p, start_p) {
            ui.painter().text(
                pos2(draw_rect.center().x, draw_rect.max.y - 26.0),
                Align2::CENTER_CENTER,
                letter,
                bold(8.0),
                if active { Color32::from_rgb(10, 30, 50) } else { Color32::from_rgb(160, 165, 180) },
            );
        }
    }

    // 2. DRAW BLACK KEYS (Ebony 3D with Drop Shadows)
    for &(p, kr) in &black_rects {
        let is_held = app.keyboard.held_notes.contains(&p);
        let is_hovered = is_down && current_hovered_pitch == Some(p);
        let active = is_held || is_hovered;

        // Soft 3D drop shadow onto adjacent white keys
        let shadow_rect = Rect::from_min_size(pos2(kr.min.x + 1.5, kr.min.y + 2.0), kr.size());
        ui.painter().rect_filled(shadow_rect, 2.0, Color32::from_black_alpha(85));

        let draw_rect = if active {
            // Recessed depressed key
            Rect::from_min_size(pos2(kr.min.x + 0.5, kr.min.y + 2.5), vec2(kr.width() - 1.0, kr.height() - 2.5))
        } else {
            kr
        };

        let bg = if active {
            Color32::from_rgb(0, 180, 220)
        } else {
            Color32::from_rgb(18, 19, 23) // Deep matte ebony
        };

        // Keycap body
        ui.painter().rect(
            draw_rect,
            CornerRadius { nw: 0, ne: 0, sw: 2, se: 2 },
            bg,
            Stroke::new(1.0, if active { Color32::from_rgb(0, 250, 255) } else { Color32::from_rgb(10, 11, 14) }),
            StrokeKind::Inside,
        );

        // Top specular reflection highlight
        if !active {
            ui.painter().line_segment(
                [pos2(draw_rect.min.x + 2.0, draw_rect.min.y + 1.0), pos2(draw_rect.max.x - 2.0, draw_rect.min.y + 1.0)],
                Stroke::new(1.0, Color32::from_white_alpha(55)),
            );
            // Front lip bevel
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(draw_rect.min.x + 1.0, draw_rect.max.y - 3.5), pos2(draw_rect.max.x - 1.0, draw_rect.max.y)),
                1.0,
                Color32::from_rgb(38, 40, 48),
            );
        }

        // QWERTY shortcut letter preview (e.g. W, E, T, Y, U, O, P)
        if let Some(letter) = qwerty_label_for_pitch(p, start_p) {
            ui.painter().text(
                pos2(draw_rect.center().x, draw_rect.max.y - 10.0),
                Align2::CENTER_CENTER,
                letter,
                bold(7.5),
                if active { Color32::WHITE } else { Color32::from_rgb(120, 125, 140) },
            );
        }
    }
}

/// Releases notes the on-screen keyboard started whose release never arrived: the pointer left the
/// window, the dock was closed, another window took the mouse, or the key-up was swallowed. A
/// hosted instrument droning forever is worse than a cut note, so anything the keyboard started and
/// nobody is holding is released. Notes from a hardware MIDI keyboard are left alone: their own
/// note-offs are on the wire.
pub fn release_orphan_notes(app: &mut SoundApp, ctx: &egui::Context) {
    if app.keyboard.typed_notes.is_empty() {
        return;
    }
    // A release arriving this frame is handled by the keyboard itself: let it, so the instrument
    // gets one note-off and not two.
    // A window that lost the focus while a key was held never sees the key-up: release there too.
    if !ctx.input(|i| i.focused) {
        let orphans: Vec<u8> = app.keyboard.typed_notes.drain().collect();
        let target = app.keyboard.last_clicked_track.take().or_else(|| active_target_track(app));
        if let Some(tid) = target {
            for &pitch in &orphans {
                app.send_live_midi(tid, [0x80, pitch, 0]);
            }
        }
        for pitch in &orphans {
            app.keyboard.held_notes.remove(pitch);
        }
        app.keyboard.last_clicked = None;
        if !orphans.is_empty() {
            log::info!("released {} note(s) after the window lost the focus", orphans.len());
        }
        return;
    }
    let (pointer_down, keys_down, release_now) = ctx.input(|i| {
        let release_now =
            i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: false, .. } | egui::Event::PointerButton { pressed: false, .. }));
        (i.pointer.any_down(), COMPUTER_KEYS.iter().any(|k| i.key_down(*k)), release_now)
    });
    if pointer_down || keys_down || release_now {
        return;
    }
    let orphans: Vec<u8> = app.keyboard.typed_notes.drain().collect();
    let target = app.keyboard.last_clicked_track.take().or_else(|| active_target_track(app));
    if let Some(tid) = target {
        for &pitch in &orphans {
            app.send_live_midi(tid, [0x80, pitch, 0]);
        }
    }
    for pitch in &orphans {
        app.keyboard.held_notes.remove(pitch);
    }
    app.keyboard.last_clicked = None;
    log::info!("released {} note(s) the virtual keyboard never let go", orphans.len());
}

/// The computer keys that play notes (the same table the handler below walks).
const COMPUTER_KEYS: &[Key] =
    &[Key::A, Key::W, Key::S, Key::E, Key::D, Key::F, Key::T, Key::G, Key::Y, Key::H, Key::U, Key::J, Key::K, Key::O, Key::L, Key::P];

/// Maps computer keys to pitch offsets for instant musical playback.
///
/// Typing is captured nowhere else: when a text field (a track name, the browser search, a
/// dialog) has the keyboard, the DAW must not play notes, so the whole handler stands down.
fn handle_computer_keyboard_input(app: &mut SoundApp, ui: &mut Ui) {
    if ui.ctx().egui_wants_keyboard_input() || app.dialogs.open.is_some() {
        return;
    }
    let (start_p, _) = calculate_range(app.keyboard.num_keys, app.keyboard.base_octave);
    let target = active_target_track(app);

    let key_map = [
        (Key::A, 0),  // C
        (Key::W, 1),  // C#
        (Key::S, 2),  // D
        (Key::E, 3),  // D#
        (Key::D, 4),  // E
        (Key::F, 5),  // F
        (Key::T, 6),  // F#
        (Key::G, 7),  // G
        (Key::Y, 8),  // G#
        (Key::H, 9),  // A
        (Key::U, 10), // A#
        (Key::J, 11), // B
        (Key::K, 12), // C (+1)
        (Key::O, 13), // C# (+1)
        (Key::L, 14), // D (+1)
        (Key::P, 15), // D# (+1)
    ];

    for (k, semitone) in key_map {
        let pitch = (start_p as i32 + semitone).clamp(0, 127) as u8;
        // The operating system repeats a held key several times a second. A repeat is not a new
        // press: re-sending the note-on retriggered the instrument once per repeat (a stuck,
        // pulsing drone). Only a real press starts a note.
        let (pressed, released) = ui.input(|i| {
            let mut pressed = false;
            let mut released = false;
            for event in &i.events {
                match event {
                    egui::Event::Key { key, pressed: true, repeat: false, .. } if *key == k => pressed = true,
                    egui::Event::Key { key, pressed: false, .. } if *key == k => released = true,
                    _ => {}
                }
            }
            (pressed, released)
        });
        if pressed {
            if let Some(tid) = target {
                app.send_live_midi(tid, [0x90, pitch, app.keyboard.velocity]);
            }
            app.keyboard.held_notes.insert(pitch);
            app.keyboard.typed_notes.insert(pitch);
        }
        if released {
            if let Some(tid) = target {
                app.send_live_midi(tid, [0x80, pitch, 0]);
            }
            app.keyboard.held_notes.remove(&pitch);
            app.keyboard.typed_notes.remove(&pitch);
        }
    }
}

fn qwerty_label_for_pitch(pitch: u8, start_p: u8) -> Option<&'static str> {
    let diff = pitch as i32 - start_p as i32;
    match diff {
        0 => Some("A"),
        1 => Some("W"),
        2 => Some("S"),
        3 => Some("E"),
        4 => Some("D"),
        5 => Some("F"),
        6 => Some("T"),
        7 => Some("G"),
        8 => Some("Y"),
        9 => Some("H"),
        10 => Some("U"),
        11 => Some("J"),
        12 => Some("K"),
        13 => Some("O"),
        14 => Some("L"),
        15 => Some("P"),
        _ => None,
    }
}

fn step_record_note(app: &mut SoundApp, _track: TrackId, pitch: u8) {
    if let Some(cid) = crate::midi_editor::target_clip(app) {
        let s = app.engine.session();
        let (clip_start, at) = (s.find_clip(cid).map_or(0, |(_, c)| c.start), s.edit.selection.start);
        let rel = s.tempo.samples_to_ticks(at, s.sample_rate) - s.tempo.samples_to_ticks(clip_start, s.sample_rate);
        let len = soundcraft_time::TICKS_PER_QUARTER / 2;
        let _ = app.run("midi.note_add", json!({"clip": cid.0, "pitch": pitch, "start_ticks": rel.max(0), "length_ticks": len}));
        let s = app.engine.session();
        let next = s.tempo.tick_to_samples(s.tempo.samples_to_ticks(at, s.sample_rate) + len, s.sample_rate);
        let _ = app.engine.execute("edit.locate", &json!({"at": next}));
    }
}
