//! Regression tests for the hardware control surface (M-Vave SMC-Mixer / iCON P1-M), the
//! persisted preferences and the horizontal virtual keyboard.

use soundcraft_ui_egui::midi_io::{self, LinkState, Status};
use soundcraft_ui_egui::msc_mixer::{LearnTarget, MscConfig, MscManager, MscMode, MscPrefs};
use soundcraft_ui_egui::theme::{ThemeMode, Tokens};
use soundcraft_ui_egui::virtual_keyboard::{KeyboardPrefs, calculate_range, is_black_key, pitch_name};
use soundcraft_ui_egui::{Services, SoundApp};

/// A connected link snapshot, as the MIDI worker would publish it.
fn connected_link(port: &str) -> Status {
    Status {
        state: LinkState::Connected,
        keyboard_port: String::new(),
        keyboard_state: LinkState::Searching,
        ble_state: LinkState::Unavailable,
        ble_device: String::new(),
        backend: "ALSA sequencer".to_string(),
        port_name: port.to_string(),
        filter: "SMC-Mixer".to_string(),
        message: String::new(),
        ports: Vec::new(),
        received: 0,
        sent: 0,
        reconnects: 0,
    }
}

fn app_with_demo() -> SoundApp {
    SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default())
}

fn first_track(app: &SoundApp) -> soundcraft_model::TrackId {
    app.engine.session().tracks.iter().find(|t| !t.hidden).expect("the demo session has tracks").id
}

#[test]
fn virtual_keyboard_ranges_pitch_names_and_black_keys() {
    // 49 keys at octave 0: C2 (36) to C6 (84).
    let (start, end) = calculate_range(49, 0);
    assert_eq!((start, end), (36, 84));
    assert_eq!(pitch_name(start), "C2");
    assert_eq!(pitch_name(60), "C4");
    assert!(!is_black_key(60));
    assert!(is_black_key(61));

    // 25 keys: two octaves starting at C2.
    assert_eq!(calculate_range(25, 0), (36, 60));
    // 88 keys: A0 (21) to C8 (108), the standard concert range.
    let (start88, end88) = calculate_range(88, 0);
    assert_eq!((start88, end88), (21, 108));
    assert_eq!(pitch_name(start88), "A0");
    assert_eq!(pitch_name(end88), "C8");
    // Every configured range stays inside MIDI and never inverts.
    for keys in [25u8, 37, 49, 61, 88] {
        for octave in -3..=3 {
            let (a, b) = calculate_range(keys, octave);
            assert!(b <= 127 && b >= a, "range {keys}/{octave} = {a}..{b}");
        }
    }
}

#[test]
fn keyboard_preferences_are_validated_on_load() {
    let mut prefs = KeyboardPrefs { num_keys: 61, base_octave: 0, velocity: 100, step_rec: false };
    let mut state = soundcraft_ui_egui::virtual_keyboard::KeyboardState::new();
    // A hand-edited config cannot set an unsupported range, an absurd octave or a zero velocity.
    prefs.num_keys = 77;
    prefs.base_octave = 42;
    prefs.velocity = 0;
    state.apply_prefs(&prefs);
    assert_eq!(state.num_keys, 61);
    assert_eq!(state.base_octave, 3);
    assert_eq!(state.velocity, 1);
    // A valid one round-trips.
    prefs.num_keys = 88;
    prefs.base_octave = -2;
    prefs.velocity = 64;
    state.apply_prefs(&prefs);
    assert_eq!(state.prefs(), prefs);
}

