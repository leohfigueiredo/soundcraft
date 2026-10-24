# Instrument consoles: alternative GUIs for hosted instruments

> **Last reviewed:** 2026-10-20 · **Last updated:** 2026-10-20 · **Change:** captured-sound browser, profile-scoped routing, and stopped-transport plugin processing · **Target:** Avid Pro Tools Ultimate 2026.4.1

One rack-style console window that drives a hosted instrument from inside SoundCraft. It carries an
**instrument tab row**: choosing another instrument swaps the racks, the accent colour and the window
title, and every instrument keeps its own values. It is an **alternative** GUI — it opens in its own
window (`Window ▸ Instrument Console`, `Shift+P`, or the `ALT GUI ▾` toolbar menu, or the keyboard
dock's `ALT GUI` button) and never closes, replaces or takes over the plugin's own editor window.

Instruments: **Pianoteq, Pigments, Omnisphere, Kontakt 7, Analog Lab V, LABS**. All UI text is
English. Each console's state (values, preset, velocity curve, voices, MIDI channel) is persisted in
`ui.json`, keyed by instrument id.

| Instrument | Kind | Theme | Racks |
|---|---|---|---|
| Pianoteq | modelled piano | graphite + amber | tuning & string, voicing, resonance, effects |
| Pigments | hybrid synth | near-black + magenta | engine 1, engine 2, filters & utility, modulation, FX, macros |
| Omnisphere | hybrid synth | indigo + cyan | part, layer A, layer B, filter & mod, FX |
| Kontakt 7 | sampler | graphite + NI orange | instrument, voice, filter, amplifier, FX |
| Analog Lab V | preset synth | charcoal + electric blue | layer A, layer B, macros, FX |
| LABS | sampled instruments | slate + warm gold | instrument, mic positions, shape, reverb |

## What is on screen

| Block | Contents |
|---|---|
| Tab row | one tab per instrument, each with its own pictogram and accent colour; the active one lights up |
| Instrument list | every instrument/bank the selected VST ships, grouped in category columns, each name behind its own pictogram |
| Header | brand + kind, category/bank tabs, preset-variant dropdown, master knobs, MIDI-out toggle and channel |
| Racks | two columns: knobs with arc meters (drag, shift = fine, double-click = default) and button rows for discrete choices |
| Dynamic response | interactive 9-point velocity curve (`Line`/`Exp`/`Log`/`S-Curve`, drag any point), dynamic-range fader where the instrument has one, voice limit, sustain |
| Footer | 88-key strip (A0–C8) with played notes lit, MIDI in/out lights, CPU (ms/frame), voice count, last CC sent |

### From the track to the console

A track's own header in the Edit window carries an **ALT GUI** badge beside ACT (only for tracks
whose hosted plugin has a console), the track's right-click menu gains **Alternative GUI (…)** for
the same tracks, and **Window ▸ Alternative GUI ▸ This Track** (`Shift+Alt+P`) opens the console of
whatever the selected track holds. In the Mix window, a channel strip's INSTRUMENT row carries the
plugin's own **GUI** button and, beside it, an **ALT** button that appears for exactly the hosted plugins a console exists for (matched on the plugin's
class id, so a session's track knows its console even before the catalogue has been scanned).
Clicking ALT selects that track and opens its instrument's console, so the console's notes and
controllers go to that VST. A plugin with no console (an EQ, a reverb) shows no ALT button at all,
instead of a button that does nothing.

With the audio device running, hosted plugins continue processing while the transport is stopped so
their internal state and tails are not globally suspended. MIDI clips do not play at the frozen
playhead in this mode; only live MIDI input reaches instruments.

While the console is open, its selected instrument profile scopes live MIDI input to tracks carrying
that matching plugin. If no matching track is loaded, the virtual keyboard stays silent rather than
playing a different selected instrument by accident. Closing the console restores the normal
selected/armed instrument routing.

### Instrument → preset

