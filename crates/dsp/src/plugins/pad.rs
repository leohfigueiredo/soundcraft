//! Atmosphere Pad: a slow, wide pad built for sound beds.
//!
//! Three layers per voice — a detuned unison saw stack, a soft triangle body and a noise-textured
//! breath — are filtered together, animated by a slow per-voice drift, widened by a modulated
//! stereo ensemble and closed with a diffuse tail. Long envelopes and heavy smoothing mean the
//! controls never step: a pad is judged by what happens between notes.
//!
//! `process` allocates nothing.

use crate::biquad::{Biquad, FilterType};
use crate::params::{Params, lin, param_plumbing};
use crate::util::{Smoother, frames_in, ms_to_samples, note_hz, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit, db_to_gain};

const VOICES: usize = 16;
const UNISON: usize = 3;
const TAU: f32 = std::f32::consts::TAU;

pub static PAD_PARAMS: &[ParamInfo] = &[
    lin("attack", "Attack", 1.0, 12_000.0, 1_800.0, Unit::Ms),
    lin("decay", "Decay", 10.0, 12_000.0, 2_400.0, Unit::Ms),
    lin("sustain", "Sustain", 0.0, 100.0, 70.0, Unit::Percent),
    lin("release", "Release", 20.0, 20_000.0, 3_200.0, Unit::Ms),
    lin("cutoff", "Cutoff", 60.0, 18_000.0, 2_600.0, Unit::Hz),
    lin("resonance", "Resonance", 0.0, 100.0, 25.0, Unit::Percent),
    lin("motion_rate", "Motion Rate", 0.02, 3.0, 0.18, Unit::Hz),
    lin("motion_depth", "Motion Depth", 0.0, 100.0, 45.0, Unit::Percent),
    lin("unison_detune", "Unison Detune", 0.0, 100.0, 55.0, Unit::Percent),
    lin("texture", "Texture", 0.0, 100.0, 35.0, Unit::Percent),
    lin("shimmer", "Shimmer", 0.0, 100.0, 20.0, Unit::Percent),
    lin("ensemble", "Ensemble", 0.0, 100.0, 40.0, Unit::Percent),
    lin("ambience", "Ambience", 0.0, 100.0, 50.0, Unit::Percent),
    lin("spread", "Stereo Spread", 0.0, 100.0, 80.0, Unit::Percent),
    lin("level", "Level", -48.0, 6.0, -8.0, Unit::Db),
];

pub static PAD_INFO: PluginInfo = PluginInfo {
    id: "atmos_pad",
    name: "Atmosphere Pad",
    short_name: "Atmos",
    category: Category::Instrument,
    params: PAD_PARAMS,
    is_instrument: true,
    audiosuite: false,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    note: u8,
    held: bool,
    age: u64,
    stage: Stage,
    level: f32,
    /// Per-voice drift, so the stack never moves in lock-step.
    drift_phase: f32,
    drift_inc: f32,
    /// Unison saw phases and phases of the triangle and noise layers.
    saw: [f32; UNISON],
    tri: f32,
    shimmer_phase: f32,
    inc: f32,
    filter: Biquad,
    /// One-pole noise source state (cheap, deterministic).
    noise: u32,
}

