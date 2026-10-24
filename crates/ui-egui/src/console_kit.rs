//! One console window for every hosted instrument: the palette, the parameter map types, the state
//! each instrument remembers, the dispatch hooks and the controls. An instrument is a [`Profile`] —
//! its racks, parameters, banks and **its own theme** — so switching instruments inside the window
//! changes the racks, the accent colour and the window title without any other code.
//!
//! Every control ends in `dispatch`/`select`/`play_note`: the single place that knows how values
//! leave the app (today: a MIDI CC to the active instrument track; a bridge replaces that body).

use crate::SoundApp;
use crate::theme::{bold, regular};
use egui::{Align, Align2, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, pos2, vec2};

// ---- theme ------------------------------------------------------------------------------------

/// The console's palette: every instrument brings its own, so the window reads as that instrument.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_head: Color32,
    pub line: Color32,
    pub edge: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub live: Color32,
}

impl Palette {
    /// Graphite and amber: the neutral console look.
    pub const GRAPHITE: Palette = Palette {
        bg: Color32::from_rgb(0x12, 0x13, 0x16),
        panel: Color32::from_rgb(0x1a, 0x1c, 0x23),
        panel_head: Color32::from_rgb(0x20, 0x22, 0x2a),
        line: Color32::from_rgb(0x2c, 0x2f, 0x38),
        edge: Color32::from_rgb(0x44, 0x48, 0x53),
        accent: Color32::from_rgb(0xf0, 0xb4, 0x4a),
        accent_dim: Color32::from_rgb(0xa8, 0x7a, 0x2c),
        text: Color32::from_rgb(0xe7, 0xe9, 0xee),
        text_dim: Color32::from_rgb(0x8b, 0x90, 0x9c),
        live: Color32::from_rgb(0x5d, 0xd6, 0x8a),
    };
}

// ---- parameter map ----------------------------------------------------------------------------

/// One continuous parameter.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    /// Which rack of the profile it belongs to (index into [`Profile::racks`], or [`NO_RACK`]).
    pub group: u8,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default: f32,
    pub unit: &'static str,
    pub cc: u8,
    /// The name the plugin's own parameter list uses.
    pub automation: &'static str,
    pub decimals: u8,
}

/// Marker for a control that is not in a rack (the header, or the dynamic-response row).
pub const NO_RACK: u8 = 255;

/// A discrete choice (engine model, filter type, reverb algorithm), drawn as a row of buttons.
#[derive(Debug, Clone, Copy)]
pub struct Select {
    pub id: &'static str,
    pub label: &'static str,
    pub group: u8,
    pub options: &'static [&'static str],
    pub cc: u8,
    pub automation: &'static str,
}

/// A small pictogram per instrument, drawn with primitives: every icon in SoundCraft is code, so
/// nothing is copied from the plugins (see `AGENTS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Acoustic grand: lid curve over a keyboard strip.
    Grand,
    /// Upright: tall case over a keyboard strip.
    Upright,
    /// Electric piano / tines: keyboard strip and two tine dots.
    Electric,
    /// Mallet percussion: crossed sticks over a bar.
    Mallet,
    /// Historical: keyboard strip under a quill line.
    Historical,
    /// Generic synth: a sine.
    Synth,
    /// Pad: a slow swell.
    Pad,
    /// Bass: a low square wave.
    Bass,
    /// Lead: a sharp saw.
    Lead,
    /// Keys/sampled instrument: a note head on a staff line.
    Keys,
    /// Sequence: a row of steps.
    Sequence,
    /// Texture: a noisy cloud.
    Texture,
    /// Sampler: a waveform inside a loop bracket.
    Sampler,
    /// Bells: a dome with its clapper.
    Bell,
    /// Bow on strings: two string lines and a bow.
    Strings,
    /// Brass: a bell flare off a tube.
    Brass,
}

impl Icon {
    /// Every kind, for the pictogram test.
    pub const ALL: &'static [Icon] = &[
        Icon::Grand,
        Icon::Upright,
        Icon::Electric,
        Icon::Mallet,
        Icon::Historical,
        Icon::Synth,
        Icon::Pad,
        Icon::Bass,
        Icon::Lead,
        Icon::Keys,
        Icon::Sequence,
        Icon::Texture,
        Icon::Sampler,
        Icon::Bell,
        Icon::Strings,
        Icon::Brass,
    ];

    /// The icon's primitive shapes inside `r`: `paint` is `painter.extend(shapes)`, and a test can
    /// assert the drawing without a GPU.
    pub fn shapes(self, r: Rect, color: Color32) -> Vec<egui::Shape> {
        let stroke = Stroke::new(1.2, color);
        let faint = Stroke::new(0.9, color.gamma_multiply(0.7));
        let (l, t, w, h) = (r.left(), r.top(), r.width(), r.height());
        let y = |f: f32| t + h * f;
        let x = |f: f32| l + w * f;
        let line = |pts: Vec<Pos2>| egui::Shape::line(pts, stroke);
        let bar = |top: f32, bottom: f32| egui::Shape::rect_filled(Rect::from_min_max(pos2(x(0.12), y(top)), pos2(x(0.88), y(bottom))), 1.0, color);
        match self {
            // Lid curve over a keyboard strip.
            Icon::Grand => {
                vec![line(vec![pos2(x(0.1), y(0.7)), pos2(x(0.25), y(0.25)), pos2(x(0.75), y(0.2)), pos2(x(0.9), y(0.45))]), bar(0.72, 0.88)]
            }
            // Tall case over a keyboard strip.
            Icon::Upright => vec![
                egui::Shape::rect_stroke(Rect::from_min_max(pos2(x(0.24), y(0.18)), pos2(x(0.76), y(0.62))), 1.0, stroke, egui::StrokeKind::Inside),
                bar(0.68, 0.86),
            ],
            // Keyboard strip and two tines.
            Icon::Electric => vec![
                bar(0.6, 0.78),
                egui::Shape::circle_filled(pos2(x(0.32), y(0.34)), w * 0.07, color),
                egui::Shape::circle_filled(pos2(x(0.68), y(0.34)), w * 0.07, color),
            ],
            // Crossed sticks over a bar.
            Icon::Mallet => vec![
                egui::Shape::line_segment([pos2(x(0.2), y(0.2)), pos2(x(0.6), y(0.68))], stroke),
                egui::Shape::line_segment([pos2(x(0.8), y(0.2)), pos2(x(0.4), y(0.68))], stroke),
                bar(0.74, 0.9),
            ],
            // A quill line over a key bed: the historical keyboards.
            Icon::Historical => vec![egui::Shape::line_segment([pos2(x(0.12), y(0.3)), pos2(x(0.88), y(0.3))], faint), bar(0.5, 0.82)],
            // A sine.
            Icon::Synth => vec![line(
                (0..=12)
                    .map(|i| {
                        let f = i as f32 / 12.0;
                        pos2(x(0.1 + 0.8 * f), y(0.5) - (f * std::f32::consts::TAU).sin() * h * 0.24)
                    })
                    .collect(),
            )],
            // A slow swell.
            Icon::Pad => vec![line(
                (0..=12)
                    .map(|i| {
                        let f = i as f32 / 12.0;
                        pos2(x(0.1 + 0.8 * f), y(0.62) - (f * std::f32::consts::PI).sin() * h * 0.34)
                    })
                    .collect(),
            )],
            // A low square wave.
            Icon::Bass => vec![line(vec![
                pos2(x(0.1), y(0.7)),
                pos2(x(0.28), y(0.7)),
                pos2(x(0.28), y(0.32)),
                pos2(x(0.5), y(0.32)),
                pos2(x(0.5), y(0.7)),
                pos2(x(0.72), y(0.7)),
                pos2(x(0.72), y(0.32)),
                pos2(x(0.9), y(0.32)),
            ])],
            // A sharp saw.
            Icon::Lead => vec![line(vec![pos2(x(0.1), y(0.76)), pos2(x(0.32), y(0.76)), pos2(x(0.58), y(0.24)), pos2(x(0.9), y(0.24))])],
            // A note head on two staff lines.
            Icon::Keys => vec![
                egui::Shape::line_segment([pos2(x(0.12), y(0.36)), pos2(x(0.88), y(0.36))], faint),
                egui::Shape::line_segment([pos2(x(0.12), y(0.6)), pos2(x(0.88), y(0.6))], faint),
                egui::Shape::circle_filled(pos2(x(0.4), y(0.64)), w * 0.1, color),
                egui::Shape::line_segment([pos2(x(0.49), y(0.62)), pos2(x(0.49), y(0.24))], stroke),
            ],
            // Four steps of different heights.
            Icon::Sequence => {
                let mut shapes = Vec::new();
                for (fx, fy) in [(0.16, 0.55), (0.4, 0.28), (0.64, 0.72), (0.86, 0.38)] {
                    shapes.push(egui::Shape::rect_filled(
                        Rect::from_min_max(pos2(x(fx), y(fy)), pos2(x(fx + 0.1), y(0.9))),
                        1.0,
                        color.gamma_multiply(0.9),
                    ));
                }
                shapes
            }
            // A cloud of grains.
            Icon::Texture => (0..7)
                .map(|i| {
                    let fx = 0.18 + 0.1 * i as f32;
                    let fy = 0.3 + 0.11 * ((i * 3) % 4) as f32;
                    egui::Shape::circle_stroke(pos2(x(fx), y(fy)), w * 0.09, stroke)
                })
                .collect(),
            // A waveform over a loop line.
            Icon::Sampler => vec![
                line(
                    (0..=12)
                        .map(|i| {
                            let f = i as f32 / 12.0;
                            pos2(x(0.12 + 0.76 * f), y(0.5) - ((f * 3.0 * std::f32::consts::PI).sin() * (1.0 - f) * 0.3))
                        })
                        .collect(),
                ),
                egui::Shape::line_segment([pos2(x(0.12), y(0.86)), pos2(x(0.88), y(0.86))], faint),
            ],
            // A bell: dome, rim and clapper.
            Icon::Bell => vec![
                line(vec![pos2(x(0.26), y(0.66)), pos2(x(0.34), y(0.4)), pos2(x(0.5), y(0.22)), pos2(x(0.66), y(0.4)), pos2(x(0.74), y(0.66))]),
                egui::Shape::line_segment([pos2(x(0.18), y(0.72)), pos2(x(0.82), y(0.72))], stroke),
                egui::Shape::circle_filled(pos2(x(0.5), y(0.84)), w * 0.08, color),
            ],
            // Two strings and a bow across them.
            Icon::Strings => vec![
                egui::Shape::line_segment([pos2(x(0.24), y(0.1)), pos2(x(0.24), y(0.9))], faint),
                egui::Shape::line_segment([pos2(x(0.38), y(0.1)), pos2(x(0.38), y(0.9))], faint),
                egui::Shape::line_segment([pos2(x(0.14), y(0.8)), pos2(x(0.6), y(0.18))], stroke),
                egui::Shape::line_segment([pos2(x(0.52), y(0.86)), pos2(x(0.92), y(0.3))], stroke),
            ],
            // A tube flaring into a bell.
            Icon::Brass => vec![
                egui::Shape::rect_filled(Rect::from_min_max(pos2(x(0.1), y(0.44)), pos2(x(0.54), y(0.58))), 1.0, color),
                egui::Shape::convex_polygon(
                    vec![pos2(x(0.54), y(0.3)), pos2(x(0.9), y(0.12)), pos2(x(0.9), y(0.88)), pos2(x(0.54), y(0.7))],
                    color,
                    stroke,
                ),
            ],
        }
    }

    /// Paints the icon inside `r`, in `color`.
    pub fn paint(self, painter: &egui::Painter, r: Rect, color: Color32) {
        painter.extend(self.shapes(r, color));
    }
}