Picking an instrument in the console tries three routes, in order, and the status line says which
one answered:

1. **The plugin's own program list** (VST3 `IUnitInfo`). `plugin_programs` reads
   `getProgramListCount`/`getProgramListInfo`/`getProgramName` through the instance's link, and
   `select_plugin_program` sets the controller parameter flagged `kIsProgramChange` (its `ParamID`
   is the list id) and **forwards the same normalized value to the processor** with the next block —
   with a separate controller (Pianoteq, Kontakt) the processor only learns about it that way.
   The instrument's `preset` key is matched case-insensitively against the program names; `None`
   means its own name is the key.
2. **A state saved from that instrument** (`SAVE PRESET` in the console, or `preset.save`). The
   blob is the plugin's `save_state`, base64 in `presets/<profile>.json` next to `ui.json`, and it
   is applied with `load_state` on the **live** instance, so no session reload is involved. This is
   the route that works for the plugins whose program list is a stub.
3. **A MIDI program change**, as before, for plugins that only listen to that.

Measured on this machine: Pianoteq 9 publishes one 128-entry list (`Factory Presets`) whose
`getProgramName` answers nothing, and selecting a program leaves its state byte-identical — the
list exists and is useless. Kontakt/Pigments answer the same way. So for them `SAVE PRESET` or the
captured-sound browser is the working path.

### Captured-sound browser

Every instrument console has a **CAPTURED SOUNDS** row. Choose a sound in that plugin's own
window, type the name shown by the plugin, then click **SAVE CURRENT** to capture its live plugin
state. The selected track must contain that console's instrument. The list is stored separately for each profile (`presets/<profile>.json`), so identical
names in Pianoteq, Kontakt, Omnisphere, Pigments, Analog Lab and LABS do not collide. Choose a
captured entry and click **LOAD** to restore it into the active instrument track, or **DELETE** to
remove it. These are user-captured plugin states, not bundled presets. Plugin names are entered by
the user because these VSTs do not expose a reliable current-preset name to SoundCraft.

Reachable from a script: `plugin.programs {track, slot?}`, `plugin.program {track, list, program}`,
`plugin.state {track}` (a fingerprint, to prove a program change did something), `preset.save`
and `preset.apply` (`{track, profile, name}`), plus `verify-plugin --program N` on the CLI.

### Pictograms

Each instrument carries an [`Icon`], drawn from primitives in `crates/ui-egui/src/console_kit.rs`
(a lid curve, a tine pair, crossed mallets, a bell, a waveform…). Nothing is loaded from the plugin:
the pictograms are code, so they scale, tint with the instrument's palette and need no assets.
`Icon::shapes` returns the drawing, which lets the tests assert the geometry without a GPU.

The instrument lists come from the plugins themselves, not from a wish list:

| Console | Instruments listed | Where the list comes from |
|---|---|---|
| Pianoteq 9 | 36 | the seven installed add-ons (`~/.local/share/Modartt/Pianoteq/Addons/*.ptq`: KIViR, KPRAUSCH, R1, V3, K1, Steinway DV80, Bells) plus the core models |
| Omnisphere | 45 | the patch categories inside the plugin's own library indexes (`…/STEAM/Omnisphere/Settings Library/Patches/Factory/*.db`) |
| Pigments | 13 | the console's own macro banks |
| Kontakt 7 | 11 + the scanned library list | `scripts/kontakt-libraries.py` |
| Analog Lab V / LABS | 13 / 11 | per-instrument sound banks |

## Wiring it up

Every control ends in one hook — the only place that knows how values leave the app:

```rust
console_kit::dispatch(app, &omnisphere_console::PROFILE, "filter_cutoff", 4200.0); // continuous
console_kit::select(app, &pigments_console::PROFILE, "engine1_type", 3);           // discrete choice
console_kit::play_note(app, &kontakt_console::PROFILE, 60, 100, true);             // keyboard
```

