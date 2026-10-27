//! SoundFont Player: multi-sampled instrument playing SoundFont 2 (.sf2) banks.
//!
//! Maps notes and velocities to the active preset's multi-sample zones with per-zone root key,
//! tuning, velocity curves, attenuation, pan, loop points and envelopes. Renders with 4-point
//! cubic Hermite interpolation across up to 64 polyphonic voices.

use std::sync::Arc;

use crate::biquad::{Biquad, FilterType};
use crate::params::{Params, lin, log, param_plumbing};
use crate::sf2::{LoopMode, ResolvedZone, SoundFont};
use crate::util::{Smoother, frames_in, ms_to_samples, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, PluginProgramReader, PluginStateReader, ProgramList, Unit, db_to_gain};

const PRESET: usize = 0;
const BANK: usize = 1;
const LEVEL: usize = 2;
const TRANSPOSE: usize = 3;
const TUNE: usize = 4;
const ATTACK: usize = 5;
const RELEASE: usize = 6;
const CUTOFF: usize = 7;

static SOUNDFONT_PARAMS: [ParamInfo; 8] = [
    lin("preset", "Preset", 0.0, 127.0, 0.0, Unit::None),
    lin("bank", "Bank", 0.0, 128.0, 0.0, Unit::None),
    lin("level", "Level", -48.0, 12.0, 0.0, Unit::Db),
    lin("transpose", "Transpose", -36.0, 36.0, 0.0, Unit::Semitones),
    lin("tune", "Fine Tune", -100.0, 100.0, 0.0, Unit::Cents),
    log("attack", "Attack Scale", 0.1, 5000.0, 100.0, Unit::Percent),
    log("release", "Release Scale", 1.0, 10000.0, 100.0, Unit::Percent),
    log("cutoff", "Filter Cutoff", 20.0, 20000.0, 20000.0, Unit::Hz),
];

pub static SOUNDFONT_INFO: PluginInfo = PluginInfo {
    id: "soundfont",
    name: "SoundFont Player",
    short_name: "SoundFnt",
    category: Category::Instrument,
    params: &SOUNDFONT_PARAMS,
    is_instrument: true,
    audiosuite: false,
};

const VOICES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
enum EnvStage {
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
struct Voice {
    note: u8,
    held: bool,
    sustained: bool,
    zone: Option<ResolvedZone>,
    sample_idx: usize,
    pos: f64,
    step: f64,
    gain_l: f32,
    gain_r: f32,
    env_stage: EnvStage,
    env_val: f32,
    env_samples: usize,
    stage_samples: usize,
    filter_l: Biquad,
    filter_r: Biquad,
}

impl Voice {
    fn new() -> Self {
        Self {
            note: 0,
            held: false,
            sustained: false,
            zone: None,
            sample_idx: 0,
            pos: 0.0,
            step: 1.0,
            gain_l: 0.0,
            gain_r: 0.0,
            env_stage: EnvStage::Idle,
            env_val: 0.0,
            env_samples: 0,
            stage_samples: 0,
            filter_l: Biquad::new(crate::biquad::Coeffs::IDENTITY),
            filter_r: Biquad::new(crate::biquad::Coeffs::IDENTITY),
        }
    }

    fn active(&self) -> bool {
        self.env_stage != EnvStage::Idle
    }

    fn stop(&mut self) {
        self.env_stage = EnvStage::Idle;
        self.env_val = 0.0;
        self.held = false;
        self.sustained = false;
        self.zone = None;
    }
}

/// Hermite 4-point cubic interpolation.
#[inline]
fn hermite(x: &[f32], pos: f64) -> f32 {
    let i = pos as usize;
    let t = (pos - i as f64) as f32;
    let at = |k: usize| x.get(k).copied().unwrap_or(0.0);
    let x0 = at(i);
    if t == 0.0 {
        return if x0.is_finite() { x0 } else { 0.0 };
    }
    let xm1 = at(i.saturating_sub(1));
    let x1 = at(i.saturating_add(1));
    let x2 = at(i.saturating_add(2));
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    let y = ((c3 * t + c2) * t + c1) * t + x0;
    if y.is_finite() { y } else { 0.0 }
}

pub struct SoundFontPlayer {
    p: Params,
    sr: f32,
    ch: usize,
    soundfont: Option<Arc<SoundFont>>,
    soundfont_path: Option<String>,
    voices: Vec<Voice>,
    level: Smoother,
    pitch_bend: f64,
    sustain_pedal: bool,
}

impl Default for SoundFontPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundFontPlayer {
    pub fn new() -> Self {
        let mut v = Vec::with_capacity(VOICES);
        for _ in 0..VOICES {
            v.push(Voice::new());
        }
        Self {
            p: Params::new(&SOUNDFONT_PARAMS),
            sr: 48000.0,
            ch: 2,
            soundfont: None,
            soundfont_path: None,
            voices: v,
            level: Smoother::new(1.0),
            pitch_bend: 0.0,
            sustain_pedal: false,
        }
    }

