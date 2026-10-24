//! The FFI boundary: the only module of this crate allowed to use `unsafe`.
//!
//! It loads VST3 binaries (`libloading`), runs the platform module entry, calls the COM
//! interfaces of the factory, component, audio processor and edit controller (`vst3` raw
//! bindings), and implements the host-side COM objects (host context, parameter changes, event
//! lists, a memory stream). Everything it exposes to the rest of the crate is a safe API.
//!
//! Soundness rests on the VST3 contract (the plugin honours the COM vtable signatures and the
//! documented pointer lifetimes). A plugin that violates it can still crash the process: hosting
//! in-process can't prevent that. What this module guarantees is that *SoundCraft* never hands
//! the plugin dangling or mis-sized pointers and never reads plugin strings beyond their bounds.

// VST3 enum constants are `c_int` on Windows and `c_uint` elsewhere, so casts that are no-ops on
// one platform are needed on another.
#![allow(clippy::unnecessary_cast)]

use crate::{Vst3Error, scan};
use soundcraft_dsp::Transport;
use std::ffi::{c_char, c_int, c_void};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, TryLockError, mpsc};
use std::time::Duration;
use vst3::Steinberg::Linux::{IEventHandlerTrait, IRunLoopTrait, ITimerHandlerTrait};
use vst3::Steinberg::Vst::{self as sv, IUnitInfoTrait};
use vst3::Steinberg::Vst::{
    IAudioProcessorTrait, IComponentHandlerTrait, IComponentTrait, IConnectionPointTrait, IEditControllerTrait, IEventListTrait,
    IHostApplicationTrait, IMidiMappingTrait, IParamValueQueueTrait, IParameterChangesTrait,
};
use vst3::Steinberg::{self as sb};
use vst3::Steinberg::{FUnknown, IBStreamTrait, IPlugFrameTrait, IPlugViewTrait, IPluginBaseTrait, IPluginFactory2Trait, IPluginFactoryTrait};
use vst3::{Class, ComPtr, ComWrapper, Interface};

/// Most classes one factory may report (hostile-input cap).
const MAX_CLASSES: i32 = 1024;
/// Most parameters read from one controller.
pub(crate) const MAX_PARAMS: i32 = 4096;
/// Most audio buses per direction we provide buffers for.
const MAX_BUSES: i32 = 16;
/// Most channels per audio bus.
const MAX_BUS_CHANNELS: u32 = 64;
/// Capacity of the per-block event lists (preallocated: `process` never allocates).
pub(crate) const EVENT_CAP: usize = 1024;
/// Most parameter queues per block (preallocated).
pub(crate) const PARAM_QUEUE_CAP: usize = 1024;
/// Largest plugin state we buffer (component or controller state).
const MAX_STATE_BYTES: usize = 64 << 20;
/// Most editor parameter edits buffered between two UI frames.
const EDIT_CAP: usize = 256;
/// Magic of SoundCraft's state container: `SCV3` + u32 LE component length + component state +
/// u32 LE controller length + controller state.
const STATE_MAGIC: &[u8; 4] = b"SCV3";

// ---- strings -------------------------------------------------------------------------------

/// Reads a fixed-size `char8[N]` buffer, stopping at the first NUL or the end of the array.
fn read_c8(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// Reads a fixed-size UTF-16 `String128`, stopping at the first NUL or the end of the array.
fn read_c16(buf: &[u16]) -> String {
    let units: Vec<u16> = buf.iter().take_while(|c| **c != 0).copied().collect();
    String::from_utf16_lossy(&units).trim().to_string()
}

/// Writes `s` into a UTF-16 buffer, truncated and always NUL-terminated.
fn write_c16(s: &str, dst: &mut [u16]) {
    let Some(room) = dst.len().checked_sub(1) else { return };
    let mut n = 0;
    for (d, u) in dst.iter_mut().zip(s.encode_utf16().take(room)) {
        *d = u;
        n += 1;
    }
    if let Some(z) = dst.get_mut(n) {
        *z = 0;
    }
}

fn ok(r: sb::tresult) -> bool {
    r == sb::kResultOk
}

/// VST3's numbers for the MIDI controllers that aren't CCs.
pub(crate) const AFTERTOUCH: usize = sv::ControllerNumbers_::kAfterTouch as usize;
pub(crate) const PITCH_BEND: usize = sv::ControllerNumbers_::kPitchBend as usize;
pub(crate) const PROGRAM_CHANGE: usize = sv::ControllerNumbers_::kCtrlProgramChange as usize;

// ---- loaded binaries -----------------------------------------------------------------------

/// One class as reported by the factory.
#[derive(Debug, Clone)]
pub(crate) struct RawClass {
    pub cid: [u8; 16],
    pub category: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sdk_version: String,
    pub sub_categories: String,
}

/// A loaded VST3 binary and its plugin factory. Loaded binaries are cached for the life of the
/// process (see [`Bundle::load`]): plugin code must never be unmapped while an instance may run,
/// and VST3 modules are not required to support being re-entered after their exit function, which
/// therefore runs once, at process exit (see [`exit_modules`]).
pub(crate) struct Bundle {
    factory: ComPtr<sb::IPluginFactory>,
    /// The module exit matching the entry that ran at load (`bundleExit`, `ExitDll`, `ModuleExit`).
    exit: Option<ModuleExit>,
    /// The thread that ran the module entry, and so must run the exit.
    entered_on: EntryThread,
    /// Set when plugin objects were leaked on purpose (see [`EditorLink::kill`]): the module exit
    /// must not run while any of its objects are alive.
    leaked: AtomicBool,
    // Never closed: code stays mapped for the process lifetime, even after the module exit.
    _lib: ManuallyDrop<libloading::Library>,
}

/// Loaded binaries, oldest first (module exits run newest first, the reverse of loading).
fn loaded() -> &'static Mutex<Vec<(PathBuf, Arc<Bundle>)>> {
    static L: OnceLock<Mutex<Vec<(PathBuf, Arc<Bundle>)>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Vec::new()))
}

/// Set once the module exits have run: loading another module after that is refused.
static MODULES_EXITED: AtomicBool = AtomicBool::new(false);

/// Where a module's entry ran. Its exit must run on the same thread: Qt-based modules tie their
/// application object to it (Native Instruments' Massive X crashes in `bundleExit` on another
/// thread). Modules wanted on the main thread are entered there; any other thread's requests go
/// to the module thread, because the asking thread may be gone by the time the process exits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryThread {
    Main,
    Modules,
}

/// Whether this is the main thread. Not for exit handlers, where `std::thread::current` panics
/// (its thread-local data is gone by then): compare [`os_thread`] with [`MAIN_OS_THREAD`] there.
fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

/// The OS id of the main thread, recorded when it loads a module (0 until then).
static MAIN_OS_THREAD: AtomicUsize = AtomicUsize::new(0);

/// The OS id of the calling thread, which (unlike `std::thread::current`) exit handlers can use.
fn os_thread() -> usize {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn pthread_self() -> usize;
        }
        // SAFETY: `pthread_self` takes no arguments and cannot fail; `pthread_t` is an integer or
        // a pointer of pointer size on the Unix systems SoundCraft builds for.
        unsafe { pthread_self() }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        // SAFETY: `GetCurrentThreadId` takes no arguments and cannot fail.
        unsafe { GetCurrentThreadId() as usize }
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// The module thread (started on first use): it runs jobs until the process exits.
fn module_thread() -> Option<&'static mpsc::Sender<Job>> {
    static TX: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("vst3-modules".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            }
        });
        spawned.ok().map(|_| tx)
    })
    .as_ref()
}

/// Runs `f` on the module thread and waits for its result (at most `timeout`, when given).
fn on_module_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static, timeout: Option<Duration>) -> Option<R> {
    let (rtx, rrx) = mpsc::sync_channel(1);
    module_thread()?
        .send(Box::new(move || {
            let _ = rtx.send(f());
        }))
        .ok()?;
    match timeout {
        Some(t) => rrx.recv_timeout(t).ok(),
        None => rrx.recv().ok(),
    }
}

type GetFactory = unsafe extern "system" fn() -> *mut sb::IPluginFactory;
/// `bundleExit` (macOS), `ExitDll` (Windows) or `ModuleExit` (Linux/BSD). `extern "system"` is the
/// C ABI everywhere except 32-bit Windows, where it is the `PLUGIN_API` (stdcall) `ExitDll` uses.
type ModuleExit = unsafe extern "system" fn() -> bool;

#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buf: *const u8, len: isize, is_dir: u8) -> *const c_void;
        pub fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *mut c_void;
        pub fn CFRelease(cf: *const c_void);
    }
}

/// Opens the shared library and runs the platform module entry (`bundleEntry` with a
/// `CFBundleRef` on macOS, `InitDll` on Windows, `ModuleEntry` with the `dlopen` handle on Linux).
/// Also returns the matching module exit, when the entry ran and the module exports one.
fn open_library(bundle: &Path, exe: &Path) -> Result<(libloading::Library, Option<ModuleExit>), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use libloading::os::unix::{Library as UnixLib, RTLD_LOCAL, RTLD_NOW};
        // SAFETY: loading a shared library runs its static initialisers. That is inherent to
        // hosting plugins; we only load files found in VST3 plugin folders or named explicitly.
        let lib = unsafe { UnixLib::open(Some(exe), RTLD_NOW | RTLD_LOCAL) }.map_err(|e| e.to_string())?;
        type ModuleEntry = unsafe extern "C" fn(*mut c_void) -> bool;
        // SAFETY: `ModuleEntry` is the VST3-mandated Linux entry with this signature.
        let entry = unsafe { lib.get::<ModuleEntry>(b"ModuleEntry\0") }.ok().map(|s| *s);
        let raw = lib.into_raw();
        // SAFETY: `raw` is the handle we just took out of `lib`; it is rewrapped right away.
        let lib = unsafe { UnixLib::from_raw(raw) };
        let mut exit = None;
        if let Some(f) = entry {
            // SAFETY: called once, with the module's own `dlopen` handle, as the spec requires.
            if !unsafe { f(raw) } {
                return Err("ModuleEntry failed".into());
            }
            // SAFETY: `ModuleExit` is the VST3-mandated Linux exit with this signature; the
            // pointer stays valid because loaded modules are never unmapped.
            exit = unsafe { lib.get::<ModuleExit>(b"ModuleExit\0") }.ok().map(|s| *s);
        }
        let _ = bundle;
        Ok((lib.into(), exit))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        // SAFETY: loading a shared library runs its static initialisers. That is inherent to
        // hosting plugins; we only load files found in VST3 plugin folders or named explicitly.
        let lib = unsafe { libloading::Library::new(exe) }.map_err(|e| e.to_string())?;
        let mut exit: Option<ModuleExit> = None;
        #[cfg(windows)]
        {
            let _ = bundle;
            type InitDll = unsafe extern "system" fn() -> bool;
            // SAFETY: `InitDll` is the VST3-mandated Windows entry with this signature.
            if let Ok(f) = unsafe { lib.get::<InitDll>(b"InitDll\0") } {
                // SAFETY: called once after loading, as the spec requires.
                if !unsafe { f() } {
                    return Err("InitDll failed".into());
                }
                // SAFETY: `ExitDll` is the VST3-mandated Windows exit with this signature; the
                // pointer stays valid because loaded modules are never unmapped.
                exit = unsafe { lib.get::<ModuleExit>(b"ExitDll\0") }.ok().map(|s| *s);
            }
        }
        #[cfg(target_os = "macos")]
        {
            type BundleEntry = unsafe extern "C" fn(*mut c_void) -> bool;
            // SAFETY: `bundleEntry`/`BundleEntry` is the VST3-mandated macOS entry with this signature.
            let entry = unsafe { lib.get::<BundleEntry>(b"bundleEntry\0") }
                .or_else(|_| {
                    // SAFETY: as above (older SDKs export the capitalised name).
                    unsafe { lib.get::<BundleEntry>(b"BundleEntry\0") }
                })
                .ok()
                .map(|s| *s);
            if let Some(f) = entry
                && bundle.is_dir()
            {
                use std::os::unix::ffi::OsStrExt;
                let bytes = bundle.as_os_str().as_bytes();
                let len = isize::try_from(bytes.len()).map_err(|_| "path too long".to_string())?;
                // SAFETY: `bytes` is a valid buffer of `len` bytes for the duration of the call;
                // a null allocator means the default one.
                let url = unsafe { cf::CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), len, 1) };
                if url.is_null() {
                    return Err("cannot make a CFURL for the bundle".into());
                }
                // SAFETY: `url` is a valid CFURL created above.
                let cfbundle = unsafe { cf::CFBundleCreate(std::ptr::null(), url) };
                // SAFETY: we own `url` (Create rule) and no longer need it.
                unsafe { cf::CFRelease(url) };
                if cfbundle.is_null() {
                    return Err("not a loadable bundle".into());
                }
                // SAFETY: called once with a valid CFBundleRef. The bundle ref is intentionally
                // kept (never released): the module may hold on to it for its whole life, and
                // loaded modules are never unloaded.
                if !unsafe { f(cfbundle) } {
                    return Err("bundleEntry failed".into());
                }
                // SAFETY: `bundleExit`/`BundleExit` is the VST3-mandated macOS exit with this
                // signature; the pointer stays valid because loaded modules are never unmapped.
                exit = unsafe { lib.get::<ModuleExit>(b"bundleExit\0") }
                    .or_else(|_| {
                        // SAFETY: as above (older SDKs export the capitalised name).
                        unsafe { lib.get::<ModuleExit>(b"BundleExit\0") }
                    })
                    .ok()
                    .map(|s| *s);
            }
        }
        Ok((lib, exit))
    }
}

/// Runs the module exits at process exit, newest module first, each on the thread that ran its
/// entry. VST3 modules expect their exit before their static destructors run, and some crash in
/// those destructors without it (seen with Steinberg's HALion Sonic, after a plain scan). A module
/// with a live instance or leaked objects keeps running and is not exited, and so is one entered
/// on the main thread when the process exits from another thread. Runs once; later loads are
/// refused.
pub(crate) fn exit_modules() {
    if MODULES_EXITED.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut cache = match loaded().try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        // Another thread is loading a module right now: leave every module as it is.
        Err(TryLockError::WouldBlock) => return,
    };
    let main = MAIN_OS_THREAD.load(Ordering::Acquire) == os_thread();
    let (mut kept, mut here, mut there) = (Vec::new(), Vec::new(), Vec::new());
    while let Some((path, b)) = cache.pop() {
        if b.leaked.load(Ordering::Acquire) || (b.entered_on == EntryThread::Main && !main) {
            kept.push((path, b));
            continue;
        }
        match Arc::try_unwrap(b) {
            Ok(bundle) if bundle.entered_on == EntryThread::Main => here.push(bundle),
            Ok(bundle) => there.push(bundle),
            Err(b) => kept.push((path, b)),
        }
    }
    kept.reverse();
    *cache = kept;
    drop(cache);
    here.into_iter().for_each(Bundle::exit);
    if !there.is_empty() {
        // Bounded: a module thread stuck inside a plugin must not hang the process exit.
        let _ = on_module_thread(move || there.into_iter().for_each(Bundle::exit), Some(Duration::from_secs(5)));
    }
}

/// Registers [`exit_modules`] to run at process exit. Exit handlers run in reverse order of
/// registration, C++ static destructors included, so registering again after each module load
/// puts the exits ahead of the destructors of every module loaded so far.
fn register_exit_hook() {
    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> c_int;
    }
    extern "C" fn hook() {
        // Nothing may unwind out of an exit handler.
        let _ = std::panic::catch_unwind(exit_modules);
    }
    // SAFETY: `atexit` is the C library function with this signature; `hook` is a plain function
    // of this binary, so it lives as long as the process.
    if unsafe { atexit(hook) } != 0 {
        log::warn!("vst3: cannot register the module exit handler");
    }
}