Each stores the value in that instrument's state, formats it as a 7-bit CC and sends it to the
active instrument track through `SoundApp::send_live_midi` (the path the virtual keyboard's notes
take). Replace that function's body to use **WebMIDI**, a **WebSocket/OSC** bridge or a **JUCE**
backend — the tables below travel with it.

Notes go through the velocity curve before they leave, so it applies to every instrument.

## Adding another instrument

A console is a profile: the palette, widgets, state and dispatch live in `console_kit.rs`, and a
profile declares its racks, parameters, banks and its own `Palette`:

```rust
pub static PROFILE: Profile = Profile { id: "omnisphere", kind: "hybrid synth", palette: PALETTE,
    brand: "OMNISPHERE", name: "CONSOLE", tagline: "…", categories: CATEGORIES, models: MODELS,
    variants: VARIANTS, master: HEADER, racks: RACKS, specs: SPECS, selects: SELECTS,
    velocity_curve: true, program_change: true };
```

Add it to `console_kit::profiles()` and give it a menu command; nothing else changes.

## Omnisphere parameter map

| Rack | id | Control | Range | Unit | Default | MIDI CC | Parameter |
|---|---|---|---|---|---|---|---|
| Part | `part_level` | PART LEVEL | 0.0 … 100.0 | % | 80.0 | CC 20 | Part Level |
| Part | `part_pan` | PART PAN | -100.0 … 100.0 | % | 0.0 | CC 21 | Part Pan |
| Part | `part_tune` | PART TUNE | -24.0 … 24.0 | st | 0.0 | CC 22 | Part Tune |
| Part | `part_glide` | GLIDE | 0.0 … 2000.0 | ms | 0.0 | CC 23 | Portamento |
| Layer A | `layer_a_level` | A LEVEL | 0.0 … 100.0 | % | 100.0 | CC 24 | Layer A Level |
| Layer A | `layer_a_pan` | A PAN | -100.0 … 100.0 | % | 0.0 | CC 25 | Layer A Pan |
| Layer A | `layer_a_tune` | A TUNE | -24.0 … 24.0 | st | 0.0 | CC 26 | Layer A Tune |
| Layer A | `layer_a_fine` | A FINE | -100.0 … 100.0 | ct | 0.0 | CC 27 | Layer A Fine Tune |
| Layer A | `layer_a_unison` | A UNISON | 1.0 … 8.0 | — | 1.0 | CC 28 | Layer A Unison |
| Layer A | `layer_a_detune` | A DETUNE | 0.0 … 100.0 | % | 15.0 | CC 29 | Layer A Detune |
| Layer B | `layer_b_level` | B LEVEL | 0.0 … 100.0 | % | 60.0 | CC 30 | Layer B Level |
| Layer B | `layer_b_pan` | B PAN | -100.0 … 100.0 | % | 0.0 | CC 31 | Layer B Pan |
| Layer B | `layer_b_tune` | B TUNE | -24.0 … 24.0 | st | 0.0 | CC 32 | Layer B Tune |
| Layer B | `layer_b_fine` | B FINE | -100.0 … 100.0 | ct | 0.0 | CC 33 | Layer B Fine Tune |
| Layer B | `layer_b_unison` | B UNISON | 1.0 … 8.0 | — | 1.0 | CC 34 | Layer B Unison |
| Layer B | `layer_b_detune` | B DETUNE | 0.0 … 100.0 | % | 15.0 | CC 35 | Layer B Detune |
| Filter & mod | `filter_cutoff` | CUTOFF | 20.0 … 20000.0 | Hz | 12000.0 | CC 36 | Filter Cutoff |
| Filter & mod | `filter_reso` | RESONANCE | 0.0 … 100.0 | % | 12.0 | CC 37 | Filter Resonance |
| Filter & mod | `filter_env` | ENV AMOUNT | -100.0 … 100.0 | % | 25.0 | CC 38 | Filter Env Amount |
| Filter & mod | `filter_keytrack` | KEYTRACK | -100.0 … 100.0 | % | 0.0 | CC 39 | Filter Keytrack |
| Filter & mod | `lfo1_rate` | LFO RATE | 0.05 … 40.0 | Hz | 0.8 | CC 40 | LFO Rate |
| Filter & mod | `lfo1_depth` | LFO DEPTH | 0.0 … 100.0 | % | 0.0 | CC 41 | LFO Depth |
| Filter & mod | `env_attack` | ATTACK | 0.0 … 20.0 | s | 0.02 | CC 42 | Attack |
| Filter & mod | `env_decay` | DECAY | 0.0 … 20.0 | s | 0.6 | CC 43 | Decay |
| Filter & mod | `env_sustain` | SUSTAIN | 0.0 … 100.0 | % | 70.0 | CC 44 | Sustain |
| Filter & mod | `env_release` | RELEASE | 0.0 … 30.0 | s | 0.8 | CC 45 | Release |
| FX rack | `reverb_size` | REVERB SIZE | 0.0 … 100.0 | % | 60.0 | CC 46 | Reverb Size |
| FX rack | `reverb_decay` | REV DECAY | 0.2 … 30.0 | s | 4.0 | CC 47 | Reverb Decay |
| FX rack | `reverb_mix` | REV MIX | 0.0 … 100.0 | % | 20.0 | CC 48 | Reverb Mix |
| FX rack | `delay_time` | DELAY TIME | 1.0 … 2000.0 | ms | 375.0 | CC 49 | Delay Time |
| FX rack | `delay_feedback` | DELAY FB | 0.0 … 100.0 | % | 28.0 | CC 50 | Delay Feedback |
| FX rack | `delay_mix` | DELAY MIX | 0.0 … 100.0 | % | 0.0 | CC 51 | Delay Mix |
| FX rack | `chorus_rate` | CHORUS RATE | 0.05 … 8.0 | Hz | 0.4 | CC 52 | Chorus Rate |
| FX rack | `chorus_depth` | CHORUS DEPTH | 0.0 … 100.0 | % | 0.0 | CC 53 | Chorus Depth |
| FX rack | `drive_amount` | DRIVE | 0.0 … 100.0 | % | 0.0 | CC 54 | Distortion Drive |
| FX rack | `drive_mix` | DRIVE MIX | 0.0 … 100.0 | % | 0.0 | CC 55 | Distortion Mix |
| Header | `master_volume` | OUTPUT | -60.0 … 6.0 | dB | -6.0 | CC 7 | Master Volume |
| Header | `master_tune` | TUNE | -100.0 … 100.0 | ct | 0.0 | CC 8 | Master Tune |

