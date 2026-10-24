//! MSC Mixer Hardware Recognition, Communication, and Control Management.
//!
//! Provides automatic hardware detection (ALSA MIDI, USB-MIDI, Serial, HID),
//! customizable JSON mapping, 14-bit high-resolution faders, V-pot encoders,
//! Kontakt macro automation, and bidirectional LED / motorized fader synchronization.

use crate::SoundApp;
use crate::midi_io::{LinkState, Status as LinkStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soundcraft_model::TrackId;
use std::collections::HashMap;
use std::path::Path;

fn default_true() -> bool {
    true
}

/// Feedback is droppable: the queue is bounded so a dead device can never grow memory.
pub const MAX_FEEDBACK_QUEUE: usize = 512;

/// How close (in normalized fader travel) the physical fader must be to the virtual value for a
/// soft takeover to engage without crossing it.
const PICKUP_EPSILON: f32 = 0.01;

/// Applies a mixing change coming from the surface. A failure is logged instead of dropped: a
/// fader that silently does nothing is the hardest failure for a user to report.
fn apply(app: &mut SoundApp, command: &str, params: &Value, key: &str) {
    if let Err(e) = app.engine.execute_merged(command, params, key) {
        log::warn!("MCU: `{command}` {params} failed: {e}");
    }
}

/// Runs a command triggered by a surface button, logging a failure instead of dropping it.
fn run(app: &mut SoundApp, command: &str, params: Value) {
    if let Err(e) = app.run(command, params) {
        log::warn!("MCU: `{command}` failed: {e}");
    }
}

fn default_device_filter() -> String {
    "SMC-Mixer".to_string()
}
fn default_latency() -> u32 {
    10
}
fn default_master_channel() -> u8 {
    8
}
fn default_loop_note() -> u8 {
    86
}
fn default_rewind_note() -> u8 {
    91
}
fn default_forward_note() -> u8 {
    92
}
fn default_vpot_click_base() -> u8 {
    32
}
fn default_ch_left_note() -> u8 {
    48
}
fn default_ch_right_note() -> u8 {
    49
}
fn default_profile() -> ControllerProfile {
    ControllerProfile::IconP1M
}
fn default_jog_cc() -> u8 {
    60
}
fn default_scrub_note() -> u8 {
    101
}
fn default_marker_note() -> u8 {
    84
}
fn default_flip_note() -> u8 {
    50
}
fn default_auto_read() -> u8 {
    74
}
fn default_auto_write() -> u8 {
    75
}
fn default_auto_touch() -> u8 {
    77
}
fn default_auto_latch() -> u8 {
    78
}
fn default_auto_trim() -> u8 {
    76
}
fn default_auto_off() -> u8 {
    79
}

/// Supported Hardware Controller Profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerProfile {
    IconP1M,
    MVaveSmc,
    GenericMcu,
}

/// Discovered Bluetooth BLE device info.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BleDevice {
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    pub is_mvave: bool,
}

/// MSC Mixer hardware configuration mapping (Mackie Control Universal & Custom CC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MscConfig {
    pub device_name: String,
    pub protocol: String,
    pub channels: usize,
    pub fader_pitch_bend: bool,
    pub fader_cc_base: u8,
    pub fader_min_db: f32,
    pub fader_max_db: f32,
    pub pan_cc_base: u8,
    pub send_cc_base: u8,
    pub kontakt_macro_cc: Vec<u8>,
    pub mute_note_base: u8,
    pub solo_note_base: u8,
    pub arm_note_base: u8,
    pub select_note_base: u8,
    pub bank_left_note: u8,
    pub bank_right_note: u8,
    pub play_note: u8,
    pub stop_note: u8,
    pub rec_note: u8,
    pub bidirectional_led: bool,
    pub motorized_faders: bool,
    #[serde(default = "default_true")]
    pub bluetooth_auto_connect: bool,
    #[serde(default = "default_device_filter")]
    pub bluetooth_device_name_filter: String,
    #[serde(default = "default_latency")]
    pub bluetooth_latency_ms: u32,
    #[serde(default = "default_master_channel")]
    pub master_fader_channel: u8,
    #[serde(default = "default_loop_note")]
    pub loop_note: u8,
    #[serde(default = "default_rewind_note")]
    pub rewind_note: u8,
    #[serde(default = "default_forward_note")]
    pub forward_note: u8,
    #[serde(default = "default_vpot_click_base")]
    pub vpot_click_base: u8,
    #[serde(default = "default_ch_left_note")]
    pub ch_left_note: u8,
    #[serde(default = "default_ch_right_note")]
    pub ch_right_note: u8,
    #[serde(default = "default_profile")]
    pub profile: ControllerProfile,
    #[serde(default = "default_jog_cc")]
    pub jog_wheel_cc: u8,
    #[serde(default = "default_scrub_note")]
    pub scrub_note: u8,
    #[serde(default = "default_marker_note")]
    pub marker_note: u8,
    #[serde(default = "default_flip_note")]
    pub flip_note: u8,
    #[serde(default = "default_auto_read")]
    pub auto_read_note: u8,
    #[serde(default = "default_auto_write")]
    pub auto_write_note: u8,
    #[serde(default = "default_auto_touch")]
    pub auto_touch_note: u8,
    #[serde(default = "default_auto_latch")]
    pub auto_latch_note: u8,
    #[serde(default = "default_auto_trim")]
    pub auto_trim_note: u8,
    #[serde(default = "default_auto_off")]
    pub auto_off_note: u8,
    #[serde(default = "default_true")]
    pub mcu_led_meters: bool,
    /// Soft takeover: a physical fader only moves its track once it has reached (or crossed)
    /// the track's current value. Off by default (a fader moves its track at once); opt in from
    /// the controller window or with `controller.soft_takeover`.
    #[serde(default)]
    pub soft_takeover: bool,
}

impl MscConfig {
    /// Factory preset for iCON P1-M Precision Mackie Control Surface.
    pub fn icon_p1m() -> Self {
        MscConfig {
            device_name: "iCON P1-M Precision DAW Controller".to_string(),
            protocol: "Mackie Control Universal (MCU)".to_string(),
            channels: 8,
            fader_pitch_bend: true,
            fader_cc_base: 0,
            fader_min_db: -60.0,
            fader_max_db: 12.0,
            pan_cc_base: 16,
            send_cc_base: 24,
            kontakt_macro_cc: vec![1, 11, 71, 74, 75, 76, 77, 78],
            mute_note_base: 16,
            solo_note_base: 8,
            arm_note_base: 0,
            select_note_base: 24,
            bank_left_note: 46,
            bank_right_note: 47,
            play_note: 94,
            stop_note: 93,
            rec_note: 95,
            bidirectional_led: true,
            motorized_faders: true,
            bluetooth_auto_connect: false,
            bluetooth_device_name_filter: "iCON".to_string(),
            bluetooth_latency_ms: 1,
            master_fader_channel: 8,
            loop_note: 86,
            rewind_note: 91,
            forward_note: 92,
            vpot_click_base: 32,
            ch_left_note: 48,
            ch_right_note: 49,
            profile: ControllerProfile::IconP1M,
            jog_wheel_cc: 60,
            scrub_note: 101,
            marker_note: 84,
            flip_note: 50,
            auto_read_note: 74,
            auto_write_note: 75,
            auto_touch_note: 77,
            auto_latch_note: 78,
            auto_trim_note: 76,
            auto_off_note: 79,
            mcu_led_meters: true,
            soft_takeover: false,
        }
    }

