//! Realtime scheduling of the audio callback thread.
//!
//! cpal runs each stream's callback on a thread of its own (ALSA) or of the audio server (JACK,
//! CoreAudio, WASAPI). On Linux with ALSA that thread starts as an ordinary `SCHED_OTHER` thread,
//! so a busy desktop can delay it past the device period and the device underruns (xruns).
//!
//! The first callback promotes its own thread, once:
//! 1. `SCHED_FIFO` at [`FIFO_PRIORITY`] (capped by `RLIMIT_RTPRIO`) through `pthread_setschedparam`,
//!    which succeeds for users with a realtime limit (the `audio` group / `limits.conf`) or
//!    `CAP_SYS_NICE`. One non-blocking system call, no allocation.
//! 2. Otherwise a helper thread asks RTKit (`org.freedesktop.RealtimeKit1`, the desktop's
//!    documented D-Bus service) to raise the thread's nice level. RTKit's *realtime* request is
//!    deliberately not used: it requires lowering `RLIMIT_RTTIME` for the whole process, and a
//!    plugin that then held the audio thread for ~200 ms would get the process killed by SIGXCPU.
//!
//! The callback never blocks or logs here: everything slow or chatty happens on the helper.
//! Under JACK (and pipewire-jack) the server owns the thread and its priority, so nothing is done.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};

/// The `SCHED_FIFO` priority requested for the audio thread: below PipeWire's and JACK's own
/// driver threads (88 and 70+ by default), above ordinary desktop threads.
pub const FIFO_PRIORITY: i32 = 60;
/// The nice level requested from RTKit when realtime scheduling is not permitted.
pub const HIGH_PRIORITY_NICE: i32 = -11;

/// How the audio thread is scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Realtime {
    /// Not known yet (no callback ran) or not applicable (no audio device).
    Unknown,
    /// The audio server owns the thread (JACK, CoreAudio, WASAPI, the browser).
    Managed,
    /// `SCHED_FIFO` at this priority.
    Fifo(i32),
    /// Normal scheduling at this (negative) nice level, granted by RTKit.
    HighPriority(i32),
    /// Normal scheduling: neither realtime nor RTKit was available.
    Normal,
}

impl Realtime {
    /// A one-line description for the audio settings and the log.
    pub fn describe(self) -> String {
        match self {
            Realtime::Unknown => "unknown".into(),
            Realtime::Managed => "managed by the audio system".into(),
            Realtime::Fifo(p) => format!("realtime (SCHED_FIFO {p})"),
            Realtime::HighPriority(n) => format!("high priority (nice {n}, RTKit)"),
            Realtime::Normal => "normal priority (add your user to the `audio` group for realtime)".into(),
        }
    }
}

/// The scheduling outcome, shared between the callback, the helper and the UI.
#[derive(Debug, Default)]
pub struct RtStatus {
    kind: AtomicU8,
    value: AtomicI32,
}

impl RtStatus {
    pub fn get(&self) -> Realtime {
        let v = self.value.load(Ordering::Acquire);
        match self.kind.load(Ordering::Acquire) {
            1 => Realtime::Managed,
            2 => Realtime::Fifo(v),
            3 => Realtime::HighPriority(v),
            4 => Realtime::Normal,
            _ => Realtime::Unknown,
        }
    }

    pub fn set(&self, r: Realtime) {
        let (k, v) = match r {
            Realtime::Unknown => (0, 0),
            Realtime::Managed => (1, 0),
            Realtime::Fifo(p) => (2, p),
            Realtime::HighPriority(n) => (3, n),
            Realtime::Normal => (4, 0),
        };
        self.value.store(v, Ordering::Release);
        self.kind.store(k, Ordering::Release);
    }
}

/// The `SCHED_FIFO` priority to ask for under a realtime limit of `limit` (`None`: unlimited);
/// `None` when the limit forbids realtime scheduling (a privileged process may still succeed,
/// so the caller tries [`FIFO_PRIORITY`] anyway).
pub fn fifo_priority(limit: Option<u64>) -> Option<i32> {
    match limit {
        None => Some(FIFO_PRIORITY),
        Some(0) => None,
        Some(l) => Some(i32::try_from(l).unwrap_or(FIFO_PRIORITY).clamp(1, FIFO_PRIORITY)),
    }
}

/// The nice level to ask RTKit for, given its `MinNiceLevel` (the most negative it grants).
pub fn rtkit_nice(min_nice: i32) -> i32 {
    HIGH_PRIORITY_NICE.max(min_nice).min(-1)
}

/// What the callback reports to the helper.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Msg {
    Fifo(i32),
    /// Realtime refused: ask RTKit for this kernel thread id.
    Rtkit(u64),
}

/// Lives in a stream callback; promotes the thread on the first call.
pub(crate) struct Promoter {
    done: bool,
    status: Arc<RtStatus>,
    tx: Option<std::sync::mpsc::SyncSender<Msg>>,
}

impl Promoter {
    /// `managed`: the audio server owns the callback thread (JACK); nothing to do.
    pub(crate) fn new(status: Arc<RtStatus>, managed: bool) -> Promoter {
        if managed || !cfg!(target_os = "linux") {
            status.set(Realtime::Managed);
            return Promoter { done: true, status, tx: None };
        }
        status.set(Realtime::Unknown);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Msg>(1);
        let helper_status = Arc::clone(&status);
        let spawned = std::thread::Builder::new().name("soundcraft-rt".into()).spawn(move || {
            // Ends when the stream (and its callback, holding the sender) is dropped unused.
            match rx.recv() {
                Ok(Msg::Fifo(p)) => log::info!("audio thread: realtime (SCHED_FIFO {p})"),
                Ok(Msg::Rtkit(tid)) => {
                    let r = rtkit_high_priority(tid);
                    helper_status.set(r);
                    log::info!("audio thread: {}", r.describe());
                }
                Err(_) => {}
            }
        });
        match spawned {
            Ok(_) => Promoter { done: false, status, tx: Some(tx) },
            Err(e) => {
                log::warn!("realtime helper thread failed: {e}");
                Promoter { done: false, status, tx: None }
            }
        }
    }