impl Bundle {
    /// Loads (once per path, then cached for the life of the process) a `.vst3`.
    pub fn load(path: &Path) -> Result<Arc<Bundle>, Vst3Error> {
        ensure_wine_dll_dirs_initialized();
        let mut cache = loaded().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, b)) = cache.iter().find(|(p, _)| p == path) {
            return Ok(b.clone());
        }
        let err = |m: &str| Vst3Error::Load(path.display().to_string(), m.to_string());
        if MODULES_EXITED.load(Ordering::Acquire) {
            return Err(err("plugin hosting has shut down"));
        }
        let b = if on_main_thread() {
            MAIN_OS_THREAD.store(os_thread(), Ordering::Release);
            Bundle::open(path, EntryThread::Main)?
        } else {
            let p = path.to_path_buf();
            on_module_thread(move || Bundle::open(&p, EntryThread::Modules), None).ok_or_else(|| err("the module thread is not running"))??
        };
        let b = Arc::new(b);
        cache.push((path.to_path_buf(), b.clone()));
        // After every new module, so that the exits run before its static destructors.
        register_exit_hook();
        Ok(b)
    }

    fn open(path: &Path, entered_on: EntryThread) -> Result<Bundle, Vst3Error> {
        let err = |m: String| Vst3Error::Load(path.display().to_string(), m);
        let exe = scan::bundle_binary(path).ok_or_else(|| err("no loadable binary".into()))?;
        let (lib, exit) = open_library(path, &exe).map_err(err)?;
        // The module entry ran: balance it before giving up on the module.
        let fail = |m: String| {
            if let Some(f) = exit {
                // SAFETY: balanced with the entry that ran in `open_library`; no plugin object exists.
                unsafe { f() };
            }
            err(m)
        };
        // SAFETY: `GetPluginFactory` is the VST3-mandated export with this signature.
        let get = match unsafe { lib.get::<GetFactory>(b"GetPluginFactory\0") } {
            Ok(sym) => *sym,
            Err(e) => return Err(fail(format!("not a VST3 plugin: {e}"))),
        };
        // SAFETY: the module is loaded and its entry function (if any) has run.
        let raw = unsafe { get() };
        // SAFETY: a non-null result is a factory pointer with one reference owned by the caller
        // (VST3 contract), which the `ComPtr` takes over.
        let factory = unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| fail("GetPluginFactory returned null".into()))?;
        Ok(Bundle { factory, exit, entered_on, leaked: AtomicBool::new(false), _lib: ManuallyDrop::new(lib) })
    }

    /// Releases the factory, then runs the module exit, as the VST3 SDK's own host does.
    fn exit(self) {
        let Bundle { factory, exit, .. } = self;
        drop(factory);
        if let Some(f) = exit {
            // SAFETY: balanced with the entry that ran when the module was loaded; called once
            // (`self` is consumed), after every instance is gone (each holds an `Arc` to the
            // bundle) and the factory released. The code stays mapped (`_lib` is never closed).
            unsafe { f() };
        }
    }

    /// Plugin objects of this module were leaked on purpose: never run its module exit.
    pub fn mark_leaked(&self) {
        self.leaked.store(true, Ordering::Release);
    }

    /// The vendor from the factory info (fallback for classes without one).
    fn factory_vendor(&self) -> String {
        // SAFETY: `PFactoryInfo` is plain data (char arrays and an int); all-zero is valid.
        let mut info: sb::PFactoryInfo = unsafe { std::mem::zeroed() };
        // SAFETY: valid factory; `info` is a writable struct of the right type.
        if ok(unsafe { self.factory.getFactoryInfo(&mut info) }) { read_c8(&info.vendor) } else { String::new() }
    }

    /// Every class this binary exports.
    pub fn classes(&self) -> Vec<RawClass> {
        // SAFETY: valid factory.
        let n = unsafe { self.factory.countClasses() }.clamp(0, MAX_CLASSES);
        let f2 = self.factory.cast::<sb::IPluginFactory2>();
        let vendor = self.factory_vendor();
        let mut out = Vec::new();
        for i in 0..n {
            if let Some(f2) = &f2 {
                // SAFETY: plain data; all-zero is valid.
                let mut c: sb::PClassInfo2 = unsafe { std::mem::zeroed() };
                // SAFETY: index below the count; `c` is writable.
                if ok(unsafe { f2.getClassInfo2(i, &mut c) }) {
                    let v = read_c8(&c.vendor);
                    out.push(RawClass {
                        cid: tuid_bytes(&c.cid),
                        category: read_c8(&c.category),
                        name: read_c8(&c.name),
                        vendor: if v.is_empty() { vendor.clone() } else { v },
                        version: read_c8(&c.version),
                        sdk_version: read_c8(&c.sdkVersion),
                        sub_categories: read_c8(&c.subCategories),
                    });
                    continue;
                }
            }
            // SAFETY: plain data; all-zero is valid.
            let mut c: sb::PClassInfo = unsafe { std::mem::zeroed() };
            // SAFETY: index below the count; `c` is writable.
            if ok(unsafe { self.factory.getClassInfo(i, &mut c) }) {
                out.push(RawClass {
                    cid: tuid_bytes(&c.cid),
                    category: read_c8(&c.category),
                    name: read_c8(&c.name),
                    vendor: vendor.clone(),
                    version: String::new(),
                    sdk_version: String::new(),
                    sub_categories: String::new(),
                });
            }
        }
        out
    }

    /// Creates an object of class `cid` and queries interface `I` on it.
    fn create<I: Interface>(&self, cid: &[u8; 16]) -> Option<ComPtr<I>> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = I::IID;
        // SAFETY: valid factory; `cid` and `iid` point to 16 readable bytes each (VST3 passes TUIDs
        // as `FIDString`); `obj` is writable.
        let r = unsafe { self.factory.createInstance(cid.as_ptr().cast::<c_char>(), iid.as_ptr().cast::<c_char>(), &mut obj) };
        if !ok(r) {
            return None;
        }
        // SAFETY: on success `obj` holds an `I` pointer with one reference owned by us.
        unsafe { ComPtr::from_raw(obj.cast::<I>()) }
    }

    /// Creates, initialises and wires up one plugin instance.
    pub fn instantiate(self: &Arc<Self>, cid: &[u8; 16]) -> Result<Instance, Vst3Error> {
        let name = hex(cid);
        let err = |m: &str| Vst3Error::Instantiate(name.clone(), m.to_string());
        let host = ComWrapper::new(HostContext::default());
        let host_unknown = host.as_com_ref::<sv::IHostApplication>().ok_or_else(|| err("host context"))?.upcast::<FUnknown>().as_ptr();
        let component = self.create::<sv::IComponent>(cid).ok_or_else(|| err("factory cannot create the component"))?;
        // SAFETY: fresh component; the host context outlives it (owned by the returned Instance).
        if !ok(unsafe { component.initialize(host_unknown) }) {
            return Err(err("component initialize failed"));
        }
        let mut inst = Instance {
            processor: None,
            controller: None,
            single: false,
            connections: None,
            component_initialized: true,
            controller_initialized: false,
            active: false,
            processing: false,
            max_frames: 0,
            in_params: ComWrapper::new(ParamChanges::new(PARAM_QUEUE_CAP)),
            out_params: ComWrapper::new(ParamChanges::new(PARAM_QUEUE_CAP)),
            in_events: ComWrapper::new(EventList::new(EVENT_CAP)),
            out_events: ComWrapper::new(EventList::new(EVENT_CAP)),
            // SAFETY: `ProcessContext` is plain data (numbers and small structs); all-zero is valid.
            context: unsafe { std::mem::zeroed() },
            event_in: false,
            editor: Arc::new(EditorLink::default()),
            component,
            host,
            bundle: self.clone(),
        };
        let processor = inst.component.cast::<sv::IAudioProcessor>().ok_or_else(|| err("no IAudioProcessor"))?;
        inst.processor = Some(processor);
        // Edit controller. The component may implement `IEditController` itself, but when it also
        // names a *separate* controller class that class owns the parameters: some plugins stub
        // `IEditController` on the component (it answers everything with nothing) and keep the real
        // controller beside it. Pianoteq 9 is one, and trusting the component left it with no
        // parameters, no state and silence. So: a distinct controller class wins; the component is
        // the controller only when it is the class the component names (single-component plugin).
        let mut ccid: sb::TUID = [0; 16];
        // SAFETY: valid component; `ccid` is a writable 16-byte TUID.
        let names_controller = ok(unsafe { inst.component.getControllerClassId(&mut ccid) });
        let controller_id = tuid_bytes(&ccid);
        let separate = names_controller && controller_id != *cid;
        log::warn!(
            "vst3 {name}: component-as-controller={}, names_controller={names_controller} as {} (component {}), separate={separate}",
            inst.component.cast::<sv::IEditController>().is_some(),
            hex(&controller_id),
            hex(cid)
        );
        if separate {
            match self.create::<sv::IEditController>(&controller_id) {
                Some(c) => {
                    // SAFETY: fresh controller; the host context outlives it.
                    if ok(unsafe { c.initialize(host_unknown) }) {
                        inst.controller = Some(c);
                        inst.controller_initialized = true;
                    } else {
                        log::warn!("vst3 {name}: controller initialize failed; parameters unavailable");
                    }
                }
                None => log::warn!("vst3 {name}: the module cannot create its edit controller {}; parameters unavailable", hex(&controller_id)),
            }
        }
        if inst.controller.is_none()
            && let Some(c) = inst.component.cast::<sv::IEditController>()
        {
            inst.controller = Some(c);
            inst.single = true;
        }
        if inst.controller.is_none() {
            log::warn!("vst3 {name}: no edit controller; parameters and the editor are unavailable");
        } else {
            let walked = inst.params().len();
            let declared = inst.declared_param_count();
            let first = inst.first_param_info_ok();
            log::warn!("vst3 {name}: controller: declared {declared}, walk {walked}, getParameterInfo(0) ok {first} (single={})", inst.single);
        }
        if let Some(c) = &inst.controller {
            if !inst.single
                && let (Some(a), Some(b)) = (inst.component.cast::<sv::IConnectionPoint>(), c.cast::<sv::IConnectionPoint>())
            {
                // SAFETY: both connection points are valid for as long as `inst` lives; they are
                // disconnected in `Drop` before either side terminates.
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                inst.connections = Some((a, b));
            }
            inst.sync_controller_state();
            if let Some(h) = inst.host.as_com_ref::<sv::IComponentHandler>() {
                // SAFETY: valid controller; the handler (host context) outlives it.
                unsafe { c.setComponentHandler(h.as_ptr()) };
            }
        }
        inst.editor = Arc::new(EditorLink {
            audio_busy: std::sync::atomic::AtomicBool::new(false),
            inner: Mutex::new(EditorInner { component: Some(inst.component.clone()), controller: inst.controller.clone(), ..EditorInner::default() }),
        });
        Ok(inst)
    }
}

/// A `TUID` (`[i8; 16]`) as raw bytes.
fn tuid_bytes(t: &sb::TUID) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (o, b) in out.iter_mut().zip(t.iter()) {
        *o = *b as u8;
    }
    out
}

/// Folders a bridged Windows plugin imports DLLs from: for every `.vst3` in the yabridge mirrors,
/// the folder on the Windows side that the mirror links to. A plugin that ships DLLs beside itself
/// (Pianoteq's engine, many synths) only loads when Wine searches there.
pub fn ensure_wine_dll_dirs_initialized() {
    static DONE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    DONE.get_or_init(|| {
        let dirs = bridged_plugin_dirs();
        if !dirs.is_empty() {
            add_wine_dll_dirs(&dirs);
        }
    });
}

pub fn bridged_plugin_dirs() -> Vec<String> {
    /// Deepest nesting a mirror can have (`yabridgectl add` accepts any folder, and mirrors nest).
    const MAX_DEPTH: usize = 6;
    /// Most folders read per mirror (a hostile tree must not hang start-up).
    const MAX_ENTRIES: usize = 4096;

    let mut out: Vec<String> = Vec::new();
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        roots.push(home.join(".vst3").join("yabridge"));
        roots.push(home.join(".vst").join("yabridge"));
    }
    let mut pending: Vec<(std::path::PathBuf, usize)> = roots.into_iter().map(|r| (r, 0)).collect();
    let mut seen = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            let path = entry.path();
            let is_vst3 = path.extension().and_then(|e| e.to_str()) == Some("vst3");
            if is_vst3 {
                // The Windows side of the mirror: a file (single-file plugin) or a bundle.
                for arch in ["x86_64-win", "x86-win"] {
                    let win = path.join("Contents").join(arch);
                    let Ok(files) = std::fs::read_dir(&win) else { continue };
                    for file in files.flatten() {
                        let real = std::fs::canonicalize(file.path()).unwrap_or_else(|_| file.path());
                        let Some(parent) = real.parent() else { continue };
                        let unix = parent.to_path_buf();
                        for candidate in [wine_path(&unix), unix.to_string_lossy().into_owned()] {
                            if !candidate.is_empty() && !out.contains(&candidate) {
                                out.push(candidate);
                            }
                        }
                    }
                }
            } else if depth < MAX_DEPTH && path.is_dir() {
                pending.push((path, depth + 1));
            }
        }
    }
    out
}

/// Prepends folders to `WINEPATH`, Wine's DLL search path, so a bridged plugin finds the DLLs it
/// imports from its own folder. Call it once, before any thread starts: the environment is
/// process-wide and this changes it.
pub fn add_wine_dll_dirs(dirs: &[String]) {
    if dirs.is_empty() {
        return;
    }
    let mut value = std::env::var_os("WINEPATH").map(|v| v.to_string_lossy().into_owned()).unwrap_or_default();
    let mut changed = false;
    for dir in dirs {
        let dir = dir.clone();
        if dir.is_empty() || value.split(';').any(|part| part == dir) {
            continue;
        }
        if value.is_empty() {
            value = dir;
        } else {
            value = format!("{dir};{value}");
        }
        changed = true;
    }
    if changed {
        // SAFETY: the caller runs this from `main` before it starts any thread (the audio, MIDI,
        // scanner and autosave workers all start later), so no other thread can be reading the
        // environment while it changes; the variable is only ever grown, never removed.
        unsafe { std::env::set_var("WINEPATH", value) };
    }
}

/// A Unix folder as the Windows path Wine expects in `WINEPATH`: `drive_c` is `C:`, anything else
/// is on `Z:`.
fn wine_path(dir: &std::path::Path) -> String {
    let text = dir.to_string_lossy().replace('/', "\\");
    if let Some(home) = std::env::var_os("HOME") {
        let prefix = std::path::PathBuf::from(home).join(".wine").join("drive_c");
        let prefix = prefix.to_string_lossy().replace('/', "\\");
        if let Some(rest) = text.strip_prefix(&format!("{prefix}\\")) {
            return format!("C:\\{rest}");
        }
    }
    format!("Z:{text}")
}

/// Upper-case hex of a class id (the form used in `vst3:<hex>` ids).
pub(crate) fn hex(cid: &[u8; 16]) -> String {
    cid.iter().map(|b| format!("{b:02X}")).collect()
}

// ---- host-side COM objects -----------------------------------------------------------------

/// The host context handed to `initialize` (`IHostApplication`) and to the controller
/// (`IComponentHandler`: parameter edits made in the plugin's editor are queued for the UI, which
/// writes them into the session, from where they reach the processor).
pub(crate) struct HostContext {
    pub latency_changed: AtomicBool,
    pub restart_requested: AtomicBool,
    /// `performEdit` values (normalized), bounded by `EDIT_CAP`.
    pub edits: Mutex<Vec<(u32, f64)>>,
}

impl Default for HostContext {
    fn default() -> Self {
        HostContext {
            latency_changed: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
            edits: Mutex::new(Vec::with_capacity(EDIT_CAP)),
        }
    }
}

impl HostContext {
    /// Takes the queued editor edits.
    pub fn take_edits(&self) -> Vec<(u32, f64)> {
        let mut e = self.edits.lock().unwrap_or_else(PoisonError::into_inner);
        let out = e.clone();
        e.clear();
        out
    }
}

impl Class for HostContext {
    type Interfaces = (sv::IHostApplication, sv::IComponentHandler);
}

