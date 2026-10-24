//! The instrument console: one window, an instrument tab row, its own theme per instrument.

use egui::{Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soundcraft_ui_egui::console_kit::{self, Icon, profiles};
use soundcraft_ui_egui::{Services, SoundApp};

const WINDOW_PX: Vec2 = vec2(1400.0, 950.0);

fn app() -> SoundApp {
    SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default())
}

fn add_instrument_track(app: &mut SoundApp, profile: &'static soundcraft_ui_egui::console_kit::Profile) {
    use soundcraft_model::{ChannelFormat, Insert, TrackKind};

    assert!(!profile.plugin_ids.is_empty(), "{} needs a plugin id for the routing test", profile.id);
    let session = app.engine.session_mut();
    let track_id = session.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some(profile.brand));
    if let Some(track) = session.track_mut(track_id)
        && let Some(plugin_id) = profile.plugin_ids.first()
    {
        track.instrument = Some(Insert::new(*plugin_id));
    }
    session.edit.selected_tracks = vec![track_id];
}

fn harness(app: SoundApp) -> Harness<'static, SoundApp> {
    Harness::builder().with_size(WINDOW_PX).with_pixels_per_point(1.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

#[test]
fn the_console_ships_several_instruments_each_with_its_own_theme() {
    assert!(profiles().len() >= 6, "the console must offer several instruments, got {}", profiles().len());
    let mut accents = std::collections::HashSet::new();
    for profile in profiles() {
        assert!(profile.window_title().contains(profile.name), "{}: the title names the instrument", profile.id);
        assert!(!profile.racks.is_empty() && !profile.specs.is_empty(), "{}: the profile has no content", profile.id);
        assert!(accents.insert(profile.palette.accent), "{}: the accent colour is not unique", profile.id);
    }
    for id in ["pianoteq", "pigments", "omnisphere", "kontakt", "analoglab", "labs"] {
        assert!(console_kit::profile(id).is_some(), "profile `{id}` is missing");
    }
}

#[test]
fn choosing_another_instrument_changes_the_title_the_theme_and_the_racks() {
    let mut app = app();
    console_kit::open(&mut app, "pianoteq");
    assert!(app.ui.show_console);
    let mut h = harness(app);
    h.run_steps(6);
    // The tab row lists every instrument.
    assert!(h.query_by_label("INSTRUMENT").is_some(), "the instrument tab row is drawn");
    for profile in profiles() {
        // The tab carries the VST's name (marked with a dot when it is on the active track).
        assert!(h.query_all_by_label_contains(profile.name).count() >= 1, "tab `{}` is missing", profile.name);
    }
    // The header names the instrument the console is on (its brand and kind).
    let active = soundcraft_ui_egui::console_kit::active(h.state());
    assert!(h.query_all_by_label(active.brand).count() >= 1, "the header shows `{}`", active.brand);
    // The tab row must not repeat the generic word "CONSOLE": it names the instruments.
    assert!(h.query_all_by_label("CONSOLE").count() == 0, "the tabs show VST names, not `CONSOLE`");
    // Pianoteq's racks are on screen, Omnisphere's are not.
    assert!(h.query_by_label("TUNING & STRING").is_some(), "Pianoteq's racks are drawn");
    assert!(h.query_by_label("PART").is_none(), "Omnisphere's racks are not drawn until it is selected");
    assert!(h.query_by_label("ACOUSTIC GRANDS").is_some(), "Pianoteq category tabs are drawn");

    // Switch instrument through the same command the menu/toolbar uses.
    let result = soundcraft_ui_egui::menus::run_ui_command(h.state_mut(), "window.console.omnisphere", &serde_json::json!({}));
    assert!(result.is_some(), "the per-instrument command exists");
    h.step();
    h.run_steps(4);
    assert_eq!(h.state().ui.console_profile, "omnisphere");
    assert!(h.query_by_label("PART").is_some(), "Omnisphere's racks are drawn after switching");
    assert!(h.query_by_label("TUNING & STRING").is_none(), "Pianoteq's racks are gone");
    for profile in profiles() {
        soundcraft_ui_egui::console_kit::open(h.state_mut(), profile.id);
        h.run_steps(2);
        assert!(h.query_all_by_label(profile.brand).count() >= 1, "{} header shows its brand", profile.brand);
    }

    // The window title follows the instrument.
    let title = console_kit::profile("omnisphere").map(|p| p.window_title()).unwrap_or_default();
    assert_eq!(title, "Omnisphere — Alternative GUI", "the window is titled after the chosen VST");
}

#[test]
fn each_instrument_keeps_its_own_values_and_activity_light() {
    let mut app = app();
    let piano = console_kit::profile("pianoteq").expect("pianoteq profile");
    let omni = console_kit::profile("omnisphere").expect("omnisphere profile");
    console_kit::dispatch(&mut app, piano, "hammer_piano", 77.0);
    console_kit::dispatch(&mut app, omni, "layer_a_level", 42.0);
    assert_eq!(console_kit::state(&app, piano).value(piano, "hammer_piano"), 77.0);
    assert_eq!(console_kit::state(&app, omni).value(omni, "layer_a_level"), 42.0);
    assert_eq!(console_kit::state(&app, piano).value(piano, "layer_a_level"), 0.0, "one instrument's ids are not another's");
    // Unknown ids are ignored by every profile.
    console_kit::dispatch(&mut app, omni, "hammer_piano", 10.0);
    assert_eq!(console_kit::state(&app, omni).value(omni, "hammer_piano"), 0.0);
}

#[test]
fn the_console_has_a_shortcut_and_a_toolbar_button() {
    assert_eq!(soundcraft_ui_egui::menus::shortcut_for("window.console"), Some("Shift+P"));
    assert!(soundcraft_ui_egui::shortcuts::parse("Shift+P").is_some());
    let mut app = app();
    let mut h = harness(app);
    h.run_steps(4);
    assert!(h.query_by_label_contains("ALT GUI").is_some(), "the main toolbar shows the ALT GUI button");
    app = h.into_state();
    let result = soundcraft_ui_egui::menus::run_ui_command(&mut app, "window.console", &serde_json::json!({}));
    assert!(result.is_some());
    assert!(app.ui.show_console);
}

#[test]
fn the_kontakt_console_lists_libraries_and_selecting_one_sends_a_program_change() {
    // The list comes from the scan script's JSON; point the loader at a fixture.
    soundcraft_ui_egui::console_kit::set_libraries(vec![
        soundcraft_ui_egui::console_kit::Library { name: "Alicias Keys".into(), path: "F:\\Pianote\\Alicias Keys".into() },
        soundcraft_ui_egui::console_kit::Library { name: "Abbey Road 60s Drummer".into(), path: String::new() },
        soundcraft_ui_egui::console_kit::Library { name: "Berlin Concert Grand".into(), path: String::new() },
    ]);
    let profile = console_kit::profile("kontakt").expect("kontakt profile");
    let mut app = app();
    add_instrument_track(&mut app, profile);
    console_kit::open(&mut app, "kontakt");
    let mut h = harness(app);
    h.run_steps(6);
    // The picker names the libraries and shows the current one in the header.
    for name in ["Alicias Keys", "Abbey Road 60s Drummer", "Berlin Concert Grand"] {
        assert!(h.query_all_by_label_contains(name).count() >= 1, "library `{name}` is listed");
    }
    // "LIBRARY" appears twice: the header line and the picker's own title.
    assert!(h.query_all_by_label("LIBRARY").count() >= 2, "the library picker is drawn");

    // Selecting the third library stores it and dispatches a program change (index 2).
    console_kit::choose_library(h.state_mut(), profile, "kontakt_library", 2);
    assert_eq!(console_kit::state(h.state(), profile).value(profile, "kontakt_library"), 2.0);
    let (msg, _) = h.state().ui.console_sent.expect("the selection is dispatched");
    assert_eq!(msg[0] & 0xF0, 0xC0, "a library is a program change");
    assert_eq!(msg[1], 2, "the program number is the library index");
}

#[test]
fn the_console_never_touches_the_plugin_editors() {
    let mut app = app();
    let track = app.engine.session().tracks.iter().find(|t| t.kind == soundcraft_model::TrackKind::Audio).map(|t| t.id);
    if let Some(track) = track {
        app.ui.plugin_windows.push((track, 0));
    }
    let before = app.ui.plugin_windows.clone();
    let result = soundcraft_ui_egui::menus::run_ui_command(&mut app, "window.console.pigments", &serde_json::json!({}));
    assert!(result.is_some());
    assert!(app.ui.show_console);
    assert_eq!(app.ui.plugin_windows, before, "plugin windows are untouched");
}

#[test]
fn every_instrument_has_a_pictogram_and_a_valid_program_number() {
    let mut total = 0;
    let mut kinds = std::collections::HashSet::new();
    for profile in profiles() {
        let mut programs = std::collections::HashSet::new();
        for model in profile.models {
            total += 1;
            kinds.insert(model.icon);
            assert!(
                model.category < profile.categories.len(),
                "{}: `{}` points at category {} but there are only {}",
                profile.id,
                model.name,
                model.category,
                profile.categories.len()
            );
            assert!(programs.insert(model.program), "{}: program {} is used twice (two instruments share it)", profile.id, model.program);
            assert!(!model.name.is_empty(), "{}: an instrument has no name", profile.id);
        }
    }
    assert!(total >= 90, "the consoles list the instruments of every hosted VST, got {total}");
    assert!(kinds.len() >= 8, "the pictogram vocabulary must tell instruments apart, got {}", kinds.len());
}

#[test]
fn model_tabs_are_labelled_and_selecting_one_sends_its_program_change() {
    let profile = console_kit::profile("pianoteq").expect("pianoteq profile");
    let mut app = app();
    add_instrument_track(&mut app, profile);
    console_kit::open(&mut app, "pianoteq");
    let mut h = harness(app);
    h.run_steps(6);
    // The add-on instruments are on screen and reachable by name.
    for name in ["Church Bells", "Blanchet Harpsichord", "K1 Grand", "Rhody R1 (MK1 73)", "Steinway D New York"] {
        assert!(h.query_all_by_label_contains(name).count() >= 1, "instrument `{name}` is listed");
    }
    // Clicking an instrument selects it and sends its program change.
    let index = profile.models.iter().position(|m| m.name == "Church Bells").expect("Church Bells");
    h.get_by_label("Church Bells").click();
    h.run_steps(4);
    assert_eq!(console_kit::state(h.state(), profile).model, index, "the click selects the instrument");
    let (msg, _) = h.state().ui.console_sent.expect("the selection is dispatched");
    assert_eq!(msg[0] & 0xF0, 0xC0, "picking an instrument is a program change");
    assert_eq!(msg[1], profile.models[index].program, "the program number is the instrument's");
}

#[test]
fn every_pictogram_draws_inside_its_box() {
    // The icons are drawn from primitives, so their geometry can be checked without a GPU: each
    // kind must emit shapes, keep them inside the box it is given, and actually fill it.
    let box_side = 16.0;
    let r = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), vec2(box_side, box_side));
    let slack = r.expand(1.0);
    let mut signatures = std::collections::HashSet::new();
    for icon in Icon::ALL {
        let shapes = icon.shapes(r, egui::Color32::WHITE);
        assert!(!shapes.is_empty(), "{icon:?}: draws nothing");
        let mut ink = egui::Rect::NOTHING;
        for shape in &shapes {
            let bounds = shape.visual_bounding_rect();
            assert!(bounds.width() > 0.0 || bounds.height() > 0.0, "{icon:?}: a shape is empty");
            assert!(slack.contains_rect(bounds), "{icon:?}: a shape leaves the box ({bounds:?})");
            ink = ink.union(bounds);
        }
        assert!(ink.width() >= box_side * 0.5, "{icon:?}: the pictogram is only {:.0}% wide", ink.width() / box_side * 100.0);
        assert!(ink.height() >= box_side * 0.4, "{icon:?}: the pictogram is only {:.0}% tall", ink.height() / box_side * 100.0);
        // No two kinds draw the same picture.
        let signature = format!("{shapes:?}");
        assert!(signatures.insert(signature), "{icon:?} is drawn like another kind");
    }
    assert_eq!(signatures.len(), Icon::ALL.len());
}

