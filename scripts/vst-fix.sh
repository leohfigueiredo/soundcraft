#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# vst-fix.sh — apply the fixes `vst-doctor.sh` reports for VST3 plugins on Linux.
#
# Every fix is idempotent and reversible; nothing is deleted. Without --apply
# it only prints what it would do (the default, so a mistyped command cannot
# touch a working setup).
#
# What it can fix:
#   1. Spectrasonics/Omnisphere: Omnisphere resolves its sample library at
#      %PROGRAMDATA%\Spectrasonics\STEAM and, when the folder is missing, shows
#      "STEAM Folder not found. Would you like to locate it?" on every load.
#      Its own dialog creates an NTFS link with CreateSymbolicLinkW, which fails
#      in a Wine prefix — the link has to be made on the Linux side. This is the
#      fixed path; there is no registry value or config file involved.
#   2. Native-vs-bridged preference: when the same plugin is installed both as a
#      native Linux build and as a Windows build behind yabridge, SoundCraft
#      loads the *bridged* one. For plugins whose GUI a bridge cannot render
#      (Pianoteq 9 draws through Direct2D/DirectComposition, which Wine 11 does
#      not implement), that is exactly the "window opens but is missing pieces"
#      symptom. --native-first makes SoundCraft prefer native builds.
#
# Usage: scripts/vst-fix.sh [--apply] [--native-first] [--prefix DIR]
# ---------------------------------------------------------------------------
set -uo pipefail

APPLY=0
NATIVE_FIRST=0
PREFIX="${WINEPREFIX:-$HOME/.wine}"

while (($#)); do
    case "$1" in
        --apply) APPLY=1; shift ;;
        --native-first) NATIVE_FIRST=1; shift ;;
        --prefix) PREFIX="${2:?--prefix needs a directory}"; shift 2 ;;
        -h | --help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "vst-fix: unknown option: $1" >&2; exit 2 ;;
    esac
done

if [[ -t 1 ]] && [[ -z ${NO_COLOR:-} ]]; then
    C_OK=$'\033[32m' C_DIM=$'\033[2m' C_OFF=$'\033[0m'
else
    C_OK='' C_DIM='' C_OFF=''
fi
say() { printf '  %s\n' "$1"; }
did() { printf '  %sfixed%s %s\n' "$C_OK" "$C_OFF" "$1"; }
plan() { printf '  %swould%s %s\n' "$C_DIM" "$C_OFF" "$1"; }
act() { if ((APPLY)); then did "$1"; else plan "$1"; fi; }

if ((APPLY)); then mode=applying; else mode='dry run — pass --apply to change anything'; fi
echo "vst-fix — ${PREFIX} (${mode})"

# -- 1. Omnisphere STEAM ----------------------------------------------------
fix_steam() {
    local link="$PREFIX/drive_c/ProgramData/Spectrasonics/STEAM"
    local cand
    if [[ -e $link ]]; then
        say "STEAM already reachable at $link -> $(readlink -f "$link")"
    else
        for cand in \
            /run/media/*/*/ProgramData/Spectrasonics/STEAM \
            /media/*/*/ProgramData/Spectrasonics/STEAM \
            /mnt/*/ProgramData/Spectrasonics/STEAM \
            /mnt/*/*/ProgramData/Spectrasonics/STEAM; do
            [[ -d $cand/Omnisphere ]] || continue
            if ((APPLY)); then
                mkdir -p "$(dirname "$link")" && ln -sfn "$cand" "$link" || { say "cannot link $link"; return 1; }
            fi
            act "link $link -> $cand"
            [[ -w $cand ]] || say "note: $cand is not writable — Omnisphere will play but cannot store user patches there"
            return 0
        done
        say "no STEAM folder found (install Omnisphere's library, or set the path manually)"
        return 1
    fi
    if [[ ! -w $(readlink -f "$link") ]]; then
        say "note: STEAM is read-only — Omnisphere plays but cannot save user patches"
    fi
}

# -- 2. native build preference ---------------------------------------------
native_first() {
    # Two places, because either alone misses launches: the launcher script (a
    # terminal start) and systemd's user environment (a desktop entry or app-grid
    # start, which never sees the launcher's exports). Only SoundCraft reads the
    # variable, so setting it session-wide changes nothing else.
    local envd="${XDG_CONFIG_HOME:-$HOME/.config}/environment.d/50-soundcraft.conf"
    if [[ -f $envd ]] && grep -q 'SOUNDCRAFT_PREFER_NATIVE' "$envd"; then
        say "session environment already prefers native builds ($envd)"
    elif ((APPLY)); then
        mkdir -p "$(dirname "$envd")" && printf 'SOUNDCRAFT_PREFER_NATIVE=1\n' >"$envd" &&
            did "write $envd (applies after the next log-in; desktop starts included)"
    else
        plan "write $envd with SOUNDCRAFT_PREFER_NATIVE=1"
    fi

    local launcher
    launcher=$(command -v soundcraft 2>/dev/null) || launcher=''
    if [[ -z $launcher ]]; then
        say "no soundcraft launcher in PATH; export SOUNDCRAFT_PREFER_NATIVE=1 before starting it"
        return 0
    fi
    launcher=$(readlink -f "$launcher")
    if grep -q 'SOUNDCRAFT_PREFER_NATIVE' "$launcher" 2>/dev/null; then
        say "already set in $launcher"
        return 0
    fi
    if ((APPLY)); then
        if [[ -w $launcher ]]; then
            # The launcher `exec`s the app, so anything appended to its end never
            # runs: the export has to go in before the first exec, right after
            # `set -e` (or the shebang when there is none).
            export_line='# Prefer native Linux builds over Windows builds behind yabridge:
# a bridged GUI that Wine cannot render is the usual reason — see
# docs/vst3-linux-troubleshooting.md
export SOUNDCRAFT_PREFER_NATIVE=1'
            if grep -q '^set ' "$launcher"; then
                awk -v line="$export_line" '
                    !done && /^set / { print; print ""; print line; print ""; done=1; next }
                    { print }
                ' "$launcher" >"$launcher.new" && mv -f "$launcher.new" "$launcher"
            else
                awk -v line="$export_line" '
                    NR == 1 { print; print ""; print line; print ""; next }
                    { print }
                ' "$launcher" >"$launcher.new" && mv -f "$launcher.new" "$launcher"
            fi
            chmod +x "$launcher"
        else
            say "cannot write $launcher (owned by $(stat -c %U "$launcher")); add the line by hand:"
            say "  export SOUNDCRAFT_PREFER_NATIVE=1"
            return 1
        fi
    fi
    act "export SOUNDCRAFT_PREFER_NATIVE=1 in $launcher (before its first exec)"
}

# -- 3. advisories ----------------------------------------------------------
advisories() {
    local rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
    if [[ ! -d $rt || ! -w $rt ]]; then
        say "FAIL: $rt is not writable; yabridge fails with 'Read-only file system' — fix its permissions first"
    fi
    if ! compgen -G '/dev/dri/*' >/dev/null; then
        say "note: no /dev/dri device; Wine falls back to software OpenGL and editors may draw slowly"
    fi
    say "if a bridged editor still renders wrong after this, the plugin is using Direct2D/DirectComposition,"
    say "which Wine does not implement; options are the native build, or a Wine patched for D2D1+DComp"
    say "(docs/vst3-linux-troubleshooting.md)"
}

fix_steam
((NATIVE_FIRST)) && native_first
advisories
echo "done$([[ $APPLY -eq 1 ]] || echo ' (nothing changed; pass --apply)')"
