//! Pigments: the alternative console profile (see [`crate::console_kit`] for the machinery).
//!
//! Same contract as the Pianoteq console: every control is in [`SPECS`]/[`SELECTS`] with an id, a
//! range, a unit, a MIDI CC and the parameter name Pigments uses, and every change leaves through
//! the shared dispatch hook — so it can be wired to WebMIDI, a WebSocket or OSC bridge without
//! touching the painting code.
//!
//! The names used here are SoundCraft's own: the racks mirror what a two-engine synth exposes
//! (engines, filters, modulation, effects, macros), not any proprietary preset or asset.

use crate::console_kit::{Category, Icon, Model, Palette, Profile, Select, Spec};
#[allow(unused_imports)]
use egui::Color32;

/// This instrument's console theme (magenta on near-black, Pigments' own colour).
pub const PALETTE: Palette = Palette {
    bg: Color32::from_rgb(0x0e, 0x0a, 0x12),
    panel: Color32::from_rgb(0x19, 0x12, 0x21),
    panel_head: Color32::from_rgb(0x22, 0x18, 0x2d),
    line: Color32::from_rgb(0x33, 0x27, 0x42),
    edge: Color32::from_rgb(0x53, 0x41, 0x68),
    accent: Color32::from_rgb(0xe0, 0x5a, 0x9e),
    accent_dim: Color32::from_rgb(0x9c, 0x3f, 0x6f),
    text: Color32::from_rgb(0xec, 0xe6, 0xf2),
    text_dim: Color32::from_rgb(0x95, 0x8b, 0xa3),
    live: Color32::from_rgb(0x5d, 0xd6, 0x8a),
};

/// Preset banks in the header (the console labels them; the program change selects one).
pub const CATEGORIES: &[Category] = &[
    Category { title: "PADS" },
    Category { title: "BASSES" },
    Category { title: "LEADS" },
    Category { title: "KEYS" },
    Category { title: "SEQUENCES" },
    Category { title: "TEXTURES" },
];

/// Morph/preset variants in the header dropdown.
pub const VARIANTS: &[&str] = &["Neutral", "Soft Attack", "Bright", "Dark", "Wide Stereo"];

/// The racks, in drawing order (a parameter picks one by index).
pub const RACKS: &[&str] = &["ENGINE 1", "ENGINE 2", "FILTERS & UTILITY", "MODULATION", "FX RACK", "MACROS"];

/// Oscillator engines offered by each engine slot.
pub const ENGINE_TYPES: &[&str] = &["Analog", "Wavetable", "Sample", "Granular", "Harmonic", "Modal"];
/// Filter models.
pub const FILTER_TYPES: &[&str] = &["LP24", "LP12", "HP24", "BP12", "Notch", "Comb", "Phaser", "Bypass"];
/// Insert effect models.
pub const INSERT_TYPES: &[&str] = &["None", "Chorus", "Phaser", "Flanger", "Distortion", "Bit Crusher", "Compressor", "Param EQ"];
/// Filter routing between the two filters.
pub const ROUTING: &[&str] = &["Serial", "Parallel", "Split"];
/// LFO shapes.
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H", "Random"];

/// Pitch-bend style macro labels: the four macros every preset exposes.
const fn knob(
    id: &'static str,
    label: &'static str,
    group: u8,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
    cc: u8,
    automation: &'static str,
) -> Spec {
    Spec { id, label, group, min, max, step: 0.0, default, unit, cc, automation, decimals: 0 }
}

