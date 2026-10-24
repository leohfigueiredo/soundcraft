//! The on-screen keyboard driven by the mouse, which is how it is actually played.

use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use soundcraft_ui_egui::{Services, SoundApp};

fn app() -> SoundApp {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    let track = app.engine.session().tracks.iter().find(|t| t.kind.is_midi() && t.instrument.is_some()).map(|t| t.id).expect("an instrument track");
    app.engine.session_mut().edit.selected_tracks = vec![track];
    app.ui.show_midi_keyboard = true;
    app
}

fn harness(app: SoundApp) -> Harness<'static, SoundApp> {
    Harness::builder().with_size(Vec2::new(1400.0, 950.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

fn statuses(h: &Harness<'static, SoundApp>) -> Vec<u8> {
    h.state().live_midi_log.iter().map(|(_, m)| m[0] & 0xF0).collect()
}

#[test]
fn a_mouse_press_on_a_key_starts_a_note_and_the_release_stops_it() {
    let mut h = harness(app());
    h.run_steps(4);
    // The keyboard dock is 155 px tall at the bottom of the Edit window; the key bed is its lower
    // half. Press in the middle of it.
    let pos = Pos2::new(700.0, 870.0);
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let on = statuses(&h).iter().filter(|s| **s == 0x90).count();
    assert_eq!(on, 1, "the press starts exactly one note (log: {:?})", h.state().live_midi_log);
    assert!(h.state().keyboard.last_clicked.is_some(), "the key is held");
    // Release.
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let off = statuses(&h).iter().filter(|s| **s == 0x80).count();
    assert_eq!(off, 1, "the release stops exactly one note (log: {:?})", h.state().live_midi_log);
    assert!(h.state().keyboard.last_clicked.is_none(), "and nothing is left held");
    assert!(h.state().keyboard.typed_notes.is_empty());
}

#[test]
fn letting_go_outside_the_keyboard_still_stops_the_note() {
    let mut h = harness(app());
    h.run_steps(4);
    let pos = Pos2::new(700.0, 870.0);
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(3);
    // The pointer is dragged off the keys (still pressed), then released over the timeline.
    let away = Pos2::new(700.0, 300.0);
    h.input_mut().events.push(Event::PointerMoved(away));
    h.run_steps(2);
    h.input_mut().events.push(Event::PointerButton { pos: away, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(!h.state().live_midi_log.is_empty() && statuses(&h).contains(&0x80), "the note is stopped (log: {:?})", h.state().live_midi_log);
    assert!(h.state().keyboard.typed_notes.is_empty(), "nothing left held");
}

#[test]
fn all_midi_notes_off_clears_what_the_keyboard_was_holding() {
    // The panic command used to be a no-op. It now clears the keyboard's held state (and, with a
    // player, sends the silence messages to every instrument — see the mix test).
    let mut app = app();
    app.keyboard.held_notes.insert(60);
    app.keyboard.typed_notes.insert(60);
    let mut h = harness(app);
    h.run_steps(2);
    let executed = h.state_mut().engine.execute("event.all_notes_off", &serde_json::json!({}));
    assert!(executed.is_ok(), "the command exists: {executed:?}");
    h.run_steps(2);
    assert!(h.state().keyboard.held_notes.is_empty(), "nothing is left lit");
    assert!(h.state().keyboard.typed_notes.is_empty(), "nothing is left held");
}
