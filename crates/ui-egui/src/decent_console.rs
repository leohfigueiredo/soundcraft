//! Decent Sampler: the Craft Console profile for the `.dspreset` libraries on this machine.
//!
//! Decent Sampler is a bridged Windows plugin (JUCE). It loads fine here (2114 parameters,
//! editor opens), but a `.dspreset` can only be opened from inside the plugin: the format is
//! not a parameter value, it is a file the plugin reads. So this console works the way the
//! capture-first consoles do — pick a library, load it once in the plugin's own window, then
//! **right-click** the library here to save the plugin's state onto it. After that, clicking a
//! library restores it, which is what makes the packs one-click in Craft Console.
//!
//! The libraries listed are the ones found under `Pianote/.../Decent Samples/` on this machine.

use crate::console_kit::{Category, Icon, Model, Palette, Profile, Spec};

pub const EMBEDDED_STEINWAY_B64: &str = include_str!("decent_steinway.b64");

/// Slate and muted violet, its own accent (every console must be tellable apart).
pub const PALETTE: Palette = Palette::GRANITE;

pub const CATEGORIES: &[Category] = &[Category { title: "PIANOS" }, Category { title: "STRINGS" }, Category { title: "ELECTRIC" }, Category { title: "VOICES" }];

pub const MODELS: &[Model] = &[
    Model { category: 0, name: "Steinway Grand", program: 0, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Church Steinway", program: 1, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Manthey Klaviano", program: 2, icon: Icon::Grand, preset: None },
    Model { category: 0, name: "Legacy Knight", program: 3, icon: Icon::Upright, preset: None },
    Model { category: 1, name: "Woven Strings", program: 4, icon: Icon::Strings, preset: None },
    Model { category: 1, name: "Beyond the Strings", program: 5, icon: Icon::Strings, preset: None },
    Model { category: 2, name: "Matt's Fender Rhodes", program: 6, icon: Icon::Electric, preset: None },
    Model { category: 2, name: "Electro Acoustic Piano", program: 7, icon: Icon::Electric, preset: None },
    Model { category: 3, name: "Dave Choir", program: 8, icon: Icon::Pad, preset: None },
];

pub const RACKS: &[&str] = &["LIBRARY"];

/// One control only: Decent Sampler's own per-library controls differ between `.dspreset` files,
/// so the console does not pretend to know them. Level is the one every library exposes.
pub const SPECS: &[Spec] = &[Spec {
    id: "level",
    label: "LEVEL",
    group: 0,
    min: -48.0,
    max: 6.0,
    step: 0.0,
    default: -6.0,
    unit: "dB",
    cc: 7,
    automation: "Level",
    decimals: 1,
}];

pub static PROFILE: Profile = Profile {
    id: "decent_sampler",
    kind: "sample library",
    brand: "DECENT SAMPLER",
    name: "SAMPLE LIBRARIES",
    tagline: "the .dspreset packs, one click each",
    palette: PALETTE,
    icon: Icon::Sampler,
    // The bridged Decent Sampler bundle on this machine (class id from the VST3 metadata).
    plugin_ids: &["vst3:ABCDEF019182FAEB446C647944736D70"],
    categories: CATEGORIES,
    models: MODELS,
    variants: &["Close", "Room", "Wide", "Bright"],
    master: &[],
    racks: RACKS,
    specs: SPECS,
    selects: &[],
    library_picker: None,
    velocity_curve: true,
    // A `.dspreset` is a file, not a program: the plugin does not switch libraries on a program
    // change, so sounds are recalled from the state saved for each model.
    program_change: false,
    capture_first: true,
};