| Rack | id | Control | Range | Unit | Default | MIDI CC | Parameter |
|---|---|---|---|---|---|---|---|
| Instrument | `instrument_tune` | TUNE | -100.0 … 100.0 | ct | 0.0 | CC 20 | Tune |
| Instrument | `instrument_transpose` | TRANSPOSE | -24.0 … 24.0 | st | 0.0 | CC 21 | Transpose |
| Instrument | `instrument_start` | SAMPLE START | 0.0 … 100.0 | % | 0.0 | CC 22 | Sample Start |
| Instrument | `instrument_xfade` | LOOP XFADE | 0.0 … 100.0 | % | 20.0 | CC 23 | Loop Crossfade |
| Instrument | `instrument_level` | INST LEVEL | 0.0 … 100.0 | % | 80.0 | CC 24 | Instrument Level |
| Voice | `voice_unison` | UNISON | 1.0 … 8.0 | — | 1.0 | CC 25 | Unison Voices |
| Voice | `voice_detune` | DETUNE | 0.0 … 100.0 | % | 10.0 | CC 26 | Unison Detune |
| Voice | `voice_glide` | GLIDE | 0.0 … 2000.0 | ms | 0.0 | CC 27 | Portamento |
| Voice | `voice_stereo` | STEREO | 0.0 … 100.0 | % | 50.0 | CC 28 | Stereo Width |
| Filter | `filter_cutoff` | CUTOFF | 20.0 … 20000.0 | Hz | 14000.0 | CC 29 | Filter Cutoff |
| Filter | `filter_reso` | RESONANCE | 0.0 … 100.0 | % | 8.0 | CC 30 | Filter Resonance |
| Filter | `filter_env` | ENV AMOUNT | -100.0 … 100.0 | % | 0.0 | CC 31 | Filter Env Amount |
| Filter | `filter_keytrack` | KEYTRACK | -100.0 … 100.0 | % | 0.0 | CC 32 | Filter Keytrack |
| Amplifier | `amp_attack` | ATTACK | 0.0 … 20.0 | s | 0.0 | CC 33 | Attack |
| Amplifier | `amp_decay` | DECAY | 0.0 … 20.0 | s | 0.5 | CC 34 | Decay |
| Amplifier | `amp_sustain` | SUSTAIN | 0.0 … 100.0 | % | 90.0 | CC 35 | Sustain |
| Amplifier | `amp_release` | RELEASE | 0.0 … 30.0 | s | 0.4 | CC 36 | Release |
| Amplifier | `amp_gain` | GAIN | -60.0 … 12.0 | dB | 0.0 | CC 37 | Amplifier Gain |
| FX rack | `eq_low` | EQ LOW | -18.0 … 18.0 | dB | 0.0 | CC 38 | EQ Low |
| FX rack | `eq_high` | EQ HIGH | -18.0 … 18.0 | dB | 0.0 | CC 39 | EQ High |
| FX rack | `fx_reverb_mix` | REVERB MIX | 0.0 … 100.0 | % | 12.0 | CC 40 | Reverb Mix |
| FX rack | `fx_reverb_size` | REVERB SIZE | 0.0 … 100.0 | % | 40.0 | CC 41 | Reverb Size |
| FX rack | `fx_comp_ratio` | COMP RATIO | 1.0 … 20.0 | :1 | 4.0 | CC 42 | Compressor Ratio |
| FX rack | `fx_saturation` | SATURATION | 0.0 … 100.0 | % | 0.0 | CC 43 | Saturation |
| Header | `master_volume` | OUTPUT | -60.0 … 6.0 | dB | -6.0 | CC 7 | Master Volume |