/// An instrument or preset bank in the header.
#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub category: usize,
    pub name: &'static str,
    pub program: u8,
    pub icon: Icon,
    /// The preset this instrument maps to inside the plugin, when its name is not the key: the
    /// first program (or saved state) whose name contains this text, case-insensitively. `None`
    /// means the instrument's own name is the key.
    pub preset: Option<&'static str>,
}

/// One library of a sampler profile (a Kontakt library: name + content folder).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub path: String,
}

/// The library list, cached; `None` until it is read (or replaced by a test).
static LIBRARIES: std::sync::Mutex<Option<Vec<Library>>> = std::sync::Mutex::new(None);

/// Where the library list lives (`SOUNDCRAFT_KONTAKT_LIBRARIES` overrides it, which is also how a
/// test points at a fixture).
pub fn library_file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SOUNDCRAFT_KONTAKT_LIBRARIES") {
        return Some(std::path::PathBuf::from(path));
    }
    Some(settings_dir()?.join("kontakt_libraries.json"))
}

/// The app's settings directory (`SOUNDCRAFT_CONFIG_DIR` overrides it; `None` disables it).
pub fn settings_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("SOUNDCRAFT_CONFIG_DIR").map(std::path::PathBuf::from).or_else(|| {
        std::env::var_os("HOME").map(|home| {
            let mut p = std::path::PathBuf::from(home);
            p.push(".config");
            p.push("soundcraft");
            p
        })
    })
}

/// The installed libraries (empty when the scan has not run).
pub fn libraries() -> Vec<Library> {
    if let Ok(mut cache) = LIBRARIES.lock() {
        if cache.is_none() {
            let loaded = library_file()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| serde_json::from_str::<Vec<Library>>(&text).ok())
                .unwrap_or_default();
            *cache = Some(loaded);
        }
        return cache.clone().unwrap_or_default();
    }
    Vec::new()
}

/// Replaces the library list (the scan finished, or a test pins its own fixture).
pub fn set_libraries(list: Vec<Library>) {
    if let Ok(mut cache) = LIBRARIES.lock() {
        *cache = Some(list);
    }
}

impl Library {
    /// The pictogram for a library, from the words in its name: a sampler library says what it is
    /// ("Alicia's Keys", "Studio Drummer", "Symphonic Strings"), and the console draws that.
    pub fn icon(&self) -> Icon {
        let name = self.name.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
        #[allow(clippy::if_same_then_else)]
        let icon = if has(&["piano", "grand", "upright", "steinway", "bechstein", "yamaha", "kawai"]) {
            Icon::Grand
        } else if has(&["drum", "kit", "percussion", "taiko", "djembe", "cajon"]) {
            Icon::Sampler
        } else if has(&["string", "violin", "cello", "viola", "orchestra", "ensemble", "quartet"]) {
            Icon::Strings
        } else if has(&["choir", "voice", "vocal", "choral"]) {
            Icon::Strings
        } else if has(&["brass", "horn", "trumpet", "trombone", "tuba", "sax", "flute", "clarinet"]) {
            Icon::Brass
        } else if has(&["bell", "chime", "gong", "celesta", "vibraphone", "glockenspiel", "marimba"]) {
            Icon::Bell
        } else if has(&["bass"]) {
            Icon::Bass
        } else if has(&["synth", "analog", "analogue", "arp", "moog", "prophet", "juno", "wavetable"]) {
            Icon::Synth
        } else if has(&["organ", "keys", "clav", "rhodes", "wurli", "electric"]) {
            Icon::Keys
        } else if has(&["guitar", "bass guitar", "banjo", "mandolin", "ukulele"]) {
            Icon::Lead
        } else if has(&["harp", "harpsichord", "clavichord", "fortepiano"]) {
            Icon::Historical
        } else if has(&["pad", "texture", "ambient", "atmosphere", "drone"]) {
            Icon::Pad
        } else {
            Icon::Sampler
        };
        icon
    }
}

/// A header tab ("Acoustic Grands", "Pads", "Multi").
#[derive(Debug, Clone, Copy)]
pub struct Category {
    pub title: &'static str,
}

/// Everything that makes one instrument's console: its own racks, map, lists and theme.
#[derive(Debug)]
pub struct Profile {
    /// Stable id ("pianoteq", "omnisphere"): keys the state, the window and the shortcuts.
    pub id: &'static str,
    /// Short kind label for the header ("modelled piano", "hybrid synth").
    pub kind: &'static str,
    /// Uppercase brand shown first in the header.
    pub brand: &'static str,
    /// Product name after the brand.
    pub name: &'static str,
    /// One line under the title.
    pub tagline: &'static str,
    /// The instrument's own palette: the window and its controls take this colour.
    pub palette: Palette,
    /// The instrument's pictogram, drawn in code (see [`Icon`]).
    pub icon: Icon,
    /// The hosted plugins this console drives (their class ids), so a track's ALT button appears
    /// for exactly these VSTs even before the plugin catalogue has been scanned.
    pub plugin_ids: &'static [&'static str],
    pub categories: &'static [Category],
    pub models: &'static [Model],
    pub variants: &'static [&'static str],
    /// Parameters drawn in the header.
    pub master: &'static [Spec],
    /// Rack titles, in order.
    pub racks: &'static [&'static str],
    pub specs: &'static [Spec],
    pub selects: &'static [Select],
    /// A sampler library picker: the rack it belongs to and the id its state is stored under.
    pub library_picker: Option<(u8, &'static str)>,
    /// Draw the dynamic-response row (velocity curve, dynamic range, voices, sustain).
    pub velocity_curve: bool,
    /// Send a program change when a model is picked.
    pub program_change: bool,
}

impl Profile {
    pub fn spec(&self, id: &str) -> Option<&'static Spec> {
        self.specs.iter().find(|s| s.id == id).or_else(|| self.master.iter().find(|s| s.id == id))
    }

    pub fn select(&self, id: &str) -> Option<&'static Select> {
        self.selects.iter().find(|s| s.id == id)
    }

    /// The window title for this instrument: the VST's own name.
    pub fn window_title(&self) -> String {
        format!("{} — Alternative GUI", self.name)
    }
}

/// Every console the app can show, in tab order.
pub fn profiles() -> &'static [&'static Profile] {
    PROFILE_LIST.get_or_init(|| {
        vec![
            &crate::pianoteq_console::PROFILE,
            &crate::pigments_console::PROFILE,
            &crate::omnisphere_console::PROFILE,
            &crate::kontakt_console::PROFILE,
            &crate::analoglab_console::PROFILE,
            &crate::labs_console::PROFILE,
        ]
    })
}

static PROFILE_LIST: std::sync::OnceLock<Vec<&'static Profile>> = std::sync::OnceLock::new();

/// The profile with this id (`None` for an unknown one).
pub fn profile(id: &str) -> Option<&'static Profile> {
    profiles().iter().copied().find(|p| p.id == id)
}

