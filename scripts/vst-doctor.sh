#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# vst-doctor.sh — diagnose why VST3 plugins do (or do not) load in SoundCraft
#                 on Linux, natively and through a yabridge/Wine bridge.
#
# Why this exists: the previous `check_vsts.sh` ran `ldd` on every `*.vst3` it
# found. Almost every Windows plugin on a Linux box is a **PE32+ DLL**, and
# `ldd` refuses those with "not a dynamic executable" — the old script read
# that empty output as "no missing libs" and reported every plugin healthy.
# Windows binaries import DLLs (not SONAMEs), so they need `objdump -p`, and
# their dependencies live in the Wine prefix / beside the plugin, not in
# /usr/lib. This script checks the right thing for each kind of binary.
#
# Usage:
#   scripts/vst-doctor.sh [options] [DIR ...]
#
#   DIR...                 extra folders to scan (default: the standard VST3
#                          locations plus $VST3_PATH)
#   -p, --prefix DIR       Wine prefix to inspect (default: $WINEPREFIX or ~/.wine)
#   -v, --verbose          also list the plugins that pass
#   --no-bridge            skip Wine/yabridge checks
#   --json                 machine-readable summary on stdout
#   -h, --help             this text
#
# Exit status: 0 when nothing needs attention, 1 when at least one plugin has a
# blocking problem, 2 when only warnings were found.
#
# Read-only: it never installs, moves or edits anything. `scripts/vst-fix.sh`
# applies the fixes it suggests.
# ---------------------------------------------------------------------------
set -uo pipefail

VERBOSE=0
JSON=0
BRIDGE=1
PREFIX="${WINEPREFIX:-$HOME/.wine}"
EXTRA_DIRS=()

while (($#)); do
    case "$1" in
        -p | --prefix) PREFIX="${2:?--prefix needs a directory}"; shift 2 ;;
        -v | --verbose) VERBOSE=1; shift ;;
        --json) JSON=1; shift ;;
        --no-bridge) BRIDGE=0; shift ;;
        -h | --help) sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "vst-doctor: unknown option: $1" >&2; exit 2 ;;
        *) EXTRA_DIRS+=("$1"); shift ;;
    esac
done

# ---------------------------------------------------------------- output ----
if [[ -t 1 ]] && [[ -z ${NO_COLOR:-} ]]; then
    C_OK=$'\033[32m' C_WARN=$'\033[33m' C_BAD=$'\033[31m' C_DIM=$'\033[2m' C_OFF=$'\033[0m'
else
    C_OK='' C_WARN='' C_BAD='' C_DIM='' C_OFF=''
fi

PROBLEMS=0   # blocking
WARNINGS=0   # cosmetic / conditional
FINDINGS=()  # "LEVEL|plugin|message|fix" for the JSON summary

# Whether SoundCraft will prefer a native Linux build over a bridged Windows one:
# the environment wins, then the launcher script that most people start it from.
prefer_native() {
    case "${SOUNDCRAFT_PREFER_NATIVE:-}" in
        1 | true | yes | on) return 0 ;;
    esac
    local l
    l=$(command -v soundcraft 2>/dev/null) || return 1
    grep -qE '^[[:space:]]*export SOUNDCRAFT_PREFER_NATIVE=1' "$(readlink -f "$l")" 2>/dev/null
}

section() { printf '\n%s== %s ==%s\n' "$C_DIM" "$1" "$C_OFF"; }
ok() { printf '  %sOK%s   %s\n' "$C_OK" "$C_OFF" "$1"; }
warn() {
    printf '  %sWARN%s %s\n' "$C_WARN" "$C_OFF" "$1"
    WARNINGS=$((WARNINGS + 1))
    FINDINGS+=("warn|${2:-host}|$1|${3:-}")
}
bad() {
    printf '  %sFAIL%s %s\n' "$C_BAD" "$C_OFF" "$1"
    [[ -n ${3:-} ]] && printf '       %s-> %s%s\n' "$C_DIM" "$3" "$C_OFF"
    PROBLEMS=$((PROBLEMS + 1))
    FINDINGS+=("fail|${2:-host}|$1|${3:-}")
}
info() { printf '  %s%s%s\n' "$C_DIM" "$1" "$C_OFF"; }