    /// Factory preset for M-Vave SMC-Mixer (Bluetooth BLE + USB).
    pub fn mvave_smc() -> Self {
        MscConfig {
            device_name: "M-Vave SMC-Mixer".to_string(),
            protocol: "Mackie Control Universal (MCU)".to_string(),
            channels: 8,
            fader_pitch_bend: true,
            fader_cc_base: 0,
            fader_min_db: -60.0,
            fader_max_db: 12.0,
            pan_cc_base: 16,
            send_cc_base: 24,
            kontakt_macro_cc: vec![1, 11, 71, 74, 75, 76, 77, 78],
            mute_note_base: 16,
            solo_note_base: 8,
            arm_note_base: 0,
            select_note_base: 24,
            bank_left_note: 46,
            bank_right_note: 47,
            play_note: 94,
            stop_note: 93,
            rec_note: 95,
            bidirectional_led: true,
            motorized_faders: true,
            bluetooth_auto_connect: true,
            bluetooth_device_name_filter: "SMC-Mixer".to_string(),
            bluetooth_latency_ms: 10,
            master_fader_channel: 8,
            loop_note: 86,
            rewind_note: 91,
            forward_note: 92,
            vpot_click_base: 32,
            ch_left_note: 48,
            ch_right_note: 49,
            profile: ControllerProfile::MVaveSmc,
            jog_wheel_cc: 60,
            scrub_note: 101,
            marker_note: 84,
            flip_note: 50,
            auto_read_note: 74,
            auto_write_note: 75,
            auto_touch_note: 77,
            auto_latch_note: 78,
            auto_trim_note: 76,
            auto_off_note: 79,
            mcu_led_meters: true,
            soft_takeover: false,
        }
    }
}

impl Default for MscConfig {
    fn default() -> Self {
        Self::icon_p1m()
    }
}

/// How the unlearned faders and knobs behave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MscMode {
    /// Global: faders/knobs control the tracks of the current bank (FL-style "global" mode).
    #[default]
    Global,
    /// Link to controller: only explicitly learned controls do anything.
    Linked,
}

impl MscMode {
    pub const fn label(self) -> &'static str {
        match self {
            MscMode::Global => "Global (bank tracks)",
            MscMode::Linked => "Link to controller (MIDI Learn only)",
        }
    }
}

/// A parameter a physical control can be permanently linked to (MIDI Learn).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearnTarget {
    /// A track's fader.
    Volume { track: u64 },
    /// A track's pan pot (`index` 0..1 for stereo).
    Pan { track: u64, index: u32 },
    /// A send level.
    SendLevel { track: u64, slot: u32 },
    /// A plugin parameter in an insert slot (plain range `min..=max`).
    InsertParam { track: u64, slot: u32, param: String, min: f32, max: f32 },
    /// A built-in instrument parameter (plain range `min..=max`).
    InstrumentParam { track: u64, param: String, min: f32, max: f32 },
}

impl LearnTarget {
    pub fn describe(&self) -> String {
        match self {
            LearnTarget::Volume { track } => format!("Track {track} volume"),
            LearnTarget::Pan { track, index } => format!("Track {track} pan {}", if *index == 0 { "L" } else { "R" }),
            LearnTarget::SendLevel { track, slot } => format!("Track {track} send {slot}"),
            LearnTarget::InsertParam { track, slot, param, .. } => format!("Track {track} insert {slot} `{param}`"),
            LearnTarget::InstrumentParam { track, param, .. } => format!("Track {track} instrument `{param}`"),
        }
    }
}

/// One learned control: a MIDI message (status without channel, channel, first data byte) bound
/// to a parameter. Persisted with the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiBinding {
    /// Message status without the channel nibble: 0x90, 0x80, 0xB0 or 0xE0.
    pub status: u8,
    pub channel: u8,
    pub data1: u8,
    pub target: LearnTarget,
}

impl MidiBinding {
    /// Human description for the hardware window and the context menu.
    pub fn describe(&self) -> String {
        let kind = match self.status {
            0x90 => "Note",
            0x80 => "Note off",
            0xB0 => "CC",
            _ => "Pitch bend",
        };
        if self.status == 0xE0 { format!("Fader ch {}", self.channel + 1) } else { format!("{kind} {} ch {}", self.data1, self.channel + 1) }
    }
}

/// The persisted part of the controller setup (written to `config.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MscPrefs {
    /// Settings format version; 0 means "written before the soft-takeover default changed".
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub config: MscConfig,
    /// The MIDI keyboard port to subscribe to (a piano or a controller), by exact port name.
    #[serde(default)]
    pub keyboard_port: Option<String>,
    /// Use Bluetooth (BLE-MIDI) for the control surface.
    #[serde(default)]
    pub ble: bool,
    /// A MIDI port forced as the control surface (exact name), when the device filter misses it.
    #[serde(default)]
    pub surface_port: Option<String>,
    #[serde(default)]
    pub mode: MscMode,
    #[serde(default)]
    pub bindings: Vec<MidiBinding>,
    #[serde(default)]
    pub filter: String,
}

/// Hardware state and port manager for iCON P1-M & M-Vave SMC-Mixer.
pub struct MscManager {
    pub config: MscConfig,
    pub active_bank: usize,
    pub port_name: String,
    pub connected: bool,
    pub connection_type: String,
    pub midi_learn_target: Option<String>,
    pub output_queue: Vec<[u8; 3]>,
    pub last_sent_feedback: HashMap<u8, u8>,
    pub last_fader_feedback: HashMap<u8, u16>,
    pub fader_touched: [bool; 9],
    pub scrub_active: bool,
    pub faders_flipped: bool,
    pub last_meter_bytes: [u8; 8],
    pub bluetooth_available: bool,
    pub bluetooth_powered: bool,
    pub ble_devices: Vec<BleDevice>,
    pub ble_scanning: bool,
    pub ble_status: String,
    pub ble_connected_mac: Option<String>,
    pub last_log_message: Option<String>,
    /// How unlearned faders and knobs behave (global bank vs learned links only).
    pub mode: MscMode,
    /// Learned control → parameter links (MIDI Learn), persisted.
    pub bindings: Vec<MidiBinding>,
    /// The parameter waiting for the next incoming message (MIDI Learn armed).
    pub learn: Option<LearnTarget>,
    /// Live status of the hardware MIDI link (port, throughput, hot-plug counters).
    pub link: LinkStatus,
    /// The MIDI keyboard port the user picked (persisted).
    pub keyboard_port: Option<String>,
    /// Use the Bluetooth (BLE-MIDI) link for the control surface.
    pub ble: bool,
    /// A MIDI port forced as the control surface (persisted).
    pub surface_port: Option<String>,
    /// The last messages received, for the controller window's input monitor.
    pub recent_input: Vec<(String, [u8; 3])>,
    /// When the last input message arrived (UI clock), for the activity light.
    pub last_input_at: Option<f64>,
    /// Physical fader position at the previous message, for soft takeover crossing tests.
    pub last_phys: [f32; 9],
    /// The virtual value a fader must reach before it takes over its track (waiting state).
    pub pickup: [Option<f32>; 9],
    /// True once a fader has taken over its track: it then follows until the bank changes.
    pub engaged: [bool; 9],
}