// ---- state ------------------------------------------------------------------------------------

/// A velocity map: incoming MIDI velocity (0..127) to the velocity handed to the plugin.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VelocityCurve {
    /// Nine control points, x fixed at 0, 16, 32, … 127 (velocity in), y = velocity out.
    pub points: [f32; 9],
}

impl Default for VelocityCurve {
    fn default() -> Self {
        VelocityCurve { points: [0.0, 20.0, 40.0, 58.0, 74.0, 90.0, 104.0, 117.0, 127.0] }
    }
}

impl VelocityCurve {
    /// One of the presets in the curve editor.
    pub fn preset(index: usize) -> VelocityCurve {
        let mut c = VelocityCurve::default();
        for (i, slot) in c.points.iter_mut().enumerate() {
            let x = i as f32 / 8.0;
            *slot = 127.0
                * match index {
                    0 => x,
                    1 => x * x,
                    2 => x.sqrt(),
                    3 => x * x * (3.0 - 2.0 * x),
                    _ => 0.5 * (x + x * x * (3.0 - 2.0 * x)),
                };
        }
        if let Some(first) = c.points.first_mut() {
            *first = 0.0;
        }
        if let Some(last) = c.points.last_mut() {
            *last = 127.0;
        }
        c
    }

    /// Maps an incoming velocity through the curve (linear between the control points).
    pub fn apply(&self, velocity: u8) -> u8 {
        let x = f32::from(velocity.min(127)) / 127.0 * 8.0;
        let i = x.floor().clamp(0.0, 8.0) as usize;
        let frac = x - i as f32;
        let a = self.points.get(i).copied().unwrap_or(0.0);
        let b = self.points.get((i + 1).min(8)).copied().unwrap_or(127.0);
        let v = a + (b - a) * frac;
        v.clamp(0.0, 127.0).round() as u8
    }

    fn at(&self, x: f32) -> f32 {
        let i = (x * 8.0).clamp(0.0, 8.0);
        let low = i.floor() as usize;
        let frac = i - low as f32;
        let a = self.points.get(low).copied().unwrap_or(0.0);
        let b = self.points.get((low + 1).min(8)).copied().unwrap_or(127.0);
        (a + (b - a) * frac) / 127.0
    }
}

/// What one instrument's console remembers between runs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ConsoleState {
    pub model: usize,
    pub variant: usize,
    pub velocity: VelocityCurve,
    pub polyphony: usize,
    pub sustain: bool,
    /// Values by parameter id; ids missing here use the spec's default.
    pub values: std::collections::BTreeMap<String, f32>,
    #[serde(default = "default_true")]
    pub midi_out: bool,
    pub channel: u8,
    /// Name being entered in the captured-sound browser.
    pub capture_name: String,
    /// Captured sound selected for load/delete.
    pub selected_capture: String,
}

fn default_true() -> bool {
    true
}

impl Default for ConsoleState {
    fn default() -> Self {
        ConsoleState {
            model: 0,
            variant: 0,
            velocity: VelocityCurve::default(),
            polyphony: 128,
            sustain: false,
            values: std::collections::BTreeMap::new(),
            midi_out: true,
            channel: 0,
            capture_name: String::new(),
            selected_capture: String::new(),
        }
    }
}

impl ConsoleState {
    /// A value in `profile` (its default when the control was never touched).
    pub fn value(&self, profile: &Profile, id: &str) -> f32 {
        self.values.get(id).copied().or_else(|| profile.spec(id).map(|s| s.default)).unwrap_or(0.0)
    }

    /// Records a value, clamped into the profile's range for it (a choice keeps its option index).
    pub fn set(&mut self, profile: &Profile, id: &str, value: f32) {
        if let Some(s) = profile.spec(id) {
            self.values.insert(id.to_string(), value.clamp(s.min, s.max));
        } else if let Some(sel) = profile.select(id) {
            let last = sel.options.len().saturating_sub(1) as f32;
            self.values.insert(id.to_string(), value.clamp(0.0, last));
        }
    }
}

/// One instrument's state (created on first use, so a profile added later just works).
pub fn state<'a>(app: &'a SoundApp, profile: &Profile) -> &'a ConsoleState {
    static EMPTY: std::sync::OnceLock<ConsoleState> = std::sync::OnceLock::new();
    app.ui.consoles.get(profile.id).unwrap_or_else(|| EMPTY.get_or_init(ConsoleState::default))
}

/// One instrument's state, mutable.
pub fn state_mut<'a>(app: &'a mut SoundApp, profile: &'a Profile) -> &'a mut ConsoleState {
    app.ui.consoles.entry(profile.id.to_string()).or_default()
}

// ---- dispatch hooks ---------------------------------------------------------------------------

fn param_value(app: &SoundApp, profile: &Profile, id: &str) -> f32 {
    state(app, profile).value(profile, id)
}

/// The CC a parameter sends for a value.
pub fn cc_message(spec: &Spec, channel: u8, value: f32) -> [u8; 3] {
    let span = spec.max - spec.min;
    let norm = if span.abs() > f32::EPSILON { ((value - spec.min) / span).clamp(0.0, 1.0) } else { 0.0 };
    let v = (norm * 127.0).round().clamp(0.0, 127.0) as u8;
    [0xB0 | (channel & 0x0F), spec.cc, v]
}

fn record_sent(app: &mut SoundApp, msg: [u8; 3]) {
    let at = app.last_frame.unwrap_or_default();
    app.ui.console_sent = Some((msg, at));
}

/// Dispatches one continuous parameter: stores it, sends it as a CC to the active instrument track
/// and records it (for the activity light). A bridge (WebMIDI, WebSocket, OSC, JUCE) replaces this
/// body and everything in the console keeps working.
pub fn dispatch(app: &mut SoundApp, profile: &'static Profile, id: &str, value: f32) {
    let Some(spec) = profile.spec(id) else { return };
    state_mut(app, profile).set(profile, id, value);

    // CORREÇÃO (Seção 4): Atualiza o parâmetro diretamente no engine do plugin.
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let _ = app.run("mix.instrument_param", serde_json::json!({"track": track.0, "param": id, "value": value}));

    // Se MIDI CC estiver ativado, envia também o control change para hardware externo.
    if !state(app, profile).midi_out {
        return;
    }
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let channel = state(app, profile).channel;
    let msg = cc_message(spec, channel, state(app, profile).value(profile, id));
    app.send_live_midi(track, msg);
    record_sent(app, msg);
}

/// Dispatches one discrete choice: stores the option index and sends it as its CC.
pub fn select(app: &mut SoundApp, profile: &'static Profile, id: &str, index: usize) {
    let Some(sel) = profile.select(id) else { return };
    let option = index.min(sel.options.len().saturating_sub(1));
    state_mut(app, profile).set(profile, id, option as f32);

    // CORREÇÃO (Seção 4): Atualiza o parâmetro diretamente no engine do plugin.
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let _ = app.run("mix.instrument_param", serde_json::json!({"track": track.0, "param": id, "value": option as f32}));

    // Se MIDI CC estiver ativado, envia também o control change para hardware externo.
    if !state(app, profile).midi_out {
        return;
    }
    // If console is open but not for the current instrument, don't send MIDI
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let channel = state(app, profile).channel;
    let msg = [0xB0 | (channel & 0x0F), sel.cc, option as u8 & 0x7F];
    app.send_live_midi(track, msg);
    record_sent(app, msg);
}

/// One note from a console's keyboard: velocity through the curve, then onto the track.
pub fn play_note(app: &mut SoundApp, profile: &'static Profile, note: u8, velocity: u8, on: bool) {
    // Check if console is open, but if it's not for the active instrument, don't send MIDI
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let curve = state(app, profile).velocity.clone();
    let velocity = if on { curve.apply(velocity.max(1)) } else { 0 };
    let msg = [if on { 0x90 } else { 0x80 } | (state(app, profile).channel & 0x0F), note & 0x7F, velocity & 0x7F];
    app.send_live_midi(track, msg);
    app.light_keys(msg);
    if on {
        app.keyboard.held_notes.insert(note);
        app.keyboard.typed_notes.insert(note);
    } else {
        app.keyboard.held_notes.remove(&note);
        app.keyboard.typed_notes.remove(&note);
    }
    record_sent(app, msg);
}

/// The sustain pedal (CC 64) as a latching toggle.
pub fn set_sustain(app: &mut SoundApp, profile: &'static Profile, on: bool) {
    state_mut(app, profile).sustain = on;
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let msg = [0xB0 | (state(app, profile).channel & 0x0F), 64, if on { 127 } else { 0 }];
    app.send_live_midi(track, msg);
    record_sent(app, msg);
}

// ---- window -----------------------------------------------------------------------------------

/// The instrument the console shows (kept in the UI state so it survives a restart).
pub fn active(app: &SoundApp) -> &'static Profile {
    profile(&app.ui.console_profile).unwrap_or(&crate::pianoteq_console::PROFILE)
}

