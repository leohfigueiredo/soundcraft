# Plugin verification: every installed VST3/CLAP, measured

> **Last reviewed:** 2026-10-19 · **Last updated:** 2026-10-19 · **Change:** created (per-plugin verification harness; bridged-Windows findings) · **Target:** Avid Pro Tools Ultimate 2026.4.1

Third-party hosting is only useful if the plugins a user owns actually come up. This document
records how every installed plugin is exercised and what the last full run found on the reference
Linux machine (yabridge 5.1.1 + Wine Staging 11.19, 53 plugins: 36 VST3, 17 CLAP).

## How a plugin is verified

Two scripts, both committed, both runnable anywhere:

```sh
scripts/scan-plugins.py                 # the whole catalogue, skipping bundles that crash the scanner
scripts/verify-plugins.py               # core pass: load, instantiate, prepare, process, state
scripts/verify-plugins.py --editor      # the same, then open and close the plugin's GUI
scripts/verify-plugins.py --only Piano  # one plugin (substring of the name or path)
```

Each plugin is exercised by `soundcraft --verify-plugin <bundle> <id>` **in a child process**,
because a plugin that segfaults, aborts or hangs is the normal case with bridged Windows plugins.
The child prints its stage to stderr (`load_bundle`, `instantiate`, `prepare`, `process`, `state`,
`editor`) so a crash names the call that caused it, and one JSON line with the result. The driver
adds a timeout, so a plugin that never returns costs one plugin of the batch, not the run.

What the core pass checks, in the same order the audio side does:

| Stage | What it means if it fails |
|---|---|
| `load_bundle` | the bundle is not a loadable VST3/CLAP module (missing binary, wrong format) |
| `instantiate` | the plugin (or its Wine host) refuses to be created — often a crash, a licence check or a stack overflow inside the plugin |
| `prepare` | bus layouts / sample rate refused, or the plugin aborts while activating |
| `process` | the plugin crashes or emits NaN/infinity while running |
| `state` | saving its state aborts the plugin, or a second instance refuses the state it saved |

`process` feeds one block of silence and one of a 0.25 sine (effects) or a note-on (instruments),
and `state` saves the plugin's state and loads it into a fresh instance: a plugin that cannot
round-trip its own state loses the user's patch on save, which is worse than not loading at all.

## Last full run (reference machine)

48 of 53 plugins verified: 36/36 VST3 modules present in the catalogue load, and every CLAP.

| Plugin | Format | Params | State | Audio on a plain note/tone | Notes |
|---|---|---|---|---|---|
| Analog Lab V | VST3 (yabridge) | 2591 | none | silence (needs a preset) | no state at all — the plugin keeps its patch in its own window |
| Kontakt 7 | VST3 (yabridge) | 3120 | 6 KB | silence (needs a library) | |
| Kontakt 8 | VST3 (yabridge) | — | — | — | **fails**: the plugin overflows its stack inside Wine during `instantiate` |
| Omnisphere | VST3 (yabridge) | 3097 | 41 B | silence (needs a patch) | |
| Pianoteq 8 | VST3 (yabridge) | 2302 | 39 KB | 0.17 | |
| Pianoteq 9 | VST3 (yabridge) | 2294 | 41 KB | 0.21 | **fixed**, see below |
| Pigments | VST3 (yabridge) | 4067 | 284 KB | 0.13 | |
| LABS | VST3 (yabridge) | 2133 | 304 B | silence (needs a library) | |
| TRITON | VST3 (yabridge) | 2573 | 493 KB | silence (needs a patch) | |
| ZENOLOGY | VST3 (yabridge) | 301 | 163 KB | 0.004 | |
| Vital | VST3 (yabridge) | 2854 | 175 KB | 0.34 | |
| ValhallaSupermassive | VST3 (yabridge) | 18 | 706 B | 0.18 | |
| Strings Q3, Serenity 2 Lite, Dub Stage Piano | VST3 (yabridge) | 162–2174 | 1–4 KB | 0 | |
| 17 zam-plugins effects | VST3 + CLAP (native) | 12–40 | 163–175 B | passes | ZamEQ2 emits NaN on a plain sine: the mixer silences the block and takes it out of the path |
| DecentSampler | VST3 (yabridge) | — | — | — | **fails**: stack overflow inside Wine during `instantiate` |
| UADx 610-A / 610-B | VST3 (yabridge) | — | — | — | **fails**: waits forever for the UAD Connect service (`TPipe::open … CreateFile errored`) during `process` |
| Vaults | VST3 (yabridge) | — | — | — | **fails**: aborts the Wine host (`c0000409`) as soon as it is asked to run |

