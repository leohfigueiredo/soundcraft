//! Isolated unsafe FFI module for VST 2.4 hosting.
//!
//! Clean-room declarations written by hand; no Steinberg headers, no GPL code.
//! Unsafe code is denied crate-wide and isolated exclusively to this module.
//! Every unsafe block carries a `// SAFETY:` rationale.

use std::ffi::{c_char, c_void};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const VST_MAGIC: i32 = 0x56737450; // 'VstP'

// VST2 Dispatcher Opcodes
pub const EFF_OPEN: i32 = 0;
pub const EFF_CLOSE: i32 = 1;
pub const EFF_SET_PROGRAM: i32 = 2;
pub const EFF_GET_PROGRAM: i32 = 3;
pub const EFF_SET_PROGRAM_NAME: i32 = 4;
pub const EFF_GET_PROGRAM_NAME: i32 = 5;
pub const EFF_GET_PARAM_LABEL: i32 = 6;
pub const EFF_GET_PARAM_DISPLAY: i32 = 7;
pub const EFF_GET_PARAM_NAME: i32 = 8;
pub const EFF_SET_SAMPLE_RATE: i32 = 10;
pub const EFF_SET_BLOCK_SIZE: i32 = 11;
pub const EFF_MAINS_CHANGED: i32 = 12;
pub const EFF_EDIT_GET_RECT: i32 = 13;
pub const EFF_EDIT_OPEN: i32 = 14;
pub const EFF_EDIT_CLOSE: i32 = 15;
pub const EFF_EDIT_IDLE: i32 = 19;
pub const EFF_GET_CHUNK: i32 = 23;
pub const EFF_SET_CHUNK: i32 = 24;
pub const EFF_PROCESS_EVENTS: i32 = 25;
pub const EFF_CAN_BE_AUTOMATED: i32 = 26;
pub const EFF_GET_EFFECT_NAME: i32 = 45;
pub const EFF_GET_VENDOR_STRING: i32 = 47;
pub const EFF_GET_PRODUCT_STRING: i32 = 48;
pub const EFF_GET_VENDOR_VERSION: i32 = 49;
pub const EFF_CAN_DO: i32 = 51;
pub const EFF_GET_VST_VERSION: i32 = 58;

// VST2 AudioMaster Opcodes
pub const AUDIO_MASTER_AUTOMATE: i32 = 0;
pub const AUDIO_MASTER_VERSION: i32 = 1;
pub const AUDIO_MASTER_CURRENT_ID: i32 = 2;
pub const AUDIO_MASTER_IDLE: i32 = 3;
pub const AUDIO_MASTER_GET_TIME: i32 = 7;
pub const AUDIO_MASTER_GET_SAMPLE_RATE: i32 = 16;
pub const AUDIO_MASTER_GET_BLOCK_SIZE: i32 = 17;
pub const AUDIO_MASTER_WANT_MIDI: i32 = 31;
pub const AUDIO_MASTER_GET_VENDOR_STRING: i32 = 32;
pub const AUDIO_MASTER_GET_PRODUCT_STRING: i32 = 33;
pub const AUDIO_MASTER_GET_VENDOR_VERSION: i32 = 34;
pub const AUDIO_MASTER_CAN_DO: i32 = 37;

// AEffect Flags
pub const EFF_FLAGS_HAS_EDITOR: i32 = 1 << 0;
pub const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;
pub const EFF_FLAGS_PROGRAM_CHUNKS: i32 = 1 << 5;
pub const EFF_FLAGS_IS_SYNTH: i32 = 1 << 8;

#[repr(C)]
pub struct AEffect {
    pub magic: i32,
    pub dispatcher: unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize,
    pub process: Option<unsafe extern "C" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32)>,
    pub set_parameter: unsafe extern "C" fn(*mut AEffect, i32, f32),
    pub get_parameter: unsafe extern "C" fn(*mut AEffect, i32) -> f32,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub flags: i32,
    pub resvd1: isize,
    pub resvd2: isize,
    pub initial_delay: i32,
    pub real_qualities: i32,
    pub off_qualities: i32,
    pub io_ratio: f32,
    pub object: *mut c_void,
    pub user: *mut c_void,
    pub unique_id: i32,
    pub version: i32,
    pub process_replacing: unsafe extern "C" fn(*mut AEffect, *mut *const f32, *mut *mut f32, i32),
    pub process_double_replacing: Option<unsafe extern "C" fn(*mut AEffect, *mut *const f64, *mut *mut f64, i32)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ERect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VstMidiEvent {
    pub event_type: i32, // 1 = kVstMidiType
    pub byte_size: i32,
    pub delta_frames: i32,
    pub flags: i32,
    pub note_length: i32,
    pub note_offset: i32,
    pub midi_data: [u8; 4],
    pub detune: i8,
    pub note_off_velocity: u8,
    pub reserved1: u8,
    pub reserved2: u8,
}

