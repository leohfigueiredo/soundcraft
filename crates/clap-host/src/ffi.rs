//! The FFI boundary: the only module in SoundCraft allowed to use `unsafe`.
//!
//! It loads CLAP binaries (`libloading`), calls the plugin entry/factory/plugin vtables
//! (`clap-sys` raw bindings) and implements the host callbacks. Everything it exposes to the rest
//! of the crate is a safe API returning `Result`.
//!
//! Soundness rests on the CLAP contract (the plugin honours the vtable signatures and the
//! documented pointer lifetimes). A plugin that violates it (e.g. dereferences garbage) can still
//! crash the process: hosting in-process can't prevent that. What this module guarantees is that
//! *SoundCraft* never hands the plugin dangling or mis-sized pointers, never calls a missing
//! (null) function pointer, and never reads plugin strings beyond their bounds.

use crate::{ClapError, scan};
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT,
    CLAP_TRANSPORT_HAS_BEATS_TIMELINE, CLAP_TRANSPORT_HAS_SECONDS_TIMELINE, CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_TIME_SIGNATURE,
    CLAP_TRANSPORT_IS_PLAYING, CLAP_TRANSPORT_IS_RECORDING, clap_event_header, clap_event_midi, clap_event_note, clap_event_param_value,
    clap_event_transport, clap_input_events, clap_output_events,
};
use clap_sys::ext::audio_ports::{CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS, clap_audio_port_info, clap_plugin_audio_ports};
use clap_sys::ext::gui::{CLAP_EXT_GUI, clap_host_gui, clap_plugin_gui, clap_window, clap_window_handle};
use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency};
use clap_sys::ext::log::{CLAP_EXT_LOG, CLAP_LOG_ERROR, CLAP_LOG_FATAL, CLAP_LOG_WARNING, clap_host_log, clap_log_severity};
use clap_sys::ext::note_ports::{CLAP_EXT_NOTE_PORTS, clap_note_port_info, clap_plugin_note_ports};
use clap_sys::ext::posix_fd_support::{
    CLAP_EXT_POSIX_FD_SUPPORT, CLAP_POSIX_FD_ERROR, CLAP_POSIX_FD_READ, CLAP_POSIX_FD_WRITE, clap_host_posix_fd_support, clap_plugin_posix_fd_support,
    clap_posix_fd_flags,
};
use clap_sys::ext::params::{
    CLAP_EXT_PARAMS, clap_host_params, clap_param_clear_flags, clap_param_info, clap_param_rescan_flags, clap_plugin_params,
};
use clap_sys::ext::state::{CLAP_EXT_STATE, clap_host_state, clap_plugin_state};
use clap_sys::ext::timer_support::{CLAP_EXT_TIMER_SUPPORT, clap_host_timer_support, clap_plugin_timer_support};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::host::clap_host;
use clap_sys::id::{CLAP_INVALID_ID, clap_id};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{CLAP_PROCESS_ERROR, clap_process};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::{CLAP_VERSION, clap_version_is_compatible};
use soundcraft_dsp::Transport;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Most plugins one factory may report (hostile-input cap).
const MAX_PLUGINS_PER_FACTORY: u32 = 1024;
/// Most parameters read from one plugin.
pub(crate) const MAX_PARAMS: u32 = 4096;
/// Most audio ports per direction we provide buffers for.
pub(crate) const MAX_PORTS: u32 = 16;
/// Most channels per audio port.
pub(crate) const MAX_PORT_CHANNELS: u32 = 64;
/// Most feature strings read from a descriptor.
const MAX_FEATURES: usize = 64;
/// Capacity of the per-block event list (preallocated: `process` never allocates).
pub(crate) const EVENT_CAP: usize = 1024;
/// Largest plugin state we accept from `clap_plugin_state.save` (hostile-input cap).
pub(crate) const MAX_STATE_BYTES: usize = 64 << 20;
/// Most editor parameter edits buffered between two UI frames.
const EDIT_CAP: usize = 256;

// ---- plugin strings ------------------------------------------------------------------------

/// Reads a NUL-terminated plugin string (`None` for null).
///
/// SAFETY contract for callers: `p` is null or points to a NUL-terminated string that stays valid
/// for the duration of the call (CLAP guarantees this for descriptor fields).
unsafe fn read_cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: non-null and NUL-terminated per this function's contract.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// Reads a fixed-size `char[N]` name buffer, stopping at the first NUL or the end of the array.
/// `c_char` is `i8` on x86 and `u8` on ARM/RISC-V Linux; the byte reinterpretation is the same for both.
fn read_name(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| u8::from_ne_bytes(c.to_ne_bytes())).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---- loaded binaries -----------------------------------------------------------------------

/// A plugin descriptor as reported by the factory.
#[derive(Debug, Clone)]
pub(crate) struct RawDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub features: Vec<String>,
}

/// A loaded, initialised CLAP binary. Loaded binaries are cached for the life of the process
/// (see [`Bundle::load`]): plugin code must never be unmapped while an instance may still run.
pub(crate) struct Bundle {
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    // Dropped last: the library must outlive every pointer above.
    _lib: libloading::Library,
}

// SAFETY: CLAP requires the entry and the plugin factory to be thread-safe; the raw pointers are
// read-only vtables inside the loaded library, which `_lib` keeps mapped.
unsafe impl Send for Bundle {}
// SAFETY: as above; `Bundle` has no interior mutability on the Rust side.
unsafe impl Sync for Bundle {}

fn loaded() -> &'static Mutex<HashMap<PathBuf, Arc<Bundle>>> {
    static L: OnceLock<Mutex<HashMap<PathBuf, Arc<Bundle>>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Bundle {
    /// Loads (once per path, then cached for the life of the process) and initialises a `.clap`.
    pub fn load(path: &Path) -> Result<Arc<Bundle>, ClapError> {
        let mut map = loaded().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(b) = map.get(path) {
            return Ok(b.clone());
        }
        let b = Arc::new(Bundle::open(path)?);
        map.insert(path.to_path_buf(), b.clone());
        Ok(b)
    }

    fn open(path: &Path) -> Result<Bundle, ClapError> {
        let err = |m: String| ClapError::Load(path.display().to_string(), m);
        let exe = scan::bundle_executable(path).ok_or_else(|| err("no loadable binary".into()))?;
        // SAFETY: loading a shared library runs its static initialisers. That is inherent to
        // hosting plugins; we only load files found in CLAP plugin folders or named explicitly.
        let lib = unsafe { libloading::Library::new(&exe) }.map_err(|e| err(e.to_string()))?;
        // SAFETY: `clap_entry` is the CLAP-mandated exported data symbol of type
        // `const clap_plugin_entry`; the symbol's address is therefore a `*const clap_plugin_entry`.
        let entry: *const clap_plugin_entry = match unsafe { lib.get::<*const clap_plugin_entry>(b"clap_entry\0") } {
            Ok(sym) => *sym,
            Err(e) => return Err(err(format!("not a CLAP plugin: {e}"))),
        };
        if entry.is_null() {
            return Err(err("null clap_entry".into()));
        }
        // SAFETY: non-null pointer to the entry struct inside the library, which `lib` keeps mapped.
        let e = unsafe { *entry };
        if !clap_version_is_compatible(e.clap_version) {
            return Err(err(format!("unsupported CLAP version {}.{}", e.clap_version.major, e.clap_version.minor)));
        }
        let (Some(init), Some(get_factory)) = (e.init, e.get_factory) else { return Err(err("incomplete clap_entry".into())) };
        let cpath = CString::new(path.to_string_lossy().into_owned()).map_err(|_| err("path contains NUL".into()))?;
        // SAFETY: `init` comes from a valid entry; the path is a valid NUL-terminated string that
        // outlives the call.
        if !unsafe { init(cpath.as_ptr()) } {
            return Err(err("clap_entry.init failed".into()));
        }
        // SAFETY: the entry is initialised; the id is a static NUL-terminated string.
        let factory = unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) } as *const clap_plugin_factory;
        if factory.is_null() {
            if let Some(deinit) = e.deinit {
                // SAFETY: balanced with the successful `init` above.
                unsafe { deinit() };
            }
            return Err(err("no plugin factory".into()));
        }
        Ok(Bundle { entry, factory, _lib: lib })
    }

    fn factory(&self) -> clap_plugin_factory {
        // SAFETY: `factory` was non-null at load time and points into the still-mapped library.
        unsafe { *self.factory }
    }

    /// The plugins this binary provides.
    pub fn descriptors(&self) -> Vec<RawDescriptor> {
        let f = self.factory();
        let (Some(count), Some(get)) = (f.get_plugin_count, f.get_plugin_descriptor) else { return Vec::new() };
        // SAFETY: vtable call with the factory pointer it belongs to.
        let n = unsafe { count(self.factory) }.min(MAX_PLUGINS_PER_FACTORY);
        let mut out = Vec::new();
        for i in 0..n {
            // SAFETY: index is below the reported count.
            let d = unsafe { get(self.factory, i) };
            if d.is_null() {
                continue;
            }
            // SAFETY: non-null descriptor owned by the plugin library, valid while it is loaded.
            let d = unsafe { *d };
            // SAFETY: descriptor string fields are null or NUL-terminated (CLAP contract).
            let Some(id) = (unsafe { read_cstr(d.id) }).filter(|s| !s.is_empty()) else { continue };
            // SAFETY: as above.
            let name = unsafe { read_cstr(d.name) }.unwrap_or_else(|| id.clone());
            let mut features = Vec::new();
            if !d.features.is_null() {
                for k in 0..MAX_FEATURES {
                    // SAFETY: `features` is a NULL-terminated array; we stop at the terminator
                    // (or the cap) and never read past it.
                    let p = unsafe { *d.features.add(k) };
                    if p.is_null() {
                        break;
                    }
                    // SAFETY: a non-null entry of the feature array is a NUL-terminated string.
                    if let Some(s) = unsafe { read_cstr(p) } {
                        features.push(s);
                    }
                }
            }
            out.push(RawDescriptor {
                id,
                name,
                // SAFETY: descriptor string fields are null or NUL-terminated.
                vendor: unsafe { read_cstr(d.vendor) }.unwrap_or_default(),
                // SAFETY: as above.
                version: unsafe { read_cstr(d.version) }.unwrap_or_default(),
                // SAFETY: as above.
                description: unsafe { read_cstr(d.description) }.unwrap_or_default(),
                features,
            });
        }
        out
    }

    /// Creates and initialises one plugin instance.
    pub fn instantiate(self: &Arc<Self>, plugin_id: &str) -> Result<Instance, ClapError> {
        let err = |m: &str| ClapError::Instantiate(plugin_id.to_string(), m.to_string());
        let create = self.factory().create_plugin.ok_or_else(|| err("factory cannot create plugins"))?;
        let cid = CString::new(plugin_id).map_err(|_| err("id contains NUL"))?;
        let state = Arc::new(HostState::default());
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: Arc::as_ptr(&state).cast_mut().cast::<c_void>(),
            name: c"SoundCraft".as_ptr(),
            vendor: c"SoundCraft contributors".as_ptr(),
            url: c"https://github.com/storytold/soundcraft".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        });
        // SAFETY: the factory is valid; `host` is boxed (stable address) and owned by the returned
        // `Instance`, which destroys the plugin before dropping the box; `cid` outlives the call.
        let plugin = unsafe { create(self.factory, &*host, cid.as_ptr()) };
        if plugin.is_null() {
            return Err(err("factory returned no instance"));
        }
        let mut inst = Instance {
            plugin,
            params: std::ptr::null(),
            ports: std::ptr::null(),
            latency: std::ptr::null(),
            notes: std::ptr::null(),
            state_ext: std::ptr::null(),
            gui: Arc::new(GuiLink::default()),
            raw: Vec::with_capacity(EVENT_CAP),
            out_params: Vec::with_capacity(EVENT_CAP),
            active: false,
            processing: false,
            max_frames: 0,
            created: true,
            _host: host,
            state,
            _bundle: self.clone(),
        };
        // SAFETY: `plugin` is a fresh non-null instance; we copy its vtable struct.
        let vt = unsafe { *plugin };
        let ok = match vt.init {
            // SAFETY: `init` is called exactly once, right after creation, as CLAP requires.
            Some(init) => unsafe { init(plugin) },
            None => false,
        };
        if !ok {
            // `Drop` destroys the instance (without deactivating: it never activated).
            return Err(err("plugin init failed"));
        }
        inst.params = inst.extension::<clap_plugin_params>(CLAP_EXT_PARAMS);
        inst.ports = inst.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS);
        inst.latency = inst.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY);
        inst.notes = inst.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS);
        inst.state_ext = inst.extension::<clap_plugin_state>(CLAP_EXT_STATE);
        let gui = inst.extension::<clap_plugin_gui>(CLAP_EXT_GUI);
        let timer_ext = inst.extension::<clap_plugin_timer_support>(CLAP_EXT_TIMER_SUPPORT);
        let fd_ext = inst.extension::<clap_plugin_posix_fd_support>(CLAP_EXT_POSIX_FD_SUPPORT);
        let inner = GuiInner { plugin, gui, state: inst.state_ext, alive: true, created: false, visible: false, window: None, timer_ext, fd_ext };
        inst.gui = Arc::new(GuiLink {
            inner: Mutex::new(inner),
            host: Arc::clone(&inst.state),
        });
        Ok(inst)
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        // SAFETY: `entry` was valid at load time and `_lib` (dropped after this) keeps it mapped.
        let e = unsafe { *self.entry };
        if let Some(deinit) = e.deinit {
            // SAFETY: balanced with the successful `init` in `open`; no instance holds an `Arc`
            // to this bundle any more (they keep it alive), so no plugin code still runs.
            unsafe { deinit() };
        }
    }
}

