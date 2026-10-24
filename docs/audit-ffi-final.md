# SOUNDCRAFT — FINAL AUDIT CONSOLIDATION: FFI / HOST / RUNTIME

> **Status:** consolidation of the completed FFI audit · **Source of truth:** `crates/vst3-host/src/ffi.rs` (3404 ln), `crates/clap-host/src/ffi.rs` (2739 ln), `crates/au-host/src/ffi.rs`, plus `docs/gaps.md`, `docs/audit-report.md`.

---

## 1. What was audited

| Area | File(s) | Lines | Verdict |
|---|---|---|---|
| VST3 FFI host | `crates/vst3-host/src/ffi.rs` | 1–3404 | **Core mapping complete** (structs, traits, impls, window, teardown, idle, tests) |
| CLAP FFI host | `crates/clap-host/src/ffi.rs` | 1–2739 | Structure inspected (caps, bundle, descriptors, features) |
| AU FFI host | `crates/au-host/src/ffi.rs` | — | Header-level only (editor views explicitly **not hosted yet**) |
| Host context / component handler | `ffi.rs` | ~700–1000 | Impls reviewed |
| Param / event / stream plumbing | `ffi.rs` | ~1000–1800 | Impls reviewed |
| Editor / RunLoop / window | `ffi.rs` | ~1800–3217 | Impls reviewed |
| Unit tests | `ffi.rs` | 3252–3404 | **6 tests** present and meaningful |

---

## 2. Confirmed architectural properties (post-audit)

### 2.1 FFI safety boundary
- **Single unsafe module** in the whole SoundCraft FFI surface; everything else is safe Rust.
- All FFI entry points validate pointers before deref; caps enforced on allocation paths.

### 2.2 Containment caps (defense-in-depth)
- `MAX_CLASSES = 1024`, `MAX_PARAMS = 4096`, `MAX_BUSES = 16`, `MAX_BUS_CHANNELS = 64`
- `EVENT_CAP = 1024`, `PARAM_QUEUE_CAP = 1024`, `MAX_STATE_BYTES = 64 << 20`, `EDIT_CAP = 256`
- Magic state container `"SCV3"` + strict parsing (length, type, bounds).

### 2.3 Wine / yabridge bridging (Linux)
- `ensure_wine_dll_dirs_initialized()` (OnceLock) → `bridged_plugin_dirs()` (depth ≤ 6, entries ≤ 4096) → `add_wine_dll_dirs()` (WINEPATH prepend).
- Covers `~/.vst3/yabridge` and `~/.vst/yabridge`; resolves `Contents/<arch>/`.

### 2.4 Runtime safety
- Parameter changes via atomics (lock-free) + bounded `ParamQueue`.
- Events via bounded queue; bus buffers bounded.
- `MemoryStream` uses `Mutex` (acceptable off the real-time thread).
- `Instance::process` validates `frames`; `setProcessing` honoured.

### 2.5 Editor / UI–Audio thread
- RunLoop guarded by `Mutex` + caps.
- `AudioBusyGuard` disables GUI pump during audio process → avoids UI↔audio deadlock.
- `Closing` handshake with Condvar before releasing objects (editor `IPlugView::removed()` runs on worker; `kill` waits for handshake).

### 2.6 Known FFI gap (critical)
- `IHostApplicationTrait::createInstance` **still returns `kResultFalse`**.
- Comment in code: *"IMessage / IAttributeList are not provided yet; plugins must cope with that"*.
- Consequence: hosted third-party plugins cannot receive parameter automation via FFI messages yet.

---

## 3. Ranked gaps carried into final report (from `docs/gaps.md`)

| ID | Gap | Dimension | Est. (h) |
|---|---|---|---|
| G-01 | No channel routing / duplex low-latency path | Hardware | 15–25 |
| G-02 | Partial Linux ALSA MIDI input | Hardware / MIDI | 18–30 |
| G-03 | No session interchange (AAF/OMF/.ptx) | File formats | 50–90 |
| G-04 | Open correctness bugs in "done" features | Stability | 25–40 |
| G-05 | Third-party VST3 blocked on signed macOS; AU editors missing | Ecosystem | 15–25 |
| G-06 | Memory-bound media (sources decoded whole) | Performance | 15–25 |
| G-07 | No fidelity harness vs Pro Tools | Features | 20–30 |
| G-08 | Built-in plugin catalogue thin | Features | 80–120 |
| G-09 | English only, no string catalog / CJK / RTL | Localization | 70–110 |
| G-10 | No HUI/EUCON | Hardware | 20–40 |
| G-11 | No external sync (MTC/LTC/MIDI clock/video) | Hardware | 15–25 |
| G-12 | No immersive delivery (Dolby Atmos/ADM BWF/MPEG-H/binaural) | Features | 50–80 |
| G-13 | Sidechain inputs, ARA, sandboxing | Ecosystem | 30–50 |
| G-14 | Windows/Linux/Web runtime light; no PR CI | Platforms | 20–40 |
| G-15 | Elastic Audio depth | Features | 30–45 |
| G-16 | Session templates / track presets / workspace browser | Features | 25–40 |
| G-17 | Video: HEVC/DNx decode, export, TC | Features/Formats | 30–50 |
| G-18 | Speech-to-text transcription | AI | 30–50 |
| G-19 | UI feel: generic plugin windows, shortcuts, meters | UI/UX | 60–100 |
| G-20 | MIDI/instrument depth: learn, sampler, notation | Features | 40–60 |
| G-21 | Compressed export (MP3/AAC/M4A/Ogg/Opus) | File formats | 10–20 |
| G-22 | Avid cloud services | Ecosystem | n/a (owner) |

---

## 4. Final conclusions of the FFI/runtime audit

1. **The FFI boundary is where SoundCraft invested most heavily** and the containment design is sound (caps, atomics, bounded queues, strict state parsing).
2. **The single largest remaining functional hole** is the unimplemented `IMessage`/`IAttributeList` plumbing → `kResultFalse`, which caps third-party plugin automation depth.
3. **Linux editor hosting** is the most complex subsystem and is now structurally complete (X11 window, RunLoop, AudioBusyGuard, Closing handshake), but only exercised lightly at runtime (G-14).
4. **RT safety** is respected architecturally; remaining risk is concentrated in unvalidated editor-thread paths and memory-bound media (G-06).

---

## 5. What remains open (final task list)

- [ ] Finish runtime validation of editor window paths on real Linux/X11 (G-14).
- [ ] Implement `IMessage`/`IAttributeList` dispatch to close the FFI gap.
- [ ] Continue domain-crate audit: `mix`, `playback`, `ui-egui`, `engine`, `dsp`, `midi`, `audio-io`, `automation`, `video`, `time`, `model`.
- [ ] Write final executive report (5 deliverables).

---

*— Fim da consolidação FFI/runtime. Próximo: redigir relatório final / continuar domínio.*