impl Default for VstMidiEvent {
    fn default() -> Self {
        Self {
            event_type: 1,
            byte_size: std::mem::size_of::<VstMidiEvent>() as i32,
            delta_frames: 0,
            flags: 0,
            note_length: 0,
            note_offset: 0,
            midi_data: [0; 4],
            detune: 0,
            note_off_velocity: 0,
            reserved1: 0,
            reserved2: 0,
        }
    }
}

#[repr(C)]
pub struct VstEventsHeader {
    pub num_events: i32,
    pub reserved: isize,
    pub events: [*mut c_void; 256],
}

#[repr(C)]
pub struct VstTimeInfo {
    pub sample_pos: f64,
    pub sample_rate: f64,
    pub nano_seconds: f64,
    pub ppq_pos: f64,
    pub tempo: f64,
    pub bar_start_pos: f64,
    pub cycle_start_pos: f64,
    pub cycle_end_pos: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub smpte_offset: i32,
    pub smpte_frame_rate: i32,
    pub samples_to_next_clock: i32,
    pub flags: i32,
}

static TIME_INFO: VstTimeInfo = VstTimeInfo {
    sample_pos: 0.0,
    sample_rate: 48000.0,
    nano_seconds: 0.0,
    ppq_pos: 0.0,
    tempo: 120.0,
    bar_start_pos: 0.0,
    cycle_start_pos: 0.0,
    cycle_end_pos: 0.0,
    time_sig_numerator: 4,
    time_sig_denominator: 4,
    smpte_offset: 0,
    smpte_frame_rate: 1,
    samples_to_next_clock: 0,
    flags: 1, // kVstTransportPlaying
};

/// Host callback invoked by the VST2 plugin to interact with the host.
///
/// # Safety
/// The caller must pass valid pointers for `effect` and `ptr` according to the VST2 specification.
pub unsafe extern "C" fn host_callback(effect: *mut AEffect, opcode: i32, _index: i32, _value: isize, ptr: *mut c_void, _opt: f32) -> isize {
    match opcode {
        AUDIO_MASTER_VERSION => 2400,
        AUDIO_MASTER_CURRENT_ID => {
            if !effect.is_null() {
                // SAFETY: effect is checked non-null and valid pointer passed by plugin.
                unsafe { (*effect).unique_id as isize }
            } else {
                0
            }
        }
        AUDIO_MASTER_GET_SAMPLE_RATE => 48000,
        AUDIO_MASTER_GET_BLOCK_SIZE => 1024,
        AUDIO_MASTER_WANT_MIDI => 1,
        AUDIO_MASTER_GET_VENDOR_STRING => {
            if !ptr.is_null() {
                // SAFETY: pointer passed by plugin is expected to hold at least 64 bytes.
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, 64) };
                let s = b"SoundCraft\0";
                dst[..s.len()].copy_from_slice(s);
            }
            1
        }
        AUDIO_MASTER_GET_PRODUCT_STRING => {
            if !ptr.is_null() {
                // SAFETY: pointer passed by plugin is expected to hold at least 64 bytes.
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, 64) };
                let s = b"SoundCraft\0";
                dst[..s.len()].copy_from_slice(s);
            }
            1
        }
        AUDIO_MASTER_GET_VENDOR_VERSION => 1000,
        AUDIO_MASTER_CAN_DO => {
            if !ptr.is_null() {
                // SAFETY: ptr is a null-terminated C string passed by plugin.
                let s = unsafe { std::ffi::CStr::from_ptr(ptr as *const c_char) }.to_string_lossy();
                match s.as_ref() {
                    "sendVstEvents" | "sendVstMidiEvent" | "receiveVstEvents" | "receiveVstMidiEvent" | "sizeWindow" => 1,
                    _ => 0,
                }
            } else {
                0
            }
        }
        AUDIO_MASTER_GET_TIME => &TIME_INFO as *const _ as isize,
        _ => 0,
    }
}

type VstPluginMainFn = unsafe extern "C" fn(unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize) -> *mut AEffect;

pub struct Instance {
    _lib: Arc<libloading::Library>,
    effect: *mut AEffect,
    active: AtomicBool,
}