/// Every continuous control of the Pigments console.
pub const SPECS: &[Spec] = &[
    // Engine 1
    knob("engine1_level", "E1 LEVEL", 0, 0.0, 100.0, 75.0, "%", 20, "Engine 1 Level"),
    knob("engine1_tune", "E1 TUNE", 0, -24.0, 24.0, 0.0, "st", 21, "Engine 1 Tune"),
    knob("engine1_fine", "E1 FINE", 0, -100.0, 100.0, 0.0, "ct", 22, "Engine 1 Fine Tune"),
    knob("engine1_pan", "E1 PAN", 0, -100.0, 100.0, 0.0, "%", 23, "Engine 1 Pan"),
    knob("engine1_unison", "VOICES", 0, 1.0, 16.0, 1.0, "", 24, "Engine 1 Unison Voices"),
    knob("engine1_detune", "E1 DETUNE", 0, 0.0, 100.0, 20.0, "%", 25, "Engine 1 Unison Detune"),
    knob("engine1_character", "WARP", 0, 0.0, 100.0, 50.0, "%", 26, "Engine 1 Warp"),
    knob("engine1_position", "E1 POSITION", 0, 0.0, 100.0, 0.0, "%", 27, "Engine 1 Position"),
    // Engine 2
    knob("engine2_level", "E2 LEVEL", 1, 0.0, 100.0, 60.0, "%", 28, "Engine 2 Level"),
    knob("engine2_tune", "E2 TUNE", 1, -24.0, 24.0, 0.0, "st", 29, "Engine 2 Tune"),
    knob("engine2_fine", "E2 FINE", 1, -100.0, 100.0, 0.0, "ct", 30, "Engine 2 Fine Tune"),
    knob("engine2_pan", "E2 PAN", 1, -100.0, 100.0, 0.0, "%", 31, "Engine 2 Pan"),
    knob("engine2_unison", "VOICES", 1, 1.0, 16.0, 1.0, "", 32, "Engine 2 Unison Voices"),
    knob("engine2_detune", "E2 DETUNE", 1, 0.0, 100.0, 15.0, "%", 33, "Engine 2 Unison Detune"),
    knob("engine2_character", "WARP", 1, 0.0, 100.0, 50.0, "%", 34, "Engine 2 Warp"),
    knob("engine2_position", "E2 POSITION", 1, 0.0, 100.0, 0.0, "%", 35, "Engine 2 Position"),
    // Filters and utility engine
    knob("filter1_cutoff", "F1 CUTOFF", 2, 20.0, 20000.0, 8000.0, "Hz", 36, "Filter 1 Cutoff"),
    knob("filter1_reso", "F1 RESO", 2, 0.0, 100.0, 15.0, "%", 37, "Filter 1 Resonance"),
    knob("filter1_env", "F1 ENV", 2, -100.0, 100.0, 30.0, "%", 38, "Filter 1 Env Amount"),
    knob("filter1_drive", "F1 DRIVE", 2, 0.0, 100.0, 0.0, "%", 39, "Filter 1 Drive"),
    knob("filter2_cutoff", "F2 CUTOFF", 2, 20.0, 20000.0, 12000.0, "Hz", 40, "Filter 2 Cutoff"),
    knob("filter2_reso", "F2 RESO", 2, 0.0, 100.0, 10.0, "%", 41, "Filter 2 Resonance"),
    knob("filter2_env", "F2 ENV", 2, -100.0, 100.0, 0.0, "%", 42, "Filter 2 Env Amount"),
    knob("noise_level", "NOISE", 2, 0.0, 100.0, 0.0, "%", 43, "Noise Level"),
    knob("sub_level", "SUB", 2, 0.0, 100.0, 0.0, "%", 44, "Sub Oscillator Level"),
    knob("click_level", "CLICK", 2, 0.0, 100.0, 20.0, "%", 45, "Click Level"),
    // Modulation: LFOs and the two envelopes
    knob("lfo1_rate", "LFO1 RATE", 3, 0.05, 40.0, 0.5, "Hz", 46, "LFO 1 Rate"),
    knob("lfo1_amount", "LFO1 AMT", 3, 0.0, 100.0, 0.0, "%", 47, "LFO 1 Amount"),
    knob("lfo2_rate", "LFO2 RATE", 3, 0.05, 40.0, 2.0, "Hz", 48, "LFO 2 Rate"),
    knob("lfo2_amount", "LFO2 AMT", 3, 0.0, 100.0, 0.0, "%", 49, "LFO 2 Amount"),
    knob("env1_attack", "ENV1 A", 3, 0.0, 10.0, 0.01, "s", 50, "Envelope 1 Attack"),
    knob("env1_decay", "ENV1 D", 3, 0.0, 10.0, 0.4, "s", 51, "Envelope 1 Decay"),
    knob("env1_sustain", "ENV1 S", 3, 0.0, 100.0, 70.0, "%", 52, "Envelope 1 Sustain"),
    knob("env1_release", "ENV1 R", 3, 0.0, 20.0, 0.5, "s", 53, "Envelope 1 Release"),
    knob("env2_attack", "ENV2 A", 3, 0.0, 10.0, 0.5, "s", 54, "Envelope 2 Attack"),
    knob("env2_decay", "ENV2 D", 3, 0.0, 10.0, 1.0, "s", 55, "Envelope 2 Decay"),
    knob("env2_sustain", "ENV2 S", 3, 0.0, 100.0, 50.0, "%", 56, "Envelope 2 Sustain"),
    knob("env2_release", "ENV2 R", 3, 0.0, 20.0, 1.0, "s", 57, "Envelope 2 Release"),
    // FX rack
    knob("delay_time", "DELAY", 4, 1.0, 2000.0, 320.0, "ms", 58, "Delay Time"),
    knob("delay_feedback", "FEEDBACK", 4, 0.0, 100.0, 30.0, "%", 59, "Delay Feedback"),
    knob("delay_mix", "D MIX", 4, 0.0, 100.0, 0.0, "%", 60, "Delay Mix"),
    knob("delay_pingpong", "PING-PONG", 4, 0.0, 100.0, 0.0, "%", 61, "Delay Ping Pong"),
    knob("reverb_size", "ROOM", 4, 0.0, 100.0, 50.0, "%", 62, "Reverb Size"),
    knob("reverb_decay", "DECAY", 4, 0.2, 30.0, 3.0, "s", 63, "Reverb Decay"),
    knob("reverb_mix", "R MIX", 4, 0.0, 100.0, 15.0, "%", 64, "Reverb Mix"),
    knob("insert_drive", "INS DRIVE", 4, 0.0, 100.0, 0.0, "%", 65, "Insert Drive"),
    knob("insert_mix", "INS MIX", 4, 0.0, 100.0, 0.0, "%", 66, "Insert Mix"),
    // Macros: what a preset exposes for remote control
    knob("macro1", "MACRO 1", 5, 0.0, 100.0, 0.0, "%", 67, "Macro 1"),
    knob("macro2", "MACRO 2", 5, 0.0, 100.0, 0.0, "%", 68, "Macro 2"),
    knob("macro3", "MACRO 3", 5, 0.0, 100.0, 0.0, "%", 69, "Macro 3"),
    knob("macro4", "MACRO 4", 5, 0.0, 100.0, 0.0, "%", 70, "Macro 4"),
];