# ------------------------------------------------------------- discovery ----
wine_bin=$(command -v wine 2>/dev/null || true)
wine_builtin_dir=''
find_wine_builtins() {
    local real root
    [[ -n $wine_bin ]] || return
    real=$(readlink -f "$wine_bin") || return
    for root in "$(dirname "$(dirname "$real")")" /usr /usr/local "$HOME/.local/share/wine"; do
        local d
        for d in "$root/lib/wine/x86_64-windows" "$root/lib64/wine/x86_64-windows" \
                 "$root/lib/x86_64-linux-gnu/wine/x86_64-windows"; do
            if [[ -d $d ]]; then wine_builtin_dir=$d; return; fi
        done
    done
}

# Wine provides these as builtins (PE or .dll.so); a plugin importing any of
# them needs nothing installed. Non-system DLLs (a vendor's own helpers) must
# sit beside the plugin instead, because that is where Wine's loader looks.
is_windows_system_dll() {
    case "${1,,}" in
        api-ms-win-* | ext-ms-*) return 0 ;;
        kernel32.dll | kernelbase.dll | ntdll.dll | user32.dll | gdi32.dll | advapi32.dll | ole32.dll | oleaut32.dll | \
            shell32.dll | shlwapi.dll | comctl32.dll | comdlg32.dll | ws2_32.dll | winmm.dll | wininet.dll | \
            imm32.dll | version.dll | uxtheme.dll | msimg32.dll | gdiplus.dll | oleacc.dll | oledlg.dll | \
            dbghelp.dll | powrprof.dll | dxgi.dll | d3d9.dll | d3d11.dll | d3d12.dll | d2d1.dll | d3dcompiler_* | \
            msvcrt.dll | ucrtbase.dll | vcruntime140.dll | vcruntime140_1.dll | msvcp140.dll | msvcp140_*.dll | \
            concrt140.dll | mfplat.dll | mf.dll | setupapi.dll | winspool.drv | wintrust.dll | crypt32.dll | \
            iphlpapi.dll | dbgeng.dll | xinput*.dll | hid.dll | dnsapi.dll | urlmon.dll | mpr.dll | netapi32.dll | \
            userenv.dll | psapi.dll | rpcrt4.dll | secur32.dll | shcore.dll | dwmapi.dll | winhttp.dll | \
            propsys.dll | uiautomationcore.dll | msacm32.dll | avrt.dll | opengl32.dll | glu32.dll | winusb.dll | \
            dinput8.dll | dsound.dll | msvfw32.dll | bcrypt.dll | ncrypt.dll | wer.dll | winmmbase.dll) return 0 ;;
        *) return 1 ;;
    esac
}

# ELF: the honest check is the dynamic loader itself.
check_native_binary() {
    local so=$1 name=$2
    local out missing
    out=$(ldd "$so" 2>&1)
    missing=$(printf '%s\n' "$out" | awk '/not found/ {print $1}' | sort -u)
    if [[ -n $missing ]]; then
        bad "$name: native binary cannot load — missing shared libraries:" "$name" \
            "$(printf '%s ' $missing)"
        local lib
        while read -r lib; do
            [[ -z $lib ]] && continue
            local pkg
            pkg=$(apt-cache search --names-only "^${lib%%\.so*}" 2>/dev/null | head -n1 | cut -d' ' -f1)
            printf '       %s%s%s\n' "$C_DIM" "$lib${pkg:+  (apt: $pkg)}" "$C_OFF"
        done <<<"$missing"
        return 1
    fi
    # A binary that resolves but has unloadable symbols fails only at dlopen.
    if ! printf '%s\n' "$out" | grep -q 'not a dynamic executable\|statically linked'; then
        [[ $VERBOSE -eq 1 ]] && ok "$name: native, all shared libraries resolve"
    fi
    return 0
}

