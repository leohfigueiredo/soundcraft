# Control surfaces: iCON P1-M and M-Vave SMC-Mixer

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** BLE direct-link behavior,
> keyboard-vs-surface routing, and Linux hardware troubleshooting

SoundCraft drives Mackie-Control-style surfaces over the ALSA sequencer. The same worker thread
serves USB-MIDI (class-compliant) controllers, Bluetooth BLE-MIDI bridges that expose an ALSA
sequencer port, and PipeWire's ALSA-MIDI layer. The audio thread is never involved: messages cross
a bounded channel to the UI thread, and feedback (LEDs, meters, motorized faders) crosses another.

Open **Setup ▸ Hardware ▸ DAW Controller Setup (iCON / M-Vave)…** to see the live link, the port
list, the mapping state and the learned controls. From the control channel:
`{"id":1,"method":"engine.execute","params":{"command":"controller.status"}}`.

## 1. Connection

### USB-MIDI (recommended for jog wheels and motorized faders)

Both the iCON P1-M and the M-Vave SMC-Mixer are USB class-compliant: no driver, no Wine, no
`bluetoothctl`. Plug the device in; ALSA creates a raw-MIDI port and a sequencer client.

```sh
amidi -l                  # raw-MIDI hardware ports (card, device)
aconnect -l               # sequencer clients and ports: find "M-Vave SMC-Mixer" / "iCON P1-M"
aconnect -o               # writable (feedback) ports
```

The worker subscribes to the readable port of the matching client and sends feedback to that
client's writable port. Nothing has to be connected by hand; an existing `aconnect` connection is
harmless.

**Permissions.** Your user needs access to `/dev/snd/*`. On most distributions the `audio` group is
enough (`sudo usermod -aG audio "$USER"`, then log out and in). Verify:

```sh
ls -l /dev/snd/seq /dev/snd/midiC*    # group audio, mode 0660
id -nG | tr ' ' '\n' | grep -E '^(audio|plugdev)$'
```

### Bluetooth BLE-MIDI

SoundCraft can connect directly to a paired BLE-MIDI device through BlueZ's GATT API; no separate
ALSA bridge is required when **Bluetooth (BLE-MIDI)** is enabled in the controller window. The
worker also supports an external BLE-MIDI bridge that exposes the device as an ALSA sequencer port.
Use one path at a time if the bridge already owns the Bluetooth connection:

* built-in: pair/connect with BlueZ, enable **Bluetooth (BLE-MIDI)** in SoundCraft, and check the
  BLE state in the controller window;
* bridge: a BLE-MIDI daemon can expose a sequencer port named after the device, which the regular
  ALSA surface scan will then discover.

Pairing (the helper in `packaging/linux/setup_msc_bluetooth.sh` automates the same steps):

```sh
bluetoothctl power on
bluetoothctl scan on                 # wait for "SMC-Mixer"
bluetoothctl pair    AA:BB:CC:DD:EE:FF
bluetoothctl trust   AA:BB:CC:DD:EE:FF   # reconnect after power cycling
bluetoothctl connect AA:BB:CC:DD:EE:FF
```

The device filter (default `SMC-Mixer`) matches the Bluetooth device or the ALSA bridge port; the
controller window's **Device filter** field changes it. ALSA **Rescan** re-reads the port list.
Bluetooth adds latency: use the jog wheel for scrubbing and the transport buttons for start/stop,
and expect the motorized faders to lag a few milliseconds behind the screen.

The SMC-Mixer is a control surface, not a note keyboard: its faders, knobs and transport buttons
control the mixer and transport. To hear notes, connect a separate MIDI keyboard in **MIDI
Devices** as **Input**, select/create an instrument track with a loaded instrument, and confirm
that **Last MIDI input** reports `KEYBOARD` messages.

### MIDI keyboard (a piano or a controller)

The same link subscribes a second port for a MIDI keyboard. Notes, controllers and pitch bend go
to the selected instrument track and light the on-screen keyboard. In the controller window pick
a port in the **MIDI keyboard** row (or press **Auto-connect piano**), or from the control channel:

```sh
{"id":1,"method":"engine.execute","params":{"command":"controller.midi_ports"}}
{"id":2,"method":"engine.execute","params":{"command":"controller.keyboard","params":{"port":"hw:3,0,0"}}}
{"id":3,"method":"engine.execute","params":{"command":"controller.keyboard","params":{"port":null}}}
```

The choice is saved in `config.json` and re-attached at startup. **Last MIDI input** shows every
message that arrives, tagged SURFACE or KEYBOARD — the fastest way to tell a dead cable from a
mis-mapped control.

### Hot-plug

The worker re-enumerates the sequencer every 250 ms. If the surface disappears (cable, power,
Bluetooth drop) it unsubscribes, reports **Searching** and keeps the session untouched; when the
device comes back it subscribes again and re-sends the full LED/fader state. The resync counter in
the controller window shows how often this happened.

## 2. Mapping (MCU)