impl Default for MscManager {
    fn default() -> Self {
        MscManager {
            config: MscConfig::default(),
            active_bank: 0,
            port_name: "iCON P1-M DAW Controller [USB/MCU]".to_string(),
            connected: false,
            connection_type: "USB 2.0 / Class-Compliant MCU".to_string(),
            midi_learn_target: None,
            output_queue: Vec::new(),
            last_sent_feedback: HashMap::new(),
            last_fader_feedback: HashMap::new(),
            fader_touched: [false; 9],
            scrub_active: false,
            faders_flipped: false,
            last_meter_bytes: [0; 8],
            bluetooth_available: false,
            bluetooth_powered: false,
            ble_devices: Vec::new(),
            ble_scanning: false,
            ble_status: "Ready".to_string(),
            ble_connected_mac: None,
            last_log_message: None,
            mode: MscMode::default(),
            bindings: Vec::new(),
            learn: None,
            link: LinkStatus::default(),
            keyboard_port: None,
            ble: false,
            surface_port: None,
            recent_input: Vec::new(),
            last_input_at: None,
            last_phys: [0.0; 9],
            pickup: [None; 9],
            engaged: [false; 9],
        }
    }
}

impl MscManager {
    /// Picks the hardware profile from the live link (or, when the MIDI layer is unavailable,
    /// from a diagnostic `aconnect -l` probe). Connection state is never invented: a surface is
    /// "connected" only while the MIDI worker holds a subscription to it.
    pub fn auto_detect(&mut self) {
        let probe = if self.link.state == LinkState::Connected {
            self.link.port_name.to_ascii_lowercase()
        } else if let Ok(out) = std::process::Command::new("aconnect").arg("-l").output() {
            String::from_utf8_lossy(&out.stdout).to_ascii_lowercase()
        } else {
            String::new()
        };
        if probe.contains("icon") || probe.contains("p1-m") {
            self.config.profile = ControllerProfile::IconP1M;
            self.config.device_name = "iCON P1-M Precision DAW Controller".to_string();
        } else if probe.contains("smc") || probe.contains("m-vave") || probe.contains("cuvave") {
            self.config.profile = ControllerProfile::MVaveSmc;
            self.config.device_name = "M-Vave SMC-Mixer".to_string();
        }
        self.apply_link_status(self.link.clone());
    }

    /// Copies a link snapshot from the MIDI worker into the mixer's own state. Called every
    /// frame; cheap and allocation-light when nothing changed.
    pub fn apply_link_status(&mut self, status: LinkStatus) {
        self.connected = status.state == LinkState::Connected;
        if self.connected {
            if !status.port_name.is_empty() {
                self.port_name = status.port_name.clone();
            }
            let lower = status.port_name.to_ascii_lowercase();
            self.connection_type = if lower.contains("bluetooth") || lower.contains("ble") {
                format!("Bluetooth BLE-MIDI ({})", status.port_name)
            } else if lower.contains("icon") || lower.contains("p1-m") {
                "USB-MIDI · iCON P1-M (MCU)".to_string()
            } else {
                "USB-MIDI · ALSA sequencer (MCU)".to_string()
            };
        } else if !status.port_name.is_empty() {
            self.port_name = status.port_name.clone();
        }
        if status.state == LinkState::Searching && !status.message.is_empty() {
            self.ble_status = status.message.clone();
        }
        self.link = status;
    }

    /// The port forced as the control surface (if any).
    pub fn surface_port(&self) -> Option<String> {
        self.surface_port.clone()
    }

    /// True when a hardware link is live.
    pub fn is_linked(&self) -> bool {
        self.link.state == LinkState::Connected
    }

    /// Arms MIDI Learn: the next incoming message is bound to `target`.
    pub fn learn_start(&mut self, target: LearnTarget) {
        self.last_log_message = Some(format!("MIDI Learn armed: move a knob/fader or press a button ({}).", target.describe()));
        self.learn = Some(target);
    }

    pub fn learn_cancel(&mut self) {
        if self.learn.take().is_some() {
            self.last_log_message = Some("MIDI Learn cancelled.".to_string());
        }
    }

    /// Drops the learned links of one parameter, or all of them.
    pub fn learn_clear(&mut self, target: Option<&LearnTarget>) {
        match target {
            Some(t) => self.bindings.retain(|b| &b.target != t),
            None => self.bindings.clear(),
        }
    }

    fn binding_for(&self, status: u8, channel: u8, data1: u8) -> Option<&MidiBinding> {
        self.bindings.iter().find(|b| b.status == status && b.channel == channel && b.data1 == data1)
    }

    /// Fader normalization: 0.75 of the travel is the 0 dB point, matching the feedback curve.
    pub fn norm_from_db(&self, db: f32) -> f32 {
        if db <= self.config.fader_min_db {
            0.0
        } else if db <= 0.0 {
            ((db - self.config.fader_min_db) / (-self.config.fader_min_db)) * 0.75
        } else {
            0.75 + (db / self.config.fader_max_db) * 0.25
        }
    }

    /// Inverse of [`MscManager::norm_from_db`].
    pub fn db_from_norm(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        if n <= 0.001 {
            self.config.fader_min_db
        } else if (n - 0.75).abs() < 0.02 {
            0.0
        } else if n < 0.75 {
            self.config.fader_min_db + (n / 0.75) * (-self.config.fader_min_db)
        } else {
            ((n - 0.75) / 0.25) * self.config.fader_max_db
        }
    }

    /// Soft takeover: may this physical fader move the virtual parameter now? It engages when
    /// the physical position is at (or within [`PICKUP_EPSILON`] of) the virtual one, or when it
    /// crosses it while moving. Until then the parameter is untouched, so a bank switch never
    /// jumps a volume.
    pub fn pickup_gate(&mut self, channel: usize, phys: f32, virt: f32) -> bool {
        let Some(slot) = self.pickup.get_mut(channel) else { return true };
        let last = self.last_phys.get(channel).copied().unwrap_or(phys);
        if let Some(l) = self.last_phys.get_mut(channel) {
            *l = phys;
        }
        if !self.config.soft_takeover {
            *slot = None;
            if let Some(e) = self.engaged.get_mut(channel) {
                *e = true;
            }
            return true;
        }
        if self.engaged.get(channel).copied().unwrap_or(true) {
            return true;
        }
        let near = (phys - virt).abs() <= PICKUP_EPSILON;
        let engage = match *slot {
            None => {
                if !near {
                    *slot = Some(virt);
                }
                near
            }
            Some(_) => {
                // Engage at the virtual value, or when the fader crosses it while moving.
                let crossed = (last - virt).signum() != (phys - virt).signum() && (last - phys).abs() > f32::EPSILON;
                near || crossed
            }
        };
        if engage {
            *slot = None;
            if let Some(e) = self.engaged.get_mut(channel) {
                *e = true;
            }
        }
        engage
    }

    /// Forgets every pickup, so the next fader movement must re-acquire its track. Called when
    /// the bank or the track selection changes under the faders.
    pub fn reset_pickups(&mut self) {
        self.pickup = [None; 9];
        self.engaged = [false; 9];
        self.last_phys = [0.0; 9];
    }

    /// The persisted preferences (mapping + mode + learned links).
    pub fn prefs(&self) -> MscPrefs {
        MscPrefs {
            version: 2,
            config: self.config.clone(),
            keyboard_port: self.keyboard_port.clone(),
            ble: self.ble,
            surface_port: self.surface_port.clone(),
            mode: self.mode,
            bindings: self.bindings.clone(),
            filter: self.link.filter.clone(),
        }
    }