/// The header controls (master output and glide).
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
        id: "glide",
        label: "GLIDE",
        group: 255,
        min: 0.0,
        max: 2000.0,
        step: 0.0,
        default: 0.0,
        unit: "ms",
        cc: 8,
        automation: "Portamento",
        decimals: 0,
    },
];

/// The discrete choices: engine models, filters, routing, LFO shapes and the insert effect.
pub const SELECTS: &[Select] = &[
    Select { id: "engine1_type", label: "ENGINE 1 MODEL", group: 0, options: ENGINE_TYPES, cc: 71, automation: "Engine 1 Type" },
    Select { id: "engine2_type", label: "ENGINE 2 MODEL", group: 1, options: ENGINE_TYPES, cc: 72, automation: "Engine 2 Type" },
    Select { id: "filter1_type", label: "FILTER 1", group: 2, options: FILTER_TYPES, cc: 73, automation: "Filter 1 Type" },
    Select { id: "filter2_type", label: "FILTER 2", group: 2, options: FILTER_TYPES, cc: 74, automation: "Filter 2 Type" },
    Select { id: "filter_route", label: "ROUTING", group: 2, options: ROUTING, cc: 75, automation: "Filter Routing" },
    Select { id: "lfo1_shape", label: "LFO1 SHAPE", group: 3, options: LFO_SHAPES, cc: 76, automation: "LFO 1 Shape" },
    Select { id: "lfo2_shape", label: "LFO2 SHAPE", group: 3, options: LFO_SHAPES, cc: 77, automation: "LFO 2 Shape" },
    Select { id: "insert_type", label: "INSERT FX", group: 4, options: INSERT_TYPES, cc: 78, automation: "Insert Effect" },
];