// ---- host side -----------------------------------------------------------------------------

/// Flags the plugin raises through the host callbacks (any thread).
pub(crate) struct HostState {
    pub restart: AtomicBool,
    pub process: AtomicBool,
    pub callback: AtomicBool,
    pub latency_changed: AtomicBool,
    pub params_rescan: AtomicBool,
    /// `clap_host_state.mark_dirty`.
    pub dirty: AtomicBool,
    /// `clap_host_gui` requests, serviced by the editor's `idle` on the main thread.
    pub gui_closed: AtomicBool,
    pub gui_destroyed: AtomicBool,
    pub gui_show: AtomicBool,
    pub gui_hide: AtomicBool,
    /// A size an embedded editor asked for (`width << 32 | height`; 0 = none pending).
    pub gui_resize: AtomicU64,
    /// Parameter changes the plugin reported (output events: typically edits in its GUI),
    /// waiting for the UI. Bounded by `EDIT_CAP`; the audio thread only `try_lock`s it.
    pub edits: Mutex<Vec<(u32, f64)>>,
    /// `clap.timer-support` registrations, fired by the editor's `idle` on the main thread.
    pub timers: Mutex<TimerRegistry>,
    /// `clap.posix-fd-support` registrations, polled by the editor's `idle` on the main thread.
    pub fds: Mutex<FdRegistry>,
}

impl Default for HostState {
    fn default() -> Self {
        HostState {
            restart: AtomicBool::new(false),
            process: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            latency_changed: AtomicBool::new(false),
            params_rescan: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            gui_closed: AtomicBool::new(false),
            gui_destroyed: AtomicBool::new(false),
            gui_show: AtomicBool::new(false),
            gui_hide: AtomicBool::new(false),
            gui_resize: AtomicU64::new(0),
            edits: Mutex::new(Vec::with_capacity(EDIT_CAP)),
            timers: Mutex::new(TimerRegistry::default()),
            fds: Mutex::new(FdRegistry::default()),
        }
    }
}

// ---- host timers and file descriptors ------------------------------------------------------

/// Most timers one plugin may register (hostile-input cap).
pub(crate) const MAX_TIMERS: usize = 64;
/// Most file descriptors one plugin may register (hostile-input cap).
pub(crate) const MAX_FDS: usize = 64;
/// Longest timer period we honour (an hour); shorter than 1 ms is raised to 1 ms.
const MAX_TIMER_MS: u32 = 3_600_000;
/// Every flag `clap.posix-fd-support` defines.
const FD_FLAGS: clap_posix_fd_flags = CLAP_POSIX_FD_READ | CLAP_POSIX_FD_WRITE | CLAP_POSIX_FD_ERROR;

/// One `clap_host_timer_support.register_timer` registration.
#[derive(Debug, Clone, Copy)]
struct HostTimer {
    id: clap_id,
    period: Duration,
    next_due: Instant,
}

/// The timers one plugin instance registered. Plain data: the host callbacks edit it (under the
/// `HostState` mutex) and the editor's `idle` asks which are due, then calls the plugin with no
/// lock held (a timer callback may register or unregister timers).
#[derive(Debug, Default)]
pub(crate) struct TimerRegistry {
    timers: Vec<HostTimer>,
    next_id: clap_id,
}

impl TimerRegistry {
    /// Registers a timer firing every `period_ms` (clamped to 1 ms ..= 1 h), first due one period
    /// after `now`. `None` when the cap is reached.
    pub fn register(&mut self, period_ms: u32, now: Instant) -> Option<clap_id> {
        if self.timers.len() >= MAX_TIMERS {
            return None;
        }
        // Fewer than `MAX_TIMERS` ids are in use, so a free one is among the next few.
        let mut id = self.next_id;
        for _ in 0..=MAX_TIMERS {
            if id != CLAP_INVALID_ID && !self.contains(id) {
                break;
            }
            id = id.wrapping_add(1);
        }
        if id == CLAP_INVALID_ID || self.contains(id) {
            return None;
        }
        self.next_id = id.wrapping_add(1);
        let period = Duration::from_millis(u64::from(period_ms.clamp(1, MAX_TIMER_MS)));
        self.timers.push(HostTimer { id, period, next_due: now.checked_add(period).unwrap_or(now) });
        Some(id)
    }

    /// Forgets a timer; `false` when it was not registered.
    pub fn unregister(&mut self, id: clap_id) -> bool {
        let before = self.timers.len();
        self.timers.retain(|t| t.id != id);
        self.timers.len() != before
    }