| Rack | id | Control | Range | Unit | Default | MIDI CC | Parameter |
|---|---|---|---|---|---|---|---|
| Layer A | `layer_a_level` | A LEVEL | 0.0 … 100.0 | % | 100.0 | CC 20 | Layer A Level |
| Layer A | `layer_a_pan` | A PAN | -100.0 … 100.0 | % | 0.0 | CC 21 | Layer A Pan |
| Layer A | `layer_a_tune` | A TUNE | -24.0 … 24.0 | st | 0.0 | CC 22 | Layer A Tune |
| Layer A | `layer_a_fine` | A FINE | -100.0 … 100.0 | ct | 0.0 | CC 23 | Layer A Fine Tune |
| Layer A | `layer_a_cutoff` | A CUTOFF | 20.0 … 20000.0 | Hz | 12000.0 | CC 24 | Layer A Cutoff |
| Layer A | `layer_a_reso` | A RESO | 0.0 … 100.0 | % | 10.0 | CC 25 | Layer A Resonance |
| Layer B | `layer_b_level` | B LEVEL | 0.0 … 100.0 | % | 0.0 | CC 26 | Layer B Level |
| Layer B | `layer_b_pan` | B PAN | -100.0 … 100.0 | % | 0.0 | CC 27 | Layer B Pan |
| Layer B | `layer_b_tune` | B TUNE | -24.0 … 24.0 | st | 0.0 | CC 28 | Layer B Tune |
| Layer B | `layer_b_fine` | B FINE | -100.0 … 100.0 | ct | 0.0 | CC 29 | Layer B Fine Tune |
| Layer B | `layer_b_cutoff` | B CUTOFF | 20.0 … 20000.0 | Hz | 12000.0 | CC 30 | Layer B Cutoff |
| Layer B | `layer_b_reso` | B RESO | 0.0 … 100.0 | % | 10.0 | CC 31 | Layer B Resonance |
| Macros | `macro1` | MACRO 1 | 0.0 … 100.0 | % | 0.0 | CC 32 | Macro 1 |
| Macros | `macro2` | MACRO 2 | 0.0 … 100.0 | % | 0.0 | CC 33 | Macro 2 |
| Macros | `macro3` | MACRO 3 | 0.0 … 100.0 | % | 0.0 | CC 34 | Macro 3 |
| Macros | `macro4` | MACRO 4 | 0.0 … 100.0 | % | 0.0 | CC 35 | Macro 4 |
| FX rack | `drive_amount` | DRIVE | 0.0 … 100.0 | % | 0.0 | CC 36 | Distortion Drive |
| FX rack | `drive_mix` | DRIVE MIX | 0.0 … 100.0 | % | 0.0 | CC 37 | Distortion Mix |
| FX rack | `chorus_rate` | CHORUS RATE | 0.05 … 8.0 | Hz | 0.5 | CC 38 | Chorus Rate |
| FX rack | `chorus_depth` | CHORUS DEPTH | 0.0 … 100.0 | % | 0.0 | CC 39 | Chorus Depth |
| FX rack | `delay_time` | DELAY TIME | 1.0 … 2000.0 | ms | 350.0 | CC 40 | Delay Time |
| FX rack | `delay_feedback` | FEEDBACK | 0.0 … 100.0 | % | 25.0 | CC 41 | Delay Feedback |
| FX rack | `delay_mix` | DELAY MIX | 0.0 … 100.0 | % | 0.0 | CC 42 | Delay Mix |
| FX rack | `reverb_size` | REVERB SIZE | 0.0 … 100.0 | % | 50.0 | CC 43 | Reverb Size |
| FX rack | `reverb_mix` | REVERB MIX | 0.0 … 100.0 | % | 15.0 | CC 44 | Reverb Mix |
| Header | `master_volume` | OUTPUT | -60.0 … 6.0 | dB | -6.0 | CC 7 | Master Volume |
| Header | `master_tune` | TUNE | -100.0 … 100.0 | ct | 0.0 | CC 8 | Master Tune |

