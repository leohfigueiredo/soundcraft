#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# wine-runner.sh — choose which Wine (or Proton/Bottles runner) yabridge spawns.
#
# Why: a bridged plugin's GUI is drawn by whatever Wine runs it, so swapping the
# runner is the real lever when an editor opens but does not paint. Bottles is
# *not* a separate technology from Wine — it is a front-end that manages Wine and
# Proton "runners" (Caffe/Soda/Proton-GE builds) plus DXVK/vkd3d, each runner a
# plain Wine tree. This script uses those runner trees directly: yabridge finds
# `wine` on PATH, so pointing PATH at a runner is enough. (It also works for a
# wine-tkg / patched-Wine build you compiled yourself, which is currently the only
# way to get complete Direct2D/DirectComposition — the API JUCE 8 and VSTGUI 4
# editors paint through.)
#
# Usage:
#   scripts/wine-runner.sh --list
#   scripts/wine-runner.sh --use bottles:soda      # or a name from --list
#   scripts/wine-runner.sh --use /path/to/runner   # a Wine tree or wine prefix runner
#   scripts/wine-runner.sh --current
#   scripts/wine-runner.sh --reset                 # back to the system wine
#   scripts/wine-runner.sh --use NAME --in-launcher # also export it in the launcher
#
# The shim only redirects `wine`, `wine64` and `wineserver`; nothing is installed
# into the runner and no prefix is modified.
# ---------------------------------------------------------------------------
set -uo pipefail

SHIM="${XDG_DATA_HOME:-$HOME/.local/share}/soundcraft/wine-runner/bin"
STATE="${XDG_DATA_HOME:-$HOME/.local/share}/soundcraft/wine-runner/current"

runner_root() { echo "${XDG_DATA_HOME:-$HOME/.local/share}/soundcraft/wine-runner"; }

# Every Wine tree this machine can offer, as "name<TAB>path-to-its-bin-dir".
list_runners() {
    local d b
    # Bottles (Flatpak and native installs). A runner is a Wine tree in runners/.
    for d in "$HOME/.var/app/com.usebottles.bottles/data/bottles/runners" \
             "$HOME/.local/share/bottles/runners" \
             "$HOME/.bottles/runners"; do
        [[ -d $d ]] || continue
        for b in "$d"/*; do
            [[ -d $b/bin ]] && printf 'bottles:%s\t%s\n' "$(basename "$b")" "$b/bin"
        done
    done
    # Steam Proton (Proton is Wine + DXVK + vkd3d; usable as a runner).
    for d in "$HOME/.steam/steam/steamapps/common" "$HOME/.local/share/Steam/steamapps/common" \
             "$HOME/snap/steam/common/.local/share/Steam/steamapps/common"; do
        [[ -d $d ]] || continue
        for b in "$d"/Proton*; do
            [[ -x $b/dist/bin/wine ]] && printf 'proton:%s\t%s\n' "$(basename "$b")" "$b/dist/bin"
        done
    done
    # Lutris runners and hand-built wine-tkg.
    for d in "$HOME/.local/share/lutris/runners/wine" "$HOME/.local/share/wine-tkg"* \
             "$HOME/.local/share/wine"*; do
        [[ -d $d ]] || continue
        if [[ -x $d/bin/wine ]]; then
            printf 'wine:%s\t%s\n' "$(basename "$d")" "$d/bin"
        else
            for b in "$d"/*; do [[ -x $b/bin/wine ]] && printf 'wine:%s\t%s\n' "$(basename "$b")" "$b/bin"; done
        fi
    done
    local sys
    sys=$(command -v wine 2>/dev/null) && printf 'system:%s\t%s\n' "$(wine --version 2>/dev/null | head -1)" "$(dirname "$(readlink -f "$sys")")"
}

current() {
    if [[ -f $STATE ]]; then
        printf 'shim:    %s\nrunner:  %s\n' "$SHIM" "$(cat "$STATE")"
        [[ ":$PATH:" == *":$SHIM:"* ]] && echo "active:  yes (PATH has the shim)" || echo "active:  not in PATH — export PATH=\"$SHIM:\$PATH\""
    else
        echo "no runner selected (system wine: $(wine --version 2>/dev/null || echo absent))"
    fi
}

use_runner() {
    local want=$1 bindir=''
    if [[ -d $want && -x $want/wine ]]; then
        bindir=$want
    else
        bindir=$(list_runners | awk -F'\t' -v n="$want" '$1==n {print $2; exit}')
    fi
    [[ -n $bindir && -x $bindir/wine ]] || { echo "wine-runner: no runner named '$want' (see --list)" >&2; return 1; }
    mkdir -p "$SHIM" && printf '%s\n' "$bindir" >"$STATE"
    local t
    for t in wine wine64 wineserver wineboot winecfg winetricks; do
        [[ -x $bindir/$t ]] || continue
        printf '#!/bin/sh\nexec "%s/%s" "$@"\n' "$bindir" "$t" >"$SHIM/$t"
        chmod +x "$SHIM/$t"
    done
    echo "wine-runner: using $bindir"
    echo "wine-runner: shim written to $SHIM"
    echo "Start SoundCraft with:  PATH=\"$SHIM:\$PATH\" soundcraft"
    echo "then re-sync the bridge:  PATH=\"$SHIM:\$PATH\" yabridgectl sync"
    echo "note: a different runner needs its own prefix (WINEPREFIX=...) and a fresh 'wineboot -u'."
}

in_launcher() {
    local launcher
    launcher=$(readlink -f "$(command -v soundcraft 2>/dev/null)") || { echo "wine-runner: no soundcraft in PATH" >&2; return 1; }
    grep -q "$SHIM" "$launcher" 2>/dev/null && { echo "wine-runner: launcher already points at the shim"; return 0; }
    awk -v line="export PATH=\"$SHIM:\$PATH\"" '
        !done && /^set / { print; print ""; print "# Wine runner selected by scripts/wine-runner.sh"; print line; print ""; done=1; next }
        { print }
    ' "$launcher" >"$launcher.new" && mv -f "$launcher.new" "$launcher" && chmod +x "$launcher" &&
        echo "wine-runner: launcher now prepends $SHIM"
}

case "${1:---list}" in
    --list) list_runners | sort | column -t -s$'\t' ;;
    --current) current ;;
    --use)
        shift
        target=${1:?--use needs a runner name or path}
        shift || true
        use_runner "$target" || exit 1
        [[ ${1:-} == --in-launcher ]] && in_launcher
        ;;
    --reset) rm -rf "$(runner_root)" && echo "wine-runner: shim removed; system wine is back" ;;
    -h | --help) sed -n '2,35p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) echo "wine-runner: unknown option $1" >&2; exit 2 ;;
esac