None of the failures can hurt the editor: a third-party plugin is only loaded in-process after the
same calls succeeded in a child process (`--probe-plugin`), and the probe now runs the plugin and
saves its state — not just instantiating it — because Vaults passed the old probe and then died
the moment it processed audio. A plugin that fails validation stays out of the signal path and the
FX list says why (`loading…` / `did not load`).

## Plugin GUIs: what a host must do, and what was missing

A VST3 editor is an `IPlugView` attached to a host window (`X11EmbedWindowID` on Linux). Attaching
it is only half of it: under Linux a plugin view that draws with JUCE, Wine or its own toolkit
registers **file descriptors and timers** through `Linux::IRunLoop`, which the host must obtain from
the view's `IPlugFrame` and service every frame. SoundCraft was claiming that contract in its crate
docs and not implementing it: the view's `queryInterface` for `IRunLoop` failed, so a GUI opened,
painted once and then never handled anything — buttons, knobs and the keyboard were dead while
audio kept working. `crates/vst3-host/src/ffi.rs` now implements it (`RunLoop`: `registerEventHandler`,
`unregisterEventHandler`, `registerTimer`, `unregisterTimer`, plus `pump()` from the editor `idle`
each frame), offers it from the plug frame's `queryInterface`, tells the attached view its size with
`IPlugView::onSize`, and gives the editor window input focus when it maps.

Two more host-side faults, both found by inspecting the live X window tree and by injecting real
input with `scripts/click-editor.py` (XTest):

* **The plugin's own window stayed at the virtual screen size.** A bridged plugin creates its Win32
  window at 1920×1080 and yabridge resizes only the wrapper it puts around it, so the plugin laid
  its GUI out for a full screen while the editor showed a corner of it. The host now resizes the
  plugin's window inside its own to the editor's client size (`fit_plugin_windows`, repeated for the
  first frames after attaching) — the same window id the plugin was given, so Wine adopts the size.
  Measured: Vital's editor window was 1920×1080 inside a 1414×828 editor; it is now 1414×828.
* **Focus.** The editor window is given the keyboard when it maps (`XSetInputFocus`,
  `RevertToParent`), so plugins that ignore clicks until focused behave.

Proof that the pieces reach the plugin: injecting three clicks into ValhallaSupermassive's editor
produces four `IComponentHandler::performEdit` calls (and `beginEdit`/`endEdit`), i.e. the mouse
reaches the plugin's controls and the plugin reports the edit back to the host.

The log now says what a GUI asked for, which is the first thing to look at when an editor is dead:

```
editor run loop: 1 descriptor(s), 0 timer(s) from the plugin's GUI
```

Measured with the reference machine (ValhallaSupermassive, Vital): both register one descriptor and
answer edits afterwards; a plugin that reports `0 descriptor(s), 0 timer(s)` does its own drawing and
does not depend on the host pump.

GUI verification is the one part that cannot be automated end to end here: this machine reports
`Device: llvmpipe` (software rendering) for X11, so a heavy OpenGL editor takes tens of seconds just
to compile its shaders, and the editor checks are therefore run by hand on a GPU-backed session
(`scripts/verify-plugins.py --editor` exists and uses the app's own `Plugin::editor` + `idle` path).

Known Wine-side GUI failures on the reference machine (nothing a host can fix): Kontakt 7 and
Kontakt 8 abort with `virtual_setup_exception stack overflow` while creating or instantiating their
GUI, DecentSampler the same at instantiation, Vaults aborts its Wine host when asked to run.

## Case study: Pianoteq 9 (why it "loaded but did nothing")

Pianoteq 9 appeared in the browser and its window opened, but it reported **0 parameters**, saved
**12 bytes** of state and produced **silence**. Three separate causes, found in this order:

1. **The plugin file was a 2 MB stub.** The Windows drive holds `Common Files\VST3\Pianoteq 9\Pianoteq 9.vst3`
   at 2,110,976 bytes with no instrument or preset strings inside; the real module is
   `Modartt\Pianoteq 9\Pianoteq 9.vst3plugin` at 60,844,544 bytes. A host cannot do anything with the
   stub. The same is true of `Pianoteq 8.vst3` on that drive (2,104,320 bytes) — while the working
   copy on the Elements drive is 50,601,376 bytes. Replacing the stub with the real module (and
   keeping a `.stub-bak` backup) is what makes the plugin exist at all.
2. **Its imported DLLs were not on Wine's search path.** The real module imports `ptq912w.dll` (and
   `ptq912p/v.dll`), which live next to it in the install folder. Wine resolves such imports from
   the *host* directory, not the plugin's, so loading failed with
   `import_dll Library ptq912w.dll … not found`. SoundCraft now adds every bridged plugin's folder
   to `WINEPATH` before any worker thread starts (`crates/vst3-host`, `bridged_plugin_dirs` +
   `add_wine_dll_dirs`), which fixes this class of plugin generally.