    fn update(&mut self) {
        self.level.set(db_to_gain(self.p.v(LEVEL)));
    }

    pub fn load_soundfont(&mut self, sf: Arc<SoundFont>, path: Option<String>) {
        self.soundfont = Some(sf);
        self.soundfont_path = path;
        for v in &mut self.voices {
            v.stop();
        }
    }

    pub fn load_file(&mut self, path: &str) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let sf = SoundFont::parse(&bytes)?;
        self.load_soundfont(Arc::new(sf), Some(path.to_string()));
        Ok(())
    }

    fn active_preset_idx(&self) -> Option<usize> {
        let sf = self.soundfont.as_ref()?;
        if sf.presets.is_empty() {
            return None;
        }
        let b = self.p.v(BANK).round() as u16;
        let pr = self.p.v(PRESET).round() as u16;

        // Try exact bank + preset
        if let Some(i) = sf.presets.iter().position(|p| p.bank == b && p.preset == pr) {
            return Some(i);
        }
        // Fallback to bank 0 + preset
        if let Some(i) = sf.presets.iter().position(|p| p.bank == 0 && p.preset == pr) {
            return Some(i);
        }
        // Fallback to first
        Some(0)
    }

    fn start_note(&mut self, note: u8, vel: u8) {
        if vel == 0 {
            self.stop_note(note);
            return;
        }
        let Some(sf) = self.soundfont.clone() else { return };
        let Some(p_idx) = self.active_preset_idx() else { return };
        let Some(preset) = sf.presets.get(p_idx) else { return };

        let note = note.min(127);
        let vel = vel.min(127);

        // Find matching zones
        let mut matched = Vec::new();
        for z in &preset.zones {
            if note >= z.key_lo && note <= z.key_hi && vel >= z.vel_lo && vel <= z.vel_hi {
                matched.push(z.clone());
            }
        }
        if matched.is_empty() {
            return;
        }

        let transpose = f64::from(self.p.v(TRANSPOSE));
        let tune = f64::from(self.p.v(TUNE)) / 100.0;
        let vel_scale = f32::from(vel) / 127.0;

        for zone in matched {
            let Some(sample) = sf.samples.get(zone.sample_idx) else { continue };
            if sample.data.is_empty() {
                continue;
            }

            let semis = f64::from(note as i32 - zone.root_key as i32) * (zone.scale_tuning / 100.0)
                + zone.tuning_semitones
                + transpose
                + tune
                + self.pitch_bend;
            let step = (f64::from(sample.sample_rate) / f64::from(self.sr)) * 2f64.powf(semis / 12.0);
            if !step.is_finite() || step <= 0.0 {
                continue;
            }

            // Allocate a voice
            let v_idx = self.voices.iter().position(|v| !v.active()).unwrap_or(0);
            let v = match self.voices.get_mut(v_idx) {
                Some(v) => v,
                None => continue,
            };

            let att = zone.attenuation_gain * vel_scale;
            // Panning: zone.pan is -1.0 (left) .. +1.0 (right)
            let pan_l = ((1.0 - zone.pan) * 0.5).clamp(0.0, 1.0);
            let pan_r = ((1.0 + zone.pan) * 0.5).clamp(0.0, 1.0);

            let att_sec = (zone.attack_s * (self.p.v(ATTACK) / 100.0)).clamp(0.0001, 10.0);

            v.note = note;
            v.held = true;
            v.sustained = false;
            v.sample_idx = zone.sample_idx;
            v.pos = 0.0;
            v.step = step;
            v.gain_l = att * pan_l;
            v.gain_r = att * pan_r;
            v.env_stage = EnvStage::Attack;
            v.env_val = 0.0;
            v.env_samples = 0;
            v.stage_samples = (ms_to_samples(att_sec * 1000.0, self.sr).round() as usize).max(1);

            let cutoff = self.p.v(CUTOFF).clamp(20.0, 20000.0);
            v.filter_l = Biquad::design(FilterType::LowPass, cutoff, 0.707, 0.0, self.sr);
            v.filter_r = Biquad::design(FilterType::LowPass, cutoff, 0.707, 0.0, self.sr);

            v.zone = Some(zone);
        }
    }

