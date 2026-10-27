# Free Linux plugins and the two new built-in instruments

Last revised: 2026-10-24.

Two things live here: the plugins downloaded into
`/run/media/leonardo/Elements/Pianote/plugins/` (grouped by role), and the two built-in
instruments added to SoundCraft as free stand-ins for the commercial ones that do not work under
Wine on this machine.

## Built-in instruments (no plugin, no Wine)

Both are DSP code in `crates/dsp/src/plugins/`, and both have a **Craft Console** (the Alt GUI)
(`crates/ui-egui/src/*_console.rs`) — the ALT button on a track that plays them opens it.

| Id | Name | What it is | Console |
|---|---|---|---|
| `grand_piano` | Grand Piano | struck-string model: 24 voices × 2 strings × 8 inharmonic partials, hammer spectrum that brightens with velocity, damper + sustain pedal, soundboard tail | `grand_console.rs`, 14 parameters, 3 racks, mahogany theme |
| `atmos_pad` | Atmosphere Pad | 16 voices × 3 detuned unison saws + triangle + noise breath, per-voice drift, shimmer octave, stereo ensemble, diffuse tail | `pad_console.rs`, 15 parameters, 3 racks, nebula theme |

Names are deliberately our own: no vendor trademark, no sampled audio, nothing derived from the
proprietary instruments. The consoles drive the instruments by parameter id, and a test
(`the_builtin_consoles_match_their_instrument_parameters`) fails if a console and its instrument
ever disagree about an id or a range.

Both survive the hostile-host test suite (`crates/dsp/tests/plugins.rs`): odd block sizes,
mismatched channel lengths, NaN parameters, note-offs without note-ons.

## Plugins installed under `Pianote/plugins/`

All of these are **native x86_64 Linux VST3** (no Wine, no bridge), taken from the vendors'
releases, and each bundle is symlinked into `~/.vst3/` so any host finds it (SoundCraft, REAPER…).

| Folder | Plugin | Licence | Why it is here |
|---|---|---|---|
| `pads/` | **Surge XT 1.3.4** (+ Surge XT Effects) | GPL-3.0 | the Omnisphere-shaped hole: 2800+ factory patches including a deep pad/texture bank, dual scenes, wavetable/window/FM oscillators, MPE |
| `effects/` | **Dragonfly Reverb 3.2.10** — Hall, Plate, Room, Early Reflections | GPL-3.0 | best-sounding free algorithmic reverb on Linux (its GUI wants GL ≥ 3.0) |
| `effects/` | **MVerb 2020** | AGPL-3.0 | plate reverb, from the `.deb` extracted without root |
| `effects/` | **Airwindows Consolidated** | MIT source / GPL-3.0 binary | ~400 effects in one plugin: tape, console, saturation, dither |
| `pianos/` | **SocaLabs Piano** | AGPL-3.0 | a real native VST3 sampled piano instrument — "simple" per the vendor, fine for scratch use |

Installed layout: each vendor tree keeps its own directory, so `plugins/<role>/<vendor>/…`; the
`.vst3` bundles are found by `scripts/vst-doctor.sh` like any other.

Verified through the host (native plugins are fast, unlike bridged ones):

```
Surge XT   ok  editor: ok  2854 params  3.3 s
Piano      ok  editor: ok  2115 params  3.3 s
```

## Craft Console (the Alt GUI)

The console layer is named **Craft Console**: one window, its own theme per instrument, opened by
the ALT button on a track (or the Window menu, `Shift+P` / `Shift+Alt+P`). It drives both the
bridged plugins and the built-in instruments — for the built-ins it *is* their GUI, because a
parameter written in the console goes straight to `mix.instrument_param`. It stays a separate
window on purpose: a plugin whose own editor does not paint (Direct2D under Wine) is still fully
playable through it.

## Pianobook

`scripts/pianobook.sh` works with the pianobook.co.uk libraries:

```bash
scripts/pianobook.sh list              # 10 packs per category, with links
scripts/pianobook.sh import ~/Downloads  # install the zips you downloaded
scripts/pianobook.sh players           # which player opens which format
```

