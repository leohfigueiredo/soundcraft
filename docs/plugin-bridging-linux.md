# Windows plugins on Linux (Wine + yabridge) and the low-latency stack

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** first version (native
> X11 editor windows, Linux run loop, yabridge search paths, hot-plug-safe scanning)

SoundCraft hosts native Linux VST3 and CLAP plugins in-process and opens their editors as native
X11 windows (through XWayland on a Wayland session). Windows plugins are reached with
**Wine-Staging + yabridge**: `yabridgectl` turns each Windows `.dll`/`.vst3` into a native Linux VST3
that SoundCraft scans and loads like any other.

```
Wine/Wine-Staging ── yabridge (wine-host) ── yabridge .so (native VST3) ── SoundCraft (in-process)
   Kontakt 7/8            IPC + X11 window             scan / load / editor       audio thread
```

## 1. Install the bridge

```sh
# Debian/Ubuntu example; use the distro's Wine-Staging packages for the rest
sudo dpkg --add-architecture i386                # only for 32-bit plugins
sudo apt install wine-staging winehq-staging    # 9.x or newer
# yabridge: distro package, or the release tarball on the project's page
sudo apt install yabridge yabridgectl           # if packaged
yabridgectl --version
yabridge --version
```

Point Wine at one prefix (the default `~/.wine` is fine) and install Kontakt into it with its
Windows installer. Keep the sample library outside the prefix when possible (e.g.
`~/Sample Libraries/…`): Kontakt streams samples from the host filesystem, and a path under
`~/.wine/drive_c` works too but makes backups awkward.

## 2. Register and sync the folders

```sh
yabridgectl add  "$HOME/.wine/drive_c/Program Files/Common Files/VST3"
yabridgectl add  "$HOME/.wine/drive_c/Program Files/VstPlugins"     # VST2 (needs a VST2 host)
yabridgectl status
yabridgectl sync
```

`yabridgectl sync` writes the bridged plugins under:

| Path | For |
|---|---|
| `~/.vst3/yabridge/*.vst3` | native VST3 wrappers — **this is the path SoundCraft uses** |
| `~/.vst/yabridge/*.so` | VST2 wrappers (no VST2 host in SoundCraft yet) |
| `~/.vst3/yabridge/…` | multi-plugin bundles keep their `Contents/` layout |

SoundCraft's scanner already searches `~/.vst3`, `~/.vst3/yabridge`, `/usr/lib/vst3`,
`/usr/lib/vst3/yabridge`, and every folder added in **Setup ▸ Hardware ▸ VST & CLAP Plugin Folders**
(or through `yabridgectl add`). The scan runs on a worker thread; a cold Wine prefix is slow the
first time, which is why the FX browser never blocks on it.

If a plugin does not appear: **FX Browser ▸ 🌉 Sync Windows VSTs (yabridge)** runs
`yabridgectl sync` again and reloads the catalogue; **🔄 Scan VST3 & CLAP** re-reads the folders.

## 3. Editors on Linux

* A plugin's own window opens as a **native, top-level X11 window** managed by the window manager
  (XWayland under Wayland). Resizing, stacking, focus and the close button behave like any other
  window; nothing is embedded in the DAW's canvas, so Kontakt's browser and text fields do not
  fight the host for input.
* Open it from the **GUI** button on a mixer strip (instrument row), by **double-clicking** the
  plugin name, or from the track FX row (`double-click`).
* Closing the window (title-bar **✕**) hides the editor. The plugin instance, its parameters, its
  loaded library and the audio stay alive; reopening builds a fresh view. The instance is only
  destroyed when the plugin is removed from the slot or the track is deleted.
* Typing inside Kontakt (browser search, file names) goes to the focused plugin window only: the
  DAW's shortcuts and the virtual keyboard see nothing while it is focused.
* Bridged GUIs need three host-side things, all implemented in `crates/vst3-host`:
  `kPlatformTypeX11EmbedWindowID` with a real window id, `IPlugFrame::resizeView` driving
  `XResizeWindow`, and `Linux::IRunLoop` (file descriptors and timers) pumped from the main thread
  every UI frame.

Known limits: VST2 hosting is not implemented (Kontakt 7/8 is VST3-only, so the required workflow is
covered); CLAP editors are still Windows/macOS-only; a plugin that segfaults takes the process with
it (in-process hosting, no sandbox).

## 4. Audio: PipeWire (JACK layer) and ALSA

SoundCraft plays and records through cpal. On Linux that is ALSA (Pulse via the ALSA plugin, PipeWire
via its ALSA compatibility or its JACK layer when the `jack` build feature is on). For a DAW, use
PipeWire directly:

```sh
pipewire --version && pipewire-pulse --version   # 1.0+ recommended
pw-cli info 0 | head                              # server info
pw-top                                            # real-time graph: xruns are the ERR column
pw-metadata -n settings | grep -E 'clock|rate'    # rate, quantum
```

