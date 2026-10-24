# Plugin parity: built-in processors and plugin hosting

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** minor (insert and instrument commands resolve bridged plugins from the catalogue, never loading them on the UI thread; plugin validation moved off the UI thread with a visible state in the FX list) · **Target:** Avid Pro Tools Ultimate 2026.4.1

Pro Tools users live in plugins: the bundled set covers everyday mixing, and third-party plugins
cover the rest. This doc tracks both. Our plugins have our own names and are written from
scratch (clean-room rule): Avid plugin names are not used here, only categories.

**Built-in plugins and AudioSuite: breadth ~40 %, ready ~35 %** (80–120 h).
**Third-party hosting: breadth ~70 %, ready ~55 %** (40–60 h). Both estimated. Every installed
plugin is now measured rather than assumed: see [plugin-verification.md](plugin-verification.md).

## Built-in processors (measured from `crates/dsp/src/plugins`)

| Category | Pro Tools bundles (approx., by category) | SoundCraft (ids) | Gap |
|---|---|---|---|
| EQ | 1-band and 7-band EQ, channel strip, vintage EQ models | `eq_1band`, `eq_7band`, `channel_strip` | Vintage models, linear phase, dynamic EQ; parametric UI (#139) |
| Dynamics | compressor/limiter, expander/gate, de-esser, maximizer, vintage compressors, multiband | `compressor`, `expander_gate`, `de_esser`, `maximizer` | Vintage/opto models, multiband, sidechain input (no key input anywhere) |
| Reverb | algorithmic room/plate/hall; convolution (paid tier) | `room_reverb`, `plate_reverb` | Hall/chamber, convolution, early-reflection editor |
| Delay | modulated delay, tape/analog delays | `mod_delay` | Tape, ping-pong, multi-tap, tempo-sync depth |
| Modulation | chorus, flanger, phaser, tremolo, ring/frequency shift | `chorus`, `flanger`, `phaser` | Tremolo, auto-pan, ring modulation |
| Harmonic | lo-fi, rectifier, amp simulation, saturation, tape | `lofi`, `rectifier`, `saturator` | Amp and cabinet models, tape |
| Pitch and time | pitch shift, varispeed effects, time shift | `pitch_shifter`, `time_shift` | Formant control, varispeed effect, pitch correction |
| Utility | trim, gain, invert, DC offset, signal generator, dither, time adjuster | `trim`, `gain`, `invert`, `dc_offset_removal`, `signal_generator`, `dither` | Time adjuster, meter/analyzer plugins, noise shaping options |
| Instruments | several virtual instruments (multi-sampler, piano, drum machine, organ, synths, sample player; Massive X Player added in 2026.4) | `subtractive_synth`, `drum_synth`; `mix.instrument` takes the instrument's parameters in `params` and refuses unknown fields (#265) | Sampler (PR #108 open), piano, organ, wavetable, content library |
| **Total** | **≈ 80 processors and instruments** (estimate from Avid's bundle documentation) | **26 processors + 2 instruments** | |

AudioSuite: every catalog AudioSuite leaf maps to one of our processes (`AUDIOSUITE` in
`crates/ui-egui/src/menus.rs`), so the AudioSuite menu is 100 % present. Depth: no preview,
handles, "clip by clip / entire selection" and multi-input modes beyond the basics; hosted
plugins can't be used offline as AudioSuite yet.

Fidelity: **unmeasured.** Nothing compares a SoundCraft processor's frequency response, gain
reduction curve or reverb tail with the Pro Tools equivalent (G-07 in [`gaps.md`](gaps.md)).
User report #152: a saved instrument level does not reach the bounce.

## Third-party plugin hosting

| Feature | Pro Tools | SoundCraft | Notes |
|---|---|---|---|
| Plugin format | AAX only (Native and DSP) | CLAP, VST3, Audio Units (macOS) | Users' libraries mostly ship VST3/AU too. AAX (#109) needs Avid's SDK licence and PACE signing: owner decision, likely never |
| VST2, LV2, LADSPA | — | ❌ (#30) | Beyond Pro Tools; VST2 SDK is no longer licensable |
| Scan, vendor menus, instrument picker | ✅ | ✅ | #23 |
| Audio, parameters, automation, latency (PDC) | ✅ | ✅ | |
| Notes / MIDI to instruments | ✅ | ✅ | But no live MIDI input (G-02) |
| State saved in sessions, presets | ✅ | ✅ | |
| Create/destroy off the audio thread | ✅ | ✅ | |
| Editors: CLAP | — | 🟡 floating windows; embedded editors on Windows | Embedded on macOS missing; Linux CLAP editors still report unsupported |
| Editors: VST3 | — | 🟡 macOS (verified with a commercial pitch editor) and Windows (verified with a free JUCE compressor); Linux: native X11 windows with `Linux::IRunLoop` fd/timer dispatch — what bridged Kontakt under yabridge needs | not exercised on Linux with a real bridged plugin in this environment |
| Editors: AU | — | ❌ | Cocoa view hosting missing |
| Multiple editor pages | ✅ | 🟡 | #6 |
| macOS hardened-runtime entitlement for third-party code | ✅ | ❌ | #153, #149 — signed builds refuse third-party VST3 |
| Sidechain / key input | ✅ | ❌ | All three hosts |
| ARA 2 (pitch/time editors in the timeline) | ✅ | ❌ | Menu leaves Track/Clip/Window ▸ ARA still missing |
| Crash isolation | in-process (DSP on HDX) | in-process, behind a validation child | Out-of-process hosting would beat Pro Tools |
| Bridged (Wine) plugins: insert, load, report | ✅ | 🟡 | Inserting resolves the id from the scan catalogue without loading the bundle (a bridged plugin takes seconds under Wine and one that hangs froze the interface); the audio side loads a third-party plugin only after `--probe-plugin` proved it loads, now on a worker thread, and a slot that is still loading or was refused says so in the FX list |
| Plugin delay reporting changes on the fly | ✅ | 🟡 | Aux insert latency bug #103 (PR #133) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-19 | minor | Every installed plugin verified end to end (48/53); bridged Windows plugins find their sibling DLLs through `WINEPATH`; Pianoteq 9 fixed. See plugin-verification.md |
| 2026-10-18 | minor | Bridged plugin insert no longer loads the bundle on the UI thread; validation child asynchronous with state in the FX list |
| 2026-10-13 | trivial | mix.instrument sets built-in instrument parameters in params (#265) |
| 2026-10-18 | minor | Linux VST3 editor windows (X11 + Linux run loop), yabridge search paths, plugin scanning off the UI thread |
| 2026-10-12 | minor | Editors: VST3 and embedded CLAP editors on Windows |
| 2026-10-10 | major | Created: built-in processor inventory by category and third-party hosting checklist |