/// Preset banks: the console's own labels (a preset is picked by program change).
pub const MODELS: &[Model] = &[
    Model { category: 0, name: "Warm Analog Pad", program: 0, icon: Icon::Pad, preset: None },
    Model { category: 0, name: "Glass Wavetable", program: 1, icon: Icon::Synth, preset: None },
    Model { category: 0, name: "Granular Cloud", program: 2, icon: Icon::Texture, preset: None },
    Model { category: 1, name: "Solid Sub Bass", program: 3, icon: Icon::Bass, preset: None },
    Model { category: 1, name: "Acid Wavetable Bass", program: 4, icon: Icon::Bass, preset: None },
    Model { category: 2, name: "Screaming Lead", program: 5, icon: Icon::Lead, preset: None },
    Model { category: 2, name: "Soft Sine Lead", program: 6, icon: Icon::Lead, preset: None },
    Model { category: 3, name: "Electric Keys", program: 7, icon: Icon::Electric, preset: None },
    Model { category: 3, name: "Plucked Keys", program: 8, icon: Icon::Keys, preset: None },
    Model { category: 4, name: "Arp Sequence", program: 9, icon: Icon::Sequence, preset: None },
    Model { category: 4, name: "Rhythmic Pulse", program: 10, icon: Icon::Sequence, preset: None },
    Model { category: 5, name: "Slow Texture", program: 11, icon: Icon::Texture, preset: None },
    Model { category: 5, name: "Metallic Texture", program: 12, icon: Icon::Texture, preset: None },
];

/// The console's profile.
pub static PROFILE: Profile = Profile {
    id: "pigments",
    kind: "hybrid synth",
    palette: PALETTE,
    plugin_ids: &["vst3:41727475415649534B61743150726F63"],
    icon: Icon::Synth,
    brand: "PIGMENTS",
    name: "Pigments",
    tagline: "alternative GUI · two engines, filters, mod & FX",
    categories: CATEGORIES,
    models: MODELS,
    variants: VARIANTS,
    master: HEADER,
    racks: RACKS,
    specs: SPECS,
    selects: SELECTS,
    library_picker: None,
    velocity_curve: true,
    program_change: false,
    capture_first: true,
};

/// A parameter of the Pigments console.
pub fn spec_of(id: &str) -> Option<&'static Spec> {
    PROFILE.spec(id)
}

/// A discrete choice of the Pigments console.
pub fn select_of(id: &str) -> Option<&'static Select> {
    PROFILE.select(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_kit;

    #[test]
    fn every_control_has_a_unique_id_a_cc_and_a_real_range() {
        let mut ids = std::collections::HashSet::new();
        let mut ccs = std::collections::HashSet::new();
        let all = SPECS.iter().chain(HEADER.iter());
        for spec in all {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            assert!(ccs.insert(spec.cc), "CC {} is used twice ({})", spec.cc, spec.id);
            assert!(!spec.automation.is_empty(), "{} has no automation name", spec.id);
            assert!(spec.min < spec.max, "{} has an empty range", spec.id);
            assert!(spec.default >= spec.min && spec.default <= spec.max, "{} default outside its range", spec.id);
            assert!(spec.group == 255 || (spec.group as usize) < RACKS.len(), "{} points at a rack that does not exist", spec.id);
        }
        for sel in SELECTS {
            assert!(ids.insert(sel.id), "duplicate id {}", sel.id);
            assert!(ccs.insert(sel.cc), "CC {} is used twice ({})", sel.cc, sel.id);
            assert!(!sel.options.is_empty(), "{} has no options", sel.id);
            assert!(sel.options.len() <= 128, "{} has more options than a CC can carry", sel.id);
            assert!((sel.group as usize) < RACKS.len(), "{} points at a rack that does not exist", sel.id);
        }
    }

    #[test]
    fn every_rack_and_bank_has_something_in_it() {
        for (index, rack) in RACKS.iter().enumerate() {
            let has_spec = SPECS.iter().any(|s| s.group as usize == index);
            let has_select = SELECTS.iter().any(|s| s.group as usize == index);
            assert!(has_spec || has_select, "rack `{rack}` would draw empty");
        }
        for (index, category) in CATEGORIES.iter().enumerate() {
            assert!(MODELS.iter().any(|m| m.category == index), "bank `{}` has no preset", category.title);
        }
        assert_eq!(ENGINE_TYPES.len(), 6, "the engine model list is the six Pigments engines");
    }

    #[test]
    fn the_state_holds_this_console_s_own_defaults() {
        let mut state = console_kit::ConsoleState::default();
        assert_eq!(state.value(&PROFILE, "engine1_level"), 75.0);
        assert_eq!(state.value(&PROFILE, "macro1"), 0.0);
        state.set(&PROFILE, "engine1_level", 500.0);
        assert_eq!(state.value(&PROFILE, "engine1_level"), 100.0, "clamped into the range");
        // A Pianoteq id is not a Pigments parameter: the two consoles never share a namespace.
        assert_eq!(state.value(&PROFILE, "unison_width"), 0.0);
    }
}
