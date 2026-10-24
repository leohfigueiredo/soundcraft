//! Realtime playback through the system audio device (cpal: CoreAudio, WASAPI, ALSA/Pulse/
//! PipeWire via ALSA, JACK, WebAudio).
//!
//! The UI thread owns a [`Player`]; the audio callback owns the [`MixEngine`] and a snapshot of
//! the session (`Arc<Session>`) that the UI swaps whenever the document changes. Commands go to
//! the callback through a channel; position and meters come back through atomics and a
//! `try_lock`ed snapshot, so the callback never blocks. When no device is available the player
//! falls back to a silent clock thread so the transport, meters and automation still run.
//!
//! Surround: the device is opened with as many output channels as it offers, up to the width of
//! the session's main format. Main channels (SMPTE/WAV order) go to device channels in order; when
//! the device has fewer channels than the main mix, the mix is folded down to stereo (ITU-R
//! BS.775) on the first two device channels.
//!
//! Plugins and threads: hosted third-party plugins (CLAP/VST3) are created, given their stored
//! state and prepared on the thread that calls [`Player::update_session`] (the UI thread), and
//! travel to the audio thread together with the session snapshot that needs them; the mix
//! engine adopts them (see `soundcraft_mix`'s crate docs). Instances the engine retires, and the
//! session snapshots the callback replaces, come back on return channels and are dropped on the UI
//! thread ([`Player::idle`]). Plugin editors are driven, and plugin states for saving read
//! ([`Player::capture_states`]), on the UI thread
//! through handles taken from each instance before it leaves ([`Player::open_editor`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod devices;
pub mod record;
pub mod rt;

pub use devices::AudioConfig;
pub use rt::Realtime;

use soundcraft_dsp::{Plugin, PluginEditor, PluginStateReader};
use soundcraft_mix::{MAX_CHANNELS, MixEngine, PreparedInstance, StripMeter, itu_stereo_fold, main_channels};
use soundcraft_model::{Session, TrackId};
use soundcraft_time::{Range, Samples};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};

const BLOCK: usize = 512;

/// Room for replaced snapshots on their way back; `update_session` empties it before each send.
const OLD_SESSIONS: usize = 16;

/// The realtime audio thread marker lives in the mix engine, which carries it into the strips it
/// renders on worker threads; the cpal callbacks here set it.
pub use soundcraft_mix::{mark_audio_thread, on_audio_thread};

/// Meter values for the UI.
#[derive(Debug, Clone, Default)]
pub struct MeterSnapshot {
    pub tracks: HashMap<TrackId, StripMeter>,
    pub main: StripMeter,
}

/// Plugin states read from the live instances: `(track, slot or INSTRUMENT_SLOT, state)`.
pub type PluginStates = Vec<(TrackId, usize, Vec<u8>)>;

/// How long a plugin may take to answer in the validation child. A bridged Windows plugin starts
/// Wine, so seconds are normal and a minute is possible; a plugin that hangs is given this long
/// before it is refused, and the wait happens off the UI thread.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Where a third-party plugin stands. Yabridge aborts its host process when a Wine plugin fails to
/// initialise, and here that host process is the editor itself, so a plugin is only loaded
/// in-process once a child process has proved it loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeState {
    /// The validation child is still running: nothing is loaded yet.
    Pending,
    /// The child created and prepared the plugin: safe to load here.
    Loadable,
    /// The child crashed, exited with an error or timed out: never load it here.
    Refused,
}

impl ProbeState {
    /// What the interface tells the user about a plugin in this state.
    pub fn describe(self) -> &'static str {
        match self {
            ProbeState::Pending => "loading…",
            ProbeState::Loadable => "ready",
            ProbeState::Refused => "did not load (the plugin crashed or timed out)",
        }
    }
}

/// Which third-party plugins passed validation, and which are being validated right now.
static PROBES: Mutex<Vec<(String, ProbeState)>> = Mutex::new(Vec::new());

/// Bumped whenever a validation finishes, so the editor re-sends the session and the newly
/// validated plugin is loaded without the user doing anything.
static PROBE_READY: AtomicU64 = AtomicU64::new(0);

/// Changes every time a plugin finishes validation ([`Player`] callers compare it each frame and
/// re-send the session when it moves).
pub fn probes_changed() -> u64 {
    PROBE_READY.load(Ordering::SeqCst)
}

/// Forgets every validation result, so plugins that were refused start over. A scan that finishes,
/// a yabridge sync or a repaired Wine prefix can make a plugin that crashed before load now, and a
/// refusal must not be a life sentence for the rest of the session.
pub fn reset_probes() {
    if let Ok(mut cache) = PROBES.lock() {
        cache.clear();
    }
    PROBE_READY.fetch_add(1, Ordering::SeqCst);
}

/// Plugin ids a session asked for that the catalogue does not have a bundle for yet. A scan that
/// was interrupted (a crashing DLL, a killed process) or an interrupted first launch leaves every
/// plugin stuck in [`ProbeState::Pending`]; the UI watches this and starts one background catalogue
/// pass so those ids get their paths back.
static AWAITING_CATALOGUE: Mutex<Vec<(String, std::time::Instant)>> = Mutex::new(Vec::new());

/// How long a request counts as still outstanding: long enough that one scan may take minutes under
/// Wine, short enough that a plugin the user removed stops asking.
const AWAITING_WINDOW: std::time::Duration = std::time::Duration::from_secs(30 * 60);

fn note_awaiting_catalogue(id: &str) {
    if let Ok(mut waiting) = AWAITING_CATALOGUE.lock() {
        let now = std::time::Instant::now();
        waiting.retain(|(_, seen)| now.duration_since(*seen) < AWAITING_WINDOW);
        if !waiting.iter().any(|(k, _)| k == id) {
            log::info!("plugin {id}: no catalogue entry yet; the plugin folders will be scanned for it");
            waiting.push((id.to_string(), now));
        }
    }
}

/// Ids that were asked for but are not in the plugin catalogue. Non-empty means a background scan
/// can still fix them ([`crate::fx_browser`] in the UI starts one); it clears as soon as the
/// catalogue knows the id.
pub fn plugins_awaiting_catalogue() -> Vec<String> {
    let Ok(waiting) = AWAITING_CATALOGUE.lock() else { return Vec::new() };
    let now = std::time::Instant::now();
    waiting.iter().filter(|(_, seen)| now.duration_since(*seen) < AWAITING_WINDOW).map(|(id, _)| id.clone()).collect()
}

/// Drops one id from [`plugins_awaiting_catalogue`] once the catalogue (or a probe) found it.
pub fn resolve_plugin(id: &str) {
    if let Ok(mut waiting) = AWAITING_CATALOGUE.lock() {
        waiting.retain(|(k, _)| k != id);
    }
}

/// The validation state of a third-party plugin, starting a child process for it the first time.
/// Never blocks: the child is waited for on a worker thread, because a bridged Windows plugin can
/// take a minute under Wine and the editor must not freeze meanwhile.
fn probe_state(id: &str) -> ProbeState {
    if let Ok(cache) = PROBES.lock()
        && let Some((_, state)) = cache.iter().find(|(k, _)| k == id)
    {
        return *state;
    }
    let Some(path) = soundcraft_vst3_host::bundle_path(id).or_else(|| soundcraft_clap_host::bundle_path(id)) else {
        // Not in the catalogue yet (the background scan may still be running, or an earlier one died
        // halfway): do not load it, but say so — the UI turns this into a background rescan of the
        // configured folders, which is what frees a plugin stuck here forever.

        // CORREÇÃO: Se o caminho não estiver disponível mas o plugin pode estar num diretório customizado,
        // verificar se ele está na lista de plugins conhecidos ou em pastas configuradas.
        let is_known_plugin = soundcraft_vst3_host::cached().unwrap_or_default()
            .iter()
            .any(|v| v.id == id)
            || soundcraft_clap_host::cached().unwrap_or_default()
                .iter()
                .any(|v| v.id == id);

        if !is_known_plugin {
            // Plugin não conhecido, mantém Pending para evitar carregar plugins desconhecidos
            note_awaiting_catalogue(id);
            return ProbeState::Pending;
        }

        // CORREÇÃO: O plugin é conhecido mas o catálogo ainda não tem o caminho.
        // Marcar como awaiting e deixar o scanner de fundo resolver.
        note_awaiting_catalogue(id);
        return ProbeState::Pending;
    };
    resolve_plugin(id);
    if let Ok(mut cache) = PROBES.lock() {
        cache.push((id.to_string(), ProbeState::Pending));
    }
    let owned = id.to_string();
    let spawned = std::thread::Builder::new().name(format!("probe-{owned}")).spawn(move || {
        let loadable = probe_child(&path, &owned);
        let state = if loadable { ProbeState::Loadable } else { ProbeState::Refused };
        if let Ok(mut cache) = PROBES.lock()
            && let Some(slot) = cache.iter_mut().find(|(k, _)| *k == owned)
        {
            slot.1 = state;
        }
        match state {
            ProbeState::Loadable => log::info!("plugin {owned}: validated as loadable"),
            _ => log::warn!("plugin {owned}: did not load in a child process (crash or timeout); it stays disabled"),
        }
        PROBE_READY.fetch_add(1, Ordering::SeqCst);
    });
    if spawned.is_err() {
        log::warn!("plugin {id}: cannot start a validation thread; it stays disabled");
        if let Ok(mut cache) = PROBES.lock()
            && let Some(slot) = cache.iter_mut().find(|(k, _)| k == id)
        {
            slot.1 = ProbeState::Refused;
        }
        PROBE_READY.fetch_add(1, Ordering::SeqCst);
        return ProbeState::Refused;
    }
    ProbeState::Pending
}

