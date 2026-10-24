#!/usr/bin/env python3
"""Scan every plugin, skipping the ones that crash the scanner.

`soundcraft --scan-plugins` prints one JSON object per line and dies with the plugin it was
inspecting when that plugin aborts the process. This wraps it the way the editor does: rerun with
`--skip <bundle>` for each culprit, keeping what every pass found, until a pass completes.

Usage: scripts/scan-plugins.py [--out FILE] [--skip-file FILE] [--dir DIR]…
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "release" / "soundcraft"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(ROOT / "tmp" / "catalogue.ndjson"))
    parser.add_argument("--skip-file", default=str(ROOT / "tmp" / "scan-skipped.txt"))
    parser.add_argument("--dir", action="append", default=[], help="extra folder to scan")
    args = parser.parse_args()

    env = dict(os.environ)
    # A bridged plugin binds its socket here: keep the path short ("File name too long").
    # A bridged plugin binds its socket in this folder: keep the path short, and writable.
    env["XDG_RUNTIME_DIR"] = "/tmp/scx"

    skipped = []
    skip_file = Path(args.skip_file)
    if skip_file.exists():
        skipped = [line.strip() for line in skip_file.read_text(encoding="utf-8").splitlines() if line.strip()]

    plugins = {}
    for attempt in range(64):
        cmd = [str(BINARY), "--scan-plugins"]
        for d in args.dir:
            cmd += ["--dir", d]
        for s in skipped:
            cmd += ["--skip", s]
        done = subprocess.run(cmd, capture_output=True, text=True, env=env)
        last_scanning = None
        for line in done.stdout.splitlines():
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "scanning" in entry:
                last_scanning = entry["scanning"]
            else:
                for kind in ("vst3", "clap"):
                    if kind in entry:
                        plugins[entry[kind]["id"]] = entry[kind]
        print(f"pass {attempt + 1}: exit {done.returncode}, {len(plugins)} plugins known", file=sys.stderr)
        if done.returncode == 0:
            break
        if not last_scanning:
            print("the scanner stopped without naming a plugin; giving up", file=sys.stderr)
            break
        if last_scanning in skipped:
            print(f"the scanner keeps crashing on {last_scanning}", file=sys.stderr)
            break
        print(f"the scanner crashed while loading {last_scanning}; skipping it", file=sys.stderr)
        skipped.append(last_scanning)

    skip_file.write_text("\n".join(skipped) + ("\n" if skipped else ""), encoding="utf-8")
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w", encoding="utf-8") as fh:
        for plugin in sorted(plugins.values(), key=lambda p: (p.get("name", "").lower(), p.get("path", ""))):
            fh.write(json.dumps({"vst3" if plugin["id"].startswith("vst3:") else "clap": plugin}) + "\n")
    print(f"{len(plugins)} plugins, {len(skipped)} skipped -> {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
