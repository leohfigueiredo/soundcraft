//! Hardware MIDI I/O for control surfaces (iCON P1-M, M-Vave SMC-Mixer, any Mackie-style MCU).
//!
//! The user interface never talks to the device itself. A worker thread owns the platform
//! MIDI connection and exchanges three-byte messages and status snapshots with the UI through
//! bounded channels; the audio thread is not involved at all.
//!
//! On Linux the backend is the ALSA sequencer (`/dev/snd/seq`): the same path USB-MIDI
//! class-compliant controllers, BlueZ-paired BLE-MIDI bridges and PipeWire's ALSA-MIDI layer
//! all expose. The worker re-enumerates the sequencer every [`RESCAN_PERIOD_MS`] and, when the
//! surface disappears (unplugged, unpaired, powered off) or reappears under the same name, it
//! unsubscribes and subscribes again by itself: hot-plug without touching the session.
//!
//! Everywhere else (macOS, Windows, WASM) the backend is a stub that reports why the surface is
//! unavailable. Adding a platform means adding a [`Link`] implementation; the worker, the
//! channel protocol and the [`Status`] reporting are shared.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the worker re-reads the platform port list (hot-plug detection).
pub const RESCAN_PERIOD_MS: u64 = 250;

/// The UI drains at most this many incoming messages per frame, so a flooding or broken device
/// cannot stall a repaint.
pub const MAX_IN_PER_FRAME: usize = 512;

/// Bounded outbound queue: LED/meter feedback is droppable, never a reason to grow memory.
pub const MAX_OUT_QUEUE: usize = 1024;

/// Connection state of the surface link, as shown to the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkState {
    /// No MIDI backend on this platform (or its device layer could not be opened).
    #[default]
    Unavailable,
    /// The backend is running but no matching surface port is present.
    Searching,
    /// A surface port is attached: input arrives and feedback is sent.
    Connected,
}

impl LinkState {
    pub const fn label(self) -> &'static str {
        match self {
            LinkState::Unavailable => "Unavailable",
            LinkState::Searching => "Searching",
            LinkState::Connected => "Connected",
        }
    }
}

/// A MIDI port the backend can see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortEntry {
    pub name: String,
    /// Backend address (`client:port` for ALSA), for diagnostics and `aconnect` troubleshooting.
    pub address: String,
    /// True when the port matches the configured device filter.
    pub matched: bool,
}

/// Where an incoming message came from: the control surface (faders/buttons) or a MIDI keyboard
/// (notes). The UI routes each kind differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Surface,
    Keyboard,
}

/// One incoming message and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InMessage {
    pub source: Source,
    pub data: [u8; 3],
    /// A keyboard message the worker already played on the live target (straight to the audio
    /// callback); the UI only lights the keys for it.
    pub played: bool,
}

/// What the worker reaches without the UI: the player's live-MIDI input (so a piano's notes
/// sound within one audio block, not on the next UI frame) and the UI's repaint waker.
#[derive(Default)]
struct Direct {
    sink: Option<soundcraft_playback::LiveMidiSink>,
    waker: Option<egui::Context>,
}

/// Plays a keyboard message on the live target; false when there is none (the UI then plays it).
fn play_direct(direct: &Mutex<Direct>, data: [u8; 3]) -> bool {
    direct.lock().ok().and_then(|d| d.sink.as_ref().map(|s| s.send(data))).unwrap_or(false)
}

/// Asks the UI for a frame (keys to light, a fader moved) instead of waiting for its idle tick.
fn wake_ui(direct: &Mutex<Direct>) {
    if let Some(ctx) = direct.lock().ok().and_then(|d| d.waker.clone()) {
        ctx.request_repaint();
    }
}

/// A status snapshot, refreshed by the worker and copied into the mixer's own state every frame.
/// It never holds channel data, so it can be logged as-is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub state: LinkState,
    /// The MIDI keyboard port the user picked (empty: none).
    pub keyboard_port: String,
    /// Connection state of that keyboard port.
    pub keyboard_state: LinkState,
    /// Bluetooth (BLE-MIDI) link state, for the controller window.
    pub ble_state: LinkState,
    pub ble_device: String,
    pub backend: String,
    pub port_name: String,
    pub filter: String,
    pub message: String,
    pub ports: Vec<PortEntry>,
    pub received: u64,
    pub sent: u64,
    pub reconnects: u32,
}

/// Commands from the UI to the worker.
enum Command {
    /// Re-read the port list now.
    Rescan,
    /// Match a different device name (the control surface).
    Filter(String),
    /// Subscribe to this MIDI keyboard port (by its exact port name); `None` stops it.
    Keyboard(Option<String>),
    /// Use BLE-MIDI over Bluetooth for the control surface (on/off).
    Ble(bool),
    /// Force the control surface port by exact name, instead of matching the device filter.
    Surface(Option<String>),
    /// Stop the worker.
    Quit,
}