    pub fn contains(&self, id: clap_id) -> bool {
        self.timers.iter().any(|t| t.id == id)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// The timers due at `now`, each rescheduled one period later. A timer fires at most once
    /// per call: ticks missed while the UI was stalled are not replayed.
    pub fn due(&mut self, now: Instant) -> Vec<clap_id> {
        let mut out = Vec::new();
        for t in &mut self.timers {
            if t.next_due <= now {
                t.next_due = now.checked_add(t.period).unwrap_or(now);
                out.push(t.id);
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.timers.clear();
    }
}

/// The file descriptors one plugin instance registered, with the events it wants for each.
#[derive(Debug, Default)]
pub(crate) struct FdRegistry {
    fds: Vec<(i32, clap_posix_fd_flags)>,
}

impl FdRegistry {
    /// `false` for a negative fd, one already registered, or past the cap. Unknown flag bits are
    /// dropped.
    pub fn register(&mut self, fd: i32, flags: clap_posix_fd_flags) -> bool {
        if fd < 0 || self.flags(fd).is_some() || self.fds.len() >= MAX_FDS {
            return false;
        }
        self.fds.push((fd, flags & FD_FLAGS));
        true
    }

    /// Changes the events wanted for a registered fd; `false` when it is not registered.
    pub fn modify(&mut self, fd: i32, flags: clap_posix_fd_flags) -> bool {
        match self.fds.iter_mut().find(|(f, _)| *f == fd) {
            Some(e) => {
                e.1 = flags & FD_FLAGS;
                true
            }
            None => false,
        }
    }

    pub fn unregister(&mut self, fd: i32) -> bool {
        let before = self.fds.len();
        self.fds.retain(|(f, _)| *f != fd);
        self.fds.len() != before
    }

    /// The flags a registered fd waits for (`None` when not registered).
    pub fn flags(&self, fd: i32) -> Option<clap_posix_fd_flags> {
        self.fds.iter().find(|(f, _)| *f == fd).map(|&(_, fl)| fl)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.fds.len()
    }

    pub fn snapshot(&self) -> Vec<(i32, clap_posix_fd_flags)> {
        self.fds.clone()
    }

    pub fn clear(&mut self) {
        self.fds.clear();
    }
}

/// `poll(2)` event bits (identical on Linux, the BSDs and macOS) and their CLAP translation.
#[cfg_attr(not(unix), allow(dead_code))]
mod pollbits {
    use clap_sys::ext::posix_fd_support::{CLAP_POSIX_FD_ERROR, CLAP_POSIX_FD_READ, CLAP_POSIX_FD_WRITE, clap_posix_fd_flags};

    pub const POLLIN: i16 = 0x001;
    pub const POLLPRI: i16 = 0x002;
    pub const POLLOUT: i16 = 0x004;
    pub const POLLERR: i16 = 0x008;
    pub const POLLHUP: i16 = 0x010;
    pub const POLLNVAL: i16 = 0x020;

    /// The `poll` events to ask for, for the CLAP flags a plugin registered.
    pub fn events_for(flags: clap_posix_fd_flags) -> i16 {
        let mut ev = 0;
        if flags & CLAP_POSIX_FD_READ != 0 {
            ev |= POLLIN | POLLPRI;
        }
        if flags & CLAP_POSIX_FD_WRITE != 0 {
            ev |= POLLOUT;
        }
        ev
    }

    /// The CLAP flags to report for `revents`, limited to what the plugin registered for (a
    /// hang-up reads as readable: the plugin's read then sees the end of the stream).
    pub fn ready_flags(registered: clap_posix_fd_flags, revents: i16) -> clap_posix_fd_flags {
        let mut f = 0;
        if revents & (POLLIN | POLLPRI | POLLHUP) != 0 {
            f |= CLAP_POSIX_FD_READ;
        }
        if revents & POLLOUT != 0 {
            f |= CLAP_POSIX_FD_WRITE;
        }
        if revents & (POLLERR | POLLNVAL) != 0 {
            f |= CLAP_POSIX_FD_ERROR;
        }
        f & registered
    }
}

/// Polls the descriptors with a zero timeout; returns the ready ones with the CLAP flags to report.
#[cfg(unix)]
fn poll_fds(fds: &[(i32, clap_posix_fd_flags)]) -> Vec<(i32, clap_posix_fd_flags)> {
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    type NfdsT = std::ffi::c_ulong;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    type NfdsT = std::ffi::c_uint;
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: std::ffi::c_int) -> std::ffi::c_int;
    }
    if fds.is_empty() {
        return Vec::new();
    }
    let mut set: Vec<PollFd> = fds.iter().take(MAX_FDS).map(|&(fd, fl)| PollFd { fd, events: pollbits::events_for(fl), revents: 0 }).collect();
    let Ok(n) = NfdsT::try_from(set.len()) else { return Vec::new() };
    // SAFETY: `set` is a valid array of `n` pollfd structs; zero timeout; nothing is retained.
    if unsafe { poll(set.as_mut_ptr(), n, 0) } <= 0 {
        return Vec::new();
    }
    set.iter()
        .zip(fds)
        .filter_map(|(p, &(fd, fl))| {
            let f = pollbits::ready_flags(fl, p.revents);
            (f != 0).then_some((fd, f))
        })
        .collect()
}

#[cfg(not(unix))]
fn poll_fds(_fds: &[(i32, clap_posix_fd_flags)]) -> Vec<(i32, clap_posix_fd_flags)> {
    Vec::new()
}

/// SAFETY contract: `host` is null or one of our `clap_host` structs, whose `host_data` points
/// to the `HostState` owned by the same `Instance`.
unsafe fn state<'a>(host: *const clap_host) -> Option<&'a HostState> {
    if host.is_null() {
        return None;
    }
    // SAFETY: non-null host created by `Bundle::instantiate` (contract above).
    let data = unsafe { (*host).host_data } as *const HostState;
    // SAFETY: `host_data` points to the boxed `HostState`, alive as long as the instance.
    unsafe { data.as_ref() }
}

unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.restart.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_process(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.process.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.callback.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_log(_host: *const clap_host, severity: clap_log_severity, msg: *const c_char) {
    // SAFETY: CLAP passes a NUL-terminated message (or null, handled).
    let m = unsafe { read_cstr(msg) }.unwrap_or_default();
    match severity {
        CLAP_LOG_ERROR | CLAP_LOG_FATAL => log::error!("clap plugin: {m}"),
        CLAP_LOG_WARNING => log::warn!("clap plugin: {m}"),
        _ => log::debug!("clap plugin: {m}"),
    }
}

unsafe extern "C" fn host_params_rescan(host: *const clap_host, _flags: clap_param_rescan_flags) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.params_rescan.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_params_clear(_host: *const clap_host, _id: clap_id, _flags: clap_param_clear_flags) {}

unsafe extern "C" fn host_params_request_flush(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.process.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_latency_changed(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.latency_changed.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_state_mark_dirty(host: *const clap_host) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.dirty.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_gui_resize_hints_changed(_host: *const clap_host) {}

unsafe extern "C" fn host_gui_request_resize(host: *const clap_host, width: u32, height: u32) -> bool {
    // Floating windows belong to the plugin, which resizes them itself; an embedded editor's host
    // window is resized by the editor's `idle` on the main thread.
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) }
        && width > 0
        && height > 0
    {
        s.gui_resize.store((u64::from(width) << 32) | u64::from(height), Ordering::Relaxed);
    }
    true
}

unsafe extern "C" fn host_gui_request_show(host: *const clap_host) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    match unsafe { state(host) } {
        Some(s) => {
            s.gui_show.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

unsafe extern "C" fn host_gui_request_hide(host: *const clap_host) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    match unsafe { state(host) } {
        Some(s) => {
            s.gui_hide.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

unsafe extern "C" fn host_gui_closed(host: *const clap_host, was_destroyed: bool) {
    // SAFETY: the plugin passes back the host pointer we gave it.
    if let Some(s) = unsafe { state(host) } {
        if was_destroyed {
            s.gui_destroyed.store(true, Ordering::Relaxed);
        }
        s.gui_closed.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_register_timer(host: *const clap_host, period_ms: u32, timer_id: *mut clap_id) -> bool {
    if timer_id.is_null() {
        return false;
    }
    // SAFETY: the plugin passes back the host pointer we gave it.
    let id = unsafe { state(host) }.and_then(|s| s.timers.lock().unwrap_or_else(PoisonError::into_inner).register(period_ms, Instant::now()));
    // SAFETY: non-null out-pointer the plugin provides for the call.
    unsafe { *timer_id = id.unwrap_or(CLAP_INVALID_ID) };
    id.is_some()
}

unsafe extern "C" fn host_unregister_timer(host: *const clap_host, timer_id: clap_id) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    unsafe { state(host) }.is_some_and(|s| s.timers.lock().unwrap_or_else(PoisonError::into_inner).unregister(timer_id))
}

unsafe extern "C" fn host_register_fd(host: *const clap_host, fd: i32, flags: clap_posix_fd_flags) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    unsafe { state(host) }.is_some_and(|s| s.fds.lock().unwrap_or_else(PoisonError::into_inner).register(fd, flags))
}

unsafe extern "C" fn host_modify_fd(host: *const clap_host, fd: i32, flags: clap_posix_fd_flags) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    unsafe { state(host) }.is_some_and(|s| s.fds.lock().unwrap_or_else(PoisonError::into_inner).modify(fd, flags))
}

unsafe extern "C" fn host_unregister_fd(host: *const clap_host, fd: i32) -> bool {
    // SAFETY: the plugin passes back the host pointer we gave it.
    unsafe { state(host) }.is_some_and(|s| s.fds.lock().unwrap_or_else(PoisonError::into_inner).unregister(fd))
}

static HOST_TIMERS: clap_host_timer_support =
    clap_host_timer_support { register_timer: Some(host_register_timer), unregister_timer: Some(host_unregister_timer) };
static HOST_FDS: clap_host_posix_fd_support =
    clap_host_posix_fd_support { register_fd: Some(host_register_fd), modify_fd: Some(host_modify_fd), unregister_fd: Some(host_unregister_fd) };

/// Timers and fds are served on X11 platforms only, where plugin GUIs have no event loop of their
/// own to run them. On Windows and macOS plugins use the native run loop, which never stalls when
/// the host's UI frame does.
const SERVES_RUN_LOOP: bool = cfg!(all(unix, not(target_os = "macos")));

static HOST_STATE: clap_host_state = clap_host_state { mark_dirty: Some(host_state_mark_dirty) };
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(host_gui_resize_hints_changed),
    request_resize: Some(host_gui_request_resize),
    request_show: Some(host_gui_request_show),
    request_hide: Some(host_gui_request_hide),
    closed: Some(host_gui_closed),
};
static HOST_LOG: clap_host_log = clap_host_log { log: Some(host_log) };
static HOST_PARAMS: clap_host_params =
    clap_host_params { rescan: Some(host_params_rescan), clear: Some(host_params_clear), request_flush: Some(host_params_request_flush) };
static HOST_LATENCY: clap_host_latency = clap_host_latency { changed: Some(host_latency_changed) };

unsafe extern "C" fn host_get_extension(_host: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: extension ids are NUL-terminated strings (CLAP contract); non-null checked above.
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_LOG {
        (&HOST_LOG as *const clap_host_log).cast()
    } else if id == CLAP_EXT_PARAMS {
        (&HOST_PARAMS as *const clap_host_params).cast()
    } else if id == CLAP_EXT_LATENCY {
        (&HOST_LATENCY as *const clap_host_latency).cast()
    } else if id == CLAP_EXT_STATE {
        (&HOST_STATE as *const clap_host_state).cast()
    } else if id == CLAP_EXT_GUI {
        (&HOST_GUI as *const clap_host_gui).cast()
    } else if SERVES_RUN_LOOP && id == CLAP_EXT_TIMER_SUPPORT {
        (&HOST_TIMERS as *const clap_host_timer_support).cast()
    } else if SERVES_RUN_LOOP && id == CLAP_EXT_POSIX_FD_SUPPORT {
        (&HOST_FDS as *const clap_host_posix_fd_support).cast()
    } else {
        // Everything else (thread-check, …) is unsupported: CLAP allows null.
        std::ptr::null()
    }
}

// ---- events --------------------------------------------------------------------------------

/// A host → plugin event, safe to build anywhere; converted to CLAP structs inside `process`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Event {
    Param {
        time: u32,
        id: u32,
        value: f64,
    },
    NoteOn {
        time: u32,
        key: u8,
        velocity: f64,
    },
    NoteOff {
        time: u32,
        key: u8,
    },
    /// A raw MIDI 1.0 message (for plugins whose note port speaks only MIDI).
    Midi {
        time: u32,
        data: [u8; 3],
    },
}

impl Event {
    pub fn time(&self) -> u32 {
        match *self {
            Event::Param { time, .. } | Event::NoteOn { time, .. } | Event::NoteOff { time, .. } | Event::Midi { time, .. } => time,
        }
    }
    pub fn set_time(&mut self, t: u32) {
        match self {
            Event::Param { time, .. } | Event::NoteOn { time, .. } | Event::NoteOff { time, .. } | Event::Midi { time, .. } => *time = t,
        }
    }
}

#[derive(Clone, Copy)]
enum RawEvent {
    Note(clap_event_note),
    Param(clap_event_param_value),
    Midi(clap_event_midi),
}

impl RawEvent {
    fn header(&self) -> *const clap_event_header {
        // The header is the first field of every `repr(C)` CLAP event struct, so a pointer to it is
        // a pointer to the whole event, as CLAP expects.
        match self {
            RawEvent::Note(e) => &e.header,
            RawEvent::Param(e) => &e.header,
            RawEvent::Midi(e) => &e.header,
        }
    }

    fn from_event(e: &Event, offset: u32) -> RawEvent {
        let header = |size: usize, type_: u16, time: u32| clap_event_header {
            size: u32::try_from(size).unwrap_or(u32::MAX),
            time: time.saturating_sub(offset),
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_,
            flags: 0,
        };
        match *e {
            Event::Param { time, id, value } => RawEvent::Param(clap_event_param_value {
                header: header(size_of::<clap_event_param_value>(), CLAP_EVENT_PARAM_VALUE, time),
                param_id: id,
                cookie: std::ptr::null_mut(),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                value,
            }),
            Event::NoteOn { time, key, velocity } => RawEvent::Note(clap_event_note {
                header: header(size_of::<clap_event_note>(), CLAP_EVENT_NOTE_ON, time),
                note_id: -1,
                port_index: 0,
                channel: 0,
                key: i16::from(key),
                velocity,
            }),
            Event::NoteOff { time, key } => RawEvent::Note(clap_event_note {
                header: header(size_of::<clap_event_note>(), CLAP_EVENT_NOTE_OFF, time),
                note_id: -1,
                port_index: 0,
                channel: 0,
                key: i16::from(key),
                velocity: 0.0,
            }),
            Event::Midi { time, data } => {
                RawEvent::Midi(clap_event_midi { header: header(size_of::<clap_event_midi>(), CLAP_EVENT_MIDI, time), port_index: 0, data })
            }
        }
    }
}

fn transport_event(t: &Transport) -> clap_event_transport {
    let fixed = |x: f64, factor: i64| if x.is_finite() { (x * factor as f64).round() as i64 } else { 0 };
    let mut flags =
        CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_BEATS_TIMELINE | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
    if t.playing {
        flags |= CLAP_TRANSPORT_IS_PLAYING;
    }
    if t.recording {
        flags |= CLAP_TRANSPORT_IS_RECORDING;
    }
    clap_event_transport {
        header: clap_event_header {
            size: u32::try_from(size_of::<clap_event_transport>()).unwrap_or(u32::MAX),
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_TRANSPORT,
            flags: 0,
        },
        flags,
        song_pos_beats: fixed(t.quarters, CLAP_BEATTIME_FACTOR),
        song_pos_seconds: fixed(t.seconds, CLAP_SECTIME_FACTOR),
        tempo: t.tempo,
        tempo_inc: 0.0,
        loop_start_beats: 0,
        loop_end_beats: 0,
        loop_start_seconds: 0,
        loop_end_seconds: 0,
        bar_start: fixed(t.bar_start, CLAP_BEATTIME_FACTOR),
        bar_number: i32::try_from(t.bar).unwrap_or(0),
        tsig_num: u16::try_from(t.numerator).unwrap_or(4),
        tsig_denom: u16::try_from(t.denominator).unwrap_or(4),
    }
}

unsafe extern "C" fn in_events_size(list: *const clap_input_events) -> u32 {
    if list.is_null() {
        return 0;
    }
    // SAFETY: `list` is the `clap_input_events` we built in `process`; its ctx is the event Vec.
    let v = unsafe { ((*list).ctx as *const Vec<RawEvent>).as_ref() };
    v.map_or(0, |v| u32::try_from(v.len()).unwrap_or(u32::MAX))
}

unsafe extern "C" fn in_events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    if list.is_null() {
        return std::ptr::null();
    }
    // SAFETY: as in `in_events_size`; the Vec is not mutated while the plugin processes.
    let v = unsafe { ((*list).ctx as *const Vec<RawEvent>).as_ref() };
    v.and_then(|v| v.get(index as usize)).map_or(std::ptr::null(), RawEvent::header)
}

unsafe extern "C" fn out_events_try_push(list: *const clap_output_events, event: *const clap_event_header) -> bool {
    // Plugin → host events are accepted. Parameter values (typically edits in the plugin's GUI)
    // are kept, while there is room, so the UI can write them into the session; the rest is
    // ignored.
    if list.is_null() || event.is_null() {
        return true;
    }
    // SAFETY: CLAP passes a valid event header for the duration of the call.
    let h = unsafe { &*event };
    if h.space_id != CLAP_CORE_EVENT_SPACE_ID || h.type_ != CLAP_EVENT_PARAM_VALUE || (h.size as usize) < size_of::<clap_event_param_value>() {
        return true;
    }
    // SAFETY: the header says this is a complete `clap_event_param_value` (type and size checked).
    let e = unsafe { &*event.cast::<clap_event_param_value>() };
    // SAFETY: `ctx` is null or the instance's preallocated `out_params` Vec, which `process`
    // borrows exclusively for the duration of the plugin call.
    if let Some(v) = unsafe { ((*list).ctx as *mut Vec<(u32, f64)>).as_mut() }
        && v.len() < v.capacity()
        && e.value.is_finite()
    {
        v.push((e.param_id, e.value));
    }
    true
}

// ---- state streams -------------------------------------------------------------------------

/// Read side of an in-memory state blob.
struct ReadCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "C" fn istream_read(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    if stream.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: `ctx` is the `ReadCursor` on `load_state`'s stack frame, alive during the call.
    let Some(c) = (unsafe { ((*stream).ctx as *mut ReadCursor<'_>).as_mut() }) else { return -1 };
    let rest = c.data.get(c.pos..).unwrap_or(&[]);
    let n = rest.len().min(usize::try_from(size).unwrap_or(usize::MAX));
    if let Some(src) = rest.get(..n) {
        // SAFETY: the plugin provides `size` ≥ `n` writable bytes at `buffer`; `src` is `n`
        // readable bytes of our own slice; they cannot overlap.
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), buffer.cast::<u8>(), n) };
    }
    c.pos += n;
    i64::try_from(n).unwrap_or(-1)
}

unsafe extern "C" fn ostream_write(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    if stream.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: `ctx` is the output Vec on `save_state`'s stack frame, alive during the call.
    let Some(v) = (unsafe { ((*stream).ctx as *mut Vec<u8>).as_mut() }) else { return -1 };
    let Ok(n) = usize::try_from(size) else { return -1 };
    if v.len().saturating_add(n) > MAX_STATE_BYTES {
        return -1;
    }
    // SAFETY: the plugin provides `size` readable bytes at `buffer` for the call.
    let src = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
    v.extend_from_slice(src);
    i64::try_from(n).unwrap_or(-1)
}

// ---- editor (clap.gui) ---------------------------------------------------------------------

/// True on the process's main thread (AppKit requires GUI calls there).
#[cfg(target_os = "macos")]
pub(crate) fn on_main_thread() -> bool {
    unsafe extern "C" {
        fn pthread_main_np() -> std::ffi::c_int;
    }
    // SAFETY: a libSystem function without arguments or preconditions.
    unsafe { pthread_main_np() == 1 }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn on_main_thread() -> bool {
    true
}

/// The windowing API of this platform.
fn window_api() -> Option<&'static CStr> {
    if cfg!(target_os = "macos") {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_COCOA)
    } else if cfg!(windows) {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_WIN32)
    } else if cfg!(all(unix, not(target_os = "android"))) {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_X11)
    } else {
        None
    }
}

struct GuiInner {
    plugin: *const clap_plugin,
    gui: *const clap_plugin_gui,
    state: *const clap_plugin_state,
    /// The plugin instance exists (cleared by `Instance::drop` before it destroys the plugin).
    alive: bool,
    created: bool,
    visible: bool,
    /// The host window of an embedded editor (`None` for floating editors).
    window: Option<hostwin::HostWindow>,
    /// The plugin's `clap.timer-support` and `clap.posix-fd-support` (null when absent).
    timer_ext: *const clap_plugin_timer_support,
    fd_ext: *const clap_plugin_posix_fd_support,
}

/// The link between an instance and its editor and state handles. The instance may process on
/// the audio thread while the editor is driven and the state saved from the main thread: CLAP
/// allows `clap.gui` and `clap.state` (main-thread) calls concurrently with audio-thread calls.
/// Every such call happens under this lock, and the instance clears `alive` (destroying any GUI)
/// under the same lock before it destroys the plugin, so a handle can never call into a destroyed
/// plugin.
pub(crate) struct GuiLink {
    inner: Mutex<GuiInner>,
    host: Arc<HostState>,
}

impl Default for GuiLink {
    fn default() -> Self {
        GuiLink {
            inner: Mutex::new(GuiInner {
                plugin: std::ptr::null(),
                gui: std::ptr::null(),
                state: std::ptr::null(),
                alive: false,
                created: false,
                visible: false,
                window: None,
                timer_ext: std::ptr::null(),
                fd_ext: std::ptr::null(),
            }),
            host: Arc::new(HostState::default()),
        }
    }
}

// SAFETY: the raw pointers are only dereferenced under the mutex, while `alive` guarantees the
// plugin exists; CLAP's threading rules (main-thread GUI calls) are checked at each call.
unsafe impl Send for GuiInner {}