/// Runs `soundcraft --probe-plugin <bundle> <id>` and reports whether it created the plugin.
fn probe_child(path: &std::path::Path, id: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let spawned = std::process::Command::new(exe)
        .args(["--probe-plugin", &path.to_string_lossy(), id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else { return false };
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                return false;
            }
        }
        if std::time::Instant::now() >= deadline {
            log::warn!("plugin {id}: no answer after {} s; killing the validation child", PROBE_TIMEOUT.as_secs());
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A parameter edit made in a plugin's own editor: `(track, slot, param id, value)`.
pub type EditorEdit = (TrackId, usize, String, f32);

enum Cmd {
    /// A new snapshot plus the third-party instances it needs (created on the UI thread).
    Session(Arc<Session>, Vec<PreparedInstance>),
    /// Sets one parameter of a live plugin by its plugin-side name: how a console knob drives a
    /// hosted plugin that does not listen to MIDI CC.
    PluginParam {
        track: TrackId,
        slot: usize,
        label: String,
        value: f32,
    },
    /// Where the callback sends the snapshots it replaces (freeing one takes thousands of frees).
    OldSessions(SyncSender<Arc<Session>>),
    Input(Arc<record::InputRing>),
    Recording(bool),
    Play {
        from: Samples,
        end: Option<Samples>,
        looped: Option<Range>,
    },
    Stop,
    Locate(Samples),
    /// Stops every sounding note everywhere: the panic button (a stuck note from a plugin's own
    /// GUI keyboard, a controller left holding a key, a lost note-off).
    Panic,
    /// Switch a live plugin's program (preset) on the audio side.
    SelectProgram {
        track: TrackId,
        slot: usize,
        list: usize,
        program: usize,
    },
    /// Restore a state blob into a live plugin.
    PluginState {
        track: TrackId,
        slot: usize,
        data: Vec<u8>,
    },
    LiveMidi {
        track: TrackId,
        msg: [u8; 3],
    },
}

struct Shared {
    position: AtomicI64,
    playing: AtomicBool,
    /// Speed × 1000.
    speed: AtomicU32,
    meters: Mutex<MeterSnapshot>,
}

/// What stopped the audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTrouble {
    /// A block panicked; the output stays silent.
    Bug,
    /// The device went away.
    Gone,
    /// The device stopped asking for audio while playing.
    Stalled,
}

/// How the audio output is doing, for the UI.
#[derive(Default)]
struct Fault {
    /// A block panicked: output stays silent from then on.
    failed: AtomicBool,
    gone: AtomicBool,
    /// Blocks the device asked for later than its own clock expected: a dropout, which is heard as
    /// a click, a pop or a stutter. Counted from the timestamps cpal hands the callback, so it is
    /// the *device* being late, not the mix being slow.
    glitches: std::sync::atomic::AtomicU64,
    /// The device time of the last callback, for the comparison above.
    last_callback: Mutex<Option<cpal::StreamInstant>>,
    /// How many blocks the device asked for in total (for the ratio in the report).
    blocks: std::sync::atomic::AtomicU64,
    /// Blocks so far, to tell a device that stopped asking for audio.
    beats: std::sync::atomic::AtomicU64,
    /// The UI has been told.
    reported: AtomicBool,
}

impl Fault {
    /// What stopped the output, once.
    fn trouble(&self, stalled: bool) -> Option<OutputTrouble> {
        let trouble = if self.failed.load(Ordering::Relaxed) {
            OutputTrouble::Bug
        } else if self.gone.load(Ordering::Relaxed) {
            OutputTrouble::Gone
        } else if stalled {
            OutputTrouble::Stalled
        } else {
            return None;
        };
        (!self.reported.swap(true, Ordering::Relaxed)).then_some(trouble)
    }
}

/// Counts a dropout when the device's own clock moved much further than one block: a late block is
/// heard as a click. `expected` is one block at the device rate.
fn note_callback(fault: &Fault, callback: cpal::StreamInstant, expected: std::time::Duration) {
    fault.blocks.fetch_add(1, Ordering::Relaxed);
    // Never wait for the lock: a contended callback simply skips the measurement.
    let Ok(mut last) = fault.last_callback.try_lock() else { return };
    if let Some(previous) = last.as_ref().and_then(|p| callback.duration_since(p))
        && previous > expected.mul_f64(1.6) + std::time::Duration::from_millis(5)
    {
        fault.glitches.fetch_add(1, Ordering::Relaxed);
    }
    *last = Some(callback);
}

/// How long a playing output may go without a block before it counts as stalled.
#[cfg(not(target_arch = "wasm32"))]
const STALL: std::time::Duration = std::time::Duration::from_secs(2);

/// True once the block count `beats` has stood still for `STALL` while playing; `seen` is the
/// count last seen and since when.
#[cfg(not(target_arch = "wasm32"))]
fn stalled(seen: &mut Option<(u64, std::time::Instant)>, beats: u64, playing: bool) -> bool {
    match *seen {
        Some((b, since)) if playing && b == beats => since.elapsed() >= STALL,
        _ => {
            *seen = Some((beats, std::time::Instant::now()));
            false
        }
    }
}

/// Fill one block with `fill`. If it panics, the block and every later one are silent and the
/// transport stops, rather than the panic taking the app down.
fn guarded_block(fault: &Fault, shared: &Shared, out: &mut [f32], fill: impl FnOnce(&mut [f32])) {
    fault.beats.fetch_add(1, Ordering::Relaxed);
    if fault.failed.load(Ordering::Relaxed) || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(out))).is_err() {
        fault.failed.store(true, Ordering::Relaxed);
        shared.playing.store(false, Ordering::Relaxed);
        out.fill(0.0);
    }
}

/// State owned by the audio callback.
struct AudioState {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    session: Arc<Session>,
    mix: MixEngine,
    pos: Samples,
    playing: bool,
    end: Option<Samples>,
    looped: Option<Range>,
    /// Rendered frames waiting to be consumed (planar, main channels) and the fractional read index.
    pending: Vec<Vec<f32>>,
    /// Stereo fold-down gains per main channel (used when the device has fewer channels).
    fold: [(f32, f32); MAX_CHANNELS],
    pending_len: usize,
    read: f64,
    device_rate: f64,
    input: Option<Arc<record::InputRing>>,
    recording: bool,
    /// Retired plugin instances go back to the UI thread to be dropped there.
    garbage: Sender<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<SyncSender<Arc<Session>>>,
}

fn new_engine(sr: f32) -> MixEngine {
    let mut m = MixEngine::new(sr, BLOCK);
    m.set_external_instances(true);
    m.metronome = true;
    m
}