    /// Call at the top of every callback; does work only the first time.
    #[inline]
    pub(crate) fn on_callback(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let msg = promote_current_thread(&self.status);
        if let (Some(tx), Some(msg)) = (self.tx.take(), msg) {
            // Capacity 1, one message: never blocks.
            let _ = tx.try_send(msg);
        }
    }
}

/// Tries `SCHED_FIFO` for the calling thread; on refusal returns the RTKit request.
#[cfg(target_os = "linux")]
fn promote_current_thread(status: &RtStatus) -> Option<Msg> {
    use thread_priority::{RealtimeThreadSchedulePolicy, ThreadPriority, ThreadPriorityValue, ThreadSchedulePolicy};
    let limit = rustix::process::getrlimit(rustix::process::Resource::Rtprio).current;
    let prio = fifo_priority(limit).unwrap_or(FIFO_PRIORITY);
    let value = u8::try_from(prio).ok().and_then(|p| ThreadPriorityValue::try_from(p).ok());
    if let Some(value) = value {
        let ok = thread_priority::set_thread_priority_and_policy(
            thread_priority::thread_native_id(),
            ThreadPriority::Crossplatform(value),
            ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo),
        )
        .is_ok();
        if ok {
            status.set(Realtime::Fifo(prio));
            return Some(Msg::Fifo(prio));
        }
    }
    let tid = rustix::thread::gettid().as_raw_nonzero().get();
    Some(Msg::Rtkit(u64::try_from(tid).unwrap_or(0)))
}

#[cfg(not(target_os = "linux"))]
fn promote_current_thread(_status: &RtStatus) -> Option<Msg> {
    None
}

/// Asks RTKit to raise thread `tid` of this process to a high (negative nice) priority.
#[cfg(target_os = "linux")]
fn rtkit_high_priority(tid: u64) -> Realtime {
    const DEST: &str = "org.freedesktop.RealtimeKit1";
    const PATH: &str = "/org/freedesktop/RealtimeKit1";
    if tid == 0 {
        return Realtime::Normal;
    }
    let conn = match zbus::blocking::Connection::system() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("RTKit: no system bus: {e}");
            return Realtime::Normal;
        }
    };
    // MinNiceLevel is an RTKit property (i32); a missing one falls back to our default.
    let min_nice = conn
        .call_method(Some(DEST), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(DEST, "MinNiceLevel"))
        .ok()
        .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(HIGH_PRIORITY_NICE);
    let nice = rtkit_nice(min_nice);
    let pid = u64::from(std::process::id());
    match conn.call_method(Some(DEST), PATH, Some(DEST), "MakeThreadHighPriorityWithPID", &(pid, tid, nice)) {
        Ok(_) => Realtime::HighPriority(nice),
        Err(e) => {
            log::warn!("RTKit refused a high-priority audio thread: {e}");
            Realtime::Normal
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn rtkit_high_priority(_tid: u64) -> Realtime {
    Realtime::Managed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fifo_priority_respects_the_realtime_limit() {
        assert_eq!(fifo_priority(None), Some(FIFO_PRIORITY));
        assert_eq!(fifo_priority(Some(0)), None);
        assert_eq!(fifo_priority(Some(20)), Some(20));
        assert_eq!(fifo_priority(Some(95)), Some(FIFO_PRIORITY));
        assert_eq!(fifo_priority(Some(u64::MAX)), Some(FIFO_PRIORITY));
    }

    #[test]
    fn the_rtkit_nice_level_stays_inside_what_rtkit_grants() {
        assert_eq!(rtkit_nice(-20), HIGH_PRIORITY_NICE);
        assert_eq!(rtkit_nice(-5), -5);
        // A nonsensical (positive) minimum still asks for a raised priority.
        assert_eq!(rtkit_nice(3), -1);
    }

    #[test]
    fn the_status_round_trips_every_outcome() {
        let s = RtStatus::default();
        assert_eq!(s.get(), Realtime::Unknown);
        for r in [Realtime::Managed, Realtime::Fifo(60), Realtime::HighPriority(-11), Realtime::Normal, Realtime::Unknown] {
            s.set(r);
            assert_eq!(s.get(), r);
        }
    }

    #[test]
    fn a_managed_stream_is_left_alone() {
        let status = Arc::new(RtStatus::default());
        let mut p = Promoter::new(Arc::clone(&status), true);
        p.on_callback();
        assert_eq!(status.get(), Realtime::Managed);
    }

    #[test]
    fn the_first_callback_promotes_once_and_reports_an_outcome() {
        let status = Arc::new(RtStatus::default());
        let s2 = Arc::clone(&status);
        // Promote a throwaway thread, not the test runner's.
        let t = std::thread::spawn(move || {
            let mut p = Promoter::new(s2, false);
            p.on_callback();
            p.on_callback();
        });
        assert!(t.join().is_ok());
        if cfg!(target_os = "linux") {
            // FIFO is decided on the callback; the RTKit answer arrives from the helper.
            for _ in 0..200 {
                if status.get() != Realtime::Unknown {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(matches!(status.get(), Realtime::Fifo(_) | Realtime::HighPriority(_) | Realtime::Normal), "{:?}", status.get());
        } else {
            assert_eq!(status.get(), Realtime::Managed);
        }
    }
}