#[test]
fn a_sampler_library_draws_the_pictogram_of_what_it_holds() {
    // Kontakt's libraries are named after their instrument; the picker reads that name.
    let icon = |name: &str| console_kit::Library { name: name.to_string(), path: String::new() }.icon();
    assert_eq!(icon("Berlin Concert Grand"), Icon::Grand);
    assert_eq!(icon("Steinway D"), Icon::Grand);
    // A library that only says "Keys" gets the keys pictogram, not a grand lid.
    assert_eq!(icon("Alicia's Keys"), Icon::Keys);
    assert_eq!(icon("Abbey Road 60s Drummer"), Icon::Sampler);
    assert_eq!(icon("Symphonic Strings"), Icon::Strings);
    assert_eq!(icon("Studio Choir"), Icon::Strings);
    assert_eq!(icon("Session Horns"), Icon::Brass);
    assert_eq!(icon("Church Bells"), Icon::Bell);
    assert_eq!(icon("Sub Bass"), Icon::Bass);
    assert_eq!(icon("Massive Synth"), Icon::Synth);
    assert_eq!(icon("Vintage Organ"), Icon::Keys);
    assert_eq!(icon("Ambient Textures"), Icon::Pad);
    // Anything unrecognised still gets a pictogram, so no row is blank.
    assert_eq!(icon("Some Kontakt Library"), Icon::Sampler);
}