The generated catalogue is `docs/pianobook-catalogue.md`. Two honest limits, both on the site's
side: it computes "highest rated" in the browser from review data it does not expose (its filter
endpoint answers `{"success": false}` without a session nonce), and downloads need a free account
("Log in to download"), so the packs have to be fetched in a browser — the local cookie store
cannot be reused because Chrome/Edge encrypt it with a GNOME-keyring key and `secret-tool` is not
installed here. Format decides the player: `.dspreset` → Decent Sampler, `.nki` → Kontakt (already
installed through yabridge), `.sfz` → an SFZ player.

## Still missing: a sampled grand

Nothing free ships a *high-quality* native Linux piano **instrument**; the good ones are SFZ
libraries that need an SFZ player. When that is wanted, the pair to use is:

- **SFZ player**: *DSK SFz Player* (Linux VST3, freeware, direct download) or *sfizz* (BSD-2,
  VST3/LV2 — archived upstream and not in apt, so Flathub/OBS/source).
- **Samples**: *Accurate Salamander* (CC-BY 3.0, 1.6 GB per voicing, documented to work with
  sfizz half-pedalling) or *Osiris Piano* (CC0, ~447 MB, beta) — CC0 is the licence with no
  obligations at all.

## Pianos with direct download (no account)

| Source | What | Licence | Status |
|---|---|---|---|
| [FreePats — Kawai upright](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html) | `UprightPianoKW-SFZ+FLAC` (32 MB, 66 FLACs + `.sfz`) | **CC0 1.0** | **installed** in `plugins/pianos/samples/upright-kawai/` |
| FreePats — Salamander Yamaha C5 | SFZ+FLAC, 707 MB | CC-BY 3.0 | available on request |
| [archive.org — Salamander Grand V3](https://archive.org/details/SalamanderGrandPianoV3) | 16 velocity layers | CC-BY 3.0 | available on request |
| `sfzinstruments/Osiris_Piano` (GitHub) | CC0 grand, ~447 MB | CC0 1.0 | available on request |
| `peastman/sso` (GitHub) | Sonatina orchestra + piano | CC Sampling Plus | available on request |

**The blocker is the player, not the samples.** A VST3 SFZ player on Linux cannot be fetched
without an account or a build:

- **sfizz** publishes **no Linux binary** (its GitHub releases are macOS/Windows only) and its OBS
  repository builds openSUSE packages, not `.deb`. Building it needs JUCE plus GL/X11 dev headers
  (`libgl-dev` is missing here and needs root).
- **Decent Sampler** is the format most Pianobook packs use, and its download is account-gated.
- **DSK SFz Player** advertises a Linux VST3 but its download link is not reachable from a script.
- **Kontakt 7/8** already works on this machine (through yabridge), so Kontakt-format libraries are
  the only "player at hand" today.
- The long-term answer that removes the dependency: an **SFZ instrument inside SoundCraft** (parse
  `.sfz` regions, stream the samples through the existing sampler). Not written yet.

## Optional, needs root (`sudo apt install`)

```
zam-plugins            # reverb + delay + tube saturation + granular, VST3 and CLAP
dragonfly-reverb-vst3  # same reverb as above, packaged
lsp-plugins-vst3       # pro EQ/dynamics/delay/modulation suite
```

## Adding a plugin the same way

```bash
DEST=/run/media/leonardo/Elements/Pianote/plugins
# 1. unpack the vendor tree into DEST/<pads|effects|pianos>/<vendor>/
# 2. link the bundle so every host sees it
ln -sfn "$DEST/effects/<vendor>/<Name>.vst3" ~/.vst3/<Name>.vst3
# 3. check it before blaming the host
scripts/vst-doctor.sh && soundcraft --scan-plugins | grep -i <Name>
```

The Elements drive is NTFS mounted `rw` with working exec bits (checked), so bundles can live
there — a VST3 bundle is a directory with a `Contents/x86_64-linux/*.so` that has to be
executable.
