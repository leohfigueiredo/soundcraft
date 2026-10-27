//! Grand Piano: a struck-string model (modal synthesis), not a sample player.
//!
//! A note excites a bank of inharmonic partials — a stiff string puts partial `n` slightly above
//! `n·f₀` — with a hammer spectrum that brightens with velocity and a comb from the striking point.
//! Higher partials decay first, a released key's damper multiplies those rates, and the sustain
//! pedal holds the note off the damper. A shared body filter and a short diffuse tail stand in for
//! the soundboard.
//!
//! `process` allocates nothing: the voice bank is fixed at construction.

use crate::biquad::{Biquad, FilterType};
use crate::params::{Params, lin, param_plumbing};
use crate::util::{Smoother, frames_in, note_hz, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit, db_to_gain};

/// Partials per string. Eight is where a piano's character lives without costing a sine bank.
const PARTIALS: usize = 8;
/// Simultaneous notes; the least valuable voice is stolen when a further one arrives.
const VOICES: usize = 24;
/// Two strings per note, detuned apart and panned across the stereo field.
const STRINGS: usize = 2;
const SILENT: f32 = 1.0e-6;
const TAU: f32 = std::f32::consts::TAU;

pub static PIANO_PARAMS: &[ParamInfo] = &[
    lin("brightness", "Brightness", 0.0, 100.0, 50.0, Unit::Percent),
    lin("hammer_hardness", "Hammer Hardness", 0.0, 100.0, 50.0, Unit::Percent),
    lin("dynamics", "Dynamic Range", 0.0, 100.0, 60.0, Unit::Percent),
    lin("inharmonicity", "String Tension", 0.0, 100.0, 45.0, Unit::Percent),
    lin("resonance", "Sympathetic", 0.0, 100.0, 50.0, Unit::Percent),
    lin("damper", "Damper", 0.0, 100.0, 60.0, Unit::Percent),
    lin("pedal", "Sustain Pedal", 0.0, 100.0, 0.0, Unit::Percent),
    lin("octave_stretch", "Octave Stretch", 0.0, 100.0, 30.0, Unit::Percent),
    lin("unison_detune", "Unison Detune", 0.0, 100.0, 25.0, Unit::Percent),
    lin("stereo_width", "Stereo Width", 0.0, 100.0, 70.0, Unit::Percent),
    lin("ambience", "Soundboard", 0.0, 100.0, 35.0, Unit::Percent),
    lin("release", "Release", 5.0, 800.0, 220.0, Unit::Ms),
    lin("tune", "Tune", -100.0, 100.0, 0.0, Unit::Cents),
    lin("level", "Level", -48.0, 6.0, -6.0, Unit::Db),
];

pub static PIANO_INFO: PluginInfo = PluginInfo {
    id: "grand_piano",
    name: "Grand Piano",
    short_name: "Grand",
    category: Category::Instrument,
    params: PIANO_PARAMS,
    is_instrument: true,
    audiosuite: false,
};

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    note: u8,
    /// Key still down (the damper is lifted).
    held: bool,
    age: u64,
    phase: [[f32; PARTIALS]; STRINGS],
    inc: [[f32; PARTIALS]; STRINGS],
    amp: [[f32; PARTIALS]; STRINGS],
    decay: [[f32; PARTIALS]; STRINGS],
    hammer: f32,
    hammer_decay: f32,
    hammer_gain: f32,
    hammer_state: f32,
}

impl Voice {
    const SILENT_VOICE: Voice = Voice {
        active: false,
        note: 0,
        held: false,
        age: 0,
        phase: [[0.0; PARTIALS]; STRINGS],
        inc: [[0.0; PARTIALS]; STRINGS],
        amp: [[0.0; PARTIALS]; STRINGS],
        decay: [[1.0; PARTIALS]; STRINGS],
        hammer: 0.0,
        hammer_decay: 1.0,
        hammer_gain: 0.0,
        hammer_state: 0.0,
    };
}

#[derive(Debug, Clone)]
pub struct GrandPiano {
    p: Params,
    voices: [Voice; VOICES],
    age: u64,
    sample_rate: f32,
    level: Smoother,
    pedal: Smoother,
    body_low: Biquad,
    body_peak: Biquad,
    tail: [f32; 2],
    tail_pos: usize,
    tail_len: usize,
    tail_fb: f32,
}

impl Default for GrandPiano {
    fn default() -> Self {
        Self::new()
    }
}

impl GrandPiano {
    pub fn new() -> GrandPiano {
        GrandPiano {
            p: Params::new(PIANO_PARAMS),
            voices: [Voice::SILENT_VOICE; VOICES],
            age: 1,
            sample_rate: 48_000.0,
            level: Smoother::new(1.0),
            pedal: Smoother::new(0.0),
            body_low: Biquad::design(FilterType::LowPass, 7_500.0, 0.707, 0.0, 48_000.0),
            body_peak: Biquad::design(FilterType::Peak, 180.0, 1.1, 3.0, 48_000.0),
            tail: [0.0; 2],
            tail_pos: 0,
            tail_len: 1024,
            tail_fb: 0.3,
        }
    }

