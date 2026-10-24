//! BLE-MIDI for the M-Vave SMC-Mixer over Bluetooth LE.
//!
//! BlueZ does not implement a MIDI profile, so this module talks the standard BLE-MIDI GATT
//! service directly through BlueZ's D-Bus API (`org.bluez.GattCharacteristic1.AcquireNotify`
//! for input, `WriteValue` for LED/fader feedback) and decodes the BLE-MIDI packet format:
//! a header with the high 6 bits of a 13-bit timestamp, then each MIDI message preceded by a
//! timestamp byte (the low 7 bits). A timestamp byte is a byte with bit 7 set that does not
//! follow another timestamp byte; everything else is MIDI data.
//!
//! Wire format reference: the packet layout was taken from the public BLE-MIDI reference decoders
//! (Android's `BluetoothPacketDecoder`, Control-Surface's `BLEMIDIPacketBuilder`).

use crate::midi_io::{InMessage, LinkState, Source};
use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The BLE-MIDI service and characteristic (MIDI Association assigned numbers).
const MIDI_SERVICE: &str = "03b80e5a-ede8-4b33-a751-6ce34ec4c700";
const MIDI_CHAR: &str = "7772e5db-3868-4112-a1a9-f2669d106bf3";
/// Only lowercase, no dashes, for BlueZ object paths and UUID comparisons.
fn norm_uuid(uuid: &str) -> String {
    uuid.to_ascii_lowercase().replace('-', "")
}

/// State of the Bluetooth link, for the controller window and the toolbar lights.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BleStatus {
    pub state: LinkState,
    pub device: String,
    pub message: String,
    pub received: u64,
    pub sent: u64,
}

/// Decodes one BLE-MIDI packet into channel-voice messages (2-byte messages are zero padded).
pub fn decode_packet(packet: &[u8]) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    if packet.len() < 2 {
        return out;
    }
    let header = packet[0];
    if (header & 0xC0) != 0x80 {
        return out;
    }
    // Strip the timestamp bytes: a byte with bit 7 set that does not follow another timestamp
    // byte is a timestamp; every other byte is part of the MIDI stream.
    let mut midi: Vec<u8> = Vec::with_capacity(packet.len());
    let mut last_was_timestamp = false;
    for &b in &packet[1..] {
        if (b & 0x80) != 0 && !last_was_timestamp {
            last_was_timestamp = true;
        } else {
            last_was_timestamp = false;
            midi.push(b);
        }
    }
    // Parse the stream with running status.
    let mut status: u8 = 0;
    let mut data = [0u8; 3];
    let mut have = 0usize;
    for &b in &midi {
        if b >= 0xF8 {
            // Real-time messages are single-byte and independent of running status.
            continue;
        }
        if b >= 0x80 {
            status = b;
            have = 0;
            data[0] = status;
            continue;
        }
        if status == 0 {
            continue;
        }
        data[1 + have] = b & 0x7F;
        have += 1;
        let need = if (0xC0..=0xDF).contains(&(status & 0xF0)) { 1 } else { 2 };
        if have >= need {
            // Zero-pad the third byte for 2-byte messages.
            let third = if need == 1 { 0 } else { data[2] };
            out.push([data[0], data[1], third]);
            have = 0;
            // Running status keeps `status` for the next message.
        }
    }
    out
}

/// Encodes messages into one BLE-MIDI packet with the given 13-bit timestamp. Each message gets
/// its own timestamp byte (the running-timestamp form is an optional optimisation).
pub fn encode_packet(messages: &[[u8; 3]], timestamp: u16) -> Vec<u8> {
    let ts = timestamp & 0x1FFF;
    let mut out = Vec::with_capacity(2 + messages.len() * 4);
    out.push(0x80 | ((ts >> 7) & 0x3F) as u8);
    for m in messages {
        let status = m[0] & 0xF0;
        out.push(0x80 | (ts & 0x7F) as u8);
        out.push(m[0]);
        out.push(m[1] & 0x7F);
        if (0xC0..=0xDF).contains(&status) {
            // 2-byte message: no third byte.
        } else {
            out.push(m[2] & 0x7F);
        }
    }
    out
}

