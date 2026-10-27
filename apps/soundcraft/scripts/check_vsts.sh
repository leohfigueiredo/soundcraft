#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# check_vsts.sh — the desktop launcher's "Check VST plugins" entry.
#
# This file used to *be* the checker: it ran `ldd` on every `*.vst3`, which on
# Linux means running it on Windows PE binaries — `ldd` refuses those with "not
# a dynamic executable", so every bridged plugin was reported healthy no matter
# what was really missing. The real checker lives in the repo
# (scripts/vst-doctor.sh) and the fixer beside it (scripts/vst-fix.sh); this file
# only runs them, and keeps the window open so the report can be read.
#
# From a terminal, call the tools directly:  scripts/vst-doctor.sh
# ---------------------------------------------------------------------------
set -uo pipefail

# The repo root, from this file's own location (apps/soundcraft/scripts/ → ../../..).
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd) || exit 1
root=$(cd -- "$here/../../.." && pwd) || exit 1
doctor="$root/scripts/vst-doctor.sh"
fixer="$root/scripts/vst-fix.sh"

pause() {
    # A desktop entry runs with Terminal=true and the window vanishes on exit, so a
    # report nobody can read is the same as no report.
    printf '\nPress Enter to close…'
    read -r _ || true
}

if [[ ! -x $doctor ]]; then
    echo "check_vsts: $doctor is missing (expected the SoundCraft checkout at $root)."
    pause
    exit 1
fi

clear 2>/dev/null || true
echo "=== SoundCraft VST3 diagnostics ==="
echo
"$doctor" "$@"
status=$?

echo
case $status in
    0) echo "Nothing to fix." ;;
    1) echo "At least one plugin cannot load — see the FAIL lines above." ;;
    2) echo "Warnings only (it works, but something is worth a look)." ;;
esac
echo
echo "Apply the suggested fixes (dry run first):"
echo "  $fixer"
echo "  $fixer --apply --native-first"
pause
exit "$status"