// SAFETY: VST2 instances can be sent across threads if synchronized.
unsafe impl Send for Instance {}
// SAFETY: VST2 instances can be referenced across threads with internal locking or disjoint audio/UI usage.
unsafe impl Sync for Instance {}

impl Instance {
    pub fn load(path: &Path, sample_rate: f32) -> Result<Instance, String> {
        // SAFETY: Loading a dynamic library via libloading. Library pointer kept alive in Arc.
        let lib = unsafe { libloading::Library::new(path) }.map_err(|e| format!("cannot load dynamic library {}: {e}", path.display()))?;
        let lib = Arc::new(lib);

        // SAFETY: Looking up VSTPluginMain or legacy main symbol.
        let entry: libloading::Symbol<VstPluginMainFn> = unsafe {
            lib.get(b"VSTPluginMain\0").or_else(|_| lib.get(b"main\0")).map_err(|e| format!("no VST entry point in {}: {e}", path.display()))?
        };

        // SAFETY: Invoking VSTPluginMain with our valid host_callback function pointer.
        let effect = unsafe { entry(host_callback) };
        if effect.is_null() {
            return Err(format!("VSTPluginMain returned NULL in {}", path.display()));
        }

        // SAFETY: Checking AEffect magic.
        let magic = unsafe { (*effect).magic };
        if magic != VST_MAGIC {
            return Err(format!("invalid VST magic 0x{magic:08x} in {}", path.display()));
        }

        let inst = Instance { _lib: lib, effect, active: AtomicBool::new(false) };

        inst.dispatch(EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
        inst.dispatch(EFF_SET_SAMPLE_RATE, 0, 0, std::ptr::null_mut(), sample_rate);
        inst.dispatch(EFF_SET_BLOCK_SIZE, 0, 1024, std::ptr::null_mut(), 0.0);
        inst.dispatch(EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0);
        inst.active.store(true, Ordering::SeqCst);

        Ok(inst)
    }

    #[inline]
    pub(crate) fn dispatch(&self, opcode: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32) -> isize {
        // SAFETY: dispatcher function pointer is validated by AEffect magic.
        unsafe { ((*self.effect).dispatcher)(self.effect, opcode, index, value, ptr, opt) }
    }

    pub fn unique_id(&self) -> i32 {
        // SAFETY: reading unique_id field.
        unsafe { (*self.effect).unique_id }
    }

    pub fn num_params(&self) -> usize {
        // SAFETY: reading num_params field.
        unsafe { (*self.effect).num_params.max(0) as usize }
    }

    pub fn num_inputs(&self) -> usize {
        // SAFETY: reading num_inputs field.
        unsafe { (*self.effect).num_inputs.max(0) as usize }
    }

    pub fn num_outputs(&self) -> usize {
        // SAFETY: reading num_outputs field.
        unsafe { (*self.effect).num_outputs.max(0) as usize }
    }

    pub fn num_programs(&self) -> usize {
        // SAFETY: reading num_programs field.
        unsafe { (*self.effect).num_programs.max(0) as usize }
    }

    pub fn is_synth(&self) -> bool {
        // SAFETY: reading flags field.
        unsafe { ((*self.effect).flags & EFF_FLAGS_IS_SYNTH) != 0 }
    }

    pub fn has_editor(&self) -> bool {
        // SAFETY: reading flags field.
        unsafe { ((*self.effect).flags & EFF_FLAGS_HAS_EDITOR) != 0 }
    }

    pub fn effect_name(&self) -> String {
        let mut buf = [0u8; 64];
        if self.dispatch(EFF_GET_EFFECT_NAME, 0, 0, buf.as_mut_ptr() as *mut c_void, 0.0) != 0 {
            let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = String::from_utf8_lossy(&buf[..nul]).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        format!("vst2_{:08x}", self.unique_id())
    }

    pub fn vendor_string(&self) -> String {
        let mut buf = [0u8; 64];
        if self.dispatch(EFF_GET_VENDOR_STRING, 0, 0, buf.as_mut_ptr() as *mut c_void, 0.0) != 0 {
            let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..nul]).trim().to_string();
        }
        "VST2".to_string()
    }

    pub fn param_name(&self, idx: usize) -> String {
        let mut buf = [0u8; 64];
        self.dispatch(EFF_GET_PARAM_NAME, idx as i32, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let s = String::from_utf8_lossy(&buf[..nul]).trim().to_string();
        if s.is_empty() { format!("Param {idx}") } else { s }
    }

    pub fn param_label(&self, idx: usize) -> String {
        let mut buf = [0u8; 64];
        self.dispatch(EFF_GET_PARAM_LABEL, idx as i32, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..nul]).trim().to_string()
    }