| Rack | id | Control | Range | Unit | Default | MIDI CC | Parameter |
|---|---|---|---|---|---|---|---|
| Instrument | `level` | LEVEL | 0.0 … 100.0 | % | 80.0 | CC 20 | Level |
| Instrument | `tune` | TUNE | -100.0 … 100.0 | ct | 0.0 | CC 21 | Tune |
| Instrument | `attack` | ATTACK | 0.0 … 10.0 | s | 0.02 | CC 22 | Attack |
| Instrument | `release` | RELEASE | 0.0 … 20.0 | s | 0.6 | CC 23 | Release |
| Instrument | `dynamics` | DYNAMICS | 0.0 … 100.0 | % | 50.0 | CC 24 | Dynamics |
| Instrument | `expression` | EXPRESSION | 0.0 … 100.0 | % | 80.0 | CC 25 | Expression |
| Mic positions | `mic_close` | CLOSE | 0.0 … 100.0 | % | 70.0 | CC 26 | Close Mic |
| Mic positions | `mic_tree` | TREE | 0.0 … 100.0 | % | 40.0 | CC 27 | Tree Mic |
| Mic positions | `mic_ambient` | AMBIENT | 0.0 … 100.0 | % | 20.0 | CC 28 | Ambient Mic |
| Mic positions | `mic_width` | WIDTH | 0.0 … 200.0 | % | 100.0 | CC 29 | Stereo Width |
| Shape | `shape_cutoff` | CUTOFF | 20.0 … 20000.0 | Hz | 16000.0 | CC 30 | Low Pass Cutoff |
| Shape | `shape_reso` | RESONANCE | 0.0 … 100.0 | % | 5.0 | CC 31 | Resonance |
| Shape | `shape_drive` | DRIVE | 0.0 … 100.0 | % | 0.0 | CC 32 | Drive |
| Reverb | `reverb_size` | SIZE | 0.0 … 100.0 | % | 60.0 | CC 33 | Reverb Size |
| Reverb | `reverb_decay` | DECAY | 0.2 … 30.0 | s | 5.0 | CC 34 | Reverb Decay |
| Reverb | `reverb_predelay` | PRE-DELAY | 0.0 … 200.0 | ms | 25.0 | CC 35 | Reverb Pre-delay |
| Reverb | `reverb_mix` | MIX | 0.0 … 100.0 | % | 30.0 | CC 36 | Reverb Mix |
| Header | `master_volume` | OUTPUT | -60.0 … 6.0 | dB | -6.0 | CC 7 | Master Volume |

