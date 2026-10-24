# SoundCraft audit: real-time safety, hardware control surfaces, Linux plugin bridging

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** §2.1 REAPER comparison redone
> black-box with fact/hypothesis/conclusion tags (its Linux audio route is PulseAudio, not ALSA mmap); follow-up fixes for MIDI/BLE
> transport independence and port routing, in addition to implicit plugin scans, dock layout,
> audio-thread contract, M-Vave/iCON control surfaces,
> Linux plugin editors and the horizontal keyboard dock) · **Scope:** `crates/mix`, `crates/playback`, `crates/ui-egui`,
> `crates/vst3-host`, `crates/clap-host`, `apps/soundcraft`

This document records what the tree actually contained, what was broken, what was built, and what
is still missing. Companion setup/validation docs:
[`hardware-control-surfaces.md`](hardware-control-surfaces.md) (iCON P1-M / M-Vave SMC-Mixer) and
[`plugin-bridging-linux.md`](plugin-bridging-linux.md) (Wine + yabridge + PipeWire/Kontakt).

## 1. Architecture found

| Layer | Crate | What it owns | Thread |
|---|---|---|---|
| L0 | `soundcraft-time`, `soundcraft-audio-io`, `soundcraft-midi` | tempo/grid, audio file decode + peaks, Standard MIDI Files | any |
| L1 | `soundcraft-dsp` | built-in plugins (EQ, dynamics, reverb, pitch…), meters, FFT | audio (process) / any (edit) |
| L2 | `soundcraft-model` | the session document | UI thread only |
| L3 | `soundcraft-mix` | the render graph shared by playback and bounce | audio thread (`MixEngine::render`) |
| L4 | `soundcraft-engine`, `soundcraft-playback` | commands/undo/document; cpal device I/O, the audio callback, plugin-host driving | audio + UI |
| L6 | `soundcraft-ui-egui` | the egui interface, panels, dialogs, control channel | UI thread |

The audio callback (`soundcraft-playback::AudioState::fill`) drains a command channel, calls
`MixEngine::render` and publishes meters through a `try_lock`ed slot. The UI never mutates the mix
engine directly: it changes the document, which is handed over as an immutable snapshot through a
channel (`Player::update_session`). Plugin instances are hosted in-process (VST3/CLAP/AU) behind the
safe `soundcraft_dsp::Plugin` trait; the unsafe FFI lives only in the `-host` crates.

### 1.1 What the tree had before this pass

A previous pass had added the *skeleton* of the requested features (theme tokens, a keyboard dock,
an FX browser, an MSC manager, keyboard/profiler windows). Measured against the request, the
skeleton was UI-only in several places:

* `MscManager::auto_detect` set `connected = true` unconditionally, with no MIDI port behind it.
* Nothing ever called `handle_msc_midi`: no MIDI device was opened anywhere in the tree.
* `drain_output` (the LED/fader feedback queue) was called only by a test.
* MIDI Learn consumed the incoming message and printed a log line; no binding was recorded.
* No soft takeover; fader messages applied their value immediately.
* `vst3-host` on Linux returned *"editors are not hosted on this platform yet"*; there was no X11
  window and no `Linux::IRunLoop`, so a yabridged Kontakt could load but never show its GUI.
* The FX browser scanned all plugin folders on the UI thread and the app primed that scan during
  startup, which freezes the window while Wine-hosted plugins wake up.
* The computer-keyboard → MIDI handler ignored `egui_wants_keyboard_input`, so typing a track name
  played notes.
* Live MIDI note bookkeeping grew a `Vec<u8>` per instrument strip (`push`/`retain`) inside the
  audio callback.

## 2. Critical bugs eliminated

