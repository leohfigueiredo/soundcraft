//! Pianoteq: the alternative console profile (see [`crate::console_kit`] for the machinery).
//!
//! The parameter map below is the contract: id, label, rack, range, unit, MIDI CC and the Pianoteq
//! parameter name. Nothing in the painting code decides a range, and every control dispatches
//! through the shared hook, so a bridge (WebMIDI, WebSocket, OSC, JUCE) only replaces that body.

use crate::console_kit::{Category, Icon, Model, Palette, Profile, Select, Spec};
#[allow(unused_imports)]
use egui::Color32;
/// This instrument's console theme (graphite and amber).
pub const PALETTE: Palette = Palette::GRAPHITE;

/// The category tabs in the header.
pub const CATEGORIES: &[Category] = &[
    Category { title: "ACOUSTIC GRANDS" },
    Category { title: "UPRIGHTS" },
    Category { title: "ELECTRIC & TINES" },
    Category { title: "MALLETS & CHROMATIC" },
    Category { title: "HISTORICAL" },
    Category { title: "BELLS & CARILLON" },
    Category { title: "LEGACY & RECORDING" },
];

/// Recording/perspective variants (the "preset variant" dropdown).
pub const VARIANTS: &[&str] = &["Classical Recording", "Player Perspective", "Jazz Studio", "Warm", "Bright", "Concert Hall", "Under Lid"];

/// Temperaments, as the selector in the header.
pub const TEMPERAMENTS: &[&str] = &["Equal", "Just", "Werckmeister III", "Meantone"];

/// Reverb algorithms.
pub const REVERB_TYPES: &[&str] = &["Algorithmic Room", "Studio", "Concert Hall", "Cathedral", "Plate"];

/// The racks, in drawing order (a parameter picks one by index).
pub const RACKS: &[&str] = &["TUNING & STRING", "VOICING / HAMMER", "RESONANCE & BODY", "EFFECTS"];