    /// A parameter value (`0.0` for an unknown id, which the table makes impossible).
    fn knob(&self, id: &str) -> f32 {
        self.p.get(id).unwrap_or(0.0)
    }

    /// Pushes the smoothed parameters to their targets; `set_param` calls this.
    fn update(&mut self) {
        self.level.set(db_to_gain(self.knob("level")));
        self.pedal.set((self.knob("pedal") / 100.0).clamp(0.0, 1.0));
    }

    fn key_hz(&self, note: u8) -> f32 {
        let base = note_hz(f32::from(note));
        let cents = self.knob("tune");
        let stretch = (self.knob("octave_stretch") / 100.0).clamp(0.0, 1.0);
        // Stretch pushes the extremes apart and leaves the middle of the keyboard alone.
        let off = (f32::from(note) - 60.0) / 60.0;
        let octaves = cents / 1200.0 + off.abs() * off * stretch * 0.03;
        base * (2.0_f32).powf(octaves)
    }

    fn steal_slot(&self) -> usize {
        let mut best = 0;
        let mut best_score = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active {
                return i;
            }
            // A held note is worth more than a released one of the same age.
            let score = if v.held { v.age } else { v.age / 2 };
            if score < best_score {
                best_score = score;
                best = i;
            }
        }
        best
    }

    fn start_voice(&mut self, note: u8, velocity: u8) {
        let vel = (f32::from(velocity.min(127)) / 127.0).clamp(0.01, 1.0);
        self.age = self.age.wrapping_add(1);
        let slot = self.steal_slot();
        let sr = self.sample_rate;
        let f0 = self.key_hz(note).clamp(16.0, 12_000.0);
        // Stiffness: how far above n·f0 the nth partial sits.
        let inharm = 0.000_05 + 0.001_2 * (1.0 - self.knob("inharmonicity") / 100.0).clamp(0.0, 1.0);
        let bright = (self.knob("brightness") / 100.0).clamp(0.0, 1.0);
        let hard = (self.knob("hammer_hardness") / 100.0).clamp(0.0, 1.0);
        let dyn_range = (self.knob("dynamics") / 100.0).clamp(0.0, 1.0);
        // Harder hammer, brighter setting and a harder hit all lift the top partials.
        let tilt = (1.0 - bright) * 0.5 + (1.0 - hard) * 0.3 + (1.0 - vel) * 0.5 * dyn_range;
        // Struck at about an eighth of the string: the partials at that ratio are weak.
        let strike = 0.125;
        // Bass notes ring far longer than treble ones.
        let t60 = (0.8 + 5.5 * (f32::from(note).max(21.0) / 127.0).powf(-0.6)).clamp(0.6, 9.0);
        let detune = (self.knob("unison_detune") / 100.0).clamp(0.0, 1.0);
        let mut v = Voice { active: true, note, held: true, age: self.age, ..Voice::SILENT_VOICE };
        for s in 0..STRINGS {
            let cents = if s == 0 { -detune * 7.0 } else { detune * 7.0 };
            let ratio = (2.0_f32).powf(cents / 1200.0);
            for n in 1..=PARTIALS {
                let nf = n as f32;
                let f = nf * f0 * (1.0 + inharm * nf * nf).sqrt() * ratio;
                if !f.is_finite() || f >= sr * 0.45 {
                    continue;
                }
                let comb = (std::f32::consts::PI * nf * strike).sin().abs().max(0.02);
                let spectral = (-tilt * (nf - 1.0)).exp() / nf.powf(0.9);
                let amp = spectral * comb * vel.powf(1.4) * 0.3;
                let life = t60 / (1.0 + 0.5 * (nf - 1.0));
                let per_sample = (-1000.0_f32.ln() / (life * sr)).exp();
                v.phase[s][n - 1] = (nf * 0.37 + s as f32 * 0.11).rem_euclid(1.0);
                v.inc[s][n - 1] = f / sr;
                v.amp[s][n - 1] = amp;
                v.decay[s][n - 1] = per_sample;
            }
        }
        v.hammer_gain = 0.02 + 0.22 * vel * vel * (0.4 + 0.6 * hard);
        v.hammer = 1.0;
        v.hammer_decay = (-1.0 / (0.004 * sr)).exp();
        self.voices[slot] = v;
    }
}