| # | Bug | Where | Fix | Test |
|---|---|---|---|---|
| A-1 | **No hardware MIDI I/O**: surface "connected", nothing read or written | `ui-egui` | New `midi_io` worker thread over the ALSA sequencer, subscribed device, bounded channels both ways | `midi_io_without_hardware_reports_a_state`, `matches_filter` cases |
| A-2 | **No hot-plug**: a re-plugged surface never reattached | `ui-egui::midi_io` | Worker re-enumerates every 250 ms, unsubscribes/subscribes on appearance/disappearance, counts resyncs | (manual, documented checklist) |
| A-3 | **Feedback never sent**: LED/meter/motor queue undrained and unbounded (`Vec::push`) | `ui-egui::msc_mixer` | `SoundApp::pump_control_surface` drains it to the worker each frame; `queue()` caps at `MAX_FEEDBACK_QUEUE` | `msc_fader_buttons_transport_and_feedback` (10 000 floods) |
| A-4 | **MIDI Learn was a no-op** that swallowed the message | `ui-egui::msc_mixer` | Persistent `MidiBinding` list (status/channel/data1 → volume, pan, send or plugin/instrument parameter), armed one-shot learn, precedence over the global map | `midi_learn_binds_a_control_and_mode_switches_the_global_mapping` |
| A-5 | **No soft takeover**: bank switching jumped the volume | `ui-egui::msc_mixer` | `pickup_gate` with waiting/engaged state, shared dB↔travel curve, pickups reset on bank/selection change | `soft_takeover_blocks_a_jump_and_engages_at_the_virtual_value` |
| A-6 | **Linux plugin editors unsupported** | `vst3-host::ffi` | X11 host window (`kPlatformTypeX11EmbedWindowID`), `WM_DELETE_WINDOW` handling, `Linux::IRunLoop` on both `IHostApplication` and `IPlugFrame` with fd/timer dispatch | `x11_editor_window_opens_resizes_and_closes` |
| A-7 | **UI froze while scanning plugins** | `ui-egui::fx_browser` | No synchronous scan anywhere on the UI thread; startup and buttons go through `spawn_async_sync`; the catalogue reads the cache only | manual: FX window stays responsive |
| A-8 | **Typing in a text field played notes** | `ui-egui::virtual_keyboard` | Handler stands down when `egui_wants_keyboard_input()` or a dialog owns the keyboard | `the_computer_keyboard_plays_notes_and_stands_down_for_text_input` |
| A-9 | **Audio-thread allocation in live MIDI** (`Vec::push`/`retain`) | `mix::Strip::held` | Fixed 128-bit `HeldNotes` bitmap | `cargo test -p soundcraft-mix` + RT contract (below) |
| A-10 | Mouse note never released (release frame counted as "down") | `ui-egui::virtual_keyboard` | `is_down` is press-and-hold / drag only | `the_keyboard_dock_plays_and_releases_a_note_on_a_mouse_press` |
| A-11 | Bank/track change left stale pickup state | `ui-egui::msc_mixer` | `reset_pickups()` on bank, selection and profile change | soft-takeover test |
| A-12 | Two dB normalization curves (UI vs hardware) could disagree | `ui-egui::msc_mixer` | One `norm_from_db`/`db_from_norm` pair used by both directions | implied by feedback + soft takeover tests |
| A-13 | Mixed Portuguese/English UI strings | `ui-egui` (menus, mixer, FX browser, edit window) | All user-visible strings English (localization parity doc) | `grep` for the old strings in CI review |
| A-14 | Preferences written to `ui.json`; controller/keyboard state not persisted at all | `apps/soundcraft` | `config.json` (with one-time `ui.json` migration), `UiState::msc`/`UiState::keyboard`, saved promptly on change | `config_tests` + `msc_preferences_round_trip_through_json` |
| A-15 | Empty-mixer dead end, no way to add a track from the console | `ui-egui::mix_window` | Empty-console view with instrument/audio/master/demo actions and an ADD strip | offscreen render |
| A-17 | **A scan could close the editor**: scanning loaded every plugin inside the app process, so one plugin that crashed while being inspected (bridged Wine plugins on a slow drive) killed the window | `soundcraft` + host crates | Scanning now runs in a child process (`soundcraft --scan-plugins`): one JSON line per plugin, adopted back with `adopt_scan`; a crash mid-scan keeps the plugins already reported, and the editor survives. Out-of-process is the default, with an in-process fallback and a 15-minute watchdog | scanner line-parsing tests, `fixture_e2e`, headless child run |
| A-18 | **Virtual keyboard clicks landed on the mixer dock** and the MIDI editor could not expand to its tested height with the mixer visible | `ui-egui::edit_window` | Dock stacking now puts the virtual keyboard at the bottom above the status tabs, with the mixer above it; the mixer contracts while the MIDI editor is open to preserve piano-roll workspace | `virtual_keyboard_ui`, `midi_panel` |
| A-16 | **UI freeze / process abort from implicit plugin scans**: lookup on a cache miss could synchronously walk plugin folders and `dlopen` every bundle, including yabridge/Wine modules, from render or UI paths | `vst3-host`, `clap-host` registries; `ui-egui::fx_browser` | Host instantiation and parameter lookup are cache-only; discovery is explicit, and the FX browser scans in a child process. This prevents an unknown plugin ID from starting an in-process scan. Explicit host `scan` APIs remain caller-controlled and must run off real-time threads | `cargo test -p soundcraft-mix`, host lib tests, workspace Clippy |
| A-19 | **BLE-MIDI was coupled to ALSA startup**, and MIDI events from different ports of one ALSA client could be mislabeled | `ui-egui::midi_io`, `ui-egui::ble_midi` | BLE now runs and retries independently when ALSA is unavailable; ALSA source routing compares exact ports with surface precedence; BLE GATT service lookup is scoped to its owning device | Linux port-classification tests; UI crate tests |

