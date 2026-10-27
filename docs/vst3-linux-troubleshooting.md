# VST3 plugins on Linux: diagnosing and fixing them

How to find out why a VST3 plugin does not load or draws a broken window in
SoundCraft on Linux, and what the actual causes are on a machine that bridges
Windows plugins through yabridge + Wine. Last revised: 2026-10-24.

## Tools

```bash
scripts/vst-doctor.sh          # read-only diagnosis, exit 1 on a blocking problem
scripts/vst-doctor.sh --json   # same, machine-readable
scripts/vst-fix.sh             # show what it would fix (dry run)
scripts/vst-fix.sh --apply --native-first   # apply
```

Both are dependency-free bash (`objdump`, `apt-cache`, `jq` optional-to-required
depending on the mode). `vst-doctor.sh` never changes anything.

The app itself validates one plugin at a time in a child process — the same path
the editor uses before it loads a plugin in-process:

```bash
soundcraft --verify-plugin "<bundle path>" "vst3:<class id hex>" --editor --editor-hold 4000
# → one JSON line: params, programs, latency, state round trip, editor ok/error, ms
soundcraft --scan-plugins            # the catalogue, one JSON line per plugin
```

## Why the old `check_vsts.sh` was wrong

It ran `ldd` on every `*.vst3` it found. On Linux a Windows plugin is a **PE32+
DLL**, and `ldd` refuses those with `not a dynamic executable` (exit 1). The
script only kept lines containing `not found`, so every Windows plugin printed
"no missing libs" no matter what was missing. It also looked for the Linux
SONAME→package mapping, which Windows binaries do not have, and it treated a
`.vst3` *bundle directory* as a binary.

What is actually true:

| Binary | Correct check |
|---|---|
| ELF (`Contents/x86_64-linux/*.so`) | `ldd` — a Linux plugin really does import SONAMEs |
| PE (`Contents/x86_64-win/*.vst3`, or a legacy single-file `.vst3`) | `objdump -p` → `DLL Name:` list; each import must be a Wine builtin (`<wine>/lib/wine/*-windows`) **or** sit beside the plugin, which is where Wine's loader looks |

A Windows plugin additionally needs a **yabridge wrapper** (`~/.vst3/yabridge/…`)
to be loadable at all: the wrapper is a native `.so` that starts
`yabridge-host.exe` under Wine. `yabridgectl sync` creates them; the wrapper's
`Contents/x86_64-win/<name>.vst3` is a symlink to the real plugin, so a dangling
symlink means the drive holding the plugin is not mounted.

## Causes found on this machine (2026-10-24)

Environment: Ubuntu 26.10, Wine 11.19 staging at `~/.local/share/wine`,
yabridge 5.1.1, 26 bridged Windows plugins + 19 native, prefix `~/.wine`.

### Everything was *not* fine, but dependencies were

26 bridged plugins, every imported DLL satisfiable, all bundles present. The
breakages were per-plugin data/GUI issues, not missing Windows libraries.

### Omnisphere — "STEAM Folder not found. Would you like to locate it?"