| Control | Message | Action |
|---|---|---|
| Fader 1–8 | Pitch bend ch 1–8 (14-bit) | Volume of the bank's tracks, through soft takeover |
| Fader 9 | Pitch bend ch 9 | Master volume |
| Fader touch | Note 104–112 on/off | Suppresses motor feedback while the fader is held |
| V-pot 1–8 | CC 16–23 (relative) | Pan of the bank's tracks |
| V-pot push | Note 32–39 | Pan to centre |
| Sends | CC 24–31 | Send 1 level |
| Mute | Note 16–23 | Mute (LED fed back) |
| Solo | Note 8–15 | Solo (LED fed back) |
| Record arm | Note 0–7 | Record arm (LED fed back) |
| Select | Note 24–31 | Select the track (drives the keyboard and the "active" badge) |
| Bank ◀ ▶ | Note 46 / 47 | Previous / next bank of 8 tracks |
| Channel ◀ ▶ | Note 48 / 49 | Previous / next track |
| Play / Stop / Record | Note 94 / 93 / 95 | Transport (LEDs fed back) |
| Loop | Note 86 | Loop playback toggle (LED fed back) |
| Rewind / Forward | Note 91 / 92 | Transport rewind / fast-forward |
| Marker | Note 84 | Drop a memory location at the playhead |
| Flip | Note 50 | Faders ⇄ V-pots (faders become pan) |
| Scrub | Note 101 | Jog wheel: timeline jog ⇄ fine scrub |
| Jog wheel | CC 60 | Timeline jog / scrub |
| Automation | Notes 74–79 | Read / Write / Touch / Latch / Trim / Off on the selected track |
| Macros | CC 1, 11, 71, 74–78 | Routed live to the active instrument (Kontakt macros) |
| Meters | Channel pressure 0xD0–0xD7 | 12-segment MCU meter ladder |
| Motorized faders | Pitch bend out | Sent whenever the value changes and the fader is untouched |

Profiles and every number above live in `msc_mixer_map.json` / `iCON_p1m` presets; the controller
window switches profile without a restart.

## 3. Soft takeover (fader pickup)

Bank switching moves eight faders to eight different tracks. Without pickup, the first message from
a fader that was left at a different position would jump the track's volume. With soft takeover
(default on, `controller.soft_takeover` toggles it):

1. the fader is **waiting**: its messages are ignored and the status line says where to bring it;
2. it **engages** when it reaches the track's current value (within 1 % of the travel) or crosses it;
3. it then follows continuously until the bank, the selection or the profile changes.

The V-pots are relative encoders and need no pickup. Turning soft takeover off makes the faders
absolute again (useful for a surface that reports the physical position reliably).

## 4. Global mode vs. Link to controller (MIDI Learn)

Two modes (`controller.mode`, or the two selectable labels in the controller window):

* **Global** — unlearned faders and V-pots control the tracks of the current bank (FL-style);
* **Link to controller** — only learned controls do anything; the faders are free for plugin
  macros.

Learned controls always win over the bank mapping, in either mode. Arm one and move a control:

```sh
# from the control channel, for the selected track
{"id":1,"method":"engine.execute","params":{"command":"controller.learn","params":{"target":"volume"}}}
{"id":2,"method":"engine.execute","params":{"command":"controller.learn","params":{"target":"pan"}}}
# or with an explicit track and a plugin parameter (plain range min..max)
{"id":3,"method":"engine.execute","params":{"command":"controller.learn","params":{"target":"insert","track":12,"slot":0,"param":"Cutoff","min":20,"max":20000}}}
```

The controller window shows the armed target, the list of learned controls (`✖` unlearns one) and
**Clear all**. Bindings are saved in `~/.config/soundcraft/config.json` and survive restarts.

## 5. Feedback (MIDI out)

Every UI frame, changed state is sent back: mute/solo/arm LEDs, selected fader positions (never while
the fader is touched), the 12-segment meter byte per channel, transport, loop, flip, scrub and the
automation-mode LEDs of the selected track. **Flash LEDs & exercise faders** queues a full test
sweep. Feedback is droppable and capped (`MAX_FEEDBACK_QUEUE`), so an unplugged device cannot grow
memory.

## 6. Validation checklist

Run these in order; every step states what a healthy result looks like.

| # | Check | Command | Healthy result |
|---|---|---|---|
| 1 | Kernel sees the device | `lsusb` | a line naming the surface |
| 2 | Raw-MIDI port exists | `amidi -l` | a hardware MIDI port for the connected USB device |
| 3 | Sequencer client exists | `aconnect -l` | client with a readable port named like the surface (or BLE bridge) |
| 4 | Messages arrive | `aseqdump -p "hw:2,0,0"` then move a fader | `Pitch-bend … value …` lines (14-bit) |
| 5 | SoundCraft's client is visible | `aconnect -l` with SoundCraft running | client `SoundCraft Control Surface` with `Control In` / `Control Out` |
| 6 | SoundCraft attached | `controller.status` | `"link":"Connected"`, `"port"` = the surface |
| 7 | Feedback reaches the device | move a fader in the UI | the physical fader motor follows; mute buttons light up |
| 8 | Hot-plug | unplug, watch status, re-plug | `Searching` → `Connected`, `"reconnects"` increments |
| 9 | Jog/transport | press Play/Stop on the surface | transport follows, LED reflects state |
| 10 | Permissions | `id -nG` | `audio` (or the group owning `/dev/snd/*`) |
| 11 | Bluetooth direct link | `bluetoothctl info <MAC>` | paired and connected; SoundCraft BLE state becomes `Connected` |

When nothing appears in `aconnect -l`, the sequencer itself is the problem:

```sh
lsmod | grep snd_seq
sudo modprobe snd-seq
ls -l /dev/snd/seq
```

To test without hardware, connect a software port to SoundCraft's input and send messages by hand:

```sh
aconnect "Virtual Keyboard:0" "SoundCraft Control Surface:Control In"   # any MIDI source
# or poke it from the control channel:
{"id":9,"method":"engine.execute","params":{"command":"controller.rescan"}}
```

`aseqdump -p 128:0` (a CLI player) is a convenient source when no keyboard is around.