impl GuiLink {
    fn lock(&self) -> std::sync::MutexGuard<'_, GuiInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn available(&self) -> bool {
        let g = self.lock();
        g.alive && !g.gui.is_null()
    }

    pub fn host(&self) -> &HostState {
        &self.host
    }

    fn vt(g: &GuiInner) -> Option<clap_plugin_gui> {
        // SAFETY: `gui` is null or the plugin's static extension vtable, valid while `alive`.
        (g.alive).then(|| unsafe { g.gui.as_ref() }.copied()).flatten()
    }

    /// Opens (creates and shows) the plugin's floating editor window.
    pub fn open(&self, title: &str) -> Result<(), String> {
        if !on_main_thread() {
            return Err("plugin editors must be opened on the main thread".into());
        }
        let mut g = self.lock();
        let vt = GuiLink::vt(&g).ok_or_else(|| "this plugin has no editor".to_string())?;
        if g.created {
            if let Some(show) = vt.show {
                // SAFETY: live plugin with a created GUI; main thread (checked).
                unsafe { show(g.plugin) };
            }
            if let Some(w) = &g.window {
                w.raise();
            }
            g.visible = true;
            return Ok(());
        }
        let api = window_api().ok_or_else(|| "plugin editors are not supported on this platform".to_string())?;
        let (Some(supported), Some(create)) = (vt.is_api_supported, vt.create) else { return Err("incomplete clap.gui extension".into()) };
        // On X11 an embedded editor lives in our window: transient for the app, closing it only
        // hides it, and its focus is ours to give. A plugin-owned floating window has none of
        // that, so it is the fallback there.
        // SAFETY: live plugin; `api` is a static NUL-terminated string; main thread.
        if cfg!(target_os = "linux") && hostwin::SUPPORTED && unsafe { supported(g.plugin, api.as_ptr(), false) } {
            return self.open_embedded(&mut g, vt, api, title);
        }
        // SAFETY: as above.
        if !unsafe { supported(g.plugin, api.as_ptr(), true) } {
            // SAFETY: as above.
            if hostwin::SUPPORTED && unsafe { supported(g.plugin, api.as_ptr(), false) } {
                return self.open_embedded(&mut g, vt, api, title);
            }
            return Err("this plugin offers only an embedded editor, which SoundCraft hosts on Windows and Linux (X11) only".into());
        }
        self.host.gui_closed.store(false, Ordering::Relaxed);
        self.host.gui_destroyed.store(false, Ordering::Relaxed);
        // SAFETY: as above; no GUI exists yet.
        if !unsafe { create(g.plugin, api.as_ptr(), true) } {
            return Err("the plugin could not create its editor".into());
        }
        g.created = true;
        if !cfg!(target_os = "macos")
            && let Some(f) = vt.set_scale
        {
            // SAFETY: live plugin with a created GUI. (Cocoa uses logical pixels: not called.)
            unsafe { f(g.plugin, 1.0) };
        }
        if let (Some(f), Ok(t)) = (vt.suggest_title, CString::new(title)) {
            // SAFETY: live plugin with a created floating GUI; `t` outlives the call.
            unsafe { f(g.plugin, t.as_ptr()) };
        }
        let shown = match vt.show {
            // SAFETY: as above.
            Some(f) => unsafe { f(g.plugin) },
            None => false,
        };
        if !shown {
            GuiLink::destroy(&mut g);
            return Err("the plugin could not show its editor".into());
        }
        g.visible = true;
        Ok(())
    }

    /// Embeds the editor in a host window of ours (plugins that offer no floating editor).
    fn open_embedded(&self, g: &mut GuiInner, vt: clap_plugin_gui, api: &CStr, title: &str) -> Result<(), String> {
        let (Some(create), Some(set_parent)) = (vt.create, vt.set_parent) else { return Err("incomplete clap.gui extension".into()) };
        self.host.gui_closed.store(false, Ordering::Relaxed);
        self.host.gui_destroyed.store(false, Ordering::Relaxed);
        self.host.gui_resize.store(0, Ordering::Relaxed);
        // SAFETY: live plugin; `api` is a static NUL-terminated string; main thread; no GUI yet.
        if !unsafe { create(g.plugin, api.as_ptr(), false) } {
            return Err("the plugin could not create its editor".into());
        }
        g.created = true;
        if let Some(f) = vt.set_scale {
            // SAFETY: live plugin with a created GUI.
            unsafe { f(g.plugin, 1.0) };
        }
        let (mut w, mut h) = (640u32, 480u32);
        if let Some(f) = vt.get_size {
            let (mut a, mut b) = (0u32, 0u32);
            // SAFETY: live plugin with a created GUI; both out-pointers are valid locals.
            if unsafe { f(g.plugin, &mut a, &mut b) } && a > 0 && b > 0 {
                (w, h) = (a.min(16_384), b.min(16_384));
            }
        }
        let win = match hostwin::HostWindow::new(title, f64::from(w), f64::from(h)) {
            Ok(win) => win,
            Err(e) => {
                GuiLink::destroy(g);
                return Err(e);
            }
        };
        let parent = clap_window { api: api.as_ptr(), specific: win.handle() };
        // SAFETY: live plugin with a created embedded GUI; `parent` names our live window and
        // outlives the call.
        if !unsafe { set_parent(g.plugin, &parent) } {
            win.close();
            GuiLink::destroy(g);
            return Err("the editor could not attach to its window".into());
        }
        g.window = Some(win);
        let shown = match vt.show {
            // SAFETY: live plugin with a created, parented GUI.
            Some(f) => unsafe { f(g.plugin) },
            None => true,
        };
        if !shown {
            GuiLink::destroy(g);
            return Err("the plugin could not show its editor".into());
        }
        if let Some(win) = &g.window {
            win.raise();
        }
        g.visible = true;
        Ok(())
    }

    fn destroy(g: &mut GuiInner) {
        if g.created
            && let Some(f) = GuiLink::vt(g).and_then(|v| v.destroy)
        {
            // SAFETY: live plugin with a created GUI; destroyed exactly once.
            unsafe { f(g.plugin) };
        }
        g.created = false;
        g.visible = false;
        // The plugin's child window is gone; now its host window can go too.
        if let Some(w) = g.window.take() {
            w.close();
        }
    }

    /// `clap_plugin_state.save` into memory (`None` without the extension, on failure, or once the
    /// plugin is gone).
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let g = self.lock();
        if !g.alive {
            return None;
        }
        // SAFETY: `state` is null or the plugin's static extension vtable, valid while `alive`.
        let save = unsafe { g.state.as_ref() }?.save?;
        let mut out: Vec<u8> = Vec::new();
        let stream = clap_ostream { ctx: (&mut out as *mut Vec<u8>).cast::<c_void>(), write: Some(ostream_write) };
        // SAFETY: live plugin (`alive`, under the lock); the stream and its context live on this
        // frame for the call.
        unsafe { save(g.plugin, &stream) }.then_some(out)
    }

    /// Closes (destroys) the editor.
    pub fn close(&self) {
        if !on_main_thread() {
            return;
        }
        GuiLink::destroy(&mut self.lock());
    }

    pub fn is_open(&self) -> bool {
        let g = self.lock();
        g.alive && g.created && g.visible && g.window.as_ref().is_none_or(hostwin::HostWindow::is_visible)
    }

    /// Main-thread housekeeping: runs a requested `on_main_thread`, follows the plugin's
    /// show/hide/closed requests, and returns the parameter edits reported since the last call.
    pub fn idle(&self) -> Vec<(u32, f64)> {
        if !on_main_thread() {
            return Vec::new();
        }
        let mut g = self.lock();
        if !g.alive {
            return Vec::new();
        }
        if self.host.callback.swap(false, Ordering::Relaxed) {
            // SAFETY: `plugin` is valid while `alive`; we copy its vtable struct.
            if let Some(f) = unsafe { (*g.plugin).on_main_thread } {
                // SAFETY: live plugin, main thread, requested via `request_callback`.
                unsafe { f(g.plugin) };
            }
        }
        GuiLink::pump_run_loop(&g, &self.host);
        if let Some(win) = &g.window {
            // X11: drain the host window's events (a window-manager close hides it). Windows
            // dispatches through the app's own message loop: a no-op there.
            win.poll_events();
        }
        if g.created {
            let resize = self.host.gui_resize.swap(0, Ordering::Relaxed);
            if resize != 0
                && let Some(win) = &g.window
            {
                let (w, h) = ((resize >> 32) as u32, resize as u32);
                hostwin::resize(win.raw(), f64::from(w), f64::from(h));
                if let Some(f) = GuiLink::vt(&g).and_then(|v| v.set_size) {
                    // SAFETY: live plugin with a created embedded GUI, main thread.
                    unsafe { f(g.plugin, w, h) };
                }
            }
            if g.window.as_ref().is_some_and(|w| !w.is_visible()) {
                // The user closed the embedded editor's host window: free the editor.
                GuiLink::destroy(&mut g);
            } else if self.host.gui_closed.swap(false, Ordering::Relaxed) {
                // The user closed the floating window (or the plugin lost its GUI): free it.
                self.host.gui_destroyed.store(false, Ordering::Relaxed);
                GuiLink::destroy(&mut g);
            } else if let Some(vt) = GuiLink::vt(&g) {
                if self.host.gui_show.swap(false, Ordering::Relaxed)
                    && let Some(f) = vt.show
                {
                    // SAFETY: live plugin with a created GUI, main thread.
                    g.visible = unsafe { f(g.plugin) };
                }
                if self.host.gui_hide.swap(false, Ordering::Relaxed)
                    && let Some(f) = vt.hide
                {
                    // SAFETY: as above.
                    unsafe { f(g.plugin) };
                    g.visible = false;
                }
            }
        }
        drop(g);
        let mut e = self.host.edits.lock().unwrap_or_else(PoisonError::into_inner);
        let out = e.clone();
        e.clear();
        out
    }

    /// Fires the plugin's due `clap.timer-support` timers and reports its ready
    /// `clap.posix-fd-support` descriptors (zero-timeout `poll`). Main thread, `g.alive`. The
    /// registries are only locked to read them: a callback may (un)register timers and fds, and
    /// each one is re-checked right before its call so a just-removed one is never reported.
    fn pump_run_loop(g: &GuiInner, host: &HostState) {
        if !g.alive {
            return;
        }
        // SAFETY: null or the plugin's static extension vtable, valid while `alive`.
        if let Some(on_timer) = unsafe { g.timer_ext.as_ref() }.and_then(|t| t.on_timer) {
            let due = host.timers.lock().unwrap_or_else(PoisonError::into_inner).due(Instant::now());
            for id in due {
                let live = host.timers.lock().unwrap_or_else(PoisonError::into_inner).contains(id);
                if live {
                    // SAFETY: live plugin, main thread, a timer it registered and still holds.
                    unsafe { on_timer(g.plugin, id) };
                }
            }
        }
        // SAFETY: as above.
        if let Some(on_fd) = unsafe { g.fd_ext.as_ref() }.and_then(|f| f.on_fd) {
            let fds = host.fds.lock().unwrap_or_else(PoisonError::into_inner).snapshot();
            for (fd, flags) in poll_fds(&fds) {
                let wanted = host.fds.lock().unwrap_or_else(PoisonError::into_inner).flags(fd);
                let flags = flags & wanted.unwrap_or(0);
                if flags != 0 {
                    // SAFETY: live plugin, main thread, an fd it registered and still holds.
                    unsafe { on_fd(g.plugin, fd, flags) };
                }
            }
        }
    }

    /// Called by the instance before it destroys the plugin.
    fn kill(&self) {
        let mut g = self.lock();
        if g.created {
            if on_main_thread() {
                GuiLink::destroy(&mut g);
            } else {
                log::warn!("clap: editor still open while its plugin is destroyed off the main thread");
                g.created = false;
                // Window calls are main-thread only: leave the host window rather than crash.
                g.window = None;
            }
        }
        g.alive = false;
        drop(g);
        self.host.timers.lock().unwrap_or_else(PoisonError::into_inner).clear();
        self.host.fds.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

// ---- audio buffers -------------------------------------------------------------------------

/// Planar f32 buffers for every audio port in one direction, with the pointer tables CLAP needs.
/// Allocated in `new`; `process` only rewrites pointers in place (no allocation).
pub(crate) struct PortBuffers {
    data: Vec<Vec<Vec<f32>>>,
    ptrs: Vec<Vec<*mut f32>>,
    bufs: Vec<clap_audio_buffer>,
    frames: usize,
}

// SAFETY: the raw pointers only ever point into `data`, which this struct owns; moving the struct
// to another thread moves that ownership with it.
unsafe impl Send for PortBuffers {}

impl PortBuffers {
    /// One buffer set per port, `channels[p]` channels each (capped), `frames` long.
    pub fn new(channels: &[u32], frames: usize) -> PortBuffers {
        let data: Vec<Vec<Vec<f32>>> =
            channels.iter().take(MAX_PORTS as usize).map(|&c| (0..c.min(MAX_PORT_CHANNELS)).map(|_| vec![0.0; frames]).collect()).collect();
        let ptrs = data.iter().map(|p| vec![std::ptr::null_mut(); p.len()]).collect();
        let bufs = data
            .iter()
            .map(|_| clap_audio_buffer { data32: std::ptr::null_mut(), data64: std::ptr::null_mut(), channel_count: 0, latency: 0, constant_mask: 0 })
            .collect();
        PortBuffers { data, ptrs, bufs, frames }
    }

    pub fn channels(&self, port: usize) -> usize {
        self.data.get(port).map_or(0, Vec::len)
    }

    pub fn channel(&self, port: usize, ch: usize) -> Option<&[f32]> {
        self.data.get(port).and_then(|p| p.get(ch)).map(Vec::as_slice)
    }

    pub fn channel_mut(&mut self, port: usize, ch: usize) -> Option<&mut [f32]> {
        self.data.get_mut(port).and_then(|p| p.get_mut(ch)).map(Vec::as_mut_slice)
    }

    /// Points the CLAP tables at the current data (no allocation).
    fn refresh(&mut self) {
        for ((d, p), b) in self.data.iter_mut().zip(self.ptrs.iter_mut()).zip(self.bufs.iter_mut()) {
            for (dc, pc) in d.iter_mut().zip(p.iter_mut()) {
                *pc = dc.as_mut_ptr();
            }
            b.data32 = p.as_mut_ptr();
            b.data64 = std::ptr::null_mut();
            b.channel_count = u32::try_from(p.len()).unwrap_or(0);
            b.constant_mask = 0;
        }
    }
}

// ---- instances -----------------------------------------------------------------------------

/// What one audio port looks like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PortInfo {
    pub channels: u32,
    pub main: bool,
}