#[test]
fn a_track_with_a_console_instrument_shows_an_alt_button_that_opens_it() {
    // The instrument row of a channel strip: a hosted VST that has a console gets an ALT button
    // next to GUI, and the console opens on that instrument with the track selected.
    let mut app = app();
    let track = app
        .engine
        .session()
        .tracks
        .iter()
        .find(|t| t.kind.is_midi() && t.instrument.is_some())
        .map(|t| t.id)
        .expect("the demo session has an instrument track");
    // The class id alone is enough: a session stores ids, and the catalogue may not be scanned yet.
    assert_eq!(
        console_kit::profile_for_plugin("vst3:565354507439717069616E6F74657120").map(|p| p.id),
        Some("pianoteq"),
        "Pianoteq 9's class id opens the Pianoteq console"
    );
    // Known names map to their consoles; an unknown plugin has none (and so gets no button).
    assert_eq!(console_kit::profile_for_plugin("Pianoteq 9").map(|p| p.id), Some("pianoteq"));
    assert_eq!(console_kit::profile_for_plugin("Kontakt 7").map(|p| p.id), Some("kontakt"));
    assert_eq!(console_kit::profile_for_plugin("Pigments").map(|p| p.id), Some("pigments"));
    assert_eq!(console_kit::profile_for_plugin("Omnisphere").map(|p| p.id), Some("omnisphere"));
    assert_eq!(console_kit::profile_for_plugin("Analog Lab V").map(|p| p.id), Some("analoglab"));
    assert_eq!(console_kit::profile_for_plugin("LABS").map(|p| p.id), Some("labs"));
    // KONTRA is a Kontakt-library sampler, so it drives the Kontakt console (and its libraries).
    assert_eq!(console_kit::profile_for_plugin("vst3:FC092AF5096DE321B960F4D245F8D72A").map(|p| p.id), Some("kontakt"));
    assert_eq!(console_kit::profile_for_plugin("clap:audio.matari.kontra").map(|p| p.id), Some("kontakt"));
    assert!(console_kit::profile_for_plugin("Pro-Q 3").is_none(), "an EQ has no instrument console");
    // Opening for a track selects the track (so the console's notes and CCs reach that VST).
    console_kit::open_for_track(&mut app, track);
    assert!(app.ui.show_console);
    assert_eq!(app.engine.session().edit.selected_tracks, vec![track]);
}