/// Opens the console on one instrument (used by the menu commands and the toolbar).
/// The console that drives a hosted plugin, from the plugin's own name ("Pianoteq 9", "Kontakt 7",
/// "Analog Lab V", "Omnisphere", "Pigments", "LABS") or its id. This is what the track strip's
/// ALT button uses: pick the instrument on the track, and the console knows which VST it is on.
pub fn profile_for_plugin(plugin: &str) -> Option<&'static Profile> {
    use crate::mix_window::plugin_info;
    // The class id is the reliable key (a session stores it, not the plugin's name).
    if let Some(p) = profiles().iter().copied().find(|p| p.plugin_ids.contains(&plugin)) {
        return Some(p);
    }
    let mut names: Vec<String> = vec![plugin.to_string()];
    if let Some(info) = plugin_info(plugin) {
        names.push(info.name.to_string());
        names.push(info.id.to_string());
    }
    let haystack = names.join(" ").to_ascii_lowercase();
    profiles().iter().copied().find(|p| haystack.contains(&p.name.to_ascii_lowercase()) || haystack.contains(&p.brand.to_ascii_lowercase()))
}

/// Where an instrument's own presets are remembered: `presets/<profile>.json`, a map from the
/// instrument's name (or [`Model::preset`] key) to a base64 plugin state. It is the half of the
/// instrument→preset map that works for VSTs which cannot switch programs by themselves.
pub fn preset_file(profile: &str) -> Option<std::path::PathBuf> {
    let dir = settings_dir()?;
    Some(dir.join("presets").join(format!("{profile}.json")))
}

/// The saved preset states of one console: instrument key → base64 state.
pub fn saved_presets(profile: &str) -> std::collections::BTreeMap<String, String> {
    preset_file(profile)
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

const CAPTURE_PREFIX: &str = "capture:";
const MAX_CAPTURE_NAME_CHARS: usize = 128;

fn capture_key(name: &str) -> String {
    format!("{CAPTURE_PREFIX}{}", name.trim())
}

fn captured_track(app: &SoundApp, profile: &'static Profile) -> Result<soundcraft_model::TrackId, String> {
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else {
        return Err("select the instrument track first".into());
    };
    let plugin = app.engine.session().track(track).and_then(|track| track.instrument.as_ref()).map(|instrument| instrument.plugin.as_str());
    if plugin.and_then(profile_for_plugin).is_none_or(|track_profile| track_profile.id != profile.id) {
        return Err(format!("the active track is not loaded with {}", profile.brand));
    }
    Ok(track)
}

/// Names of sounds captured from this console's live plugin.
pub fn captured_presets(profile: &str) -> Vec<String> {
    capture_names(&saved_presets(profile))
}

fn capture_names(map: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    map.keys().filter_map(|key| key.strip_prefix(CAPTURE_PREFIX).map(str::to_string)).collect()
}

pub fn write_saved_presets(profile: &str, map: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let path = preset_file(profile).ok_or_else(|| "no settings directory".to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The key an instrument is remembered under.
pub fn preset_key(model: &Model) -> &'static str {
    model.preset.unwrap_or(model.name)
}

/// Binds the plugin's **current** state to this instrument: pick the sound in the plugin's own
/// window, then save it here, and selecting the instrument in the console restores it.
pub fn save_preset(app: &mut SoundApp, profile: &'static Profile, model: &Model) -> Result<(), String> {
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else {
        return Err("select the instrument track first".into());
    };
    let Some(player) = app.player.as_ref() else { return Err("no audio engine".into()) };
    let state =
        player.capture_plugin_state(track, soundcraft_mix::INSTRUMENT_SLOT).ok_or_else(|| format!("`{}` did not hand over its state", model.name))?;
    let mut map = saved_presets(profile.id);
    map.insert(preset_key(model).to_string(), soundcraft_engine::model::b64::encode(&state));
    write_saved_presets(profile.id, &map)?;
    log::info!("preset saved: {} → {} ({} bytes)", profile.id, preset_key(model), state.len());
    Ok(())
}

/// Saves the live plugin state under a user-provided name for the captured-sound browser.
pub fn save_captured_preset(app: &SoundApp, profile: &'static Profile, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("enter a name for this sound".into());
    }
    if name.chars().count() > MAX_CAPTURE_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(format!("sound names must be at most {MAX_CAPTURE_NAME_CHARS} characters and contain no control characters"));
    }
    let track = captured_track(app, profile)?;
    let Some(player) = app.player.as_ref() else { return Err("no audio engine".into()) };
    let state = player
        .capture_plugin_state(track, soundcraft_mix::INSTRUMENT_SLOT)
        .ok_or_else(|| "the instrument did not hand over its state".to_string())?;
    let key = capture_key(name);
    let mut map = saved_presets(profile.id);
    map.insert(key, soundcraft_engine::model::b64::encode(&state));
    write_saved_presets(profile.id, &map)?;
    log::info!("captured preset saved: {} → {name} ({} bytes)", profile.id, state.len());
    Ok(())
}

/// Loads a named sound captured from this console's live plugin.
pub fn apply_captured_preset(app: &SoundApp, profile: &'static Profile, name: &str) -> Result<(), String> {
    let name = name.trim();
    let map = saved_presets(profile.id);
    let encoded = map.get(&capture_key(name)).ok_or_else(|| format!("captured sound `{name}` was not found"))?;
    let state = soundcraft_engine::model::b64::decode(encoded).ok_or_else(|| format!("captured sound `{name}` has invalid state data"))?;
    let track = captured_track(app, profile)?;
    let Some(player) = app.player.as_ref() else { return Err("no audio engine".into()) };
    player.load_plugin_state(track, soundcraft_mix::INSTRUMENT_SLOT, state);
    Ok(())
}

/// Removes one named captured sound without touching instrument-bound presets.
pub fn delete_captured_preset(profile: &'static Profile, name: &str) -> Result<(), String> {
    let key = capture_key(name);
    let mut map = saved_presets(profile.id);
    if map.remove(&key).is_none() {
        return Err(format!("captured sound `{}` was not found", name.trim()));
    }
    write_saved_presets(profile.id, &map)
}

/// Applies the instrument's preset to the plugin on the active track: the plugin's own program
/// list first (IUnitInfo), then a state saved from that instrument, and the caller falls back to a
/// MIDI program change. Returns what happened, for the status line.
pub fn apply_preset(app: &SoundApp, profile: &'static Profile, model: &Model) -> (bool, String) {
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return (false, "no instrument track selected".into()) };
    let Some(player) = app.player.as_ref() else { return (false, "no audio engine".into()) };
    let slot = soundcraft_mix::INSTRUMENT_SLOT;
    let key = preset_key(model).to_ascii_lowercase();
    // 1. The plugin's own program list.
    let lists = player.plugin_programs(track, slot);
    for (li, list) in lists.iter().enumerate() {
        if let Some((pi, name)) = list.programs.iter().enumerate().find(|(_, n)| n.to_ascii_lowercase().contains(&key)) {
            player.select_plugin_program(track, slot, li, pi);
            return (true, format!("{} → preset `{name}`", model.name));
        }
    }
    // 2. A state saved from this instrument.
    if let Some(state) = saved_presets(profile.id).get(preset_key(model)).and_then(|b| soundcraft_engine::model::b64::decode(b)) {
        player.load_plugin_state(track, slot, state);
        return (true, format!("{} → saved preset", model.name));
    }
    // 3. Direct program index if published by the plugin (e.g. Pianoteq "Program 0..127").
    if let Some((li, list)) = lists.iter().enumerate().find(|(_, l)| (model.program as usize) < l.programs.len()) {
        player.select_plugin_program(track, slot, li, model.program as usize);
        let prog_name = list.programs.get(model.program as usize).map(|s| s.as_str()).unwrap_or("");
        return (true, format!("{} → program {} ({prog_name})", model.name, model.program));
    }
    let what =
        if lists.is_empty() { "the plugin publishes no program list".to_string() } else { format!("no program matches `{}`", preset_key(model)) };
    (false, format!("{}: {what}", model.name))
}

/// Opens the console on the instrument loaded in `track` (and makes that track the one the console
/// plays into, so its notes and program changes reach this VST).
pub fn open_for_track(app: &mut SoundApp, track: soundcraft_model::TrackId) {
    let plugin = app.engine.session().track(track).and_then(|t| t.instrument.as_ref()).map(|i| i.plugin.clone());
    let id = plugin.as_deref().and_then(profile_for_plugin).map(|p| p.id).unwrap_or("pianoteq");
    let edit = &mut app.engine.session_mut().edit;
    edit.selected_tracks.clear();
    edit.selected_tracks.push(track);
    open(app, id);
}

pub fn open(app: &mut SoundApp, id: &str) {
    // When switching instruments, don't re-open the console window
    // to prevent interference with the plugin state
    if app.ui.show_console {
        app.ui.console_profile = id.to_string();
    } else {
        app.ui.console_profile = id.to_string();
        app.ui.show_console = true;
    }
}

/// Draws the console window when it is open. Switching the instrument tab changes the racks, the
/// palette and the window title; the window keeps its place (it is identified by a fixed id).
pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_console {
        return;
    }
    let profile = active(app);
    let mut open = true;
    egui::Window::new(profile.window_title())
        .id(egui::Id::new("instrument_console"))
        .open(&mut open)
        .default_size(vec2(1180.0, 780.0))
        .min_size(vec2(900.0, 560.0))
        .resizable(true)
        .show(ctx, |ui| {
            let palette = active(app).palette;
            egui::Frame::new().fill(palette.bg).inner_margin(10.0).show(ui, |ui| {
                instrument_tabs(app, ui);
                ui.add_space(6.0);
                body(app, ui, active(app));
            });
        });
    app.ui.show_console = open;
}