    pub fn param_display(&self, idx: usize) -> String {
        let mut buf = [0u8; 64];
        self.dispatch(EFF_GET_PARAM_DISPLAY, idx as i32, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..nul]).trim().to_string()
    }

    pub fn get_param(&self, idx: usize) -> f32 {
        // SAFETY: get_parameter function pointer called on valid effect.
        unsafe { ((*self.effect).get_parameter)(self.effect, idx as i32) }
    }

    pub fn set_param(&self, idx: usize, val: f32) {
        // SAFETY: set_parameter function pointer called on valid effect.
        unsafe { ((*self.effect).set_parameter)(self.effect, idx as i32, val.clamp(0.0, 1.0)) }
    }

    pub fn process_replacing(&self, inputs: &[*const f32], outputs: &mut [*mut f32], frames: usize) {
        let in_ptr = inputs.as_ptr() as *mut *const f32;
        let out_ptr = outputs.as_mut_ptr();
        // SAFETY: process_replacing called with valid channel pointer buffers and sample count.
        unsafe {
            ((*self.effect).process_replacing)(self.effect, in_ptr, out_ptr, frames as i32);
        }
    }

    pub fn send_events(&self, events: &[VstMidiEvent]) {
        if events.is_empty() {
            return;
        }
        let count = events.len().min(256);
        let mut header = VstEventsHeader { num_events: count as i32, reserved: 0, events: [std::ptr::null_mut(); 256] };
        for i in 0..count {
            if let Some(e) = events.get(i) {
                header.events[i] = e as *const _ as *mut c_void;
            }
        }
        self.dispatch(EFF_PROCESS_EVENTS, 0, 0, &mut header as *mut _ as *mut c_void, 0.0);
    }

    pub fn get_program(&self) -> i32 {
        self.dispatch(EFF_GET_PROGRAM, 0, 0, std::ptr::null_mut(), 0.0) as i32
    }

    pub fn set_program(&self, prog: i32) {
        self.dispatch(EFF_SET_PROGRAM, 0, prog as isize, std::ptr::null_mut(), 0.0);
    }

    pub fn program_name(&self, prog: i32) -> String {
        let mut buf = [0u8; 64];
        self.dispatch(EFF_GET_PROGRAM_NAME, prog, 0, buf.as_mut_ptr() as *mut c_void, 0.0);
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..nul]).trim().to_string()
    }

    pub fn save_chunk(&self, is_preset: bool) -> Option<Vec<u8>> {
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let idx = if is_preset { 1 } else { 0 };
        let len = self.dispatch(EFF_GET_CHUNK, idx, 0, &mut ptr as *mut _ as *mut c_void, 0.0);
        if len > 0 && !ptr.is_null() {
            // SAFETY: plugin returned pointer and valid length of chunk memory.
            let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            Some(slice.to_vec())
        } else {
            None
        }
    }

    pub fn load_chunk(&self, data: &[u8], is_preset: bool) -> bool {
        let idx = if is_preset { 1 } else { 0 };
        self.dispatch(EFF_SET_CHUNK, idx, data.len() as isize, data.as_ptr() as *mut c_void, 0.0) != 0
    }

    pub fn editor_rect(&self) -> Option<(u32, u32)> {
        let mut ptr: *mut ERect = std::ptr::null_mut();
        self.dispatch(EFF_EDIT_GET_RECT, 0, 0, &mut ptr as *mut _ as *mut c_void, 0.0);
        if !ptr.is_null() {
            // SAFETY: ptr points to valid ERect returned by plugin.
            let r = unsafe { *ptr };
            let w = (r.right - r.left).max(0) as u32;
            let h = (r.bottom - r.top).max(0) as u32;
            if w > 0 && h > 0 {
                return Some((w, h));
            }
        }
        None
    }

    pub fn open_editor(&self, window: *mut c_void) -> bool {
        self.dispatch(EFF_EDIT_OPEN, 0, 0, window, 0.0) != 0
    }

    pub fn close_editor(&self) {
        self.dispatch(EFF_EDIT_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
    }

    pub fn editor_idle(&self) {
        self.dispatch(EFF_EDIT_IDLE, 0, 0, std::ptr::null_mut(), 0.0);
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::SeqCst) {
            self.dispatch(EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
        }
        self.dispatch(EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
    }
}