### 2.1 Reference-DAW comparison (REAPER, observed black-box)

REAPER (v7.82, linux-x86_64, in `~/opt/REAPER`) is a technical reference only. Rules: observe the
running program, the user's own config files, logs, public docs and generic binary metadata
(`file`, `ldd`, `readelf -d`); never decompile, never copy its code, strings or undocumented
internals; SoundCraft's solutions are original, built on public APIs (ALSA, PipeWire, X11/ICCCM/EWMH,
CLAP/VST3) and our own tests. Each claim below is tagged **F** (observed fact), **H** (hypothesis,
not verified) or **C** (conclusion confirmed by a reproducible check).

**Method (2026-10-18, GNOME/mutter Wayland session with XWayland, PipeWire + pipewire-pulse).**
REAPER was started with a *copy* of the user's `reaper.ini` in a scratch directory
(`reaper -cfgfile /tmp/…/reaper.ini -new -nosplash`), so the user's configuration was not modified.
Observed with `/proc/<pid>/{maps,fd,task}`, `/proc/asound/*/hw_params`, `pw-dump`, `xprop`/`xwininfo`
and `xwd` window captures (kept under `plan/reaper/screenshots/`, never committed). A plugin window
was opened with REAPER's public ReaScript API (`TrackFX_AddByName` + `TrackFX_Show`) on a native
CLAP plugin (ZamComp), and the device page with its documented action 40099. The backend list was
read by restarting with only `linux_audio_mode` changed (0…5) and capturing the device page.

**Audio**

| # | Observation | Tag |
|---|---|---|
| A1 | `readelf -d reaper` lists `libasound.so.2` as the only audio library; no JACK, Pulse or PipeWire in `NEEDED` | F |
| A2 | At runtime `libpulse-simple`, `libpulse` and `libjack` are mapped although not in `NEEDED`, in every mode tried | F |
| A3 | So the non-ALSA backends are loaded at runtime (`libdl`) and are optional on the target system | H (consistent with A1–A2; mechanism not inspected) |
| A4 | The device page names four audio systems: `linux_audio_mode` 0 = JACK, 1 = ALSA, 2 = Dummy Audio, 3 = PulseAudio; 4 and 5 show an empty selector | C (reproduced by restart per value) |
| A5 | The user's configuration selects mode 3 (**PulseAudio**), 2 in / 2 out, 44100 Hz, block 512; ALSA in/out devices empty | F (`reaper.ini` + device page) |
| A6 | In mode 3 REAPER holds no `/dev/snd/*` descriptor; it appears in PipeWire as two `pipewire-pulse` streams ("REAPER output", "REAPER input", 44100 Hz); the hardware PCM is held by `pipewire` | C |
| A7 | One `reaper/audio` thread runs `SCHED_OTHER` (config `alsa_rtprio=0`, `jack_rtprio=-1`, `linux_mlockall=0`) | F |
| A8 | The ALSA page offers input/output device, rate, block and bit depth; the JACK page offers a jackd launch command and auto-connect to hardware | F (screenshots) |