/// The instrument chooser: every profile as a tab in its own accent colour.
fn instrument_tabs(app: &mut SoundApp, ui: &mut Ui) {
    let palette = active(app).palette;
    let current = app.ui.console_profile.clone();
    egui::Frame::new().fill(palette.panel).stroke(Stroke::new(1.0, palette.line)).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("INSTRUMENT").font(bold(8.5)).color(palette.text_dim));
            for profile in profiles() {
                let on = profile.id == current.as_str();
                // The tab carries the VST's name, and a dot when it is the instrument loaded on the
                // active track (so the tab row answers "which VST am I driving?").
                let loaded_here = active_instrument_matches(app, profile);
                let label = if loaded_here { format!("● {}", profile.name) } else { profile.name.to_string() };
                let text =
                    RichText::new(label).font(if on { bold(10.0) } else { regular(10.0) }).color(if on { Color32::BLACK } else { palette.text });
                let button = egui::Button::new(text)
                    .fill(if on { profile.palette.accent } else { palette.panel_head })
                    .stroke(Stroke::new(1.0, if on { profile.palette.accent } else { palette.line }))
                    .corner_radius(3.0);
                // The VST's own pictogram, left of its name.
                let resp = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (glyph, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        profile.icon.paint(ui.painter(), glyph.shrink(1.0), if on { Color32::BLACK } else { profile.palette.accent });
                        ui.add(button)
                    })
                    .inner;
                if resp.on_hover_text(format!("{} — {} · {}", profile.brand, profile.kind, profile.tagline)).clicked() && !on {
                    app.ui.console_profile = profile.id.to_string();
                    // Prefer to load the plugin on the currently selected instrument track if Alt GUI is open.
                    // If no track is selected (unlikely in Alt context), fall back to searching for a matching track.
                    let mut target_track_opt = app.engine.session().edit.selected_tracks.first().cloned();
                    if target_track_opt.is_none() {
                        target_track_opt = app.engine.session().tracks.iter()
                            .find(|t| {
                                t.kind.is_midi() && !t.hidden && t.instrument.as_ref().is_some_and(|ins| {
                                    profile_for_plugin(&ins.plugin).is_some_and(|p| p.id == profile.id)
                                })
                            })
                            .map(|t| t.id);
                    }
                    if let Some(track_id) = target_track_opt {
                        // Switch the plugin on this track so the console controls actually drive it
                        let _ = app.run("instrument.load", serde_json::json!({"track": track_id.0, "plugin_id": &profile.id}));
                        let _ = app.run("edit.select", serde_json::json!({"tracks": [track_id.0]}));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let profile = active(app);
                ui.label(RichText::new(format!("{} · {}", profile.kind, profile.id)).font(regular(8.5)).color(palette.text_dim));
            });
        });
    });
}

/// True when the instrument on the active track is this profile (matched by the plugin's name or
/// id — "Pianoteq 9", "vst3:…Pianoteq…", "Pigments").
fn active_instrument_matches(app: &SoundApp, profile: &Profile) -> bool {
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return false };
    app.engine
        .session()
        .track(track)
        .and_then(|t| t.instrument.as_ref())
        .and_then(|instrument| profile_for_plugin(&instrument.plugin))
        .is_some_and(|loaded| loaded.id == profile.id)
}

fn body(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        header(app, ui, profile);
        ui.add_space(8.0);
        let half = profile.racks.len().div_ceil(2);
        let left: Vec<usize> = (0..half.min(profile.racks.len())).collect();
        let right: Vec<usize> = (half.min(profile.racks.len())..profile.racks.len()).collect();
        ui.columns(2, |cols| {
            for index in left {
                if let Some(title) = profile.racks.get(index) {
                    let column = &mut cols[0];
                    rack(column, &profile.palette, title, |ui| rack_body(app, ui, profile, index as u8));
                    column.add_space(4.0);
                }
            }
            for index in right {
                if let Some(title) = profile.racks.get(index) {
                    let column = &mut cols[1];
                    rack(column, &profile.palette, title, |ui| rack_body(app, ui, profile, index as u8));
                    column.add_space(4.0);
                }
            }
        });
        ui.add_space(6.0);
        dynamics_row(app, ui, profile);
        ui.add_space(8.0);
        footer(app, ui, profile);
    });
}

fn rack_body(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile, group: u8) {
    let p = profile.palette;
    for sel in profile.selects.iter().filter(|s| s.group == group) {
        egui::Frame::new().fill(p.panel_head).corner_radius(3.0).inner_margin(5.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(sel.label).font(bold(8.5)).color(p.text_dim));
                let current = param_value(app, profile, sel.id).round() as usize;
                for (i, name) in sel.options.iter().enumerate() {
                    let on = current == i;
                    let text =
                        RichText::new(*name).font(if on { bold(9.0) } else { regular(9.0) }).color(if on { Color32::BLACK } else { p.text_dim });
                    let button = egui::Button::new(text).fill(if on { p.accent } else { p.panel }).stroke(Stroke::new(1.0, p.line));
                    if ui.add(button).clicked() && !on {
                        select(app, profile, sel.id, i);
                    }
                }
            });
        });
        ui.add_space(4.0);
    }
    ui.horizontal_wrapped(|ui| {
        for spec in profile.specs.iter().filter(|s| s.group == group) {
            knob(app, ui, profile, spec);
        }
    });
    if let Some((picker_group, id)) = profile.library_picker
        && picker_group == group
    {
        library_picker(app, ui, profile, id);
    }
}

/// The sampler's library chooser: a filter box over the installed libraries, the current one lit.
fn library_picker(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile, id: &'static str) {
    let p = profile.palette;
    let list = libraries();
    let selected = param_value(app, profile, id).round() as usize;
    let search_id = ui.id().with(("library_search", profile.id));
    let mut search: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    egui::Frame::new().fill(p.panel_head).corner_radius(3.0).inner_margin(5.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LIBRARY").font(bold(8.5)).color(p.text_dim));
            if ui.small_button("Rescan").on_hover_text("Re-read the installed libraries from the Windows drive").clicked() {
                rescan_libraries(app);
            }
            let current = list.get(selected).map(|l| l.name.clone()).unwrap_or_else(|| "— none —".to_string());
            ui.label(RichText::new(current).font(bold(9.5)).color(p.accent));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("search").font(regular(8.5)).color(p.text_dim));
            ui.add(egui::TextEdit::singleline(&mut search).desired_width(150.0).hint_text("filter libraries"));
        });
        let needle = search.to_ascii_lowercase();
        let mut shown = 0usize;
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            if list.is_empty() {
                ui.label(RichText::new("no libraries yet — press Rescan (scripts/kontakt-libraries.py)").font(regular(9.0)).color(p.text_dim));
            }
            for (index, library) in list.iter().enumerate() {
                if !needle.is_empty() && !library.name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                shown += 1;
                let on = index == selected;
                let text =
                    RichText::new(&library.name).font(if on { bold(9.5) } else { regular(9.5) }).color(if on { Color32::BLACK } else { p.text });
                // The library's own pictogram, in the same row as its name.
                let clicked = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (glyph, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        library.icon().paint(ui.painter(), glyph.shrink(1.0), if on { Color32::BLACK } else { p.accent_dim });
                        ui.add(
                            egui::Button::new(text)
                                .fill(if on { p.accent } else { p.panel })
                                .stroke(Stroke::new(1.0, p.line))
                                .min_size(vec2(ui.available_width() - 18.0, 0.0)),
                        )
                    })
                    .inner
                    .clicked();
                if clicked && !on {
                    choose_library(app, profile, id, index);
                }
            }
        });
        if shown == 0 && !needle.is_empty() {
            ui.label(RichText::new("nothing matches that filter").font(regular(9.0)).color(p.text_dim));
        }
    });
    ui.data_mut(|d| d.insert_temp(search_id, search));
}

/// Selects a library: stores the index and dispatches it to the sampler as Bank Select + Program
/// Change (the standard way to address the first 128 programs of the 128th bank).
pub fn choose_library(app: &mut SoundApp, profile: &'static Profile, id: &'static str, index: usize) {
    let list = libraries();
    if index >= list.len() {
        return;
    }
    // A library index is not a mapped parameter: it is stored as-is (the picker clamps on use).
    state_mut(app, profile).values.insert(id.to_string(), index as f32);
    if !state(app, profile).midi_out {
        return;
    }
    let Some(track) = crate::virtual_keyboard::active_target_track(app) else { return };
    let channel = state(app, profile).channel & 0x0F;
    let bank = (index / 128) as u8 & 0x7F;
    let program = (index % 128) as u8 & 0x7F;
    app.send_live_midi(track, [0xB0 | channel, 0x00, bank]);
    let msg = [0xC0 | channel, program, 0];
    app.send_live_midi(track, msg);
    record_sent(app, msg);
}

