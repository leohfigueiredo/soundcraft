//! Inserting a bridged (Windows) VST3 must not load its binary.
//!
//! Loading one means starting Wine, which takes seconds, and a plugin that hangs there used to
//! freeze the editor: the interface called `plugin_info`, which instantiates the plugin to read
//! its parameters. A command only needs the catalogue entry, so this test registers an entry whose
//! bundle does not exist at all — with the old lookup the insert failed, with the catalogue-based
//! one it succeeds.

use serde_json::json;
use soundcraft_engine::Engine;
use soundcraft_vst3_host::{Vst3Descriptor, adopt_scan};

const CLASS_ID: &str = "74657374436C6173732E696400000000";
const ID: &str = "vst3:74657374436C6173732E696400000000";

/// The catalogue is process-wide, so everything lives in one test (a second test would race it).
#[test]
fn inserting_a_catalogued_vst3_never_loads_its_bundle() {
    let descriptor = |path: &str, is_instrument: bool| Vst3Descriptor {
        id: ID.to_string(),
        class_id: CLASS_ID.to_string(),
        name: "Test Synth".to_string(),
        vendor: "A Vendor".to_string(),
        version: "1.0".to_string(),
        sdk_version: "VST 3.7.4".to_string(),
        sub_categories: if is_instrument { vec!["Instrument".to_string(), "Synth".to_string()] } else { vec!["Fx".to_string()] },
        category: if is_instrument { soundcraft_dsp::Category::Instrument } else { soundcraft_dsp::Category::Other },
        is_instrument,
        path: std::path::PathBuf::from(path),
    };

    // The bundle cannot exist: any code path that tried to load it would fail.
    adopt_scan(vec![descriptor("/nonexistent/Test Synth.vst3", true)]);
    let mut e = Engine::default();
    e.execute("track.new", &json!({"kind": "instrument"})).expect("new instrument track");
    let track = e.session().tracks[0].id.0;
    let r = e.execute("mix.instrument", &json!({"track": track, "plugin": ID})).expect("the catalogue is enough to set an instrument");
    assert_eq!(r["plugin"], ID);
    let ins = e.session().tracks[0].instrument.clone().expect("instrument set");
    assert!(ins.params.is_empty(), "a hosted plugin keeps its parameters in its own state");

    adopt_scan(vec![descriptor("/nonexistent/Test Fx.vst3", false)]);
    e.execute("track.new", &json!({})).expect("new audio track");
    let audio = e.session().tracks[1].id.0;
    let r = e.execute("mix.insert", &json!({"track": audio, "slot": 0, "plugin": ID})).expect("the catalogue is enough to insert");
    assert_eq!(r["plugin"], ID);
    let ins = e.session().tracks[1].mixer.inserts[0].clone().expect("insert set");
    assert!(ins.params.is_empty(), "a hosted plugin keeps its parameters in its own state");

    // The same plugin installed twice (native + bridged) keeps one id: the licensed (bridged) build
    // wins by default, so a session loads the one that plays. SOUNDCRAFT_PREFER_NATIVE flips it.
    let native = descriptor("/home/x/.vst3/Test Synth.vst3", true);
    let mut bridged = native.clone();
    bridged.path = std::path::PathBuf::from("/home/x/.vst3/yabridge/Test Synth.vst3");
    for order in [[native.clone(), bridged.clone()], [bridged.clone(), native.clone()]] {
        adopt_scan(order.to_vec());
        let kept = soundcraft_vst3_host::cached().and_then(|list| list.into_iter().find(|d| d.id == ID));
        assert!(
            kept.as_ref().is_some_and(|d| d.path.to_string_lossy().contains("yabridge")),
            "the bridged build is kept by default"
        );
    }
    adopt_scan(Vec::new());
}