/// One parameter as reported by `clap_plugin_params.get_info`.
#[derive(Debug, Clone)]
pub(crate) struct RawParam {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub flags: u32,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// Note dialects of the plugin's first input note port.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NotePort {
    pub supported: u32,
    pub preferred: u32,
}

/// A live plugin instance plus the host struct it was created with.
pub(crate) struct Instance {
    plugin: *const clap_plugin,
    params: *const clap_plugin_params,
    ports: *const clap_plugin_audio_ports,
    latency: *const clap_plugin_latency,
    notes: *const clap_plugin_note_ports,
    state_ext: *const clap_plugin_state,
    gui: Arc<GuiLink>,
    raw: Vec<RawEvent>,
    /// Parameter values the plugin reported through output events in the current `process`.
    out_params: Vec<(u32, f64)>,
    active: bool,
    processing: bool,
    max_frames: usize,
    created: bool,
    // Field order matters for drop: the plugin is destroyed in `Drop::drop` first; then the host
    // struct and its state, then the bundle (which keeps the code mapped) go.
    _host: Box<clap_host>,
    state: Arc<HostState>,
    _bundle: Arc<Bundle>,
}

// SAFETY: an `Instance` is used through `&mut self` only, so calls into the plugin are always
// serialised. CLAP lets audio-thread functions run on any one thread at a time; SoundCraft creates
// instances on the engine/UI thread and then moves them into the mix engine (documented
// limitation: some main-thread calls such as `activate` may then happen on the audio thread).
unsafe impl Send for Instance {}

impl Instance {
    fn vt(&self) -> clap_plugin {
        // SAFETY: `plugin` is non-null (checked at creation) and alive until `Drop`.
        unsafe { *self.plugin }
    }

    fn extension<T>(&self, id: &CStr) -> *const T {
        match self.vt().get_extension {
            // SAFETY: valid instance; `id` is a static NUL-terminated string.
            Some(f) => unsafe { f(self.plugin, id.as_ptr()) }.cast::<T>(),
            None => std::ptr::null(),
        }
    }

    pub fn state(&self) -> &HostState {
        &self.state
    }

    /// The editor link shared with editor handles (`None` when the plugin has no `clap.gui`).
    pub fn gui(&self) -> Option<Arc<GuiLink>> {
        self.gui.available().then(|| Arc::clone(&self.gui))
    }

    /// The same link, for state handles (`None` when the plugin has no `clap.state`).
    pub fn state_link(&self) -> Option<Arc<GuiLink>> {
        (!self.state_ext.is_null()).then(|| Arc::clone(&self.gui))
    }

    /// `clap_plugin_state.load` from memory. `false` without the extension or when refused.
    pub fn load_state(&self, data: &[u8]) -> bool {
        // SAFETY: the extension pointer, when non-null, points to a static vtable in the plugin.
        let Some(load) = (unsafe { self.state_ext.as_ref() }).and_then(|s| s.load) else { return false };
        let mut cursor = ReadCursor { data, pos: 0 };
        let stream = clap_istream { ctx: (&mut cursor as *mut ReadCursor<'_>).cast::<c_void>(), read: Some(istream_read) };
        // SAFETY: valid instance; the stream and its cursor live on this frame for the call.
        unsafe { load(self.plugin, &stream) }
    }

    /// Runs `on_main_thread` if the plugin asked for a callback.
    pub fn service_callback(&mut self) {
        if self.state.callback.swap(false, Ordering::Relaxed)
            && let Some(f) = self.vt().on_main_thread
        {
            // SAFETY: valid instance; the plugin asked for this call via `request_callback`.
            unsafe { f(self.plugin) };
        }
    }

    pub fn params(&self) -> Vec<RawParam> {
        // SAFETY: the extension pointer, when non-null, points to a static vtable in the plugin.
        let Some(p) = (unsafe { self.params.as_ref() }) else { return Vec::new() };
        let (Some(count), Some(get_info)) = (p.count, p.get_info) else { return Vec::new() };
        // SAFETY: vtable call with the instance it belongs to.
        let n = unsafe { count(self.plugin) }.min(MAX_PARAMS);
        let mut out = Vec::new();
        for i in 0..n {
            // SAFETY: `clap_param_info` is plain data (integers, floats, char arrays, a nullable
            // pointer); all-zero is a valid value.
            let mut info: clap_param_info = unsafe { std::mem::zeroed() };
            // SAFETY: index below the count; `info` is a valid, writable struct.
            if !unsafe { get_info(self.plugin, i, &mut info) } {
                continue;
            }
            let (min, max) = (info.min_value, info.max_value);
            if !(min.is_finite() && max.is_finite()) || max < min {
                continue;
            }
            let default = if info.default_value.is_finite() { info.default_value.clamp(min, max) } else { min };
            out.push(RawParam { id: info.id, name: read_name(&info.name), module: read_name(&info.module), flags: info.flags, min, max, default });
        }
        out
    }

    pub fn param_value(&self, id: u32) -> Option<f64> {
        // SAFETY: see `params`.
        let p = unsafe { self.params.as_ref() }?;
        let get = p.get_value?;
        let mut v = 0.0f64;
        // SAFETY: valid instance; `v` is writable.
        (unsafe { get(self.plugin, id, &mut v) } && v.is_finite()).then_some(v)
    }

    pub fn value_text(&self, id: u32, value: f64) -> Option<String> {
        // SAFETY: see `params`.
        let p = unsafe { self.params.as_ref() }?;
        let f = p.value_to_text?;
        let mut buf = [0 as c_char; 128];
        // SAFETY: valid instance; we pass the buffer's true capacity.
        let ok = unsafe { f(self.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32) };
        ok.then(|| read_name(&buf)).filter(|s| !s.is_empty())
    }

    pub fn audio_ports(&self, is_input: bool) -> Vec<PortInfo> {
        // SAFETY: see `params`.
        let Some(p) = (unsafe { self.ports.as_ref() }) else { return Vec::new() };
        let (Some(count), Some(get)) = (p.count, p.get) else { return Vec::new() };
        // SAFETY: vtable call with its instance.
        let n = unsafe { count(self.plugin, is_input) }.min(MAX_PORTS);
        let mut out = Vec::new();
        for i in 0..n {
            // SAFETY: plain data; all-zero (null port_type pointer) is valid.
            let mut info: clap_audio_port_info = unsafe { std::mem::zeroed() };
            // SAFETY: index below the count; `info` is writable.
            if unsafe { get(self.plugin, i, is_input, &mut info) } {
                out.push(PortInfo { channels: info.channel_count.min(MAX_PORT_CHANNELS), main: info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0 });
            } else {
                // Keep port indices aligned: an unreadable port still needs a (silent) buffer slot.
                out.push(PortInfo { channels: 0, main: false });
            }
        }
        out
    }

    pub fn note_port(&self) -> Option<NotePort> {
        // SAFETY: see `params`.
        let p = unsafe { self.notes.as_ref() }?;
        let (count, get) = (p.count?, p.get?);
        // SAFETY: vtable call with its instance.
        if unsafe { count(self.plugin, true) } == 0 {
            return None;
        }
        // SAFETY: plain data; all-zero is valid.
        let mut info: clap_note_port_info = unsafe { std::mem::zeroed() };
        // SAFETY: index 0 is below the non-zero count; `info` is writable.
        unsafe { get(self.plugin, 0, true, &mut info) }.then_some(NotePort { supported: info.supported_dialects, preferred: info.preferred_dialect })
    }

    pub fn latency(&self) -> u32 {
        // SAFETY: see `params`.
        match unsafe { self.latency.as_ref() }.and_then(|l| l.get) {
            // SAFETY: valid instance.
            Some(f) => unsafe { f(self.plugin) },
            None => 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn activate(&mut self, sample_rate: f64, min_frames: u32, max_frames: u32) -> Result<(), ClapError> {
        self.deactivate();
        let f = self.vt().activate.ok_or_else(|| ClapError::Process("plugin has no activate".into()))?;
        // SAFETY: valid, inactive instance; arguments are sane (checked by the caller).
        if !unsafe { f(self.plugin, sample_rate, min_frames, max_frames) } {
            return Err(ClapError::Process("activate failed".into()));
        }
        self.active = true;
        self.max_frames = max_frames as usize;
        Ok(())
    }

    pub fn deactivate(&mut self) {
        self.stop_processing();
        if self.active {
            if let Some(f) = self.vt().deactivate {
                // SAFETY: valid, active instance.
                unsafe { f(self.plugin) };
            }
            self.active = false;
        }
    }

    fn stop_processing(&mut self) {
        if self.processing {
            if let Some(f) = self.vt().stop_processing {
                // SAFETY: valid instance in the processing state.
                unsafe { f(self.plugin) };
            }
            self.processing = false;
        }
    }

    pub fn reset(&mut self) {
        if self.active
            && let Some(f) = self.vt().reset
        {
            // SAFETY: valid, active instance.
            unsafe { f(self.plugin) };
        }
    }

    /// Runs one `clap_process` call over `frames` frames. Events must be sorted by time; their
    /// times are made relative to `offset`. Returns the CLAP process status.
    pub fn process(
        &mut self,
        frames: usize,
        steady_time: i64,
        inputs: &mut PortBuffers,
        outputs: &mut PortBuffers,
        events: &[Event],
        offset: u32,
        transport: Option<&Transport>,
    ) -> Result<i32, ClapError> {
        if !self.active {
            return Err(ClapError::Process("not active".into()));
        }
        if frames == 0 || frames > self.max_frames || frames > inputs.frames || frames > outputs.frames {
            return Err(ClapError::Process("block larger than the activated size".into()));
        }
        if !self.processing {
            let ok = match self.vt().start_processing {
                // SAFETY: valid, active instance.
                Some(f) => unsafe { f(self.plugin) },
                None => true,
            };
            if !ok {
                return Err(ClapError::Process("start_processing failed".into()));
            }
            self.processing = true;
        }
        let process = self.vt().process.ok_or_else(|| ClapError::Process("plugin has no process".into()))?;
        self.raw.clear();
        for e in events.iter().take(self.raw.capacity()) {
            self.raw.push(RawEvent::from_event(e, offset));
        }
        inputs.refresh();
        outputs.refresh();
        let in_events = clap_input_events {
            ctx: (&self.raw as *const Vec<RawEvent>).cast_mut().cast::<c_void>(),
            size: Some(in_events_size),
            get: Some(in_events_get),
        };
        self.out_params.clear();
        let out_events =
            clap_output_events { ctx: (&mut self.out_params as *mut Vec<(u32, f64)>).cast::<c_void>(), try_push: Some(out_events_try_push) };
        let transport = transport.map(transport_event);
        let p = clap_process {
            steady_time,
            frames_count: u32::try_from(frames).unwrap_or(0),
            transport: transport.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
            audio_inputs: inputs.bufs.as_ptr(),
            audio_outputs: outputs.bufs.as_mut_ptr(),
            audio_inputs_count: u32::try_from(inputs.bufs.len()).unwrap_or(0),
            audio_outputs_count: u32::try_from(outputs.bufs.len()).unwrap_or(0),
            in_events: &in_events,
            out_events: &out_events,
        };
        // SAFETY: valid, processing instance. Every buffer pointer refers to `frames` (or more)
        // valid f32s owned by `inputs`/`outputs`, which are borrowed for the whole call; the event
        // lists and their contexts and the transport live on this stack frame / in `self.raw`,
        // untouched until the call returns.
        let status = unsafe { process(self.plugin, &p) };
        if !self.out_params.is_empty()
            && let Ok(mut edits) = self.state.edits.try_lock()
        {
            // Never blocks or grows: a busy UI or a full queue drops the edits.
            for &e in &self.out_params {
                if edits.len() < edits.capacity() {
                    edits.push(e);
                }
            }
        }
        self.out_params.clear();
        if status == CLAP_PROCESS_ERROR { Err(ClapError::Process("plugin reported a processing error".into())) } else { Ok(status) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Editors first (CLAP: destroy the GUI before the plugin), and no handle may call in after.
        self.gui.kill();
        self.deactivate();
        if self.created
            && let Some(f) = self.vt().destroy
        {
            // SAFETY: valid instance, deactivated above; destroyed exactly once.
            unsafe { f(self.plugin) };
        }
        self.created = false;
    }
}

// ---- host window for embedded editors ------------------------------------------------------

/// The Windows host window for embedded editors: a top-level window owned by the app's active
/// window (no taskbar button) whose client area is the plugin's parent `HWND`. Closing it only
/// hides it; the editor link notices in `idle`, destroys the plugin's GUI, then the window.
/// Messages are dispatched by the app's own event loop on the main thread.
#[cfg(windows)]
mod hostwin {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GWL_STYLE,
        GetWindowLongPtrW, IDC_ARROW, IsWindow, IsWindowVisible, LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_OVERLAPPED, WS_SYSMENU,
    };

    pub const SUPPORTED: bool = true;

    /// Rust names the process's first thread "main"; the app runs its UI (and message loop) there.
    fn on_main_thread() -> bool {
        std::thread::current().name() == Some("main")
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().filter(|&c| c != 0).chain(Some(0)).collect()
    }

    const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_CLOSE {
            // SAFETY: `hwnd` is the window this procedure was called for.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        // SAFETY: default handling of a message for the window it was sent to.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Registers the window class once; the name lives for the whole process.
    fn class_name() -> Option<&'static [u16]> {
        static CLASS: OnceLock<Option<Vec<u16>>> = OnceLock::new();
        CLASS
            .get_or_init(|| {
                let name = wide("SoundCraftClapEditor");
                // SAFETY: null asks for this process's module handle.
                let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: instance,
                    hIcon: std::ptr::null_mut(),
                    // SAFETY: a system cursor (null instance + predefined id).
                    hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: name.as_ptr(),
                    hIconSm: std::ptr::null_mut(),
                };
                // SAFETY: a fully initialised class description whose strings outlive the call
                // (the name is kept for the process lifetime in this `OnceLock`).
                (unsafe { RegisterClassExW(&wc) } != 0).then_some(name)
            })
            .as_deref()
    }

    /// Outer window size for a client area of `w`×`h`.
    fn outer_size(style: u32, ex_style: u32, w: f64, h: f64) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
        // SAFETY: `r` is a valid, writable rect.
        unsafe { AdjustWindowRectEx(&mut r, style, 0, ex_style) };
        (r.right.saturating_sub(r.left), r.bottom.saturating_sub(r.top))
    }

    /// Owns the window handle; destroyed in `close` (main thread only).
    pub struct HostWindow {
        hwnd: HWND,
    }

    // SAFETY: the handle is only used on the main thread (every method checks); the struct
    // merely carries it between threads' locks.
    unsafe impl Send for HostWindow {}

    impl HostWindow {
        fn live(&self) -> Option<HWND> {
            // SAFETY: `IsWindow` accepts any value and only reports whether it names a window.
            (on_main_thread() && unsafe { IsWindow(self.hwnd) } != 0).then_some(self.hwnd)
        }

        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            if !on_main_thread() {
                return Err("plugin editors must be opened on the main thread".into());
            }
            let class = class_name().ok_or_else(|| "cannot register the editor window class".to_string())?;
            let title = wide(title);
            let (ow, oh) = outer_size(STYLE, 0, w, h);
            // SAFETY: main thread; the active window (or null) of this thread's queue.
            let owner = unsafe { GetActiveWindow() };
            // SAFETY: registered class and NUL-terminated strings that outlive the call; the
            // owner is null or a window of this thread.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    STYLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    ow,
                    oh,
                    owner,
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err("cannot create the editor window".into());
            }
            Ok(HostWindow { hwnd })
        }

        /// The window handle (for resizes).
        pub fn raw(&self) -> *mut c_void {
            self.hwnd
        }

        /// The `HWND` the plugin creates its child window in.
        pub fn parent(&self) -> *mut c_void {
            self.live().unwrap_or(std::ptr::null_mut())
        }

        /// The parent as `clap_window.win32`.
        pub fn handle(&self) -> super::clap_window_handle {
            super::clap_window_handle { win32: self.parent() }
        }

        /// Messages reach `wndproc` through the app's own message loop: nothing to drain.
        pub fn poll_events(&self) {}

        pub fn raise(&self) {
            if let Some(h) = self.live() {
                // SAFETY: a live window of this thread.
                unsafe {
                    ShowWindow(h, SW_SHOW);
                    SetForegroundWindow(h);
                }
            }
        }

        pub fn is_visible(&self) -> bool {
            // SAFETY: a live window of this thread.
            self.live().is_some_and(|h| unsafe { IsWindowVisible(h) } != 0)
        }

        pub fn close(self) {
            if let Some(h) = self.live() {
                // SAFETY: our own live window; the plugin's GUI was destroyed first.
                unsafe { DestroyWindow(h) };
            }
        }
    }