impl IHostApplicationTrait for HostContext {
    unsafe fn getName(&self, name: *mut sv::String128) -> sb::tresult {
        // SAFETY: the plugin passes a pointer to a writable String128 (or null, handled).
        match unsafe { name.as_mut() } {
            Some(n) => {
                write_c16("SoundCraft", n);
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }

    unsafe fn createInstance(&self, _cid: *mut sb::TUID, _iid: *mut sb::TUID, obj: *mut *mut c_void) -> sb::tresult {
        // IMessage / IAttributeList are not provided yet; plugins must cope with that.
        // SAFETY: `obj` is the plugin's out-pointer (or null, handled).
        if let Some(o) = unsafe { obj.as_mut() } {
            *o = std::ptr::null_mut();
        }
        sb::kResultFalse
    }
}

impl IComponentHandlerTrait for HostContext {
    unsafe fn beginEdit(&self, _id: sv::ParamID) -> sb::tresult {
        sb::kResultOk
    }
    unsafe fn performEdit(&self, id: sv::ParamID, value: sv::ParamValue) -> sb::tresult {
        if value.is_finite() {
            let mut e = self.edits.lock().unwrap_or_else(PoisonError::into_inner);
            let v = value.clamp(0.0, 1.0);
            if let Some(x) = e.iter_mut().find(|(i, _)| *i == id) {
                x.1 = v;
            } else if e.len() < EDIT_CAP {
                e.push((id, v));
            }
        }
        sb::kResultOk
    }
    unsafe fn endEdit(&self, _id: sv::ParamID) -> sb::tresult {
        sb::kResultOk
    }
    unsafe fn restartComponent(&self, flags: sb::int32) -> sb::tresult {
        if flags as u32 & sv::RestartFlags_::kLatencyChanged as u32 != 0 {
            self.latency_changed.store(true, Ordering::Relaxed);
        }
        self.restart_requested.store(true, Ordering::Relaxed);
        sb::kResultOk
    }
}

/// One parameter's points for one block. SoundCraft sends at most one point per block; a
/// plugin writing output changes may add more, of which only the last is kept.
#[derive(Default)]
pub(crate) struct ParamQueue {
    id: AtomicU32,
    offset: AtomicI32,
    value: AtomicU64,
    points: AtomicI32,
}

impl Class for ParamQueue {
    type Interfaces = (sv::IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> sv::ParamID {
        self.id.load(Ordering::Relaxed)
    }
    unsafe fn getPointCount(&self) -> sb::int32 {
        self.points.load(Ordering::Relaxed)
    }
    unsafe fn getPoint(&self, index: sb::int32, offset: *mut sb::int32, value: *mut sv::ParamValue) -> sb::tresult {
        if index != 0 || self.points.load(Ordering::Relaxed) < 1 {
            return sb::kInvalidArgument;
        }
        // SAFETY: out-pointers from the plugin (or null, handled).
        if let (Some(o), Some(v)) = (unsafe { offset.as_mut() }, unsafe { value.as_mut() }) {
            *o = self.offset.load(Ordering::Relaxed);
            *v = f64::from_bits(self.value.load(Ordering::Relaxed));
            sb::kResultOk
        } else {
            sb::kInvalidArgument
        }
    }
    unsafe fn addPoint(&self, offset: sb::int32, value: sv::ParamValue, index: *mut sb::int32) -> sb::tresult {
        self.offset.store(offset, Ordering::Relaxed);
        self.value.store(value.to_bits(), Ordering::Relaxed);
        self.points.store(1, Ordering::Relaxed);
        // SAFETY: out-pointer from the plugin (or null, handled).
        if let Some(i) = unsafe { index.as_mut() } {
            *i = 0;
        }
        sb::kResultOk
    }
}

/// A fixed pool of parameter queues (`IParameterChanges`), reused every block.
pub(crate) struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    count: AtomicI32,
}

impl Class for ParamChanges {
    type Interfaces = (sv::IParameterChanges,);
}

impl ParamChanges {
    fn new(cap: usize) -> ParamChanges {
        ParamChanges { queues: (0..cap).map(|_| ComWrapper::new(ParamQueue::default())).collect(), count: AtomicI32::new(0) }
    }

    fn clear(&self) {
        self.count.store(0, Ordering::Relaxed);
    }

    /// Finds or adds the queue for `id`; `None` when the pool is full.
    fn queue_for(&self, id: u32) -> Option<(i32, &ParamQueue)> {
        let n = self.count.load(Ordering::Relaxed).max(0);
        for (i, q) in self.queues.iter().enumerate().take(n as usize) {
            if q.id.load(Ordering::Relaxed) == id {
                return Some((i32::try_from(i).ok()?, q));
            }
        }
        let q = self.queues.get(n as usize)?;
        q.id.store(id, Ordering::Relaxed);
        q.points.store(0, Ordering::Relaxed);
        self.count.store(n + 1, Ordering::Relaxed);
        Some((n, q))
    }

    /// Host side: one point for `id` at `offset` (replacing an earlier one this block).
    fn set(&self, id: u32, offset: i32, value: f64) {
        if let Some((_, q)) = self.queue_for(id) {
            q.offset.store(offset, Ordering::Relaxed);
            q.value.store(value.to_bits(), Ordering::Relaxed);
            q.points.store(1, Ordering::Relaxed);
        }
    }

    fn queue_ptr(&self, index: usize) -> *mut sv::IParamValueQueue {
        self.queues.get(index).and_then(|q| q.as_com_ref::<sv::IParamValueQueue>()).map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> sb::int32 {
        self.count.load(Ordering::Relaxed)
    }
    unsafe fn getParameterData(&self, index: sb::int32) -> *mut sv::IParamValueQueue {
        if index < 0 || index >= self.count.load(Ordering::Relaxed) {
            return std::ptr::null_mut();
        }
        self.queue_ptr(index as usize)
    }
    unsafe fn addParameterData(&self, id: *const sv::ParamID, index: *mut sb::int32) -> *mut sv::IParamValueQueue {
        // SAFETY: in-pointer from the plugin (or null, handled).
        let Some(&id) = (unsafe { id.as_ref() }) else { return std::ptr::null_mut() };
        let Some((i, _)) = self.queue_for(id) else { return std::ptr::null_mut() };
        // SAFETY: out-pointer from the plugin (or null, handled).
        if let Some(out) = unsafe { index.as_mut() } {
            *out = i;
        }
        self.queue_ptr(i as usize)
    }
}

/// A bounded event list (`IEventList`). Only ever touched by one thread at a time (the host
/// fills it, then the plugin reads it inside `process`), so the lock is never contended; the
/// plugin side uses `try_lock` and never blocks.
pub(crate) struct EventList {
    events: Mutex<Vec<sv::Event>>,
    cap: usize,
}

impl Class for EventList {
    type Interfaces = (sv::IEventList,);
}

impl EventList {
    fn new(cap: usize) -> EventList {
        EventList { events: Mutex::new(Vec::with_capacity(cap)), cap }
    }
    fn with<R>(&self, f: impl FnOnce(&mut Vec<sv::Event>) -> R) -> Option<R> {
        match self.events.try_lock() {
            Ok(mut g) => Some(f(&mut g)),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(f(&mut p.into_inner())),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> sb::int32 {
        self.with(|v| i32::try_from(v.len()).unwrap_or(i32::MAX)).unwrap_or(0)
    }
    unsafe fn getEvent(&self, index: sb::int32, e: *mut sv::Event) -> sb::tresult {
        // SAFETY: out-pointer from the plugin (or null, handled).
        let Some(out) = (unsafe { e.as_mut() }) else { return sb::kInvalidArgument };
        let Ok(i) = usize::try_from(index) else { return sb::kInvalidArgument };
        match self.with(|v| v.get(i).copied()).flatten() {
            Some(ev) => {
                *out = ev;
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, e: *mut sv::Event) -> sb::tresult {
        // SAFETY: in-pointer from the plugin (or null, handled).
        let Some(&ev) = (unsafe { e.as_ref() }) else { return sb::kInvalidArgument };
        let cap = self.cap;
        // Output events (e.g. MIDI out) are accepted while there is room; SoundCraft ignores them.
        match self.with(|v| {
            if v.len() < cap {
                v.push(ev);
                true
            } else {
                false
            }
        }) {
            Some(true) => sb::kResultOk,
            _ => sb::kResultFalse,
        }
    }
}

/// An in-memory `IBStream`, used to hand the component's state to its controller.
pub(crate) struct MemoryStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl Class for MemoryStream {
    type Interfaces = (sb::IBStream,);
}

impl MemoryStream {
    fn new(data: Vec<u8>) -> ComWrapper<MemoryStream> {
        ComWrapper::new(MemoryStream { inner: Mutex::new((data, 0)) })
    }

    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.lock().0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<u8>, usize)> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: sb::int32, num_read: *mut sb::int32) -> sb::tresult {
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        let want = usize::try_from(num_bytes).unwrap_or(0);
        let avail = data.len().saturating_sub(*pos);
        let n = want.min(avail);
        if n > 0 && !buffer.is_null() {
            let Some(src) = data.get(*pos..*pos + n) else { return sb::kInternalError };
            // SAFETY: the plugin provides `buffer` with room for `num_bytes` ≥ `n` bytes; `src`
            // is `n` valid bytes; the regions cannot overlap (ours is a private Vec).
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), buffer.cast::<u8>(), n) };
            *pos += n;
        }
        // SAFETY: out-pointer from the plugin (or null, handled).
        if let Some(r) = unsafe { num_read.as_mut() } {
            *r = i32::try_from(n).unwrap_or(0);
        }
        sb::kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, num_bytes: sb::int32, num_written: *mut sb::int32) -> sb::tresult {
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        let n = usize::try_from(num_bytes).unwrap_or(0);
        let end = pos.saturating_add(n);
        if buffer.is_null() || end > MAX_STATE_BYTES {
            return sb::kResultFalse;
        }
        if data.len() < end {
            data.resize(end, 0);
        }
        // SAFETY: the plugin provides `n` readable bytes at `buffer`; the destination range was
        // just made valid.
        let src = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
        if let Some(dst) = data.get_mut(*pos..end) {
            dst.copy_from_slice(src);
        }
        *pos = end;
        // SAFETY: out-pointer from the plugin (or null, handled).
        if let Some(w) = unsafe { num_written.as_mut() } {
            *w = num_bytes;
        }
        sb::kResultOk
    }
    unsafe fn seek(&self, pos: sb::int64, mode: sb::int32, result: *mut sb::int64) -> sb::tresult {
        let mut g = self.lock();
        let len = i64::try_from(g.0.len()).unwrap_or(i64::MAX);
        let cur = i64::try_from(g.1).unwrap_or(0);
        let base = match mode as u32 {
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekSet as u32 => 0,
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekCur as u32 => cur,
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekEnd as u32 => len,
            _ => return sb::kInvalidArgument,
        };
        let new = base.saturating_add(pos).clamp(0, len);
        g.1 = usize::try_from(new).unwrap_or(0);
        // SAFETY: out-pointer from the plugin (or null, handled).
        if let Some(r) = unsafe { result.as_mut() } {
            *r = new;
        }
        sb::kResultOk
    }
    unsafe fn tell(&self, pos: *mut sb::int64) -> sb::tresult {
        let g = self.lock();
        // SAFETY: out-pointer from the plugin (or null, handled).
        match unsafe { pos.as_mut() } {
            Some(p) => {
                *p = i64::try_from(g.1).unwrap_or(0);
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }
}

// ---- events and buffers --------------------------------------------------------------------

/// A host → plugin note event, safe to build anywhere; converted inside `process`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NoteEvent {
    On { time: u32, key: u8, velocity: f32 },
    Off { time: u32, key: u8 },
    Pressure { time: u32, key: u8, pressure: f32 },
}

impl NoteEvent {
    pub fn time(&self) -> u32 {
        match *self {
            NoteEvent::On { time, .. } | NoteEvent::Off { time, .. } | NoteEvent::Pressure { time, .. } => time,
        }
    }
    pub fn set_time(&mut self, t: u32) {
        match self {
            NoteEvent::On { time, .. } | NoteEvent::Off { time, .. } | NoteEvent::Pressure { time, .. } => *time = t,
        }
    }

    fn to_raw(self, offset: u32) -> sv::Event {
        // SAFETY: `Event` is plain data (numbers and a union of plain structs); all-zero is valid.
        let mut e: sv::Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = i32::try_from(self.time().saturating_sub(offset)).unwrap_or(0);
        e.flags = sv::Event_::EventFlags_::kIsLive as u16;
        match self {
            NoteEvent::On { key, velocity, .. } => {
                e.r#type = sv::Event_::EventTypes_::kNoteOnEvent as u16;
                e.__field0.noteOn = sv::NoteOnEvent { channel: 0, pitch: i16::from(key), tuning: 0.0, velocity, length: 0, noteId: -1 };
            }
            NoteEvent::Off { key, .. } => {
                e.r#type = sv::Event_::EventTypes_::kNoteOffEvent as u16;
                e.__field0.noteOff = sv::NoteOffEvent { channel: 0, pitch: i16::from(key), velocity: 0.0, noteId: -1, tuning: 0.0 };
            }
            NoteEvent::Pressure { key, pressure, .. } => {
                e.r#type = sv::Event_::EventTypes_::kPolyPressureEvent as u16;
                e.__field0.polyPressure = sv::PolyPressureEvent { channel: 0, pitch: i16::from(key), pressure, noteId: -1 };
            }
        }
        e
    }
}

/// Planar f32 buffers for every audio bus in one direction, with the pointer tables VST3 needs.
/// Allocated in `new`; `process` only rewrites pointers in place (no allocation).
pub(crate) struct BusBuffers {
    data: Vec<Vec<Vec<f32>>>,
    ptrs: Vec<Vec<*mut f32>>,
    bufs: Vec<sv::AudioBusBuffers>,
    frames: usize,
}

// SAFETY: the raw pointers only ever point into `data`, which this struct owns; moving the struct
// to another thread moves that ownership with it.
unsafe impl Send for BusBuffers {}

impl BusBuffers {
    /// One buffer set per bus, `channels[b]` channels each (capped), `frames` long.
    pub fn new(channels: &[u32], frames: usize) -> BusBuffers {
        let data: Vec<Vec<Vec<f32>>> =
            channels.iter().take(MAX_BUSES as usize).map(|&c| (0..c.min(MAX_BUS_CHANNELS)).map(|_| vec![0.0; frames]).collect()).collect();
        let ptrs = data.iter().map(|p| vec![std::ptr::null_mut(); p.len()]).collect();
        let bufs = data
            .iter()
            .map(|_| sv::AudioBusBuffers {
                numChannels: 0,
                silenceFlags: 0,
                __field0: sv::AudioBusBuffers__type0 { channelBuffers32: std::ptr::null_mut() },
            })
            .collect();
        BusBuffers { data, ptrs, bufs, frames }
    }

    pub fn channels(&self, bus: usize) -> usize {
        self.data.get(bus).map_or(0, Vec::len)
    }

    pub fn channel(&self, bus: usize, ch: usize) -> Option<&[f32]> {
        self.data.get(bus).and_then(|p| p.get(ch)).map(Vec::as_slice)
    }

    pub fn channel_mut(&mut self, bus: usize, ch: usize) -> Option<&mut [f32]> {
        self.data.get_mut(bus).and_then(|p| p.get_mut(ch)).map(Vec::as_mut_slice)
    }

    /// Points the VST3 tables at the current data (no allocation).
    fn refresh(&mut self) {
        for ((d, p), b) in self.data.iter_mut().zip(self.ptrs.iter_mut()).zip(self.bufs.iter_mut()) {
            for (dc, pc) in d.iter_mut().zip(p.iter_mut()) {
                *pc = dc.as_mut_ptr();
            }
            b.numChannels = i32::try_from(p.len()).unwrap_or(0);
            b.silenceFlags = 0;
            b.__field0.channelBuffers32 = p.as_mut_ptr();
        }
    }
}

// ---- instances -----------------------------------------------------------------------------

/// One parameter as reported by `IEditController::getParameterInfo`, plus its plain range.
#[derive(Debug, Clone)]
pub(crate) struct RawParam {
    pub id: u32,
    pub title: String,
    pub units: String,
    pub step_count: i32,
    pub default_normalized: f64,
    pub flags: i32,
}

pub(crate) const PARAM_HIDDEN: i32 = sv::ParameterInfo_::ParameterFlags_::kIsHidden;
pub(crate) const PARAM_READ_ONLY: i32 = sv::ParameterInfo_::ParameterFlags_::kIsReadOnly;
pub(crate) const PARAM_BYPASS: i32 = sv::ParameterInfo_::ParameterFlags_::kIsBypass;

/// The audio bus layout chosen by [`Instance::configure`].
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Layout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub event_input: bool,
}

fn apply_transport(c: &mut sv::ProcessContext, t: &Transport) {
    use sv::ProcessContext_::StatesAndFlags_::*;
    let mut state = kTempoValid | kTimeSigValid | kProjectTimeMusicValid | kBarPositionValid | kContTimeValid;
    if t.playing {
        state |= kPlaying;
    }
    if t.recording {
        state |= kRecording;
    }
    // The flag constants are the C default enum type: i32 on Windows, u32 elsewhere.
    #[allow(clippy::unnecessary_cast)]
    let state = state as u32;
    c.state = state;
    c.tempo = t.tempo;
    c.timeSigNumerator = i32::try_from(t.numerator).unwrap_or(4);
    c.timeSigDenominator = i32::try_from(t.denominator).unwrap_or(4);
    c.projectTimeSamples = t.samples;
    c.projectTimeMusic = t.quarters;
    c.barPositionMusic = t.bar_start;
}

/// A live plugin: component + audio processor (+ edit controller) and the host objects.
pub(crate) struct Instance {
    processor: Option<ComPtr<sv::IAudioProcessor>>,
    controller: Option<ComPtr<sv::IEditController>>,
    /// The controller is the component itself (single-component plugin).
    single: bool,
    connections: Option<(ComPtr<sv::IConnectionPoint>, ComPtr<sv::IConnectionPoint>)>,
    component_initialized: bool,
    controller_initialized: bool,
    active: bool,
    processing: bool,
    max_frames: usize,
    in_params: ComWrapper<ParamChanges>,
    out_params: ComWrapper<ParamChanges>,
    in_events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    context: sv::ProcessContext,
    event_in: bool,
    editor: Arc<EditorLink>,
    // Field order matters for drop: `Drop::drop` shuts the plugin down first; then the component
    // and host objects are released, and the bundle (which keeps the code mapped) goes last.
    component: ComPtr<sv::IComponent>,
    host: ComWrapper<HostContext>,
    bundle: Arc<Bundle>,
}

// SAFETY: an `Instance` is used through `&mut self` only, so calls into the plugin are always
// serialised. SoundCraft creates instances on the engine/UI thread and then moves them into the
// mix engine (documented limitation: VST3 expects `setActive`, controller calls etc. on the UI
// thread; they may then happen on the audio thread).
unsafe impl Send for Instance {}

impl Instance {
    fn processor(&self) -> Option<&ComPtr<sv::IAudioProcessor>> {
        self.processor.as_ref()
    }

    pub fn host(&self) -> &HostContext {
        &self.host
    }

    /// A counted reference to the host context (editor handles read its edit queue).
    pub fn host_ref(&self) -> ComWrapper<HostContext> {
        self.host.clone()
    }

    /// Turns the host's run loop off for this plugin's editor: a bridged (Wine) plugin answers its
    /// own Win32 event loop inside yabridge, and driving the same GUI from here as well can block it.
    pub fn set_run_loop_enabled(&self, enabled: bool) {
        if let Ok(mut g) = self.editor.inner.lock() {
            g.run_loop_enabled = enabled;
        }
    }

    /// Marks this instance busy while the returned guard lives (called around `process`). The
    /// guard keeps its own reference, so it does not borrow the instance that is being used.
    pub fn busy_guard(&self) -> AudioBusyGuard {
        AudioBusyGuard::new(std::sync::Arc::clone(&self.editor))
    }

    /// The editor link shared with editor handles (`None` without an edit controller).
    pub fn editor(&self) -> Option<Arc<EditorLink>> {
        self.editor.has_controller().then(|| Arc::clone(&self.editor))
    }

    /// The same link, for state handles.
    pub fn state_link(&self) -> Arc<EditorLink> {
        Arc::clone(&self.editor)
    }

    /// Writes one state into a fresh memory stream.
    fn read_state(f: impl FnOnce(*mut sb::IBStream) -> sb::tresult) -> Option<Vec<u8>> {
        let stream = MemoryStream::new(Vec::new());
        let s = stream.as_com_ref::<sb::IBStream>()?;
        ok(f(s.as_ptr())).then(|| stream.take())
    }

    /// Restores a [`EditorLink::save_state`] blob (or a bare component state): component
    /// `setState`, then the controller's `setComponentState` and `setState`. `false` when the
    /// blob is malformed or the component refuses it.
    pub fn load_state(&self, data: &[u8]) -> bool {
        let Some((comp, ctrl)) = split_state(data) else { return false };
        let stream = MemoryStream::new(comp.to_vec());
        let Some(s) = stream.as_com_ref::<sb::IBStream>() else { return false };
        // SAFETY: valid component; the stream lives on this frame for the call.
        if !ok(unsafe { self.component.setState(s.as_ptr()) }) {
            return false;
        }
        if let Some(c) = &self.controller {
            stream.lock().1 = 0;
            // SAFETY: valid controller; the same stream, rewound.
            unsafe { c.setComponentState(s.as_ptr()) };
            if !ctrl.is_empty() {
                let cs = MemoryStream::new(ctrl.to_vec());
                if let Some(r) = cs.as_com_ref::<sb::IBStream>() {
                    // SAFETY: valid controller; the stream lives on this frame for the call.
                    unsafe { c.setState(r.as_ptr()) };
                }
            }
        }
        true
    }

    /// Copies the component's state into the controller (`getState` → `setComponentState`).
    fn sync_controller_state(&self) {
        let Some(c) = &self.controller else { return };
        let stream = ComWrapper::new(MemoryStream { inner: Mutex::new((Vec::new(), 0)) });
        let Some(s) = stream.as_com_ref::<sb::IBStream>() else { return };
        // SAFETY: valid component; the stream lives on this frame for the whole call.
        if ok(unsafe { self.component.getState(s.as_ptr()) }) {
            stream.lock().1 = 0;
            // SAFETY: valid controller; same stream, rewound.
            unsafe { c.setComponentState(s.as_ptr()) };
        }
    }

    /// `IEditController::getParameterCount` (deprecated but still declared by many plugins).
    pub fn declared_param_count(&self) -> i32 {
        let Some(c) = &self.controller else { return 0 };
        // SAFETY: valid controller.
        unsafe { c.getParameterCount() }
    }

    /// Whether the controller answers `getParameterInfo(0)` right now.
    pub fn first_param_info_ok(&self) -> bool {
        let Some(c) = &self.controller else { return false };
        // SAFETY: `ParameterInfo` is plain data; all-zero is valid.
        let mut info: sv::ParameterInfo = unsafe { std::mem::zeroed() };
        // SAFETY: index 0; `info` is writable.
        ok(unsafe { c.getParameterInfo(0, &mut info) })
    }

    /// The plugin's `IUnitInfo` program lists (program names included), or empty when it has none.
    ///
    /// VST3 has no "set program" call: a plugin that ships preset banks publishes them as program
    /// lists here, and the host selects one through a parameter flagged `kIsProgramChange` (see
    /// [`Instance::select_program`]).
    pub fn program_lists(&self) -> Vec<soundcraft_dsp::ProgramList> {
        Self::program_lists_of(&self.editor.lock())
    }

    fn program_lists_of(g: &EditorInner) -> Vec<soundcraft_dsp::ProgramList> {
        let unit =
            g.controller.as_ref().and_then(|c| c.cast::<sv::IUnitInfo>()).or_else(|| g.component.as_ref().and_then(|c| c.cast::<sv::IUnitInfo>()));
        Self::program_lists_from(unit)
    }

    fn program_lists_from(u: Option<ComPtr<sv::IUnitInfo>>) -> Vec<soundcraft_dsp::ProgramList> {
        let Some(u) = u else { return Vec::new() };
        // SAFETY: valid `IUnitInfo`; the count is a plain answer.
        let lists = unsafe { u.getProgramListCount() }.clamp(0, MAX_PROGRAM_LISTS);
        let mut out = Vec::new();
        for i in 0..lists {
            // SAFETY: `ProgramListInfo` is plain data; all-zero is valid.
            let mut info: sv::ProgramListInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `i` is within the count the plugin just gave; `info` is writable.
            if !ok(unsafe { u.getProgramListInfo(i, &mut info) }) {
                continue;
            }
            let count = info.programCount.clamp(0, MAX_PROGRAMS);
            let mut programs = Vec::with_capacity(count as usize);
            for j in 0..count {
                let mut name: sv::String128 = [0; 128];
                // SAFETY: valid list id and index; `name` is a writable String128.
                if ok(unsafe { u.getProgramName(info.id, j, &mut name) }) {
                    programs.push(read_c16(&name));
                } else {
                    programs.push(format!("Program {}", j + 1));
                }
            }
            out.push(soundcraft_dsp::ProgramList { id: info.id, name: read_c16(&info.name), programs });
        }
        out
    }

    /// Selects `program` of `list` (indices into [`Instance::program_lists`]).
    ///
    /// The VST3 way is a controller parameter flagged `kIsProgramChange` whose normalized value
    /// addresses the program list; its value is the program index scaled to the parameter's range.
    /// A plugin that has program lists but no such parameter is reported, not guessed at.
    /// Returns the `(parameter id, normalized value)` it set, so the caller can forward the change
    /// to the processor: with a *separate* controller (Pianoteq is one) the processor only learns
    /// about it through the next block's parameter queue, and setting the controller alone changes
    /// nothing audible.
    pub fn select_program(&mut self, list: usize, program: usize) -> Result<(u32, f64), String> {
        let lists = self.program_lists();
        let Some(l) = lists.get(list) else { return Err(format!("this plugin has no program list {list}")) };
        if program >= l.programs.len() {
            return Err(format!("`{}` has {} programs, so {program} is out of range", l.name, l.programs.len()));
        }
        let Some(c) = self.controller.as_ref().cloned() else { return Err("the plugin has no edit controller".into()) };
        // The program-change parameter: the one the SDK tags for exactly this.
        let mut seen = std::collections::HashSet::new();
        for i in 0..MAX_PARAMS {
            // SAFETY: `ParameterInfo` is plain data; all-zero is valid.
            let mut info: sv::ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: the controller answers by index; `info` is writable.
            if !ok(unsafe { c.getParameterInfo(i, &mut info) }) || !seen.insert(info.id) {
                break;
            }
            if info.flags & sv::ParameterInfo_::ParameterFlags_::kIsProgramChange == 0 {
                continue;
            }
            // The parameter's own step count wins (a plugin may expose 128 steps for fewer programs).
            let steps = if info.stepCount > 0 { info.stepCount as f64 } else { (l.programs.len().max(2) - 1) as f64 };
            let normalized = (program as f64 / steps).clamp(0.0, 1.0);
            // SAFETY: valid controller and a parameter id it just reported; the value is clamped.
            unsafe { c.setParamNormalized(info.id, normalized) };
            log::info!(
                "vst3: program `{}` selected through parameter {} ({normalized:.4})",
                l.programs.get(program).cloned().unwrap_or_default(),
                info.id
            );
            return Ok((info.id, normalized));
        }
        Err(format!("`{}` publishes programs but no program-change parameter", l.name))
    }

    pub fn params(&self) -> Vec<RawParam> {
        let Some(c) = &self.controller else { return Vec::new() };
        let mut out = Vec::new();
        // `IEditController::getParameterCount` was deprecated in VST3 3.6.14: modern plugins
        // (Pianoteq 9 among them) return 0 there while still answering `getParameterInfo` by
        // index. Trusting the count alone left such a plugin with no parameters at all — no
        // defaults, no automation and nothing to restore — so walk the indices instead and stop
        // at the first refusal or at a repeated id (a plugin that loops forever).
        let mut seen = std::collections::HashSet::new();
        for i in 0..MAX_PARAMS {
            // SAFETY: `ParameterInfo` is plain data; all-zero is valid.
            let mut info: sv::ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: the controller answers by index; `info` is writable.
            if !ok(unsafe { c.getParameterInfo(i, &mut info) }) || !seen.insert(info.id) {
                break;
            }
            let d = info.defaultNormalizedValue;
            out.push(RawParam {
                id: info.id,
                title: read_c16(&info.title),
                units: read_c16(&info.units),
                step_count: info.stepCount.max(0),
                default_normalized: if d.is_finite() { d.clamp(0.0, 1.0) } else { 0.0 },
                flags: info.flags,
            });
        }
        out
    }

    /// Normalized → plain (`None` without a controller or for a non-finite answer).
    pub fn to_plain(&self, id: u32, normalized: f64) -> Option<f64> {
        let c = self.controller.as_ref()?;
        // SAFETY: valid controller.
        let v = unsafe { c.normalizedParamToPlain(id, normalized.clamp(0.0, 1.0)) };
        v.is_finite().then_some(v)
    }

    /// Plain → normalized, clamped to 0..1.
    pub fn to_normalized(&self, id: u32, plain: f64) -> Option<f64> {
        let c = self.controller.as_ref()?;
        if !plain.is_finite() {
            return None;
        }
        // SAFETY: valid controller.
        let v = unsafe { c.plainParamToNormalized(id, plain) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    pub fn normalized_value(&self, id: u32) -> Option<f64> {
        let c = self.controller.as_ref()?;
        // SAFETY: valid controller.
        let v = unsafe { c.getParamNormalized(id) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    /// The parameter each MIDI controller drives on channel 1 (`IMidiMapping`), indexed by VST3's
    /// controller numbers: CCs 0–127, then [`AFTERTOUCH`] and [`PITCH_BEND`]. Main thread only.
    pub fn midi_mapping(&self) -> Vec<Option<u32>> {
        let Some(m) = self.controller.as_ref().and_then(|c| c.cast::<sv::IMidiMapping>()) else { return Vec::new() };
        (0..=PROGRAM_CHANGE)
            .map(|n| {
                let mut id: sv::ParamID = 0;
                let n = i16::try_from(n).unwrap_or(0);
                // SAFETY: valid mapping interface on the main thread; `id` is a local.
                ok(unsafe { m.getMidiControllerAssignment(0, 0, n, &mut id) }).then_some(id)
            })
            .collect()
    }

    pub fn value_text(&self, id: u32, normalized: f64) -> Option<String> {
        let c = self.controller.as_ref()?;
        let mut s: sv::String128 = [0; 128];
        // SAFETY: valid controller; `s` is a writable String128.
        ok(unsafe { c.getParamStringByValue(id, normalized.clamp(0.0, 1.0), &mut s) }).then(|| read_c16(&s)).filter(|t| !t.is_empty())
    }

    pub fn latency(&self) -> u32 {
        // SAFETY: valid processor.
        self.processor().map_or(0, |p| unsafe { p.getLatencySamples() })
    }

    pub fn tail(&self) -> u32 {
        // SAFETY: valid processor.
        self.processor().map_or(0, |p| unsafe { p.getTailSamples() })
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    fn bus_count(&self, media: sv::MediaTypes, dir: sv::BusDirections) -> i32 {
        // SAFETY: valid component.
        unsafe { self.component.getBusCount(media as sv::MediaType, dir as sv::BusDirection) }.clamp(0, MAX_BUSES)
    }

    fn arrangement(&self, dir: sv::BusDirections, index: i32) -> Option<sv::SpeakerArrangement> {
        let p = self.processor()?;
        let mut arr: sv::SpeakerArrangement = 0;
        // SAFETY: valid processor; `arr` is writable.
        ok(unsafe { p.getBusArrangement(dir as sv::BusDirection, index, &mut arr) }).then_some(arr)
    }

    fn bus_channels(&self, dir: sv::BusDirections, index: i32) -> u32 {
        if let Some(arr) = self.arrangement(dir, index) {
            return arr.count_ones().min(MAX_BUS_CHANNELS);
        }
        // SAFETY: plain data; all-zero is valid.
        let mut info: sv::BusInfo = unsafe { std::mem::zeroed() };
        // SAFETY: valid component; index below the bus count; `info` is writable.
        if ok(unsafe { self.component.getBusInfo(sv::MediaTypes_::kAudio as sv::MediaType, dir as sv::BusDirection, index, &mut info) }) {
            u32::try_from(info.channelCount).unwrap_or(0).min(MAX_BUS_CHANNELS)
        } else {
            0
        }
    }

    /// Stops processing, deactivates, negotiates the bus layout (main buses mono or stereo as
    /// asked, falling back to stereo and then to the plugin's own choice), activates the main
    /// buses, sets up 32-bit processing and activates the component.
    pub fn configure(&mut self, sample_rate: f64, max_frames: usize, mono: bool) -> Result<Layout, Vst3Error> {
        self.shutdown();
        let audio = sv::MediaTypes_::kAudio;
        let event = sv::MediaTypes_::kEvent;
        let (din, dout) = (sv::BusDirections_::kInput, sv::BusDirections_::kOutput);
        let nin = self.bus_count(audio, din);
        let nout = self.bus_count(audio, dout);
        let current = |s: &Self, dir: sv::BusDirections, n: i32| -> Vec<sv::SpeakerArrangement> {
            (0..n).map(|i| s.arrangement(dir, i).unwrap_or(sv::SpeakerArr::kStereo)).collect()
        };
        let want = if mono { sv::SpeakerArr::kMono } else { sv::SpeakerArr::kStereo };
        let p = self.processor().ok_or_else(|| Vst3Error::Process("no audio processor".into()))?.clone();
        let mut tried_ok = false;
        for main in [want, sv::SpeakerArr::kStereo] {
            let mut ins = current(self, din, nin);
            let mut outs = current(self, dout, nout);
            if let Some(a) = ins.first_mut() {
                *a = main;
            }
            if let Some(a) = outs.first_mut() {
                *a = main;
            }
            // SAFETY: valid processor; the arrays hold exactly `nin`/`nout` arrangements and
            // outlive the call.
            if ok(unsafe { p.setBusArrangements(ins.as_mut_ptr(), nin, outs.as_mut_ptr(), nout) }) {
                tried_ok = true;
                break;
            }
            if main == sv::SpeakerArr::kStereo {
                break;
            }
        }
        if !tried_ok {
            log::debug!("vst3: plugin refused mono/stereo bus arrangements; using its own");
        }
        let layout = Layout {
            inputs: (0..nin).map(|i| self.bus_channels(din, i)).collect(),
            outputs: (0..nout).map(|i| self.bus_channels(dout, i)).collect(),
            event_input: self.bus_count(event, din) > 0,
        };
        // Main buses on, aux buses off (VST3 still gets buffers for every bus).
        for (dir, n) in [(din, nin), (dout, nout)] {
            for i in 0..n {
                // SAFETY: valid component; index below the bus count.
                unsafe { self.component.activateBus(audio as sv::MediaType, dir as sv::BusDirection, i, u8::from(i == 0)) };
            }
        }
        if layout.event_input {
            // SAFETY: valid component; event input bus 0 exists.
            unsafe { self.component.activateBus(event as sv::MediaType, din as sv::BusDirection, 0, 1) };
        }
        self.event_in = layout.event_input;
        let sample32 = sv::SymbolicSampleSizes_::kSample32 as i32;
        // SAFETY: valid processor.
        if !ok(unsafe { p.canProcessSampleSize(sample32) }) {
            return Err(Vst3Error::Process("plugin cannot process 32-bit float".into()));
        }
        let max = i32::try_from(max_frames).unwrap_or(i32::MAX);
        let mut setup = sv::ProcessSetup {
            processMode: sv::ProcessModes_::kRealtime as i32,
            symbolicSampleSize: sample32,
            maxSamplesPerBlock: max,
            sampleRate: sample_rate,
        };
        // SAFETY: valid processor, inactive; `setup` is a valid struct for the call.
        if !ok(unsafe { p.setupProcessing(&mut setup) }) {
            return Err(Vst3Error::Process("setupProcessing failed".into()));
        }
        // SAFETY: valid component, configured above.
        if !ok(unsafe { self.component.setActive(1) }) {
            return Err(Vst3Error::Process("setActive failed".into()));
        }
        self.active = true;
        self.max_frames = max_frames;
        self.context.sampleRate = sample_rate;
        self.context.tempo = 120.0;
        self.context.timeSigNumerator = 4;
        self.context.timeSigDenominator = 4;
        // The flag constants are the C default enum type: i32 on Windows, u32 elsewhere.
        #[allow(clippy::unnecessary_cast)]
        let flags = (sv::ProcessContext_::StatesAndFlags_::kTempoValid | sv::ProcessContext_::StatesAndFlags_::kTimeSigValid) as u32;
        self.context.state = flags;
        self.host.latency_changed.store(false, Ordering::Relaxed);
        Ok(layout)
    }

    /// `setProcessing(false)` + `setActive(false)` when needed.
    pub fn shutdown(&mut self) {
        if self.processing {
            if let Some(p) = self.processor() {
                // SAFETY: valid processor in the processing state.
                unsafe { p.setProcessing(0) };
            }
            self.processing = false;
        }
        if self.active {
            // SAFETY: valid, active component.
            unsafe { self.component.setActive(0) };
            self.active = false;
        }
    }

    /// Runs one `process` call over `frames` frames: `params` become one point each at offset 0,
    /// `controls` (time, id, value) one point at their time when it falls in this call (the last
    /// one wins per parameter), and `notes` (sorted by time) have their times made relative to
    /// `offset`.
    pub fn process(
        &mut self,
        frames: usize,
        inputs: &mut BusBuffers,
        outputs: &mut BusBuffers,
        params: &[(u32, f64)],
        controls: &[(u32, u32, f64)],
        notes: &[NoteEvent],
        offset: u32,
        project_time: i64,
        transport: Option<&Transport>,
    ) -> Result<(), Vst3Error> {
        if !self.active {
            return Err(Vst3Error::Process("not active".into()));
        }
        if frames == 0 || frames > self.max_frames || frames > inputs.frames || frames > outputs.frames {
            return Err(Vst3Error::Process("block larger than the configured size".into()));
        }
        let p = self.processor.clone().ok_or_else(|| Vst3Error::Process("no audio processor".into()))?;
        if !self.processing {
            // Many plugins return kNotImplemented here; that is not an error.
            // SAFETY: valid, active processor.
            unsafe { p.setProcessing(1) };
            self.processing = true;
        }
        self.in_params.clear();
        for &(id, v) in params.iter().take(PARAM_QUEUE_CAP) {
            self.in_params.set(id, 0, v.clamp(0.0, 1.0));
        }
        for &(time, id, v) in controls {
            if let Some(at) = time.checked_sub(offset).filter(|&t| (t as usize) < frames) {
                self.in_params.set(id, i32::try_from(at).unwrap_or(0), v.clamp(0.0, 1.0));
            }
        }
        self.out_params.clear();
        let event_in = self.event_in;
        self.in_events.with(|v| {
            v.clear();
            if event_in {
                for n in notes.iter().take(EVENT_CAP) {
                    v.push(n.to_raw(offset));
                }
            }
        });
        self.out_events.with(Vec::clear);
        inputs.refresh();
        outputs.refresh();
        self.context.continousTimeSamples = project_time;
        match transport {
            Some(t) => apply_transport(&mut self.context, t),
            None => self.context.projectTimeSamples = project_time,
        }
        let mut data = sv::ProcessData {
            processMode: sv::ProcessModes_::kRealtime as i32,
            symbolicSampleSize: sv::SymbolicSampleSizes_::kSample32 as i32,
            numSamples: i32::try_from(frames).unwrap_or(0),
            numInputs: i32::try_from(inputs.bufs.len()).unwrap_or(0),
            numOutputs: i32::try_from(outputs.bufs.len()).unwrap_or(0),
            inputs: if inputs.bufs.is_empty() { std::ptr::null_mut() } else { inputs.bufs.as_mut_ptr() },
            outputs: if outputs.bufs.is_empty() { std::ptr::null_mut() } else { outputs.bufs.as_mut_ptr() },
            inputParameterChanges: self.in_params.as_com_ref::<sv::IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputParameterChanges: self.out_params.as_com_ref::<sv::IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            inputEvents: self.in_events.as_com_ref::<sv::IEventList>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputEvents: self.out_events.as_com_ref::<sv::IEventList>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            processContext: &mut self.context,
        };
        // SAFETY: valid, active processor. Every channel pointer refers to `frames` (or more) valid
        // f32s owned by `inputs`/`outputs`, borrowed for the whole call; the bus tables, the
        // parameter/event objects and the context are owned by `self` and untouched until the
        // call returns.
        let r = unsafe { p.process(&mut data) };
        if ok(r) { Ok(()) } else { Err(Vst3Error::Process(format!("process returned {r}"))) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Close any editor and cut its handles off before the controller goes away.
        if self.editor.kill() {
            self.bundle.mark_leaked();
        }
        self.shutdown();
        if let Some((a, b)) = self.connections.take() {
            // SAFETY: the pair was connected in `instantiate`; disconnect before terminating.
            unsafe {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
        }
        if let Some(c) = self.controller.take() {
            // SAFETY: valid controller; detach our handler before it goes away.
            unsafe { c.setComponentHandler(std::ptr::null_mut()) };
            if self.controller_initialized {
                // SAFETY: separately created controller, initialised once; terminated once.
                unsafe { c.terminate() };
            }
        }
        self.processor = None;
        if self.component_initialized {
            // SAFETY: valid component, inactive; terminated exactly once.
            unsafe { self.component.terminate() };
            self.component_initialized = false;
        }
    }
}

/// Splits a state blob into (component, controller) parts. A blob without SoundCraft's container
/// header is taken as a bare component state.
fn split_state(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if data.len() > MAX_STATE_BYTES.saturating_mul(2) {
        return None;
    }
    let Some(rest) = data.strip_prefix(STATE_MAGIC) else { return (!data.is_empty()).then_some((data, &[][..])) };
    let take = |b: &[u8]| -> Option<(usize, usize)> {
        let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        (n <= MAX_STATE_BYTES && b.len() >= 4 + n).then_some((4, 4 + n))
    };
    let (a, b) = take(rest)?;
    let comp = rest.get(a..b)?;
    let rest = rest.get(b..)?;
    let (c, d) = take(rest)?;
    let ctrl = rest.get(c..d)?;
    (d == rest.len()).then_some((comp, ctrl))
}

// ---- editor (IPlugView) --------------------------------------------------------------------

// True while the host is inside a callback the plugin's own GUI started (a run-loop handler).
// Whatever the plugin calls back into the host during that time must not call the plugin again:
// nested calls into a Win32/Wine UI from inside its own event handler deadlock the UI thread.
#[cfg(target_os = "linux")]
thread_local! {
    static IN_PLUGIN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(target_os = "linux")]
fn in_plugin_callback() -> bool {
    IN_PLUGIN_CALLBACK.with(|f| f.get())
}

/// Runs `f` inside the plugin's GUI callback (see [`IN_PLUGIN_CALLBACK`]).
#[cfg(target_os = "linux")]
fn enter_plugin_callback<R>(f: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            IN_PLUGIN_CALLBACK.with(|f| f.set(self.0));
        }
    }
    let previous = IN_PLUGIN_CALLBACK.with(|f| f.replace(true));
    let _reset = Reset(previous);
    f()
}

/// The host side of `Linux::IRunLoop`: the file descriptors and timers a plugin view registers so
/// its GUI can deliver events. Without it a bridged (Wine) or JUCE editor opens a window that never
/// answers: Wine and JUCE hand their event pump to the host and wait for it to fire.
#[cfg(target_os = "linux")]
pub(crate) struct RunLoop {
    handlers: Mutex<Vec<(sb::int32, ComPtr<sb::Linux::IEventHandler>)>>,
    timers: Mutex<Vec<RunTimer>>,
}

#[cfg(target_os = "linux")]
struct RunTimer {
    handler: ComPtr<sb::Linux::ITimerHandler>,
    interval_ms: u64,
    next: std::time::Instant,
}

#[cfg(target_os = "linux")]
impl Default for RunLoop {
    fn default() -> Self {
        RunLoop { handlers: Mutex::new(Vec::new()), timers: Mutex::new(Vec::new()) }
    }
}

#[cfg(target_os = "linux")]
impl Class for RunLoop {
    type Interfaces = (sb::Linux::IRunLoop,);
}

#[cfg(target_os = "linux")]
impl RunLoop {
    /// Fires the handlers whose descriptor is readable and the timers that are due. Called from
    /// the UI thread every frame while an editor is open; never blocks.
    pub fn pump(&self) {
        // Descriptors first: JUCE-style GUIs wake on their X connection and then want their timers.
        // Snapshot the registrations, then call the handlers with no lock held: a handler may
        // register or unregister another descriptor from inside the callback.
        let handlers: Vec<(sb::int32, ComPtr<sb::Linux::IEventHandler>)> = self.handlers.lock().map(|h| h.clone()).unwrap_or_default();
        if !handlers.is_empty() {
            let mut fds: Vec<PollFd> = handlers.iter().map(|(fd, _)| PollFd { fd: *fd, events: POLLIN, revents: 0 }).collect();
            let n = poll_fds(&mut fds);
            if n > 0 {
                let readable: Vec<sb::int32> = fds.iter().filter(|f| f.revents != 0).map(|f| f.fd).collect();
                for (fd, handler) in handlers.iter() {
                    if readable.contains(fd) {
                        // SAFETY: the handler stays valid until the plugin unregisters it (the VST3
                        // contract for `IRunLoop`), and we hold our own reference to it.
                        enter_plugin_callback(|| unsafe { handler.onFDIsSet(*fd) });
                    }
                }
            }
        }
        let now = std::time::Instant::now();
        let mut due: Vec<(ComPtr<sb::Linux::ITimerHandler>, u64)> = Vec::new();
        if let Ok(mut timers) = self.timers.lock() {
            for timer in timers.iter_mut() {
                if now >= timer.next {
                    // Periodic: keep the cadence, but never fire a backlog after a stall.
                    timer.next = now + std::time::Duration::from_millis(timer.interval_ms.max(1));
                    due.push((timer.handler.clone(), timer.interval_ms));
                }
            }
        }
        for (handler, _) in due {
            // SAFETY: registered handler, released only when the plugin unregisters it.
            enter_plugin_callback(|| unsafe { handler.onTimer() });
        }
    }

    /// `(descriptors, timers)` currently registered.
    pub fn load(&self) -> (usize, usize) {
        let fds = self.handlers.lock().map(|h| h.len()).unwrap_or(0);
        let timers = self.timers.lock().map(|t| t.len()).unwrap_or(0);
        (fds, timers)
    }

    fn remove_handler(&self, handler: *mut sb::Linux::IEventHandler) {
        if let Ok(mut handlers) = self.handlers.lock() {
            handlers.retain(|(_, h)| h.as_ptr() != handler);
        }
    }

    fn remove_timer(&self, handler: *mut sb::Linux::ITimerHandler) {
        if let Ok(mut timers) = self.timers.lock() {
            timers.retain(|t| t.handler.as_ptr() != handler);
        }
    }
}

#[cfg(target_os = "linux")]
impl IRunLoopTrait for RunLoop {
    unsafe fn registerEventHandler(&self, handler: *mut sb::Linux::IEventHandler, fd: sb::Linux::FileDescriptor) -> sb::tresult {
        // SAFETY: the plugin passes a live handler (or null, handled); the reference taken below
        // keeps it alive for as long as it is registered.
        let Some(handler) = (unsafe { vst3::ComRef::from_raw(handler) }) else { return sb::kInvalidArgument };
        let owned = handler.to_com_ptr();
        if let Ok(mut handlers) = self.handlers.lock()
            && handlers.len() < MAX_RUN_LOOP_ENTRIES
        {
            log::debug!("editor run loop: view registered file descriptor {fd}");
            handlers.push((fd, owned));
            return sb::kResultOk;
        }
        sb::kResultFalse
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut sb::Linux::IEventHandler) -> sb::tresult {
        self.remove_handler(handler);
        sb::kResultOk
    }

    unsafe fn registerTimer(&self, handler: *mut sb::Linux::ITimerHandler, milliseconds: sb::Linux::TimerInterval) -> sb::tresult {
        // SAFETY: as above for the timer handler.
        let Some(handler) = (unsafe { vst3::ComRef::from_raw(handler) }) else { return sb::kInvalidArgument };
        let owned = handler.to_com_ptr();
        let ms = milliseconds.clamp(1, 60_000);
        if let Ok(mut timers) = self.timers.lock()
            && timers.len() < MAX_RUN_LOOP_ENTRIES
        {
            log::debug!("editor run loop: view registered a {ms} ms timer");
            timers.push(RunTimer { handler: owned, interval_ms: ms, next: std::time::Instant::now() + std::time::Duration::from_millis(ms) });
            return sb::kResultOk;
        }
        sb::kResultFalse
    }

    unsafe fn unregisterTimer(&self, handler: *mut sb::Linux::ITimerHandler) -> sb::tresult {
        self.remove_timer(handler);
        sb::kResultOk
    }
}

/// One `struct pollfd`, for the run loop's descriptor check.
#[cfg(target_os = "linux")]
#[repr(C)]
struct PollFd {
    fd: sb::int32,
    events: i16,
    revents: i16,
}

#[cfg(target_os = "linux")]
const POLLIN: i16 = 0x0001;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: std::ffi::c_ulong, timeout: std::ffi::c_int) -> std::ffi::c_int;
}

/// How many descriptors/timers one view may register (a hostile plugin must not grow memory).
#[cfg(target_os = "linux")]
const MAX_RUN_LOOP_ENTRIES: usize = 256;

/// Polls `fds` with a zero timeout; a negative result (interrupted) reads as "nothing ready".
#[cfg(target_os = "linux")]
fn poll_fds(fds: &mut [PollFd]) -> i32 {
    if fds.is_empty() {
        return 0;
    }
    // SAFETY: `fds` is a live slice of `struct pollfd`; the length is its own.
    let n = unsafe { poll(fds.as_mut_ptr(), fds.len() as std::ffi::c_ulong, 0) };
    n.max(0)
}

/// The `IPlugFrame` handed to a plugin view: `resizeView` resizes our host window.
pub(crate) struct PlugFrame {
    /// The host window (an `NSWindow*` on macOS), not owned; null when there is none.
    window: std::sync::atomic::AtomicPtr<c_void>,
    /// Whether this view may register its event pump with us. Bridged (Wine) plugins answer their
    /// own Win32 event loop inside yabridge at 60 Hz; registering here too means two threads drive
    /// the same GUI, and a plugin whose GUI blocks (seen with Omnisphere) then freezes the editor.
    run_loop_enabled: std::sync::atomic::AtomicBool,
    /// The Linux run loop the plugin's view registers its event pump with. Shared with the editor
    /// link, which pumps it *without* holding the editor's lock: a handler can call back into the
    /// host (`resizeView`, parameter edits), and those calls take the same lock.
    #[cfg(target_os = "linux")]
    run_loop: std::sync::Arc<RunLoop>,
}

#[cfg(target_os = "linux")]
impl Class for PlugFrame {
    // A Linux plugin view asks its frame for `Linux::IRunLoop`: that is how the GUI gets its file
    // descriptors and timers serviced.
    type Interfaces = (sb::IPlugFrame, sb::Linux::IRunLoop);
}

#[cfg(not(target_os = "linux"))]
impl Class for PlugFrame {
    type Interfaces = (sb::IPlugFrame,);
}

#[cfg(target_os = "linux")]
impl IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(&self, handler: *mut sb::Linux::IEventHandler, fd: sb::Linux::FileDescriptor) -> sb::tresult {
        if !self.run_loop_enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return sb::kResultFalse;
        }
        // SAFETY: forwarded to the run loop, which takes a reference to the handler.
        unsafe { self.run_loop.registerEventHandler(handler, fd) }
    }
    unsafe fn unregisterEventHandler(&self, handler: *mut sb::Linux::IEventHandler) -> sb::tresult {
        // SAFETY: as above.
        unsafe { self.run_loop.unregisterEventHandler(handler) }
    }
    unsafe fn registerTimer(&self, handler: *mut sb::Linux::ITimerHandler, milliseconds: sb::Linux::TimerInterval) -> sb::tresult {
        if !self.run_loop_enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return sb::kResultFalse;
        }
        // SAFETY: as above.
        unsafe { self.run_loop.registerTimer(handler, milliseconds) }
    }
    unsafe fn unregisterTimer(&self, handler: *mut sb::Linux::ITimerHandler) -> sb::tresult {
        // SAFETY: as above.
        unsafe { self.run_loop.unregisterTimer(handler) }
    }
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut sb::IPlugView, new_size: *mut sb::ViewRect) -> sb::tresult {
        // SAFETY: in-pointer from the plugin (or null, handled).
        let Some(r) = (unsafe { new_size.as_mut() }) else { return sb::kInvalidArgument };
        let (w, h) = rect_size(r);
        window::resize(self.window.load(Ordering::Relaxed), w, h);
        // Calling `onSize` tells the plugin the size it just asked for. When the request comes from
        // inside the plugin's own GUI callback (its event handler), that call would re-enter its UI
        // from within itself, which deadlocks a Wine-hosted editor, so it is skipped: the plugin
        // already knows the size it asked for.
        #[cfg(target_os = "linux")]
        let skip_on_size = in_plugin_callback();
        #[cfg(not(target_os = "linux"))]
        let skip_on_size = false;
        if !skip_on_size && let Some(v) = unsafe { vst3::ComRef::from_raw(view) } {
            // SAFETY: valid view; `r` is the plugin's own writable rect.
            unsafe { v.onSize(r) };
        }
        sb::kResultOk
    }
}

/// Most program lists we read from one plugin (a guarded envelope, not a prediction).
const MAX_PROGRAM_LISTS: i32 = 64;
/// Most programs read from one list (hostile input: a plugin may claim millions).
const MAX_PROGRAMS: i32 = 8192;

/// Width and height of a view rect, clamped to a sane window size.
fn rect_size(r: &sb::ViewRect) -> (f64, f64) {
    let w = r.right.saturating_sub(r.left).clamp(1, 16_384);
    let h = r.bottom.saturating_sub(r.top).clamp(1, 16_384);
    (f64::from(w), f64::from(h))
}

#[derive(Default)]
struct EditorInner {
    /// `None` once the instance is gone.
    component: Option<ComPtr<sv::IComponent>>,
    /// `None` once the instance is gone, or without an edit controller.
    controller: Option<ComPtr<sv::IEditController>>,
    view: Option<ComPtr<sb::IPlugView>>,
    frame: Option<ComWrapper<PlugFrame>>,
    window: Option<window::HostWindow>,
    /// The run-loop load last reported to the log, so the message appears when it changes.
    run_loop_seen: (usize, usize),
    /// How many more frames keep fitting the plugin's window to the editor's size (a plugin window
    /// created late would otherwise stay at the virtual screen size).
    fit_passes: u8,
    /// The view's run loop (Linux), pumped from `idle` outside this lock.
    #[cfg(target_os = "linux")]
    run_loop: Option<std::sync::Arc<RunLoop>>,
    /// Whether the view may register its event pump with the host's run loop (off for bridged
    /// plugins, see [`PlugFrame::run_loop_enabled`]).
    run_loop_enabled: bool,
    /// True once the host window has been seen on screen. Before that a "not visible" reading is
    /// just the window manager not having mapped it yet, and tearing the editor down there used to
    /// call `IPlugView::removed()` while the plugin was still starting up.
    seen_visible: bool,
    /// Set while a worker thread is still closing this instance's view (see `EditorLink::teardown`).
    /// Its `IPlugView::removed()` runs inside the plugin, so the plugin objects must not be released
    /// until it returns — releasing them first is a use-after-free in the plugin's own code, which
    /// takes the whole app down without a panic.
    closing: Option<std::sync::Arc<Closing>>,
}

/// The handshake with the worker that closes an editor view (Linux, see `EditorLink::teardown`).
#[derive(Default)]
struct Closing {
    done: std::sync::Mutex<bool>,
    finished: std::sync::Condvar,
}

impl Closing {
    /// Marks the view as detached and wakes whoever is waiting before the plugin goes.
    fn finish(&self) {
        if let Ok(mut done) = self.done.lock() {
            *done = true;
        }
        self.finished.notify_all();
    }