Omnisphere does not use a registry value or a config file for its library. It
resolves `%PROGRAMDATA%\Spectrasonics\STEAM`, validates it, and only then has a
dialog that *creates a link itself* (`CreateSymbolicLinkW`) — which fails inside
a Wine prefix, so the prompt returns on every load. Confirmed by the vendor
manual (<https://support.spectrasonics.net/manual/Omnisphere3/30/en/topic/get-started-page04>)
and by the binary's strings/imports.

The prefix had an empty `drive_c/ProgramData/Spectrasonics`, while Arturia,
Modartt, Native Instruments and Valhalla were all symlinks to the Windows
partition. Fix (`vst-fix.sh` does this): link it on the Linux side —

```bash
ln -sfn /run/media/<disk>/Windows/ProgramData/Spectrasonics/STEAM \
        ~/.wine/drive_c/ProgramData/Spectrasonics/STEAM
```

Verified: `wine cmd /c 'dir /b C:\ProgramData\Spectrasonics\STEAM'` lists
`Omnisphere`, and `--verify-plugin` opens the real 1210×757 Omnisphere window
with no STEAM dialog, 3097 parameters, state round trip OK.

### Pianoteq 9 — window opens but is missing pieces

The Windows payload is a **JUCE 8 build using JUCE's Direct2D renderer**: it
imports `d3d11.dll` and `dwmapi.dll`, and its symbol table carries
`juce::RectangleListSpriteBatch` over `ComSmartPtr<ID2D1DeviceContext1>`,
`Direct2DMetrics` and DirectWrite. Wine implements Direct2D/DirectComposition
incompletely, so the editor window maps and then never composites its surface.
(A 2.1 MB `Pianoteq 9.vst3.stub-bak` beside it is the previous VSTGUI 4 build —
same conclusion: no GDI+ fallback is compiled in either.) There is no renderer or
OpenGL switch in Pianoteq's preferences, so this is not configurable plugin-side;
out-of-tree Wine patches exist for exactly this class (`d2d1`+`dcomp`), and
yabridge's own README recommends DXVK for JUCE plugins that stop redrawing under
WineD3D.

Observed directly: running the Windows standalone under Wine, no main window ever
mapped (only JUCE helper windows), while the plugin's VST3 entry point and editor
both "succeed" when SoundCraft drives them.

Pianoteq also **ignores host program changes**, which matters for the instrument
console: it publishes a program list and a `kIsProgramChange` parameter, accepts a
value, accepts a MIDI program change, and renders bit-identical audio for programs
0, 12 and 30 (this SDK has no `IUnitInfo::selectProgram`, so those are the only
routes a host has). A console whose plugin behaves that way is marked
`capture_first`: clicking an instrument loads the state **saved for it** (right-click
on the instrument saves the plugin's current sound onto it), and until one is saved
the status line says so instead of reporting a switch that did not happen.

**A native Linux Pianoteq 9 was already installed** at
`~/.vst3/Pianoteq 9.vst3` (9.2.5), but SoundCraft loaded the *bridged* Windows
9.1.2 with the same VST3 class id: `registry::candidate_wins` prefers the
bridged build by default (the licensed Windows install usually beats a native
trial). Neither `vst-doctor.sh` nor a user can see that collision without being
told about it.

Fix: prefer native builds when both exist — `SOUNDCRAFT_PREFER_NATIVE=1` in the
launcher (`vst-fix.sh --apply --native-first` inserts it before the launcher's
first `exec`, since the launcher `exec`s the app). Bridged-only plugins
(Omnisphere, Kontakt, Roland…) are unaffected.

Measured with `--verify-plugin --editor`: native editor opens in ~5 s, bridged
takes ~27 s under Wine. The bridged window still *reports* success — the failure
is in painting, which no host-side check can see. That is why the native build is
the right answer rather than a host workaround, and why `vst-doctor.sh` prints the
Direct2D hint only under `--verbose`: almost every modern Windows binary contains
those symbols whether or not it uses them, so as a warning it fired for 18 of the
26 bridged plugins here, including ones that draw fine.

### The host deliberately refuses yabridge's run loop

`vst3-host` calls `set_run_loop_enabled(false)` for bridged plugins, and yabridge
logs `The host does not support IRunLoop, falling back to naive GUI function
execution`. That is yabridge's supported fallback and it is intentional: a
bridged plugin already runs its own Win32 event loop in `yabridge-host.exe`, and
two event pumps behind one GUI froze Omnisphere's editor.

## What a bridged GUI cannot do

If a Windows plugin's editor is written against Direct2D/DirectComposition (or
JUCE 8's D2D renderer), stock Wine will not paint it correctly no matter what the
host does. Options, in order:

1. Install the native Linux build if the vendor ships one (Pianoteq, Vital, LABS,
   DecentSampler…), and let `SOUNDCRAFT_PREFER_NATIVE=1` pick it.
2. Use a Wine patched for D2D1/DComp, or a Proton/Bottles runner — see
   [Choosing the Wine runner](#choosing-the-wine-runner).
3. Try DXVK with a clamped feature level (`d3d11.maxFeatureLevel = 9_1`) — DXVK
   replaces `d3d11`/`dxgi` with Vulkan and fixes the "window never repaints" class
   of bug, but it does not implement `d2d1` itself.

## Measuring a black editor (and the limits of the measurement)

`xwd -id <window>` + `ffmpeg` is how the windows here were inspected: it reads the
window's pixels from the X server and works for X11-rendered editors (a bridged
Arturia editor captured at 13 506 distinct colours). It is **blind to
GPU-presented content**: a window whose contents arrive through a GL/Vulkan
swapchain (native VSTGUI/OpenGL editors, D2D-through-D3D editors) reads back as a
single black colour even when the screen shows it fine. GNOME's screenshot D-Bus
API (`org.gnome.Shell.Screenshot`) is usually disabled (`AccessDenied`), so there
is no unattended way to grab the composited screen. Practical rule: treat `xwd`
black as "cannot see", not as "broken" — confirm against the screen.

Observed on this machine with that method, all four through the real host:

| plugin | window | capture |
|---|---|---|
| Pigments (bridged) | 1280×802 | draws (13 506 colours) |
| Omnisphere (bridged) | 1160×670, yabridge window correctly reparented inside it | black |
| Pianoteq 9 (native) | 911×890 | black |
| Pianoteq 9 (bridged) | opens, ~27 s | — |

Omnisphere's window tree proves the embedding handshake is fine — the
`yabridge plugin` window is a child of our container at exactly its size — so a
black result there is a painting problem inside Wine, not a host layout problem.
The container window is a normal top-level X11 window (WM_CLASS
`soundcraft-plugin`/`SoundCraft`) and the Wine child is inside it.

## Choosing the Wine runner

yabridge runs plugins with the `wine` it finds on `PATH`, so a different runner is
a `PATH` change, not a code change:

```bash
scripts/wine-runner.sh --list                 # bottles / proton / lutris / wine-tkg / system
scripts/wine-runner.sh --use bottles:soda     # write a wine/wine64/wineserver shim
PATH="$HOME/.local/share/soundcraft/wine-runner/bin:$PATH" soundcraft
```

**Bottles is not an alternative to Wine.** It is a front-end that manages Wine and
Proton *runners* (Caffe/Soda/Proton-GE — plain Wine trees) plus DXVK/vkd3d and
per-bottle configuration in a sandbox. What it can give a plugin host is the
*runner*: a different Wine build, possibly with patches stock Wine lacks. There is
no mechanism by which a Bottles bottle can host a VST3 inside another process —
a bottle runs `.exe` files, and a VST3 has no `.exe` entry point, which is exactly
why yabridge exists. So the useful move is to point the bridge at a runner, which
`wine-runner.sh` does.

A new runner needs its own prefix: `WINEPREFIX=~/.wine-<runner> wineboot -u`, then
`yabridgectl sync` again, because the wrappers hard-code nothing but the plugin
paths and the host binary is looked up at load time.

DXVK: installed into `~/.wine/drive_c/windows/system32/{d3d9,d3d10core,d3d11,dxgi}.dll`
(v3.1.1) with `HKCU\Software\Wine\DllOverrides` set to `native,builtin` for those
four. Revert with:

```bash
W=~/.wine/drive_c/windows/system32
rm -f "$W"/{d3d9,d3d10core,d3d11,dxgi}.dll
for d in d3d9 d3d10core d3d11 dxgi; do wine reg delete 'HKCU\Software\Wine\DllOverrides' /v $d /f; done
wineboot -u
```


## Quick checks worth remembering

- `yabridgectl status` — which plugins are synced, and whether their sources exist.
- `XDG_RUNTIME_DIR` must be writable, or yabridge fails with
  `Read-only file system: '/run/user/…/yabridge-…'` and every bridged plugin
  silently disappears from the catalogue (`--scan-plugins` shows
  `{"scanning":…}` lines with no plugin line after them).
- `/dev/dri` missing means software OpenGL: everything still works, slowly.
- `YABRIDGE_DEBUG_LEVEL=1+editor` (plus `WINEDEBUG=+d2d,+dxgi,+dcomp`) tells you
  whether a window is failing to composite.