    /// Resizes a host window's client area (`window` from [`HostWindow::raw`]); main thread only.
    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        // SAFETY: `IsWindow` accepts any value.
        if !on_main_thread() || window.is_null() || unsafe { IsWindow(window) } == 0 {
            return;
        }
        // SAFETY: a live window of this thread; style values are read, not changed.
        let (style, ex) = unsafe { (GetWindowLongPtrW(window, GWL_STYLE) as u32, GetWindowLongPtrW(window, GWL_EXSTYLE) as u32) };
        let (ow, oh) = outer_size(style, ex, w, h);
        // SAFETY: as above; only the size changes.
        unsafe { SetWindowPos(window, std::ptr::null_mut(), 0, 0, ow, oh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }
}

/// The Linux host window for embedded editors: an X11 top-level window (through XWayland under
/// Wayland) whose window id is the plugin's `clap_window.x11` parent. It is transient for the app
/// window (ICCCM `WM_TRANSIENT_FOR`), so window managers keep it above SoundCraft. The window
/// manager's close button (`WM_DELETE_WINDOW`) only unmaps it: the editor link drains this
/// window's events in `idle`, notices, destroys the plugin's GUI and then the window, while the
/// plugin instance keeps processing.
///
/// Each window has its own Xlib connection, used only under the editor link's lock. libX11 is
/// linked the way `soundcraft-vst3-host` links it (`#[link(name = "X11")]`); the declarations
/// below are the documented Xlib C API (Xlib manual, ICCCM, EWMH).
#[cfg(target_os = "linux")]
mod hostwin {
    use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock, PoisonError};

    pub const SUPPORTED: bool = true;

    /// Xlib's opaque connection.
    #[repr(C)]
    pub struct Display {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct XClassHint {
        res_name: *mut c_char,
        res_class: *mut c_char,
    }

    /// `XSizeHints` (the two aspect `struct { int x, y; }` members flattened: same layout).
    #[repr(C)]
    #[derive(Default)]
    struct XSizeHints {
        flags: c_long,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
        min_width: c_int,
        min_height: c_int,
        max_width: c_int,
        max_height: c_int,
        width_inc: c_int,
        height_inc: c_int,
        min_aspect_x: c_int,
        min_aspect_y: c_int,
        max_aspect_x: c_int,
        max_aspect_y: c_int,
        base_width: c_int,
        base_height: c_int,
        win_gravity: c_int,
    }

    /// Room for any `XEvent` (a union of 24 longs); members are read through the structs below.
    #[repr(C, align(8))]
    struct XEventBuf([u8; 256]);

    #[repr(C)]
    struct XAnyEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: c_ulong,
    }