    fn stop_note(&mut self, note: u8) {
        let rel_scale = (self.p.v(RELEASE) / 100.0).clamp(0.01, 100.0);
        for v in &mut self.voices {
            if v.active() && v.note == note && v.held {
                v.held = false;
                if self.sustain_pedal {
                    v.sustained = true;
                } else {
                    v.env_stage = EnvStage::Release;
                    v.env_samples = 0;
                    if let Some(zone) = &v.zone {
                        let rel_sec = (zone.release_s * rel_scale).clamp(0.001, 20.0);
                        v.stage_samples = (ms_to_samples(rel_sec * 1000.0, self.sr).round() as usize).max(1);
                    } else {
                        v.stage_samples = 1;
                    }
                }
            }
        }
    }

    fn release_sustain(&mut self) {
        let rel_scale = (self.p.v(RELEASE) / 100.0).clamp(0.01, 100.0);
        for v in &mut self.voices {
            if v.active() && v.sustained && !v.held {
                v.sustained = false;
                v.env_stage = EnvStage::Release;
                v.env_samples = 0;
                if let Some(zone) = &v.zone {
                    let rel_sec = (zone.release_s * rel_scale).clamp(0.001, 20.0);
                    v.stage_samples = (ms_to_samples(rel_sec * 1000.0, self.sr).round() as usize).max(1);
                } else {
                    v.stage_samples = 1;
                }
            }
        }
    }
}