impl AudioState {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, session: Arc<Session>, device_rate: f64, garbage: Sender<Vec<Box<dyn Plugin>>>) -> Self {
        let sr = session.sample_rate.as_f64() as f32;
        let nch = main_channels(&session);
        AudioState {
            garbage,
            old_sessions: None,
            fold: fold_gains(&session),
            rx,
            shared,
            mix: new_engine(sr),
            session,
            pos: 0,
            playing: false,
            end: None,
            looped: None,
            pending: vec![vec![0.0; BLOCK]; nch],
            pending_len: 0,
            read: 0.0,
            device_rate,
            input: None,
            recording: false,
        }
    }

    fn drain(&mut self) {
        let mut changed = false;
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Input(r) => self.input = Some(r),
                Cmd::Recording(r) => self.recording = r,
                Cmd::Session(s, instances) => {
                    changed = true;
                    if (s.sample_rate.as_f64() - self.session.sample_rate.as_f64()).abs() > 0.5 {
                        let mut old = std::mem::replace(&mut self.mix, new_engine(s.sample_rate.as_f64() as f32));
                        let _ = self.garbage.send(old.drain_plugins());
                    }
                    self.mix.adopt(instances);
                    // The main format changed (a document edit, not steady state): resize.
                    let nch = main_channels(&s);
                    if self.pending.len() != nch {
                        self.pending = vec![vec![0.0; BLOCK]; nch];
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&s);
                    let old = std::mem::replace(&mut self.session, s);
                    if let Some(tx) = &self.old_sessions {
                        let _ = tx.try_send(old);
                    }
                }
                Cmd::OldSessions(tx) => self.old_sessions = Some(tx),
                Cmd::Play { from, end, looped } => {
                    self.pos = from;
                    self.end = end;
                    self.looped = looped.filter(|r| r.len() > 64);
                    self.playing = true;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.mix.reset();
                }
                Cmd::Stop => {
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    self.mix.reset();
                }
                Cmd::Locate(at) => {
                    self.pos = at;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.mix.reset();
                }
                Cmd::LiveMidi { track, msg } => {
                    self.mix.dispatch_live_midi(track, msg);
                }
                Cmd::PluginParam { track, slot, label, value } => {
                    let ok = self.mix.set_plugin_param_by_label(track, slot, &label, value);
                    log::debug!("plugin parameter `{label}` on track {} slot {slot} = {value}: {ok}", track.0);
                }
                Cmd::Panic => {
                    let stopped = self.mix.panic_all_notes();
                    log::info!("panic: {stopped} live note(s) stopped");
                }
                Cmd::SelectProgram { track, slot, list, program } => {
                    let ok = self.mix.select_plugin_program(track, slot, list, program);
                    log::info!("plugin program {program} of list {list} on track {} slot {slot}: {ok}", track.0);
                }
                Cmd::PluginState { track, slot, data } => {
                    let ok = self.mix.load_plugin_state(track, slot, &data);
                    log::info!("plugin state restored on track {} slot {slot}: {ok}", track.0);
                }
            }
        }
        if changed {
            // Place adopted instances now (not only once playback starts), so editors see them;
            // `sync` creates no third-party plugins here.
            self.mix.ensure_synced(&self.session);
        }
        self.send_garbage();
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn send_garbage(&mut self) {
        let g = self.mix.take_retired();
        if !g.is_empty() {
            let _ = self.garbage.send(g);
        }
    }

    /// Render the next block at the session rate into `pending`.
    fn render_next(&mut self, quantum: usize) {
        let mut n = quantum.clamp(1, BLOCK);
        if let Some(l) = self.looped
            && self.pos >= l.end
        {
            self.pos = l.start;
        }
        if let Some(l) = self.looped {
            n = n.min(usize::try_from((l.end - self.pos).max(1)).unwrap_or(BLOCK));
        } else if let Some(e) = self.end {
            if self.pos >= e {
                self.playing = false;
                self.end = None;
                self.shared.playing.store(false, Ordering::Relaxed);
                n = 0;
            } else {
                n = n.min(usize::try_from(e - self.pos).unwrap_or(BLOCK));
            }
        }
        if n == 0 {
            for c in &mut self.pending {
                c.iter_mut().for_each(|x| *x = 0.0);
            }
            self.pending_len = BLOCK;
            return;
        }
        if let Some(ring) = &self.input {
            let ch = ring.channels.load(Ordering::Relaxed).max(1);
            if self.mix.input.len() != ch {
                self.mix.input = vec![vec![0.0; BLOCK]; ch];
            }
            ring.pop_into(&mut self.mix.input, n, self.session.sample_rate.hz());
        }
        self.mix.monitor_only = !self.playing;
        self.mix.recording = self.recording;
        self.mix.render(&self.session, self.pos, n, &mut self.pending);
        self.send_garbage();
        self.pending_len = n;
        if let Ok(mut m) = self.shared.meters.try_lock() {
            m.main.copy_from(&self.mix.main_meter);
            for (k, v) in &self.mix.meters {
                match m.tracks.get_mut(k) {
                    Some(d) => d.copy_from(v),
                    None => {
                        m.tracks.insert(*k, v.clone());
                    }
                }
            }
        }
        if self.playing {
            self.pos += n as i64;
            self.shared.position.store(self.pos, Ordering::Relaxed);
        }
    }

    /// Fill an interleaved device buffer.
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let live_held = self.mix.has_live_held();
        // A live message waiting for the next block counts as "still running": the transport may be
        // stopped, but a note-off sitting in a plugin's queue has to be delivered or the note drones.
        let pending = self.mix.has_live_pending();
        let monitoring = (self.input.is_some() && self.session.tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm))
            || live_held
            || pending
            || self.mix.has_active_plugins(&self.session);
        if !self.playing && !monitoring {
            out.iter_mut().for_each(|x| *x = 0.0);
            if let Ok(mut m) = self.shared.meters.try_lock() {
                m.main = StripMeter::default();
                m.tracks.clear();
            }
            return;
        }
        let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
        let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.1, 4.0);
        let callback_frames = out.len() / channels;
        let quantum = ((callback_frames as f64 * step).ceil() as usize).clamp(1, BLOCK);
        for frame in out.chunks_mut(channels) {
            while self.read >= self.pending_len as f64 {
                self.read -= self.pending_len as f64;
                if !self.playing && !monitoring {
                    break;
                }
                self.render_next(quantum);
                if self.pending_len == 0 {
                    break;
                }
            }
            let i0 = self.read.floor() as usize;
            let fr = (self.read - i0 as f64) as f32;
            let sounding = self.playing || monitoring;
            let nch = self.pending.len();
            let sample = |c: usize| {
                let ch = self.pending.get(c);
                let a = ch.and_then(|v| v.get(i0)).copied().unwrap_or(0.0);
                let b = ch.and_then(|v| v.get(i0 + 1)).copied().unwrap_or(a);
                a + (b - a) * fr
            };
            if nch > 2 && channels < nch {
                // Fewer device channels than the main mix: ITU fold-down to stereo.
                let (mut l, mut r) = (0.0f32, 0.0f32);
                if sounding {
                    for (c, (gl, gr)) in self.fold.iter().enumerate().take(nch) {
                        let v = sample(c);
                        l += v * gl;
                        r += v * gr;
                    }
                }
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = match c {
                        0 => l.clamp(-1.0, 1.0),
                        1 => r.clamp(-1.0, 1.0),
                        _ => 0.0,
                    };
                }
            } else {
                for (c, o) in frame.iter_mut().enumerate() {
                    // Stereo mixes on a mono device play the left channel (as before).
                    *o = if c < nch.max(1) && sounding { sample(c).clamp(-1.0, 1.0) } else { 0.0 };
                }
            }
            self.read += step;
        }
    }
}

/// UI-thread bookkeeping of the third-party instances the audio thread holds.
struct Side {
    /// What was last created for each slot: (plugin id, channels, created ok).
    shadow: HashMap<(TrackId, usize), (String, usize, bool)>,
    /// Session sample rate the instances were prepared for.
    rate: f32,
    /// Editor handles of the live instances.
    editors: HashMap<(TrackId, usize), Box<dyn PluginEditor>>,
    /// State readers of the live instances.
    states: BTreeMap<(TrackId, usize), Box<dyn PluginStateReader>>,
    /// Program-list readers of the live instances (what a console lists as presets).
    programs: BTreeMap<(TrackId, usize), Box<dyn soundcraft_dsp::PluginProgramReader>>,
    /// Instances that refused their stored state: saves keep that state until the plugin's editor
    /// is opened.
    refused: HashSet<(TrackId, usize)>,
    /// Third-party plugins the audio side has not loaded, and why (validation still running, or
    /// refused): the FX list shows this instead of an empty slot with no explanation.
    unavailable: HashMap<(TrackId, usize), (String, ProbeState)>,
    garbage: Receiver<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<Receiver<Arc<Session>>>,
}