# Windows PE: read its DLL imports and prove each one is satisfiable from the
# Wine builtins or from a folder next to the plugin (Wine's own search order).
check_windows_binary() {
    local pe=$1 name=$2 host_dir=$3
    local imports dll
    imports=$(objdump -p "$pe" 2>/dev/null | awk '/DLL Name:/ {print $3}' | sort -u)
    if [[ -z $imports ]]; then
        warn "$name: could not read the PE import table (objdump missing?)" "$name"
        return 0
    fi
    local unresolved=()
    while read -r dll; do
        [[ -z $dll ]] && continue
        is_windows_system_dll "$dll" && continue
        local found=0 f
        for f in "$host_dir/$dll" "$(dirname "$host_dir")/$dll" "$host_dir/../.." ; do
            [[ -e $f ]] && { found=1; break; }
        done
        [[ $found -eq 0 ]] && unresolved+=("$dll")
    done <<<"$imports"
    if ((${#unresolved[@]})); then
        bad "$name: Windows plugin needs DLLs that are neither in Wine nor beside it: ${unresolved[*]}" \
            "$name" "copy them next to the plugin, or add their folder to WINEPATH"
        return 1
    fi
    local win_arch=64
    if objdump -f "$pe" 2>/dev/null | grep -q 'pe-i386'; then win_arch=32; fi
    if [[ $win_arch == 32 && ! -d $(dirname "$wine_builtin_dir")/i386-windows ]]; then
        bad "$name: 32-bit Windows plugin and this Wine has no 32-bit support" "$name" \
            "install a WoW64-capable Wine, or use the 64-bit build of the plugin"
        return 1
    fi
    [[ $VERBOSE -eq 1 ]] && ok "$name: Windows plugin, $(printf '%s\n' "$imports" | wc -l) imported DLLs all satisfiable"

    # Loading is not drawing: an editor that paints through Direct2D/DirectComposition
    # (JUCE 8's D2D renderer, VSTGUI 4) opens its window and then never composites the
    # surface — the "opens but is missing pieces" symptom. Almost every modern Windows
    # plugin binary *contains* those symbols whether or not it uses them at runtime, so
    # this is a hint for --verbose, never a warning: on a real library it would fire for
    # two thirds of the bridged plugins, including ones that draw fine.
    [[ $VERBOSE -eq 1 ]] &&
        grep -qa -m1 -iE 'd2d1\.dll|Direct2D|DirectComposition|dcomp\.dll' "$pe" 2>/dev/null &&
        info "$name: references Direct2D/DirectComposition — if its editor draws partially, that is why (docs/vst3-linux-troubleshooting.md)"
    return 0
}

# A bridged Windows plugin is only reachable through a yabridge wrapper bundle
# in ~/.vst3/yabridge; without one no Linux host can load it at all. The index
# is built once — probing every wrapper for every plugin is quadratic.
declare -A WRAPPERS=()
index_wrappers() {
    local w link
    for w in "$HOME/.vst3/yabridge"/*.vst3 "$HOME/.vst3/yabridge"/*/*.vst3 "$HOME/.vst3/yabridge"/*/*/*.vst3; do
        [[ -e $w ]] || continue
        link=$(find "$w/Contents" -maxdepth 2 -path '*-win/*' \( -type l -o -type f \) 2>/dev/null | head -n1)
        [[ -z $link ]] && continue
        WRAPPERS["$(readlink -f "$link")"]=$w
    done
}

wrapper_for() {
    local real
    real=$(readlink -f "$1") || return 1
    [[ -n ${WRAPPERS[$real]:-} ]] && { printf '%s' "${WRAPPERS[$real]}"; return 0; }
    return 1
}

check_bridge_env() {
    section "Wine / yabridge bridge"
    if [[ -z $wine_bin ]]; then
        bad "Wine not found — Windows VST3 plugins cannot run at all" host "install wine-staging"
        return
    fi
    ok "Wine: $("$wine_bin" --version 2>/dev/null | head -n1) ($(readlink -f "$wine_bin"))"
    find_wine_builtins
    if [[ -n $wine_builtin_dir ]]; then
        ok "Wine builtin DLLs: $wine_builtin_dir ($(ls "$wine_builtin_dir" 2>/dev/null | wc -l) DLLs)"
    else
        warn "Could not locate Wine's builtin DLL directory; Windows dependency checks are weaker" host
    fi

    if [[ ! -d $PREFIX ]]; then
        bad "Wine prefix missing: $PREFIX" host "run: WINEPREFIX=$PREFIX wineboot -u"
    else
        ok "Wine prefix: $PREFIX"
        if [[ -d $PREFIX/drive_c/windows/syswow64 ]] || [[ -f $PREFIX/system.reg ]]; then
            ok "Prefix looks initialised"
        else
            bad "Prefix looks incomplete (no system.reg)" host "run: WINEPREFIX=$PREFIX wineboot -u"
        fi
    fi

    local yb=${YABRIDGE_DIR:-$HOME/.local/share/yabridge}
    if [[ -f $yb/libyabridge-vst3.so ]]; then
        ok "yabridge: $yb ($(ls "$yb"/libyabridge-*.so 2>/dev/null | wc -l) libraries)"
        [[ -f $yb/yabridge-host.exe.so ]] || bad "yabridge-host.exe.so missing from $yb" host "reinstall yabridge"
    else
        bad "yabridge not installed at $yb — Windows plugins have no bridge" host \
            "https://github.com/robbert-vdh/yabridge#readme"
    fi
    if [[ -d $HOME/.vst3/yabridge ]]; then
        ok "yabridge VST3 wrappers: $(find "$HOME/.vst3/yabridge" -name '*.vst3' -maxdepth 3 2>/dev/null | wc -l)"
    fi
    command -v yabridgectl >/dev/null && info "yabridgectl $(yabridgectl --version 2>/dev/null | head -n1)"

    # Which Wine actually runs the plugins (a runner shim from scripts/wine-runner.sh
    # changes this), and whether d3d11/dxgi are Wine's builtins or a native swap-in.
    local d3d_type
    d3d_type=$(file -b "$PREFIX/drive_c/windows/system32/d3d11.dll" 2>/dev/null)
    case $d3d_type in
        *WINE*) info "d3d11: Wine's builtin (DXVK not installed)" ;;
        *"MS Windows"*) ok "d3d11: native replacement installed (DXVK or similar)" ;;
        '') info "d3d11: not present in the prefix" ;;
        *) info "d3d11: $d3d_type" ;;
    esac
    if ! compgen -G '/usr/share/vulkan/icd.d/*.json' >/dev/null; then
        warn "no Vulkan ICD: DXVK cannot work and Wine falls back to its own d3d" host \
            "install the Vulkan driver for this GPU (mesa-vulkan-drivers)"
    fi
    local shim="${XDG_DATA_HOME:-$HOME/.local/share}/soundcraft/wine-runner/bin"
    if [[ -n $wine_bin && $wine_bin == "$shim"* ]]; then
        ok "Wine runner: shim active ($(cat "$shim/../current" 2>/dev/null))"
    fi

    # yabridge keeps its socket and Wine's temp files in $XDG_RUNTIME_DIR and
    # fails with "Read-only file system" when that is not writable — the single
    # most common bridge breakage after an upgrade.
    local rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
    if [[ -d $rt && -w $rt ]]; then
        ok "Runtime dir writable: $rt"
    else
        bad "Runtime dir not writable: $rt" host \
            "yabridge fails with 'Read-only file system'; fix the permissions or export XDG_RUNTIME_DIR"
    fi

    local drm=0
    compgen -G '/dev/dri/*' >/dev/null && drm=1
    if [[ $drm -eq 0 ]]; then
        warn "No /dev/dri device: Wine falls back to software OpenGL (llvmpipe); plugin GUIs may draw slowly or partially" host \
            "check that you are in the 'video'/'render' group and that the GPU driver is loaded"
    else
        ok "GPU render nodes: $(ls /dev/dri | tr '\n' ' ')"
    fi
    if [[ -z ${DISPLAY:-} && -z ${WAYLAND_DISPLAY:-} ]]; then
        warn "No DISPLAY/WAYLAND_DISPLAY: plugin editors cannot open a window" host
    else
        ok "Display: DISPLAY=${DISPLAY:-<unset>} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-<unset>}"
    fi
}