impl Plugin for SoundFontPlayer {
    param_plumbing!(SOUNDFONT_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sane_sr(sample_rate);
        self.ch = sane_channels(channels);
        self.update();
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.stop();
        }
    }

    fn note_on(&mut self, _offset: usize, note: u8, velocity: u8) {
        self.start_note(note, velocity);
    }

    fn note_off(&mut self, _offset: usize, note: u8) {
        self.stop_note(note);
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            v.stop();
        }
    }

    fn midi(&mut self, _offset: usize, msg: [u8; 3]) {
        let status = msg[0] & 0xF0;
        match status {
            0xB0 => {
                let cc = msg[1];
                let val = msg[2];
                match cc {
                    7 => {
                        let db = (f32::from(val) / 127.0) * 60.0 - 48.0;
                        self.set_param("level", db);
                    }
                    64 => {
                        let down = val >= 64;
                        if self.sustain_pedal && !down {
                            self.sustain_pedal = false;
                            self.release_sustain();
                        } else {
                            self.sustain_pedal = down;
                        }
                    }
                    72 => {
                        let pct = (f32::from(val) / 127.0) * 500.0;
                        self.set_param("release", pct);
                    }
                    73 => {
                        let pct = (f32::from(val) / 127.0) * 500.0;
                        self.set_param("attack", pct);
                    }
                    74 => {
                        let hz = 20.0 * (1000.0_f32).powf(f32::from(val) / 127.0);
                        self.set_param("cutoff", hz);
                    }
                    120 | 123 => {
                        self.all_notes_off();
                    }
                    _ => {}
                }
            }
            0xC0 => {
                let prog = msg[1] as usize;
                self.select_program(0, prog);
            }
            0xE0 => {
                let raw = u16::from(msg[1]) | (u16::from(msg[2]) << 7);
                let norm = (f64::from(raw) - 8192.0) / 8192.0;
                self.pitch_bend = norm * 2.0;
            }
            _ => {}
        }
    }

    fn program_lists(&self) -> Vec<ProgramList> {
        let Some(sf) = &self.soundfont else { return Vec::new() };
        let programs = sf
            .presets
            .iter()
            .map(|p| if p.bank == 0 { format!("{:03} {}", p.preset, p.name) } else { format!("{}:{:03} {}", p.bank, p.preset, p.name) })
            .collect();
        vec![ProgramList { id: 0, name: sf.name.clone(), programs }]
    }

    fn select_program(&mut self, _list: usize, program: usize) -> bool {
        let Some(sf) = &self.soundfont else { return false };
        if let Some(p) = sf.presets.get(program) {
            self.p.set("bank", p.bank as f32);
            self.p.set("preset", p.preset as f32);
            return true;
        }
        false
    }

    fn program_reader(&mut self) -> Option<Box<dyn PluginProgramReader>> {
        let sf = self.soundfont.clone();
        Some(Box::new(move || {
            let Some(sf) = &sf else { return Vec::new() };
            let programs = sf
                .presets
                .iter()
                .map(|p| if p.bank == 0 { format!("{:03} {}", p.preset, p.name) } else { format!("{}:{:03} {}", p.bank, p.preset, p.name) })
                .collect();
            vec![ProgramList { id: 0, name: sf.name.clone(), programs }]
        }))
    }

    fn state_reader(&mut self) -> Option<Box<dyn PluginStateReader>> {
        let path = self.soundfont_path.clone();
        let b = self.p.v(BANK);
        let pr = self.p.v(PRESET);
        Some(Box::new(move || {
            let p = path.as_deref().unwrap_or("");
            let s = format!("{{\"path\":\"{p}\",\"bank\":{b},\"preset\":{pr}}}");
            Some(s.into_bytes())
        }))
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        self.update();
        let n = frames_in(io, frames, self.ch);
        let ch = io.len().min(self.ch);
        if ch == 0 || n == 0 {
            return;
        }

        // Render voices
        let Some(sf) = self.soundfont.clone() else {
            for buf in io.iter_mut().take(ch) {
                buf[..n].fill(0.0);
            }
            return;
        };

        let (l, r) = io.split_at_mut(1);
        let (left_buf, right_buf) = (l.get_mut(0), r.get_mut(0));

        let left = match left_buf {
            Some(l) => &mut l[..n],
            None => return,
        };
        left.fill(0.0);

        let mut right = right_buf.map(|r| &mut r[..n]);
        if let Some(r) = right.as_deref_mut() {
            r.fill(0.0);
        }

        for v in &mut self.voices {
            if !v.active() {
                continue;
            }
            let Some(sample) = sf.samples.get(v.sample_idx) else {
                v.stop();
                continue;
            };
            let s_data = &sample.data;
            if s_data.is_empty() {
                v.stop();
                continue;
            }

            let zone = match v.zone.as_ref() {
                Some(z) => z,
                None => {
                    v.stop();
                    continue;
                }
            };

            let loop_active = match zone.loop_mode {
                LoopMode::Continuous => true,
                LoopMode::LoopDuringHeldKey => v.held || v.sustained,
                LoopMode::Off => false,
            };
            let loop_start = zone.loop_start as f64;
            let loop_end = (zone.loop_end as f64).min(s_data.len() as f64);
            let can_loop = loop_active && (loop_end - loop_start >= 1.0);

            for f in 0..n {
                // Envelope step
                match v.env_stage {
                    EnvStage::Attack => {
                        v.env_val = (v.env_samples as f32 / v.stage_samples as f32).min(1.0);
                        v.env_samples += 1;
                        if v.env_samples >= v.stage_samples {
                            v.env_stage = EnvStage::Hold;
                            v.env_val = 1.0;
                            v.env_samples = 0;
                            v.stage_samples = (ms_to_samples(zone.hold_s * 1000.0, self.sr).round() as usize).max(1);
                        }
                    }
                    EnvStage::Hold => {
                        v.env_val = 1.0;
                        v.env_samples += 1;
                        if v.env_samples >= v.stage_samples {
                            v.env_stage = EnvStage::Decay;
                            v.env_samples = 0;
                            v.stage_samples = (ms_to_samples(zone.decay_s * 1000.0, self.sr).round() as usize).max(1);
                        }
                    }
                    EnvStage::Decay => {
                        let t = (v.env_samples as f32 / v.stage_samples as f32).min(1.0);
                        v.env_val = 1.0 - t * (1.0 - zone.sustain_gain);
                        v.env_samples += 1;
                        if v.env_samples >= v.stage_samples {
                            v.env_stage = EnvStage::Sustain;
                            v.env_val = zone.sustain_gain;
                        }
                    }
                    EnvStage::Sustain => {
                        v.env_val = zone.sustain_gain;
                        if !v.held && !v.sustained {
                            v.env_stage = EnvStage::Release;
                            v.env_samples = 0;
                            let rel_sec = (zone.release_s * (self.p.v(RELEASE) / 100.0)).clamp(0.001, 20.0);
                            v.stage_samples = (ms_to_samples(rel_sec * 1000.0, self.sr).round() as usize).max(1);
                        }
                    }
                    EnvStage::Release => {
                        let t = (v.env_samples as f32 / v.stage_samples as f32).min(1.0);
                        v.env_val = (1.0 - t) * zone.sustain_gain;
                        v.env_samples += 1;
                        if v.env_samples >= v.stage_samples || v.env_val <= 1e-4 {
                            v.stop();
                            break;
                        }
                    }
                    EnvStage::Idle => break,
                }

                // Sample read with cubic hermite interpolation
                let smp = hermite(s_data, v.pos) * v.env_val;

                // Advance position
                v.pos += v.step;
                if can_loop && v.pos >= loop_end {
                    let span = loop_end - loop_start;
                    if span >= 1.0 {
                        v.pos = loop_start + ((v.pos - loop_start) % span);
                    }
                } else if v.pos >= s_data.len() as f64 {
                    v.stop();
                    break;
                }

                let sl = v.filter_l.process_sample(smp * v.gain_l);
                let sr = v.filter_r.process_sample(smp * v.gain_r);

                if let Some(l) = left.get_mut(f) {
                    *l += sl;
                }
                if let Some(r) = right.as_deref_mut().and_then(|buf| buf.get_mut(f)) {
                    *r += sr;
                }
            }
        }

        // Apply master level
        for f in 0..n {
            let gain = self.level.next();
            if let Some(l) = left.get_mut(f) {
                *l *= gain;
            }
            if let Some(r) = right.as_deref_mut().and_then(|buf| buf.get_mut(f)) {
                *r *= gain;
            }
        }
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        let path = self.soundfont_path.as_deref().unwrap_or("");
        let b = self.p.v(BANK);
        let pr = self.p.v(PRESET);
        let s = format!("{{\"path\":\"{path}\",\"bank\":{b},\"preset\":{pr}}}");
        Some(s.into_bytes())
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let Ok(s) = std::str::from_utf8(data) else { return false };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(s) else { return false };
        if let Some(p) = val.get("path").and_then(|v| v.as_str())
            && !p.is_empty()
        {
            let _ = self.load_file(p);
        }
        if let Some(b) = val.get("bank").and_then(|v| v.as_f64()) {
            self.p.set("bank", b as f32);
        }
        if let Some(pr) = val.get("preset").and_then(|v| v.as_f64()) {
            self.p.set("preset", pr as f32);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_soundfont_player_info() {
        let p = SoundFontPlayer::new();
        assert_eq!(p.info().id, "soundfont");
        assert!(p.info().is_instrument);
    }

    #[test]
    fn test_soundfont_player_silent_without_sf() {
        let mut p = SoundFontPlayer::new();
        p.prepare(48000.0, 256, 2);
        p.note_on(0, 60, 100);
        let mut io = vec![vec![1.0; 64], vec![1.0; 64]];
        p.process(&mut io, 64);
        assert_eq!(io[0], vec![0.0; 64]);
        assert_eq!(io[1], vec![0.0; 64]);
    }

    #[test]
    fn test_soundfont_player_real_playback() {
        let path = "/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Clavinet.sf2";
        if std::path::Path::new(path).exists() {
            let mut p = SoundFontPlayer::new();
            p.load_file(path).expect("load clavinet");
            p.prepare(48000.0, 256, 2);
            p.note_on(0, 60, 100);
            let mut io = vec![vec![0.0; 256], vec![0.0; 256]];
            p.process(&mut io, 256);
            let energy: f32 = io[0].iter().map(|s| s * s).sum();
            assert!(energy > 0.0, "should have produced audio energy from Clavinet");
        }
    }
}
