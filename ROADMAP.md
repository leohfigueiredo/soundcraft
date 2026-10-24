# SoundCraft roadmap

**Stage: pre-alpha** · next: alpha, ~5% and ~110–180 h away (the alpha gate's three failing workflows)

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (readiness by audience with hours to ~95 %) · **Target:** Avid Pro Tools Ultimate 2026.4.1

SoundCraft is a clean-room, pure-Rust digital audio workstation that aims at full parity with
Avid Pro Tools, and then beyond it: faster, open, scriptable and agent-controllable. This page is
the one-page summary; the detail is in [`docs/target-app-parity.md`](docs/target-app-parity.md)
(assessment), [`docs/gaps.md`](docs/gaps.md) (ranked work list), [`docs/roadmap.md`](docs/roadmap.md)
(milestones and beta plan) and [`docs/parity-checklist.md`](docs/parity-checklist.md) (generated
menu checklist).

## Headline numbers

| Number | Value | Kind |
|---|---|---|
| Menu breadth | **482 / 512 Pro Tools menu leaves (94.1 %)**; engine alone 397 / 512 | measured (`cargo xtask parity` + UI-layer aliases) |
| Feature breadth | **~70 %** | estimated (menus plus plugins, hardware, formats, immersive, cloud) |
| **Ready for real work** | **~40 %** | estimated, weighted by dimension |
| Mainstream practitioner (tracking/mixing engineer) | **~36 %** | estimated: 50.6 % weekly-area depth × 0.90 interaction × 0.92 stability × 0.85 file exchange ([how](docs/target-app-parity.md#mainstream-practitioner-and-essentials-user)) |
| Essentials user (song or podcast, defaults) | **~57 %** | estimated: 72.4 % core-feature depth × 0.90 launch/stability × 0.90 discoverability × 0.97 files ([how](docs/target-app-parity.md#mainstream-practitioner-and-essentials-user)) |
| Remaining to alpha | **~110–180 Opus 5.5 agent-hours** (the failing gate rows) | estimated, calibrated on PR history |
| Remaining to beta | **~350–570 Opus 5.5 agent-hours** (includes the alpha work; ~60–70 % parallelizable) | estimated |
| Remaining to full parity | **~950–1,450 Opus 5.5 agent-hours** | estimated |

| Audience | Ready % | Agent-hours to ~95 % | Work that dominates |
|---|---:|---:|---|
| Full target (ready for real work) | ~40 % | ~950–1,450 h | Hardware, the stock plugin catalogue, AAF/OMF and immersive, localization, fidelity |
| Mainstream practitioner | ~36 % | ~380–600 h | Interface and MIDI recording, AAF/OMF, stock-plugin depth, core-path bugs |
| Essentials user | ~57 % | ~110–200 h | Launch and platform fixes, onboarding, effect panels and presets, MP3/AAC export |

Hours are Opus 5.5 agent wall-clock hours, calibrated on this repo's PRs; 60–70 % parallelizes.
Detail: [target-app-parity.md](docs/target-app-parity.md#readiness-by-audience).

**Why pre-alpha:** the core-workflow gate in [docs/roadmap.md](docs/roadmap.md#alpha-gate) fails.
Editing, mixing, bouncing, and saving and reopening work end to end. But a Pro Tools user can't
record from a chosen audio interface at a chosen buffer size (only the default devices, a fixed
512-sample block, and takes held in RAM). They can't record MIDI from a keyboard (there is no
MIDI device layer). And they can't bring a session in or hand one back (no AAF/OMF/`.ptx`).
Readiness (~40 %) sits right at the alpha bar, but a gate failure keeps the app pre-alpha. The
2026-10-07 estimate ("~65 % overall, 110–150 h to 100 %") counted feature presence. This one
measures readiness and adds hardware, localization and field bug reports.

## By dimension

| Dimension | Ready | Remaining (h) | Doc |
|---|---:|---:|---|
| Features | ~45 % | 520–810 | [target-app-parity.md](docs/target-app-parity.md#feature-areas) |
| UI/UX fidelity | ~55 % | 60–100 | [ui-parity.md](docs/ui-parity.md) |
| File formats | ~40 % | 70–120 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (interfaces, MIDI, surfaces, sync) | ~12 % | 110–180 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization | ~12 % (1 of Pro Tools' 8 languages) | 70–110 | [localization-parity.md](docs/localization-parity.md) |
| Performance | ~35 % (unmeasured) | 40–70 | [hardware-parity.md](docs/hardware-parity.md#performance) |
| Stability | ~45 % | 50–80 | [gaps.md](docs/gaps.md) |
| Platforms | ~65 % (5 platforms built, macOS exercised) | 20–40 | [gaps.md](docs/gaps.md) |
| Ecosystem / plugins | ~30 % | in Features + Hardware | [plugin-parity.md](docs/plugin-parity.md) |
| AI features | ~5 % | 30–50 | [gaps.md](docs/gaps.md) |
| Agent control | ahead of Pro Tools | 3 | [control-protocol.md](docs/control-protocol.md), [mcp.md](docs/mcp.md) |

## Features

| Area | Breadth | Ready | Remaining (h) |
|---|---:|---:|---:|
| Recording and monitoring | 75 % | 40 % | 40–60 |
| Editing | 92 % | 65 % | 40–60 |
| Mixing and routing | 85 % | 60 % | 35–50 |
| Automation | 80 % | 55 % | 20–30 |
| Built-in plugins and AudioSuite | 40 % | 35 % | 80–120 |
| Third-party plugin hosting | 70 % | 45 % | 40–60 |
| MIDI and instruments | 70 % | 30 % | 50–80 |
| Score and notation | 60 % | 35 % | 20–30 |
| Elastic Audio, TCE, Beat Detective | 70 % | 35 % | 40–60 |
| Surround and immersive | 55 % | 35 % | 50–80 |
| Video and post | 60 % | 40 % | 40–60 |
| Session, file management, interchange | 55 % | 35 % | 60–100 |
| Collaboration and cloud (Avid services) | 0 % | 0 % | owner decision |
| Agent control | 100 % | 100 % | 3 |

Weights and evidence per row: [target-app-parity.md](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | Translated |
|---|---|---|---:|
| English | en | full | 100 % |
| Simplified Chinese | zh-Hans | none | 0 % |
| Spanish | es | none (PR #143 open) | 0 % |
| Hindi | hi | none | 0 % |
| Arabic | ar | none | 0 % |
| French | fr | none (PR #143 open) | 0 % |
| Portuguese | pt | none | 0 % |
| Indonesian | id | none | 0 % |
| Japanese | ja | none | 0 % |
| German | de | none | 0 % |
| Korean | ko | none | 0 % |
| Vietnamese | vi | none | 0 % |

Other languages shipped: 0. Detail: [localization-parity.md](docs/localization-parity.md).

## Upcoming

Ranked; the alpha gate, the alpha checklist and the beta plan are in [docs/roadmap.md](docs/roadmap.md).

1. **Alpha blocker:** audio interfaces: device and buffer-size choice, duplex stream, ASIO; record to disk (G-01, G-06). 35–55 h.
2. **Alpha blocker:** MIDI devices: record from keyboards, play instruments live, output (G-02). 20–35 h.
3. **Alpha blocker:** AAF/OMF import and export (G-03). 50–90 h.
4. macOS plugin entitlement (#153), then AU editors (G-05). 1 h for the entitlement (needed for the mix row), then 15–25 h.
5. Land the open bug-fix PRs and clear the correctness backlog (G-04). 15–25 h.
6. PR CI running `cargo xtask ci` on three OSes (G-14). 5–10 h.
7. Fidelity harness against Pro Tools (G-07). 20–30 h.

## Robustness

Property tests drive random edit sequences (with full undo), mutated session files and hostile
command parameters through the engine and mixer; they have caught fade, overflow and
unbounded-allocation bugs. Every command also runs with empty parameters on empty and demo
sessions. Hosted plugins live in isolated unsafe crates (`clap-host`, `vst3-host`, `au-host`) and
are created, loaded and destroyed off the audio thread. The video decoders survive hundreds of
mutated and truncated movies. Field reports from the first public week are tracked in
[gaps.md](docs/gaps.md) (G-04).

## Progress log

- **2026-10-20 (teclado ALT GUI não toca outro plugin):** Com o console de um instrumento aberto, o teclado virtual e as teclas MIDI agora procuram uma faixa daquele plugin e ficam silenciosos se ela não existir; não enviam mais as notas para um instrumento diferente só porque ele está selecionado ou aparece primeiro na sessão. O contador de dropouts continua sendo um diagnóstico separado da carga de áudio.
- **2026-10-20 (browser de timbres capturados):** Cada ALT GUI de instrumento agora tem uma lista independente de timbres capturados: escolhe-se o som na janela do plugin, guarda-se o estado com um nome e pode-se carregar ou apagar pela lista. Aplica-se a Pianoteq, Pigments, Omnisphere, Kontakt, Analog Lab e LABS; não depende dos `.fxp` VST2 nem inventa nomes que o plugin não disponibiliza.
- **2026-10-20 (contador de dropouts do dispositivo):** "There is a strange noise" now has a number. The output callback compares the device's own block timestamps and counts the blocks that arrived much later than one block ("dropouts"), which is what a click, pop or stutter is. It shows in the status bar (`⚠ N dropout(s) / M blocks`, only when non-zero), in **Window ▸ Session Audio Health** (with the device, rate, channels and buffer, and what to do about it), and in the control channel as `mixer.meters {dropouts, blocks}`. Measured on this machine's default output: 12 748 blocks, **0 dropouts** — a clean audio path, which points a stutter at the plugin or the session rather than at the device. The check never blocks: a contended lock just skips one measurement.
- **2026-10-20 (a outra metade da nota presa):** `AudioState::fill` stopped rendering entirely as soon as nothing was held and the transport was stopped — *before* the mix engine ever ran. So the note-off the mix engine had queued was never flushed, whatever the mix did. The audio side now keeps rendering while a live message is pending (`MixEngine::has_live_pending`), which is the second of the two gates that left notes droning. Regression test `a_note_off_is_flushed_even_with_the_transport_stopped` fails without the change.
- **2026-10-20 (KONTRA, um sampler nativo de bibliotecas Kontakt):** [KONTRA](https://github.com/DerpcatMusic/KONTRA) is a Rust, Apache-2.0 sampler that reads Kontakt-format instruments and ships Linux x64 builds in CLAP, VST3 and standalone. Verified on this machine: SoundCraft's scanner loads both formats with no Wine (`vst3:FC092AF5096DE321B960F4D245F8D72A`, `clap:audio.matari.kontra`), the editor opens, and state round-trips. It now drives the **Kontakt console** in the alternative GUI (library picker included), so a Kontakt library can be played without Kontakt, yabridge or Wine. Alpha software: playback is experimental and the licence requires legally usable libraries.
- **2026-10-20 (ALT GUI: escolher o instrumento):** Picking an instrument with no preset bound used to do nothing audible, because the VSTs in question ignore program changes (Pianoteq's program list is a stub). The console now marks bound instruments with a dot, and an unbound pick *opens the plugin's own window* and says what to do next ("pick the sound, then SAVE PRESET"), so the workflow is one click instead of a dead end.
- **2026-10-20 (nota presa: a causa):** With the transport stopped, the mix engine runs instrument tracks in *monitor-only* mode and skips any instrument that holds no note — and a note-off empties that set immediately, so the release stayed queued inside the plugin and the note sounded forever. Clicking the on-screen keyboard with the mouse left one stuck voice per click: a droning, distorted cluster. The strip now keeps processing while a live MIDI message is pending (`Strip::live_pending`), so the release is flushed. Measured: the regression test `a_live_note_off_reaches_the_instrument_even_with_the_transport_stopped` fails without the fix (the plugin never sees the note-off) and passes with it. `Event ▸ All MIDI Notes Off` had also been a no-op and now really stops every instrument (that is the escape hatch for notes started inside a plugin's own GUI).
- **2026-10-20 (All MIDI Notes Off era um no-op):** `Event ▸ All MIDI Notes Off` (`Cmd+Shift+.`) only pushed a transport request that the UI matched with `{}` — it did nothing, so there was no way to stop a note the host never saw start (a click on the keyboard inside a plugin's own GUI, a controller left holding a key, a lost note-off). It now sends `allNotesOff` to every instrument and insert, plus All Sound Off (CC 120) and All Notes Off (CC 123) on all 16 channels, and clears the keyboard's held state. The on-screen keyboard's mouse path got its own tests (press/release, dragging off the keys) proving one note-on and one note-off per click.
- **2026-10-20 (nota presa do teclado virtual):** Playing a note on the virtual keyboard could leave the hosted instrument droning: the window system repeats a held key several times a second and each repeat was treated as a new press (so the note retriggered as a pulsing distorted drone), and a release that never arrived — pointer leaving the window, the dock closed, the window losing focus — left the note on for good. Repeats are now ignored (only a real press starts a note), and a per-frame watchdog releases any note the keyboard started that nobody is holding, including on focus loss. `SoundApp::live_midi_log` (bounded) records what the keyboard actually sent, which is what the regression tests assert: one note-on per press, one note-off per release.
- **2026-10-20 (instrument → preset):** Choosing an instrument in the alternative GUI now changes the sound. Three routes, tried in order: the plugin's own **VST3 `IUnitInfo` program list** (listed through the instance's link, selected through the `kIsProgramChange` parameter *and forwarded to the processor* with the next block — a separate controller ignores a controller-only change), then a **state blob saved from that instrument** (`SAVE PRESET` in the console, `presets/<profile>.json`), then a MIDI program change. On this machine Pianoteq 9 publishes a 128-entry `Factory Presets` list whose names come back empty and whose selection leaves its state byte-identical, so the saved-state map is what actually works there; the state path is plugin-agnostic by design. New control methods (`plugin.programs`, `plugin.program`, `plugin.state`, `preset.save`, `preset.apply`) and `verify-plugin --program N` make the whole thing measurable from a script. See `docs/instrument-consoles.md`.
- **2026-10-20 (ALT on the track, and a silent close fixed):** A channel strip's INSTRUMENT row now carries an **ALT** button next to the plugin's GUI button, shown for exactly the hosted VSTs that have a console (matched on the VST3 class id, so it works before any scan) and opening that instrument's alternative GUI with the track selected. Verifying the sound end-to-end needed two control-channel reads that did not exist, so they were added: `mixer.meters` (per-track and master peak/level, the answer to "is there sound?") and `midi.live` (one live MIDI message to an instrument track, the hardware-keyboard path). The chase also found why the app sometimes closed with nothing in the log: the instance released its edit controller while the detached thread was still inside `IPlugView::removed()`, a use-after-free in the plugin that no panic hook can see. The instance now waits for that thread (leaking the objects if the plugin hangs) — see `docs/plugin-verification.md`.
- **2026-10-20 (every instrument, with its pictogram):** The alternative GUI now lists the instruments the hosted VSTs actually ship, and every entry draws its own small pictogram (grand lid, upright case, tines, crossed mallets, bell, strings and bow, brass flare, waveforms, grains) — 16 kinds, drawn from primitives in `console_kit.rs`, so no artwork is loaded and each icon tints with the instrument's palette. **Pianoteq 9** went from 19 to **36** instruments (the core models plus every installed add-on read from `~/.local/share/Modartt/Pianoteq/Addons`: KIViR historical keyboards, KPRAUSCH CL1, Rhody R1, V3 legacy grands, K1, Steinway DV80 New York, Bells and carillon), and **Omnisphere** from 14 curated sounds to **45 of its own patch categories** (Synth Long…Spotlight EDM) taken from its library indexes. The model buttons measure themselves to their labels, and a geometry test asserts every pictogram stays inside its box and differs from every other. See `docs/instrument-consoles.md`. No readiness percentage change.
- **2026-10-20 (Pianoteq Console, an alternative GUI):** A rack-style console window drives a hosted modelled piano from SoundCraft: instrument tabs by category, preset variants, concert pitch and temperament, physical-model racks (tuning, voicing, resonance), an interactive velocity-curve editor with a dynamic-range fader, a reverb/EQ/delay rack, and an 88-key strip with MIDI activity lights. Every control goes out as a MIDI CC through one `dispatch` hook, with a 42-entry parameter map (id, range, unit, CC, Pianoteq parameter name) as the wiring contract; it opens alongside the plugin's own editor instead of replacing it. See `docs/instrument-consoles.md`. No readiness percentage change.
- **2026-10-20 (yabridge upgraded):** Building yabridge from `master` (`b580a9f`) with the Wine cross compiler and re-syncing every bridge fixed the plugins that used to fail on this machine: **Kontakt 8** (was a stack overflow), **DecentSampler** (was a hang), the **UADx 610** pair (was a hang waiting for its service) now load and expose their parameters, and **Kontakt 7's editor opens** instead of crashing. The fix also carries upstream's Wine ≥ 9.22 mouse-click fix. Old install kept as `~/.local/share/yabridge-5.1.1-backup`.
- **2026-10-20 (one console, six instruments):** The alternative GUI became a single window with an instrument tab row: **Pianoteq, Pigments, Omnisphere, Kontakt 7, Analog Lab V, LABS** each bring their own racks, parameter map and **theme** (accent colour, panels, window title) and keep their own values, velocity curve and MIDI channel. A new instrument is a declarative profile in `console_kit.rs`. See `docs/instrument-consoles.md`. The same doc records the **Wine ≥ 9.22 + yabridge 5.1.1 click regression** that makes plugin GUIs ignore the mouse, with the two ways out (yabridge from master, or Wine ≤ 9.21).
- **2026-10-20 (second console, shared kit):** The console machinery became a profile-driven kit (`console_kit.rs`): palette, knob/fader/switch widgets, velocity curve, 88-key strip, per-console state and the single MIDI dispatch hook. **Pigments** got the second profile — two oscillator engines (six models each), filters and utility engine, LFOs and two envelopes, FX rack (delay/reverb/insert), four macros, in its own window with its own shortcut (`Shift+O`) — so a new instrument is declarative, not a copy. See `docs/instrument-consoles.md`.
- **2026-10-18 (control surfaces and the Linux bridge):** A hardware MIDI worker over the ALSA sequencer drives Mackie-style surfaces (iCON P1-M, M-Vave SMC-Mixer): hot-plug resubscription, soft takeover on the faders, a recorded MIDI Learn map persisted in `config.json`, global vs. link-to-controller mode, and bidirectional LED/meter/motor feedback. VST3 editors open in native X11 windows on Linux (with the `Linux::IRunLoop` fd/timer dispatch bridged plugins need), and the plugin scan leaves the UI thread. Audio-thread note bookkeeping is now allocation-free. See `docs/audit-report.md`; readiness percentages unchanged (G-02 and G-10 shrink, hardware-parity rows updated).
- **2026-10-13:** `mix.instrument` takes the built-in instrument's parameters in `params` (validated, clamped) and refuses any other field, so a synth setting written beside `plugin` is an error instead of being accepted and ignored (#265)
- **2026-10-13:** A bounce of a bus that an aux returns onto lines up with the timeline: the aux insert's lookahead is compensated like the main mix's (it left the bus late and cut the end off), and tracks routed to that bus wait for it too (#246)
- **2026-10-13:** Delay compensation covers the inserts of the master faders: a bounce of the main mix with a Maximizer on the master lines up with the same mix without it (it came out 71 samples late) (#351)
- **2026-10-12 (audio device settings):** Setup › Playback Engine chooses the audio system, output and input device, sample rate and buffer size, kept per machine with the preferences; 16/32-bit integer devices are converted; ASIO is available behind the opt-in `asio` build feature (verified with an SSL 2+ at 128 samples). Channel routing, one duplex clock and latency compensation remain (G-01). No readiness percentage change.
- **2026-10-12 (Mix in a separate window):** Window › Mix in Separate Window shows the Mix window in its own native window, so Edit and Mix can be on screen at once (e.g. one per monitor), as in Pro Tools. No readiness percentage change.
- **2026-10-12 (plugin editors on Windows):** VST3 editors and embedded CLAP editors open in a SoundCraft host window on Windows (owned by the app window, resized on request, closed from its title bar); verified with a JUCE VST3 compressor and a CLAP effect. No readiness percentage change.
- **2026-10-12:** `soundcraft-cli convert` rejects an unparsable or zero `--rate` and an unknown `--bit-depth` (exit 1, nothing written) instead of converting with a default (#256)
- **2026-10-12:** Timecode labels with huge fields no longer overflow: frame counts are computed in 128 bits and clamp, so `setup.current_timecode` and `session.format_time` saturate instead of failing (#248)
- **2026-10-12:** `session.format_time` with the smallest bar number no longer fails with an internal error: the bar offset saturates (#249)
- **2026-10-12 (plugin editors on Windows):** VST3 editors and embedded CLAP editors open in a SoundCraft host window on Windows (owned by the app window, resized on request, closed from its title bar); verified with a JUCE VST3 compressor and a CLAP effect. No readiness percentage change.
- **2026-10-12 (Mix in a separate window):** Window › Mix in Separate Window shows the Mix window in its own native window, so Edit and Mix can be on screen at once (e.g. one per monitor), as in Pro Tools. No readiness percentage change.
- **2026-10-12 (audio device settings):** Setup › Playback Engine chooses the audio system, output and input device, sample rate and buffer size, kept per machine with the preferences; 16/32-bit integer devices are converted; ASIO is available behind the opt-in `asio` build feature (verified with an SSL 2+ at 128 samples). Channel routing, one duplex clock and latency compensation remain (G-01). No readiness percentage change.
- **2026-10-10 (UI docking):** Tracks and Groups, Clips, MIDI Editor, Memory Locations, and Undo History use movable panels. Edit and Mix retain separate arrangements; Clips and MIDI Editor belong to Edit. Transport, plug-in, and specialist utility windows retain their existing hosts. Panel moves and reset are available as UI commands, and saved UI preferences retain arrangements. Common toolbar and track record-enable buttons share interaction, accessible labels and keyboard focus through `craft-ui`, retaining blue tool selection and red record-arm states. No readiness percentage change.
- **2026-10-10** — Stage re-normalized to **pre-alpha** by the core-workflow gate (interface recording, MIDI recording and session interchange fail). Full re-measure against Pro Tools Ultimate 2026.4.1 and the standard
  progress-docs set (this page, target-app parity, gaps, roadmap, UI/format/hardware/plugin/
  localization parity). v0.4.0 released. Community fixes: Cut All Automation, snap to neighbours,
  tempo stretch, I/O Setup, clip export names, menu-only dialogs, playback stop on new session.
- **2026-10-09** — Light theme and live system appearance, clip-gain fader and nudge, missing-media
  warnings, session audio health, malformed-JSON and encoder-extension checks in the CLI, macOS
  system menu bar, Windows DX12 default, Linux fontconfig, metronome click in playback.
- **2026-10-08** — v0.2.x–v0.3.0 released (Flatpak, AppImage zsync); VST3 quit crash fixed;
  Ctrl-as-Cmd shortcuts on Windows/Linux; plugin menus by vendor, AU inserts, instrument picker;
  rotating log file; branded DMG.
- **2026-10-07** — Repository published; About window credits; release CI on macOS runners.
- **2026-10-06** — Video track, Audio Unit hosting, plugin state and editors, Renderer; Mix
  window views; score export and print; surround mixing; VST3 hosting; property tests.
- **2026-10-05** — Foundation crates, Edit and Mix windows, playback, CLI/MCP, parity report,
  recording and punch, CLAP hosting, Elastic Audio, Beat Detective, release pipeline.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Live MIDI routing follows the open instrument console; prevents playing a different selected plugin |
| 2026-10-20 | minor | Captured-sound browser in each instrument console; independent plugin-state lists for all six profiles |
| 2026-10-20 | minor | Progress log: instrument pictograms and the complete instrument lists (Pianoteq 36, Omnisphere 45) |
| 2026-10-18 | minor | Control surfaces (ALSA sequencer, hot-plug, soft takeover, MIDI Learn, feedback), Linux X11 VST3 editor windows, background plugin scans, allocation-free live notes, `config.json` |
| 2026-10-20 | minor | Progress log: Pianoteq Console alternative GUI (racks, velocity curve, 88-key strip, MIDI CC map) |
| 2026-10-13 | trivial | Progress log: mix.instrument params, unknown fields refused (#265) |
| 2026-10-13 | trivial | Progress log: bus bounce compensates aux returns (#246) |
| 2026-10-13 | trivial | Progress log: master insert latency compensated (#351) |
| 2026-10-12 | trivial | Progress log: convert option validation (#256) |
| 2026-10-12 | trivial | Progress log: timecode overflow (#248) |
| 2026-10-12 | trivial | Progress log: bar offset saturates (#249) |
| 2026-10-10 | minor | Readiness-by-audience table with hours: full 950–1,450 h, mainstream 380–600 h, essentials 110–200 h |
| 2026-10-10 | minor | Added mainstream practitioner ~36 % and essentials user ~57 % to the headline numbers |
| 2026-10-10 | minor | Stage alpha → pre-alpha: the core-workflow gate fails on interface recording, MIDI recording and session interchange; alpha distance added |
| 2026-10-10 | major | Rewritten to the progress-docs standard: stage alpha (re-normalized the same day), two numbers (breadth ~70 %, ready ~40 %), dimension/feature/language tables, hours re-calibrated; area table moved to docs/target-app-parity.md, milestones to docs/roadmap.md |
| 2026-10-07 | major | Menu parity 94 %, ~65 % estimated feature parity, alpha list |
| 2026-10-06 | minor | Alpha and parity estimates |