#[test]
fn the_mix_strip_shows_the_alt_button_for_a_hosted_console_instrument() {
    // "When I select the VST on the track, the ALT GUI button shows up there."
    let mut app = app();
    app.ui.mix_views.push("instruments".into());
    app.ui.mix_views.push("inserts_ae".into());
    let track = app
        .engine
        .session()
        .tracks
        .iter()
        .find(|t| t.kind == soundcraft_model::TrackKind::Instrument && t.instrument.is_some())
        .map(|t| t.id)
        .expect("an instrument track");
    // The demo's instrument stand-in gets the class id of a hosted VST that has a console.
    if let Some(t) = app.engine.session_mut().track_mut(track)
        && let Some(ins) = t.instrument.as_mut()
    {
        ins.plugin = "vst3:565354507439717069616E6F74657120".to_string();
    }
    app.engine.session_mut().edit.selected_tracks = vec![track];
    app.ui.window = soundcraft_ui_egui::MainWindow::Mix;
    let mut h = harness(app);
    h.run_steps(6);
    let button = h.query_by_label("Alt GUI");
    assert!(button.is_some(), "the channel strip's instrument row carries the ALT button");
    h.get_by_label("Alt GUI").click();
    h.run_steps(4);
    assert!(h.state().ui.show_console, "clicking it opens the alternative GUI");
    assert_eq!(h.state().ui.console_profile, "pianoteq", "on the console of that very VST");
    assert_eq!(h.state().engine.session().edit.selected_tracks, vec![track], "with the track selected");
}

