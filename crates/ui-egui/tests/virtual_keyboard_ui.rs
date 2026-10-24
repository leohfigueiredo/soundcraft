//! The horizontal virtual keyboard dock: it renders in the Edit window, plays notes on a mouse
//! press, holds them while pressed and releases them, and stays quiet while a text field has the
//! keyboard.

use egui::{Key, Modifiers, Pos2, Rect, vec2};
use soundcraft_ui_egui::{Services, SoundApp};

const W: f32 = 1600.0;
const H: f32 = 1000.0;

fn run(ctx: &egui::Context, app: &mut SoundApp, events: Vec<egui::Event>) {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(W, H))), events, ..Default::default() };
    ctx.run_ui(input, |ui| {
        let ctx = ui.ctx().clone();
        app.logic(&ctx);
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

fn press_at(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]
}

fn release_at(pos: Pos2) -> Vec<egui::Event> {
    vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]
}

#[test]
fn the_keyboard_dock_plays_and_releases_a_note_on_a_mouse_press() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.show_midi_keyboard = true;
    let ctx = egui::Context::default();
    // Two frames: the first lays the dock out, the second is where a user would click.
    run(&ctx, &mut app, Vec::new());
    run(&ctx, &mut app, Vec::new());

    // A point inside the keybed: below the keyboard's control bar, right of the wheel bay.
    let key = Pos2::new(600.0, H - 90.0);
    run(&ctx, &mut app, press_at(key));
    assert!(!app.keyboard.held_notes.is_empty(), "pressing a key holds a note: {:?}", app.keyboard.held_notes);
    assert!(app.keyboard.last_clicked.is_some());

    run(&ctx, &mut app, release_at(key));
    assert!(app.keyboard.held_notes.is_empty(), "releasing clears the held notes");
    assert!(app.keyboard.last_clicked.is_none());
}

#[test]
fn the_computer_keyboard_plays_notes_and_stands_down_for_text_input() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.show_midi_keyboard = true;
    let ctx = egui::Context::default();
    run(&ctx, &mut app, Vec::new());

    let key_event = |pressed| egui::Event::Key { key: Key::A, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    run(&ctx, &mut app, vec![key_event(true)]);
    assert!(!app.keyboard.held_notes.is_empty(), "A plays the first key of the range");
    run(&ctx, &mut app, vec![key_event(false)]);
    assert!(app.keyboard.held_notes.is_empty(), "releasing A releases the note");

    // A dialog that takes typing (a track rename) owns the keyboard: no notes sound.
    let first = app.engine.session().tracks.first().map(|t| t.id);
    if let Some(id) = first {
        app.dialogs.open_rename_track(id, "Track");
    }
    app.keyboard.held_notes.clear();
    run(&ctx, &mut app, vec![key_event(true)]);
    assert!(app.keyboard.held_notes.is_empty(), "typing in a dialog must not play notes");
}

#[test]
fn incoming_midi_skips_a_selected_track_without_an_instrument() {
    use soundcraft_model::{ChannelFormat, Insert, Session, TrackKind};

    let mut session = Session::default();
    let empty_midi = session.add_track(TrackKind::Midi, ChannelFormat::Stereo, Some("MIDI"));
    let instrument = session.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Synth"));
    if let Some(track) = session.track_mut(instrument) {
        track.instrument = Some(Insert::new("subtractive_synth"));
    }
    session.edit.selected_tracks = vec![empty_midi];
    let app = SoundApp::new(soundcraft_engine::Engine::new(session), None, Services::default());

    assert_eq!(soundcraft_ui_egui::virtual_keyboard::active_target_track(&app), Some(instrument));
}

#[test]
fn an_open_instrument_console_never_routes_notes_to_a_different_plugin() {
    use soundcraft_model::{ChannelFormat, Insert, Session, TrackKind};

    let mut session = Session::default();
    let other = session.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Other instrument"));
    if let Some(track) = session.track_mut(other) {
        track.instrument = Some(Insert::new("subtractive_synth"));
    }
    session.edit.selected_tracks = vec![other];
    let mut app = SoundApp::new(soundcraft_engine::Engine::new(session), None, Services::default());
    app.ui.show_console = true;
    app.ui.console_profile = "pianoteq".into();

    assert_eq!(
        soundcraft_ui_egui::virtual_keyboard::active_target_track(&app),
        None,
        "a Pianoteq console must not play the selected, unrelated instrument"
    );

    let mut session = app.engine.session().clone();
    let piano = session.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Pianoteq"));
    let plugin_id = soundcraft_ui_egui::pianoteq_console::PROFILE.plugin_ids.first().copied().unwrap_or_default();
    if let Some(track) = session.track_mut(piano) {
        track.instrument = Some(Insert::new(plugin_id));
    }
    app.engine = soundcraft_engine::Engine::new(session);
    assert_eq!(
        soundcraft_ui_egui::virtual_keyboard::active_target_track(&app),
        Some(piano),
        "the Pianoteq console should find its matching track even if another plugin is selected"
    );

    app.ui.show_console = false;
    assert_eq!(
        soundcraft_ui_egui::virtual_keyboard::active_target_track(&app),
        Some(other),
        "without the console, the regular selected-track routing is unchanged"
    );
}

#[test]
fn shift_p_opens_the_instrument_console_even_with_commands_keyboard_focus() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    assert_eq!(app.engine.session().edit.keyboard_focus, "commands");
    app.ui.show_console = false;

    let ctx = egui::Context::default();
    run(&ctx, &mut app, Vec::new());
    run(&ctx, &mut app, Vec::new());
    run(&ctx, &mut app, vec![egui::Event::Key { key: Key::P, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT }]);

    assert!(app.ui.show_console, "Shift+P is the global Instrument Console shortcut");
}

#[test]
fn the_virtual_keyboard_releases_on_original_track_when_selection_changes_while_held() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.show_midi_keyboard = true;
    let ctx = egui::Context::default();
    run(&ctx, &mut app, Vec::new());
    run(&ctx, &mut app, Vec::new());

    let orig_track = soundcraft_ui_egui::virtual_keyboard::active_target_track(&app).expect("an instrument track");
    let key = Pos2::new(600.0, H - 90.0);
    run(&ctx, &mut app, press_at(key));
    assert!(app.keyboard.last_clicked.is_some());
    assert_eq!(app.keyboard.last_clicked_track, Some(orig_track));

    // Deselect all tracks or change selection
    app.engine.session_mut().edit.selected_tracks.clear();

    run(&ctx, &mut app, release_at(key));
    assert!(app.keyboard.held_notes.is_empty());
    assert!(app.keyboard.last_clicked.is_none());
    assert!(app.keyboard.last_clicked_track.is_none());

    // Check live MIDI log: the note off was sent to orig_track!
    let note_offs: Vec<_> = app.live_midi_log.iter().filter(|(t, m)| *t == orig_track.0 && (m[0] & 0xF0 == 0x80)).collect();
    assert!(!note_offs.is_empty(), "note-off was routed to original track even after selection changed: {note_offs:?}");
}