    /// Waits up to `limit` for the worker to finish. `true` when it is done.
    fn wait(&self, limit: std::time::Duration) -> bool {
        let Ok(done) = self.done.lock() else {
            return false;
        };
        let (done, _) = self.finished.wait_timeout_while(done, limit, |d| !*d).unwrap_or_else(PoisonError::into_inner);
        *done
    }
}

/// The link between an instance and its editor and state handles. The instance may process on
/// the audio thread while the editor (the edit controller's `IPlugView`) is driven and the state
/// read from the main thread, which is VST3's own threading model (controller, views and
/// `getState` on the UI thread). All such calls happen under this lock, and the instance closes
/// the view and drops the component and controller references under the same lock before it
/// terminates, so a handle never calls into a dead plugin.
#[derive(Default)]
pub(crate) struct EditorLink {
    inner: Mutex<EditorInner>,
    /// True while this instance is inside `IAudioProcessor::process` on the audio thread.
    ///
    /// The GUI run loop is *not* pumped during that time. A plugin call from the UI thread queues
    /// behind the audio call in the bridge, and the host cannot finish the audio call until the GUI
    /// event it is waiting for has been delivered — a deadlock that freezes the editor and the
    /// audio at the same time. Deferring the pump by one frame breaks that cycle; the GUI events
    /// are delivered on the next idle instead.
    audio_busy: std::sync::atomic::AtomicBool,
}

/// Marks an instance as busy in `process` for as long as it lives (see [`EditorLink::audio_busy`]).
pub(crate) struct AudioBusyGuard(std::sync::Arc<EditorLink>);

impl AudioBusyGuard {
    fn new(link: std::sync::Arc<EditorLink>) -> Self {
        link.audio_busy.store(true, Ordering::Release);
        AudioBusyGuard(link)
    }
}

impl Drop for AudioBusyGuard {
    fn drop(&mut self) {
        self.0.audio_busy.store(false, Ordering::Release);
    }
}

impl EditorLink {
    /// Lists the plugin's programs through the link's controller: callable from the UI thread while
    /// the instance belongs to the audio side (same lock the editor and state handles take).
    pub fn program_lists(&self) -> Vec<soundcraft_dsp::ProgramList> {
        Instance::program_lists_of(&self.lock())
    }