#[test]
fn theme_modes_parse_and_carry_distinct_tokens() {
    for (id, mode) in [
        ("dark", ThemeMode::Dark),
        ("light", ThemeMode::Light),
        ("studiopro", ThemeMode::StudioPro),
        ("flstudio", ThemeMode::FLStudio),
        ("cyberpunk", ThemeMode::Cyberpunk),
        ("vintage", ThemeMode::Vintage),
        ("system", ThemeMode::System),
    ] {
        assert_eq!(ThemeMode::parse(id), Some(mode));
        assert_eq!(ThemeMode::parse(mode.id()), Some(mode));
    }
    assert_eq!(ThemeMode::parse("neon"), Some(ThemeMode::Cyberpunk));
    assert_eq!(ThemeMode::parse("console"), Some(ThemeMode::Vintage));
    assert_eq!(ThemeMode::parse("sepia"), None);

    // The required themes exist and are visually distinct (not aliases of each other).
    let dark = Tokens::DARK;
    let light = Tokens::LIGHT;
    let cyber = Tokens::CYBERPUNK;
    let vintage = Tokens::VINTAGE;
    assert_ne!(dark.window_bg, light.window_bg);
    assert_ne!(cyber.window_bg, dark.window_bg);
    assert_ne!(vintage.window_bg, dark.window_bg);
    assert_eq!(cyber.meter_style, soundcraft_ui_egui::theme::MeterStyle::CyberNeon);
    assert_eq!(vintage.fader_cap_style, soundcraft_ui_egui::theme::FaderCapStyle::VintageBakelite);
    // The X-Touch style exists for the always-visible luminous volume knobs.
    assert_eq!(soundcraft_ui_egui::theme::KnobStyle::XTouchLed as u8, soundcraft_ui_egui::theme::KnobStyle::XTouchLed as u8);
}

#[test]
fn msc_fader_buttons_transport_and_feedback() {
    let mut app = app_with_demo();
    let mut msc = MscManager::default();
    msc.apply_link_status(connected_link("M-Vave SMC-Mixer MIDI 1"));
    assert!(msc.connected);

    // Move fader 1 (channel 0 pitch bend) onto the 0 dB detent: it engages (soft takeover),
    // because the demo track sits at 0 dB.
    assert!(msc.handle_message(&mut app, &[0xE0, 0x00, 0x60]).is_some());

    // Pan encoder (CC 16, channel 0), then a V-Pot push to recentre.
    assert!(msc.handle_message(&mut app, &[0xB0, 16, 127]).is_some());
    assert!(msc.handle_message(&mut app, &[0x90, 32, 127]).is_some());
    assert_eq!(app.engine.session().tracks.first().and_then(|t| t.mixer.pan.first().copied()), Some(0.0));

    // Mute toggle by note.
    assert!(msc.handle_message(&mut app, &[0x90, 16, 127]).is_some());
    assert!(app.engine.session().tracks.first().is_some_and(|t| t.mixer.mute));

    // Fader touch / release (MCU notes 104..112).
    assert!(msc.handle_message(&mut app, &[0x90, 104, 127]).is_some());
    assert!(msc.fader_touched[0]);
    assert!(msc.handle_message(&mut app, &[0x80, 104, 0]).is_some());
    assert!(!msc.fader_touched[0]);

    // Transport: play, stop, loop.
    assert_eq!(msc.handle_message(&mut app, &[0x90, 94, 127]), Some("Transport: Play".to_string()));
    assert!(app.engine.transport_requests.contains(&soundcraft_engine::TransportRequest::Play));
    assert_eq!(msc.handle_message(&mut app, &[0x90, 93, 127]), Some("Transport: Stop".to_string()));
    assert!(msc.handle_message(&mut app, &[0x90, 86, 127]).is_some());

    // Diagnostics queue LED and motor packets, and the queue stays bounded.
    msc.trigger_diagnostic_test();
    assert!(!msc.drain_output().is_empty());
    for _ in 0..10_000 {
        msc.trigger_diagnostic_test();
    }
    assert!(msc.output_queue.len() <= soundcraft_ui_egui::msc_mixer::MAX_FEEDBACK_QUEUE, "the feedback queue is bounded");
    let _ = msc.drain_output();

    // Feedback: mute LED and the 12-segment MCU meter for channel 0.
    let tid = first_track(&app);
    app.meters.insert(
        tid,
        soundcraft_ui_egui::MeterDisplay { level: [0.8, 0.75], hold: [0.8, 0.75], hold_age: 0.0, clip: false, gr: 0.0, plugin_failures: 0 },
    );
    msc.sync_feedback(app.engine.session(), app.is_playing(), false, &app.meters);
    let out = msc.drain_output();
    assert!(out.iter().any(|p| p[0] == 0x90 && p[1] == 16 && p[2] == 127), "mute LED on");
    assert!(out.iter().any(|p| (p[0] & 0xF0) == 0xD0 && (p[1] & 0x0F) > 0), "12-segment meter message");
}