impl Side {
    fn new(rate: f32, garbage: Receiver<Vec<Box<dyn Plugin>>>) -> Side {
        Side {
            shadow: HashMap::new(),
            rate,
            editors: HashMap::new(),
            states: BTreeMap::new(),
            programs: BTreeMap::new(),
            refused: HashSet::new(),
            unavailable: HashMap::new(),
            garbage,
            old_sessions: None,
        }
    }

    /// Drops retired instances (closing their editors first, inside the plugin's drop) and
    /// replaced session snapshots.
    fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.try_recv() {
            for p in g {
                log::debug!("plugin instance released: {}", p.info().name);
                drop(p);
            }
        }
        if let Some(rx) = &self.old_sessions {
            rx.try_iter().for_each(drop);
        }
    }

    fn return_old_sessions(&mut self, tx: &Sender<Cmd>) {
        let (otx, orx) = sync_channel(OLD_SESSIONS);
        self.old_sessions = Some(orx);
        let _ = tx.send(Cmd::OldSessions(otx));
    }

    fn forget(&mut self, key: (TrackId, usize)) {
        self.shadow.remove(&key);
        self.states.remove(&key);
        self.programs.remove(&key);
        self.refused.remove(&key);
        self.unavailable.remove(&key);
        if let Some(mut e) = self.editors.remove(&key) {
            e.close();
        }
    }

    /// Drops the "not loaded" bookkeeping of every slot that has no instance, keeping the live ones,
    /// so the next `prepare` probes those plugins again. Returns how many slots were opened up.
    fn retry_unloaded(&mut self) -> usize {
        let mut keys: Vec<(TrackId, usize)> =
            self.shadow.iter().filter(|(_, (_, _, created))| !*created).map(|(k, _)| *k).chain(self.unavailable.keys().copied()).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in &keys {
            if self.shadow.get(key).is_some_and(|(_, _, created)| *created) {
                continue;
            }
            self.shadow.remove(key);
            self.unavailable.remove(key);
        }
        keys.len()
    }
}

/// The transport and audio device.
pub struct Player {
    shared: Arc<Shared>,
    fault: Arc<Fault>,
    /// The block count last seen while playing, and since when.
    #[cfg(not(target_arch = "wasm32"))]
    watch: Mutex<Option<(u64, std::time::Instant)>>,
    tx: Sender<Cmd>,
    side: Mutex<Side>,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub device_rate: u32,
    /// Output channels the device was opened with (0 for the silent clock).
    pub device_channels: u16,
    /// The device buffer in frames when one was chosen (`None`: the device default).
    pub device_buffer: Option<u32>,
    /// True when no audio device could be opened (silent clock).
    pub silent: bool,
    /// How the output callback's thread is scheduled (see [`rt`]).
    realtime: Arc<rt::RtStatus>,
    /// The track live MIDI from other threads plays ([`LiveMidiSink`]); [`NO_TARGET`] for none.
    live_target: Arc<AtomicU64>,
}

/// [`Player::set_live_target`]'s "no track".
const NO_TARGET: u64 = u64::MAX;

/// Plays live MIDI on the player's current live target from any thread, straight to the audio
/// callback: a hardware keyboard's notes must not wait for the next UI frame (an idle window
/// repaints about once a second). Cheap to clone; sending never blocks.
#[derive(Clone)]
pub struct LiveMidiSink {
    tx: Sender<Cmd>,
    target: Arc<AtomicU64>,
}

impl LiveMidiSink {
    /// Queues `msg` for the target track; false when there is no target or the player is gone
    /// (the caller then falls back to the UI path).
    pub fn send(&self, msg: [u8; 3]) -> bool {
        let t = self.target.load(Ordering::Acquire);
        t != NO_TARGET && self.tx.send(Cmd::LiveMidi { track: TrackId(t), msg }).is_ok()
    }

    /// True when both handles feed the same player.
    pub fn same_player(&self, other: &LiveMidiSink) -> bool {
        Arc::ptr_eq(&self.target, &other.target)
    }
}

impl std::fmt::Debug for LiveMidiSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveMidiSink").field("target", &self.target.load(Ordering::Relaxed)).finish()
    }
}

impl Player {
    /// Open the default output device. Never fails: without a device it runs a silent clock.
    pub fn new(session: Arc<Session>) -> Player {
        Player::with_config(session, &AudioConfig::default())
    }

    /// Open the output device `cfg` chooses (system, device, rate, buffer size), falling back to the
    /// defaults for whatever is unavailable. Never fails: without a device it runs a silent clock.
    pub fn with_config(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let p = Player::open(Arc::clone(&session), cfg);
        p.side().return_old_sessions(&p.tx);
        // Hand the initial session's third-party plugins over.
        p.update_session(session);
        p
    }

    /// How the audio thread is scheduled: realtime, raised by RTKit, normal, or owned by the audio
    /// system. `Unknown` until the first callback ran.
    pub fn realtime(&self) -> Realtime {
        self.realtime.get()
    }

    /// What stopped the audio output, once: a bug in the engine, or the device went away or
    /// stopped asking for audio while playing.
    pub fn trouble(&self) -> Option<OutputTrouble> {
        #[cfg(not(target_arch = "wasm32"))]
        let stalled = {
            let playing = self.shared.playing.load(Ordering::Relaxed) && !self.silent;
            stalled(&mut self.watch.lock().unwrap_or_else(PoisonError::into_inner), self.fault.beats.load(Ordering::Relaxed), playing)
        };
        #[cfg(target_arch = "wasm32")]
        let stalled = false;
        self.fault.trouble(stalled)
    }

