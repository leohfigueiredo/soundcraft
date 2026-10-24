#!/usr/bin/env python3
"""Verify every installed plugin through the calls SoundCraft's audio side and UI make.

For each plugin in the scanned catalogue it runs `soundcraft --verify-plugin <bundle> <id>` in a
child process (so a plugin that segfaults or hangs costs one plugin, not the run) and records what
happened: load, instantiate, prepare, process, state round-trip and, with `--editor`, the GUI.

Usage:
    scripts/verify-plugins.py [--catalogue FILE] [--out DIR] [--editor] [--only SUBSTRING]
                              [--timeout SECONDS] [--jobs N]

The catalogue comes from `soundcraft --scan-plugins` (one JSON object per line, as the app's
scanner child prints it). Progress goes to stderr; results are written as JSON lines plus a
markdown summary, both under --out.
"""

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "release" / "soundcraft"


def read_catalogue(path):
    plugins = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            for kind in ("vst3", "clap"):
                if kind in entry:
                    plugins.append(entry[kind])
    return plugins


def verify(plugin, timeout, editor, env):
    cmd = [str(BINARY), "--verify-plugin", str(plugin["path"]), plugin["id"]]
    if editor:
        cmd.append("--editor")
    started = time.time()
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired as expired:
        stage = "unknown"
        for line in str(expired.stderr or "").splitlines():
            if line.startswith("stage: "):
                stage = line[len("stage: ") :].strip()
        return {
            "id": plugin["id"],
            "name": plugin.get("name", ""),
            "path": str(plugin["path"]),
            "ok": False,
            "stage": f"timeout ({stage})",
            "error": f"no answer in {timeout}s, stuck in `{stage}`",
            "ms": int((time.time() - started) * 1000),
        }
    result = None
    for line in done.stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                result = json.loads(line)
            except json.JSONDecodeError:
                continue
    if result is None:
        stage = "unknown"
        for line in done.stderr.splitlines():
            if line.startswith("stage: "):
                stage = line[len("stage: ") :].strip()
        tail = " / ".join(done.stderr.strip().splitlines()[-3:])
        result = {
            "id": plugin["id"],
            "name": plugin.get("name", ""),
            "path": str(plugin["path"]),
            "ok": False,
            "stage": stage,
            "error": f"the child died (exit {done.returncode}): {tail[:300]}",
            "ms": int((time.time() - started) * 1000),
        }
    result.setdefault("name", plugin.get("name", ""))
    result.setdefault("path", str(plugin["path"]))
    result["exit"] = done.returncode
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--catalogue", default=str(ROOT / "tmp" / "catalogue.ndjson"))
    parser.add_argument("--out", default=str(ROOT / "tmp" / "verify"))
    parser.add_argument("--editor", action="store_true", help="also open and close each plugin's GUI")
    parser.add_argument("--only", default="", help="only plugins whose name or path contain this")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--jobs", type=int, default=1, help="parallel children (Wine plugins dislike many at once)")
    args = parser.parse_args()

    plugins = read_catalogue(args.catalogue)
    if args.only:
        needle = args.only.lower()
        plugins = [p for p in plugins if needle in p.get("name", "").lower() or needle in str(p["path"]).lower()]
    if not plugins:
        sys.exit(f"no plugins to verify (catalogue: {args.catalogue})")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    suffix = "editor" if args.editor else "core"
    results_path = out / f"results-{suffix}.ndjson"
    summary_path = out / f"summary-{suffix}.md"

    env = dict(os.environ)
    # A bridged plugin binds its socket in this folder: keep the path short, and writable, or the
    # Wine host cannot bind it ("File name too long") and the scan finds nothing.
    env["XDG_RUNTIME_DIR"] = "/tmp/scx"

    lock = threading.Lock()
    results = []
    done_count = 0
    started = time.time()
    work = queue.Queue()
    for p in plugins:
        work.put(p)

    def worker():
        nonlocal done_count
        while True:
            try:
                plugin = work.get_nowait()
            except queue.Empty:
                return
            result = verify(plugin, args.timeout, args.editor, env)
            with lock:
                done_count += 1
                results.append(result)
                flag = "ok  " if result.get("ok") else "FAIL"
                print(f"[{done_count}/{len(plugins)}] {flag} {result.get('name', '?')} ({result.get('stage', '')}) {result.get('ms', 0)} ms", file=sys.stderr, flush=True)

    threads = [threading.Thread(target=worker, daemon=True) for _ in range(max(1, args.jobs))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    results.sort(key=lambda r: (r.get("ok") is not True, str(r.get("name", "")).lower()))
    with open(results_path, "w", encoding="utf-8") as fh:
        for r in results:
            fh.write(json.dumps(r) + "\n")

    good = [r for r in results if r.get("ok")]
    bad = [r for r in results if not r.get("ok")]
    by_stage = {}
    for r in bad:
        by_stage.setdefault(r.get("stage", "?"), []).append(r)
    lines = [
        "# Plugin verification",
        "",
        f"- plugins: {len(results)} ({len(good)} ok, {len(bad)} failed)",
        f"- mode: {'with editors' if args.editor else 'core only'} · timeout {args.timeout}s · "
        f"{time.time() - started:.0f}s wall clock",
        "",
        "## Failures by stage",
        "",
    ]
    for stage in sorted(by_stage):
        entries = by_stage[stage]
        lines.append(f"### {stage} ({len(entries)})")
        lines.append("")
        lines.append("| plugin | error |")
        lines.append("|---|---|")
        for r in entries:
            error = str(r.get("error", "")).replace("|", "\\|")[:220]
            lines.append(f"| {r.get('name') or r.get('id')} | {error} |")
        lines.append("")
    lines += ["## Every plugin", "", "| plugin | kind | state | editor | ms |", "|---|---|---|---|---|"]
    for r in results:
        kind = "instrument" if r.get("instrument") else ("effect" if "instrument" in r else "?")
        state = r.get("state", "-")
        editor = r.get("editor", "-")
        status = "ok" if r.get("ok") else f"**{r.get('stage', 'failed')}**"
        lines.append(f"| {r.get('name') or r.get('id')} | {kind} | {state} | {editor} | {status} {r.get('ms', 0)} |")
    summary_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"results: {results_path}\nsummary: {summary_path}", file=sys.stderr)
    print(f"{len(good)}/{len(results)} plugins verified", file=sys.stderr)


if __name__ == "__main__":
    main()
