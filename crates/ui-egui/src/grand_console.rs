//! Grand Piano: the Craft Console profile for the built-in `grand_piano` instrument.
//!
//! The parameter map is the contract with `crates/dsp/src/plugins/piano.rs`: every `id` here is a
//! parameter id of that instrument, and the console writes it straight through
//! (`mix.instrument_param`), so a rename on either side is a bug this file makes obvious.

use crate::console_kit::{Category, Icon, Model, Palette, Profile, Spec};

/// Graphite and warm amber, the piano's own theme.
pub const PALETTE: Palette = Palette::MAHOGANY;

pub const CATEGORIES: &[Category] = &[Category { title: "GRANDS" }, Category { title: "UPRIGHTS" }, Category { title: "HYBRIDS" }];

/// The sounds this console offers. A built-in instrument has one engine, so these are voicings of
/// it rather than factory patches: each one is the same model with its own settings.
pub const MODELS: &[Model] = &[
    Model { category: 0, name: "Concert Grand", program: 0, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Studio Grand", program: 1, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Intimate Grand", program: 2, icon: Icon::Grand, preset: None },
    Model { category: 1, name: "Upright Felt", program: 3, icon: Icon::Upright, preset: None },
    Model { category: 1, name: "Upright Bright", program: 4, icon: Icon::Upright, preset: None },
    Model { category: 2, name: "Honky Tonk", program: 5, icon: Icon::Historical, preset: None },
    Model { category: 2, name: "Felt Grand", program: 6, icon: Icon::Grand, preset: None },
];

pub const RACKS: &[&str] = &["HAMMER & STRING", "DAMPER & PEDAL", "SPACE & TUNING"];

pub const SPECS: &[Spec] = &[
    // HAMMER & STRING
    Spec {
        id: "brightness",
        label: "BRIGHTNESS",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 74,
        automation: "Brightness",
        decimals: 0,
    },
    Spec {
        id: "hammer_hardness",
        label: "HAMMER",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 73,
        automation: "Hammer Hardness",
        decimals: 0,
    },
    Spec {
        id: "inharmonicity",
        label: "TENSION",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 45.0,
        unit: "%",
        cc: 72,
        automation: "String Tension",
        decimals: 0,
    },
    Spec {
        id: "dynamics",
        label: "DYNAMICS",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 60.0,
        unit: "%",
        cc: 11,
        automation: "Dynamic Range",
        decimals: 0,
    },
    Spec {
        id: "unison_detune",
        label: "UNISON",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 25.0,
        unit: "%",
        cc: 22,
        automation: "Unison Detune",
        decimals: 0,
    },
    // DAMPER & PEDAL
    Spec {
        id: "damper",
        label: "DAMPER",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 60.0,
        unit: "%",
        cc: 64,
        automation: "Damper",
        decimals: 0,
    },
    Spec {
        id: "release",
        label: "RELEASE",
        group: 1,
        min: 5.0,
        max: 800.0,
        step: 0.0,
        default: 220.0,
        unit: "ms",
        cc: 71,
        automation: "Release",
        decimals: 0,
    },
    Spec {
        id: "resonance",
        label: "SYMPATHETIC",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 93,
        automation: "Sympathetic",
        decimals: 0,
    },
    Spec {
        id: "pedal",
        label: "PEDAL",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "%",
        cc: 66,
        automation: "Sustain Pedal",
        decimals: 0,
    },
    // SPACE & TUNING
    Spec {
        id: "ambience",
        label: "SOUNDBOARD",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 35.0,
        unit: "%",
        cc: 91,
        automation: "Soundboard",
        decimals: 0,
    },
    Spec {
        id: "stereo_width",
        label: "WIDTH",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 70.0,
        unit: "%",
        cc: 10,
        automation: "Stereo Width",
        decimals: 0,
    },
    Spec {
        id: "octave_stretch",
        label: "STRETCH",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 30.0,
        unit: "%",
        cc: 92,
        automation: "Octave Stretch",
        decimals: 0,
    },
    Spec {
        id: "tune",
        label: "TUNE",
        group: 2,
        min: -100.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "ct",
        cc: 20,
        automation: "Tune",
        decimals: 0,
    },
];

pub const MASTER: &[Spec] = &[Spec {
    id: "level",
    label: "LEVEL",
    group: NO_RACK_LEVEL,
    min: -48.0,
    max: 6.0,
    step: 0.0,
    default: -6.0,
    unit: "dB",
    cc: 7,
    automation: "Level",
    decimals: 1,
}];

/// The master row draws specs whose `group` is this value (see [`crate::console_kit::NO_RACK`]).
const NO_RACK_LEVEL: u8 = crate::console_kit::NO_RACK;

pub static PROFILE: Profile = Profile {
    id: "grand_piano",
    kind: "modelled piano",
    brand: "SOUNDCRAFT",
    name: "GRAND PIANO",
    tagline: "struck strings, modelled — no samples",
    palette: PALETTE,
    icon: Icon::Grand,
    // The built-in instrument's id, which is what a session stores for it: this is what makes the
    // ALT button appear on a track that plays it.
    plugin_ids: &["grand_piano"],
    categories: CATEGORIES,
    models: MODELS,
    variants: &["Concert", "Studio", "Intimate", "Felt"],
    master: MASTER,
    racks: RACKS,
    specs: SPECS,
    selects: &[],
    library_picker: None,
    velocity_curve: true,
    // A built-in instrument follows `mix.instrument_param` directly, so no program change and no
    // captured-state indirection is needed.
    program_change: false,
    capture_first: false,
};
