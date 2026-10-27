#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# wine-try.sh — flip one variable at a time when a bridged plugin's editor opens
# and then does not paint (black, or "appears, blinks, goes black").
#
# The knobs, and what each one is for:
#
#   --dxvk off|on        d3d11/dxgi through Vulkan (DXVK) or Wine's own wined3d.
#                        DXVK fixes "never repaints"; it does not implement d2d1.
#   --feature-level 9_1|default
#                        dxvk.conf `d3d11.maxFeatureLevel`, the workaround reported for
#                        JUCE 8 / Direct2D editors (needs DXVK on).
#   --virtual-desktop WxH|off
#                        Wine's own desktop. Changes how every Wine window is presented;
#                        it can also break yabridge's embedding, so it is the last resort.
#   --editor-fit on|off
#                        SOUNDCRAFT_NO_EDITOR_FIT: stops the host from resizing the
#                        plugin's window after it opened. An ID2D1HwndRenderTarget paints
#                        through the window's DC, so a resize right after a paint can
#                        re-expose the window black — which looks exactly like the blink.
#   --run [nofit|baseline]
#                        Launch the app right now with that setting for one session,
#                        without touching any stored configuration.
#   --kill-stale [--force]
#                        End instances that still execute a replaced binary (a closed window
#                        does not always end the process), so the next start is the real build.
#   --status / --reset-all
#
# Everything here is reversible and scoped to the user (Wine registry, DXVK dlls in the
# prefix, one dxvk.conf, one environment.d file). Nothing needs root.
# ---------------------------------------------------------------------------
set -uo pipefail

PREFIX="${WINEPREFIX:-$HOME/.wine}"
SYS32="$PREFIX/drive_c/windows/system32"
CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/dxvk"
CONF="$CONF_DIR/dxvk.conf"
ENVD="${XDG_CONFIG_HOME:-$HOME/.config}/environment.d/51-soundcraft-diagnostics.conf"
DLLS=(d3d9 d3d10core d3d11 dxgi)

say() { printf '%s\n' "$1"; }
wine_run() { WINEDEBUG=${WINEDEBUG:-fixme-all} timeout 180 wine "$@" 2>/dev/null; }

d3d_kind() {
    local t
    t=$(file -b "$SYS32/d3d11.dll" 2>/dev/null)
    case $t in
        *WINE*) printf 'wine (wined3d)' ;;
        *"MS Windows"*) printf 'dxvk (native)' ;;
        '') printf 'absent' ;;
        *) printf '%s' "$t" ;;
    esac
}

dxvk_version() {
    # DXVK does not embed "DXVK vX.Y.Z" in d3d11.dll; the version is a bare X.Y.Z string
    # (the same one `dxvk_config` reports for its own d3d11 build).
    local v
    v=$(grep -a -o -m2 -E '[0-9]+\.[0-9]+\.[0-9]+' "$SYS32/d3d11.dll" 2>/dev/null | head -1)
    printf '%s' "v${v:-unknown}"
}

virtual_desktop() {
    local out
    out=$(wine_run reg query 'HKCU\Software\Wine\Explorer' /v Desktop 2>/dev/null | tr -d '\r')
    printf '%s\n' "$out" | grep -oE '[0-9]+x[0-9]+' | head -1
}