impl Plugin for GrandPiano {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        let _ = channels;
        self.sample_rate = sane_sr(sample_rate);
        let sr = self.sample_rate;
        self.body_low.set(FilterType::LowPass, 7_500.0, 0.707, 0.0, sr);
        self.body_peak.set(FilterType::Peak, 180.0, 1.1, 3.0, sr);
        self.tail_len = ((0.03 * sr) as usize).clamp(2, 1 << 16);
        self.tail_pos = 0;
        self.tail = [0.0; 2];
        self.level.set_time(20.0, sr);
        self.pedal.set_time(30.0, sr);
        self.update();
        self.level.snap();
        self.pedal.snap();
        self.reset();
    }

    fn reset(&mut self) {
        self.voices = [Voice::SILENT_VOICE; VOICES];
        self.body_low.reset();
        self.body_peak.reset();
        self.tail = [0.0; 2];
        self.tail_pos = 0;
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
            }
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            v.held = false;
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
        let width = (self.knob("stereo_width") / 100.0).clamp(0.0, 1.0);
        let amb = (self.knob("ambience") / 100.0).clamp(0.0, 1.0);
        let damper = (self.knob("damper") / 100.0).clamp(0.0, 1.0);
        let release = (self.knob("release").max(5.0) / 1000.0).clamp(0.001, 2.0);
        // How fast a released key's partials are stopped (the pedal overrides it).
        let damp_rate = (-1000.0_f32.ln() / (release * sr * (0.2 + damper))).exp().clamp(0.9, 0.999_999);
        self.tail_fb = (0.25 + 0.55 * amb).clamp(0.0, 0.9);
        self.update();
        let tail_len = self.tail_len.clamp(2, 1 << 16);
        for i in 0..frames {
            let pedal = self.pedal.next();
            let mut l = 0.0_f32;
            let mut r = 0.0_f32;
            for v in &mut self.voices {
                if !v.active {
                    continue;
                }
                let damped = !v.held && pedal < 0.5;
                let mut mid = 0.0_f32;
                let mut side = 0.0_f32;
                let mut left_any = false;
                for s in 0..STRINGS {
                    for n in 0..PARTIALS {
                        let a = v.amp[s][n];
                        if a <= SILENT {
                            continue;
                        }
                        left_any = true;
                        v.phase[s][n] += v.inc[s][n];
                        if v.phase[s][n] >= 1.0 {
                            v.phase[s][n] -= 1.0;
                        }
                        let x = (v.phase[s][n] * TAU).sin() * a;
                        mid += x;
                        side += if s == 0 { -x } else { x };
                        v.amp[s][n] = if damped { a * v.decay[s][n] * damp_rate } else { a * v.decay[s][n] };
                    }
                }
                if !left_any {
                    v.active = false;
                    continue;
                }
                if v.hammer > 1.0e-4 {
                    v.hammer_state += (v.hammer - v.hammer_state) * 0.35;
                    mid += v.hammer_state * v.hammer_gain * v.hammer;
                    v.hammer *= v.hammer_decay;
                }
                l += mid + side * width;
                r += mid - side * width;
            }
            l = self.body_peak.process_sample(self.body_low.process_sample(l));
            r = self.body_peak.process_sample(self.body_low.process_sample(r));
            self.tail_pos = (self.tail_pos + 1) % tail_len;
            let tap_l = self.tail[0];
            let tap_r = self.tail[1];
            self.tail[0] = l + tap_r * 0.12 + tap_l * self.tail_fb * 0.7;
            self.tail[1] = r + tap_l * 0.12 + tap_r * self.tail_fb * 0.7;
            let gain = self.level.next();
            if let Some(s) = l_buf.as_deref_mut().and_then(|b| b.get_mut(i)) {
                *s = (l + tap_l * amb * 0.4) * gain;
            }
            if let Some(s) = r_buf.as_deref_mut().and_then(|b| b.get_mut(i)) {
                *s = (r + tap_r * amb * 0.4) * gain;
            }
        }
    }

    fn tail_samples(&self) -> usize {
        (self.sample_rate * 4.0) as usize
    }

    param_plumbing!(PIANO_INFO);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_struck_note_rings_and_finishes() {
        let mut p = GrandPiano::new();
        p.prepare(48_000.0, 512, 2);
        let mut io = vec![vec![0.0f32; 1024], vec![0.0f32; 1024]];
        p.note_on(0, 60, 100);
        p.process(&mut io, 1024);
        let peak = io.iter().flatten().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.01, "a struck key must sound, peak {peak}");
        assert!(io.iter().flatten().all(|s| s.is_finite()), "no NaN or infinity");
        // The pedal lifts, then the dampers stop it: after a second of silence it is quiet.
        p.note_off(0, 60);
        p.set_param("release", 5.0);
        let mut tail = vec![vec![0.0f32; 2048], vec![0.0f32; 2048]];
        for _ in 0..24 {
            p.process(&mut tail, 2048);
        }
        let end = tail.iter().flatten().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(end < peak * 0.5, "a released key must decay, peak {peak} end {end}");
    }
}
