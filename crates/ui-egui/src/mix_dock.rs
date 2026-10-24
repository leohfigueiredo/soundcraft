//! The Mixer Dock: a console of vertical channel strips under the Edit window's timeline, so every
//! track's fader is on the main screen without switching to the Mix window.

use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::{fader, fader_slot, meter, pan_knob, pan_text};
use crate::{MainWindow, SoundApp};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_model::{TrackId, TrackKind};

/// A channel strip in the dock, and the height its controls get.
const STRIP_W: f32 = 72.0;
const STRIP_H: f32 = 236.0;

pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let ids: Vec<TrackId> = app.engine.session().tracks.iter().filter(|x| !x.hidden && x.kind != TrackKind::Video).map(|x| x.id).collect();
    // Header strip: title, the way to the full console, and the connection lights.
    let head_r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 18.0));
    ui.painter().rect_filled(head_r, 0.0, t.panel_bg2);
    ui.painter().text(pos2(head_r.min.x + 8.0, head_r.center().y), Align2::LEFT_CENTER, "MIXER", bold(10.0), t.accent);
    ui.painter().text(
        pos2(head_r.min.x + 66.0, head_r.center().y),
        Align2::LEFT_CENTER,
        "vertical faders of every track: click a name to select it, drag the fader to set the level",
        regular(9.0),
        t.text_dim,
    );
    use crate::midi_io::LinkState;
    let now = ui.input(|i| i.time);
    let (mcu_dot, mcu_col) = match app.msc.link.state {
        LinkState::Connected => ("●", Color32::from_rgb(0, 220, 130)),
        LinkState::Searching => ("◐", t.meter_yellow),
        LinkState::Unavailable => ("○", t.meter_red),
    };
    let flash = app.msc.input_active(now, 0.5);
    let mcu_label = if flash { "MCU ●" } else { "MCU" };
    let mcu_r = Rect::from_center_size(pos2(head_r.max.x - 300.0, head_r.center().y), vec2(60.0, 14.0));
    ui.painter().text(mcu_r.center(), Align2::CENTER_CENTER, format!("{mcu_dot} {mcu_label}"), bold(9.0), mcu_col);
    let (kb_dot, kb_col) = match app.msc.link.keyboard_state {
        LinkState::Connected => ("●", Color32::from_rgb(0, 220, 130)),
        LinkState::Searching => ("◐", t.meter_yellow),
        LinkState::Unavailable => ("○", t.meter_red),
    };
    let kb_label = app.msc.keyboard_port.as_deref().unwrap_or("no keyboard");
    let kb_r = Rect::from_center_size(pos2(head_r.max.x - 210.0, head_r.center().y), vec2(110.0, 14.0));
    ui.painter().with_clip_rect(kb_r).text(kb_r.center(), Align2::CENTER_CENTER, format!("{kb_dot} 🎹 {kb_label}"), regular(9.0), kb_col);
    let full = Rect::from_center_size(pos2(head_r.max.x - 92.0, head_r.center().y), vec2(170.0, 14.0));
    let fresp = ui.interact(full, ui.id().with("mixer_dock_full"), Sense::click());
    ui.painter().text(
        full.center(),
        Align2::CENTER_CENTER,
        if app.ui.mix_separate { "Mix window (separate)" } else { "Full console (Window ▸ Mix)" },
        regular(9.0),
        if fresp.hovered() { t.text } else { t.text_dim },
    );
    if fresp.clicked() {
        let _ = app.run("window.toggle_mix_edit", json!({}));
    }
    ui.add_space(20.0);

    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for id in ids {
                strip(app, ui, id);
            }
            if app.engine.session().tracks.iter().all(|x| x.hidden || x.kind == TrackKind::Video) {
                ui.label(egui::RichText::new("No tracks yet: File ▸ New Session, or Track ▸ New…").small().color(t.text_dim));
            }
        });
    });
    let _ = MainWindow::Edit;
}

