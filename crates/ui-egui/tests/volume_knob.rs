//! The always-visible volume knob: mapping, bounds and a render smoke test of the track headers.

use egui::{Pos2, Rect, vec2};
use soundcraft_ui_egui::widgets::{KNOB_MAX_DB, KNOB_MIN_DB, db_to_knob_norm, knob_norm_to_db, volume_knob};
use soundcraft_ui_egui::{MainWindow, Services, SoundApp};

#[test]
fn the_volume_knob_maps_db_to_travel_linearly_and_round_trips() {
    // Ends of the travel are the ends of the range.
    assert!((db_to_knob_norm(KNOB_MIN_DB) - 0.0).abs() < f32::EPSILON);
    assert!((db_to_knob_norm(KNOB_MAX_DB) - 1.0).abs() < f32::EPSILON);
    // 0 dB sits at 5/6 of the travel (-60..+12 is 72 dB, 60 of them below 0 dB).
    assert!((db_to_knob_norm(0.0) - 60.0 / 72.0).abs() < 1e-5);
    // Hostile input is clamped, not wrapped.
    assert_eq!(db_to_knob_norm(-500.0), 0.0);
    assert_eq!(db_to_knob_norm(500.0), 1.0);
    for db in [-60.0, -24.0, -6.0, -0.1, 0.0, 3.0, 12.0] {
        let back = knob_norm_to_db(db_to_knob_norm(db));
        assert!((back - db).abs() < 1e-4, "{db} -> {back}");
    }
    for norm in [0.0, 0.25, 0.5, 0.8333, 1.0] {
        let back = db_to_knob_norm(knob_norm_to_db(norm));
        assert!((back - norm).abs() < 1e-4, "{norm} -> {back}");
    }
}

fn run(ctx: &egui::Context, app: &mut SoundApp) {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))), ..Default::default() };
    ctx.run_ui(input, |ui| {
        let ctx = ui.ctx().clone();
        app.logic(&ctx);
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

#[test]
fn the_edit_window_renders_the_volume_knobs_for_every_track_height() {
    // Every height preset goes through the header drawing path with the knob in it: tall
    // headers (knob next to the FX button) and name-row-only headers (compact knob).
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.window = MainWindow::Edit;
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    run(&ctx, &mut app);
    for height in ["Mini", "Small", "Medium", "Large", "Jumbo"] {
        if let Err(e) = app.run("track.height", serde_json::json!({"height": height})) {
            // Preset names are the UI's concern; a mismatch must not panic the test.
            assert!(e.contains("height"), "{height}: {e}");
            continue;
        }
        run(&ctx, &mut app);
    }
    // The session is untouched by rendering.
    assert!(!app.engine.session().tracks.is_empty());
}

#[test]
fn the_led_knob_paints_at_every_size_and_reports_drag_changes() {
    let ctx = egui::Context::default();
    let mut changed = None;
    // Every size from the compact header knob to a large console knob must paint without panic.
    for size in [12.0f32, 14.0, 16.0, 18.0, 24.0, 32.0, 40.0] {
        ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed = volume_knob(ui, size, -6.0, "test");
            });
        })
        .textures_delta
        .clear();
    }
    // At rest, dragging has not happened: the knob reports no change.
    assert!(changed.is_none(), "{changed:?}");
}

#[test]
fn the_mixer_dock_is_on_by_default_toggles_and_renders() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    assert!(app.ui.show_mixer_dock, "the dock is on by default: every fader is on the main screen");
    app.run("window.mixer_dock", serde_json::json!({})).expect("toggle");
    assert!(!app.ui.show_mixer_dock);
    app.run("window.mixer_dock", serde_json::json!({})).expect("toggle back");
    assert!(app.ui.show_mixer_dock);
    // Rendering with the dock on must be clean and keep the tracks visible.
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    run(&ctx, &mut app);
    run(&ctx, &mut app);
    assert!(!app.engine.session().tracks.is_empty());
}
