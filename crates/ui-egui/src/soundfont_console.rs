//! SoundFont / SF2: the Craft Console profile for multi-sample SoundFont instruments.
//!
//! Provides one-click access to the soundfont library on this machine (grand pianos, uprights,
//! strings, brass, guitars, pads), with real-time controls for envelopes, filter cutoff, tuning,
//! master level, velocity curve and virtual keyboard.

use crate::console_kit::{Category, Icon, Model, Palette, Profile, Spec};

use egui::Color32;

pub const PALETTE: Palette = Palette {
    bg: Color32::from_rgb(0x13, 0x18, 0x15),
    panel: Color32::from_rgb(0x1a, 0x22, 0x1d),
    panel_head: Color32::from_rgb(0x22, 0x2c, 0x26),
    line: Color32::from_rgb(0x2f, 0x3d, 0x35),
    edge: Color32::from_rgb(0x46, 0x5a, 0x4e),
    accent: Color32::from_rgb(0x42, 0xd6, 0x97),
    accent_dim: Color32::from_rgb(0x2b, 0x8a, 0x62),
    text: Color32::from_rgb(0xeb, 0xf5, 0xef),
    text_dim: Color32::from_rgb(0x89, 0xa1, 0x93),
    live: Color32::from_rgb(0x70, 0xef, 0xde),
};

pub const CATEGORIES: &[Category] = &[
    Category { title: "PIANOS & KEYS" },
    Category { title: "STRINGS & ORCH" },
    Category { title: "GUITARS & BASS" },
    Category { title: "SYNTHS & PADS" },
];

pub const MODELS: &[Model] = &[
    // PIANOS & KEYS
    Model {
        category: 0,
        name: "Acoustic Piano",
        program: 0,
        icon: Icon::Grand,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Acoustic Piano.sf2"),
    },
    Model { category: 0, name: "Grand Upright N", program: 1, icon: Icon::Upright, preset: Some("/home/leonardo/Downloads/Grand_Upright__N.sf2") },
    Model { category: 0, name: "Steinway D-274", program: 2, icon: Icon::Grand, preset: Some("/home/leonardo/Downloads/D-274.sf2") },
    Model {
        category: 0,
        name: "AJH Piano",
        program: 3,
        icon: Icon::Grand,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/AJH_Piano.sf2"),
    },
    Model {
        category: 0,
        name: "Clavinet",
        program: 4,
        icon: Icon::Keys,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Clavinet.sf2"),
    },
    Model {
        category: 0,
        name: "CP-70 Piano",
        program: 5,
        icon: Icon::Electric,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/CP-70.sf2"),
    },
    Model {
        category: 0,
        name: "Dyno Rhodes",
        program: 6,
        icon: Icon::Electric,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/DynoRhodes.sf2"),
    },
    Model {
        category: 0,
        name: "Cinema Organ",
        program: 7,
        icon: Icon::Keys,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/cinema_organ_c102.sf2"),
    },
    // STRINGS & ORCH
    Model {
        category: 1,
        name: "Symphony Hall",
        program: 8,
        icon: Icon::Strings,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Strings_Orch/(130 sounds) 27mg_Symphony_Hall_Bank.SF2"),
    },
    Model {
        category: 1,
        name: "Cadenza Strings",
        program: 9,
        icon: Icon::Strings,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Strings_Orch/Cadenza Strings.SF2"),
    },
    Model {
        category: 1,
        name: "Big Strings",
        program: 10,
        icon: Icon::Strings,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Strings_Orch/BigStringz_YC.sf2"),
    },
    Model {
        category: 1,
        name: "Boston Brass",
        program: 11,
        icon: Icon::Brass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Brass_Horns/Boston Brass.sf2"),
    },
    Model {
        category: 1,
        name: "Trumpet Solo",
        program: 12,
        icon: Icon::Brass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Brass_Horns/Trumpet Solo Danny Fye (1,294KB).sf2"),
    },
    Model {
        category: 1,
        name: "Trombone",
        program: 13,
        icon: Icon::Brass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Brass_Horns/Trombone.sf2"),
    },
    Model {
        category: 1,
        name: "Alto Sax",
        program: 14,
        icon: Icon::Brass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Brass_Horns/Alto Sax.sf2"),
    },
    // GUITARS & BASS
    Model {
        category: 2,
        name: "Acoustic Guitar",
        program: 15,
        icon: Icon::Keys,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Guitars_Basses/Acoustic Guitar.sf2"),
    },
    Model {
        category: 2,
        name: "Clean Strat",
        program: 16,
        icon: Icon::Keys,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Guitars_Basses/CleanStrat.sf2"),
    },
    Model {
        category: 2,
        name: "Acoustic Bass",
        program: 17,
        icon: Icon::Bass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Guitars_Basses/ACOUSTC-BASS.sf2"),
    },
    Model {
        category: 2,
        name: "Bass Guitars",
        program: 18,
        icon: Icon::Bass,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Guitars_Basses/Bass Guitars.sf2"),
    },
    // SYNTHS & PADS
    Model {
        category: 3,
        name: "Deep Pad",
        program: 19,
        icon: Icon::Pad,
        preset: Some("/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Deep Pad.sf2"),
    },
    Model { category: 3, name: "Ethernal Reverse", program: 20, icon: Icon::Pad, preset: Some("/home/leonardo/Downloads/JF ETHERNAL REVERSE.sf2") },
    Model { category: 3, name: "Reverse", program: 21, icon: Icon::Pad, preset: Some("/home/leonardo/Downloads/Reverse .sf2") },
];