    #[repr(C)]
    struct XClientMessageEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: c_ulong,
        message_type: c_ulong,
        format: c_int,
        data: [c_long; 5],
    }

    type XErrorHandler = Option<unsafe extern "C" fn(*mut Display, *mut c_void) -> c_int>;

    const DESTROY_NOTIFY: c_int = 17;
    const CLIENT_MESSAGE: c_int = 33;
    const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
    const SUBSTRUCTURE_NOTIFY_MASK: c_long = 1 << 19;
    const SUBSTRUCTURE_REDIRECT_MASK: c_long = 1 << 20;
    const P_SIZE: c_long = 1 << 3;
    const P_MIN_SIZE: c_long = 1 << 4;
    const P_MAX_SIZE: c_long = 1 << 5;
    const PROP_MODE_REPLACE: c_int = 0;
    const XA_ATOM: c_ulong = 4;
    const XA_CARDINAL: c_ulong = 6;
    const XA_WINDOW: c_ulong = 33;
    const REVERT_TO_PARENT: c_int = 2;
    const CURRENT_TIME: c_ulong = 0;
    /// Most top-level windows inspected when looking for the app window.
    const MAX_CLIENTS: usize = 4096;

    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut Display;
        fn XCloseDisplay(display: *mut Display) -> c_int;
        fn XDefaultRootWindow(display: *mut Display) -> c_ulong;
        fn XCreateSimpleWindow(
            display: *mut Display,
            parent: c_ulong,
            x: c_int,
            y: c_int,
            width: c_uint,
            height: c_uint,
            border_width: c_uint,
            border: c_ulong,
            background: c_ulong,
        ) -> c_ulong;
        fn XStoreName(display: *mut Display, window: c_ulong, name: *const c_char) -> c_int;
        fn XInternAtom(display: *mut Display, name: *const c_char, only_if_exists: c_int) -> c_ulong;
        fn XSetWMProtocols(display: *mut Display, window: c_ulong, protocols: *mut c_ulong, count: c_int) -> c_int;
        fn XSetClassHint(display: *mut Display, window: c_ulong, hint: *mut XClassHint) -> c_int;
        fn XGetClassHint(display: *mut Display, window: c_ulong, hint: *mut XClassHint) -> c_int;
        fn XSetWMNormalHints(display: *mut Display, window: c_ulong, hints: *mut XSizeHints);
        fn XSetTransientForHint(display: *mut Display, window: c_ulong, owner: c_ulong) -> c_int;
        fn XSelectInput(display: *mut Display, window: c_ulong, mask: c_long) -> c_int;
        fn XMapRaised(display: *mut Display, window: c_ulong) -> c_int;
        fn XUnmapWindow(display: *mut Display, window: c_ulong) -> c_int;
        fn XDestroyWindow(display: *mut Display, window: c_ulong) -> c_int;
        fn XResizeWindow(display: *mut Display, window: c_ulong, width: c_uint, height: c_uint) -> c_int;
        fn XSetInputFocus(display: *mut Display, window: c_ulong, revert_to: c_int, time: c_ulong) -> c_int;
        fn XChangeProperty(
            display: *mut Display,
            window: c_ulong,
            property: c_ulong,
            type_: c_ulong,
            format: c_int,
            mode: c_int,
            data: *const c_uchar,
            nelements: c_int,
        ) -> c_int;
        fn XGetWindowProperty(
            display: *mut Display,
            window: c_ulong,
            property: c_ulong,
            long_offset: c_long,
            long_length: c_long,
            delete: c_int,
            req_type: c_ulong,
            actual_type: *mut c_ulong,
            actual_format: *mut c_int,
            nitems: *mut c_ulong,
            bytes_after: *mut c_ulong,
            prop: *mut *mut c_uchar,
        ) -> c_int;
        fn XQueryTree(
            display: *mut Display,
            window: c_ulong,
            root: *mut c_ulong,
            parent: *mut c_ulong,
            children: *mut *mut c_ulong,
            count: *mut c_uint,
        ) -> c_int;
        fn XSendEvent(display: *mut Display, window: c_ulong, propagate: c_int, mask: c_long, event: *mut XEventBuf) -> c_int;
        fn XPending(display: *mut Display) -> c_int;
        fn XNextEvent(display: *mut Display, event: *mut XEventBuf) -> c_int;
        fn XFlush(display: *mut Display) -> c_int;
        fn XFree(data: *mut c_void) -> c_int;
        fn XSetErrorHandler(handler: XErrorHandler) -> XErrorHandler;
    }

    // ---- X errors ------------------------------------------------------------------------------

    /// Our connections (one per editor window), whose errors are ignored.
    fn ours() -> &'static Mutex<Vec<usize>> {
        static OURS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
        OURS.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// The error handler installed before ours (Xlib's default exits the process).
    static PREVIOUS: OnceLock<XErrorHandler> = OnceLock::new();

    /// Xlib calls one process-wide handler for errors on every connection. Errors on our editor
    /// connections (e.g. `BadWindow` when a window manager or plugin destroys a window under us)
    /// must never end the process: they are ignored. Errors on anyone else's connection (the
    /// app's, a plugin's) go to the handler that was there before, exactly as without us.
    unsafe extern "C" fn on_x_error(display: *mut Display, event: *mut c_void) -> c_int {
        let mine = ours().lock().map(|v| v.contains(&(display as usize))).unwrap_or(true);
        if !mine && let Some(Some(previous)) = PREVIOUS.get() {
            // SAFETY: the handler Xlib had before ours, called with the arguments Xlib gave us.
            return unsafe { previous(display, event) };
        }
        0
    }

    fn install_error_handler() {
        PREVIOUS.get_or_init(|| {
            // SAFETY: `on_x_error` has Xlib's `XErrorHandler` signature and never unwinds.
            unsafe { XSetErrorHandler(Some(on_x_error)) }
        });
    }

    // ---- windows -------------------------------------------------------------------------------

    fn cstring(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    /// A window dimension from a (possibly hostile) size.
    fn dim(v: f64) -> c_uint {
        if v.is_finite() { v.clamp(32.0, 16_384.0) as c_uint } else { 640 }
    }

    /// The plugin's parent window and the connection that owns it.
    struct WindowInner {
        display: *mut Display,
        window: c_ulong,
        protocols_atom: c_ulong,
        delete_atom: c_ulong,
        /// The window manager closed (or destroyed) the window.
        closed: AtomicBool,
    }

    impl WindowInner {
        fn atom(&self, name: &CStr) -> c_ulong {
            // SAFETY: live connection; `name` is NUL-terminated.
            unsafe { XInternAtom(self.display, name.as_ptr(), 0) }
        }

        /// Pins the window to the editor's size (ICCCM `WM_NORMAL_HINTS`): the plugin sizes its
        /// view itself and asks us through `request_resize`, so the user may not stretch it.
        fn set_fixed_size(&self, w: c_uint, h: c_uint) {
            let (w, h) = (w as c_int, h as c_int);
            let mut hints = XSizeHints {
                flags: P_SIZE | P_MIN_SIZE | P_MAX_SIZE,
                width: w,
                height: h,
                min_width: w,
                min_height: h,
                max_width: w,
                max_height: h,
                ..XSizeHints::default()
            };
            // SAFETY: live connection and window; `hints` is a valid struct for the call.
            unsafe { XSetWMNormalHints(self.display, self.window, &mut hints) };
        }

        /// Sets a format-32 property of `count` longs (`data` is that array).
        fn set_longs(&self, property: c_ulong, type_: c_ulong, data: &[c_ulong]) {
            let Ok(n) = c_int::try_from(data.len()) else { return };
            if property != 0 {
                // SAFETY: live connection and window; format-32 data is passed as C longs.
                unsafe { XChangeProperty(self.display, self.window, property, type_, 32, PROP_MODE_REPLACE, data.as_ptr().cast(), n) };
            }
        }

        fn set_title(&self, title: &str) {
            let t = cstring(title);
            // SAFETY: live connection and window; `t` outlives the call.
            unsafe { XStoreName(self.display, self.window, t.as_ptr()) };
            // EWMH `_NET_WM_NAME` carries the UTF-8 title (WM_NAME is Latin-1).
            let (name, utf8) = (self.atom(c"_NET_WM_NAME"), self.atom(c"UTF8_STRING"));
            let bytes = t.as_bytes();
            if let Ok(n) = c_int::try_from(bytes.len())
                && name != 0
                && utf8 != 0
            {
                // SAFETY: as above; format-8 data of `n` bytes.
                unsafe { XChangeProperty(self.display, self.window, name, utf8, 8, PROP_MODE_REPLACE, bytes.as_ptr(), n) };
            }
        }

        /// Asks the window manager to activate (focus and raise) the window (EWMH
        /// `_NET_ACTIVE_WINDOW`; focus is the WM's to give, so a refusal is fine).
        fn request_activation(&self) {
            let active = self.atom(c"_NET_ACTIVE_WINDOW");
            if active == 0 {
                return;
            }
            let mut buf = XEventBuf([0u8; 256]);
            // SAFETY: `buf` is 8-aligned and larger than an `XClientMessageEvent`.
            let cm = unsafe { &mut *(&mut buf as *mut XEventBuf).cast::<XClientMessageEvent>() };
            cm.type_ = CLIENT_MESSAGE;
            cm.send_event = 1;
            cm.display = self.display;
            cm.window = self.window;
            cm.message_type = active;
            cm.format = 32;
            // Source indication 1 = a normal application; timestamp unknown.
            cm.data = [1, 0, 0, 0, 0];
            // SAFETY: live connection; the root window takes EWMH requests with these masks.
            unsafe {
                let root = XDefaultRootWindow(self.display);
                XSendEvent(self.display, root, 0, SUBSTRUCTURE_REDIRECT_MASK | SUBSTRUCTURE_NOTIFY_MASK, &mut buf);
            }
        }
    }

    /// An X11 window hosting a plugin's embedded editor.
    pub struct HostWindow {
        inner: Box<WindowInner>,
    }

    // SAFETY: the Xlib connection belongs to this window alone and is only used by whoever holds
    // the editor link's lock (one thread at a time); the struct merely carries it between threads.
    unsafe impl Send for HostWindow {}

    impl HostWindow {
        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            install_error_handler();
            // SAFETY: null asks Xlib for the display named by $DISPLAY.
            let display = unsafe { XOpenDisplay(std::ptr::null()) };
            if display.is_null() {
                return Err("cannot open the X11 display (is DISPLAY set? Wayland needs XWayland)".into());
            }
            ours().lock().unwrap_or_else(PoisonError::into_inner).push(display as usize);
            let (width, height) = (dim(w), dim(h));
            // SAFETY: a live connection and its root window.
            let window = unsafe { XCreateSimpleWindow(display, XDefaultRootWindow(display), 0, 0, width, height, 0, 0, 0) };
            let inner = Box::new(WindowInner { display, window, protocols_atom: 0, delete_atom: 0, closed: AtomicBool::new(false) });
            if window == 0 {
                HostWindow { inner }.close();
                return Err("cannot create the editor window".into());
            }
            let mut inner = inner;
            inner.protocols_atom = inner.atom(c"WM_PROTOCOLS");
            inner.delete_atom = inner.atom(c"WM_DELETE_WINDOW");
            inner.set_title(title);
            let (name, class) = (cstring(EDITOR_NAME), cstring("SoundCraft"));
            let mut hint = XClassHint { res_name: name.as_ptr().cast_mut(), res_class: class.as_ptr().cast_mut() };
            let mut protocols = [inner.delete_atom];
            // SAFETY: live connection and window; every pointer argument outlives its call (Xlib
            // copies the class strings and the protocol list).
            unsafe {
                XSetClassHint(display, window, &mut hint);
                // The close button sends us `WM_DELETE_WINDOW` instead of killing the connection.
                XSetWMProtocols(display, window, protocols.as_mut_ptr(), 1);
                XSelectInput(display, window, STRUCTURE_NOTIFY_MASK);
            }
            inner.set_fixed_size(width, height);
            inner.set_longs(inner.atom(c"_NET_WM_PID"), XA_CARDINAL, &[c_ulong::from(std::process::id())]);
            if let Some(owner) = find_app_window(display) {
                if owner != window {
                    // SAFETY: as above; `owner` is a window id read from the server.
                    unsafe { XSetTransientForHint(display, window, owner) };
                }
            }
            // `_NET_WM_STATE_ABOVE`, set before mapping as EWMH asks, also keeps the editor over
            // other applications (the same choice as the VST3 host; see docs/audit-report.md §2.1).
            let above = inner.atom(c"_NET_WM_STATE_ABOVE");
            if above != 0 {
                inner.set_longs(inner.atom(c"_NET_WM_STATE"), XA_ATOM, &[above]);
            }
            // SAFETY: as above.
            unsafe {
                XMapRaised(display, window);
                XFlush(display);
            }
            inner.request_activation();
            // SAFETY: as above.
            unsafe { XFlush(display) };
            Ok(HostWindow { inner })
        }

        /// The pointer [`resize`] works through (the boxed [`WindowInner`], stable while `self` lives).
        pub fn raw(&self) -> *mut c_void {
            (&*self.inner as *const WindowInner).cast_mut().cast()
        }

        /// The X11 window id the plugin embeds its editor in (`clap_window.x11`).
        pub fn parent(&self) -> c_ulong {
            self.inner.window
        }

        /// The parent as `clap_window.x11`.
        pub fn handle(&self) -> super::clap_window_handle {
            super::clap_window_handle { x11: self.parent() }
        }

        pub fn raise(&self) {
            if self.inner.closed.load(Ordering::Relaxed) {
                return;
            }
            // SAFETY: live connection and window. A focus refusal (the window not yet viewable)
            // is an X error our handler ignores.
            unsafe {
                XMapRaised(self.inner.display, self.inner.window);
                XSetInputFocus(self.inner.display, self.inner.window, REVERT_TO_PARENT, CURRENT_TIME);
            }
            self.inner.request_activation();
            // SAFETY: as above.
            unsafe { XFlush(self.inner.display) };
        }

        /// Drains this window's pending events: a window-manager close (`WM_DELETE_WINDOW`) unmaps
        /// it and marks it closed, which the editor link turns into a teardown of the GUI.
        pub fn poll_events(&self) {
            if self.inner.closed.load(Ordering::Relaxed) {
                return;
            }
            let d = self.inner.display;
            // Bounded: a flood of events is finished on the next frame.
            for _ in 0..1024 {
                // SAFETY: live connection.
                if unsafe { XPending(d) } <= 0 {
                    break;
                }
                let mut ev = XEventBuf([0u8; 256]);
                // SAFETY: live connection; `ev` has room for any event.
                unsafe { XNextEvent(d, &mut ev) };
                // SAFETY: every event starts with the `XAnyEvent` members; `ev` is 8-aligned.
                let kind = unsafe { (*(&ev as *const XEventBuf).cast::<XAnyEvent>()).type_ };
                match kind {
                    CLIENT_MESSAGE => {
                        // SAFETY: a `ClientMessage` event has the `XClientMessageEvent` layout.
                        let cm = unsafe { &*(&ev as *const XEventBuf).cast::<XClientMessageEvent>() };
                        if cm.message_type == self.inner.protocols_atom && cm.data[0] as c_ulong == self.inner.delete_atom {
                            self.inner.closed.store(true, Ordering::Relaxed);
                            // SAFETY: live connection and window.
                            unsafe { XUnmapWindow(d, self.inner.window) };
                        }
                    }
                    DESTROY_NOTIFY => self.inner.closed.store(true, Ordering::Relaxed),
                    _ => {}
                }
            }
            // SAFETY: live connection.
            unsafe { XFlush(d) };
        }

        pub fn is_visible(&self) -> bool {
            !self.inner.closed.load(Ordering::Relaxed)
        }

        pub fn close(self) {
            let d = self.inner.display;
            // SAFETY: our own window and connection, released exactly once (`self` is consumed);
            // the plugin's GUI was destroyed first.
            unsafe {
                if self.inner.window != 0 {
                    XDestroyWindow(d, self.inner.window);
                }
                XCloseDisplay(d);
            }
            ours().lock().unwrap_or_else(PoisonError::into_inner).retain(|&p| p != d as usize);
        }

        /// What a window manager sends when the user clicks the close button (tests only).
        #[cfg(test)]
        pub fn send_wm_close(&self) {
            let mut buf = XEventBuf([0u8; 256]);
            // SAFETY: `buf` is 8-aligned and larger than an `XClientMessageEvent`.
            let cm = unsafe { &mut *(&mut buf as *mut XEventBuf).cast::<XClientMessageEvent>() };
            cm.type_ = CLIENT_MESSAGE;
            cm.send_event = 1;
            cm.display = self.inner.display;
            cm.window = self.inner.window;
            cm.message_type = self.inner.protocols_atom;
            cm.format = 32;
            cm.data = [self.inner.delete_atom as c_long, 0, 0, 0, 0];
            // SAFETY: live connection and window; mask 0 delivers to the window's creator (us).
            unsafe {
                XSendEvent(self.inner.display, self.inner.window, 0, 0, &mut buf);
                XFlush(self.inner.display);
            }
        }
    }

    /// Resizes a host window (`window` from [`HostWindow::raw`]) and its fixed-size hints.
    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        // SAFETY: null or the `WindowInner` of a live `HostWindow` (callers pass `raw()` of a
        // window they hold).
        let Some(inner) = (unsafe { window.cast_const().cast::<WindowInner>().as_ref() }) else { return };
        if inner.closed.load(Ordering::Relaxed) {
            return;
        }
        let (width, height) = (dim(w), dim(h));
        inner.set_fixed_size(width, height);
        // SAFETY: live connection and window.
        unsafe {
            XResizeWindow(inner.display, inner.window, width, height);
            XFlush(inner.display);
        }
    }

    /// A format-32 property as C longs (empty when absent or unreadable).
    fn get_longs(display: *mut Display, window: c_ulong, property: c_ulong, req_type: c_ulong) -> Vec<c_ulong> {
        if property == 0 {
            return Vec::new();
        }
        let (mut actual_type, mut format, mut n, mut after) = (0 as c_ulong, 0 as c_int, 0 as c_ulong, 0 as c_ulong);
        let mut data: *mut c_uchar = std::ptr::null_mut();
        // SAFETY: live connection; every out-pointer is a valid local; Xlib allocates `data`.
        let status = unsafe {
            XGetWindowProperty(display, window, property, 0, MAX_CLIENTS as c_long, 0, req_type, &mut actual_type, &mut format, &mut n, &mut after, &mut data)
        };
        if status != 0 || data.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if format == 32 && actual_type == req_type {
            let count = usize::try_from(n).unwrap_or(0).min(MAX_CLIENTS);
            // SAFETY: format-32 property data is an array of `n` C longs (Xlib manual); we read at
            // most `n` of them.
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(data.cast::<c_ulong>(), count) });
        }
        // SAFETY: `data` was allocated by Xlib for this call.
        unsafe { XFree(data.cast()) };
        out
    }

    /// The root window's direct children (fallback without an EWMH client list).
    fn root_children(display: *mut Display, root: c_ulong) -> Vec<c_ulong> {
        let (mut r, mut p) = (0 as c_ulong, 0 as c_ulong);
        let mut children: *mut c_ulong = std::ptr::null_mut();
        let mut count: c_uint = 0;
        // SAFETY: live connection; out-pointers are valid locals; Xlib allocates `children`.
        if unsafe { XQueryTree(display, root, &mut r, &mut p, &mut children, &mut count) } == 0 || children.is_null() {
            return Vec::new();
        }
        let n = usize::try_from(count).unwrap_or(0).min(MAX_CLIENTS);
        // SAFETY: `children` holds `count` window ids; we read at most that many.
        let out = unsafe { std::slice::from_raw_parts(children, n) }.to_vec();
        // SAFETY: allocated by Xlib for this call.
        unsafe { XFree(children.cast()) };
        out
    }

    /// `WM_CLASS` instance name of our (and the VST3 host's) editor windows.
    const EDITOR_NAME: &str = "soundcraft-plugin";

    /// Whether a window's `WM_CLASS` names the SoundCraft app (not one of its editor windows).
    fn is_soundcraft(display: *mut Display, window: c_ulong) -> bool {
        let mut hint = XClassHint { res_name: std::ptr::null_mut(), res_class: std::ptr::null_mut() };
        // SAFETY: live connection; `hint` is a valid out-struct; a vanished window is an ignored
        // X error and a zero status.
        if unsafe { XGetClassHint(display, window, &mut hint) } == 0 {
            return false;
        }
        let mut parts = [String::new(), String::new()];
        for (s, out) in [hint.res_name, hint.res_class].into_iter().zip(parts.iter_mut()) {
            if !s.is_null() {
                // SAFETY: Xlib returns NUL-terminated strings, freed right after.
                *out = unsafe { CStr::from_ptr(s) }.to_string_lossy().to_ascii_lowercase();
                // SAFETY: allocated by Xlib for `XGetClassHint`.
                unsafe { XFree(s.cast()) };
            }
        }
        let [name, class] = parts;
        name != EDITOR_NAME && (class.contains("soundcraft") || name.contains("soundcraft"))
    }

    /// The application's own top-level window (`WM_CLASS` containing "soundcraft", preferring
    /// this process's `_NET_WM_PID`), the transient owner of editor windows. Looked up in the
    /// EWMH `_NET_CLIENT_LIST` (client windows, not window-manager frames), else the root's children.
    fn find_app_window(display: *mut Display) -> Option<c_ulong> {
        // SAFETY: live connection.
        let root = unsafe { XDefaultRootWindow(display) };
        // SAFETY: as above; the name is NUL-terminated.
        let atom = |name: &CStr| unsafe { XInternAtom(display, name.as_ptr(), 0) };
        let mut candidates = get_longs(display, root, atom(c"_NET_CLIENT_LIST"), XA_WINDOW);
        if candidates.is_empty() {
            candidates = root_children(display, root);
        }
        let apps: Vec<c_ulong> = candidates.into_iter().filter(|&w| w != 0 && is_soundcraft(display, w)).collect();
        let pid_atom = atom(c"_NET_WM_PID");
        let pid = c_ulong::from(std::process::id());
        apps.iter().copied().find(|&w| get_longs(display, w, pid_atom, XA_CARDINAL).first() == Some(&pid)).or_else(|| apps.first().copied())
    }
}