#[test]
fn soft_takeover_blocks_a_jump_and_engages_at_the_virtual_value() {
    let mut app = app_with_demo();
    let mut msc = MscManager::default();
    // Pickup is opt-in; this test exercises it.
    msc.config.soft_takeover = true;
    let tid = first_track(&app);
    app.engine.execute("mix.volume", &serde_json::json!({"track": tid.0, "db": -20.0})).expect("set volume");
    let virt = msc.norm_from_db(-20.0);
    assert!((virt - 0.5).abs() < 1e-3, "0 dB is at 0.75 of the travel, so -20 dB is midway: {virt}");

    // A far-away fader position must not jump the track.
    let far = (0.9f32 * 16383.0) as u16;
    let res = msc.handle_message(&mut app, &[0xE0, (far & 0x7F) as u8, ((far >> 7) & 0x7F) as u8]).expect("a message");
    assert!(res.contains("pickup"), "{res}");
    let db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!((db + 20.0).abs() < 0.01, "volume must not move before pickup, got {db}");

    // Reaching the virtual value engages the fader, keeping the volume.
    let at = (virt * 16383.0) as u16;
    msc.handle_message(&mut app, &[0xE0, (at & 0x7F) as u8, ((at >> 7) & 0x7F) as u8]);
    let db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!((db + 20.0).abs() < 1.0, "engaging at the virtual value keeps the volume, got {db}");

    // Once engaged it follows continuously, without another pickup.
    let lower = (0.25f32 * 16383.0) as u16;
    msc.handle_message(&mut app, &[0xE0, (lower & 0x7F) as u8, ((lower >> 7) & 0x7F) as u8]);
    let db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!((db + 40.0).abs() < 1.5, "an engaged fader follows, got {db}");

    // A bank change disarms every fader again.
    msc.handle_message(&mut app, &[0x90, 47, 127]);
    assert!(msc.pickup.iter().all(Option::is_none) && msc.engaged.iter().all(|e| !e));

    // Back on bank one, with soft takeover off, a fader is absolute again.
    msc.handle_message(&mut app, &[0x90, 46, 127]);
    assert_eq!(msc.active_bank, 0);
    msc.config.soft_takeover = false;
    msc.handle_message(&mut app, &[0xE0, 0x00, 0x60]);
    let db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!(db.abs() < 0.5, "absolute mode follows the fader at once, got {db}");
}

#[test]
fn midi_learn_binds_a_control_and_mode_switches_the_global_mapping() {
    let mut app = app_with_demo();
    let mut msc = MscManager::default();
    let tid = first_track(&app);

    // Learn: the next message becomes the link.
    msc.learn_start(LearnTarget::Volume { track: tid.0 });
    let res = msc.handle_message(&mut app, &[0xB0, 71, 10]).expect("a message");
    assert!(res.contains("MIDI Learn"), "{res}");
    assert_eq!(msc.bindings.len(), 1);
    assert!(msc.learn.is_none(), "learning is one-shot");

    // The learned control now drives the track, whatever the bank says.
    app.engine.execute("mix.volume", &serde_json::json!({"track": tid.0, "db": 0.0})).expect("set volume");
    msc.handle_message(&mut app, &[0xB0, 71, 0]);
    let db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!(db < -50.0, "CC 71 at 0 must pull the volume down, got {db}");

    // Linked mode turns the global bank mapping off; the learned control still works.
    msc.mode = MscMode::Linked;
    msc.reset_pickups();
    let before = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    msc.handle_message(&mut app, &[0xE0, 0x00, 0x60]);
    let after = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!((before - after).abs() < 1e-3, "unlearned faders are idle in linked mode");
    msc.handle_message(&mut app, &[0xB0, 71, 64]);
    let after = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
    assert!(after > -30.0, "the learned control still works in linked mode, got {after}");

    // Unlearn clears the link.
    msc.learn_clear(None);
    assert!(msc.bindings.is_empty());
}