/// Re-runs the library scan in the background and reloads the list when it finishes.
fn rescan_libraries(app: &mut SoundApp) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/kontakt-libraries.py");
    if !script.exists() {
        log::warn!("library scan script not found at {}", script.display());
        return;
    }
    let Some(out) = library_file() else { return };
    let spawned = std::process::Command::new("python3")
        .arg(&script)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            // Detached: the list is re-read on the next frame the picker is drawn.
            std::thread::spawn(move || {
                let _ = child.wait();
                if let Ok(mut cache) = LIBRARIES.lock() {
                    *cache = None;
                }
            });
            log::info!("library scan started");
        }
        Err(e) => log::warn!("cannot start the library scan: {e}"),
    }
    let _ = app;
}

fn header(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            // Pictogram of the selected instrument, so the window reads at a glance.
            let (glyph, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            let icon = profile.models.get(state(app, profile).model).map(|m| m.icon).unwrap_or(Icon::Keys);
            icon.paint(ui.painter(), glyph.shrink(1.0), p.accent);
            ui.add_space(2.0);
            ui.label(RichText::new(profile.brand).font(bold(15.0)).color(p.accent));
            ui.label(RichText::new(profile.kind).font(regular(12.0)).color(p.text_dim));
            ui.label(RichText::new(profile.tagline).font(regular(9.5)).color(p.text_dim));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let midi_out = state(app, profile).midi_out;
                let label = if midi_out { "MIDI OUT: ON" } else { "MIDI OUT: OFF" };
                if ui
                    .add(egui::Button::new(RichText::new(label).font(bold(9.5)).color(if midi_out { p.live } else { p.text_dim })).frame(true))
                    .on_hover_text("Send every control as a MIDI CC to the active instrument track")
                    .clicked()
                {
                    state_mut(app, profile).midi_out = !midi_out;
                }
                ui.label(RichText::new("CH").font(regular(9.0)).color(p.text_dim));
                let mut ch = u32::from(state(app, profile).channel) + 1;
                if ui.add(egui::DragValue::new(&mut ch).range(1..=16).speed(0.1)).changed() {
                    state_mut(app, profile).channel = (ch.clamp(1, 16) - 1) as u8;
                }
                for spec in profile.master {
                    knob(app, ui, profile, spec);
                }
            });
        });
        if !profile.models.is_empty() {
            ui.add_space(6.0);
            model_tabs(app, ui, profile);
        }
        if let Some((_, id)) = profile.library_picker {
            let list = libraries();
            let index = param_value(app, profile, id).round() as usize;
            let name = list.get(index).map(|l| l.name.as_str()).unwrap_or("no library selected");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("LIBRARY").font(bold(8.5)).color(p.text_dim));
                ui.label(RichText::new(name).font(bold(10.0)).color(p.accent));
                if let Some(lib) = list.get(index)
                    && !lib.path.is_empty()
                {
                    ui.label(RichText::new(&lib.path).font(regular(8.0)).color(p.text_dim));
                }
            });
        }
        if !profile.variants.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("VARIANT").font(bold(9.0)).color(p.text_dim));
                let mut variant = state(app, profile).variant;
                egui::ComboBox::from_id_salt(("variant", profile.id))
                    .selected_text(profile.variants.get(variant).copied().unwrap_or("—"))
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for (i, name) in profile.variants.iter().enumerate() {
                            ui.selectable_value(&mut variant, i, *name);
                        }
                    });
                state_mut(app, profile).variant = variant;
            });
        }
    });
}

#[allow(dead_code)]
fn captured_preset_browser(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    let mut capture_name = state(app, profile).capture_name.clone();
    let mut selected = state(app, profile).selected_capture.clone();
    let names = captured_presets(profile.id);

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("CAPTURED SOUNDS").font(bold(8.5)).color(p.text_dim));
        ui.add_sized(
            vec2(180.0, 22.0),
            egui::TextEdit::singleline(&mut capture_name).char_limit(MAX_CAPTURE_NAME_CHARS).hint_text(format!("Name shown in {}", profile.brand)),
        );
        if ui
            .add_enabled(!capture_name.trim().is_empty(), egui::Button::new("SAVE CURRENT").frame(true))
            .on_hover_text(format!("Choose a sound in the {} window, then capture its live plugin state here", profile.brand))
            .clicked()
        {
            app.ui.status = match save_captured_preset(app, profile, &capture_name) {
                Ok(()) => {
                    selected = capture_name.trim().to_string();
                    format!("{} sound captured: {selected}", profile.brand)
                }
                Err(error) => format!("sound not captured: {error}"),
            };
        }
        let shown = if selected.is_empty() { "Choose a captured sound" } else { &selected };
        egui::ComboBox::from_id_salt(("captured_sounds", profile.id)).selected_text(shown).width(210.0).show_ui(ui, |ui| {
            for name in &names {
                ui.selectable_value(&mut selected, name.clone(), name);
            }
        });
        if ui.add_enabled(names.iter().any(|name| name == &selected), egui::Button::new("LOAD").frame(true)).clicked() {
            app.ui.status = match apply_captured_preset(app, profile, &selected) {
                Ok(()) => format!("{} sound loaded: {selected}", profile.brand),
                Err(error) => format!("sound not loaded: {error}"),
            };
        }
        if ui.add_enabled(names.iter().any(|name| name == &selected), egui::Button::new("DELETE").frame(true)).clicked() {
            app.ui.status = match delete_captured_preset(profile, &selected) {
                Ok(()) => {
                    if capture_name == selected {
                        capture_name.clear();
                    }
                    selected.clear();
                    format!("Captured {} sound deleted", profile.brand)
                }
                Err(error) => format!("sound not deleted: {error}"),
            };
        }
    });

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Choose a sound in {}, enter its name, and save it here for one-click recall.", profile.brand))
                .font(regular(8.5))
                .color(p.text_dim),
        );
        ui.label(RichText::new(format!("{} captured", names.len())).font(regular(8.5)).color(p.text_dim));
    });
    let console = state_mut(app, profile);
    console.capture_name = capture_name;
    console.selected_capture = selected;
}

fn model_tabs(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    let selected = state(app, profile).model;
    ui.horizontal_wrapped(|ui| {
        for (category, cat) in profile.categories.iter().enumerate() {
            let models: Vec<(usize, &'static Model)> = profile.models.iter().enumerate().filter(|(_, m)| m.category == category).collect();
            if models.is_empty() {
                continue;
            }
            // The column is as wide as its longest instrument name, so nothing is clipped.
            let width = models
                .iter()
                .map(|(_, m)| ui.ctx().fonts_mut(|f| f.layout_no_wrap(m.name.to_string(), regular(9.5), p.text).size().x))
                .fold(0.0_f32, f32::max)
                + 26.0;
            let width = width.clamp(70.0, 200.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(cat.title).font(bold(8.5)).color(p.text_dim));
                for (i, model) in models {
                    let on = selected == i;
                    // One button per instrument: pictogram, then its name.
                    let fill = if on { p.accent } else { p.panel_head };
                    let ink = if on { Color32::BLACK } else { p.text };
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    ui.painter().rect_filled(rect, 3.0, if resp.hovered() && !on { p.panel_head.gamma_multiply(1.35) } else { fill });
                    if on {
                        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
                    }
                    // Bound = picking this instrument restores a saved sound by itself.
                    let bound = saved_presets(profile.id).contains_key(preset_key(model));
                    if bound {
                        let dot = pos2(rect.max.x - 6.0, rect.max.y - 5.0);
                        ui.painter().circle_filled(dot, 2.2, if on { Color32::BLACK } else { p.live });
                    }
                    let glyph = Rect::from_min_size(rect.min + vec2(3.0, 2.0), vec2(14.0, 14.0));
                    model.icon.paint(ui.painter(), glyph, if on { Color32::BLACK } else { p.accent });
                    ui.painter().text(rect.min + vec2(20.0, 9.0), Align2::LEFT_CENTER, model.name, if on { bold(9.5) } else { regular(9.5) }, ink);
                    let info_label = model.name;
                    let info_selected = on;
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, info_selected, info_label));
                    let resp = resp.on_hover_text(format!("{} — program {} (click or double-click to load)", model.name, model.program));
                    if resp.clicked() || resp.double_clicked() {
                        state_mut(app, profile).model = i;
                        let (applied, status) = apply_preset(app, profile, model);
                        app.ui.status = if applied {
                            status
                        } else {
                            if let Some(track) = crate::virtual_keyboard::active_target_track(app) {
                                if profile.program_change {
                                    let msg = [0xC0 | (state(app, profile).channel & 0x0F), model.program, 0];
                                    app.send_live_midi(track, msg);
                                    record_sent(app, msg);
                                    format!("Loaded {} (program {})", model.name, model.program)
                                } else {
                                    crate::mix_window::open_native_gui(app, track, soundcraft_mix::INSTRUMENT_SLOT);
                                    format!("{status}; opened the plugin to pick the sound")
                                }
                            } else {
                                format!("{status}; no {} track is loaded", profile.brand)
                            }
                        };
                    }
                }
            });
            ui.add_space(10.0);
        }
    });
}

fn rack(ui: &mut Ui, p: &Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            let r = Rect::from_min_size(ui.cursor().min, vec2(3.0, 12.0));
            ui.painter().rect_filled(r, 1.0, p.accent);
            ui.add_space(7.0);
            ui.label(RichText::new(title).font(bold(10.0)).color(p.text));
        });
        ui.add_space(4.0);
        add(ui);
    });
}

// ---- controls ---------------------------------------------------------------------------------