/// Other platforms: embedded editors are not hosted (floating ones still are).
#[cfg(not(any(windows, target_os = "linux")))]
mod hostwin {
    use std::ffi::c_void;

    pub const SUPPORTED: bool = false;

    pub struct HostWindow;

    impl HostWindow {
        pub fn new(_title: &str, _w: f64, _h: f64) -> Result<HostWindow, String> {
            Err("embedded plugin editors are not hosted on this platform".into())
        }
        pub fn raw(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn parent(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn handle(&self) -> super::clap_window_handle {
            super::clap_window_handle { ptr: std::ptr::null_mut() }
        }
        pub fn raise(&self) {}
        pub fn poll_events(&self) {}
        pub fn is_visible(&self) -> bool {
            false
        }
        pub fn close(self) {}
    }

    pub fn resize(_window: *mut c_void, _w: f64, _h: f64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transport_reaches_clap_in_fixed_point() {
        let t = Transport {
            playing: true,
            tempo: 90.0,
            numerator: 3,
            denominator: 4,
            samples: 96_000,
            seconds: 2.0,
            quarters: 3.5,
            bar_start: 3.0,
            bar: 1,
            ..Transport::default()
        };
        let e = transport_event(&t);
        assert_eq!((e.header.type_, e.header.size as usize), (CLAP_EVENT_TRANSPORT, size_of::<clap_event_transport>()));
        assert_eq!(e.flags & CLAP_TRANSPORT_IS_PLAYING, CLAP_TRANSPORT_IS_PLAYING);
        assert_eq!(e.flags & CLAP_TRANSPORT_IS_RECORDING, 0);
        assert_eq!((e.song_pos_beats, e.bar_start), (7 * CLAP_BEATTIME_FACTOR / 2, 3 * CLAP_BEATTIME_FACTOR));
        assert_eq!(e.song_pos_seconds, 2 * CLAP_SECTIME_FACTOR);
        assert_eq!((e.tempo, e.bar_number, e.tsig_num, e.tsig_denom), (90.0, 1, 3, 4));
        let nan = transport_event(&Transport { quarters: f64::NAN, ..t });
        assert_eq!(nan.song_pos_beats, 0);
    }

    #[test]
    fn timers_register_cap_unregister_and_fall_due() {
        let t0 = Instant::now();
        let mut r = TimerRegistry::default();
        let a = r.register(10, t0).expect("first timer");
        let b = r.register(0, t0).expect("a zero period is raised to 1 ms");
        assert_ne!(a, b);
        assert!(r.contains(a) && r.contains(b));
        assert!(r.due(t0).is_empty(), "nothing is due before one period");
        assert_eq!(r.due(t0 + Duration::from_millis(1)), vec![b]);
        let mut due = r.due(t0 + Duration::from_millis(10));
        due.sort_unstable();
        let mut both = vec![a, b];
        both.sort_unstable();
        assert_eq!(due, both, "each due timer fires once per pump");
        // Missed ticks are not replayed: one call after a long stall fires each timer once.
        assert_eq!(r.due(t0 + Duration::from_secs(60)).len(), 2);
        assert!(r.due(t0 + Duration::from_secs(60)).is_empty(), "rescheduled one period later");
        assert!(r.unregister(a));
        assert!(!r.unregister(a), "already gone");
        assert!(!r.contains(a));
        assert!(r.due(t0 + Duration::from_secs(120)) == vec![b]);
        // The cap holds, and ids never collide or use CLAP_INVALID_ID, also across wrap-around.
        let mut r = TimerRegistry { next_id: CLAP_INVALID_ID - 2, ..TimerRegistry::default() };
        let ids: Vec<clap_id> = (0..MAX_TIMERS).filter_map(|_| r.register(u32::MAX, t0)).collect();
        assert_eq!(ids.len(), MAX_TIMERS);
        assert!(!ids.contains(&CLAP_INVALID_ID));
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), MAX_TIMERS);
        assert_eq!(r.register(5, t0), None, "capped");
        assert!(r.unregister(ids[3]));
        assert!(r.register(5, t0).is_some(), "a freed slot is reusable");
        assert!(r.due(t0 + Duration::from_secs(3600)).len() <= MAX_TIMERS, "huge periods are clamped, not overflowing");
        r.clear();
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn fds_register_modify_cap_and_unregister() {
        let mut r = FdRegistry::default();
        assert!(!r.register(-1, CLAP_POSIX_FD_READ), "negative fds are refused");
        assert!(r.register(5, CLAP_POSIX_FD_READ | 0x80), "unknown bits are dropped");
        assert_eq!(r.flags(5), Some(CLAP_POSIX_FD_READ));
        assert!(!r.register(5, CLAP_POSIX_FD_WRITE), "already registered");
        assert!(r.modify(5, CLAP_POSIX_FD_WRITE | CLAP_POSIX_FD_ERROR));
        assert_eq!(r.flags(5), Some(CLAP_POSIX_FD_WRITE | CLAP_POSIX_FD_ERROR));
        assert!(!r.modify(6, CLAP_POSIX_FD_READ), "not registered");
        assert!(r.unregister(5));
        assert!(!r.unregister(5));
        assert_eq!(r.flags(5), None);
        for fd in 0..MAX_FDS as i32 {
            assert!(r.register(fd, CLAP_POSIX_FD_READ));
        }
        assert!(!r.register(10_000, CLAP_POSIX_FD_READ), "capped");
        assert_eq!(r.snapshot().len(), MAX_FDS);
        r.clear();
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn poll_results_map_to_the_registered_clap_flags() {
        use pollbits::*;
        let rw = CLAP_POSIX_FD_READ | CLAP_POSIX_FD_WRITE;
        assert_eq!(events_for(rw), POLLIN | POLLPRI | POLLOUT);
        assert_eq!(events_for(CLAP_POSIX_FD_ERROR), 0, "errors are always reported by poll");
        assert_eq!(ready_flags(rw, POLLIN), CLAP_POSIX_FD_READ);
        assert_eq!(ready_flags(rw, POLLHUP), CLAP_POSIX_FD_READ, "hang-up reads as end of stream");
        assert_eq!(ready_flags(CLAP_POSIX_FD_READ, POLLOUT), 0, "write readiness nobody asked for");
        assert_eq!(ready_flags(FD_FLAGS, POLLERR | POLLOUT), CLAP_POSIX_FD_WRITE | CLAP_POSIX_FD_ERROR);
        assert_eq!(ready_flags(rw, POLLNVAL), 0);
    }

    /// A real pipe through the zero-timeout `poll`: readable only once written to.
    #[cfg(unix)]
    #[test]
    fn registered_fds_are_polled_without_blocking() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        let (rx, mut tx) = std::io::pipe().expect("pipe");
        let fds = [(rx.as_raw_fd(), CLAP_POSIX_FD_READ)];
        assert!(poll_fds(&fds).is_empty(), "nothing to read yet");
        tx.write_all(b"x").expect("write");
        assert_eq!(poll_fds(&fds), vec![(rx.as_raw_fd(), CLAP_POSIX_FD_READ)]);
        assert!(poll_fds(&[]).is_empty());
    }

    /// The host extensions edit the instance's own registries through its `clap_host`.
    #[test]
    fn timer_and_fd_extensions_reach_the_instance_state() {
        let state = Arc::new(HostState::default());
        let host = clap_host {
            clap_version: CLAP_VERSION,
            host_data: Arc::as_ptr(&state).cast_mut().cast::<c_void>(),
            name: c"t".as_ptr(),
            vendor: c"t".as_ptr(),
            url: c"t".as_ptr(),
            version: c"t".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        };
        // SAFETY: test calls through our own callbacks with a valid host and valid pointers.
        unsafe {
            let timers = host_get_extension(&host, CLAP_EXT_TIMER_SUPPORT.as_ptr()).cast::<clap_host_timer_support>();
            let fds = host_get_extension(&host, CLAP_EXT_POSIX_FD_SUPPORT.as_ptr()).cast::<clap_host_posix_fd_support>();
            if !SERVES_RUN_LOOP {
                assert!(timers.is_null() && fds.is_null());
                return;
            }
            let (Some(t), Some(f)) = (timers.as_ref(), fds.as_ref()) else { panic!("extensions served") };
            let mut id = CLAP_INVALID_ID;
            assert!((t.register_timer.expect("register"))(&host, 16, &mut id));
            assert_ne!(id, CLAP_INVALID_ID);
            assert!(!(t.register_timer.expect("register"))(&host, 16, std::ptr::null_mut()), "null out-pointer");
            // A failed registration writes CLAP_INVALID_ID to the out-pointer: use another one.
            let mut refused = 7;
            assert!(!(t.register_timer.expect("register"))(std::ptr::null(), 16, &mut refused), "null host");
            assert_eq!(refused, CLAP_INVALID_ID);
            assert_eq!(state.timers.lock().expect("lock").len(), 1);
            assert!((t.unregister_timer.expect("unregister"))(&host, id));
            assert!(!(t.unregister_timer.expect("unregister"))(&host, id), "already gone");
            assert!((f.register_fd.expect("register"))(&host, 3, CLAP_POSIX_FD_READ));
            assert!((f.modify_fd.expect("modify"))(&host, 3, CLAP_POSIX_FD_WRITE));
            assert_eq!(state.fds.lock().expect("lock").flags(3), Some(CLAP_POSIX_FD_WRITE));
            assert!((f.unregister_fd.expect("unregister"))(&host, 3));
            assert!(!(f.unregister_fd.expect("unregister"))(&host, 3));
        }
    }

    /// The Linux editor window is a real X11 window: it opens, resizes, turns a window-manager
    /// close into "hidden", and closes. Skipped without a usable display (headless CI), where
    /// opening must fail cleanly instead.
    #[cfg(target_os = "linux")]
    #[test]
    fn x11_editor_window_opens_resizes_and_closes() {
        if std::env::var_os("DISPLAY").is_none() {
            assert!(hostwin::HostWindow::new("no display", 320.0, 200.0).is_err(), "without DISPLAY opening reports an error");
            return;
        }
        let Ok(win) = hostwin::HostWindow::new("SoundCraft editor test", 320.0, 200.0) else {
            // No usable X server from this process (permissions, remote CI): nothing to assert.
            return;
        };
        assert_ne!(win.parent(), 0, "the window id handed to the plugin is set");
        // SAFETY: reading the union member `handle` wrote.
        assert_eq!(unsafe { win.handle().x11 }, win.parent());
        assert!(win.is_visible());
        hostwin::resize(win.raw(), 400.0, 300.0);
        hostwin::resize(win.raw(), f64::NAN, f64::INFINITY);
        hostwin::resize(std::ptr::null_mut(), 1.0, 1.0);
        win.raise();
        win.poll_events();
        assert!(win.is_visible(), "no close request yet");
        win.send_wm_close();
        for _ in 0..50 {
            win.poll_events();
            if !win.is_visible() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!win.is_visible(), "the window manager's close hides the window");
        hostwin::resize(win.raw(), 400.0, 300.0);
        win.raise();
        win.close();
    }
}
