#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# kontakt-libraries.sh — make every Kontakt-format library on this machine visible to
# Kontakt 7/8 (the Windows builds that run here through yabridge).
#
# How it works: Kontakt keeps its content roots in a SQLite table,
#
#   AppData\Local\Native Instruments\Kontakt <version>\user_config.db3
#     k_content_path(path TEXT, content_type INTEGER, alias TEXT)
#
# with one row for "Native Instruments User Content" by default. Adding a row per
# library folder (Windows path, content_type 1) is what makes Kontakt show it under
# its content/browser pane — no Native Access account involved. The databases are
# backed up before anything is written.
#
# Usage:
#   scripts/kontakt-libraries.sh            # show the libraries found (dry run)
#   scripts/kontakt-libraries.sh --apply    # register them in Kontakt 7 and 8
#   scripts/kontakt-libraries.sh --show     # what Kontakt already knows
# ---------------------------------------------------------------------------
set -uo pipefail

PREFIX=${WINEPREFIX:-$HOME/.wine}
APPLY=0
MODE=list
for arg in "$@"; do
    case $arg in
        --apply) APPLY=1 ;;
        --show) MODE=show ;;
        -h | --help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "kontakt-libraries: unknown option $arg" >&2; exit 2 ;;
    esac
done

python3 - "$PREFIX" "$APPLY" "$MODE" <<'PY'
import glob, os, re, shutil, sqlite3, subprocess, sys, time

prefix, apply, mode = sys.argv[1], sys.argv[2] == "1", sys.argv[3]

# ---- 1. Linux → Windows path, through the prefix's own drive mappings -------------
def to_windows(path):
    path = os.path.abspath(path)
    for link in glob.glob(os.path.join(prefix, "dosdevices", "*:")):
        letter = os.path.basename(link)[0].upper()
        target = os.path.realpath(link)
        if path == target or path.startswith(target + os.sep):
            rest = path[len(target):].replace("/", "\\")
            return f"{letter}:{rest}" if rest else f"{letter}:\\"
    return None

# Library scan roots: the places content actually lives, never a whole drive (a recursive
# glob over an NTFS media mount takes minutes; `find` with a depth limit takes a second).
scan_roots = []
for base in (
    "/run/media/*/*/Plugins", "/run/media/*/*/Piano/Plugins", "/run/media/*/*/*/Plugins",
    "/media/*/*/Plugins", "/mnt/*/Plugins",
    os.path.expanduser("~/Documents/Native Instruments"),
    os.path.join(prefix, "drive_c/ProgramData/Native Instruments"),
):
    scan_roots.extend(glob.glob(base))

hits = []
for root in scan_roots:
    if not os.path.isdir(root):
        continue
    for ext in ("nki", "nkm"):
        try:
            out = subprocess.run(
                ["find", root, "-maxdepth", "7", "-iname", f"*.{ext}", "-printf", "%p\n"],
                capture_output=True, text=True, timeout=120,
            ).stdout
        except subprocess.TimeoutExpired:
            continue
        hits.extend(line for line in out.splitlines() if line)

# A library folder is the pack directory: fold Instruments/Samples/Snapshots up one level.
libraries = set()
for hit in hits:
    parent = os.path.dirname(hit)
    if os.path.basename(parent).lower() in ("instruments", "samples", "snapshots", "multis", "programs"):
        parent = os.path.dirname(parent)
    # Skip macOS resource forks and Kontakt's own program folders: they are not libraries.
    lowered = parent.lower()
    if "__macosx" in lowered or "/kontakt portable/" in lowered.replace(os.sep, "/"):
        continue
    if os.path.basename(parent).lower() in ("default", "assets", "template"):
        continue
    if to_windows(parent):
        libraries.add(parent)

libraries = sorted(x for x in libraries if to_windows(x))
if not libraries:
    print("no Kontakt library folder found (no .nki/.nkm under the mounted drives)")
    sys.exit(0)

# ---- 3. What Kontakt already knows ----------------------------------------------
def configs():
    pat = os.path.join(prefix, "drive_c/users/*/AppData/Local/Native Instruments/Kontakt */user_config.db3")
    return sorted(glob.glob(pat))

def known(db):
    tmp = f"/tmp/kl-{abs(hash(db))}.db3"
    shutil.copy2(db, tmp)
    try:
        con = sqlite3.connect(tmp)
        rows = con.execute("select path, content_type, alias from k_content_path").fetchall()
        con.close()
        return rows
    except Exception:
        return []
    finally:
        os.unlink(tmp)

if mode == "show":
    for db in configs():
        print(f"--- {db}")
        for path, kind, alias in known(db):
            print(f"    [{kind}] {alias}  ->  {path}")
    sys.exit(0)

print(f"Kontakt libraries found: {len(libraries)}")
for lib in libraries:
    alias = os.path.basename(lib.rstrip("/"))
    print(f"  {alias:42} {to_windows(lib)}")

if not apply:
    print("\n(dry run — nothing written; pass --apply to register them in Kontakt 7 and 8)")
    sys.exit(0)

written = 0
for db in configs():
    have = {p for p, _, _ in known(db)}
    todo = [(to_windows(lib), 1, os.path.basename(lib.rstrip("/"))) for lib in libraries if to_windows(lib) not in have]
    if not todo:
        print(f"[skip] {db} already has all of them")
        continue
    backup = f"{db}.bak-{time.strftime('%Y%m%d-%H%M%S')}"
    shutil.copy2(db, backup)
    con = sqlite3.connect(db)
    con.executemany("insert into k_content_path (path, content_type, alias) values (?, ?, ?)", todo)
    con.commit()
    con.close()
    written += len(todo)
    print(f"[ok  ] {os.path.basename(os.path.dirname(db))}: +{len(todo)} paths (backup {os.path.basename(backup)})")

print(f"\n{written} entries written. Open Kontakt 7/8 — the folders appear in its content pane.")
print("If Kontakt is open, restart it: it reads these at start-up.")
PY