    /// Applies persisted preferences. Runtime state (queues, ports) is left alone.
    pub fn apply_prefs(&mut self, prefs: &MscPrefs) {
        self.config = prefs.config.clone();
        // Settings written before version 2 carried pickup on by default, which made physical
        // faders look dead until they crossed the track's value. Move them to the direct mode.
        if prefs.version < 2 {
            self.config.soft_takeover = false;
        }
        self.keyboard_port = prefs.keyboard_port.clone();
        self.ble = prefs.ble;
        self.surface_port = prefs.surface_port.clone();
        self.mode = prefs.mode;
        self.bindings = prefs.bindings.clone();
    }

    /// Records an incoming message for the controller window (bounded).
    pub fn note_input(&mut self, source: &str, data: [u8; 3], now: f64) {
        if self.recent_input.len() >= 12 {
            self.recent_input.remove(0);
        }
        self.recent_input.push((source.to_string(), data));
        self.last_input_at = Some(now);
    }

    /// True while a message arrived in the last `window` seconds (the activity light).
    pub fn input_active(&self, now: f64, window: f64) -> bool {
        self.last_input_at.is_some_and(|t| now - t < window)
    }

    /// Queues one feedback packet, dropping it when the queue is full.
    fn queue(&mut self, packet: [u8; 3]) {
        if self.output_queue.len() < MAX_FEEDBACK_QUEUE {
            self.output_queue.push(packet);
        }
    }

    /// Load or save mapping configuration file `icon_p1m_map.json` and `msc_mixer_map.json`.
    pub fn ensure_config_loaded(&mut self, config_dir: Option<&Path>) {
        if let Some(dir) = config_dir {
            let p1m_path = dir.join("icon_p1m_map.json");
            let msc_path = dir.join("msc_mixer_map.json");

            let target_path = if self.config.profile == ControllerProfile::IconP1M {
                if p1m_path.exists() { &p1m_path } else { &msc_path }
            } else {
                if msc_path.exists() { &msc_path } else { &p1m_path }
            };

            if target_path.exists()
                && let Ok(content) = std::fs::read_to_string(target_path)
                && let Ok(cfg) = serde_json::from_str::<MscConfig>(&content)
            {
                self.config = cfg;
                return;
            }

            // Save default config maps for both controllers
            let _ = std::fs::create_dir_all(dir);
            if let Ok(json_p1m) = serde_json::to_string_pretty(&MscConfig::icon_p1m()) {
                let _ = std::fs::write(&p1m_path, json_p1m);
            }
            if let Ok(json_smc) = serde_json::to_string_pretty(&MscConfig::mvave_smc()) {
                let _ = std::fs::write(&msc_path, json_smc);
            }
        }
    }