fn strip(app: &mut SoundApp, ui: &mut Ui, id: TrackId) {
    let t = Tokens::current();
    // Copy what the strip draws: the session borrow must end before any command runs.
    let Some((name, colour, volume_db, pan, mute, solo, arm)) = app.engine.session().track(id).map(|tr| {
        (
            tr.name.clone(),
            tr.color,
            tr.mixer.volume_db,
            tr.mixer.pan.first().copied().unwrap_or(0.0),
            tr.mixer.mute,
            tr.mixer.solo,
            tr.mixer.record_arm,
        )
    }) else {
        return;
    };
    let selected = app.engine.session().edit.selected_tracks.contains(&id);
    let (r, _) = ui.allocate_exact_size(vec2(STRIP_W, STRIP_H), Sense::hover());
    let bg = if selected { t.strip_selected } else { t.strip_bg };
    ui.painter().rect_filled(r, 1.0, bg);
    if selected {
        ui.painter().rect_stroke(r, 1.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
    }
    // Colour tab on top, name at the bottom (console-like).
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 3.0)), 0.0, crate::theme::rgb(colour));
    let name_r = Rect::from_min_size(pos2(r.min.x + 2.0, r.max.y - 15.0), vec2(r.width() - 4.0, 13.0));
    let nresp = ui.interact(name_r, ui.id().with(("mixdock_name", id.0)), Sense::click());
    let name_col = if nresp.hovered() { t.text } else { t.text.lerp_to_gamma(t.text_dim, 0.3) };
    ui.painter().with_clip_rect(name_r).text(name_r.center(), Align2::CENTER_CENTER, &name, bold(9.5), name_col);
    if nresp.clicked() {
        app.engine.session_mut().edit.selected_tracks = vec![id];
    }
    if nresp.double_clicked() {
        app.dialogs.open_rename_track(id, &name);
    }
    // Mute / solo / record arm, above the name.
    let bw = (r.width() - 8.0) / 3.0;
    let mut x = r.min.x + 4.0;
    let btn_y = r.max.y - 32.0;
    for (label, on, cmd, colour_on) in [("M", mute, "mix.mute", t.mute), ("S", solo, "mix.solo", t.solo), ("R", arm, "mix.record_arm", t.rec)] {
        let br = Rect::from_min_size(pos2(x, btn_y), vec2(bw - 2.0, 15.0));
        let resp = ui.interact(br, ui.id().with(("mixdock_btn", id.0, label)), Sense::click());
        ui.painter().rect_filled(br, 2.0, if on { colour_on } else { t.button });
        ui.painter().text(br.center(), Align2::CENTER_CENTER, label, bold(8.5), if on { Color32::BLACK } else { t.text_dim });
        if resp.clicked() {
            if ui.input(|i| i.modifiers.alt) && label == "S" {
                let _ = app.run("mix.clear_solos", json!({}));
            } else {
                let _ = app.run(cmd, json!({"track": id.0}));
                if label == "S" && !on {
                    app.engine.session_mut().edit.selected_tracks = vec![id];
                }
            }
        }
        x += bw;
    }
    // Pan knob and its readout.
    let pan_r = Rect::from_center_size(pos2(r.center().x, btn_y - 18.0), vec2(28.0, 28.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pan_r));
    let mut v = pan;
    let before = v;
    let resp = pan_knob(&mut child, 26.0, &mut v, "Pan (drag; double-click centres)");
    if (v - before).abs() > f32::EPSILON {
        let _ = app.engine.execute_merged("mix.pan", &json!({"track": id.0, "pan": v, "index": 0}), &format!("pan:{}:0", id.0));
    }
    let _ = resp;
    let txt = Rect::from_center_size(pos2(r.center().x, pan_r.min.y - 8.0), vec2(r.width() - 8.0, 12.0));
    ui.painter().text(txt.center(), Align2::CENTER_CENTER, pan_text(pan), mono(8.5), t.counter_text);
    // Vertical fader + stereo meter, between the pan knob and the buttons.
    let top = r.min.y + 24.0;
    let bottom = pan_r.min.y - 14.0;
    let fh = (bottom - top).max(40.0);
    let fader_r = Rect::from_min_size(pos2(r.min.x + 5.0, top), vec2((r.width() * 0.55).max(20.0), fh));
    if let Some(db) = fader(ui, fader_r, volume_db, ui.id().with(("mixdock_fader", id.0))) {
        app.gesture = Some(crate::Gesture::Fader { track: id, start_db: volume_db });
        let _ = app.engine.execute_merged("mix.volume", &json!({"track": id.0, "db": db}), &format!("fader:{}", id.0));
    }
    let md = app.meters.get(&id).copied().unwrap_or_default();
    let slot = fader_slot(fader_r);
    let meter_r = Rect::from_min_max(pos2(fader_r.max.x + 3.0, slot.min.y - 5.0), pos2(r.max.x - 4.0, slot.max.y + 1.0));
    let mw = meter_r.width() * 0.5 - 1.0;
    meter(ui, Rect::from_min_size(meter_r.min, vec2(mw, meter_r.height())), md.level[0], md.hold[0], md.clip);
    meter(ui, Rect::from_min_size(pos2(meter_r.min.x + mw + 2.0, meter_r.min.y), vec2(mw, meter_r.height())), md.level[1], md.hold[1], md.clip);
}