/// A live BLE-MIDI link. Dropping it stops the worker thread.
pub struct BleMidi {
    status: Arc<Mutex<BleStatus>>,
    commands: SyncSender<BleCommand>,
    running: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

enum BleCommand {
    Send([u8; 3]),
    Quit,
}

impl BleMidi {
    /// Starts the worker. It never blocks the caller; failures are reported in [`BleStatus`].
    pub fn start(filter: String, input: SyncSender<InMessage>) -> BleMidi {
        let status = Arc::new(Mutex::new(BleStatus {
            state: LinkState::Searching,
            device: String::new(),
            message: "looking for a BLE-MIDI device…".into(),
            received: 0,
            sent: 0,
        }));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<BleCommand>(256);
        let running = Arc::new(AtomicBool::new(true));
        let shared = Arc::clone(&status);
        let alive = Arc::clone(&running);
        let join =
            std::thread::Builder::new().name("soundcraft-ble-midi".to_string()).spawn(move || worker(filter, shared, input, cmd_rx, alive)).ok();
        BleMidi { status, commands: cmd_tx, running, join }
    }

    pub fn status(&self) -> BleStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Queues three MIDI bytes for the device (LED/meter/motor feedback).
    pub fn send(&self, message: [u8; 3]) {
        let _ = self.commands.try_send(BleCommand::Send(message));
    }
}

impl Drop for BleMidi {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.commands.try_send(BleCommand::Quit);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn worker(filter: String, status: Arc<Mutex<BleStatus>>, input: SyncSender<InMessage>, commands: Receiver<BleCommand>, running: Arc<AtomicBool>) {
    let report = |state: LinkState, device: &str, message: &str| {
        if let Ok(mut s) = status.lock() {
            s.state = state;
            s.device = device.to_string();
            s.message = message.to_string();
        }
    };
    report(LinkState::Searching, "", "looking for a BLE-MIDI device…");
    let conn = match zbus::blocking::Connection::system() {
        Ok(c) => c,
        Err(e) => {
            report(LinkState::Unavailable, "", &format!("no system D-Bus (BlueZ): {e}"));
            return;
        }
    };
    // Wait for the device to be paired/connected, then acquire the MIDI characteristic.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut acquired: Option<(std::fs::File, String)> = None;
    while Instant::now() < deadline && running.load(Ordering::Relaxed) {
        if let Some((dev_path, dev_name, char_path)) = find_midi_characteristic(&conn, &filter) {
            match acquire_notify(&conn, &char_path) {
                Ok(file) => {
                    report(LinkState::Connected, &dev_name, "BLE-MIDI connected");
                    acquired = Some((file, dev_path));
                    break;
                }
                Err(e) => report(LinkState::Searching, &dev_name, &format!("cannot subscribe to the BLE-MIDI characteristic: {e}")),
            }
        } else {
            report(LinkState::Searching, "", "no BLE-MIDI service found (paired? connected? the SMC-Mixer must be on)");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let Some((mut file, dev_path)) = acquired else {
        report(LinkState::Unavailable, "", "BLE-MIDI link not established");
        return;
    };
    // Timestamp source: a free-running clock in 13-bit BLE-MIDI ticks (1 ms each is the usual
    // resolution; only relative order matters here).
    let start = Instant::now();
    let ts = || (start.elapsed().as_millis() as u16) & 0x1FFF;
    let mut buf = [0u8; 512];
    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        // Output first, then wait for input with a short timeout so shutdown stays responsive.
        let mut out: Vec<[u8; 3]> = Vec::new();
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                BleCommand::Send(m) => out.push(m),
                BleCommand::Quit => return,
            }
        }
        if !out.is_empty()
            && write_packets(&conn, &dev_path, &out, ts())
            && let Ok(mut s) = status.lock()
        {
            s.sent += out.len() as u64;
        }
        match file.read(&mut buf) {
            Ok(0) => {
                report(LinkState::Unavailable, "", "the BLE-MIDI connection was lost");
                break;
            }
            Ok(n) => {
                let messages = decode_packet(&buf[..n]);
                if !messages.is_empty() {
                    if let Ok(mut s) = status.lock() {
                        s.received += messages.len() as u64;
                    }
                    for m in messages {
                        if input.try_send(InMessage { source: Source::Surface, data: m, played: false }).is_err() {
                            break;
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                report(LinkState::Unavailable, "", "the BLE-MIDI connection was lost");
                return;
            }
        }
    }
}

/// Finds a connected device whose name matches `filter`, then its BLE-MIDI characteristic.
fn find_midi_characteristic(conn: &zbus::blocking::Connection, filter: &str) -> Option<(String, String, String)> {
    let reply = conn.call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &()).ok()?;
    let objects: std::collections::HashMap<
        zbus::zvariant::OwnedObjectPath,
        std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>>,
    > = reply.body().deserialize().ok()?;
    let filter = filter.to_ascii_lowercase();
    for (path, interfaces) in &objects {
        let Some(dev) = interfaces.get("org.bluez.Device1") else { continue };
        let name = dev.get("Name").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let alias = dev.get("Alias").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let connected = dev.get("Connected").and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
        let lower = format!("{name} {alias}").to_ascii_lowercase();
        let matches = (!filter.is_empty() && lower.contains(&filter)) || ["smc", "m-vave", "mvave", "cuvave"].iter().any(|a| lower.contains(a));
        if !matches || !connected {
            continue;
        }
        // Find the MIDI service, then the MIDI characteristic under it.
        let service = objects.iter().find_map(|(spath, sif)| {
            let s = sif.get("org.bluez.GattService1")?;
            let uuid = s.get("UUID").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
            let device = s.get("Device").and_then(|v| zbus::zvariant::OwnedObjectPath::try_from(v.clone()).ok());
            (norm_uuid(&uuid) == norm_uuid(MIDI_SERVICE) && device.as_ref() == Some(path)).then(|| spath.clone())
        });
        let char = objects.iter().find_map(|(cpath, cif)| {
            let c = cif.get("org.bluez.GattCharacteristic1")?;
            let uuid = c.get("UUID").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
            if norm_uuid(&uuid) != norm_uuid(MIDI_CHAR) {
                return None;
            }
            let service_of = c.get("Service").and_then(|v| zbus::zvariant::OwnedObjectPath::try_from(v.clone()).ok());
            let under = match (&service, &service_of) {
                (Some(want), Some(got)) => *want == *got,
                _ => true,
            };
            under.then(|| cpath.clone())
        });
        if let Some(char_path) = char {
            return Some((path.to_string(), if !name.is_empty() { name } else { alias }, char_path.to_string()));
        }
    }
    None
}

fn acquire_notify(conn: &zbus::blocking::Connection, char_path: &str) -> Result<std::fs::File, String> {
    let options: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = std::collections::HashMap::new();
    let reply = conn
        .call_method(Some("org.bluez"), char_path, Some("org.bluez.GattCharacteristic1"), "AcquireNotify", &(options,))
        .map_err(|e| format!("AcquireNotify: {e}"))?;
    let (fd, _mtu): (zbus::zvariant::OwnedFd, u16) = reply.body().deserialize().map_err(|e| format!("AcquireNotify reply: {e}"))?;
    Ok(std::fs::File::from(std::os::fd::OwnedFd::from(fd)))
}

fn write_packets(conn: &zbus::blocking::Connection, char_path: &str, messages: &[[u8; 3]], timestamp: u16) -> bool {
    let mut options: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = std::collections::HashMap::new();
    if let Ok(v) = zbus::zvariant::Value::from("command").try_into() {
        options.insert("type".to_string(), v);
    }
    let packet = encode_packet(messages, timestamp);
    conn.call_method(Some("org.bluez"), char_path, Some("org.bluez.GattCharacteristic1"), "WriteValue", &(packet, options)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_on_packet_round_trips() {
        let ts = 0x0123u16;
        let packet = encode_packet(&[[0x90, 60, 100]], ts);
        // Header carries the high 6 bits, then the timestamp byte, then the message.
        assert_eq!(packet[0], 0x80 | ((ts >> 7) & 0x3F) as u8);
        assert_eq!(packet[1], 0x80 | (ts & 0x7F) as u8);
        assert_eq!(decode_packet(&packet), vec![[0x90, 60, 100]]);
    }

    #[test]
    fn several_messages_in_one_packet_are_decoded() {
        let decoded = decode_packet(&encode_packet(&[[0x90, 60, 100], [0xB0, 7, 64]], 0x40));
        assert_eq!(decoded, vec![[0x90, 60, 100], [0xB0, 7, 64]]);
    }

    #[test]
    fn running_status_inside_a_packet_is_honoured() {
        // header, timestamp, note-on 60, then data only (running status) for note 64.
        let packet = [0x80, 0x80 | 0x10, 0x90, 60, 100, 64, 100];
        assert_eq!(decode_packet(&packet), vec![[0x90, 60, 100], [0x90, 64, 100]]);
    }

    #[test]
    fn two_byte_messages_and_garbage_are_handled() {
        // Program change (0xC0) has one data byte; its third byte is zero padded.
        let decoded = decode_packet(&encode_packet(&[[0xC0, 42, 0]], 7));
        assert_eq!(decoded, vec![[0xC0, 42, 0]]);
        // A packet that does not start with a header (10xxxxxx) is rejected, not crashing.
        assert!(decode_packet(&[0x00, 0x90, 60, 100]).is_empty());
        assert!(decode_packet(&[0x80]).is_empty());
    }
}