    /// Checks Bluetooth adapter status and lists known BLE devices via bluetoothctl.
    pub fn refresh_bluetooth(&mut self) {
        if let Ok(show_out) = std::process::Command::new("bluetoothctl").arg("show").output() {
            self.bluetooth_available = show_out.status.success();
            let text = String::from_utf8_lossy(&show_out.stdout);
            self.bluetooth_powered = text.contains("Powered: yes") || text.contains("PowerState: on");
        } else {
            self.bluetooth_available = false;
            self.bluetooth_powered = false;
        }

        if let Ok(dev_out) = std::process::Command::new("bluetoothctl").arg("devices").output() {
            let text = String::from_utf8_lossy(&dev_out.stdout);
            let mut list = Vec::new();
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[0] == "Device" {
                    let mac = parts[1].to_string();
                    let name = parts[2..].join(" ");
                    let lower = name.to_lowercase();
                    let is_mvave = lower.contains("smc")
                        || lower.contains("m-vave")
                        || lower.contains("cuvave")
                        || lower.contains("footctrl")
                        || lower.contains("mixer");
                    list.push(BleDevice { mac, name, connected: false, paired: false, is_mvave });
                }
            }

            for dev in &mut list {
                if (dev.is_mvave || self.ble_connected_mac.as_deref() == Some(&dev.mac))
                    && let Ok(info_out) = std::process::Command::new("bluetoothctl").args(["info", &dev.mac]).output()
                {
                    let info_text = String::from_utf8_lossy(&info_out.stdout);
                    dev.connected = info_text.contains("Connected: yes");
                    dev.paired = info_text.contains("Paired: yes");
                    if dev.connected {
                        self.ble_connected_mac = Some(dev.mac.clone());
                        self.connected = true;
                        self.connection_type = format!("Bluetooth BLE-MIDI ({})", dev.name);
                    }
                }
            }
            self.ble_devices = list;
        }
    }

    /// Triggers an active Bluetooth scan for nearby BLE devices (e.g. M-Vave SMC-Mixer).
    pub fn scan_bluetooth(&mut self) -> Result<usize, String> {
        self.ble_scanning = true;
        self.ble_status = "Scanning for Bluetooth BLE devices...".to_string();
        let _ = std::process::Command::new("bluetoothctl").args(["--timeout", "4", "scan", "on"]).output();
        self.refresh_bluetooth();
        self.ble_scanning = false;
        let count = self.ble_devices.len();
        self.ble_status = format!("Scan complete. Found {} devices.", count);
        Ok(count)
    }

    /// Pairs, trusts, and connects to a Bluetooth BLE device by MAC address.
    pub fn connect_ble(&mut self, mac: &str) -> Result<String, String> {
        self.ble_status = format!("Connecting to BLE device {mac}...");
        let _ = std::process::Command::new("bluetoothctl").args(["pair", mac]).output();
        let _ = std::process::Command::new("bluetoothctl").args(["trust", mac]).output();
        let conn =
            std::process::Command::new("bluetoothctl").args(["connect", mac]).output().map_err(|e| format!("Failed to invoke bluetoothctl: {e}"))?;

        let res_text = String::from_utf8_lossy(&conn.stdout);
        if conn.status.success() || res_text.contains("Connection successful") {
            self.ble_connected_mac = Some(mac.to_string());
            self.connected = true;
            self.connection_type = format!("Bluetooth BLE-MIDI ({mac})");
            self.port_name = format!("M-Vave SMC-Mixer [BLE: {mac}]");
            self.ble_status = format!("Connected to {mac} via Bluetooth BLE-MIDI!");
            self.refresh_bluetooth();
            Ok(format!("Connected to {mac}"))
        } else {
            let err = format!("Failed to connect to {mac}: {res_text}");
            self.ble_status = err.clone();
            Err(err)
        }
    }

    /// Disconnects an active Bluetooth BLE device.
    pub fn disconnect_ble(&mut self, mac: &str) -> Result<String, String> {
        let _ = std::process::Command::new("bluetoothctl").args(["disconnect", mac]).output();
        if self.ble_connected_mac.as_deref() == Some(mac) {
            self.ble_connected_mac = None;
            self.auto_detect();
        }
        self.ble_status = format!("Disconnected from {mac}.");
        self.refresh_bluetooth();
        Ok(format!("Disconnected {mac}"))
    }

    /// Automatically scans and connects to the first nearby M-Vave or SMC-Mixer controller.
    pub fn auto_connect_ble(&mut self) -> Result<String, String> {
        self.refresh_bluetooth();
        if let Some(m) = self.ble_devices.iter().find(|d| d.is_mvave) {
            let mac = m.mac.clone();
            return self.connect_ble(&mac);
        }
        let _ = self.scan_bluetooth();
        if let Some(m) = self.ble_devices.iter().find(|d| d.is_mvave) {
            let mac = m.mac.clone();
            self.connect_ble(&mac)
        } else {
            Err("No M-Vave SMC-Mixer device found nearby. Ensure Bluetooth pairing is enabled on the mixer.".to_string())
        }
    }

    /// Sends a diagnostic test sequence (flashes Mute/Solo/Arm LEDs and exercises faders).
    pub fn trigger_diagnostic_test(&mut self) {
        for ch in 0..self.config.channels {
            let mute = self.config.mute_note_base + ch as u8;
            let solo = self.config.solo_note_base + ch as u8;
            let arm = self.config.arm_note_base + ch as u8;
            self.queue([0x90, mute, 127]);
            self.queue([0x90, solo, 127]);
            self.queue([0x90, arm, 127]);

            if self.config.motorized_faders {
                self.queue([0xE0 | (ch as u8 & 0x0F), 0x00, 0x40]);
            }
        }
        if self.config.motorized_faders {
            self.queue([0xE0 | (self.config.master_fader_channel & 0x0F), 0x00, 0x40]);
        }
    }

    /// Resolves the track ID corresponding to the physical channel (0..7) in the current bank.
    pub fn track_for_channel(&self, session: &soundcraft_model::Session, channel: usize) -> Option<TrackId> {
        let target_idx = self.active_bank * self.config.channels + channel;
        session.tracks.iter().filter(|t| !t.hidden).nth(target_idx).map(|t| t.id)
    }

    /// Processes a raw 1-to-3 byte MIDI message from MSC Mixer.
    pub fn handle_message(&mut self, app: &mut SoundApp, msg: &[u8]) -> Option<String> {
        if msg.is_empty() {
            return None;
        }
        let status = msg[0] & 0xF0;
        let channel = (msg[0] & 0x0F) as usize;
        let d1 = *msg.get(1).unwrap_or(&0);
        let d2 = *msg.get(2).unwrap_or(&0);

        // Learned links (MIDI Learn) take precedence over the global bank mapping.
        if let Some(binding) = self.binding_for(status, channel as u8, d1).cloned()
            && let Some(msg) = self.apply_learned(app, &binding, d1, d2)
        {
            return Some(msg);
        }
        // An armed MIDI Learn binds the next message it sees.
        if let Some(target) = self.learn.take() {
            let binding = MidiBinding { status, channel: channel as u8, data1: d1, target };
            let msg = format!("MIDI Learn: {} -> {}", binding.describe(), binding.target.describe());
            self.bindings.push(binding);
            self.last_log_message = Some(msg.clone());
            return Some(msg);
        }
        // Linked mode: only learned controls act; the bank mapping is switched off.
        let global = self.mode == MscMode::Global;

        match status {
            // Pitch Bend: High-resolution faders (channels 0..7 for tracks 1..8, channel 8 for Master)
            0xE0 if self.config.fader_pitch_bend => {
                let val14 = (u16::from(d2) << 7) | u16::from(d1);
                let norm = val14 as f32 / 16383.0;
                if !global {
                    return None;
                }

                if self.faders_flipped {
                    // Flip mode: faders drive pan instead of volume.
                    let pan = (norm * 2.0 - 1.0).clamp(-1.0, 1.0);
                    if channel < self.config.channels
                        && let Some(tid) = self.track_for_channel(app.engine.session(), channel)
                    {
                        apply(app, "mix.pan", &json!({"track": tid.0, "pan": pan}), &format!("pan:{}", tid.0));
                        return Some(format!("MCU Fader (Flipped) ch {} -> Track {} Pan = {:.2}", channel + 1, tid.0, pan));
                    }
                } else {
                    // Normal mode: faders drive volume, through soft takeover.
                    let target = if channel < self.config.channels {
                        self.track_for_channel(app.engine.session(), channel)
                    } else if channel == 8 {
                        app.engine.session().tracks.iter().find(|t| t.kind == soundcraft_model::TrackKind::Master).map(|t| t.id)
                    } else {
                        None
                    };
                    if let Some(tid) = target {
                        let cur_db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
                        let virt = self.norm_from_db(cur_db);
                        if !self.pickup_gate(channel, norm, virt) {
                            let msg = format!(
                                "MCU Fader ch {} waiting for pickup: move it to {:.0}% (track at {:.1} dB)",
                                channel + 1,
                                virt * 100.0,
                                cur_db
                            );
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                        let db = self.db_from_norm(norm);
                        apply(app, "mix.volume", &json!({"track": tid.0, "db": db}), &format!("fader:{}", tid.0));
                        return Some(format!("MCU Fader ch {} -> Track {} = {:.1} dB", channel + 1, tid.0, db));
                    }
                }
            }

            // Control Change (Knobs, Encoders, Jog Wheel, Kontakt Macros)
            0xB0 => {
                let cc = d1;
                let val = d2;

                // 1. Optical Jog Wheel / Scrub (MCU CC 60)
                if cc == self.config.jog_wheel_cc {
                    let (cw, delta) = if val & 0x40 != 0 { (false, ((val & 0x3F).max(1)) as f64) } else { (true, ((val & 0x3F).max(1)) as f64) };
                    let step_secs = if self.scrub_active { 0.05 * delta } else { 0.25 * delta };
                    let cmd = if cw { "transport.fast_forward" } else { "transport.rewind" };
                    run(app, cmd, json!({"seconds": step_secs}));
                    let dir_txt = if cw { "Forward ▶" } else { "◀ Rewind" };
                    let mode_txt = if self.scrub_active { "Scrub" } else { "Jog" };
                    let msg = format!("{mode_txt} {dir_txt} (step {:.2}s)", step_secs);
                    self.last_log_message = Some(msg.clone());
                    return Some(msg);
                }

                // 2. Pan Knobs / V-Pots (CC 16..23)
                if global && cc >= self.config.pan_cc_base && cc < self.config.pan_cc_base + self.config.channels as u8 {
                    let ch = (cc - self.config.pan_cc_base) as usize;
                    if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                        if self.faders_flipped {
                            // FLIP MODE: Knobs control Volume!
                            let cur_db = app.engine.session().track(tid).map(|t| t.mixer.volume_db).unwrap_or(0.0);
                            let delta = if val & 0x40 != 0 { -((val & 0x3F) as f32 * 0.5) } else { (val & 0x3F) as f32 * 0.5 };
                            let new_db = (cur_db + delta).clamp(-60.0, 12.0);
                            apply(app, "mix.volume", &json!({"track": tid.0, "db": new_db}), &format!("fader:{}", tid.0));
                            let msg = format!("V-Pot (Flipped) ch {} -> Track {} Vol = {:.1} dB", ch + 1, tid.0, new_db);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        } else {
                            // NORMAL MODE: Knobs control Pan
                            let cur_pan = app.engine.session().track(tid).and_then(|t| t.mixer.pan.first().copied()).unwrap_or(0.0);
                            let pan = if val & 0x40 != 0 {
                                // MCU Relative counter-clockwise (val = 0x40 | delta)
                                let delta = (val & 0x3F) as f32 * 0.04;
                                (cur_pan - delta).clamp(-1.0, 1.0)
                            } else if val <= 15 {
                                // MCU Relative clockwise (val = delta)
                                let delta = (val & 0x3F) as f32 * 0.04;
                                (cur_pan + delta).clamp(-1.0, 1.0)
                            } else {
                                // Fallback absolute 0..127
                                (val as f32 / 127.0) * 2.0 - 1.0
                            };
                            apply(app, "mix.pan", &json!({"track": tid.0, "pan": pan}), &format!("pan:{}", tid.0));
                            let msg = format!("Pan ch {} -> Track {} = {:.2}", ch + 1, tid.0, pan);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }
                }

                // 3. Send Level Knobs (CC 24..31)
                if global && cc >= self.config.send_cc_base && cc < self.config.send_cc_base + self.config.channels as u8 {
                    let ch = (cc - self.config.send_cc_base) as usize;
                    let db = if val == 0 { -60.0 } else { (val as f32 / 127.0) * 18.0 - 6.0 };
                    if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                        let params = json!({"track": tid.0, "send": 0, "level_db": db});
                        apply(app, "mix.send_level", &params, &format!("send:{}", tid.0));
                        let msg = format!("Send ch {} -> Track {} = {:.1} dB", ch + 1, tid.0, db);
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }
                }

                // 4. Fallback 7-bit Faders (CC 0..7)
                let fader_cc_end = self.config.fader_cc_base.saturating_add(self.config.channels as u8);
                if global && !self.config.fader_pitch_bend && cc >= self.config.fader_cc_base && cc < fader_cc_end {
                    let ch = (cc - self.config.fader_cc_base) as usize;
                    let db = if val == 0 { -60.0 } else { (val as f32 / 127.0) * 72.0 - 60.0 };
                    if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                        apply(app, "mix.volume", &json!({"track": tid.0, "db": db}), &format!("fader:{}", tid.0));
                        let msg = format!("Fader CC ch {} -> Track {} = {:.1} dB", ch + 1, tid.0, db);
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }
                }

                // 5. Kontakt / Instrument Macros
                if global
                    && self.config.kontakt_macro_cc.contains(&cc)
                    && let Some(target) = crate::virtual_keyboard::active_target_track(app)
                {
                    app.send_live_midi(target, [0xB0 | (channel as u8 & 0x0F), cc, val]);
                    let msg = format!("Kontakt CC {} = {} routed to Track {}", cc, val, target.0);
                    self.last_log_message = Some(msg.clone());
                    return Some(msg);
                }
            }

            // Note On (Buttons: Mute, Solo, Arm, Select, Transport, V-Pot click, Fader touch, Flip, Scrub, Marker, Automation)
            0x90 => {
                let note = d1;
                let vel = d2;

                // Fader Touch Detection (MCU notes 104..111 for ch 1..8, 112 for Master)
                if (104..=112).contains(&note) {
                    let idx = (note - 104) as usize;
                    self.fader_touched[idx] = vel > 0;
                    return Some(format!("MCU Fader {} Touch = {}", idx + 1, vel > 0));
                }

                if vel > 0 {
                    // Flip Button (Note 50): Swap Fader & Knob roles
                    if note == self.config.flip_note {
                        self.faders_flipped = !self.faders_flipped;
                        let flip_val = if self.faders_flipped { 127 } else { 0 };
                        self.queue([0x90, self.config.flip_note, flip_val]);
                        let state = if self.faders_flipped { "Flipped (Faders=Pan, Knobs=Vol)" } else { "Normal (Faders=Vol, Knobs=Pan)" };
                        let msg = format!("MCU Flip: {state}");
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }

                    // Scrub Button (Note 101): Toggle fine scrub vs timeline jog
                    if note == self.config.scrub_note {
                        self.scrub_active = !self.scrub_active;
                        let scrub_val = if self.scrub_active { 127 } else { 0 };
                        self.queue([0x90, self.config.scrub_note, scrub_val]);
                        let msg = format!("MCU Scrub: {}", if self.scrub_active { "Active (Audio Scrub)" } else { "Off (Timeline Jog)" });
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }

                    // Marker Button (Note 84): Drop marker at playhead
                    if note == self.config.marker_note {
                        run(app, "markers.add", json!({}));
                        let msg = "MCU Marker: Location added at playhead".to_string();
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }

                    // Automation Mode Buttons (Notes 74..79: Read, Write, Touch, Latch, Trim, Off)
                    let auto_mode = if note == self.config.auto_read_note {
                        Some("read")
                    } else if note == self.config.auto_write_note {
                        Some("write")
                    } else if note == self.config.auto_touch_note {
                        Some("touch")
                    } else if note == self.config.auto_latch_note {
                        Some("latch")
                    } else if note == self.config.auto_trim_note {
                        Some("trim")
                    } else if note == self.config.auto_off_note {
                        Some("off")
                    } else {
                        None
                    };
                    if let Some(mode) = auto_mode
                        && let Some(target) = app.engine.session().edit.selected_tracks.first().copied()
                    {
                        run(app, "mix.automation_mode", json!({"tracks": [target.0], "mode": mode}));
                        let msg = format!("Track {} Automation Mode -> {}", target.0, mode.to_uppercase());
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }

                    // Mute buttons (Notes 16..23)
                    if note >= self.config.mute_note_base && note < self.config.mute_note_base + self.config.channels as u8 {
                        let ch = (note - self.config.mute_note_base) as usize;
                        if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                            let cur = app.engine.session().track(tid).is_some_and(|t| t.mixer.mute);
                            run(app, "mix.mute", json!({"track": tid.0, "value": !cur}));
                            let msg = format!("Mute toggle ch {} -> Track {}", ch + 1, tid.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // Solo buttons (Notes 8..15)
                    if note >= self.config.solo_note_base && note < self.config.solo_note_base + self.config.channels as u8 {
                        let ch = (note - self.config.solo_note_base) as usize;
                        if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                            let cur = app.engine.session().track(tid).is_some_and(|t| t.mixer.solo);
                            run(app, "mix.solo", json!({"track": tid.0, "value": !cur}));
                            if !cur {
                                app.engine.session_mut().edit.selected_tracks = vec![tid];
                            }
                            let msg = format!("Solo toggle ch {} -> Track {}", ch + 1, tid.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // Record Arm buttons (Notes 0..7)
                    if note >= self.config.arm_note_base && note < self.config.arm_note_base + self.config.channels as u8 {
                        let ch = (note - self.config.arm_note_base) as usize;
                        if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                            let cur = app.engine.session().track(tid).is_some_and(|t| t.mixer.record_arm);
                            run(app, "mix.record_arm", json!({"track": tid.0, "value": !cur}));
                            let msg = format!("Record Arm ch {} -> Track {}", ch + 1, tid.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // Select Track buttons (Notes 24..31)
                    if note >= self.config.select_note_base && note < self.config.select_note_base + self.config.channels as u8 {
                        let ch = (note - self.config.select_note_base) as usize;
                        if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                            app.engine.session_mut().edit.selected_tracks = vec![tid];
                            self.reset_pickups();
                            let msg = format!("Select Track ch {} -> Track {}", ch + 1, tid.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // V-Pot Click (MCU Notes 32..39: push knob to reset Pan to Center)
                    if note >= self.config.vpot_click_base && note < self.config.vpot_click_base + self.config.channels as u8 {
                        let ch = (note - self.config.vpot_click_base) as usize;
                        if let Some(tid) = self.track_for_channel(app.engine.session(), ch) {
                            apply(app, "mix.pan", &json!({"track": tid.0, "pan": 0.0}), &format!("pan:{}", tid.0));
                            let msg = format!("V-Pot Push ch {} -> Track {} Pan Centered", ch + 1, tid.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // Bank Navigation (Notes 46, 47)
                    if note == self.config.bank_left_note {
                        self.active_bank = self.active_bank.saturating_sub(1);
                        self.reset_pickups();
                        let msg = format!("MCU Bank shifted left to Bank {}", self.active_bank + 1);
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }
                    if note == self.config.bank_right_note {
                        self.active_bank = (self.active_bank + 1).min(8);
                        self.reset_pickups();
                        let msg = format!("MCU Bank shifted right to Bank {}", self.active_bank + 1);
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }

                    // Channel Step Navigation (Notes 48, 49)
                    if note == self.config.ch_left_note
                        && let Some(active) = app.engine.session().edit.selected_tracks.first().copied()
                    {
                        let all = &app.engine.session().tracks;
                        if let Some(idx) = all.iter().position(|t| t.id == active)
                            && idx > 0
                        {
                            let prev = all[idx - 1].id;
                            app.engine.session_mut().edit.selected_tracks = vec![prev];
                            let msg = format!("MCU Channel shifted left -> Track {}", prev.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }
                    if note == self.config.ch_right_note
                        && let Some(active) = app.engine.session().edit.selected_tracks.first().copied()
                    {
                        let all = &app.engine.session().tracks;
                        if let Some(idx) = all.iter().position(|t| t.id == active)
                            && idx + 1 < all.len()
                        {
                            let next = all[idx + 1].id;
                            app.engine.session_mut().edit.selected_tracks = vec![next];
                            let msg = format!("MCU Channel shifted right -> Track {}", next.0);
                            self.last_log_message = Some(msg.clone());
                            return Some(msg);
                        }
                    }

                    // Transport Buttons
                    if note == self.config.play_note {
                        run(app, "transport.play", json!({}));
                        self.last_log_message = Some("Transport: Play".to_string());
                        return Some("Transport: Play".to_string());
                    }
                    if note == self.config.stop_note {
                        run(app, "transport.stop", json!({}));
                        self.last_log_message = Some("Transport: Stop".to_string());
                        return Some("Transport: Stop".to_string());
                    }
                    if note == self.config.rec_note {
                        run(app, "transport.record", json!({}));
                        self.last_log_message = Some("Transport: Record".to_string());
                        return Some("Transport: Record".to_string());
                    }
                    if note == self.config.loop_note {
                        let cur = app.engine.session().edit.loop_playback;
                        run(app, "options.loop_playback", json!({}));
                        let msg = format!("Transport: Loop toggle ({})", !cur);
                        self.last_log_message = Some(msg.clone());
                        return Some(msg);
                    }
                    if note == self.config.rewind_note {
                        run(app, "transport.rewind", json!({}));
                        self.last_log_message = Some("Transport: Rewind".to_string());
                        return Some("Transport: Rewind".to_string());
                    }
                    if note == self.config.forward_note {
                        run(app, "transport.fast_forward", json!({}));
                        self.last_log_message = Some("Transport: Fast Forward".to_string());
                        return Some("Transport: Fast Forward".to_string());
                    }

                    // A control surface's stray notes are not music: a keyboard has its own port
                    // (see `play_live_note`). Routing them here flooded the instrument queues.
                } else {
                    // Note-on with velocity 0 is a note-off; the surface has nothing to release.
                }
            }

            // Note Off
            0x80 => {
                let note = d1;
                // Fader Touch Release
                if (104..=112).contains(&note) {
                    let idx = (note - 104) as usize;
                    self.fader_touched[idx] = false;
                    return Some(format!("MCU Fader {} Release", idx + 1));
                }
                // Note-off from the surface: nothing to release (it plays no notes).
            }

            _ => {}
        }
        // Nothing above claimed this message: still show it, so a device whose map differs
        // (e.g. 7-bit CC faders while the profile expects pitch bend) is visible in the
        // controller window instead of looking dead.
        self.last_log_message = Some(format!("Unmapped MIDI input {:02X} {:02X} {:02X}", status, d1, d2));
        None
    }

    /// Synchronizes Soundcraft DAW state back to the physical MSC Mixer (LEDs, meters, and motorized faders).
    pub fn sync_feedback(
        &mut self,
        session: &soundcraft_model::Session,
        is_playing: bool,
        is_recording: bool,
        meters: &HashMap<TrackId, crate::MeterDisplay>,
    ) {
        if !self.config.bidirectional_led {
            return;
        }
        let s = session;
        for ch in 0..self.config.channels {
            if let Some(tid) = self.track_for_channel(session, ch)
                && let Some(tr) = s.track(tid)
            {
                // 1. Mute LED
                let mute_note = self.config.mute_note_base + ch as u8;
                let mute_val = if tr.mixer.mute { 127 } else { 0 };
                if self.last_sent_feedback.get(&mute_note) != Some(&mute_val) {
                    self.last_sent_feedback.insert(mute_note, mute_val);
                    self.queue([0x90, mute_note, mute_val]);
                }

                // 2. Solo LED
                let solo_note = self.config.solo_note_base + ch as u8;
                let solo_val = if tr.mixer.solo { 127 } else { 0 };
                if self.last_sent_feedback.get(&solo_note) != Some(&solo_val) {
                    self.last_sent_feedback.insert(solo_note, solo_val);
                    self.queue([0x90, solo_note, solo_val]);
                }

                // 3. Record Arm LED
                let arm_note = self.config.arm_note_base + ch as u8;
                let arm_val = if tr.mixer.record_arm { 127 } else { 0 };
                if self.last_sent_feedback.get(&arm_note) != Some(&arm_val) {
                    self.last_sent_feedback.insert(arm_note, arm_val);
                    self.queue([0x90, arm_note, arm_val]);
                }

                // 4. Motorized Fader feedback (14-bit Pitch Bend, only when not physically touched)
                if self.config.motorized_faders && self.config.fader_pitch_bend && !self.fader_touched[ch] {
                    let norm = if self.faders_flipped {
                        // Fliped: Send Pan position
                        let pan = tr.mixer.pan.first().copied().unwrap_or(0.0);
                        (pan * 0.5 + 0.5).clamp(0.0, 1.0)
                    } else {
                        // Normal: Send Volume
                        self.norm_from_db(tr.mixer.volume_db)
                    };
                    let val14 = (norm * 16383.0).clamp(0.0, 16383.0).round() as u16;
                    let lsb = (val14 & 0x7F) as u8;
                    let msb = ((val14 >> 7) & 0x7F) as u8;
                    let ch_u8 = ch as u8;
                    if self.last_fader_feedback.get(&ch_u8) != Some(&val14) {
                        self.last_fader_feedback.insert(ch_u8, val14);
                        self.queue([0xE0 | (ch_u8 & 0x0F), lsb, msb]);
                    }
                }

                // 5. 12-Segment Peak Meter Feedback (MCU Channel Pressure 0xD0 | ch)
                if self.config.mcu_led_meters {
                    let (peak, clip) = meters.get(&tid).map(|md| (md.level[0].max(md.level[1]), md.clip)).unwrap_or((0.0, false));
                    let level_seg = if peak <= 0.001 {
                        0u8
                    } else {
                        let db = 20.0 * peak.log10();
                        if db < -48.0 {
                            1
                        } else if db < -36.0 {
                            2
                        } else if db < -28.0 {
                            3
                        } else if db < -20.0 {
                            4
                        } else if db < -14.0 {
                            5
                        } else if db < -10.0 {
                            6
                        } else if db < -6.0 {
                            7
                        } else if db < -4.0 {
                            8
                        } else if db < -2.0 {
                            9
                        } else if db < -1.0 {
                            10
                        } else if db < 0.0 {
                            11
                        } else {
                            12
                        }
                    };
                    let clip_bit = if clip || peak >= 1.0 { 0x10 } else { 0x00 };
                    let meter_byte = clip_bit | (level_seg & 0x0F);
                    if self.last_meter_bytes[ch] != meter_byte {
                        self.last_meter_bytes[ch] = meter_byte;
                        self.queue([0xD0 | (ch as u8 & 0x07), meter_byte, 0]);
                    }
                }
            }
        }

        // 6. Motorized Master Fader (channel 8 / 0xE8)
        if self.config.motorized_faders
            && self.config.fader_pitch_bend
            && !self.fader_touched[8]
            && let Some(master) = session.tracks.iter().find(|t| t.kind == soundcraft_model::TrackKind::Master)
        {
            let norm = self.norm_from_db(master.mixer.volume_db);
            let val14 = (norm * 16383.0).clamp(0.0, 16383.0).round() as u16;
            let lsb = (val14 & 0x7F) as u8;
            let msb = ((val14 >> 7) & 0x7F) as u8;
            let ch_u8 = self.config.master_fader_channel;
            if self.last_fader_feedback.get(&ch_u8) != Some(&val14) {
                self.last_fader_feedback.insert(ch_u8, val14);
                self.queue([0xE0 | (ch_u8 & 0x0F), lsb, msb]);
            }
        }

        // 7. Transport LEDs (Play=94, Stop=93, Record=95, Loop=86)
        let is_looping = session.edit.loop_playback;

        let play_val = if is_playing { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.play_note) != Some(&play_val) {
            self.last_sent_feedback.insert(self.config.play_note, play_val);
            self.queue([0x90, self.config.play_note, play_val]);
        }

        let stop_val = if !is_playing { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.stop_note) != Some(&stop_val) {
            self.last_sent_feedback.insert(self.config.stop_note, stop_val);
            self.queue([0x90, self.config.stop_note, stop_val]);
        }

        let rec_val = if is_recording { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.rec_note) != Some(&rec_val) {
            self.last_sent_feedback.insert(self.config.rec_note, rec_val);
            self.queue([0x90, self.config.rec_note, rec_val]);
        }

        let loop_val = if is_looping { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.loop_note) != Some(&loop_val) {
            self.last_sent_feedback.insert(self.config.loop_note, loop_val);
            self.queue([0x90, self.config.loop_note, loop_val]);
        }

        // 8. Scrub & Flip Mode LEDs
        let scrub_val = if self.scrub_active { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.scrub_note) != Some(&scrub_val) {
            self.last_sent_feedback.insert(self.config.scrub_note, scrub_val);
            self.queue([0x90, self.config.scrub_note, scrub_val]);
        }
        let flip_val = if self.faders_flipped { 127 } else { 0 };
        if self.last_sent_feedback.get(&self.config.flip_note) != Some(&flip_val) {
            self.last_sent_feedback.insert(self.config.flip_note, flip_val);
            self.queue([0x90, self.config.flip_note, flip_val]);
        }

        // 9. Selected Track Automation Mode LEDs
        if let Some(target) = session.edit.selected_tracks.first()
            && let Some(tr) = session.track(*target)
        {
            let mode = tr.mixer.automation_mode;
            use soundcraft_model::AutomationMode;
            let auto_leds = [
                (self.config.auto_read_note, mode == AutomationMode::Read),
                (self.config.auto_write_note, mode == AutomationMode::Write),
                (self.config.auto_touch_note, mode == AutomationMode::Touch),
                (self.config.auto_latch_note, mode == AutomationMode::Latch),
                (self.config.auto_trim_note, mode == AutomationMode::Trim),
                (self.config.auto_off_note, mode == AutomationMode::Off),
            ];
            for (note, is_active) in auto_leds {
                let val = if is_active { 127 } else { 0 };
                if self.last_sent_feedback.get(&note) != Some(&val) {
                    self.last_sent_feedback.insert(note, val);
                    self.queue([0x90, note, val]);
                }
            }
        }
    }

    /// Applies a learned control to its parameter. `d1`/`d2` are the raw message bytes, so the
    /// same binding works for faders (14-bit), knobs (7-bit) and buttons.
    fn apply_learned(&mut self, app: &mut SoundApp, binding: &MidiBinding, d1: u8, d2: u8) -> Option<String> {
        let n = if binding.status == 0xE0 { f32::from((u16::from(d2) << 7) | u16::from(d1)) / 16383.0 } else { f32::from(d2 & 0x7F) / 127.0 };
        let (min, max) = match &binding.target {
            LearnTarget::InsertParam { min, max, .. } | LearnTarget::InstrumentParam { min, max, .. } => (*min, *max),
            _ => (0.0, 1.0),
        };
        let plain = min + n * (max - min);
        let outcome = match &binding.target {
            LearnTarget::Volume { track } => {
                let db = self.db_from_norm(n);
                app.engine
                    .execute_merged("mix.volume", &json!({"track": track, "db": db}), &format!("fader:{track}"))
                    .map(|_| format!("{db:.1} dB"))
                    .map_err(|e| e.to_string())
            }
            LearnTarget::Pan { track, index } => {
                let pan = (n * 2.0 - 1.0).clamp(-1.0, 1.0);
                app.engine
                    .execute_merged("mix.pan", &json!({"track": track, "pan": pan, "index": index}), &format!("pan:{track}:{index}"))
                    .map(|_| format!("pan {pan:.2}"))
                    .map_err(|e| e.to_string())
            }
            LearnTarget::SendLevel { track, slot } => {
                let db = self.db_from_norm(n);
                app.run("mix.send_level", json!({"track": track, "slot": slot, "level_db": db})).map(|_| format!("send {db:.1} dB"))
            }
            LearnTarget::InsertParam { track, slot, param, .. } => app
                .run("mix.insert_param", json!({"track": track, "slot": slot, "param": param, "value": plain}))
                .map(|_| format!("`{param}` = {plain:.3}")),
            LearnTarget::InstrumentParam { track, param, .. } => {
                app.run("mix.instrument_param", json!({"track": track, "param": param, "value": plain})).map(|_| format!("`{param}` = {plain:.3}"))
            }
        };
        let target = binding.target.describe();
        let msg = match outcome {
            Ok(v) => format!("{} -> {target}: {v}", binding.describe()),
            Err(e) => format!("{} -> {target} failed: {e}", binding.describe()),
        };
        self.last_log_message = Some(msg.clone());
        Some(msg)
    }

    /// Drain queued output packets (sent to physical port or test listener).
    pub fn drain_output(&mut self) -> Vec<[u8; 3]> {
        std::mem::take(&mut self.output_queue)
    }

    /// Report current diagnostic status.
    pub fn diagnostic_status(&self, app: &SoundApp) -> Value {
        let active_tracks: Vec<Value> = (0..self.config.channels)
            .map(|ch| {
                let tid = self.track_for_channel(app.engine.session(), ch);
                let tr = tid.and_then(|id| app.engine.session().track(id));
                json!({
                    "channel": ch + 1,
                    "bank": self.active_bank,
                    "track_id": tid.map(|id| id.0),
                    "track_name": tr.map(|t| t.name.as_str()).unwrap_or("<unassigned>"),
                    "volume_db": tr.map(|t| t.mixer.volume_db).unwrap_or(-60.0),
                    "mute": tr.map(|t| t.mixer.mute).unwrap_or(false),
                    "solo": tr.map(|t| t.mixer.solo).unwrap_or(false),
                    "record_arm": tr.map(|t| t.mixer.record_arm).unwrap_or(false)
                })
            })
            .collect();

        json!({
            "device_name": self.config.device_name,
            "connected": self.connected,
            "port": self.port_name,
            "connection_type": self.connection_type,
            "protocol": self.config.protocol,
            "active_bank": self.active_bank,
            "controlled_tracks": active_tracks,
            "bidirectional_sync": {
                "led_feedback": self.config.bidirectional_led,
                "motorized_faders": self.config.motorized_faders
            }
        })
    }
}