    /// True while the instance is processing audio.
    pub fn is_audio_busy(&self) -> bool {
        self.audio_busy.load(Ordering::Acquire)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EditorInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn has_controller(&self) -> bool {
        self.lock().controller.is_some()
    }

    fn create_view(g: &EditorInner) -> Result<ComPtr<sb::IPlugView>, String> {
        let c = g.controller.as_ref().ok_or_else(|| "the plugin is gone".to_string())?;
        // SAFETY: valid controller; the name is a static NUL-terminated string.
        let raw = unsafe { c.createView(c"editor".as_ptr()) };
        // SAFETY: a non-null view comes with one reference owned by the caller.
        unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| "this plugin has no editor".to_string())
    }

    /// Creates the view, checks it supports this platform's window type and reads its size,
    /// without attaching it anywhere. For diagnostics and tests.
    pub fn probe(&self) -> Result<(u32, u32), String> {
        let g = self.lock();
        log::debug!("editor: creating the view");
        let view = EditorLink::create_view(&g)?;
        log::debug!("editor: view created");
        let platform = window::PLATFORM_TYPE.ok_or_else(|| "plugin editors are not supported on this platform".to_string())?;
        // SAFETY: valid view; `platform` is a static NUL-terminated string.
        if unsafe { view.isPlatformTypeSupported(platform) } != sb::kResultTrue {
            return Err("the editor does not support this platform's windows".into());
        }
        let mut r = sb::ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: valid view; `r` is writable.
        if !ok(unsafe { view.getSize(&mut r) }) {
            return Err("the editor reports no size".into());
        }
        let (w, h) = rect_size(&r);
        Ok((w as u32, h as u32))
    }