    fn side(&self) -> std::sync::MutexGuard<'_, Side> {
        self.side.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let want = session.sample_rate.hz();
        let want_ch = u16::try_from(main_channels(&session)).unwrap_or(2);
        let side_rate = session.sample_rate.as_f64() as f32;
        let new_side = |garbage: Receiver<Vec<Box<dyn Plugin>>>| Mutex::new(Side::new(side_rate, garbage));
        match open_device(want, want_ch, cfg) {
            Ok((device, config, format, name)) => {
                // JACK (and pipewire-jack) call back on the server's own realtime thread.
                let server_owns_thread = cfg.system.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("jack"))
                    && cpal::available_hosts().iter().any(|h| h.name().eq_ignore_ascii_case("jack"));
                // The chosen buffer size first; if the device refuses it, its default.
                let mut attempts = vec![config.clone()];
                if config.buffer_size != cpal::BufferSize::Default {
                    attempts.push(cpal::StreamConfig { buffer_size: cpal::BufferSize::Default, ..config });
                }
                for config in attempts {
                    let rate = config.sample_rate.0;
                    let channels = usize::from(config.channels);
                    let (tx, rx) = channel();
                    let (gtx, grx) = channel();
                    let state = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&session), f64::from(rate), gtx);
                    let fault = Arc::new(Fault::default());
                    let realtime = Arc::new(rt::RtStatus::default());
                    let promoter = rt::Promoter::new(Arc::clone(&realtime), server_owns_thread);
                    use cpal::traits::StreamTrait;
                    let built = match format {
                        cpal::SampleFormat::I32 => build_output::<i32>(&device, &config, state, channels, &fault, &shared, promoter),
                        cpal::SampleFormat::I16 => build_output::<i16>(&device, &config, state, channels, &fault, &shared, promoter),
                        _ => build_output::<f32>(&device, &config, state, channels, &fault, &shared, promoter),
                    };
                    match built {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("audio stream failed to start: {e}");
                            }
                            let buffer = match config.buffer_size {
                                cpal::BufferSize::Fixed(n) => Some(n),
                                cpal::BufferSize::Default => None,
                            };
                            log::info!(
                                "audio device: {name} @ {rate} Hz, {channels} ch, {format}, buffer {}",
                                buffer.map_or("default".to_string(), |n| n.to_string())
                            );
                            return Player {
                                shared,
                                fault,
                                #[cfg(not(target_arch = "wasm32"))]
                                watch: Mutex::new(None),
                                tx,
                                side: new_side(grx),
                                _stream: Some(stream),
                                device_name: name,
                                device_rate: rate,
                                device_channels: config.channels,
                                device_buffer: buffer,
                                silent: false,
                                realtime,
                                live_target: Arc::new(AtomicU64::new(NO_TARGET)),
                            };
                        }
                        Err(e) => log::warn!("cannot open audio stream ({:?}): {e}", config.buffer_size),
                    }
                }
                log::warn!("no usable audio stream; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock(shared, tx, rx, want, new_side(grx), gtx)
            }
            Err(e) => {
                log::warn!("no audio output: {e}; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock_with(shared, tx, rx, Arc::clone(&session), new_side(grx), gtx)
            }
        }
    }

    fn silent_clock(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        rate: u32,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let s = Arc::new(Session::new("Untitled", soundcraft_time::SampleRate::new(rate).unwrap_or_default()));
        Player::silent_clock_with(shared, tx, rx, s, side, gtx)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let rate = session.sample_rate.hz();
        let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
        let fault = Arc::new(Fault::default());
        let failed = Arc::clone(&fault);
        let spawn = std::thread::Builder::new().name("soundcraft-clock".into()).spawn(move || {
            let mut buf = vec![0.0f32; 4096];
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let now = std::time::Instant::now();
                let frames = ((now - last).as_secs_f64() * state.device_rate) as usize;
                last = now;
                let n = (frames * 2).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    let shared = Arc::clone(&state.shared);
                    guarded_block(&failed, &shared, b, |b| state.fill(b, 2));
                }
                if Arc::strong_count(&state.shared) <= 1 {
                    break;
                }
            }
        });
        if let Err(e) = spawn {
            log::warn!("clock thread failed: {e}");
        }
        Player {
            shared,
            fault,
            watch: Mutex::new(None),
            tx,
            side,
            _stream: None,
            device_name: "No audio device (silent)".into(),
            device_rate: rate,
            device_channels: 0,
            device_buffer: None,
            silent: true,
            realtime: Arc::default(),
            live_target: Arc::new(AtomicU64::new(NO_TARGET)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        _rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        _gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        Player {
            shared,
            fault: Arc::default(),
            tx,
            side,
            _stream: None,
            device_name: "No audio device".into(),
            device_rate: session.sample_rate.hz(),
            device_channels: 0,
            device_buffer: None,
            silent: true,
            realtime: Arc::default(),
            live_target: Arc::new(AtomicU64::new(NO_TARGET)),
        }
    }

    /// Sends a new session snapshot to the audio thread, with freshly created (state-restored,
    /// prepared) instances of any third-party plugin it newly needs. Call on the UI thread.
    pub fn update_session(&self, s: Arc<Session>) {
        let instances = self.side().prepare(&s);
        let _ = self.tx.send(Cmd::Session(s, instances));
    }

    /// Device dropouts since the output opened: blocks the sound card asked for later than its own
    /// clock expected. A climbing number means the noise is the audio path (Bluetooth, Easy Effects,
    /// a busy CPU, a buffer too small), not the mix.
    pub fn audio_dropouts(&self) -> (u64, u64) {
        let fault = &self.fault;
        (fault.glitches.load(Ordering::Relaxed), fault.blocks.load(Ordering::Relaxed))
    }

    /// Stops every sounding note on every instrument (see [`Cmd::Panic`]).
    pub fn panic_all_notes(&self) {
        let _ = self.tx.send(Cmd::Panic);
    }

    /// The preset banks of one live plugin, read through its link (see
    /// [`soundcraft_dsp::Plugin::program_lists`]).
    pub fn plugin_programs(&self, track: TrackId, slot: usize) -> Vec<soundcraft_dsp::ProgramList> {
        let mut side = self.side();
        side.programs.get_mut(&(track, slot)).map(|r| r.programs()).unwrap_or_default()
    }

    /// Selects a program of one live plugin. Runs on the audio side, so the change also reaches the
    /// processor with the next block.
    pub fn select_plugin_program(&self, track: TrackId, slot: usize, list: usize, program: usize) {
        let _ = self.tx.send(Cmd::SelectProgram { track, slot, list, program });
    }

    /// The live plugin state of one slot (a preset blob), or `None` when it has none.
    pub fn capture_plugin_state(&self, track: TrackId, slot: usize) -> Option<Vec<u8>> {
        let mut side = self.side();
        side.states.get_mut(&(track, slot)).and_then(|r| r.save_state())
    }

    /// Restores a state blob into one live plugin: how a console applies a saved instrument preset
    /// to a plugin that cannot switch programs by itself.
    pub fn load_plugin_state(&self, track: TrackId, slot: usize, data: Vec<u8>) {
        let _ = self.tx.send(Cmd::PluginState { track, slot, data });
    }

    /// Reads the state of every live third-party plugin, for saving the session.
    pub fn capture_states(&self) -> PluginStates {
        self.side().capture_states()
    }

    /// Per-UI-frame housekeeping: drops retired instances and services plugin editors. Returns
    /// the parameter edits made in open editors since the last call.
    pub fn idle(&self) -> Vec<EditorEdit> {
        let mut side = self.side();
        side.collect_garbage();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            for (id, v) in e.idle() {
                out.push((t, slot, id, v));
            }
        }
        out
    }

    /// True when the plugin in this slot has its own editor.
    pub fn has_editor(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.contains_key(&(track, slot))
    }

    /// Opens the plugin's own editor window. Main thread only.
    pub fn open_editor(&self, track: TrackId, slot: usize) -> Result<(), String> {
        self.side().open_editor((track, slot))
    }

    /// Third-party plugins the audio side has not loaded yet, with why: `(track, slot, plugin id,
    /// state)`. The FX list reads this so a slot never stays empty without an explanation.
    pub fn plugin_load_status(&self) -> Vec<(TrackId, usize, String, ProbeState)> {
        self.side().unavailable()
    }

    /// Forgets every "this plugin did not load" verdict held by the audio side and re-sends the
    /// session, so each refused or still-pending slot is probed again. Call it after a plugin scan
    /// finished or after yabridge was synced: that is exactly when a plugin that crashed earlier can
    /// suddenly load, and without this the editor keeps the old verdict until the app restarts.
    pub fn retry_unloaded_plugins(&self, s: Arc<Session>) {
        let retried = self.side().retry_unloaded();
        if retried > 0 {
            log::info!("retrying {retried} plugin slot(s) that had not loaded");
        }
        let instances = self.side().prepare(&s);
        let _ = self.tx.send(Cmd::Session(s, instances));
    }

    /// Drives one parameter of a live plugin by its SoundCraft id (`"4"`, `"cutoff"`) or by the title
    /// in the plugin's own parameter list (`"Drive"`). Values are plain display values, the same units
    /// [`soundcraft_dsp::ParamInfo`] reports.
    ///
    /// This is how an on-screen control moves a hosted plugin that does not react to MIDI CC: the
    /// change goes straight to the instance on the audio side. When `name_or_id` is an id, an open
    /// editor of that plugin is told too so its own knobs follow (a title cannot be mirrored: only the
    /// instance maps titles to ids). Returns `false` when the player has no record of the slot yet,
    /// i.e. the plugin has not been created; the value travels to the callback either way.
    pub fn set_live_param(&self, track: TrackId, slot: usize, label: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let known = {
            let side = self.side();
            side.shadow.contains_key(&(track, slot)) || side.editors.contains_key(&(track, slot))
        };
        // Mirror the change into an open editor of that plugin so its GUI does not lag behind.
        self.editor_set_param(track, slot, label, value);
        let _ = self.tx.send(Cmd::PluginParam { track, slot, label: label.to_string(), value });
        known
    }

    pub fn close_editor(&self, track: TrackId, slot: usize) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.close();
        }
    }

    /// Closes every plugin editor window (main thread). Call before replacing the player: its
    /// instances may be dropped on the audio thread, where editor windows cannot be closed.
    pub fn close_editors(&self) {
        for e in self.side().editors.values_mut() {
            e.close();
        }
    }

    pub fn editor_open(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.get(&(track, slot)).is_some_and(|e| e.is_open())
    }

    pub fn any_editor_open(&self) -> bool {
        self.side().editors.values().any(|e| e.is_open())
    }

    /// Shows a parameter change made elsewhere in the plugin's open editor.
    pub fn editor_set_param(&self, track: TrackId, slot: usize, id: &str, value: f32) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_param(id, value);
        }
    }

    /// Start playback at `from`, stopping at `end`, or looping `looped`.
    pub fn play(&self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.shared.position.store(from, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Play { from, end, looped });
    }

    /// Feed live input (from a `record::Recorder`) into the mix for input monitoring.
    pub fn set_input(&self, ring: Arc<record::InputRing>) {
        let _ = self.tx.send(Cmd::Input(ring));
    }

    pub fn set_recording(&self, on: bool) {
        let _ = self.tx.send(Cmd::Recording(on));
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn locate(&self, at: Samples) {
        self.shared.position.store(at, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Locate(at));
    }

    /// Send a real-time live MIDI message (Note On/Off, CC) directly to a track's instrument.
    pub fn send_live_midi(&self, track: TrackId, msg: [u8; 3]) {
        let _ = self.tx.send(Cmd::LiveMidi { track, msg });
    }

    /// The track [`LiveMidiSink`]s play (the UI keeps it on the active instrument track).
    pub fn set_live_target(&self, track: Option<TrackId>) {
        // Ids come from a session counter and never reach `NO_TARGET`.
        self.live_target.store(track.map_or(NO_TARGET, |t| t.0), Ordering::Release);
    }

    /// A handle that plays live MIDI on the live target from another thread.
    pub fn live_midi_sink(&self) -> LiveMidiSink {
        LiveMidiSink { tx: self.tx.clone(), target: Arc::clone(&self.live_target) }
    }

    pub fn set_speed(&self, speed: f32) {
        let v = if speed.is_finite() { (speed.clamp(0.1, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.speed.store(v, Ordering::Relaxed);
    }

    /// Playback rate last requested, 1.0 at full speed.
    pub fn speed(&self) -> f32 {
        self.shared.speed.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn meters(&self) -> MeterSnapshot {
        self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Copy meters into reusable UI storage. Once track/channel counts are stable this performs
    /// no allocation, unlike cloning a new snapshot every video frame.
    pub fn copy_meters(&self, out: &mut MeterSnapshot) {
        let source = self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner);
        copy_meter_snapshot(&source, out);
    }
}

fn copy_meter_snapshot(source: &MeterSnapshot, out: &mut MeterSnapshot) {
    out.tracks.retain(|id, _| source.tracks.contains_key(id));
    for (id, meter) in &source.tracks {
        out.tracks.entry(*id).or_default().copy_from(meter);
    }
    out.main.copy_from(&source.main);
}

impl Side {
    fn capture_states(&mut self) -> PluginStates {
        self.states.iter_mut().filter(|(k, _)| !self.refused.contains(*k)).filter_map(|(&(t, slot), r)| Some((t, slot, r.save_state()?))).collect()
    }

    /// Third-party plugins the audio side has not loaded: `(track, slot, plugin id, state)`. The
    /// FX list shows this so a slot never sits empty without saying why.
    fn unavailable(&self) -> Vec<(TrackId, usize, String, ProbeState)> {
        self.unavailable.iter().map(|(&(t, s), (id, state))| (t, s, id.clone(), *state)).collect()
    }

    fn open_editor(&mut self, key: (TrackId, usize)) -> Result<(), String> {
        let e = self.editors.get_mut(&key).ok_or_else(|| "this plugin has no editor (or it is not loaded)".to_string())?;
        e.open()?;
        self.refused.remove(&key);
        Ok(())
    }

    /// Creates the third-party instances `s` needs that the audio thread does not have yet.
    fn prepare(&mut self, s: &Session) -> Vec<PreparedInstance> {
        self.collect_garbage();
        let sr = s.sample_rate.as_f64() as f32;
        if (sr - self.rate).abs() > 0.5 {
            // The audio thread rebuilds its engine: everything is created anew.
            let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
            for k in keys {
                self.forget(k);
            }
            self.rate = sr;
        }
        let specs = soundcraft_mix::instance_specs(s);
        let stale: Vec<(TrackId, usize)> = self.shadow.keys().filter(|k| !specs.iter().any(|x| (x.track, x.slot) == **k)).copied().collect();
        for k in stale {
            self.forget(k);
        }
        let mut out = Vec::new();
        for spec in specs {
            let key = (spec.track, spec.slot);
            // Unchanged since the last pass: an instance exists (nothing to do), or the plugin is
            // still being validated in a child process, or it was refused — it is retried only
            // once a validation finishes (`probes_changed` re-sends the session).
            let unchanged = matches!(self.shadow.get(&key), Some((id, ch, _)) if *id == spec.id && *ch == spec.ch);
            if unchanged {
                let created = self.shadow.get(&key).is_some_and(|(_, _, ok)| *ok);
                if created {
                    continue;
                }
                let state = probe_state(&spec.id);
                let refused_before = self.unavailable.get(&key).is_some_and(|(_, s)| *s == ProbeState::Refused);
                if state != ProbeState::Loadable || refused_before {
                    self.unavailable.insert(key, (spec.id.clone(), state));
                    continue;
                }
            }
            self.forget(key);
            let third_party = spec.id.starts_with("vst3:") || spec.id.starts_with("clap:") || spec.id.starts_with("au:");
            if third_party {
                // A plugin is only loaded here once a child process proved it loads: yabridge
                // aborts the host process on a Wine failure, and that host is the editor.
                let state = probe_state(&spec.id);
                if state != ProbeState::Loadable {
                    self.shadow.insert(key, (spec.id.clone(), spec.ch, false));
                    self.unavailable.insert(key, (spec.id.clone(), state));
                    continue;
                }
            }
            self.unavailable.remove(&key);
            let made = soundcraft_mix::create_instance(&spec.id, spec.state.as_deref(), sr, BLOCK, spec.ch);
            log::info!("plugin instance for {:?} slot {}: {} ({} ch, created: {})", spec.track, spec.slot, spec.id, spec.ch, made.is_some());
            let created = made.is_some();
            self.shadow.insert(key, (spec.id.clone(), spec.ch, created));
            if !created {
                self.unavailable.insert(key, (spec.id.clone(), ProbeState::Refused));
            }
            if let Some(soundcraft_mix::NewInstance { mut plugin, refused_state }) = made {
                if let Some(e) = plugin.editor() {
                    self.editors.insert(key, e);
                }
                if let Some(r) = plugin.state_reader() {
                    self.states.insert(key, r);
                }
                if let Some(r) = plugin.program_reader() {
                    self.programs.insert(key, r);
                }
                if refused_state {
                    self.refused.insert(key);
                }
                out.push(PreparedInstance { track: spec.track, slot: spec.slot, id: spec.id, ch: spec.ch, plugin });
            }
        }
        out
    }
}

/// ITU stereo fold-down gains for each main channel of a session.
fn fold_gains(s: &Session) -> [(f32, f32); MAX_CHANNELS] {
    let mut g = [(0.0, 0.0); MAX_CHANNELS];
    let f = s.main_format();
    let sp = f.speakers();
    if sp.len() == f.channels() {
        for (o, x) in g.iter_mut().zip(sp.iter()) {
            *o = itu_stereo_fold(*x);
        }
    } else {
        // Ambisonics: W only, at -3 dB per side.
        g[0] = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    }
    g
}

/// The output stream: the mix renders f32 into a scratch buffer allocated here (so the callback
/// never allocates), converted to the device's sample type.
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: AudioState,
    channels: usize,
    fault: &Arc<Fault>,
    shared: &Arc<Shared>,
    mut promoter: rt::Promoter,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let frame = channels.max(1);
    let mut scratch = vec![0.0f32; 8192 * frame];
    let chunk = scratch.len();
    let (failed, guard, gone) = (Arc::clone(fault), Arc::clone(shared), Arc::clone(fault));
    // One block at the device rate: what the gap between callbacks should be.
    let expected_block = {
        let frames = match config.buffer_size {
            cpal::BufferSize::Fixed(frames) => u64::from(frames),
            cpal::BufferSize::Default => 512,
        };
        let rate = u64::from(config.sample_rate.0.max(1));
        std::time::Duration::from_secs_f64(frames as f64 / rate as f64)
    };
    let counters = Arc::clone(fault);
    device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            mark_audio_thread();
            promoter.on_callback();
            note_callback(&counters, info.timestamp().callback, expected_block);
            for out in data.chunks_mut(chunk) {
                let Some(s) = scratch.get_mut(..out.len()) else { return };
                guarded_block(&failed, &guard, s, |d| state.fill(d, channels));
                for (o, x) in out.iter_mut().zip(s.iter()) {
                    *o = T::from_sample(*x);
                }
            }
        },
        // cpal may call this on the audio thread (on ALSA in a loop while the device stays
        // broken): marked, so the logger hands the record off instead of writing it.
        move |e: cpal::StreamError| {
            mark_audio_thread();
            if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                gone.gone.store(true, Ordering::Relaxed);
            }
            log::warn!("audio stream error: {e}");
        },
        None,
    )
}