#[test]
fn every_track_with_a_console_instrument_shows_alt_gui_in_its_header() {
    // "Onde está o ALT GUI em cada faixa?": in the Edit window's track header, on the track
    // itself, next to ACT — no Mix window and no menu needed.
    let mut app = app();
    // The first track of the session: the one the Edit window certainly draws.
    let track = app.engine.session().tracks.first().map(|t| t.id).expect("a track");
    if let Some(t) = app.engine.session_mut().track_mut(track) {
        t.kind = soundcraft_model::TrackKind::Instrument;
        t.instrument = Some(soundcraft_model::Insert::new("vst3:565354507439717069616E6F74657120"));
    }
    app.ui.window = soundcraft_ui_egui::MainWindow::Edit;
    let mut h = harness(app);
    h.run_steps(6);
    let alt = h.query_by_label("Alt GUI");
    if alt.is_none() {
        let rows = h.state().edit_layout.rows.clone();
        let st = h.state();
        let ins: Vec<(u64, Option<String>)> =
            st.engine.session().tracks.iter().map(|t| (t.id.0, t.instrument.as_ref().map(|i| i.plugin.clone()))).collect();
        panic!("no ALT GUI; track {track:?}, visible rows {rows:?}, instruments {ins:?}");
    }
    h.get_by_label("Alt GUI").click();
    h.run_steps(4);
    assert!(h.state().ui.show_console, "clicking it opens the console");
    assert_eq!(h.state().ui.console_profile, "pianoteq");
    assert_eq!(h.state().engine.session().edit.selected_tracks, vec![track]);
}

#[test]
fn the_this_track_command_opens_the_console_of_the_selected_track() {
    let mut app = app();
    let track = app
        .engine
        .session()
        .tracks
        .iter()
        .find(|t| t.kind == soundcraft_model::TrackKind::Instrument && t.instrument.is_some())
        .map(|t| t.id)
        .expect("an instrument track");
    if let Some(t) = app.engine.session_mut().track_mut(track)
        && let Some(ins) = t.instrument.as_mut()
    {
        ins.plugin = "vst3:41727475415649534B61743150726F63".to_string(); // Pigments
    }
    app.engine.session_mut().edit.selected_tracks = vec![track];
    let result = soundcraft_ui_egui::menus::run_ui_command(&mut app, "window.console.track", &serde_json::json!({}));
    assert!(result.is_some(), "the command exists");
    assert!(app.ui.show_console);
    assert_eq!(app.ui.console_profile, "pigments", "it follows the track's instrument");
    assert_eq!(soundcraft_ui_egui::menus::shortcut_for("window.console.track"), Some("Shift+Alt+P"));
}