    /// Opens (or raises) the editor in a host window. Main thread only.
    pub fn open(&self, title: &str) -> Result<(), String> {
        if !window::on_main_thread() {
            return Err("plugin editors must be opened on the main thread".into());
        }
        let platform = window::PLATFORM_TYPE.ok_or_else(|| "VST3 editors are not hosted on this platform yet".to_string())?;
        let mut g = self.lock();
        if let Some(w) = &g.window {
            if w.is_visible() {
                w.raise();
                return Ok(());
            }
            // The user closed the window: start over.
            EditorLink::teardown(&mut g);
        }
        let view = EditorLink::create_view(&g)?;
        // SAFETY: valid view; `platform` is a static NUL-terminated string.
        if unsafe { view.isPlatformTypeSupported(platform) } != sb::kResultTrue {
            return Err("the editor does not support this platform's windows".into());
        }
        let mut r = sb::ViewRect { left: 0, top: 0, right: 400, bottom: 300 };
        // SAFETY: valid view; `r` is writable. On failure the default size stays.
        unsafe { view.getSize(&mut r) };
        let (w, h) = rect_size(&r);
        log::debug!("editor: creating the host window {w}x{h}");
        let win = window::HostWindow::new(title, w, h)?;
        log::debug!("editor: host window created");
        #[cfg(target_os = "linux")]
        let run_loop = std::sync::Arc::new(RunLoop::default());
        #[cfg(target_os = "linux")]
        let frame = ComWrapper::new(PlugFrame {
            window: std::sync::atomic::AtomicPtr::new(win.raw()),
            run_loop: std::sync::Arc::clone(&run_loop),
            run_loop_enabled: std::sync::atomic::AtomicBool::new(g.run_loop_enabled),
        });
        #[cfg(not(target_os = "linux"))]
        let frame = ComWrapper::new(PlugFrame { window: std::sync::atomic::AtomicPtr::new(win.raw()) });
        if let Some(f) = frame.as_com_ref::<sb::IPlugFrame>() {
            // SAFETY: valid view; the frame outlives the view's attachment (kept in `g.frame`
            // and detached in `teardown`).
            unsafe { view.setFrame(f.as_ptr()) };
        }
        log::debug!("editor: attaching the view");
        // SAFETY: valid view; the parent is the live content view of `win`, of the type named.
        if !ok(unsafe { view.attached(win.parent(), platform) }) {
            // SAFETY: valid view; detach our frame again.
            unsafe { view.setFrame(std::ptr::null_mut()) };
            win.close();
            return Err("the editor could not attach to its window".into());
        }
        // The host tells the view its size once it is attached, so a plugin that waits for the
        // frame (rather than resizing itself) still lays its GUI out inside our window.
        // SAFETY: valid, attached view; `size` is a writable rect for the call.
        let mut size = sb::ViewRect { left: 0, top: 0, right: w.clamp(1.0, 16_384.0) as sb::int32, bottom: h.clamp(1.0, 16_384.0) as sb::int32 };
        unsafe { view.onSize(&mut size) };
        log::debug!("editor: view attached");
        // The plugin's window exists from here on; make it the editor's size (see
        // `fit_plugin_windows`). Repeated on the first frames of `idle`, because some plugins create
        // the window a moment after attaching.
        let fitted = win.fit_plugin_windows(w.clamp(1.0, 16_384.0) as u32, h.clamp(1.0, 16_384.0) as u32);
        if fitted > 0 {
            log::info!("editor: resized {fitted} plugin window(s) to {}x{}", w as u32, h as u32);
        }
        g.fit_passes = 4;
        win.raise();
        g.view = Some(view);
        g.frame = Some(frame);
        #[cfg(target_os = "linux")]
        {
            g.run_loop = Some(run_loop);
        }
        g.window = Some(win);
        Ok(())
    }