/// Open the output device `cfg` chooses (else the default) at the chosen rate (else the session
/// rate) if possible, in the best sample format it offers (f32, else 32- or 16-bit integers),
/// with as many channels as it offers up to `want_ch` (at least stereo when available), and the
/// chosen buffer size when the device allows it.
fn open_device(want_rate: u32, want_ch: u16, cfg: &AudioConfig) -> Result<(cpal::Device, cpal::StreamConfig, cpal::SampleFormat, String), String> {
    use cpal::traits::DeviceTrait;
    let host = devices::host(cfg);
    let device = devices::device(&host, cfg.output.as_deref(), true).ok_or_else(|| "no output device".to_string())?;
    #[allow(deprecated)]
    let name = device.name().unwrap_or_else(|_| "Audio device".into());
    // Prefer a config at the chosen (else session) rate; else the device default (we resample).
    let want_ch = want_ch.max(2);
    let rates: Vec<u32> = cfg.sample_rate.into_iter().chain(Some(want_rate)).collect();
    let mut chosen: Option<(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)> = None;
    // Better: a preferred format first, then more channels.
    let better = |c: &cpal::SupportedStreamConfigRange, x: &(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)| {
        let (rc, rx) = (devices::format_rank(c.sample_format()).unwrap_or(usize::MAX), devices::format_rank(x.2).unwrap_or(usize::MAX));
        rc < rx || (rc == rx && c.channels() > x.0.channels)
    };
    if let Ok(configs) = device.supported_output_configs() {
        let configs: Vec<cpal::SupportedStreamConfigRange> = configs.collect();
        for rate in rates {
            for c in &configs {
                if devices::format_rank(c.sample_format()).is_some()
                    && c.min_sample_rate().0 <= rate
                    && c.max_sample_rate().0 >= rate
                    && c.channels() >= 2
                    && c.channels() <= want_ch
                    && chosen.as_ref().is_none_or(|x| better(c, x))
                {
                    chosen = Some((c.with_sample_rate(cpal::SampleRate(rate)).config(), *c.buffer_size(), c.sample_format()));
                }
            }
            if chosen.is_some() {
                break;
            }
        }
    }
    let (config, format) = match chosen {
        Some((mut c, buffers, format)) => {
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
        None => {
            let d = device.default_output_config().map_err(|e| e.to_string())?;
            if devices::format_rank(d.sample_format()).is_none() {
                return Err(format!("unsupported device sample format {:?}", d.sample_format()));
            }
            let buffers = *d.buffer_size();
            let format = d.sample_format();
            let mut c = d.config();
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
    };
    Ok((device, config, format, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_meter_snapshot_updates_and_drops_stale_tracks() {
        let keep = TrackId(1);
        let stale = TrackId(2);
        let mut source = MeterSnapshot::default();
        source.tracks.insert(keep, StripMeter { peak: [0.5, 0.25], peaks: vec![0.5, 0.25], gain_reduction_db: 3.0, ..StripMeter::default() });
        source.main.peaks = vec![0.75, 0.5];
        let mut out = MeterSnapshot::default();
        out.tracks.insert(stale, StripMeter::default());
        copy_meter_snapshot(&source, &mut out);
        assert_eq!(out.tracks.len(), 1);
        assert_eq!(out.tracks[&keep], source.tracks[&keep]);
        assert_eq!(out.main, source.main);
    }

    #[test]
    fn a_block_that_panics_goes_silent_and_is_reported_once() {
        let shared = Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(true),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        };
        let fault = Fault::default();
        let mut out = [0.5f32; 8];
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!((out, fault.trouble(false)), ([0.25; 8], None));
        guarded_block(&fault, &shared, &mut out, |_| std::panic::panic_any("a bug in the mix"));
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!(out, [0.0; 8]);
        assert!(!shared.playing.load(Ordering::Relaxed));
        assert_eq!((fault.trouble(false), fault.trouble(false)), (Some(OutputTrouble::Bug), None));
        assert_eq!(fault.beats.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_late_block_counts_as_a_dropout_but_a_steady_one_does_not() {
        // The device tells the callback when it thinks the block should have happened: a gap much
        // bigger than one block is a dropout (a click the user hears), a normal gap is not.
        let fault = Fault::default();
        let block = std::time::Duration::from_secs_f64(512.0 / 48_000.0);
        let at = |d: std::time::Duration| cpal::StreamInstant::new(d.as_secs() as i64, d.subsec_nanos());
        note_callback(&fault, at(block), block);
        note_callback(&fault, at(block * 2), block);
        note_callback(&fault, at(block * 3), block);
        assert_eq!(fault.glitches.load(Ordering::Relaxed), 0, "steady blocks are not glitches");
        note_callback(&fault, at(block * 3 + block.mul_f64(3.0)), block);
        assert_eq!(fault.glitches.load(Ordering::Relaxed), 1, "a three-block gap is one dropout");
        assert_eq!(fault.blocks.load(Ordering::Relaxed), 4, "every block is counted");
    }

    #[test]
    fn a_lost_or_stalled_device_is_reported_once() {
        let ago = |seen: &mut Option<(u64, std::time::Instant)>| {
            if let Some(s) = seen {
                s.1 = std::time::Instant::now().checked_sub(STALL).unwrap();
            }
        };
        let mut seen = None;
        assert!(!stalled(&mut seen, 5, true));
        ago(&mut seen);
        assert!(stalled(&mut seen, 5, true));
        assert!(!stalled(&mut seen, 6, true), "it asked again");
        ago(&mut seen);
        assert!(!stalled(&mut seen, 6, false), "stopped, so nothing to ask for");
        assert_eq!(Fault::default().trouble(true), Some(OutputTrouble::Stalled));
        let fault = Fault::default();
        fault.gone.store(true, Ordering::Relaxed);
        assert_eq!((fault.trouble(false), fault.trouble(true)), (Some(OutputTrouble::Gone), None));
    }

    #[test]
    fn a_note_off_is_flushed_even_with_the_transport_stopped() {
        // The audio side used to stop rendering as soon as nothing was held: with the transport
        // stopped, the release stayed in the instrument's queue and the note never stopped.
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(soundcraft_model::Insert::new("vst3:0000000000000000000000000000000a"));
        let session = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&session), 48_000.0, channel().0);
        let mut out = vec![0.0f32; 1024];
        // The strips exist once the session is synced and an instrument is adopted (the app does
        // both when it opens a session and the plugin instance is created).
        st.mix.sync(&session);
        let synth = soundcraft_dsp::create("subtractive_synth").expect("a built-in instrument");
        st.mix.adopt(vec![soundcraft_mix::PreparedInstance {
            track: t,
            slot: soundcraft_mix::INSTRUMENT_SLOT,
            id: "vst3:0000000000000000000000000000000a".into(),
            ch: 2,
            plugin: synth,
        }]);
        // One block of playback so the adopted instance lands in its strip, then the transport
        // stops and everything below happens stopped (the case that used to leave notes droning).
        let _ = tx.send(Cmd::Play { from: 0, end: Some(512), looped: None });
        st.fill(&mut out, 2);
        st.fill(&mut out, 2);
        st.mix.dispatch_live_midi(t, [0x90, 60, 100]);
        st.fill(&mut out, 2);
        assert!(st.mix.has_live_held(), "the note is held");
        st.mix.dispatch_live_midi(t, [0x80, 60, 0]);
        assert!(st.mix.has_live_pending(), "the release is waiting for a block");
        st.fill(&mut out, 2);
        assert!(!st.mix.has_live_pending(), "and the block delivered it");
    }

    #[test]
    fn hosted_plugins_keep_rendering_with_the_transport_stopped() {
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(soundcraft_model::Insert::new("vst3:0000000000000000000000000000000a"));
        let session = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        drop(tx);
        let mut st = AudioState::new(rx, shared, Arc::clone(&session), 48_000.0, channel().0);
        st.mix.sync(&session);
        st.mix.adopt(vec![soundcraft_mix::PreparedInstance {
            track: t,
            slot: soundcraft_mix::INSTRUMENT_SLOT,
            id: "vst3:0000000000000000000000000000000a".into(),
            ch: 2,
            plugin: soundcraft_dsp::create("subtractive_synth").expect("a built-in instrument"),
        }]);
        st.mix.ensure_synced(&session);

        st.fill(&mut vec![0.0; 1024], 2);

        assert!(st.pending_len > 0, "the stopped transport still renders hosted plugins");
    }

    #[test]
    fn audio_state_plays_and_stops_at_end() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(shared.position.load(Ordering::Relaxed) >= 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn short_device_callbacks_render_short_mix_blocks() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 64];
        st.fill(&mut out, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn surround_main_maps_to_device_channels_or_folds_down() {
        use soundcraft_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
        let mut s = soundcraft_model::Session::default();
        s.outputs[0].format = ChannelFormat::Surround51;
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Surround51, None);
        let chans: Vec<Vec<f32>> = (0..6).map(|c| vec![0.1 * (c + 1) as f32; 48_000]).collect();
        s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 48_000));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&s), 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        // An 8-channel device: channels in order, the rest silent.
        let mut out = vec![0.0f32; 8 * 256];
        st.fill(&mut out, 8);
        let f = &out[8 * 100..8 * 101];
        for c in 0..6 {
            assert!((f[c] - 0.1 * (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
        assert!(f[6] == 0.0 && f[7] == 0.0);
        // A stereo device: ITU fold-down.
        let mut out = vec![0.0f32; 2 * 256];
        st.fill(&mut out, 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let l = (0.1 + h * 0.3 + h * 0.5f32).min(1.0);
        assert!((out[200] - l).abs() < 1e-4, "{} vs {l}", out[200]);
        assert_eq!(shared.meters.lock().unwrap().main.peaks.len(), 6);
    }

    #[test]
    fn instances_arrive_with_the_session_and_leave_through_the_return_channel() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let plugin = soundcraft_dsp::create("gain").unwrap();
        tx.send(Cmd::Session(Arc::clone(&s), vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin }])).unwrap();
        let mut out = vec![0.0f32; 256];
        st.fill(&mut out, 2);
        assert!(grx.try_recv().is_err(), "nothing retired yet");
        // A session without the insert retires the instance to the return channel.
        tx.send(Cmd::Session(Arc::new(soundcraft_model::Session::default()), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
        // A sample-rate change rebuilds the engine without dropping plugins on this thread.
        tx.send(Cmd::Session(
            Arc::clone(&s),
            vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin: soundcraft_dsp::create("gain").unwrap() }],
        ))
        .unwrap();
        st.fill(&mut out, 2);
        let mut s44 = (*s).clone();
        s44.sample_rate = soundcraft_time::SampleRate::new(44_100).unwrap();
        tx.send(Cmd::Session(Arc::new(s44), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
    }

    #[test]
    fn plugin_states_are_read_without_the_audio_thread() {
        let mut side = Side::new(48_000.0, channel().1);
        let (a, b) = (TrackId(1), TrackId(2));
        side.states.insert((b, 0), Box::new(|| Some(b"b0".to_vec())));
        side.states.insert((a, soundcraft_mix::INSTRUMENT_SLOT), Box::new(|| Some(b"synth".to_vec())));
        side.states.insert((a, 3), Box::new(|| Some(b"a3".to_vec())));
        side.states.insert((a, 4), Box::new(|| None));
        let want = vec![(a, 3, b"a3".to_vec()), (a, soundcraft_mix::INSTRUMENT_SLOT, b"synth".to_vec()), (b, 0, b"b0".to_vec())];
        assert_eq!(side.capture_states(), want);
        side.forget((a, 3));
        assert_eq!(side.capture_states(), want[1..]);
    }

    struct Window;

    impl PluginEditor for Window {
        fn open(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {}
        fn is_open(&self) -> bool {
            false
        }
        fn idle(&mut self) -> Vec<(String, f32)> {
            Vec::new()
        }
    }

    #[test]
    fn a_refused_state_is_kept_until_the_editor_opens() {
        let mut side = Side::new(48_000.0, channel().1);
        let k = (TrackId(1), 0);
        side.states.insert(k, Box::new(|| Some(b"defaults".to_vec())));
        side.editors.insert(k, Box::new(Window));
        side.refused.insert(k);
        assert!(side.capture_states().is_empty());
        side.open_editor(k).unwrap();
        assert_eq!(side.capture_states(), vec![(TrackId(1), 0, b"defaults".to_vec())]);
    }

    #[test]
    fn side_creates_each_missing_instance_once() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut side = Side::new(48_000.0, channel().1);
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:does.not.exist"));
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("gain"));
        assert!(side.prepare(&s).is_empty(), "the plugin is not installed; built-ins stay with the engine");
        assert_eq!(side.shadow.get(&(t, 0)), Some(&("clap:does.not.exist".to_string(), 1, false)));
        assert!(side.prepare(&s).is_empty());
        assert_eq!(side.shadow.len(), 1, "failures are remembered, not retried every update");
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        side.prepare(&s);
        assert!(side.shadow.is_empty());
    }

    #[test]
    fn pending_probe_transitions_to_loadable_and_instantiates() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut side = Side::new(48_000.0, channel().1);
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.transition"));

        let key = (t, 0);
        side.shadow.insert(key, ("clap:test.transition".to_string(), 1, false));
        side.unavailable.insert(key, ("clap:test.transition".to_string(), ProbeState::Pending));

        if let Ok(mut cache) = PROBES.lock() {
            cache.retain(|(k, _)| k != "clap:test.transition");
            cache.push(("clap:test.transition".to_string(), ProbeState::Loadable));
        }

        let _ = side.prepare(&s);
        assert_ne!(side.unavailable.get(&key).map(|(_, st)| *st), Some(ProbeState::Pending));
    }

    #[test]
    fn replaced_sessions_are_freed_on_the_ui_thread() {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, shared, Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let mut side = Side::new(48_000.0, grx);
        side.return_old_sessions(&tx);
        let mut out = vec![0.0f32; 256];
        let mut watched = Vec::new();
        for _ in 0..3 * OLD_SESSIONS {
            let s = Arc::new(soundcraft_model::Session::default());
            watched.push(Arc::downgrade(&s));
            side.collect_garbage();
            tx.send(Cmd::Session(s, Vec::new())).unwrap();
            st.fill(&mut out, 2);
            let replaced = watched.len().checked_sub(2).and_then(|i| watched.get(i));
            assert!(replaced.is_none_or(|w| w.strong_count() == 1), "freed on the audio thread");
        }
        side.collect_garbage();
        assert_eq!(watched.iter().filter(|w| w.strong_count() > 0).count(), 1, "only the live snapshot is left");
    }

    #[test]
    fn looping_wraps() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: Some(Range::new(0, 1000)) }).unwrap();
        let mut out = vec![0.0f32; 20_000];
        st.fill(&mut out, 2);
        let p = shared.position.load(Ordering::Relaxed);
        assert!(p <= 1000, "position {p} should stay inside the loop");
        assert!(shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn locate_while_playing_drops_the_old_position_audio() {
        use soundcraft_model::{ChannelFormat, Clip, Insert, SourceAudio, SourceId, TrackKind};
        for (plugin, params) in [("compressor", &[("lookahead", 5.0), ("ratio", 1.0)][..]), ("room_reverb", &[][..])] {
            let mut s = soundcraft_model::Session::default();
            s.edit.click = false;
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            s.pool.insert(
                SourceId(5),
                Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 4800]] })),
            );
            let id = s.new_clip_id();
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 4800));
            let mut ins = Insert::new(plugin);
            for (k, v) in params {
                ins.params.insert((*k).into(), *v);
            }
            tr.mixer.inserts[0] = Some(ins);
            let shared = Arc::new(Shared {
                position: AtomicI64::new(0),
                playing: AtomicBool::new(false),
                speed: AtomicU32::new(1000),
                meters: Mutex::new(MeterSnapshot::default()),
            });
            let (tx, rx) = channel();
            let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
            tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
            let mut out = vec![0.0f32; 2 * 512];
            let mut before = 0.0f32;
            for _ in 0..9 {
                st.fill(&mut out, 2);
                before = out.iter().fold(before, |m, x| m.max(x.abs()));
            }
            assert!(before > 0.1, "{plugin}: the clip plays before the locate");
            tx.send(Cmd::Locate(1_000_000)).unwrap();
            for _ in 0..8 {
                st.fill(&mut out, 2);
                assert!(out.iter().all(|x| x.abs() < 1e-6), "{plugin}: audio from before the locate");
            }
        }
    }

    #[test]
    fn armed_tracks_are_heard_and_metered_after_playback_stops() {
        use soundcraft_model::{ChannelFormat, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
        let ring = record::InputRing::new(1, 48_000);
        tx.send(Cmd::Input(Arc::clone(&ring))).unwrap();
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(!shared.playing.load(Ordering::Relaxed));
        ring.push(&[0.5; 4096]);
        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 0.1), "the input is heard");
        assert!(shared.meters.lock().unwrap().tracks.get(&t).is_some_and(|m| m.peak[0] > 0.1), "and metered");
    }
}