# Vendor sound libraries live outside the prefix. Wine resolves them only when
# the path is reachable from a mapped drive or a symlink, so check the ones a
# user cannot guess from an error message.
declare -A VENDOR_DATA=(
    ["Omnisphere (Spectrasonics)"]='Spectrasonics/STEAM'
    ["Trilian/RMX (Spectrasonics)"]='Spectrasonics/STEAM'
)
check_vendor_data() {
    section "Plugin data folders"
    local steam_p
    steam_p="$PREFIX/drive_c/ProgramData/Spectrasonics/STEAM"
    if [[ -e $steam_p ]]; then
        local real
        real=$(readlink -f "$steam_p")
        if [[ -d "$real/Omnisphere" ]]; then
            ok "Spectrasonics STEAM reachable from the prefix: $steam_p -> $real"
            if [[ ! -w "$real" ]]; then
                warn "STEAM is read-only at $real: Omnisphere can play but cannot save patches or user presets" \
                    "spectrasonics" "remount the drive read-write, or move STEAM to a writable disk"
            fi
        else
            bad "STEAM exists at $steam_p but has no Omnisphere/ folder" spectrasonics \
                "point it at the STEAM folder that contains Omnisphere/"
        fi
    else
        # Canonical locations only. A recursive search over every mounted disk
        # takes minutes on a large NTFS library partition, so probe the paths a
        # Spectrasonics install actually uses.
        local cand hits=()
        for cand in \
            /run/media/*/*/ProgramData/Spectrasonics/STEAM \
            /media/*/*/ProgramData/Spectrasonics/STEAM \
            /mnt/*/ProgramData/Spectrasonics/STEAM \
            /mnt/*/*/ProgramData/Spectrasonics/STEAM \
            /run/media/*/*/Users/*/Downloads/Spectrasonics/STEAM \
            "$HOME/STEAM" "$HOME/Spectrasonics/STEAM" /opt/Spectrasonics/STEAM; do
            [[ -d $cand ]] && hits+=("$cand")
        done
        if ((${#hits[@]})); then
            bad "Omnisphere cannot see its STEAM data — the prefix has no ProgramData/Spectrasonics/STEAM, so the plugin asks for the folder on every load" \
                spectrasonics "found: ${hits[0]}  ->  ln -s '${hits[0]}' '$steam_p'"
        else
            warn "No Spectrasonics STEAM data found on this machine (Omnisphere will ask for it)" spectrasonics
        fi
    fi

    local modartt="$PREFIX/drive_c/users/$(id -un)/AppData/Roaming/Modartt"
    if [[ -d $modartt ]] && compgen -G "$modartt/*.prefs" >/dev/null; then
        ok "Pianoteq settings present: $modartt"
    fi
}

# ---------------------------------------------------------------- plugins ---
declare -a NATIVE=() WINDOWS=() OTHER=() BROKEN=()

collect_bundles() {
    local d
    local -a roots=()
    for d in "$HOME/.vst3" /usr/lib/vst3 /usr/local/lib/vst3; do
        [[ -d $d ]] && roots+=("$d")
    done
    if [[ -n ${VST3_PATH:-} ]]; then
        IFS=':' read -r -a p <<<"$VST3_PATH"
        roots+=("${p[@]}")
    fi
    roots+=("${EXTRA_DIRS[@]}")
    find "${roots[@]}" -maxdepth 6 \( -name '*.vst3' -o -name '*.VST3' \) -print0 2>/dev/null |
        sort -z | tr '\0' '\n'
}

# The loadable ELF inside a bundle, or the single-file plugin itself.
linux_binary_of() {
    local b=$1
    if [[ -f $b ]]; then
        file -b "$b" 2>/dev/null | grep -q '^ELF' && printf '%s' "$b"
        return
    fi
    local stem d
    stem=$(basename "$b" .vst3)
    for d in "$b/Contents/x86_64-linux" "$b/Contents/aarch64-linux" "$b/Contents/i386-linux"; do
        [[ -d $d ]] || continue
        [[ -f $d/$stem.so ]] && { printf '%s' "$d/$stem.so"; return; }
        local f
        f=$(find "$d" -maxdepth 1 -name '*.so' | head -n1)
        [[ -n $f ]] && { printf '%s' "$f"; return; }
    done
}

windows_binary_of() {
    local b=$1
    if [[ -f $b ]]; then
        file -b "$b" 2>/dev/null | grep -q 'PE32' && printf '%s' "$b"
        return
    fi
    local stem d
    stem=$(basename "$b" .vst3)
    for d in "$b/Contents/x86_64-win" "$b/Contents/x86-win" "$b/Contents/arm64-win"; do
        [[ -d $d ]] || continue
        [[ -f $d/$stem.vst3 ]] && { printf '%s' "$d/$stem.vst3"; return; }
        local f
        f=$(find "$d" -maxdepth 1 -name '*.vst3' | head -n1)
        [[ -n $f ]] && { printf '%s' "$f"; return; }
    done
}

scan_plugins() {
    section "VST3 plugins"
    local bundle native win name host_dir
    local total=0
    declare -A bridged_names=() native_names=()

    # 1. Bridged Windows plugins: what yabridgectl actually synced. Checking the
    #    payload (not just the wrapper) is the point — the wrapper's ELF loads
    #    fine even when the Windows plugin behind it cannot start.
    if [[ $BRIDGE -eq 1 ]] && ((${#WRAPPERS[@]})); then
        local target
        for target in "${!WRAPPERS[@]}"; do
            local wrapper=${WRAPPERS[$target]}
            name=$(basename "$wrapper" .vst3)
            total=$((total + 1))
            if [[ ! -e $target ]]; then
                bad "$name: the wrapper points at a plugin that no longer exists ($target)" "$name" \
                    "the drive holding it is not mounted; mount it and run: yabridgectl sync"
                BROKEN+=("$wrapper")
                continue
            fi
            if [[ ! -f $target/Contents/x86_64-linux/$(basename "$target" .vst3).so ]] &&
                [[ -f $wrapper/Contents/x86_64-linux/$(basename "$wrapper" .vst3).so ]]; then
                : # wrapper is a chainloader copy; nothing to verify there
            fi
            WINDOWS+=("$wrapper")
            bridged_names[${name,,}]=$wrapper
            host_dir=$(dirname "$target")
            if check_windows_binary "$target" "$name" "$host_dir"; then
                [[ $VERBOSE -eq 1 ]] && ok "$name: bridged → ${host_dir/#$HOME/~}"
            else
                BROKEN+=("$wrapper")
            fi
        done
    fi

    # 2. Native (and native-wrapper) bundles from the search path.
    while IFS= read -r bundle; do
        [[ -z $bundle ]] && continue
        [[ $(basename "$bundle") == '.vst3' ]] && continue   # the search root itself
        name=$(basename "$bundle" .vst3)
        if [[ $bundle == "$HOME/.vst3/yabridge"* ]]; then continue; fi  # already handled above
        total=$((total + 1))
        native=$(linux_binary_of "$bundle")
        win=$(windows_binary_of "$bundle")
        if [[ -n $native ]]; then
            NATIVE+=("$bundle")
            native_names[${name,,}]=$bundle
            check_native_binary "$native" "$name" || BROKEN+=("$bundle")
        elif [[ -n $win ]]; then
            total=$((total - 1))   # counted again below as a Windows plugin
            WINDOWS+=("$bundle")
            if [[ $BRIDGE -eq 1 ]]; then
                bad "$name: Windows-only plugin outside the yabridge folder — no Linux host can load it" "$name" \
                    "yabridgectl add '$(dirname "$bundle")' && yabridgectl sync"
                BROKEN+=("$bundle")
            fi
            check_windows_binary "$win" "$name" "$(dirname "$win")" || BROKEN+=("$bundle")
        else
            OTHER+=("$bundle")
            local kind
            kind=$(file -b "$bundle" 2>/dev/null | cut -c1-48)
            warn "$name: no Linux or Windows binary in this bundle ($kind)" "$name" \
                "macOS-only or incomplete install — reinstall for Linux"
        fi
    done < <(collect_bundles)

    # 3. The same plugin installed both ways collapses to one id in SoundCraft's
    #    catalogue, so silently the bridged (or native) copy is the one that
    #    runs. Say which, because a user debugging "the wrong Pianoteq opens"
    #    needs to know.
    local n which
    if prefer_native; then which=native; else which=bridged; fi
    for n in "${!native_names[@]}"; do
        if [[ -n ${bridged_names[$n]:-} ]]; then
            warn "both a native and a bridged build of '$n' are installed; SoundCraft loads the $which one" "$n" \
                "native: ${native_names[$n]/#$HOME/~}  bridged: ${bridged_names[$n]/#$HOME/~}  (flip with SOUNDCRAFT_PREFER_NATIVE=1)"
        fi
    done

    info "$total plugin(s): ${#NATIVE[@]} native, ${#WINDOWS[@]} Windows/bridged, ${#OTHER[@]} other"
}

# ------------------------------------------------------------------ main ----
if [[ $JSON -eq 0 ]]; then
    printf 'vst-doctor — VST3 plugin diagnosis for SoundCraft\n'
    printf '%shost: %s — wine prefix %s%s\n' "$C_DIM" "$(uname -srm)" "$PREFIX" "$C_OFF"
fi

section "Host"
if command -v soundcraft >/dev/null; then ok "soundcraft: $(command -v soundcraft)"; else
    warn "the soundcraft binary is not in PATH" host "cargo build --release -p soundcraft"
fi

[[ $BRIDGE -eq 1 ]] && { index_wrappers; check_bridge_env; }
check_vendor_data
scan_plugins

section "Summary"
printf '  %d problem(s), %d warning(s)\n' "$PROBLEMS" "$WARNINGS"
if ((${#BROKEN[@]})); then
    printf '  plugins that cannot load:\n'
    printf '    %s\n' "${BROKEN[@]}"
fi

if [[ $JSON -eq 1 ]]; then
    printf '{"problems":%d,"warnings":%d,"native":%d,"windows":%d,"other":%d,"findings":[' \
        "$PROBLEMS" "$WARNINGS" "${#NATIVE[@]}" "${#WINDOWS[@]}" "${#OTHER[@]}"
    first=1
    for f in "${FINDINGS[@]}"; do
        IFS='|' read -r lvl plug msg fix <<<"$f"
        [[ $first -eq 1 ]] || printf ','
        first=0
        printf '{"level":"%s","plugin":%s,"message":%s,"fix":%s}' \
            "$lvl" "$(jq -Rn --arg s "$plug" '$s')" "$(jq -Rn --arg s "$msg" '$s')" "$(jq -Rn --arg s "$fix" '$s')"
    done
    printf ']}\n'
fi

if ((PROBLEMS)); then exit 1; fi
if ((WARNINGS)); then exit 2; fi
exit 0