#[test]
fn the_virtual_keyboard_never_leaves_a_note_droning() {
    // Two ways a hosted instrument ends up droning forever on this machine: the operating system
    // repeats a held key (each repeat used to be a new note-on, so the note retriggered), and a
    // release that never arrives (pointer left the window, dock closed) used to leave the note on.
    use egui::{Event, Key, Modifiers};
    let mut app = app();
    let track = app.engine.session().tracks.iter().find(|t| t.kind.is_midi() && t.instrument.is_some()).map(|t| t.id).expect("an instrument track");
    app.engine.session_mut().edit.selected_tracks = vec![track];
    app.ui.show_midi_keyboard = true;
    let mut h = harness(app);
    h.run_steps(3);

    let key = |pressed: bool, repeat: bool| Event::Key { key: Key::A, physical_key: None, pressed, repeat, modifiers: Modifiers::NONE };
    // A press, then the repeats the window system sends while the key stays down.
    h.input_mut().events.push(key(true, false));
    h.run_steps(1);
    for _ in 0..4 {
        h.input_mut().events.push(key(true, true));
        h.run_steps(1);
    }
    let notes =
        |h: &Harness<'static, SoundApp>, status: u8| -> usize { h.state().live_midi_log.iter().filter(|(_, m)| m[0] & 0xF0 == status).count() };
    assert_eq!(notes(&h, 0x90), 1, "one press starts one note, whatever the key repeats do");
    assert!(!h.state().keyboard.typed_notes.is_empty(), "the note is held");
    // Releasing stops it; nothing is left in the "typed" set, so no orphan is pending.
    h.input_mut().events.push(key(false, false));
    h.run_steps(2);
    assert!(h.state().keyboard.typed_notes.is_empty(), "the release clears the note");
    assert_eq!(notes(&h, 0x80), 1, "and sends exactly one note-off");
}

#[test]
fn a_lost_release_is_cleaned_up_by_the_watchdog() {
    // The note was started by the keyboard and its release never arrives: no pointer down, no key
    // down → the watchdog releases it instead of leaving the instrument sounding.
    let mut app = app();
    let track = app.engine.session().tracks.iter().find(|t| t.kind.is_midi() && t.instrument.is_some()).map(|t| t.id).expect("an instrument track");
    app.engine.session_mut().edit.selected_tracks = vec![track];
    app.keyboard.held_notes.insert(64);
    app.keyboard.typed_notes.insert(64);
    let mut h = harness(app);
    h.run_steps(3);
    assert!(h.state().keyboard.typed_notes.is_empty(), "the watchdog took the orphan");
    assert!(!h.state().keyboard.held_notes.contains(&64), "and cleared its light");
    assert!(h.state().live_midi_log.iter().any(|(_, m)| m[0] & 0xF0 == 0x80 && m[1] == 64), "the instrument is told to stop the note");
}

#[test]
fn console_kit_play_note_manages_held_and_typed_notes_without_stuck_notes() {
    let mut app = app();
    let track = app
        .engine
        .session()
        .tracks
        .iter()
        .find(|t| t.kind == soundcraft_model::TrackKind::Instrument && t.instrument.is_some())
        .map(|t| t.id)
        .expect("an instrument track");
    app.engine.session_mut().edit.selected_tracks = vec![track];
    let profile = soundcraft_ui_egui::console_kit::active(&app);

    // Turn note on
    soundcraft_ui_egui::console_kit::play_note(&mut app, profile, 60, 96, true);
    assert!(app.keyboard.held_notes.contains(&60), "note 60 is tracked in held_notes");
    assert!(app.keyboard.typed_notes.contains(&60), "note 60 is tracked in typed_notes");

    // Turn note off
    soundcraft_ui_egui::console_kit::play_note(&mut app, profile, 60, 0, false);
    assert!(!app.keyboard.held_notes.contains(&60), "note 60 is removed from held_notes");
    assert!(!app.keyboard.typed_notes.contains(&60), "note 60 is removed from typed_notes");
}