#[test]
fn msc_preferences_round_trip_through_json() {
    let mut prefs = MscPrefs {
        version: 2,
        config: MscConfig::mvave_smc(),
        keyboard_port: None,
        ble: false,
        surface_port: None,
        mode: MscMode::Linked,
        bindings: Vec::new(),
        filter: "SMC-Mixer".to_string(),
    };
    prefs.bindings.push(soundcraft_ui_egui::msc_mixer::MidiBinding {
        status: 0xB0,
        channel: 0,
        data1: 74,
        target: LearnTarget::InsertParam { track: 7, slot: 1, param: "cutoff".to_string(), min: 0.0, max: 1.0 },
    });
    let json = serde_json::to_string(&prefs).expect("serialise");
    let back: MscPrefs = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back.mode, MscMode::Linked);
    assert_eq!(back.config.device_name, "M-Vave SMC-Mixer");
    assert_eq!(back.bindings, prefs.bindings);
    // Soft takeover is opt-in now, but an explicitly enabled value round-trips.
    assert!(!back.config.soft_takeover);
    prefs.config.soft_takeover = true;
    let json = serde_json::to_string(&prefs).expect("serialise");
    let back: MscPrefs = serde_json::from_str(&json).expect("deserialise");
    assert!(back.config.soft_takeover);

    // A manager copies them in and out unchanged.
    let mut msc = MscManager::default();
    msc.apply_prefs(&back);
    assert_eq!(msc.prefs().bindings, prefs.bindings);
    assert_eq!(msc.mode, MscMode::Linked);
}

#[test]
fn controller_commands_drive_link_mode_and_learn() {
    let mut app = app_with_demo();
    let tid = first_track(&app).0;

    let status = app.run("controller.status", serde_json::json!({})).expect("status");
    assert!(status.get("link").is_some() && status.get("mode").is_some());

    app.run("controller.mode", serde_json::json!({"mode": "linked"})).expect("mode");
    assert_eq!(app.msc.mode, MscMode::Linked);
    app.run("controller.mode", serde_json::json!({})).expect("toggle");
    assert_eq!(app.msc.mode, MscMode::Global);

    app.run("controller.soft_takeover", serde_json::json!({"value": false})).expect("takeover");
    assert!(!app.msc.config.soft_takeover);

    app.run("controller.learn", serde_json::json!({"target": "pan", "track": tid})).expect("learn");
    assert!(matches!(app.msc.learn, Some(LearnTarget::Pan { .. })));
    let err = app.run("controller.learn", serde_json::json!({"target": "bogus"})).expect_err("unknown target");
    assert!(err.contains("unknown target"), "{err}");
    app.run("controller.cancel_learn", serde_json::json!({})).expect("cancel");
    assert!(app.msc.learn.is_none());
}

#[test]
fn midi_io_without_hardware_reports_a_state() {
    // The worker must degrade quietly on a machine with no sequencer (CI, containers).
    let io = midi_io::MidiIo::start("SMC-Mixer");
    io.send([0x90, 60, 100]);
    assert!(io.recv().is_none());
    let status = io.status();
    assert!(matches!(status.state, LinkState::Unavailable | LinkState::Searching), "{status:?}");
    assert!(!status.backend.is_empty());
    io.rescan();
    io.set_filter("iCON");
}