pub const RACKS: &[&str] = &["ENVELOPE & TONE", "TUNING & PLAYBACK"];

pub const SPECS: &[Spec] = &[
    // ENVELOPE & TONE
    Spec {
        id: "attack",
        label: "ATTACK",
        group: 0,
        min: 0.1,
        max: 5000.0,
        step: 0.0,
        default: 100.0,
        unit: "%",
        cc: 73,
        automation: "Attack",
        decimals: 0,
    },
    Spec {
        id: "release",
        label: "RELEASE",
        group: 0,
        min: 1.0,
        max: 10000.0,
        step: 0.0,
        default: 100.0,
        unit: "%",
        cc: 72,
        automation: "Release",
        decimals: 0,
    },
    Spec {
        id: "cutoff",
        label: "CUTOFF",
        group: 0,
        min: 20.0,
        max: 20000.0,
        step: 0.0,
        default: 20000.0,
        unit: "Hz",
        cc: 74,
        automation: "Cutoff",
        decimals: 0,
    },
    // TUNING & PLAYBACK
    Spec {
        id: "transpose",
        label: "TRANSPOSE",
        group: 1,
        min: -36.0,
        max: 36.0,
        step: 1.0,
        default: 0.0,
        unit: "st",
        cc: 21,
        automation: "Transpose",
        decimals: 0,
    },
    Spec {
        id: "tune",
        label: "TUNE",
        group: 1,
        min: -100.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "ct",
        cc: 20,
        automation: "Fine Tune",
        decimals: 0,
    },
    Spec {
        id: "preset",
        label: "PRESET",
        group: 1,
        min: 0.0,
        max: 127.0,
        step: 1.0,
        default: 0.0,
        unit: "",
        cc: 0,
        automation: "Preset",
        decimals: 0,
    },
];

pub const MASTER: &[Spec] = &[Spec {
    id: "level",
    label: "LEVEL",
    group: NO_RACK_LEVEL,
    min: -48.0,
    max: 12.0,
    step: 0.0,
    default: 0.0,
    unit: "dB",
    cc: 7,
    automation: "Level",
    decimals: 1,
}];

const NO_RACK_LEVEL: u8 = crate::console_kit::NO_RACK;

pub static PROFILE: Profile = Profile {
    id: "soundfont",
    kind: "soundfont player",
    brand: "SOUNDCRAFT",
    name: "SOUNDFONT / SF2",
    tagline: "multi-sample instruments, one click each",
    palette: PALETTE,
    icon: Icon::Sampler,
    plugin_ids: &["soundfont", "sampler"],
    categories: CATEGORIES,
    models: MODELS,
    variants: &["Default", "Bright", "Warm", "Dark"],
    master: MASTER,
    racks: RACKS,
    specs: SPECS,
    selects: &[],
    library_picker: None,
    velocity_curve: true,
    program_change: false,
    capture_first: false,
};