/// The UI-side handle of the MIDI worker. Dropping it stops the thread.
pub struct MidiIo {
    status: Arc<Mutex<Status>>,
    out: SyncSender<[u8; 3]>,
    input: Receiver<InMessage>,
    commands: Sender<Command>,
    running: Arc<AtomicBool>,
    direct: Arc<Mutex<Direct>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for MidiIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiIo").field("status", &self.status()).finish()
    }
}

impl MidiIo {
    /// Starts the worker. Never blocks the caller and never panics: a backend that cannot open
    /// reports [`LinkState::Unavailable`] with a message and retries in the background.
    pub fn start(filter: impl Into<String>) -> MidiIo {
        let filter = filter.into();
        let status = Arc::new(Mutex::new(Status { filter: filter.clone(), backend: backend_name(), ..Status::default() }));
        let (out_tx, out_rx) = std::sync::mpsc::sync_channel::<[u8; 3]>(MAX_OUT_QUEUE);
        let (in_tx, in_rx) = std::sync::mpsc::sync_channel::<InMessage>(MAX_OUT_QUEUE);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let running = Arc::new(AtomicBool::new(true));
        let shared = Arc::clone(&status);
        let alive = Arc::clone(&running);
        let direct = Arc::new(Mutex::new(Direct::default()));
        let worker_direct = Arc::clone(&direct);
        let join = std::thread::Builder::new()
            .name("soundcraft-midi-io".to_string())
            .spawn(move || worker(filter, shared, in_tx, out_rx, cmd_rx, alive, worker_direct))
            .ok();
        MidiIo { status, out: out_tx, input: in_rx, commands: cmd_tx, running, direct, join }
    }

    /// The latest device status.
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Queues a message for the surface. Silently dropped when the queue is full, so a dead
    /// device can never grow memory.
    pub fn send(&self, message: [u8; 3]) {
        let _ = self.out.try_send(message);
    }

    /// Next input message, if any.
    pub fn recv(&self) -> Option<InMessage> {
        self.input.try_recv().ok()
    }

    /// Where keyboard notes play without waiting for the UI (`None`: through the UI only).
    pub fn set_live_sink(&self, sink: Option<soundcraft_playback::LiveMidiSink>) {
        if let Ok(mut d) = self.direct.lock() {
            d.sink = sink;
        }
    }

    /// The context to wake when input arrives.
    pub fn set_waker(&self, ctx: egui::Context) {
        if let Ok(mut d) = self.direct.lock() {
            d.waker = Some(ctx);
        }
    }

    /// Re-reads the port list (the user pressed "Rescan").
    pub fn rescan(&self) {
        let _ = self.commands.send(Command::Rescan);
    }

    /// Changes which device names are matched (the control surface).
    pub fn set_filter(&self, filter: impl Into<String>) {
        let _ = self.commands.send(Command::Filter(filter.into()));
    }

    /// Subscribes to a MIDI keyboard port by exact port name; `None` disconnects it.
    pub fn set_keyboard_port(&self, port: Option<String>) {
        let _ = self.commands.send(Command::Keyboard(port));
    }

    /// Turns the Bluetooth (BLE-MIDI) link for the control surface on or off.
    pub fn set_ble(&self, on: bool) {
        let _ = self.commands.send(Command::Ble(on));
    }

    /// Forces the control-surface input port by exact name (`None` returns to the device filter).
    pub fn set_surface_port(&self, port: Option<String>) {
        let _ = self.commands.send(Command::Surface(port));
    }
}