status() {
    say "wine-try — ${PREFIX}"
    say "  d3d11/dxgi : $(d3d_kind)  [$(dxvk_version)]"
    say "  dxvk.conf  : $([[ -f $CONF ]] && tr '\n' ' ' <"$CONF" || echo '(none)')"
    local vd; vd=$(virtual_desktop)
    say "  desktop    : ${vd:-off (windows are native X11)}"
    say "  editor fit : $(grep -qs NO_EDITOR_FIT "$ENVD" "$(command -v soundcraft 2>/dev/null || echo /nonexistent)" && echo 'off (SOUNDCRAFT_NO_EDITOR_FIT)' || echo 'on (host sizes the plugin window)')"
    say "  overlays   : $(env | grep -c '^WINEDLLOVERRIDES\|^DXVK_') variable(s) in this shell"
    # The trap that wastes an afternoon: `mv` over a running binary leaves the *old* image
    # executing, so every fix "does nothing" until the app is really restarted.
    local pid exe
    for pid in $(pgrep -f 'soundcraft-bin' 2>/dev/null | head -3); do
        exe=$(readlink "/proc/$pid/exe" 2>/dev/null)
        # Skip wrappers and shells whose command line merely mentions the binary.
        [[ ${exe##*/} == soundcraft-bin* ]] || continue
        if [[ $exe == *"(deleted)"* ]]; then
            # `mv` over a running binary keeps the old image mapped: the process is the previous
            # build even though the path now holds the new one.
            say "  RUNNING    : pid $pid still executes the REPLACED binary"
            say "             : quit SoundCraft completely and start it again — none of the current fixes are live"
        else
            say "  running    : pid $pid (${exe##*/}, current build)"
        fi
    done
    say ""
    say "  test one variable at a time, restarting the app between:"
    say "    $0 --editor-fit off      # blink hypothesis: stop the host resizing the window"
    say "    $0 --dxvk off             # is DXVK involved at all"
    say "    $0 --feature-level 9_1    # JUCE-8/D2D workaround (needs dxvk on)"
}

dxvk_install() {
    if [[ $(d3d_kind) == 'dxvk (native)' ]]; then say "  dxvk already installed [$(dxvk_version)]"; return 0; fi
    local url tmp
    url=$(timeout 60 curl -s https://api.github.com/repos/doitsujin/dxvk/releases/latest |
        grep -oE '"browser_download_url": *"[^"]*dxvk-[0-9.]+\.tar\.gz"' | head -1 | cut -d'"' -f4)
    [[ -n $url ]] || { say "  cannot reach the DXVK release list"; return 1; }
    tmp=$(mktemp -d)
    timeout 600 curl -sL "$url" -o "$tmp/dxvk.tar.gz" || { say "  download failed"; rm -rf "$tmp"; return 1; }
    tar -xzf "$tmp/dxvk.tar.gz" -C "$tmp" || { say "  extract failed"; rm -rf "$tmp"; return 1; }
    cp -f "$tmp"/dxvk-*/x64/*.dll "$SYS32"/ && say "  dlls copied to the prefix"
    local d
    for d in "${DLLS[@]}"; do wine_run reg add 'HKCU\Software\Wine\DllOverrides' /v "$d" /d native,builtin /f >/dev/null; done
    say "  overrides set (native,builtin) — d3d11 is now $(d3d_kind)"
    rm -rf "$tmp"
}

dxvk_revert() {
    local d gone=0
    for d in "${DLLS[@]}"; do
        if [[ -f $SYS32/$d.dll ]] && file -b "$SYS32/$d.dll" | grep -q 'MS Windows'; then
            rm -f "$SYS32/$d.dll" && gone=$((gone + 1))
        fi
        wine_run reg delete 'HKCU\Software\Wine\DllOverrides' /v "$d" /f >/dev/null
    done
    say "  removed $gone dll(s) and their overrides — d3d11 is now $(d3d_kind)"
    say "  run 'wineboot -u' once to let the prefix settle"
}

feature_level() {
    mkdir -p "$CONF_DIR"
    if [[ $1 == default ]]; then
        if [[ -f $CONF ]]; then sed -i '/maxFeatureLevel/d' "$CONF"; say "  maxFeatureLevel removed from $CONF"; fi
    else
        { [[ -f $CONF ]] && grep -v maxFeatureLevel "$CONF"; echo "d3d11.maxFeatureLevel = $1"; } >"$CONF.new" && mv -f "$CONF.new" "$CONF"
        say "  $CONF: d3d11.maxFeatureLevel = $1"
    fi
    # DXVK only reads it where it is told to: the wine host is started by yabridge, so the
    # variable has to be in the environment the app is launched with.
    mkdir -p "$(dirname "$ENVD")"
    if grep -q DXVK_CONFIG_FILE "$ENVD" 2>/dev/null; then
        say "  DXVK_CONFIG_FILE already exported by $ENVD"
    else
        printf 'DXVK_CONFIG_FILE=%s\n' "$CONF" >>"$ENVD"
        say "  added DXVK_CONFIG_FILE to $ENVD (applies at next log-in)"
    fi
    say "  for this session: export DXVK_CONFIG_FILE=$CONF"
}

virtual_desktop_set() {
    if [[ $1 == off ]]; then
        wine_run reg delete 'HKCU\Software\Wine\Explorer' /v Desktop /f >/dev/null
        say "  virtual desktop off — Wine windows are native X11 windows again"
    else
        wine_run reg add 'HKCU\Software\Wine\Explorer' /v Desktop /d "$1" /f >/dev/null
        say "  virtual desktop on at $1 (may break yabridge's embedding: revert with --virtual-desktop off)"
    fi
}

editor_fit() {
    mkdir -p "$(dirname "$ENVD")"
    if [[ $1 == on ]]; then
        sed -i '/SOUNDCRAFT_NO_EDITOR_FIT/d' "$ENVD" 2>/dev/null
        say "  editor fit on (the host sizes the plugin window to the editor)"
    else
        grep -q SOUNDCRAFT_NO_EDITOR_FIT "$ENVD" 2>/dev/null || printf 'SOUNDCRAFT_NO_EDITOR_FIT=1\n' >>"$ENVD"
        say "  editor fit off: the host will not resize the plugin's window"
    fi
    say "  for this session, or without re-logging in: $0 --run $([[ $1 == on ]] && echo baseline || echo nofit)"
}

run_session() {
    local want=${1:-baseline}
    local -a envs=()
    case $want in
        nofit) envs+=("SOUNDCRAFT_NO_EDITOR_FIT=1") ;;
        baseline) ;;
        *) say "  --run takes nofit or baseline"; return 1 ;;
    esac
    say "  launching SoundCraft ($want)…"
    exec env "${envs[@]}" soundcraft "${@:2}"
}

kill_stale() {
    local force=${1:-} pid exe found=0 windows
    local -a targets=()
    for pid in $(pgrep -f 'soundcraft-bin' 2>/dev/null); do
        exe=$(readlink "/proc/$pid/exe" 2>/dev/null) || continue
        [[ ${exe##*/} == soundcraft-bin* ]] || continue
        [[ $exe == *"(deleted)"* || $force == --force ]] && targets+=("$pid")
    done
    if ((${#targets[@]} == 0)); then
        say "  no stale SoundCraft process (nothing executes a replaced binary)"
        return 0
    fi
    # A windowless instance is the usual case; one with a window may be a session with unsaved
    # work, so it is only killed with --force.
    windows=$(xwininfo -root -tree 2>/dev/null | grep -ci 'soundcraft')
    for pid in "${targets[@]}"; do
        if ((windows > 0)) && [[ $force != --force ]]; then
            say "  pid $pid is stale but a SoundCraft window is on screen — save your work and close it,"
            say "  or repeat with: $0 --kill-stale --force"
            continue
        fi
        say "  terminating pid $pid (started $(ps -o lstart= -p "$pid" 2>/dev/null | cut -c1-24))…"
        kill -TERM "$pid" 2>/dev/null && found=1
        for _ in $(seq 1 20); do sleep 0.5; kill -0 "$pid" 2>/dev/null || break; done
        if kill -0 "$pid" 2>/dev/null; then
            say "  it ignored SIGTERM (a stuck thread held a lock) — sending SIGKILL"
            kill -KILL "$pid" 2>/dev/null
        fi
    done
    sleep 0.5
    local left=0 p
    for p in $(pgrep -f 'soundcraft-bin' 2>/dev/null); do
        [[ $(readlink "/proc/$p/exe" 2>/dev/null) == *soundcraft-bin* ]] && left=$((left + 1))
    done
    ((found)) && say "  done: $left soundcraft-bin process(es) left"
}

case "${1:---status}" in
    --status) status ;;
    --kill-stale) shift; kill_stale "${1:-}" ;;
    --dxvk) dxvk_install_or_revert=${2:?--dxvk needs on or off}; [[ $dxvk_install_or_revert == on ]] && dxvk_install || dxvk_revert ;;
    --feature-level) feature_level "${2:?--feature-level needs 9_1 or default}" ;;
    --virtual-desktop) virtual_desktop_set "${2:?--virtual-desktop needs WxH or off}" ;;
    --editor-fit) editor_fit "${2:-on}" ;;
    --run) shift; run_session "$@" ;;
    --reset-all)
        dxvk_revert; feature_level default; virtual_desktop_set off; editor_fit on
        say "  back to the plain setup" ;;
    -h | --help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) say "wine-try: unknown option $1 (try --status or --help)"; exit 2 ;;
esac