## Kontakt 7 parameter map


## Analog Lab V parameter map


## LABS parameter map


## Pianoteq and Pigments

Their maps live in `crates/ui-egui/src/pianoteq_console.rs` and `pigments_console.rs` (same shape:
id, label, rack, range, unit, default, MIDI CC, plugin parameter name).

## Known host-side issue: Wine ≥ 9.22 and yabridge 5.1.1

Plugin GUIs whose **mouse clicks do not register** (the window opens and draws, but buttons, knobs
and editors ignore the mouse) are a known Wine regression, not a SoundCraft bug: Wine bug
[59822](https://list.winehq.org/hyperkitty/list/wine-bugs@list.winehq.org/message/FZHHZDIJQTNJEQVJNEX2WOX4E3CLNFWM/)
("Mouse clicks not received in VST plugin UIs (child windows) since Wine 9.22"), fixed in
yabridge's `master` branch ("Fixed a compatibility issue with Wine 9.22 and above that caused mouse
clicks in plugin GUIs to not register properly") but **not** in the 5.1.1 release. The same
changelog fixes a hang that prebuilt yabridge binaries caused on Ubuntu 24.10+ hosts.

The reference machine was upgraded on 2026-10-20: yabridge built from `master` (`b580a9f`) and every
bridge re-synced; afterwards **Kontakt 8, DecentSampler and the UADx plugins load** (they all failed
before) and Kontakt 7's editor opens instead of crashing. Details, measurements and the rollback
(`~/.local/share/yabridge-5.1.1-backup`) are in `docs/plugin-verification.md`.

SoundCraft's own GUI side is in order regardless: the run loop (`Linux::IRunLoop`) is implemented
and pumped, the plugin window is resized to the editor's size, and the editor window takes focus —
measured against a bridged piano and synth in `docs/plugin-verification.md`.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Captured-sound browser for all six profiles; live MIDI is routed only to the matching console plugin |
| 2026-10-20 | major | Instrument → preset: VST3 `IUnitInfo` program lists (`kIsProgramChange` forwarded to the processor), the `SAVE PRESET` state map per console, and the control methods to drive both |
| 2026-10-20 | minor | ALT button on the channel strip's instrument row (per-VST class id), `mixer.meters` and `midi.live` control methods for sound verification |
| 2026-10-20 | major | Instrument pictograms drawn in code; complete instrument lists (Pianoteq 36 = core + seven add-ons, Omnisphere 45 = its own patch categories) |
| 2026-10-20 | major | One console window with an instrument tab row, six profiles, per-instrument themes; Wine ≥ 9.22 click regression documented |
| 2026-10-20 | minor | Second profile: Pigments (engines, filters, modulation, FX, macros) on a shared kit |
| 2026-10-20 | created | Pianoteq console: racks, velocity curve, 88-key strip, MIDI CC dispatch and parameter map |