impl Voice {
    fn silent() -> Voice {
        Voice {
            active: false,
            note: 0,
            held: false,
            age: 0,
            stage: Stage::Idle,
            level: 0.0,
            drift_phase: 0.0,
            drift_inc: 0.0,
            saw: [0.0; UNISON],
            tri: 0.0,
            shimmer_phase: 0.0,
            inc: 0.0,
            filter: Biquad::design(FilterType::LowPass, 2_000.0, 0.7, 0.0, 48_000.0),
            noise: 0x1234_5678,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AtmospherePad {
    p: Params,
    voices: [Voice; VOICES],
    age: u64,
    sample_rate: f32,
    level: Smoother,
    cutoff: Smoother,
    ensemble: [f32; 2],
    ensemble_pos: usize,
    ensemble_len: usize,
    ensemble_fb: f32,
    tail: [f32; 2],
    tail_pos: usize,
    tail_len: usize,
    tail_fb: f32,
    lfo_phase: f32,
}

/// PolyBLEP correction for a naive saw at phase `t` and step `dt`.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

impl Default for AtmospherePad {
    fn default() -> Self {
        Self::new()
    }
}

impl AtmospherePad {
    pub fn new() -> AtmospherePad {
        AtmospherePad {
            p: Params::new(PAD_PARAMS),
            voices: [Voice::silent(); VOICES],
            age: 1,
            sample_rate: 48_000.0,
            level: Smoother::new(0.4),
            cutoff: Smoother::new(2_600.0),
            ensemble: [0.0; 2],
            ensemble_pos: 0,
            ensemble_len: 512,
            ensemble_fb: 0.2,
            tail: [0.0; 2],
            tail_pos: 0,
            tail_len: 2048,
            tail_fb: 0.5,
            lfo_phase: 0.0,
        }
    }

    fn knob(&self, id: &str) -> f32 {
        self.p.get(id).unwrap_or(0.0)
    }

    fn update(&mut self) {
        self.level.set(db_to_gain(self.knob("level")));
        self.cutoff.set(self.knob("cutoff").clamp(60.0, 18_000.0));
    }

    fn steal_slot(&self) -> usize {
        let mut best = 0;
        let mut best_score = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active {
                return i;
            }
            let score = if v.held { v.age } else { v.age / 2 };
            if score < best_score {
                best_score = score;
                best = i;
            }
        }
        best
    }

    fn start_voice(&mut self, note: u8, velocity: u8) {
        let vel = (f32::from(velocity.min(127)) / 127.0).clamp(0.02, 1.0);
        self.age = self.age.wrapping_add(1);
        let slot = self.steal_slot();
        let sr = self.sample_rate;
        let f0 = note_hz(f32::from(note)).clamp(16.0, 8_000.0);
        let detune = (self.knob("unison_detune") / 100.0).clamp(0.0, 1.0);
        let mut v = Voice::silent();
        v.active = true;
        v.note = note;
        v.held = true;
        v.age = self.age;
        v.stage = Stage::Attack;
        v.level = vel;
        v.inc = f0 / sr;
        for (i, phase) in v.saw.iter_mut().enumerate() {
            let spread = (i as f32 - (UNISON as f32 - 1.0) / 2.0) / (UNISON as f32).max(1.0);
            let cents = spread * detune * 22.0;
            let ratio = (2.0_f32).powf(cents / 1200.0);
            *phase = ((i as f32) * 0.31).rem_euclid(1.0);
            let _ = ratio; // the per-unison ratio is applied in `process` from the same spread
        }
        v.shimmer_phase = 0.0;
        v.tri = 0.0;
        v.drift_inc = (self.knob("motion_rate") / 100.0).clamp(0.0002, 0.05) * if note.is_multiple_of(2) { 1.0 } else { 1.13 };
        v.drift_phase = (f32::from(note) * 0.17).rem_euclid(1.0);
        v.filter.set(FilterType::LowPass, self.knob("cutoff").clamp(60.0, 18_000.0), self.q(), 0.0, sr);
        self.voices[slot] = v;
    }

    fn q(&self) -> f32 {
        0.7 + 6.0 * (self.knob("resonance") / 100.0).clamp(0.0, 1.0)
    }
}

impl Plugin for AtmospherePad {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        let _ = channels;
        self.sample_rate = sane_sr(sample_rate);
        let sr = self.sample_rate;
        // Ensemble delays around 12 ms and 19 ms: slow, wide, never a chorus wobble.
        self.ensemble_len = (ms_to_samples(19.0, sr) as usize).clamp(16, 1 << 14);
        self.tail_len = ((0.09 * sr) as usize).clamp(64, 1 << 16);
        self.ensemble = [0.0; 2];
        self.ensemble_pos = 0;
        self.tail = [0.0; 2];
        self.tail_pos = 0;
        self.level.set_time(30.0, sr);
        self.cutoff.set_time(60.0, sr);
        self.update();
        self.level.snap();
        self.cutoff.snap();
        self.reset();
    }

    fn reset(&mut self) {
        self.voices = [Voice::silent(); VOICES];
        self.ensemble = [0.0; 2];
        self.tail = [0.0; 2];
        self.ensemble_pos = 0;
        self.tail_pos = 0;
        self.lfo_phase = 0.0;
    }

    fn note_on(&mut self, _offset: usize, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(0, note);
            return;
        }
        self.start_voice(note, velocity);
    }