/// Every parameter the console exposes. CC numbers avoid the reserved channel-mode range and the
/// usual expression/pedal assignments.
pub const SPECS: &[Spec] = &[
    // Master
    // Tuning & string
    Spec {
        id: "unison_width",
        label: "UNISON WIDTH",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 30.0,
        unit: "%",
        cc: 22,
        automation: "Unison Width",
        decimals: 0,
    },
    Spec {
        id: "octave_stretching",
        label: "OCTAVE STRETCH",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 23,
        automation: "Octave Stretching",
        decimals: 0,
    },
    Spec {
        id: "direct_duration",
        label: "DIRECT DURATION",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 24,
        automation: "Direct String Duration",
        decimals: 0,
    },
    Spec {
        id: "string_length",
        label: "STRING LENGTH",
        group: 0,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 61,
        automation: "String Length",
        decimals: 0,
    },
    // Voicing / hammer
    Spec {
        id: "hammer_piano",
        label: "HAMMER P",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 25,
        automation: "Hammer Hardness Piano",
        decimals: 0,
    },
    Spec {
        id: "hammer_mezzo",
        label: "HAMMER M",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 26,
        automation: "Hammer Hardness Mezzo",
        decimals: 0,
    },
    Spec {
        id: "hammer_forte",
        label: "HAMMER F",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 27,
        automation: "Hammer Hardness Forte",
        decimals: 0,
    },
    Spec {
        id: "strike_point",
        label: "STRIKE POINT",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 25.0,
        unit: "%",
        cc: 28,
        automation: "Strike Point",
        decimals: 0,
    },
    Spec {
        id: "una_corda",
        label: "UNA CORDA",
        group: 1,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 60.0,
        unit: "%",
        cc: 29,
        automation: "Soft Pedal",
        decimals: 0,
    },
    // Resonance & body
    Spec {
        id: "sympathetic",
        label: "SYMPATHETIC",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 45.0,
        unit: "%",
        cc: 30,
        automation: "Sympathetic Resonance",
        decimals: 0,
    },
    Spec {
        id: "sympathetic_decay",
        label: "RES DECAY",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 31,
        automation: "Sympathetic Resonance Decay",
        decimals: 0,
    },
    Spec {
        id: "duplex_scale",
        label: "DUPLEX SCALE",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 35.0,
        unit: "%",
        cc: 32,
        automation: "Duplex Scale Resonance",
        decimals: 0,
    },
    Spec {
        id: "soundboard_impedance",
        label: "SOUNDBOARD",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 33,
        automation: "Soundboard Impedance",
        decimals: 0,
    },
    Spec {
        id: "soundboard_cutoff",
        label: "SB CUTOFF",
        group: 2,
        min: 20.0,
        max: 20000.0,
        step: 0.0,
        default: 8000.0,
        unit: "Hz",
        cc: 34,
        automation: "Soundboard Cutoff",
        decimals: 0,
    },
    Spec {
        id: "damper_position",
        label: "DAMPER POS",
        group: 2,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 40.0,
        unit: "%",
        cc: 35,
        automation: "Damper Position",
        decimals: 0,
    },
    Spec {
        id: "damper_noise",
        label: "DAMPER NOISE",
        group: 2,
        min: -60.0,
        max: 0.0,
        step: 0.0,
        default: -24.0,
        unit: "dB",
        cc: 36,
        automation: "Damper Noise",
        decimals: 1,
    },
    Spec {
        id: "key_release_noise",
        label: "KEY RELEASE",
        group: 2,
        min: -60.0,
        max: 0.0,
        step: 0.0,
        default: -24.0,
        unit: "dB",
        cc: 37,
        automation: "Key Release Noise",
        decimals: 1,
    },
    Spec {
        id: "pedal_noise",
        label: "PEDAL NOISE",
        group: 2,
        min: -60.0,
        max: 0.0,
        step: 0.0,
        default: -30.0,
        unit: "dB",
        cc: 38,
        automation: "Pedal Noise",
        decimals: 1,
    },
    // Dynamics
    Spec {
        id: "dynamic_range",
        label: "DYNAMIC RANGE",
        group: 255,
        min: 0.0,
        max: 60.0,
        step: 0.0,
        default: 30.0,
        unit: "dB",
        cc: 39,
        automation: "Dynamic Range",
        decimals: 1,
    },
    // Reverb
    Spec {
        id: "reverb_decay",
        label: "DECAY",
        group: 3,
        min: 0.2,
        max: 20.0,
        step: 0.0,
        default: 2.5,
        unit: "s",
        cc: 41,
        automation: "Reverb Decay",
        decimals: 2,
    },
    Spec {
        id: "reverb_predelay",
        label: "PRE-DELAY",
        group: 3,
        min: 0.0,
        max: 200.0,
        step: 0.0,
        default: 20.0,
        unit: "ms",
        cc: 42,
        automation: "Reverb Pre-delay",
        decimals: 0,
    },
    Spec {
        id: "reverb_room",
        label: "ROOM SIZE",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 50.0,
        unit: "%",
        cc: 43,
        automation: "Room Size",
        decimals: 0,
    },
    Spec {
        id: "reverb_damping",
        label: "DAMPING",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 40.0,
        unit: "%",
        cc: 44,
        automation: "Damping",
        decimals: 0,
    },
    Spec {
        id: "reverb_mix",
        label: "WET / DRY",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 25.0,
        unit: "%",
        cc: 45,
        automation: "Reverb Mix",
        decimals: 0,
    },
    // EQ
    Spec {
        id: "eq_low_gain",
        label: "LOW GAIN",
        group: 3,
        min: -18.0,
        max: 18.0,
        step: 0.0,
        default: 0.0,
        unit: "dB",
        cc: 46,
        automation: "Low Shelf Gain",
        decimals: 1,
    },
    Spec {
        id: "eq_low_freq",
        label: "LOW FREQ",
        group: 3,
        min: 20.0,
        max: 1000.0,
        step: 0.0,
        default: 120.0,
        unit: "Hz",
        cc: 47,
        automation: "Low Shelf Freq",
        decimals: 0,
    },
    Spec {
        id: "eq_mid_gain",
        label: "MID GAIN",
        group: 3,
        min: -18.0,
        max: 18.0,
        step: 0.0,
        default: 0.0,
        unit: "dB",
        cc: 48,
        automation: "Mid Gain",
        decimals: 1,
    },
    Spec {
        id: "eq_mid_freq",
        label: "MID FREQ",
        group: 3,
        min: 100.0,
        max: 8000.0,
        step: 0.0,
        default: 1200.0,
        unit: "Hz",
        cc: 49,
        automation: "Mid Freq",
        decimals: 0,
    },
    Spec {
        id: "eq_mid_q",
        label: "MID Q",
        group: 3,
        min: 0.1,
        max: 8.0,
        step: 0.0,
        default: 0.9,
        unit: "",
        cc: 50,
        automation: "Mid Q",
        decimals: 2,
    },
    Spec {
        id: "eq_high_gain",
        label: "HIGH GAIN",
        group: 3,
        min: -18.0,
        max: 18.0,
        step: 0.0,
        default: 0.0,
        unit: "dB",
        cc: 51,
        automation: "High Shelf Gain",
        decimals: 1,
    },
    Spec {
        id: "eq_high_freq",
        label: "HIGH FREQ",
        group: 3,
        min: 1000.0,
        max: 20000.0,
        step: 0.0,
        default: 8000.0,
        unit: "Hz",
        cc: 52,
        automation: "High Shelf Freq",
        decimals: 0,
    },
    // Delay & modulation
    Spec {
        id: "delay_mix",
        label: "DELAY MIX",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "%",
        cc: 56,
        automation: "Delay Mix",
        decimals: 0,
    },
    Spec {
        id: "delay_time",
        label: "DELAY TIME",
        group: 3,
        min: 10.0,
        max: 2000.0,
        step: 0.0,
        default: 350.0,
        unit: "ms",
        cc: 54,
        automation: "Delay Time",
        decimals: 0,
    },
    Spec {
        id: "delay_feedback",
        label: "FEEDBACK",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 30.0,
        unit: "%",
        cc: 55,
        automation: "Delay Feedback",
        decimals: 0,
    },
    Spec {
        id: "chorus_rate",
        label: "CHORUS RATE",
        group: 3,
        min: 0.05,
        max: 8.0,
        step: 0.0,
        default: 0.6,
        unit: "Hz",
        cc: 57,
        automation: "Chorus Rate",
        decimals: 2,
    },
    Spec {
        id: "chorus_depth",
        label: "CHORUS DEPTH",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "%",
        cc: 58,
        automation: "Chorus Depth",
        decimals: 0,
    },
    Spec {
        id: "tremolo_rate",
        label: "TREM RATE",
        group: 3,
        min: 0.1,
        max: 14.0,
        step: 0.0,
        default: 5.0,
        unit: "Hz",
        cc: 59,
        automation: "Tremolo Rate",
        decimals: 2,
    },
    Spec {
        id: "tremolo_depth",
        label: "TREM DEPTH",
        group: 3,
        min: 0.0,
        max: 100.0,
        step: 0.0,
        default: 0.0,
        unit: "%",
        cc: 60,
        automation: "Tremolo Depth",
        decimals: 0,
    },
];

