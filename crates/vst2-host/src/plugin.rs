//! `Vst2Plugin`: safe `soundcraft_dsp::Plugin` implementation wrapping a hosted VST2 instance.

use crate::ffi::{Instance, VstMidiEvent};
use soundcraft_dsp::{Plugin, PluginInfo, Transport};
use std::sync::Arc;

pub struct Vst2Plugin {
    inst: Instance,
    info: &'static PluginInfo,
    sr: f32,
    max_block: usize,
    channels: usize,
    pending_events: Vec<VstMidiEvent>,
    silence_in: Vec<f32>,
    scratch_out: Vec<Vec<f32>>,
}

impl Vst2Plugin {
    pub fn new(inst: Instance, info: &'static PluginInfo) -> Self {
        let num_out = inst.num_outputs().max(2);
        let mut scratch_out = Vec::with_capacity(num_out);
        for _ in 0..num_out {
            scratch_out.push(vec![0.0f32; 1024]);
        }
        Self {
            inst,
            info,
            sr: 48000.0,
            max_block: 1024,
            channels: 2,
            pending_events: Vec::with_capacity(256),
            silence_in: vec![0.0; 1024],
            scratch_out,
        }
    }

    pub fn instance(&self) -> &Instance {
        &self.inst
    }

    fn queue_midi(&mut self, offset: usize, b0: u8, b1: u8, b2: u8) {
        if self.pending_events.len() < 256 {
            let ev = VstMidiEvent { delta_frames: offset as i32, midi_data: [b0, b1, b2, 0], ..Default::default() };
            self.pending_events.push(ev);
        }
    }
}

impl Plugin for Vst2Plugin {
    fn info(&self) -> &'static PluginInfo {
        self.info
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48000.0 };
        self.max_block = max_block.clamp(1, 8192);
        self.channels = channels.clamp(1, 64);

        // Pre-allocate buffers to maximum expected size to avoid runtime reallocation
        let target_silence_size = self.max_block;
        if self.silence_in.capacity() < target_silence_size {
            self.silence_in.reserve(target_silence_size - self.silence_in.len());
        }
        self.silence_in.resize(target_silence_size, 0.0);

        let num_out = self.inst.num_outputs().max(2);
        if self.scratch_out.capacity() < num_out {
            self.scratch_out.reserve(num_out - self.scratch_out.len());
        }
        while self.scratch_out.len() < num_out {
            self.scratch_out.push(vec![0.0f32; self.max_block]);
        }

        for buf in &mut self.scratch_out {
            if buf.len() != self.max_block {
                buf.resize(self.max_block, 0.0);
            }
        }
    }

    fn reset(&mut self) {
        self.pending_events.clear();
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        let Ok(idx) = id.parse::<usize>() else { return false };
        if idx >= self.inst.num_params() {
            return false;
        }
        self.inst.set_param(idx, value);
        true
    }

    fn param(&self, id: &str) -> Option<f32> {
        let idx = id.parse::<usize>().ok()?;
        if idx >= self.inst.num_params() {
            return None;
        }
        Some(self.inst.get_param(idx))
    }

    fn note_on(&mut self, offset: usize, note: u8, velocity: u8) {
        self.queue_midi(offset, 0x90, note, velocity);
    }

    fn note_off(&mut self, offset: usize, note: u8) {
        self.queue_midi(offset, 0x80, note, 0);
    }

    fn midi(&mut self, offset: usize, msg: [u8; 3]) {
        self.queue_midi(offset, msg[0], msg[1], msg[2]);
    }

    fn all_notes_off(&mut self) {
        self.queue_midi(0, 0xB0, 123, 0);
        self.queue_midi(0, 0xB0, 120, 0);
    }

    fn set_transport(&mut self, _transport: &Transport) {}
    fn set_sample(&mut self, sample: Option<Arc<dyn soundcraft_dsp::SampleSource>>) -> bool {
        sample.is_some()
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let n = frames.min(self.max_block);
        if n == 0 {
            return;
        }

        // Send MIDI events for this block
        if !self.pending_events.is_empty() {
            self.inst.send_events(&self.pending_events);
            self.pending_events.clear();
        }

        let num_in = self.inst.num_inputs();
        let num_out = self.inst.num_outputs();

        // Prepare input pointers
        let mut in_ptrs: Vec<*const f32> = Vec::with_capacity(num_in.max(1));
        if num_in == 0 {
            in_ptrs.push(self.silence_in.as_ptr());
        } else {
            for i in 0..num_in {
                if let Some(buf) = io.get(i) {
                    in_ptrs.push(buf.as_ptr());
                } else {
                    in_ptrs.push(self.silence_in.as_ptr());
                }
            }
        }

        // Prepare output pointers: MUST provide at least num_out pointers
        let total_out = num_out.max(1);
        let mut out_ptrs: Vec<*mut f32> = Vec::with_capacity(total_out);

        for i in 0..total_out {
            if let Some(buf) = io.get_mut(i) {
                if buf.len() < n {
                    buf.resize(n, 0.0);
                }
                out_ptrs.push(buf.as_mut_ptr());
            } else if let Some(scratch) = self.scratch_out.get_mut(i) {
                if scratch.len() < n {
                    scratch.resize(n, 0.0);
                }
                out_ptrs.push(scratch.as_mut_ptr());
            }
        }

        if !out_ptrs.is_empty() {
            self.inst.process_replacing(&in_ptrs, &mut out_ptrs, n);
        }
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        self.inst.save_chunk(false)
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        self.inst.load_chunk(data, false)
    }
}