impl Drop for MidiIo {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.commands.send(Command::Quit);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn backend_name() -> String {
    if cfg!(target_os = "linux") { "ALSA sequencer".to_string() } else { "none (platform stub)".to_string() }
}

/// Runs the worker loop: open, poll input, flush output, rescan for hot-plug.
fn worker(
    filter: String,
    status: Arc<Mutex<Status>>,
    input: SyncSender<InMessage>,
    output: Receiver<[u8; 3]>,
    commands: Receiver<Command>,
    running: Arc<AtomicBool>,
    direct: Arc<Mutex<Direct>>,
) {
    let mut filter = filter;
    let mut keyboard: Option<String> = None;
    let mut surface_name: Option<String> = None;
    let mut link: Option<Link> = None;
    #[cfg(target_os = "linux")]
    let mut ble: Option<crate::ble_midi::BleMidi> = None;
    let mut ble_wanted = false;
    let mut last_error = String::new();
    let mut last_open_attempt = Instant::now() - Duration::from_secs(2);
    #[cfg(target_os = "linux")]
    let mut last_ble_start = Instant::now() - Duration::from_secs(2);
    let mut last_scan = Instant::now();
    // When input last arrived and when the subscriptions were last verified, so a peer that
    // dropped them (a Bluetooth bridge restarting) is noticed while the link is quiet.
    let mut last_input = Instant::now();
    let mut last_sub_check = Instant::now();
    let (mut received, mut sent, mut reconnects) = (0u64, 0u64, 0u32);
    // Set after opening: scan (and publish) immediately rather than one period later.
    let mut rescan_now = true;
    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        loop {
            match commands.try_recv() {
                Ok(Command::Rescan) => rescan_now = true,
                Ok(Command::Filter(f)) => {
                    filter = f;
                    rescan_now = true;
                }
                Ok(Command::Keyboard(k)) => {
                    keyboard = k.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
                    rescan_now = true;
                }
                Ok(Command::Ble(on)) => {
                    ble_wanted = on;
                    rescan_now = true;
                }
                Ok(Command::Surface(p)) => {
                    surface_name = p.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
                    rescan_now = true;
                }
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }

        if link.is_none() && last_open_attempt.elapsed() >= Duration::from_secs(2) {
            // Retry slowly without blocking the independent BLE-MIDI transport.
            last_open_attempt = Instant::now();
            match Link::open(&filter) {
                Ok(l) => {
                    last_error.clear();
                    link = Some(l);
                    rescan_now = true;
                }
                Err(e) => {
                    last_error = e;
                }
            }
        }
        #[cfg(target_os = "linux")]
        if ble_wanted
            && ble.as_ref().is_some_and(|b| b.status().state == LinkState::Unavailable)
            && last_ble_start.elapsed() >= Duration::from_secs(2)
        {
            // A lost BLE connection or failed initial scan leaves its worker stopped; replace it
            // so BlueZ can be retried without requiring the user to toggle Bluetooth off and on.
            ble.take();
        }
        #[cfg(target_os = "linux")]
        if ble_wanted && ble.is_none() {
            log::info!("MIDI: starting the Bluetooth BLE-MIDI link ({} filter)", filter);
            ble = Some(crate::ble_midi::BleMidi::start(filter.clone(), input.clone()));
            last_ble_start = Instant::now();
        }
        #[cfg(target_os = "linux")]
        if !ble_wanted && ble.take().is_some() {
            log::info!("MIDI: Bluetooth BLE-MIDI link stopped");
        }
        let Some(l) = link.as_mut() else {
            // ALSA may be unavailable while BLE-MIDI is still usable. Keep servicing both the
            // control feedback queue and the connection status independently of ALSA.
            for _ in 0..256 {
                let Ok(message) = output.try_recv() else { break };
                #[cfg(target_os = "linux")]
                if let Some(b) = ble.as_ref() {
                    b.send(message);
                }
            }
            #[cfg(target_os = "linux")]
            let (ble_state, ble_device) = ble
                .as_ref()
                .map(|b| {
                    let st = b.status();
                    (st.state, st.device)
                })
                .unwrap_or((LinkState::Unavailable, String::new()));
            #[cfg(not(target_os = "linux"))]
            let (ble_state, ble_device) = (LinkState::Unavailable, String::new());
            publish(
                &status,
                LinkState::Unavailable,
                "",
                &[],
                &last_error,
                received,
                sent,
                reconnects,
                &last_error,
                keyboard.as_deref().unwrap_or(""),
                LinkState::Unavailable,
                ble_state,
                &ble_device,
            );
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };

        // 1. Input: wait briefly for events, then drain them in a bounded batch.
        let _ = l.wait(Duration::from_millis(50));
        let mut arrived = false;
        for _ in 0..MAX_IN_PER_FRAME {
            let Some(mut message) = l.next_message() else { break };
            received = received.saturating_add(1);
            arrived = true;
            if message.source == Source::Keyboard {
                message.played = play_direct(&direct, message.data);
            }
            match input.try_send(message) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        if arrived {
            last_input = Instant::now();
            wake_ui(&direct);
        }

        // 2. Output feedback: droppable and bounded.
        for _ in 0..256 {
            match output.try_recv() {
                Ok(message) => {
                    if l.send(message) {
                        sent = sent.saturating_add(1);
                    } else {
                        #[cfg(target_os = "linux")]
                        if let Some(b) = ble.as_ref() {
                            b.send(message);
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // 3. Hot-plug: re-enumerate and (re)attach.
        // While the link is quiet, check that the devices we believe we listen to still deliver
        // to us: a peer that restarts keeps its address but drops the subscription, and without
        // this the surface would stay "attached" and never produce another message.
        if last_input.elapsed() >= Duration::from_secs(3) && last_sub_check.elapsed() >= Duration::from_secs(1) {
            last_sub_check = Instant::now();
            if !l.subscriptions_ok() {
                log::warn!("MIDI: the device dropped its subscription; resubscribing to the surface and keyboard");
                l.forget_devices();
                reconnects = reconnects.saturating_add(1);
                rescan_now = true;
            }
        }
        if rescan_now || last_scan.elapsed() >= Duration::from_millis(RESCAN_PERIOD_MS) {
            rescan_now = false;
            last_scan = Instant::now();
            let was_attached = !l.attached_name().is_empty();
            let (state, ports) = l.rescan(&filter, keyboard.as_deref(), surface_name.as_deref());
            if was_attached && state != LinkState::Connected {
                reconnects = reconnects.saturating_add(1);
            }
            let hint = if state == LinkState::Connected { String::new() } else { l.hint() };
            #[cfg(target_os = "linux")]
            let (ble_state, ble_device) = ble
                .as_ref()
                .map(|b| {
                    let st = b.status();
                    (st.state, st.device)
                })
                .unwrap_or((LinkState::Unavailable, String::new()));
            #[cfg(not(target_os = "linux"))]
            let (ble_state, ble_device) = (LinkState::Unavailable, String::new());
            publish(
                &status,
                state,
                l.attached_name(),
                &ports,
                &hint,
                received,
                sent,
                reconnects,
                &last_error,
                keyboard.as_deref().unwrap_or(""),
                l.keyboard_state(),
                ble_state,
                &ble_device,
            );
        }

        // 4. A device that vanished mid-read: drop the link and reopen on the next pass, which
        //    reattaches as soon as the surface returns.
        if l.lost() {
            reconnects = reconnects.saturating_add(1);
            link = None;
            last_open_attempt = Instant::now() - Duration::from_secs(2);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn publish(
    status: &Arc<Mutex<Status>>,
    state: LinkState,
    port_name: &str,
    ports: &[PortEntry],
    message: &str,
    received: u64,
    sent: u64,
    reconnects: u32,
    last_error: &str,
    keyboard_port: &str,
    keyboard_state: LinkState,
    ble_state: LinkState,
    ble_device: &str,
) {
    if let Ok(mut s) = status.lock() {
        s.state = state;
        s.keyboard_port = keyboard_port.to_string();
        s.keyboard_state = keyboard_state;
        s.ble_state = ble_state;
        s.ble_device = ble_device.to_string();
        s.port_name = port_name.to_string();
        s.ports = ports.to_vec();
        s.message = if message.is_empty() { last_error.to_string() } else { message.to_string() };
        s.received = received;
        s.sent = sent;
        s.reconnects = reconnects;
    }
}

/// Human description of one MIDI message, for the monitor window.
pub fn describe(message: [u8; 3]) -> String {
    let ch = (message[0] & 0x0F) + 1;
    match message[0] & 0xF0 {
        0x90 if message[2] > 0 => format!("Note On  {} vel {}  ch {ch}", message[1], message[2]),
        0x90 | 0x80 => format!("Note Off {}  ch {ch}", message[1]),
        0xB0 => format!("CC {} = {}  ch {ch}", message[1], message[2]),
        0xD0 => format!("Pressure {}  ch {ch}", message[1]),
        0xE0 => {
            let v = ((u16::from(message[2])) << 7) | u16::from(message[1]);
            format!("Pitch Bend {v}  ch {ch}")
        }
        _ => format!("{:02X} {:02X} {:02X}", message[0], message[1], message[2]),
    }
}

/// True when a port name looks like a Mackie-style control surface. The configured device name
/// (e.g. "SMC-Mixer") is checked first; controller-specific aliases keep renamed surfaces
/// working without claiming ordinary MIDI keyboards.
pub fn matches_filter(name: &str, filter: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let filter = filter.trim().to_ascii_lowercase();
    if !filter.is_empty() && name.contains(&filter) {
        return true;
    }
    ["smc", "m-vave", "mvave", "cuvave", "icon", "p1-m", "qcon", "mackie", "mcu"].iter().any(|alias| name.contains(alias))
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{InMessage, LinkState, PortEntry, Source, matches_filter};
    use alsa::poll::Descriptors as _;
    use alsa::seq::{Addr, ClientIter, EvCtrl, EvNote, Event, EventType, PortCap, PortIter, PortSubscribe, PortSubscribeIter, QuerySubsType, Seq};
    use alsa::{Direction, poll};
    use std::ffi::CString;
    use std::time::Duration;

    /// The "no event pending" errno from ALSA's nonblocking sequencer reads.
    const EAGAIN: i32 = 11;

    /// Our client name in the ALSA sequencer port list (and what `aconnect -l` shows).
    const CLIENT_NAME: &str = "SoundCraft Control Surface";
    const IN_PORT: &str = "SoundCraft Control In";
    const OUT_PORT: &str = "SoundCraft Control Out";

    pub type Link = AlsaLink;

    pub struct AlsaLink {
        seq: Seq,
        client: i32,
        in_port: i32,
        out_port: i32,
        attached: Option<(String, Addr, Addr)>,
        /// The MIDI keyboard (a piano, a controller) subscribed for notes.
        keyboard: Option<(String, Addr)>,
        hint: String,
        lost: bool,
    }

    /// A NUL-terminated name for the ALSA API; an embedded NUL cannot occur in our fixed names.
    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap_or_else(|_| CString::new("SoundCraft").unwrap_or_default())
    }

    impl AlsaLink {
        pub fn open(filter: &str) -> Result<AlsaLink, String> {
            // Nonblocking: the worker polls with a timeout, so it can also service the output
            // queue, the hot-plug rescan and shutdown requests.
            let seq = Seq::open(None, None, true).map_err(|e| {
                log::warn!(
                    "MIDI: cannot open the ALSA sequencer ({e}). Check /dev/snd/seq and that you are in the 'audio' group \
                     (sudo usermod -aG audio $USER, then log out and in)."
                );
                format!("ALSA sequencer unavailable: {e}")
            })?;
            log::info!("MIDI: ALSA sequencer opened");
            seq.set_client_name(&cstr(CLIENT_NAME)).map_err(|e| format!("cannot name the MIDI client: {e}"))?;
            let kind = alsa::seq::PortType::MIDI_GENERIC | alsa::seq::PortType::APPLICATION;
            // ALSA READ ports are event sources; WRITE ports receive events.
            let in_port = seq
                .create_simple_port(&cstr(IN_PORT), PortCap::WRITE | PortCap::SUBS_WRITE, kind)
                .map_err(|e| format!("cannot create the MIDI input port: {e}"))?;
            let out_port = seq
                .create_simple_port(&cstr(OUT_PORT), PortCap::READ | PortCap::SUBS_READ, kind)
                .map_err(|e| format!("cannot create the MIDI output port: {e}"))?;
            let client = seq.client_id().map_err(|e| format!("cannot read the MIDI client id: {e}"))?;
            let mut link = AlsaLink { seq, client, in_port, out_port, attached: None, keyboard: None, hint: String::new(), lost: false };
            let _ = link.rescan(filter, None, None);
            Ok(link)
        }

        pub fn attached_name(&self) -> &str {
            self.attached.as_ref().map_or("", |(name, _, _)| name.as_str())
        }

        pub fn hint(&self) -> String {
            self.hint.clone()
        }

        /// True when ALSA reported a failure that means the device went away.
        pub fn lost(&self) -> bool {
            self.lost
        }

        /// True while this client's input port is still subscribed to every device it believes it
        /// is listening to. A peer that restarts (a Bluetooth bridge, for one) silently drops the
        /// subscription and keeps its address, so the link stays attached but deaf for good.
        pub fn subscriptions_ok(&self) -> bool {
            let me = Addr { client: self.client, port: self.in_port };
            let reading = |source: Addr| PortSubscribeIter::new(&self.seq, source, QuerySubsType::READ).any(|sub| sub.get_dest() == me);
            let surface = self.attached.as_ref().is_none_or(|(_, source, _)| reading(*source));
            let keyboard = self.keyboard.as_ref().is_none_or(|(_, source)| reading(*source));
            surface && keyboard
        }

        /// Forgets every device, so the next rescan subscribes from scratch.
        pub fn forget_devices(&mut self) {
            self.attached = None;
            self.keyboard = None;
        }

        pub fn keyboard_state(&self) -> LinkState {
            if self.keyboard.is_some() { LinkState::Connected } else { LinkState::Searching }
        }

        /// Waits up to `timeout` for input, without blocking the worker's other duties.
        pub fn wait(&self, timeout: Duration) -> Result<(), String> {
            let desc = (&self.seq, Some(Direction::Capture));
            let Ok(mut fds) = desc.get() else {
                std::thread::sleep(timeout);
                return Ok(());
            };
            let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
            let _ = poll::poll(&mut fds, ms);
            Ok(())
        }

        /// Next input message, tagged with the device it came from. Skips messages that are not
        /// channel-voice (clock, sysex) within a bounded batch.
        pub fn next_message(&mut self) -> Option<InMessage> {
            let mut input = self.seq.input();
            for _ in 0..16 {
                let event = match input.event_input() {
                    Ok(e) => e,
                    // Nothing pending is the normal path (the worker polls every 50 ms).
                    Err(e) if e.errno() == EAGAIN => return None,
                    // Anything else means the sequencer (or a device behind it) went away.
                    Err(_) => {
                        self.lost = true;
                        return None;
                    }
                };
                if let Some(data) = event_to_bytes(&event) {
                    let from = event.get_source();
                    let surface = self.attached.as_ref().map(|(_, address, _)| *address);
                    let keyboard = self.keyboard.as_ref().map(|(_, address)| *address);
                    let source = classify_source(from, surface, keyboard);
                    return Some(InMessage { source, data, played: false });
                }
            }
            None
        }

        /// Sends three MIDI bytes to the attached surface. False when nothing was sent.
        pub fn send(&mut self, message: [u8; 3]) -> bool {
            let Some((_, _, dest)) = self.attached.clone() else { return false };
            let status = message[0] & 0xF0;
            let channel = message[0] & 0x0F;
            let (d1, d2) = (message[1] & 0x7F, message[2] & 0x7F);
            let mut event = match status {
                0x90 | 0x80 => {
                    let kind = if status == 0x90 { EventType::Noteon } else { EventType::Noteoff };
                    Event::new(kind, &EvNote { channel, note: d1, velocity: d2, off_velocity: 0, duration: 0 })
                }
                0xB0 => Event::new(EventType::Controller, &EvCtrl { channel, param: u32::from(d1), value: i32::from(d2) }),
                0xD0 => Event::new(EventType::Chanpress, &EvCtrl { channel, param: 0, value: i32::from(d1) }),
                0xE0 => {
                    // 14-bit little-endian → ALSA's signed pitch bend.
                    let value = (i32::from(d2) << 7) | i32::from(d1);
                    Event::new(EventType::Pitchbend, &EvCtrl { channel, param: 0, value: value - 8192 })
                }
                _ => return false,
            };
            event.set_dest(dest);
            event.set_direct();
            self.seq.event_output(&mut event).is_ok() && self.seq.drain_output().is_ok()
        }

        /// Re-reads the sequencer port list: attaches the control surface (matched by name) and
        /// the MIDI keyboard (matched by exact port name), detaching either when it is gone.
        pub fn rescan(&mut self, filter: &str, keyboard: Option<&str>, forced_surface: Option<&str>) -> (LinkState, Vec<PortEntry>) {
            let mut ports = Vec::new();
            let mut candidate: Option<(String, Addr)> = None;
            let mut keys: Option<(String, Addr)> = None;
            // A device may expose several readable ports ("SINCO SMC-Mixer-Private" and
            // "...-Master" on the M-Vave, for instance). The control messages live on the
            // Master/MCU port; the private one is the vendor's side channel, so rank it last.
            let score = |name: &str| -> i32 {
                let n = name.to_ascii_lowercase();
                if n.contains("private") {
                    0
                } else if n.contains("master") || n.contains("mcu") || n.contains("control") || n.contains("daw") {
                    2
                } else {
                    1
                }
            };
            for client in ClientIter::new(&self.seq) {
                let id = client.get_client();
                if id == self.client {
                    continue;
                }
                for port in PortIter::new(&self.seq, id) {
                    let caps = port.get_capability();
                    let name = port.get_name().map(str::to_string).unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let address = format!("{id}:{}", port.get_port());
                    let readable = caps.contains(PortCap::READ);
                    // "Midi Through" (the ALSA loopback), the system timer and clients with no
                    // real device behind them are not controllers or keyboards.
                    let dummy = ["through", "system", "timer"].iter().any(|d| name.to_ascii_lowercase().contains(d));
                    let matched = readable
                        && !dummy
                        && match forced_surface {
                            Some(want) => name == want || name.contains(want),
                            None => matches_filter(&name, filter),
                        };
                    if readable && !dummy {
                        ports.push(PortEntry { name: name.clone(), address, matched });
                    }
                    if matched && candidate.as_ref().is_none_or(|(best, _)| score(best) < score(&name)) {
                        candidate = Some((name.clone(), port.addr()));
                    }
                    if let Some(want) = keyboard
                        && readable
                        && keys.is_none()
                        && (name == want || name.contains(want))
                    {
                        keys = Some((name, port.addr()));
                    }
                }
            }

            log::debug!("MIDI: readable ports: {:?}", ports.iter().map(|p| p.name.as_str()).collect::<Vec<_>>());
            // Control surface subscription.
            if let Some((name, source)) = candidate {
                let changed = self.attached.as_ref().is_none_or(|(_, s, _)| *s != source);
                if changed {
                    self.detach();
                    // Feedback destination: a writable port on the same client, or — when the input
                    // comes from a read-only bridge (the Bluetooth MIDI bridge, for instance) — any
                    // matching writable port on the same device (the SMC's USB "Master" port).
                    let dest = write_port_on(&self.seq, source.client)
                        .or_else(|| {
                            for client in ClientIter::new(&self.seq) {
                                let id = client.get_client();
                                if id == self.client {
                                    continue;
                                }
                                for port in PortIter::new(&self.seq, id) {
                                    let caps = port.get_capability();
                                    let name = port.get_name().map(str::to_string).unwrap_or_default();
                                    if caps.contains(PortCap::WRITE) && !caps.contains(PortCap::READ) && matches_filter(&name, filter) {
                                        return Some(port.addr());
                                    }
                                }
                            }
                            None
                        })
                        .unwrap_or(source);
                    if dest != source {
                        log::info!("MIDI: feedback goes to {}:{}", dest.client, dest.port);
                    }
                    let subscribe_result =
                        PortSubscribe::empty().map_err(|e| format!("cannot prepare the control-surface subscription: {e}")).and_then(|sub| {
                            sub.set_sender(source);
                            sub.set_dest(Addr { client: self.client, port: self.in_port });
                            self.seq.subscribe_port(&sub).map_err(|e| format!("cannot subscribe to the control surface: {e}"))
                        });
                    match subscribe_result {
                        Ok(()) => {
                            if let Ok(out) = PortSubscribe::empty() {
                                out.set_sender(Addr { client: self.client, port: self.out_port });
                                out.set_dest(dest);
                                let _ = self.seq.subscribe_port(&out);
                            }
                            log::info!("MIDI: control surface attached: {name}");
                            self.attached = Some((name, source, dest));
                            self.hint.clear();
                        }
                        Err(error) => {
                            if self.hint != error {
                                log::warn!("MIDI: {error}");
                            }
                            self.hint = error;
                        }
                    }
                }
            } else if self.attached.is_some() {
                log::warn!("MIDI: control surface detached (port gone)");
                self.detach();
            }
            if self.attached.is_none() && self.hint.is_empty() {
                self.hint = "no control surface port found; connect it over USB or pair it over Bluetooth".to_string();
            }

            // MIDI keyboard subscription (by exact port name).
            match (keyboard, &keys) {
                (Some(want), Some((_, addr))) => {
                    let changed = self.keyboard.as_ref().is_none_or(|(_, a)| *a != *addr);
                    if changed {
                        self.detach_keyboard();
                        let sub = match PortSubscribe::empty() {
                            Ok(sub) => sub,
                            Err(_) => return (LinkState::Searching, ports),
                        };
                        sub.set_sender(*addr);
                        sub.set_dest(Addr { client: self.client, port: self.in_port });
                        if self.seq.subscribe_port(&sub).is_ok() {
                            log::info!("MIDI: keyboard attached: {want}");
                            self.keyboard = Some((want.to_string(), *addr));
                        } else {
                            log::warn!("MIDI: cannot subscribe the keyboard port `{want}`");
                        }
                    }
                }
                (Some(want), None) => {
                    log::warn!("MIDI: keyboard port `{want}` not found among the readable ports");
                    self.detach_keyboard();
                }
                (None, _) => self.detach_keyboard(),
            }

            let state = if self.attached.is_some() { LinkState::Connected } else { LinkState::Searching };
            (state, ports)
        }

        fn detach(&mut self) {
            if let Some((_, source, dest)) = self.attached.take() {
                let _ = self.seq.unsubscribe_port(source, Addr { client: self.client, port: self.in_port });
                let _ = self.seq.unsubscribe_port(Addr { client: self.client, port: self.out_port }, dest);
            }
        }

        fn detach_keyboard(&mut self) {
            if let Some((_, source)) = self.keyboard.take() {
                let _ = self.seq.unsubscribe_port(source, Addr { client: self.client, port: self.in_port });
            }
        }
    }

    /// Tags messages by exact ALSA port, not only by client: one device can expose separate
    /// keyboard and surface ports under the same ALSA client.
    fn classify_source(from: Addr, surface: Option<Addr>, keyboard: Option<Addr>) -> Source {
        if surface == Some(from) {
            Source::Surface
        } else if keyboard == Some(from) {
            Source::Keyboard
        } else {
            Source::Surface
        }
    }

    /// The writable MIDI port on a client, used for feedback to the surface.
    fn write_port_on(seq: &Seq, client: i32) -> Option<Addr> {
        PortIter::new(seq, client).find(|p| p.get_capability().contains(PortCap::WRITE)).map(|p| p.addr())
    }

    /// ALSA event → three MIDI bytes for the channel voice messages a surface or keyboard sends.
    pub fn event_to_bytes(event: &Event) -> Option<[u8; 3]> {
        match event.get_type() {
            EventType::Noteon | EventType::Noteoff => {
                let n: EvNote = event.get_data()?;
                let status = if event.get_type() == EventType::Noteon { 0x90 } else { 0x80 };
                Some([status | (n.channel & 0x0F), n.note & 0x7F, n.velocity & 0x7F])
            }
            EventType::Controller => {
                let c: EvCtrl = event.get_data()?;
                Some([0xB0 | (c.channel & 0x0F), (c.param & 0x7F) as u8, c.value.clamp(0, 127) as u8])
            }
            EventType::Chanpress => {
                let c: EvCtrl = event.get_data()?;
                Some([0xD0 | (c.channel & 0x0F), c.value.clamp(0, 127) as u8, 0])
            }
            EventType::Pitchbend => {
                let c: EvCtrl = event.get_data()?;
                let value = (c.value + 8192).clamp(0, 16383) as u16;
                Some([0xE0 | (c.channel & 0x0F), (value & 0x7F) as u8, ((value >> 7) & 0x7F) as u8])
            }
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn classifies_keyboard_and_surface_ports_independently_within_one_client() {
            let surface = Addr { client: 32, port: 0 };
            let keyboard = Addr { client: 32, port: 1 };

            assert_eq!(classify_source(surface, Some(surface), Some(keyboard)), Source::Surface);
            assert_eq!(classify_source(keyboard, Some(surface), Some(keyboard)), Source::Keyboard);
        }

        #[test]
        fn surface_assignment_wins_when_one_port_has_both_roles() {
            let address = Addr { client: 32, port: 0 };
            assert_eq!(classify_source(address, Some(address), Some(address)), Source::Surface);
        }

        #[test]
        #[ignore = "requires an available ALSA sequencer"]
        fn advertises_input_and_output_ports_with_correct_directions() {
            let link = AlsaLink::open("iCON").expect("open ALSA sequencer");
            let input_caps = PortIter::new(&link.seq, link.client).find(|port| port.get_port() == link.in_port).map(|port| port.get_capability());
            let output_caps = PortIter::new(&link.seq, link.client).find(|port| port.get_port() == link.out_port).map(|port| port.get_capability());

            assert!(input_caps.is_some_and(|caps| caps.contains(PortCap::WRITE | PortCap::SUBS_WRITE)));
            assert!(output_caps.is_some_and(|caps| caps.contains(PortCap::READ | PortCap::SUBS_READ)));
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::{InMessage, LinkState, PortEntry};
    use std::time::Duration;

    pub type Link = StubLink;

    /// No hardware MIDI backend on this platform: the surface is reported as unavailable.
    pub struct StubLink {
        hint: String,
    }

    impl StubLink {
        pub fn open(_filter: &str) -> Result<StubLink, String> {
            Err("no hardware MIDI backend on this platform yet (Linux ALSA sequencer is implemented)".to_string())
        }
        pub fn attached_name(&self) -> &str {
            ""
        }
        pub fn hint(&self) -> String {
            self.hint.clone()
        }
        pub fn lost(&self) -> bool {
            false
        }
        pub fn subscriptions_ok(&self) -> bool {
            true
        }
        pub fn forget_devices(&mut self) {}
        pub fn keyboard_state(&self) -> LinkState {
            LinkState::Unavailable
        }
        pub fn wait(&self, _timeout: Duration) -> Result<(), String> {
            Ok(())
        }
        pub fn next_message(&mut self) -> Option<InMessage> {
            None
        }
        pub fn send(&mut self, _message: [u8; 3]) -> bool {
            false
        }
        pub fn rescan(&mut self, _filter: &str, _keyboard: Option<&str>, _forced: Option<&str>) -> (LinkState, Vec<PortEntry>) {
            (LinkState::Unavailable, Vec::new())
        }
    }
}

pub use platform::Link;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_match_the_known_surface_aliases() {
        assert!(matches_filter("M-Vave SMC-Mixer MIDI 1", "SMC-Mixer"));
        assert!(matches_filter("SMC-Mixer", ""));
        assert!(matches_filter("SMC-Mixer Bluetooth", "iCON"));
        assert!(matches_filter("iCON P1-M V2.0", "SMC-Mixer"));
        assert!(matches_filter("QCon Pro X", ""));
        assert!(matches_filter("Mackie MCU Pro", ""));
        assert!(!matches_filter("CASIO USB-MIDI MIDI 1", "SMC-Mixer"));
        assert!(!matches_filter("CASIO USB-MIDI MIDI 1", "iCON"));
        assert!(!matches_filter("USB MIDI Controller", "SMC-Mixer"));
        assert!(!matches_filter("USB Audio CODEC", "SMC-Mixer"));
        assert!(!matches_filter("Launchpad", ""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires connected CASIO USB-MIDI and SMC-Mixer devices"]
    fn connected_casio_and_smc_are_discovered_as_separate_roles() {
        let mut link = Link::open("iCON").expect("open ALSA sequencer");
        let (state, ports) = link.rescan("iCON", Some("CASIO USB-MIDI MIDI 1"), None);

        assert_eq!(state, LinkState::Connected);
        assert_eq!(link.attached_name(), "SMC-Mixer Bluetooth");
        assert_eq!(link.keyboard_state(), LinkState::Connected);
        assert!(ports.iter().any(|port| port.name == "CASIO USB-MIDI MIDI 1" && !port.matched));
        assert!(ports.iter().any(|port| port.name == "SMC-Mixer Bluetooth" && port.matched));
    }

    #[test]
    fn midi_io_without_a_device_reports_a_state_instead_of_panicking() {
        // The worker must degrade quietly on a machine with no sequencer (CI, containers, WASM).
        let io = MidiIo::start("SMC-Mixer");
        io.send([0x90, 60, 100]);
        assert!(io.recv().is_none());
        let status = io.status();
        assert!(matches!(status.state, LinkState::Unavailable | LinkState::Searching), "{status:?}");
        assert!(!status.backend.is_empty());
        io.rescan();
        io.set_filter("iCON");
    }
}