The earlier claim here, "REAPER uses ALSA PCM with mmap + poll", was inferred from imported symbol
names and is **withdrawn**: it is not what this installation does (A5–A6), and how REAPER drives
ALSA in mode 1 was not observed.

**MIDI**

| # | Observation | Tag |
|---|---|---|
| M1 | `reaper-midihw-alsa.ini` caches devices named `virtual`, `hw:USBMIDI`, `hw:SINCO`, `hw:SINCO,0,1`, i.e. ALSA hardware (rawmidi-style) names | F |
| M2 | `reaper-midihw-linux.ini` caches a second, generic list (`MIDI Output 1…N`), so there are two MIDI device modes | F (files); what selects between them: H |

**Plugin windows** (ZamComp CLAP, X11 editor)

| # | Observation | Tag |
|---|---|---|
| W1 | All REAPER windows are X11 (XWayland) even on a Wayland session; GDK 3 and `libwayland-client` are mapped at runtime although `libSwell.so` only lists X11/Xi/GL/fontconfig in `NEEDED` | F |
| W2 | The plugin window is a top-level `_NET_WM_WINDOW_TYPE_NORMAL` window with `WM_TRANSIENT_FOR` = the main window and `_NET_WM_STATE_SKIP_TASKBAR`; it does **not** carry `_NET_WM_STATE_ABOVE` (the scan-progress dialog does) | F (`xprop`) |
| W3 | Inside it, the plugin's own X window (a different X client) is a child of a host container placed under a 28-px host strip (preset menu, `+`, `Param`, pin layout "2 in 1 out", `UI` toggle, wet knob) | F (`xwininfo -tree`, capture) |
| W4 | The `.swell` section of `reaper.ini` exposes user options such as `gdk_owned_windows_keep_above` (default 1) and `gdk_owned_windows_in_tasklist` (default 0) | F; that these produce W2's transient + skip-taskbar: H |

**SoundCraft, compared**