/// A precision rotary: arc meter, pointer, value, double-click resets to the default.
fn knob(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile, spec: &'static Spec) {
    let p = profile.palette;
    let (rect, resp) = ui.allocate_exact_size(vec2(58.0, 74.0), Sense::click_and_drag());
    let value = param_value(app, profile, spec.id);
    let mut next = value;
    if resp.dragged() {
        let delta = -resp.drag_delta().y / 160.0 * (spec.max - spec.min);
        let fine = ui.input(|i| i.modifiers.shift);
        next = value + if fine { delta * 0.1 } else { delta };
        if spec.step > 0.0 {
            next = (next / spec.step).round() * spec.step;
        }
    }
    if resp.double_clicked() {
        next = spec.default;
    }
    if next != value {
        dispatch(app, profile, spec.id, next);
    }
    let value = param_value(app, profile, spec.id);

    let painter = ui.painter_at(rect);
    let radius = 18.0;
    let center = pos2(rect.center().x, rect.top() + radius + 4.0);
    let norm = ((value - spec.min) / (spec.max - spec.min).max(f32::EPSILON)).clamp(0.0, 1.0);
    let arc = |fraction: f32| -> [Pos2; 24] {
        let start = std::f32::consts::PI * 0.75;
        let sweep = std::f32::consts::PI * 1.5 * fraction;
        let mut pts = [Pos2::ZERO; 24];
        for (i, slot) in pts.iter_mut().enumerate() {
            let t = sweep * (i as f32 / 23.0);
            *slot = center + vec2((start + t).cos(), (start + t).sin()) * (radius + 4.0);
        }
        pts
    };
    let hovered = resp.hovered();
    painter.add(egui::Shape::line(arc(1.0).to_vec(), Stroke::new(3.0, p.line)));
    painter.add(egui::Shape::line(arc(norm).to_vec(), Stroke::new(3.0, if hovered { p.accent } else { p.accent_dim })));
    painter.circle_filled(center, radius, p.panel_head);
    painter.circle_stroke(center, radius, Stroke::new(1.0, p.edge));
    let angle = std::f32::consts::PI * 0.75 + std::f32::consts::PI * 1.5 * norm;
    let tip = center + vec2(angle.cos(), angle.sin()) * (radius - 4.0);
    painter.line_segment([center, tip], Stroke::new(2.0, p.accent));
    painter.text(pos2(rect.center().x, rect.bottom() - 16.0), egui::Align2::CENTER_CENTER, spec.label, FontId::proportional(8.5), p.text_dim);
    let readout = format!("{} {}", fmt_value(value, spec.decimals), spec.unit);
    painter.text(pos2(rect.center().x, rect.bottom() - 5.0), egui::Align2::CENTER_CENTER, readout, bold(9.0), p.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, spec.label));
    resp.on_hover_text(format!("{}\n{} {} … {}\nMIDI CC {}", spec.automation, spec.min, spec.unit, spec.max, spec.cc));
}

fn fmt_value(value: f32, decimals: u8) -> String {
    match decimals {
        0 => format!("{value:.0}"),
        1 => format!("{value:.1}"),
        _ => format!("{value:.2}"),
    }
}

fn hfader(ui: &mut Ui, p: &Palette, norm: f32, width: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    let mut next = None;
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        next = Some(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    }
    let painter = ui.painter_at(rect);
    let slot = Rect::from_center_size(rect.center(), vec2(rect.width() - 8.0, 4.0));
    painter.rect_filled(slot, 2.0, p.bg);
    painter.rect_stroke(slot, 2.0, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    let x = slot.left() + slot.width() * norm.clamp(0.0, 1.0);
    painter.rect_filled(Rect::from_min_max(pos2(slot.left(), slot.top()), pos2(x, slot.bottom())), 2.0, p.accent_dim);
    let cap = Rect::from_center_size(pos2(x, rect.center().y), vec2(6.0, 16.0));
    painter.rect_filled(cap, 2.0, if resp.hovered() { p.accent } else { p.edge });
    painter.rect_stroke(cap, 2.0, Stroke::new(1.0, p.edge), egui::StrokeKind::Inside);
    next
}

fn switch(ui: &mut Ui, p: &Palette, on: &mut bool, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let painter = ui.painter_at(rect);
    let track = Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), vec2(38.0, 14.0));
    painter.rect_filled(track, 7.0, if *on { p.accent_dim } else { p.bg });
    painter.rect_stroke(track, 7.0, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    let knob_x = if *on { track.right() - 7.0 } else { track.left() + 7.0 };
    painter.circle_filled(pos2(knob_x, track.center().y), 5.0, if *on { p.accent } else { p.text_dim });
    painter.text(pos2(rect.left() + 44.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(9.0), p.text_dim);
    resp
}

// ---- dynamics ---------------------------------------------------------------------------------

fn dynamics_row(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    if !profile.velocity_curve {
        return;
    }
    let p = profile.palette;
    egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("DYNAMIC RESPONSE").font(bold(10.0)).color(p.text));
                ui.label(RichText::new("velocity curve").font(regular(8.5)).color(p.text_dim));
                ui.add_space(4.0);
                for (i, name) in ["Line", "Exp", "Log", "S-Curve"].iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(*name).font(regular(9.0)).color(p.text))
                                .fill(p.panel_head)
                                .stroke(Stroke::new(1.0, p.line)),
                        )
                        .clicked()
                    {
                        state_mut(app, profile).velocity = VelocityCurve::preset(i);
                    }
                }
            });
            ui.add_space(8.0);
            velocity_editor(app, ui, profile);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                if profile.spec("dynamic_range").is_some() {
                    ui.label(RichText::new("DYNAMIC RANGE").font(bold(9.0)).color(p.text_dim));
                    let value = param_value(app, profile, "dynamic_range");
                    ui.label(RichText::new(format!("{value:.0} dB")).font(bold(13.0)).color(p.accent));
                    let norm = (value / 60.0).clamp(0.0, 1.0);
                    if let Some(n) = hfader(ui, &p, norm, 180.0) {
                        dispatch(app, profile, "dynamic_range", n * 60.0);
                    }
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VOICES").font(bold(9.0)).color(p.text_dim));
                    for voices in [64usize, 128, 256] {
                        let on = state(app, profile).polyphony == voices;
                        if ui
                            .selectable_label(
                                on,
                                RichText::new(voices.to_string()).font(if on { bold(10.0) } else { regular(10.0) }).color(if on {
                                    p.accent
                                } else {
                                    p.text_dim
                                }),
                            )
                            .clicked()
                        {
                            state_mut(app, profile).polyphony = voices;
                        }
                    }
                });
                ui.add_space(4.0);
                let mut sustain = state(app, profile).sustain;
                if switch(ui, &p, &mut sustain, "SUSTAIN").clicked() {
                    set_sustain(app, profile, sustain);
                }
            });
        });
    });
}

fn velocity_editor(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    let (resp, painter) = ui.allocate_painter(vec2(300.0, 118.0), Sense::click_and_drag());
    let r = resp.rect;
    painter.rect_filled(r, 3.0, p.bg);
    painter.rect_stroke(r, 3.0, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    for i in 1..4 {
        let y = r.top() + r.height() * i as f32 / 4.0;
        painter.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, p.line));
    }
    let curve = state(app, profile).velocity.clone();
    let pts: Vec<egui::Pos2> = (0..=32)
        .map(|i| {
            let x = i as f32 / 32.0;
            pos2(r.left() + x * r.width(), r.bottom() - curve.at(x) * r.height())
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.8, p.accent)));
    let mut drag_to: Option<(usize, f32)> = None;
    for i in 0..9 {
        let x = r.left() + r.width() * i as f32 / 8.0;
        let y = r.bottom() - (curve.points.get(i).copied().unwrap_or(0.0) / 127.0).clamp(0.0, 1.0) * r.height();
        let point = pos2(x, y);
        let grab = Rect::from_center_size(point, vec2(14.0, 14.0));
        let hot = resp.hovered() && grab.contains(resp.hover_pos().unwrap_or(pos2(-1.0, -1.0)));
        painter.circle_filled(point, if hot { 4.5 } else { 3.0 }, if hot { p.text } else { p.accent });
        if hot
            && resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            drag_to = Some((i, ((r.bottom() - pos.y) / r.height()).clamp(0.0, 1.0) * 127.0));
        }
    }
    if let Some((i, value)) = drag_to
        && i > 0
        && i < 8
        && let Some(slot) = state_mut(app, profile).velocity.points.get_mut(i)
    {
        *slot = value;
    }
    resp.on_hover_text("Drag a point to reshape the velocity response (0–127 in → 0–127 out)");
}

// ---- footer -----------------------------------------------------------------------------------

fn footer(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            let now = app.last_frame.unwrap_or_default();
            let sent_recently = app.ui.console_sent.map(|(_, at)| now - at < 0.25).unwrap_or(false);
            led(ui, &p, "MIDI OUT", sent_recently, p.live);
            let in_active = app.msc.last_input_at.map(|at| now - at < 0.25).unwrap_or(false);
            led(ui, &p, "MIDI IN", in_active, Color32::from_rgb(0x6a, 0xa8, 0xff));
            ui.add_space(10.0);
            ui.label(RichText::new(format!("CPU {:.1} ms/frame", app.frame_ms)).font(regular(9.5)).color(p.text_dim));
            ui.label(
                RichText::new(format!("VOICES {}/{}", app.keyboard.held_notes.len(), state(app, profile).polyphony))
                    .font(regular(9.5))
                    .color(p.text_dim),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if let Some((msg, _)) = app.ui.console_sent {
                    ui.label(
                        RichText::new(format!("last {:02X} {} = {} (ch {})", msg[0], msg[1], msg[2], (msg[0] & 0x0F) + 1))
                            .font(regular(9.0))
                            .color(p.text_dim),
                    );
                }
            });
        });
        ui.add_space(6.0);
        keyboard_strip(app, ui, profile);
    });
}