Set the quantum low while tracking (e.g. 256 frames) and higher for mixing. Wine-hosted plugins run
outside the RT graph; loading a multi-gigabyte Kontakt library can still cause a dropout, so:

* preload the instrument before pressing record (play a few notes without the transport running);
* keep the Kontakt sample pool on the same filesystem and out of swap pressure;
* raise Kontakt's own buffer if it exposes one, and avoid streaming from a network share.

## 5. Real-time limits and priorities

Distributions ship a conservative `rtprio`/`memlock` limit. A desktop audio setup needs:

```ini
# /etc/security/limits.d/audio.conf   (or a file in /etc/security/limits.d/)
@audio   -  rtprio     95
@audio   -  memlock    unlimited
# optional, only if you run JACK with the -R flag:
# @audio   -  nice      -19
```

Then make sure the user is in the group and raise the limits in the session (the limits file only
applies to *new* sessions, and some display managers ignore PAM limits entirely — `systemd` user
sessions read `~/.config/systemd/user.conf` `DefaultLimitRTPRIO`/`DefaultLimitMEMLOCK`):

```sh
sudo usermod -aG audio "$USER"          # then log out and in
ulimit -r -l                            # rtprio 95, memlock unlimited
chrt -f 80 true && echo "realtime scheduling works"
```

SoundCraft's own audio thread inherits the scheduler it started with; the plugin host and the MIDI
worker are ordinary threads.

## 6. Validation checklist

| # | Check | Command | Healthy result |
|---|---|---|---|
| 1 | Wine present | `wine --version` | `wine-9.x` (Staging recommended) |
| 2 | Bridge present | `yabridgectl --version && yabridge --version` | matching versions |
| 3 | Kontakt installed in the prefix | `ls ~/.wine/drive_c/Program\ Files/Common\ Files/VST3` | `Kontakt 7.vst3` (or 8) |
| 4 | Folders registered | `yabridgectl status` | the VST3 and VST2 folders listed |
| 5 | Wrappers generated | `yabridgectl sync` then `ls ~/.vst3/yabridge` | `Kontakt 7.vst3` present |
| 6 | SoundCraft sees it | FX Browser search `Kontakt` | the plugin listed with a `VST3` badge |
| 7 | Loads | instrument slot ▸ pick Kontakt | no status error; the library browser appears in its window |
| 8 | Editor opens detached | **GUI** button / double-click the name | a normal window with title-bar close button |
| 9 | Close hides, audio survives | close with **✕**, play notes | audio still sounds; GUI reopens |
| 10 | Typing isolated | click into Kontakt's browser, type | no transport shortcuts fire, no virtual-keyboard notes |
| 11 | Real-time limits | `ulimit -r -l`, `chrt -f 80 true` | 95/unlimited, no permission error |
| 12 | Graph health | `pw-top` while loading the library | `ERR` stays at 0 at your quantum |
| 13 | X11 available to the app | `echo "$DISPLAY"` (Wayland: `xlsclients`) | a display id; XWayland answering |
| 14 | No stale wrappers after a plugin update | `yabridgectl sync` | "no differences" or the changed plugin only |

## 7. Troubleshooting

| Symptom | Likely cause | Action |
|---|---|---|
| Black or blank plugin window | GUI attached before the Wine side painted, or XWayland missing | confirm `$DISPLAY`; close and reopen the GUI; check the Wine prefix starts (`wine notepad`) |
| Window flickers or resizes itself | the plugin asked for a platform it does not support | SoundCraft rejects the editor with a status message; try the VST3 build |
| Plugin missing from the list | wrapper not synced, or scanned before `yabridgectl sync` | **🌉 Sync Windows VSTs (yabridge)**, then **🔄 Scan** |
| First load hangs for 20 s | Wine prefix cold start, Kontakt library scan | expected once; the scan is on a worker thread and the UI stays responsive |
| The window froze while scanning a folder | a scan ran on the UI thread (fixed: scans now only run on the worker; `apply_to_hosts` just registers folders) | update to a build with this fix; if it still freezes, report the exact action (which button/menu) |
| The app closed while scanning | scanning used to load plugins in-process; one crashing plugin killed the editor | fixed: scanning runs in the `soundcraft --scan-plugins` child process and its result is adopted back, so a crashing plugin only kills the scanner |
| A drive of `.dll`/Windows `.vst3` scans and finds nothing | the native hosts only load Linux ELF plugins; Windows plugins need `yabridgectl sync` | install yabridge, run `yabridgectl add <folder> && yabridgectl sync` (the FX browser's Sync button does it), then rescan |
| Clicks/dropouts when loading a library | disk pressure or too low a quantum | larger quantum, preload the patch, keep samples on a fast local disk |
| `yabridge` warns about realtime | the Wine-side thread is not RT | normal for bridged plugins; keep Kontakt's voice count sane |
| Audio device busy | PipeWire/Pulse holding the device | stop other players, or pick another device in Setup ▸ Playback Engine |