3. **Parameter enumeration relied on a deprecated call.** `IEditController::getParameterCount` was
   deprecated in VST3 3.6.14; the host now walks `getParameterInfo(index)` until the plugin refuses
   or repeats an id, and prefers a separate controller class over a component that claims to be its
   own controller.

The permanent setup for a bridged instrument that ships DLLs beside itself is therefore: point
yabridge at the **real** module inside the Wine prefix (or next to its DLLs), and let SoundCraft's
`WINEPATH` setup make the imports resolve.

## Known environment bug: Wine ≥ 9.22 with yabridge 5.1.1

A GUI that opens and draws but **ignores the mouse** is a Wine regression, not a host bug: Wine bug
[59822](https://list.winehq.org/hyperkitty/list/wine-bugs@list.winehq.org/message/FZHHZDIJQTNJEQVJNEX2WOX4E3CLNFWM/)
("Mouse clicks not received in VST plugin UIs (child windows) since Wine 9.22"). yabridge fixed it on
`master` ("Fixed a compatibility issue with Wine 9.22 and above that caused mouse clicks in plugin
GUIs to not register properly") and also fixed a hang that prebuilt yabridge binaries caused on
Ubuntu 24.10+ hosts — neither is in the 5.1.1 release. Remedies: yabridge built from `master`, or the
plugin host running on Wine ≤ 9.21. SoundCraft's own side (run loop, window sizing, focus) is
otherwise exercised as described above.

Related: [Wine bug 57603](https://list.winehq.org/hyperkitty/list/wine-bugs@list.winehq.org/thread/4ZYC65VTHGKAZ45QCBFSGEFTSS7AHWR4/)
(interacting with a yabridge plugin after moving its window).

## Editor stability (2026-10-20)

Four host-side fixes came out of chasing freezes — and one silent close — when an editor was open:

1. **The editor was torn down on its first frame.** "Window not visible" meant "the user closed it",
   but the window manager has not mapped it yet on the first idle frame, so the editor could be
   destroyed immediately after opening — and that path calls `IPlugView::removed()`, which a bridged
   Pianoteq never returns from. Visibility is now only interpreted as "closed" once the window has
   been seen on screen.
2. **Closing an editor no longer blocks the UI thread.** Our window closes at once; the plugin-side
   `removed()` runs on a detached thread, which may leak if the plugin never answers (a leaked
   thread, never a frozen interface).
3. **Releasing a plugin while its editor is still closing crashed the app.** Fix 2 made the close
   asynchronous, but nothing kept the instance from releasing its `IEditController` and
   `IComponent` while that detached thread was still inside `IPlugView::removed()` — a
   use-after-free in the plugin's own code. There is no panic and no core dump: the process simply
   disappears, with "retrying plugin instances" as the last line in the log. This is exactly what a
   plugin scan finishing did to a session whose instrument track had an open editor. The instance now
   waits for a handshake from that thread (1.5 s) before releasing anything, and if the plugin hangs
   it keeps the objects alive instead of freeing memory the plugin is running on. Regression tests:
   `destroying_an_instance_waits_for_the_worker_that_closes_its_editor`,
   `an_editor_that_never_closes_is_leaked_instead_of_released_under_the_worker`;
   `--stress-plugin BUNDLE ID --rounds N` reproduces the sequence by dropping instances with their
   editors still open.
4. **The GUI run loop is not offered to bridged plugins.** A Wine plugin already answers its own
   Win32 event loop at ~60 Hz inside yabridge; letting it register with our run loop as well put two
   threads behind one GUI, and a plugin whose GUI blocks (Omnisphere) then stalled both. Native
   Linux plugins keep the run loop.
4. **The audio thread and the GUI pump can no longer deadlock**: while an instance is inside
   `process`, its editor's run loop is not pumped (the two calls queue on each other in the bridge).

Measured after the fixes: Pianoteq 9, Vital, Pigments and Kontakt 7 editors open, hold and close;
Omnisphere's editor still blocks inside the plugin (its GUI never returns from its own resize
handler), so it should be driven from the alternative console instead.

## Re-running on another machine

```sh
export XDG_RUNTIME_DIR=/tmp/scx   # short and writable: yabridge binds a socket per plugin
python3 scripts/scan-plugins.py
python3 scripts/verify-plugins.py --out tmp/verify
```

A path longer than the Unix socket limit (~108 bytes) makes every bridged plugin fail with
`File name too long` and the catalogue come back with only the native plugins; that is a
environment problem, not a plugin one.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-19 | created | Per-plugin verification harness, full-catalogue run, bridged-Windows findings, Pianoteq 9 case study |
| 2026-10-20 | minor | Plugin GUIs: `Linux::IRunLoop` implemented and pumped, `onSize` after attach, X11 input focus, run-loop logging |