fn led(ui: &mut Ui, p: &Palette, label: &str, on: bool, color: Color32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 14.0), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    let painter = ui.painter_at(rect);
    let dot = pos2(rect.left() + 5.0, rect.center().y);
    painter.circle_filled(dot, 4.0, if on { color } else { p.line });
    if on {
        painter.circle_stroke(dot, 6.0, Stroke::new(1.0, color.gamma_multiply(0.5)));
    }
    painter.text(pos2(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(8.5), p.text_dim);
}

/// The 88-key strip: A0 (21) to C8 (108), played notes lit.
fn keyboard_strip(app: &mut SoundApp, ui: &mut Ui, profile: &'static Profile) {
    let p = profile.palette;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let (low, high) = (21u8, 108u8);
    let white_count = (low..=high).filter(|n| !crate::virtual_keyboard::is_black_key(*n)).count().max(1);
    let white_w = rect.width() / white_count as f32;
    let mut white_index = 0usize;
    let mut hit: Option<u8> = None;
    let pointer = if resp.hovered() || resp.dragged() { resp.interact_pointer_pos() } else { None };
    let pressed = resp.dragged() || resp.is_pointer_button_down_on();

    for note in low..=high {
        if crate::virtual_keyboard::is_black_key(note) {
            continue;
        }
        let x = rect.left() + white_index as f32 * white_w;
        white_index += 1;
        let key = Rect::from_min_size(pos2(x, rect.top()), vec2(white_w - 0.6, rect.height()));
        let held = app.keyboard.held_notes.contains(&note);
        painter.rect_filled(key, 1.5, if held { p.accent.gamma_multiply(0.85) } else { Color32::from_rgb(0xe8, 0xe9, 0xec) });
        painter.rect_stroke(key, 1.5, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
        if let Some(pos) = pointer
            && key.contains(pos)
            && hit.is_none()
        {
            hit = Some(note);
        }
        if note % 12 == 0 {
            painter.text(
                pos2(key.center().x, key.bottom() - 7.0),
                egui::Align2::CENTER_CENTER,
                format!("C{}", note / 12 - 1),
                FontId::proportional(7.5),
                Color32::from_rgb(0x6a, 0x6e, 0x78),
            );
        }
    }
    white_index = 0;
    for note in low..=high {
        if !crate::virtual_keyboard::is_black_key(note) {
            white_index += 1;
            continue;
        }
        let x = rect.left() + white_index as f32 * white_w - white_w * 0.3;
        let key = Rect::from_min_size(pos2(x, rect.top()), vec2(white_w * 0.6, rect.height() * 0.62));
        let held = app.keyboard.held_notes.contains(&note);
        painter.rect_filled(key, 1.5, if held { p.accent } else { Color32::from_rgb(0x15, 0x17, 0x1c) });
        painter.rect_stroke(key, 1.5, Stroke::new(1.0, p.edge), egui::StrokeKind::Inside);
        if let Some(pos) = pointer
            && key.contains(pos)
        {
            hit = Some(note);
        }
    }
    if pressed && let Some(note) = hit {
        if app.keyboard.last_clicked != Some(note) {
            if let Some(prev) = app.keyboard.last_clicked {
                play_note(app, profile, prev, 0, false);
            }
            play_note(app, profile, note, 96, true);
            app.keyboard.last_clicked = Some(note);
        }
    } else if let Some(last) = app.keyboard.last_clicked.take() {
        play_note(app, profile, last, 0, false);
    }
    if resp.drag_stopped() || (!pressed && (low..=high).any(|n| app.keyboard.held_notes.contains(&n))) {
        let sounding: Vec<u8> = (low..=high).filter(|n| app.keyboard.held_notes.contains(n)).collect();
        for note in sounding {
            play_note(app, profile, note, 0, false);
        }
        app.keyboard.last_clicked = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PROFILE: Profile = Profile {
        plugin_ids: &[],
        icon: Icon::Keys,
        id: "test",
        kind: "test",
        brand: "TEST",
        name: "CONSOLE",
        tagline: "",
        palette: Palette::GRAPHITE,
        categories: &[],
        models: &[],
        variants: &[],
        master: &[],
        racks: &["RACK"],
        specs: &[Spec {
            id: "gain",
            label: "GAIN",
            group: 0,
            min: 0.0,
            max: 60.0,
            step: 0.0,
            default: 30.0,
            unit: "dB",
            cc: 39,
            automation: "Gain",
            decimals: 1,
        }],
        selects: &[Select { id: "type", label: "TYPE", group: 0, options: &["A", "B"], cc: 40, automation: "Type" }],
        library_picker: None,
        velocity_curve: false,
        program_change: false,
    };

    #[test]
    fn a_value_becomes_a_midi_cc_at_the_right_endpoints() {
        let spec = TEST_PROFILE.specs.first().copied().expect("the test profile has a spec");
        assert_eq!(cc_message(&spec, 0, spec.min), [0xB0, spec.cc, 0]);
        assert_eq!(cc_message(&spec, 0, spec.max), [0xB0, spec.cc, 127]);
        assert_eq!(cc_message(&spec, 15, spec.max)[0] & 0x0F, 15, "the channel is honoured");
        let mid = cc_message(&spec, 0, (spec.min + spec.max) / 2.0)[2];
        assert!((i32::from(mid) - 64).abs() <= 1, "the middle lands near 64, got {mid}");
    }

    #[test]
    fn velocity_presets_keep_their_endpoints_and_are_monotonic() {
        for index in 0..4 {
            let curve = VelocityCurve::preset(index);
            assert_eq!(curve.apply(0), 0, "preset {index} must map 0 to 0");
            assert_eq!(curve.apply(127), 127, "preset {index} must map 127 to 127");
            let mut previous = 0;
            for v in (0..=127).step_by(8) {
                let out = curve.apply(v);
                assert!(out >= previous, "preset {index} is not monotonic at {v}");
                previous = out;
            }
        }
        assert_eq!(VelocityCurve::preset(0).apply(64), 64, "the linear preset is the identity");
    }

    #[test]
    fn the_state_clamps_values_and_falls_back_to_defaults() {
        let mut state = ConsoleState::default();
        assert_eq!(state.value(&TEST_PROFILE, "gain"), 30.0, "a fresh console starts at the default");
        state.set(&TEST_PROFILE, "gain", 500.0);
        assert_eq!(state.value(&TEST_PROFILE, "gain"), 60.0, "clamped into the spec's range");
        state.set(&TEST_PROFILE, "type", 9.0);
        assert_eq!(state.value(&TEST_PROFILE, "type"), 1.0, "a choice keeps a valid option index");
        state.set(&TEST_PROFILE, "unknown", 1.0);
        assert_eq!(state.value(&TEST_PROFILE, "unknown"), 0.0, "unknown ids are ignored");
    }

    #[test]
    fn captured_sounds_are_namespaced_and_old_ui_state_still_loads() {
        let map = std::collections::BTreeMap::from([
            ("C. Bechstein DG".to_string(), "bound".to_string()),
            (capture_key("  Prelude  "), "captured".to_string()),
            (capture_key("Warm Felt"), "captured".to_string()),
        ]);
        assert_eq!(capture_names(&map), ["Prelude", "Warm Felt"]);

        let old_state: ConsoleState = serde_json::from_str(r#"{"model":2}"#).expect("old console state remains readable");
        assert_eq!(old_state.model, 2);
        assert!(old_state.capture_name.is_empty());
        assert!(old_state.selected_capture.is_empty());
    }

    #[test]
    fn every_profile_is_distinct_findable_and_has_its_own_theme() {
        let mut ids = std::collections::HashSet::new();
        let mut accents = std::collections::HashSet::new();
        for p in profiles() {
            assert!(ids.insert(p.id), "duplicate profile id {}", p.id);
            assert!(profile(p.id).is_some_and(|found| found.id == p.id), "{} is not findable by id", p.id);
            assert!(p.window_title().contains(p.name), "the window title names the instrument");
            assert!(!p.racks.is_empty(), "{} has no racks", p.id);
            assert!(!p.specs.is_empty(), "{} has no parameters", p.id);
            accents.insert(p.palette.accent);
        }
        assert!(profiles().len() >= 6, "the console ships several instruments, not one");
        assert_eq!(accents.len(), profiles().len(), "every instrument has its own accent colour");
    }
}

/// The instrument console follows the selected instrument track if one is selected.
pub fn follow_selection(app: &mut SoundApp) {
    let s = app.engine.session();
    if let Some(&sel) = s.edit.selected_tracks.first()
        && let Some(t) = s.track(sel)
        && let Some(ins) = &t.instrument
        && let Some(p) = profile_for_plugin(&ins.plugin)
        && app.ui.console_profile != p.id
    {
        app.ui.console_profile = p.id.to_string();
    }
}