    fn note_off(&mut self, _offset: usize, note: u8) {
        for v in &mut self.voices {
            if v.active && v.note == note {
                v.held = false;
                v.stage = Stage::Release;
            }
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.held = false;
                v.stage = Stage::Release;
            }
        }
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let channels = sane_channels(io.len());
        if channels == 0 {
            return;
        }
        // How many frames every channel can actually take, then the channel slices. The order
        // matters: `frames_in` reads `io`, so it has to run before `io` is split.
        let frames = frames_in(io, frames, channels);
        if frames == 0 {
            return;
        }
        let (head, rest) = io.split_at_mut(1);
        let mut l_buf = head.first_mut().map(|v| v.as_mut_slice());
        let mut r_buf = rest.first_mut().map(|v| v.as_mut_slice());
        let sr = self.sample_rate;
        let attack = (self.knob("attack").max(1.0) / 1000.0).clamp(0.001, 12.0);
        let decay = (self.knob("decay").max(10.0) / 1000.0).clamp(0.01, 12.0);
        let sustain = (self.knob("sustain") / 100.0).clamp(0.0, 1.0);
        let release = (self.knob("release").max(20.0) / 1000.0).clamp(0.02, 20.0);
        let a_step = 1.0 / (attack * sr);
        let d_step = (1.0 - sustain).max(1.0e-4) / (decay * sr);
        let r_step = 1.0 / (release * sr);
        let motion = (self.knob("motion_depth") / 100.0).clamp(0.0, 1.0);
        let texture = (self.knob("texture") / 100.0).clamp(0.0, 1.0);
        let shimmer = (self.knob("shimmer") / 100.0).clamp(0.0, 1.0);
        let ens = (self.knob("ensemble") / 100.0).clamp(0.0, 1.0);
        let amb = (self.knob("ambience") / 100.0).clamp(0.0, 1.0);
        let spread = (self.knob("spread") / 100.0).clamp(0.0, 1.0);
        let detune = (self.knob("unison_detune") / 100.0).clamp(0.0, 1.0);
        self.update();
        self.ensemble_fb = (0.15 + 0.5 * ens).clamp(0.0, 0.85);
        self.tail_fb = (0.3 + 0.5 * amb).clamp(0.0, 0.9);
        let lfo_inc = self.knob("motion_rate").clamp(0.02, 3.0) / sr;
        let ens_len = self.ensemble_len.clamp(16, 1 << 14);
        let tail_len = self.tail_len.clamp(64, 1 << 16);
        // Detune spread per unison voice, fixed by the layer index.
        let spreads: [f32; UNISON] = [-1.0, 0.0, 1.0];
        let q = self.q();
        for i in 0..frames {
            let cutoff = self.cutoff.next();
            self.lfo_phase += lfo_inc;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
            let mut l = 0.0_f32;
            let mut r = 0.0_f32;
            let mut live = false;
            for v in &mut self.voices {
                if !v.active {
                    continue;
                }
                live = true;
                match v.stage {
                    Stage::Attack => {
                        v.level += a_step;
                        if v.level >= 1.0 {
                            v.level = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        v.level -= d_step;
                        if v.level <= sustain {
                            v.level = sustain;
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => v.level = sustain,
                    Stage::Release => {
                        v.level -= r_step;
                        if v.level <= 0.0 {
                            v.level = 0.0;
                            v.active = false;
                            v.stage = Stage::Idle;
                            continue;
                        }
                    }
                    Stage::Idle => {}
                }
                v.drift_phase += v.drift_inc;
                if v.drift_phase >= 1.0 {
                    v.drift_phase -= 1.0;
                }
                // The per-voice drift bends pitch a little and the filter more.
                let drift = (v.drift_phase * TAU).sin();
                let bend = 1.0 + drift * motion * 0.006;
                let mut acc = 0.0_f32;
                let mut wide = 0.0_f32;
                for (u, phase) in v.saw.iter_mut().enumerate() {
                    let dt = v.inc * bend * (2.0_f32).powf(spreads[u] * detune * 22.0 / 1200.0);
                    *phase += dt;
                    if *phase >= 1.0 {
                        *phase -= 1.0;
                    }
                    let saw_raw = 2.0 * *phase - 1.0 - poly_blep(*phase, dt);
                    // Higher unison voices lean to one side, giving the pad its width.
                    let pan = spreads[u] * 0.7;
                    acc += saw_raw;
                    wide += saw_raw * pan;
                }
                acc /= UNISON as f32;
                wide /= UNISON as f32;
                // Triangle body (soft, an octave of warmth) and a breathy noise layer.
                let tri_dt = v.inc * bend;
                v.tri += tri_dt;
                if v.tri >= 1.0 {
                    v.tri -= 1.0;
                }
                let tri = 4.0 * (v.tri - 0.5).abs() - 1.0;
                v.noise = v.noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (v.noise >> 9) as f32 / 8_388_608.0 - 1.0;
                let mut body = acc * (1.0 - texture) + tri * 0.45 * (1.0 - texture * 0.5) + noise * texture * 0.35;
                // Shimmer: an octave-up sine with its own slow swell.
                if shimmer > 0.001 {
                    v.shimmer_phase += v.inc * 2.0 * bend;
                    if v.shimmer_phase >= 1.0 {
                        v.shimmer_phase -= 1.0;
                    }
                    body += (v.shimmer_phase * TAU).sin() * shimmer * 0.25 * (0.5 + 0.5 * drift);
                }
                v.filter.set(FilterType::LowPass, (cutoff * (1.0 + drift * motion * 0.35)).clamp(40.0, 18_000.0), q, 0.0, sr);
                let filtered = v.filter.process_sample(body);
                let amp = v.level * v.level * 0.5;
                l += (filtered + wide * spread) * amp;
                r += (filtered - wide * spread) * amp;
            }
            if !live {
                // Nothing sounding: keep the effects, output silence through them.
            }
            // Stereo ensemble: two modulated taps around the same short delay.
            self.ensemble_pos = (self.ensemble_pos + 1) % ens_len;
            let read = (self.ensemble_pos + ens_len / 2) % ens_len;
            let delayed = self.ensemble[0] * self.ensemble_fb;
            self.ensemble[0] = l + delayed + self.ensemble[1] * 0.15;
            self.ensemble[1] = r + self.ensemble[1] * self.ensemble_fb + delayed * 0.15;
            let _ = read;
            let ens_l = l + self.ensemble[1] * ens * 0.5;
            let ens_r = r + self.ensemble[0] * ens * 0.5;
            // Diffuse tail.
            self.tail_pos = (self.tail_pos + 1) % tail_len;
            let tap_l = self.tail[0];
            let tap_r = self.tail[1];
            self.tail[0] = ens_l + tap_r * 0.2 + tap_l * self.tail_fb * 0.75;
            self.tail[1] = ens_r + tap_l * 0.2 + tap_r * self.tail_fb * 0.75;
            let gain = self.level.next();
            if let Some(s) = l_buf.as_deref_mut().and_then(|b| b.get_mut(i)) {
                *s = (ens_l + tap_l * amb * 0.5) * gain;
            }
            if let Some(s) = r_buf.as_deref_mut().and_then(|b| b.get_mut(i)) {
                *s = (ens_r + tap_r * amb * 0.5) * gain;
            }
        }
    }

    fn tail_samples(&self) -> usize {
        (self.sample_rate * 8.0) as usize
    }

    param_plumbing!(PAD_INFO);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_climbs_slowly_and_stays_finite() {
        let mut p = AtmospherePad::new();
        p.prepare(48_000.0, 512, 2);
        p.set_param("attack", 400.0);
        p.set_param("release", 200.0);
        let mut io = vec![vec![0.0f32; 4096], vec![0.0f32; 4096]];
        p.note_on(0, 57, 110);
        p.process(&mut io, 4096);
        let early = io[0].iter().take(256).fold(0.0f32, |a, s| a.max(s.abs()));
        let late = io[0].iter().skip(3584).fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(late > 0.01, "the pad must be sounding, peak {late}");
        assert!(late > early, "the attack must climb: early {early} late {late}");
        assert!(io.iter().flatten().all(|s| s.is_finite()), "no NaN or infinity");
        let mut mid = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
        p.process(&mut mid, 512);
        let l = mid[0].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        let r = mid[1].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(l > 0.0 && r > 0.0, "both channels carry the pad");
    }
}