    fn teardown(g: &mut EditorInner) {
        #[cfg(target_os = "linux")]
        {
            // Close the window first so the editor disappears immediately, then detach the view on a
            // worker: a plugin that hangs in `removed()` (seen with a bridged Pianoteq) then costs one
            // leaked thread instead of freezing the interface and the audio.
            g.seen_visible = false;
            if let Some(w) = g.window.take() {
                w.close();
            }
            if let Some(view) = g.view.take() {
                let frame = g.frame.take();
                // The instance may be dropped while this runs (a re-instantiation, a session swap):
                // `kill` waits on this before letting the plugin objects go.
                let closing = std::sync::Arc::new(Closing::default());
                g.closing = Some(std::sync::Arc::clone(&closing));
                let spawned = std::thread::Builder::new().name("vst3-editor-close".to_string()).spawn(move || {
                    // SAFETY: the view was attached to this frame; both are owned by this thread now
                    // and nothing else touches them (the window is gone).
                    unsafe {
                        view.removed();
                        view.setFrame(std::ptr::null_mut());
                    }
                    drop(frame);
                    closing.finish();
                });
                if spawned.is_err() {
                    // The closure owns the view; a failed spawn gives it back to nobody, so it is
                    // leaked on purpose rather than blocked on. Nobody will report the close, so do
                    // not leave the instance waiting for it either.
                    g.closing = None;
                    log::warn!("editor: cannot start the thread that closes the view; leaking it instead of blocking");
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            if let Some(view) = g.view.take() {
                // SAFETY: valid, attached view; main thread (callers check). `removed` before the
                // window goes, and the frame is detached before it is released.
                unsafe {
                    view.removed();
                    view.setFrame(std::ptr::null_mut());
                }
            }
            g.frame = None;
            if let Some(w) = g.window.take() {
                w.close();
            }
        }
    }

    /// Hands a host-set value to the controller before a state save (`setParamNormalized`,
    /// under the lock like [`EditorLink::save_state`]). No-op once the instance is gone.
    pub fn sync_param(&self, id: u32, n: f64) {
        if !n.is_finite() {
            return;
        }
        let g = self.lock();
        if let Some(c) = &g.controller {
            // SAFETY: valid controller (the instance clears it under this lock before it terminates).
            unsafe { c.setParamNormalized(id, n.clamp(0.0, 1.0)) };
        }
    }

    /// The component state plus the controller state, in SoundCraft's container (see
    /// `STATE_MAGIC`). `None` when the component refuses `getState` or the instance is gone.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let g = self.lock();
        let component = g.component.as_ref()?;
        // SAFETY: valid component (the instance clears it under this lock before it terminates);
        // the stream lives in `read_state` for the whole call.
        let comp = Instance::read_state(|s| unsafe { component.getState(s) })?;
        let ctrl = match &g.controller {
            // SAFETY: valid controller; as above.
            Some(c) => Instance::read_state(|s| unsafe { c.getState(s) }).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut out = Vec::with_capacity(12 + comp.len() + ctrl.len());
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&u32::try_from(comp.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&comp);
        out.extend_from_slice(&u32::try_from(ctrl.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&ctrl);
        Some(out)
    }

    /// Closes the editor. Main thread only (no-op elsewhere).
    pub fn close(&self) {
        if window::on_main_thread() {
            EditorLink::teardown(&mut self.lock());
        }
    }

    pub fn is_open(&self) -> bool {
        let g = self.lock();
        match &g.window {
            Some(w) if window::on_main_thread() => w.is_visible(),
            Some(_) => true,
            None => false,
        }
    }

    /// Main-thread housekeeping: tears the view down when the user closed its window.
    pub fn idle(&self) {
        if !window::on_main_thread() {
            return;
        }
        // The view's event pump first, *without* the lock: a plugin handler may call back into this
        // host (`resizeView`, a parameter edit) and those callbacks take the lock, which would
        // deadlock the UI thread on a mutex that cannot be re-entered.
        // Never service the GUI while the audio thread is inside `process` (see `audio_busy`): the
        // two calls would wait for each other inside the bridge and freeze both.
        #[cfg(target_os = "linux")]
        if self.is_audio_busy() {
            return;
        }
        #[cfg(target_os = "linux")]
        let run_loop = self.lock().run_loop.clone();
        #[cfg(target_os = "linux")]
        if let Some(run_loop) = run_loop {
            let (fds, timers) = run_loop.load();
            let changed = self.lock().run_loop_seen != (fds, timers);
            if changed {
                self.lock().run_loop_seen = (fds, timers);
                log::info!("editor run loop: {fds} descriptor(s), {timers} timer(s) from the plugin's GUI");
            }
            let started = std::time::Instant::now();
            run_loop.pump();
            let took = started.elapsed();
            if took > std::time::Duration::from_millis(200) {
                log::warn!("editor: the plugin's GUI event pump took {} ms", took.as_millis());
            }
        }
        let mut g = self.lock();
        #[cfg(target_os = "linux")]
        if g.fit_passes > 0 {
            g.fit_passes -= 1;
            if let Some(w) = &g.window {
                let (cw, ch) = w.client_size();
                if cw > 0 && ch > 0 {
                    w.fit_plugin_windows(cw, ch);
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(w) = &g.window {
            w.poll_events();
        }
        // The user closed the window (only meaningful once it has actually been shown).
        if let Some(w) = &g.window {
            if w.is_visible() {
                g.seen_visible = true;
            } else if g.seen_visible {
                EditorLink::teardown(&mut g);
            }
        }
    }

    /// Normalized → plain through the controller (main thread).
    pub fn to_plain(&self, id: u32, n: f64) -> Option<f64> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        // SAFETY: valid controller.
        let v = unsafe { c.normalizedParamToPlain(id, n.clamp(0.0, 1.0)) };
        v.is_finite().then_some(v)
    }

    /// Plain → normalized through the controller (main thread).
    pub fn to_normalized(&self, id: u32, plain: f64) -> Option<f64> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        if !plain.is_finite() {
            return None;
        }
        // SAFETY: valid controller.
        let v = unsafe { c.plainParamToNormalized(id, plain) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    /// Shows a value change in the editor (`setParamNormalized`, main thread).
    pub fn set_normalized(&self, id: u32, n: f64) {
        if !window::on_main_thread() || !n.is_finite() {
            return;
        }
        let g = self.lock();
        if let Some(c) = &g.controller {
            // SAFETY: valid controller, main thread.
            unsafe { c.setParamNormalized(id, n.clamp(0.0, 1.0)) };
        }
    }

    /// Called by the instance before it terminates: closes the editor and cuts the handles off.
    /// Returns whether plugin objects had to be leaked.
    fn kill(&self) -> bool {
        let mut g = self.lock();
        let mut leaked = false;
        if g.view.is_some() || g.window.is_some() {
            if window::on_main_thread() {
                EditorLink::teardown(&mut g);
            } else {
                // Window calls are main-thread only; leak the view and window rather than crash.
                log::warn!("vst3: editor still open while its plugin is destroyed off the main thread");
                if let Some(v) = g.view.take() {
                    std::mem::forget(v);
                    leaked = true;
                }
                // `HostWindow` has no destructor: dropping it leaks its retain on purpose.
                g.window = None;
                if let Some(f) = g.frame.take() {
                    std::mem::forget(f);
                }
            }
        }
        // A worker may still be inside `IPlugView::removed()`. Releasing the controller and the
        // component before it returns is a use-after-free in the plugin (it happened when a scan
        // finished and the session's instances were rebuilt with editors open: the app vanished
        // with no panic in the log). Wait for it; if it is stuck, leak the objects instead.
        if let Some(closing) = g.closing.take()
            && !closing.wait(CLOSE_VIEW_WAIT)
        {
            log::warn!("vst3: the editor is still closing; keeping the plugin objects alive rather than releasing them under it");
            if let Some(c) = g.controller.take() {
                std::mem::forget(c);
            }
            if let Some(c) = g.component.take() {
                std::mem::forget(c);
            }
            leaked = true;
        }
        g.component = None;
        g.controller = None;
        leaked
    }
}

/// How long the instance waits for the editor-closing worker before it leaks the plugin objects.
const CLOSE_VIEW_WAIT: std::time::Duration = std::time::Duration::from_millis(1_500);

/// The platform host window for plugin views.
#[cfg(target_os = "macos")]
mod window {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSBackingStoreType, NSFloatingWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::ffi::{c_char, c_void};

    pub const PLATFORM_TYPE: Option<*const c_char> = Some(vst3::Steinberg::kPlatformTypeNSView);

    pub fn on_main_thread() -> bool {
        MainThreadMarker::new().is_some()
    }

    /// An `NSWindow` (titled, closable, floating above the app's windows) whose content view is
    /// the plugin view's parent. Holds one retain of the window; main thread only.
    pub struct HostWindow {
        window: *mut NSWindow,
    }

    // SAFETY: the pointer is only dereferenced on the main thread (every method checks
    // `MainThreadMarker`); the struct merely carries the retain between threads' locks.
    unsafe impl Send for HostWindow {}

    impl HostWindow {
        fn get(&self) -> Option<&NSWindow> {
            MainThreadMarker::new()?;
            // SAFETY: we hold a retain of this window (from `new`) until `close`.
            unsafe { self.window.as_ref() }
        }

        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            let mtm = MainThreadMarker::new().ok_or_else(|| "not on the main thread".to_string())?;
            let _app = NSApplication::sharedApplication(mtm);
            let rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(w, h));
            // Plugin editors are transient windows. In particular they must not be
            // minimizable: minimized NSWindows become persistent thumbnails in the macOS Dock,
            // which made every editor visited in a session look like another running app.
            let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
            // SAFETY: a freshly allocated window on the main thread; valid rect and style.
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false)
            };
            // SAFETY: we keep our own retain and release it in `close`, so AppKit must not
            // release the window when the user closes it.
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(&NSString::from_str(title));
            window.setLevel(NSFloatingWindowLevel);
            window.setHidesOnDeactivate(true);
            window.setExcludedFromWindowsMenu(true);
            window.setCollectionBehavior(NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle);
            window.center();
            Ok(HostWindow { window: Retained::into_raw(window) })
        }

        /// The raw window pointer (for the plug frame's resizes).
        pub fn raw(&self) -> *mut c_void {
            self.window.cast()
        }

        /// The content view (`NSView*`) the plugin view attaches to; null when unavailable.
        pub fn parent(&self) -> *mut c_void {
            self.get().and_then(|w| w.contentView()).map_or(std::ptr::null_mut(), |v| Retained::as_ptr(&v).cast_mut().cast())
        }

        pub fn raise(&self) {
            if let Some(w) = self.get() {
                w.makeKeyAndOrderFront(None);
            }
        }

        pub fn is_visible(&self) -> bool {
            self.get().is_some_and(|w| w.isVisible())
        }

        pub fn close(self) {
            if let Some(w) = self.get() {
                w.orderOut(None);
                w.close();
            }
            if MainThreadMarker::new().is_some() {
                // SAFETY: balances the retain taken in `new`; nothing uses the pointer after this.
                drop(unsafe { Retained::from_raw(self.window) });
            }
        }
    }

    /// Resizes a host window's content (`window` from [`HostWindow::raw`]); main thread only.
    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        // SAFETY: null or a window the editor link keeps retained while the frame is attached.
        if let Some(win) = unsafe { window.cast::<NSWindow>().as_ref() } {
            win.setContentSize(NSSize::new(w, h));
        }
    }
}

/// The Windows host window: a top-level window owned by the app's active window (so it stays
/// above it and has no taskbar button) whose client area is the plugin view's `HWND` parent.
/// Closing it only hides it; the editor link notices in `idle`, detaches the view, then destroys
/// the window. Messages are dispatched by the app's own event loop on the main thread.
#[cfg(windows)]
mod window {
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GWL_STYLE,
        GetWindowLongPtrW, IDC_ARROW, IsWindow, IsWindowVisible, LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_OVERLAPPED, WS_SYSMENU,
    };

    pub const PLATFORM_TYPE: Option<*const c_char> = Some(vst3::Steinberg::kPlatformTypeHWND);