| Question | REAPER (this machine) | SoundCraft today | Status |
|---|---|---|---|
| Audio backends offered on Linux | JACK, ALSA, Dummy, PulseAudio (A4) | cpal hosts: ALSA only (cpal's `jack` feature is not enabled) | gap: no JACK/PipeWire-native choice |
| Default route on PipeWire | PulseAudio API → `pipewire-pulse` (A6) | ALSA `default` device | H: reaches PipeWire through the `pipewire-alsa` plugin; verify with `pw-dump` while playing |
| Audio thread priority | `SCHED_OTHER` (A7) | `SCHED_OTHER` (none raised) | same default |
| MIDI | ALSA hw names (M1) | ALSA sequencer ports, `Master`/`MCU` ranked above `Private` | different design, both public ALSA APIs |
| Plugin window | transient + skip-taskbar, no ABOVE, host strip (W2–W3) | transient (`WM_TRANSIENT_FOR`) + `_NET_WM_STATE_ABOVE` + `XSetInputFocus`, no host strip | ours is stricter: ABOVE keeps the editor over *every* app, not only over SoundCraft |

Follow-ups (ours, original): (1) offer JACK (cpal `jack` feature, served by `pipewire-jack`) as a
selectable audio system and show the active route in the Audio Setup window; (2) re-evaluate
`_NET_WM_STATE_ABOVE` on editor windows, since `WM_TRANSIENT_FOR` alone already keeps a transient
above its owner on EWMH window managers, and add `_NET_WM_STATE_SKIP_TASKBAR`; (3) a test that opens
an editor window under Xvfb and asserts its `WM_TRANSIENT_FOR` / `_NET_WM_STATE` properties.

The two MIDI designs differ: REAPER's cache lists hardware device names, SoundCraft subscribes sequencer ports.
The M-Vave SMC-Mixer exposes two raw subdevices (`SINCO,0,0` "Private" and `SINCO,0,1` "Master"),
which the sequencer presents as two ports — the control messages live on the Master port, which
the selector now prefers (the MIDI Devices window can also force any port).

## 3. The real-time contract (what is guaranteed now)

Rules the audio callback lives by, and where each is enforced:

1. **No allocation in steady state.** `MixEngine::render` reuses per-strip buffers; note
   bookkeeping is a fixed bitmap (`HeldNotes`); live MIDI messages are three bytes on the stack.
2. **No locks.** The callback drains an `mpsc` channel with `try_recv`, and publishes meters by
   `try_lock` (a skipped frame is acceptable, a blocked callback is not).
3. **No I/O, no UI, no plugin scanning.** The MSC worker, plugin scanning and disk access all live
   on their own threads; the UI thread hands over immutable session snapshots.
4. **Bounded queues everywhere the UI can flood.** Hardware feedback is capped at 512 packets, the
   MIDI worker's channels at 1024 messages, and at most 512 input messages are handled per frame.
5. **No panics from input.** Every `unwrap` in the touched paths became `Result`/`Option` with a
   logged fallback: MIDI parsing (`get(1).unwrap_or(&0)`), device open failures, plugin editor
   errors (reported in the status bar), X11 open failures (window creation returns an error).

Known limitation, unchanged and documented: `MixEngine::sync` (instance create/destroy) still
allocates and can run from `render` when the document changed; the UI already calls
`update_session` outside the callback, so this is the L3 fallback rather than the normal path.

## 4. What was built, requirement by requirement

**1. Threading / resilience.** `HeldNotes` bitmap; bounded feedback and MIDI queues; a `LinkState`
model (`Unavailable`/`Searching`/`Connected`) with a human message for every failure; the MIDI
worker never panics and retries forever; plugin-editor failures surface in the status bar instead
of being swallowed.

**2b. MIDI keyboards.** The same ALSA worker subscribes a second port — a piano or controller such
as a Casio CDP-S160 — and routes its notes, controllers and pitch bend to the selected instrument
track, lighting the on-screen keyboard. Ports are listed in the controller window (click one to
use it), `controller.keyboard_auto` picks a piano-like port, the choice is persisted, and the
window shows the last incoming messages so a dead or mis-mapped control is visible.

**2. M-Vave SMC-Mixer / iCON P1-M.** New `midi_io` module (ALSA sequencer worker thread,
`LinkState`, port list, rx/tx counters, resyncs) plus a rewritten `MscManager`: two profiles
(SMC-Mixer, P1-M), MCU mapping (faders via 14-bit pitch bend, V-pots, mute/solo/arm/select, bank and
channel navigation, transport, markers, automation modes, flip, scrub/jog, fader touch), soft
takeover, MIDI Learn with persisted links, global vs. link mode, and bidirectional feedback (LEDs,
12-segment MCU meters over channel pressure, motorized faders, transport and automation LEDs).
The controller window shows the live link, the ports with the matched one starred, throughput,
resync count and the learned links.

**3. Windows plugins on Linux.** yabridge folders (`~/.vst3/yabridge`, `~/.vst/yabridge`,
`/usr/lib/vst3/yabridge`, CLAP equivalents) are standard search paths; the scanner never runs on the
UI thread; `yabridgectl sync` is available from the FX browser; VST3 editors open in native X11
windows (detached, window-manager managed, closing the window hides the editor while the plugin and
its audio/state survive), with `Linux::IRunLoop` fd/timer dispatch so bridged GUIs receive input and
paint, and keyboard focus isolated by the window system.

**4. Horizontal virtual keyboard.** Bottom dock (F7 / Window ▸ MIDI Keyboard / dock tab), 25–88
keys, `C…` labels on C keys, mouse press/hold/glissando (and touch, which egui delivers as pointer
input), computer-keyboard mapping with on-key hints, velocity slider, target-track indicator,
pitch-bend and mod wheels routing live MIDI, and illumination for notes coming from the mouse, the
computer keyboard and the control surface (`held_notes` is shared).

**5. Interface / themes.** Every track in the Edit window carries an always-visible volume
knob — a Behringer X-Touch Compact style rotary with a glowing LED ring (green → amber → red like
the hardware's encoder collar, dark cap with a white pointer stripe) — next to the FX button on
tall headers and a compact one on name-row-only headers, which writes
the same `mix.volume` merge as the console fader, so volume is on screen whichever columns are
shown and without opening the Mix window. Seven palettes including the four required (Dark Studio, Clean Light,
Cyberpunk/Neon, Vintage Console) with per-theme meter style (12-LED ladder, continuous, VU, neon
blocks), fader-cap style, knob style, corner radii and depth; quick selector in the top bar and
`ui.theme` for scripting; persisted in `config.json`. Track headers and strips carry a vector glyph
slot by module type (sampler/Kontakt, synth, drum, mic, bus, master), an `MSC CH n`/`ACTIVE`
badge, a highlighted border for the armed/selected/active-target track, monospaced technical
readouts, and peak meters with clip indication.

**6. Preferences.** `~/.config/soundcraft/config.json` (XDG-aware, `SOUNDCRAFT_CONFIG_DIR`
override) holds theme, audio device, layout, controller mapping/mode/soft-takeover/learned links
and keyboard range; `ui.json` from older builds is read once and migrated.

## 5. Verification performed

```sh
# build + tests (ALSA headers live in ~/.local/usr; the sandbox has no system -dev package)
export PKG_CONFIG_PATH="$HOME/.local/usr/lib/x86_64-linux-gnu/pkgconfig"
export LIBRARY_PATH="$HOME/.local/usr/lib/x86_64-linux-gnu"

cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p soundcraft-ui-egui            # 107 lib tests + every integration test
cargo test -p soundcraft-vst3-host --lib    # includes the X11 window test (skips without DISPLAY)
cargo run -q -p soundcraft-ui-egui --example ui_shot -- /tmp/shot.png script.jsonl
```

One exception: `tests/recovery.rs` (autosave leftovers and file locks) fails 2 of its 6 tests in this
sandbox, and fails the same way on a pristine `git stash`ed checkout of `HEAD` (3 of 6 there), so
those failures are environmental (the sandbox's file-lock/`/tmp` behaviour), not a regression from
this work.

Offscreen renders were produced for Dark/FL/Studio Pro/Cyberpunk/Vintage/Light with the keyboard
dock open. `cargo fmt --check` could not run in this environment (the rustfmt component is not
installed and the rustup directory is read-only); the code follows the repository's `max_width =
150`, `use_small_heuristics = "Max"` style by hand, and should be formatted before merging.

## 6. Remaining gaps (honest list)

| Gap | Status | Notes |
|---|---|---|
| VST2 (`.dll`/`.vst`) hosting | ❌ not implemented | Needs a VST2 host with its own AEffect/host-callback FFI; yabridge's VST2 path would then work. Kontakt 7/8 is VST3-only, so the required Kontakt workflow works through VST3. |
| CLAP editors on Linux | ❌ still "unsupported" | The X11 window helper exists in `clap-host` only for Windows/macOS; porting the new Linux module is mechanical (≈180 lines) and tracked here. |
| MIDI backend on macOS/Windows | ❌ stub | `midi_io` reports `Unavailable`; the channel protocol and `MscManager` are platform-neutral. |
| Web MIDI (WASM) | ❌ | The browser build has no JS bridge for Web MIDI yet. |
| BLE-MIDI pairing | 🟡 | SoundCraft now connects directly to the standard BLE-MIDI GATT service through BlueZ; an ALSA bridge remains an alternative. Pairing and device-specific GATT behavior still need validation on hardware. |
| BLE-MIDI reconnection after link loss | 🟡 | The parent MIDI worker restarts a stopped BLE worker with a retry delay. USB/ALSA hot-plug continues independently; live hardware reconnection still needs validation. |
| Per-track GUI button on **edit-window** track headers | 🟡 | The Mix console has the GUI button and double-click; the edit header has the FX button. |
| Plugin parameter picker for MIDI Learn | 🟡 | `LearnTarget` supports insert/instrument parameters and the command accepts explicit min/max; the UI currently arms volume/pan from the controller window, and plugin parameters are armed from the command channel. |
| Session-level "MIDI Learn" UI on plugin knobs (right-click) | 🟡 | The engine-side target exists; the context menu on third-party plugin parameter rows is still to wire. |
| `MixEngine::sync` allocation from `render` | 🟡 pre-existing | Only on document change; the normal path syncs outside the callback. |
| 88-key range labels | ✅ | C keys are labelled; every key shows its computer-key hint. |