/// The instruments the installed license unlocks: the core models plus every add-on found in
/// `~/.local/share/Modartt/Pianoteq/Addons` (Steinway DV80, K1, KIViR, KPRAUSCH, R1, V3, Bells).
pub const MODELS: &[Model] = &[
    // Acoustic grands
    Model { category: 0, name: "Steinway D", program: 0, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Steinway B", program: 1, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "C. Bechstein DG", program: 2, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Ant. Petrof 275", program: 3, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Steingraeber E-272", program: 4, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Grotrian Concert Royal", program: 5, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Steinway D New York", program: 6, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Grand D4", program: 7, icon: Icon::Grand, preset: None },
    // Uprights
    Model { category: 1, name: "U4 Upright", program: 8, icon: Icon::Upright, preset: None },
    Model { category: 1, name: "Concert Vintage Upright", program: 9, icon: Icon::Upright, preset: None },
    // Electric & tines
    Model { category: 2, name: "Rhodes (Vintage Tines)", program: 10, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Wurlitzer (Vintage Reeds)", program: 11, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Clavinet", program: 12, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Pianet", program: 13, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Rhody R1 (MK1 73)", program: 14, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Clavinet CL1", program: 15, icon: Icon::Electric, preset: None },
    // Mallets & chromatic
    Model { category: 3, name: "Celesta", program: 16, icon: Icon::Mallet, preset: None },
    Model { category: 3, name: "Glockenspiel", program: 17, icon: Icon::Mallet, preset: None },
    Model { category: 3, name: "Marimba", program: 18, icon: Icon::Mallet, preset: None },
    Model { category: 3, name: "Vibraphone", program: 19, icon: Icon::Mallet, preset: None },
    Model { category: 3, name: "Xylophone", program: 20, icon: Icon::Mallet, preset: None },
    Model { category: 3, name: "Cimbalom", program: 21, icon: Icon::Mallet, preset: None },
    // Historical (KIViR collection)
    Model { category: 4, name: "Blanchet Harpsichord", program: 22, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Walter Fortepiano", program: 23, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Graf Fortepiano", program: 24, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Erard Grand", program: 25, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Pleyel Fortepiano", program: 26, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Schantz Fortepiano", program: 27, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Schmidt Fortepiano", program: 28, icon: Icon::Historical, preset: None },
    Model { category: 4, name: "Schöffstoss Fortepiano", program: 29, icon: Icon::Historical, preset: None },
    // Bells & carillon
    Model { category: 5, name: "Church Bells", program: 30, icon: Icon::Bell, preset: None },
    Model { category: 5, name: "Cathedral Bells", program: 31, icon: Icon::Bell, preset: None },
    Model { category: 5, name: "Cambridge Chimes", program: 32, icon: Icon::Bell, preset: None },
    // Legacy & recording
    Model { category: 6, name: "Grand C3", program: 33, icon: Icon::Grand, preset: None },
    Model { category: 6, name: "Grand M3", program: 34, icon: Icon::Grand, preset: None },
    Model { category: 6, name: "K1 Grand", program: 35, icon: Icon::Grand, preset: None },
];

/// The controls drawn in the header (which also carry their own CCs).
pub const HEADER: &[Spec] = &[
    Spec {
        id: "master_volume",
        label: "OUTPUT",
        group: 255,
        min: -60.0,
        max: 6.0,
        step: 0.0,
        default: -6.0,
        unit: "dB",
        cc: 7,
        automation: "Master Volume",
        decimals: 1,
    },
    Spec {
        id: "tuning_a4",
        label: "A4 PITCH",
        group: 255,
        min: 415.0,
        max: 466.0,
        step: 0.0,
        default: 440.0,
        unit: "Hz",
        cc: 20,
        automation: "Tuning",
        decimals: 1,
    },
];

/// The console's profile: everything that makes it the Pianoteq console.
pub static PROFILE: Profile = Profile {
    id: "pianoteq",
    kind: "modelled piano",
    palette: PALETTE,
    plugin_ids: &["vst3:565354507439717069616E6F74657120", "vst3:565354507438717069616E6F74657120"],
    icon: Icon::Grand,
    brand: "PIANOTEQ",
    name: "Pianoteq 9",
    tagline: "alternative GUI · MIDI CC / automation",
    categories: CATEGORIES,
    models: MODELS,
    variants: VARIANTS,
    master: HEADER,
    racks: RACKS,
    specs: SPECS,
    selects: &[
        Select { id: "temperament", label: "TEMPERAMENT", group: 3, options: TEMPERAMENTS, cc: 21, automation: "Temperament" },
        Select { id: "reverb_type", label: "REVERB", group: 3, options: REVERB_TYPES, cc: 40, automation: "Reverb Type" },
    ],
    library_picker: None,
    velocity_curve: true,
    program_change: true,
    capture_first: false,
};

/// A parameter of the Pianoteq console.
pub fn spec_of(id: &str) -> Option<&'static Spec> {
    PROFILE.spec(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_kit;
    use crate::console_kit::{ConsoleState, VelocityCurve};

    #[test]
    fn every_parameter_has_a_unique_id_and_a_cc() {
        let mut ids = std::collections::HashSet::new();
        let mut ccs = std::collections::HashSet::new();
        for spec in SPECS {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            assert!(spec.cc >= 7 && spec.cc <= 119, "{} uses CC {}", spec.id, spec.cc);
            assert!(!spec.automation.is_empty(), "{} has no automation name", spec.id);
            assert!(spec.min < spec.max, "{} has an empty range", spec.id);
            assert!(spec.default >= spec.min && spec.default <= spec.max, "{} default outside its range", spec.id);
            assert!(spec.group == console_kit::NO_RACK || (spec.group as usize) < RACKS.len(), "{} points at a rack that does not exist", spec.id);
        }
        for spec in SPECS.iter().filter(|s| s.id != "master_volume") {
            assert!(ccs.insert(spec.cc), "CC {} is used twice", spec.cc);
        }
        // The selectors' CCs must not collide with the knobs' either.
        for sel in PROFILE.selects {
            assert!(ccs.insert(sel.cc), "CC {} is used twice (selector {})", sel.cc, sel.id);
            assert!(!sel.options.is_empty(), "{} has no options", sel.id);
            assert!((sel.group as usize) < RACKS.len(), "{} points at a rack that does not exist", sel.id);
        }
        // The header controls are part of the map too (they are drawn by the same knob), so they
        // must not collide with the racks' ids or CCs.
        for spec in PROFILE.master {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            assert!(ccs.insert(spec.cc), "CC {} is used twice ({})", spec.cc, spec.id);
        }
    }

    #[test]
    fn the_state_clamps_values_and_falls_back_to_defaults() {
        let mut state = ConsoleState::default();
        let unison = spec_of("unison_width").map(|s| s.default).unwrap_or(0.0);
        assert_eq!(state.value(&PROFILE, "unison_width"), unison);
        state.set(&PROFILE, "unison_width", 500.0);
        assert_eq!(state.value(&PROFILE, "unison_width"), 100.0, "clamped into the spec's range");
        state.set(&PROFILE, "nope", 1.0);
        assert_eq!(state.value(&PROFILE, "nope"), 0.0, "unknown ids are ignored");
    }

    #[test]
    fn the_model_list_covers_every_category() {
        for (index, category) in CATEGORIES.iter().enumerate() {
            assert!(MODELS.iter().any(|m| m.category == index), "{} has no instrument", category.title);
        }
        let mut programs = std::collections::HashSet::new();
        for model in MODELS {
            assert!(programs.insert(model.program), "{} reuses program {}", model.name, model.program);
        }
    }

    #[test]
    fn velocity_presets_keep_their_endpoints() {
        for index in 0..4 {
            let curve = VelocityCurve::preset(index);
            assert_eq!(curve.apply(0), 0);
            assert_eq!(curve.apply(127), 127);
        }
    }
}