    /// Rust names the process's first thread "main"; the app runs its UI (and message loop) there.
    pub fn on_main_thread() -> bool {
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
                let name = wide("SoundCraftVst3Editor");
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
                return Err("not on the main thread".into());
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

        /// The window handle (for the plug frame's resizes).
        pub fn raw(&self) -> *mut c_void {
            self.hwnd
        }

        /// The `HWND` the plugin view creates its child window in.
        pub fn parent(&self) -> *mut c_void {
            self.live().unwrap_or(std::ptr::null_mut())
        }

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
                // SAFETY: our own live window; the view was detached before (see `teardown`).
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

#[cfg(target_os = "linux")]
mod window {
    use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};
    use std::sync::atomic::{AtomicBool, Ordering};

    pub const PLATFORM_TYPE: Option<*const c_char> = Some(vst3::Steinberg::kPlatformTypeX11EmbedWindowID);

    type Display = c_void;

    #[repr(C)]
    struct XClassHint {
        res_name: *mut c_char,
        res_class: *mut c_char,
    }

    #[repr(C, align(8))]
    pub struct XEventBuf([u8; 256]);

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

    unsafe extern "system" fn ignore_x_error(_display: *mut Display, _event: *mut c_void) -> c_int {
        0
    }

    fn install_error_handler() {
        unsafe {
            x_set_error_handler(Some(ignore_x_error));
        }
    }

    const CLIENT_MESSAGE: c_int = 33;
    const DESTROY_NOTIFY: c_int = 17;
    const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

    #[link(name = "X11")]
    unsafe extern "C" {
        #[link_name = "XOpenDisplay"]
        fn x_open_display(name: *const c_char) -> *mut Display;
        #[link_name = "XCloseDisplay"]
        fn x_close_display(display: *mut Display) -> c_int;
        #[link_name = "XDefaultRootWindow"]
        fn x_default_root_window(display: *mut Display) -> c_ulong;
        #[link_name = "XCreateSimpleWindow"]
        fn x_create_simple_window(
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
        #[link_name = "XStoreName"]
        fn x_store_name(display: *mut Display, window: c_ulong, name: *const c_char) -> c_int;
        #[link_name = "XInternAtom"]
        fn x_intern_atom(display: *mut Display, name: *const c_char, only_if_exists: c_int) -> c_ulong;
        #[link_name = "XSetWMProtocols"]
        fn x_set_wm_protocols(display: *mut Display, window: c_ulong, protocols: *mut c_ulong, count: c_int) -> c_int;
        #[link_name = "XSetClassHint"]
        fn x_set_class_hint(display: *mut Display, window: c_ulong, hint: *mut XClassHint) -> c_int;
        #[link_name = "XSelectInput"]
        fn x_select_input(display: *mut Display, window: c_ulong, mask: c_long) -> c_int;
        #[link_name = "XUnmapWindow"]
        fn x_unmap_window(display: *mut Display, window: c_ulong) -> c_int;
        #[link_name = "XDestroyWindow"]
        fn x_destroy_window(display: *mut Display, window: c_ulong) -> c_int;
        #[link_name = "XResizeWindow"]
        fn x_resize_window(display: *mut Display, window: c_ulong, width: c_uint, height: c_uint) -> c_int;
        #[link_name = "XGetGeometry"]
        fn x_get_geometry(
            display: *mut Display,
            window: c_ulong,
            root: *mut c_ulong,
            x: *mut c_int,
            y: *mut c_int,
            width: *mut c_uint,
            height: *mut c_uint,
            border: *mut c_uint,
            depth: *mut c_uint,
        ) -> c_int;
        #[link_name = "XRaiseWindow"]
        fn x_raise_window(display: *mut Display, window: c_ulong) -> c_int;
        #[link_name = "XFlush"]
        fn x_flush(display: *mut Display) -> c_int;
        #[link_name = "XPending"]
        fn x_pending(display: *mut Display) -> c_int;
        #[link_name = "XNextEvent"]
        fn x_next_event(display: *mut Display, event: *mut XEventBuf) -> c_int;
        #[link_name = "XMapRaised"]
        fn x_map_raised(display: *mut Display, window: c_ulong) -> c_int;
        #[link_name = "XSetTransientForHint"]
        fn x_set_transient_for_hint(display: *mut Display, window: c_ulong, parent: c_ulong) -> c_int;
        #[link_name = "XQueryTree"]
        fn x_query_tree(
            display: *mut Display,
            window: c_ulong,
            root: *mut c_ulong,
            parent: *mut c_ulong,
            children: *mut *mut c_ulong,
            count: *mut c_uint,
        ) -> c_int;
        #[link_name = "XGetClassHint"]
        fn x_get_class_hint(display: *mut Display, window: c_ulong, hint: *mut XClassHint) -> c_int;
        #[link_name = "XChangeProperty"]
        fn x_change_property(
            display: *mut Display,
            window: c_ulong,
            property: c_ulong,
            type_: c_ulong,
            format: c_int,
            mode: c_int,
            data: *const c_uchar,
            nelements: c_int,
        ) -> c_int;
        #[link_name = "XFree"]
        fn x_free(data: *mut c_void) -> c_int;
        #[link_name = "XSetInputFocus"]
        fn x_set_input_focus(display: *mut Display, window: c_ulong, revert_to: c_int, time: c_ulong) -> c_int;
        #[link_name = "XSetErrorHandler"]
        fn x_set_error_handler(
            handler: Option<unsafe extern "system" fn(*mut Display, *mut c_void) -> c_int>,
        ) -> Option<unsafe extern "system" fn(*mut Display, *mut c_void) -> c_int>;
        #[link_name = "XInitThreads"]
        fn x_init_threads() -> c_int;
    }

    fn cstr(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    struct WindowInner {
        display: *mut Display,
        window: c_ulong,
        protocols_atom: c_ulong,
        delete_atom: c_ulong,
        closed: AtomicBool,
    }

    pub struct HostWindow {
        inner: Box<WindowInner>,
    }

    unsafe impl Send for HostWindow {}

    impl HostWindow {
        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            static XINIT: std::sync::Once = std::sync::Once::new();
            XINIT.call_once(|| unsafe {
                let _ = x_init_threads();
            });
            install_error_handler();
            let display = unsafe { x_open_display(std::ptr::null()) };
            if display.is_null() {
                return Err("cannot open the X11 display (is DISPLAY set? Wayland needs XWayland)".into());
            }
            unsafe {
                let root = x_default_root_window(display);
                let width = w.clamp(64.0, 16_384.0) as c_uint;
                let height = h.clamp(64.0, 16_384.0) as c_uint;
                let window = x_create_simple_window(display, root, 160, 120, width, height, 0, 0, 0);
                if window == 0 {
                    x_close_display(display);
                    return Err("cannot create the plugin window".into());
                }
                let title = cstr(title);
                x_store_name(display, window, title.as_ptr());
                let name = cstr("soundcraft-plugin");
                let class = cstr("SoundCraft");
                let mut hint = XClassHint { res_name: name.as_ptr().cast_mut(), res_class: class.as_ptr().cast_mut() };
                x_set_class_hint(display, window, &mut hint);
                let protocols_atom = x_intern_atom(display, c"WM_PROTOCOLS".as_ptr(), 0);
                let delete_atom = x_intern_atom(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
                let mut protocols = [delete_atom];
                x_set_wm_protocols(display, window, protocols.as_mut_ptr(), 1);
                x_select_input(display, window, STRUCTURE_NOTIFY_MASK);
                if let Some(parent) = find_app_window(display, window) {
                    if parent != window {
                        x_set_transient_for_hint(display, window, parent);
                    }
                }
                let wm_state = x_intern_atom(display, c"_NET_WM_STATE".as_ptr(), 0);
                let above = x_intern_atom(display, c"_NET_WM_STATE_ABOVE".as_ptr(), 0);
                let atom_type = x_intern_atom(display, c"ATOM".as_ptr(), 0);
                if wm_state != 0 && above != 0 && atom_type != 0 {
                    let data = [above as u8, 0, 0, 0, 0, 0, 0, 0];
                    x_change_property(display, window, wm_state, 4, 32, 0, data.as_ptr(), 1);
                }
                x_map_raised(display, window);
                x_raise_window(display, window);
                // Give the editor the keyboard as well as the mouse: the window is transient for
                // the app, so the WM keeps it on top; a plugin GUI that never gets focus swallows
                // typing (and some render their controls only once focused). Errors are ignored by
                // the handler installed above (a WM may refuse while the window is not mapped yet).
                const REVERT_TO_PARENT: c_int = 2;
                x_set_input_focus(display, window, REVERT_TO_PARENT, 0);
                x_flush(display);
                Ok(HostWindow { inner: Box::new(WindowInner { display, window, protocols_atom, delete_atom, closed: AtomicBool::new(false) }) })
            }
        }

        pub fn raw(&self) -> *mut c_void {
            (&*self.inner as *const WindowInner as *mut WindowInner).cast()
        }

        pub fn parent(&self) -> *mut c_void {
            self.inner.window as *mut c_void
        }

        /// This window's current width and height (`(0, 0)` when unknown).
        pub fn client_size(&self) -> (c_uint, c_uint) {
            if self.inner.closed.load(Ordering::Relaxed) {
                return (0, 0);
            }
            let (mut x, mut y, mut w, mut h, mut border, mut depth) = (0, 0, 0u32, 0u32, 0u32, 0u32);
            let mut root = 0 as c_ulong;
            // SAFETY: our own window, live as long as this handle is.
            let ok =
                unsafe { x_get_geometry(self.inner.display, self.inner.window, &mut root, &mut x, &mut y, &mut w, &mut h, &mut border, &mut depth) };
            if ok == 0 { (0, 0) } else { (w, h) }
        }

        /// Resizes the plugin's own window(s) inside this one to its client size.
        ///
        /// A bridged (Wine) plugin creates its window at the virtual screen size and keeps it: with
        /// yabridge the wrapper around it is resized to the editor size, but the plugin's window is
        /// not, so the plugin lays its GUI out for the whole screen and the editor shows a corner of
        /// it. Resizing that window from here makes Wine adopt the editor's size (it handles the
        /// resulting `ConfigureNotify` like any other window resize). Only windows bigger than this
        /// one are touched, a few levels deep, and never this window itself.
        pub fn fit_plugin_windows(&self, width: c_uint, height: c_uint) -> usize {
            if self.inner.closed.load(Ordering::Relaxed) {
                return 0;
            }
            let mut fitted = 0usize;
            let mut level: Vec<c_ulong> = vec![self.inner.window];
            for _depth in 0..3 {
                let mut next: Vec<c_ulong> = Vec::new();
                for parent in level {
                    let (mut root, mut child_parent, mut children, mut count) = (0 as c_ulong, 0 as c_ulong, std::ptr::null_mut(), 0u32);
                    let ok = unsafe { x_query_tree(self.inner.display, parent, &mut root, &mut child_parent, &mut children, &mut count) };
                    if ok == 0 || children.is_null() {
                        continue;
                    }
                    let list: &[c_ulong] = unsafe { std::slice::from_raw_parts(children, count as usize) };
                    for &child in list {
                        let (mut x, mut y, mut w, mut h, mut border, mut depth) = (0, 0, 0u32, 0u32, 0u32, 0u32);
                        let mut geo_root = 0 as c_ulong;
                        let got = unsafe {
                            x_get_geometry(self.inner.display, child, &mut geo_root, &mut x, &mut y, &mut w, &mut h, &mut border, &mut depth)
                        };
                        if got != 0 && (w > width || h > height) {
                            // SAFETY: a window in our own subtree; only its size changes.
                            unsafe { x_resize_window(self.inner.display, child, width, height) };
                            fitted += 1;
                        }
                        next.push(child);
                    }
                    unsafe { x_free(children.cast()) };
                }
                level = next;
            }
            unsafe { x_flush(self.inner.display) };
            fitted
        }

        pub fn raise(&self) {
            if self.inner.closed.load(Ordering::Relaxed) {
                return;
            }
            unsafe {
                x_raise_window(self.inner.display, self.inner.window);
                x_flush(self.inner.display);
            }
        }

        pub fn poll_events(&self) {
            if self.inner.closed.load(Ordering::Relaxed) {
                return;
            }
            unsafe {
                for _ in 0..1024 {
                    if x_pending(self.inner.display) <= 0 {
                        break;
                    }
                    let mut event = XEventBuf([0u8; 256]);
                    x_next_event(self.inner.display, &mut event);
                    let any = &*(&event as *const XEventBuf as *const XAnyEvent);
                    match any.type_ {
                        CLIENT_MESSAGE => {
                            let cm = &*(&event as *const XEventBuf as *const XClientMessageEvent);
                            let delete = cm.message_type == self.inner.protocols_atom && cm.data[0] as c_ulong == self.inner.delete_atom;
                            if delete {
                                self.inner.closed.store(true, Ordering::Relaxed);
                                x_unmap_window(self.inner.display, self.inner.window);
                            }
                        }
                        DESTROY_NOTIFY => self.inner.closed.store(true, Ordering::Relaxed),
                        _ => {}
                    }
                }
                x_flush(self.inner.display);
            }
        }

        pub fn is_visible(&self) -> bool {
            !self.inner.closed.load(Ordering::Relaxed)
        }

        pub fn close(self) {
            unsafe {
                if self.inner.window != 0 {
                    x_destroy_window(self.inner.display, self.inner.window);
                }
                x_close_display(self.inner.display);
            }
        }
    }

    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        let Some(inner) = (unsafe { (window as *const WindowInner).as_ref() }) else { return };
        if inner.closed.load(Ordering::Relaxed) {
            return;
        }
        let width = w.clamp(32.0, 16_384.0) as c_uint;
        let height = h.clamp(32.0, 16_384.0) as c_uint;
        unsafe {
            x_resize_window(inner.display, inner.window, width, height);
            x_flush(inner.display);
        }
    }

    fn find_app_window(display: *mut Display, ignore: c_ulong) -> Option<c_ulong> {
        unsafe {
            let root = x_default_root_window(display);
            let (mut r, mut p) = (0 as c_ulong, 0 as c_ulong);
            let mut children: *mut c_ulong = std::ptr::null_mut();
            let mut count: c_uint = 0;
            if x_query_tree(display, root, &mut r, &mut p, &mut children, &mut count) == 0 || children.is_null() {
                return None;
            }
            let mut found = None;
            for i in 0..count as isize {
                let win = *children.offset(i);
                if win == ignore || win == 0 {
                    continue;
                }
                let mut hint = XClassHint { res_name: std::ptr::null_mut(), res_class: std::ptr::null_mut() };
                if x_get_class_hint(display, win, &mut hint) != 0 {
                    let mut is_main = false;
                    if !hint.res_class.is_null() {
                        let class = CStr::from_ptr(hint.res_class).to_string_lossy().to_ascii_lowercase();
                        let res_name = if !hint.res_name.is_null() {
                            CStr::from_ptr(hint.res_name).to_string_lossy().to_ascii_lowercase()
                        } else {
                            String::new()
                        };
                        if class.contains("soundcraft") && !res_name.contains("plugin") {
                            is_main = true;
                        }
                    }
                    if !hint.res_name.is_null() {
                        x_free(hint.res_name.cast());
                    }
                    if !hint.res_class.is_null() {
                        x_free(hint.res_class.cast());
                    }
                    if is_main {
                        found = Some(win);
                    }
                }
            }
            x_free(children.cast());
            found
        }
    }

    pub fn on_main_thread() -> bool {
        std::thread::current().name() == Some("main")
    }
}

/// Other platforms: no editor windows yet (opening reports "unsupported").
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod window {
    use std::ffi::{c_char, c_void};

    pub const PLATFORM_TYPE: Option<*const c_char> = None;

    pub fn on_main_thread() -> bool {
        true
    }

    pub struct HostWindow;

    impl HostWindow {
        pub fn new(_title: &str, _w: f64, _h: f64) -> Result<HostWindow, String> {
            Err("VST3 editors are not hosted on this platform yet".into())
        }
        pub fn raw(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn parent(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn raise(&self) {}
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
    fn the_transport_fills_the_process_context() {
        use sv::ProcessContext_::StatesAndFlags_::*;
        // SAFETY: `ProcessContext` is plain data (numbers and small structs); all-zero is valid.
        let mut c: sv::ProcessContext = unsafe { std::mem::zeroed() };
        let t = Transport {
            playing: true,
            tempo: 90.0,
            numerator: 3,
            denominator: 4,
            samples: 96_000,
            quarters: 3.5,
            bar_start: 3.0,
            ..Transport::default()
        };
        apply_transport(&mut c, &t);
        let playing = kPlaying as u32;
        assert_eq!(c.state & playing, playing);
        assert_eq!(c.state & kRecording as u32, 0);
        assert_eq!((c.tempo, c.timeSigNumerator, c.timeSigDenominator), (90.0, 3, 4));
        assert_eq!((c.projectTimeSamples, c.projectTimeMusic, c.barPositionMusic), (96_000, 3.5, 3.0));
        apply_transport(&mut c, &Transport { playing: false, ..t });
        assert_eq!(c.state & playing, 0);
    }

    #[test]
    fn destroying_an_instance_waits_for_the_worker_that_closes_its_editor() {
        // The editor's `IPlugView::removed()` runs on a worker thread. Releasing the controller and
        // the component before it returns is a use-after-free inside the plugin, which took the app
        // down without a panic; `kill` waits for the handshake first.
        let link = EditorLink { audio_busy: std::sync::atomic::AtomicBool::new(false), inner: Mutex::new(EditorInner::default()) };
        let closing = std::sync::Arc::new(Closing::default());
        link.lock().closing = Some(std::sync::Arc::clone(&closing));
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            closing.finish();
        });
        let started = std::time::Instant::now();
        let leaked = link.kill();
        let waited = started.elapsed();
        worker.join().expect("worker");
        assert!(!leaked, "an editor that closes in time is not leaked");
        assert!(waited >= std::time::Duration::from_millis(100), "kill returned before the view was detached ({waited:?})");
        assert!(link.lock().closing.is_none(), "the handshake is consumed");
    }

    #[test]
    fn an_editor_that_never_closes_is_leaked_instead_of_released_under_the_worker() {
        let link = EditorLink { audio_busy: std::sync::atomic::AtomicBool::new(false), inner: Mutex::new(EditorInner::default()) };
        // No one ever reports the close (a plugin stuck in `removed()`).
        link.lock().closing = Some(std::sync::Arc::new(Closing::default()));
        let started = std::time::Instant::now();
        let leaked = link.kill();
        assert!(leaked, "the plugin objects are kept alive, and the caller is told why");
        assert!(started.elapsed() >= CLOSE_VIEW_WAIT, "kill gives the worker its full budget");
    }

    #[test]
    fn state_container_parses_strictly() {
        let mut blob = b"SCV3".to_vec();
        blob.extend_from_slice(&3u32.to_le_bytes());
        blob.extend_from_slice(b"abc");
        blob.extend_from_slice(&2u32.to_le_bytes());
        blob.extend_from_slice(b"xy");
        assert_eq!(split_state(&blob), Some((&b"abc"[..], &b"xy"[..])));
        assert_eq!(split_state(b"raw component"), Some((&b"raw component"[..], &[][..])), "bare component state");
        assert_eq!(split_state(b""), None);
        let mut trailing = blob.clone();
        trailing.push(0);
        assert_eq!(split_state(&trailing), None);
        assert_eq!(split_state(&blob[..blob.len() - 1]), None);
        let mut huge = b"SCV3".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(split_state(&huge), None);
        assert_eq!(split_state(b"SCV3"), None);
    }

    #[test]
    fn strings_are_bounded() {
        let mut b = [0u16; 4];
        write_c16("SoundCraft", &mut b);
        assert_eq!(read_c16(&b), "Sou");
        let full: Vec<u16> = "abcd".encode_utf16().collect();
        assert_eq!(read_c16(&full), "abcd");
        let c: Vec<c_char> = b"Fx|EQ\0junk".iter().map(|&x| x as c_char).collect();
        assert_eq!(read_c8(&c), "Fx|EQ");
        write_c16("x", &mut []);
    }

    #[test]
    fn host_objects_behave() {
        let pc = ParamChanges::new(2);
        pc.set(5, 0, 0.25);
        pc.set(5, 0, 0.5);
        pc.set(6, 0, 1.0);
        pc.set(7, 0, 1.0);
        // SAFETY: test calls through our own implementation.
        unsafe {
            assert_eq!(pc.getParameterCount(), 2, "full pool drops extra params");
            assert!(pc.getParameterData(2).is_null());
            assert!(pc.getParameterData(-1).is_null());
            let q = pc.queues[0].as_com_ref::<sv::IParamValueQueue>().unwrap();
            let (mut o, mut v) = (9, 0.0);
            assert_eq!(q.getPoint(0, &mut o, &mut v), sb::kResultOk);
            assert_eq!((o, v), (0, 0.5));
            assert_ne!(q.getPoint(1, &mut o, &mut v), sb::kResultOk);
            assert_ne!(q.getPoint(0, std::ptr::null_mut(), &mut v), sb::kResultOk);
        }
        let el = EventList::new(1);
        let mut e = NoteEvent::On { time: 5, key: 60, velocity: 1.0 }.to_raw(2);
        // SAFETY: as above.
        unsafe {
            assert_eq!(el.addEvent(&mut e), sb::kResultOk);
            assert_ne!(el.addEvent(&mut e), sb::kResultOk, "bounded");
            assert_eq!(el.getEventCount(), 1);
            let mut out: sv::Event = std::mem::zeroed();
            assert_eq!(el.getEvent(0, &mut out), sb::kResultOk);
            assert_eq!(out.sampleOffset, 3);
            assert_eq!(out.__field0.noteOn.pitch, 60);
            assert_ne!(el.getEvent(1, &mut out), sb::kResultOk);
        }
        let s = MemoryStream { inner: Mutex::new((Vec::new(), 0)) };
        let mut buf = *b"hello";
        // SAFETY: as above; buffers are valid for the given sizes.
        unsafe {
            let mut n = 0;
            assert_eq!(s.write(buf.as_mut_ptr().cast(), 5, &mut n), sb::kResultOk);
            assert_eq!(n, 5);
            let mut pos = 0i64;
            assert_eq!(s.seek(1, sb::IBStream_::IStreamSeekMode_::kIBSeekSet as i32, &mut pos), sb::kResultOk);
            let mut out = [0u8; 8];
            assert_eq!(s.read(out.as_mut_ptr().cast(), 8, &mut n), sb::kResultOk);
            assert_eq!((n, &out[..4]), (4, &b"ello"[..]));
            assert_eq!(s.seek(-100, sb::IBStream_::IStreamSeekMode_::kIBSeekEnd as i32, &mut pos), sb::kResultOk);
            assert_eq!(pos, 0);
            assert_eq!(s.write(std::ptr::null_mut(), 5, &mut n), sb::kResultFalse);
        }
        let h = HostContext::default();
        let mut name: sv::String128 = [0; 128];
        // SAFETY: as above.
        unsafe {
            assert_eq!(h.getName(&mut name), sb::kResultOk);
            assert_eq!(h.restartComponent(sv::RestartFlags_::kLatencyChanged as i32), sb::kResultOk);
        }
        assert_eq!(read_c16(&name), "SoundCraft");
        assert!(h.latency_changed.load(Ordering::Relaxed));
        assert_eq!(hex(&[0xAB; 16]), "AB".repeat(16));
    }
}
