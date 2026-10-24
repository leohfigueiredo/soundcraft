#!/usr/bin/env python3
"""List the Kontakt libraries installed on this machine (from a Windows drive or a Wine prefix).

The console's *library* selector reads the JSON this writes, so the list can be refreshed without
rebuilding the app. It reads Native Instruments' own bookkeeping — no plugin has to run:

* ``<root>/Users/Public/Documents/NI Resources/image/<slug>/<slug>.meta``  — the display name
* ``<root>/Users/Public/Documents/Native Instruments/installed_products/*.json`` — the content
  folder of each installed product (``ContentDir``)

Usage: scripts/kontakt-libraries.py [--root DIR]… [--out FILE]
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path

DEFAULT_OUT = Path(os.environ.get("HOME", ".")) / ".config" / "soundcraft" / "kontakt_libraries.json"


def default_roots():
    roots = []
    home = Path(os.environ.get("HOME", "."))
    if (home / ".wine/drive_c").is_dir():
        roots.append(home / ".wine/drive_c")
    # Mounted Windows installations, if any.
    for base in Path("/run/media").glob("*"):
        for candidate in (base / "Windows", base / "windows"):
            if (candidate / "Users").is_dir():
                roots.append(candidate)
    return roots


def meta_names(root):
    """`display name` for every library image a root knows about."""
    images = root / "Users/Public/Documents/NI Resources/image"
    out = {}
    if not images.is_dir():
        return out
    for entry in images.iterdir():
        metas = list(entry.glob("*.meta"))
        if not metas:
            continue
        try:
            text = metas[0].read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        match = re.search(r"<name>(.*?)</name>", text, re.S)
        name = (match.group(1).strip() if match else entry.name).strip()
        if name:
            out.setdefault(name.lower(), {"name": name, "path": "", "slug": entry.name})
    return out


def installed_content(root):
    """`library name` → content folder, from Native Access' installed_products."""
    products = root / "Users/Public/Documents/Native Instruments/installed_products"
    out = {}
    if not products.is_dir():
        return out
    for entry in products.glob("*.json"):
        try:
            data = json.loads(entry.read_text(encoding="utf-8", errors="replace"))
        except (OSError, ValueError):
            continue
        content = data.get("ContentDir") or data.get("ContentDirs") or ""
        if isinstance(content, list):
            content = content[0] if content else ""
        name = entry.stem
        out[name.lower()] = str(content)
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", action="append", default=[], help="Windows drive root or Wine drive_c")
    parser.add_argument("--out", default=str(DEFAULT_OUT))
    args = parser.parse_args()

    roots = [Path(r) for r in args.root] or default_roots()
    if not roots:
        sys.exit("no Windows drive or Wine prefix found; pass --root")
    libraries = {}
    for root in roots:
        for key, info in meta_names(root).items():
            libraries.setdefault(key, info)
        for key, content in installed_content(root).items():
            libraries.setdefault(key, {"name": key.title(), "path": "", "slug": key})
            libraries[key]["path"] = content
    rows = sorted(libraries.values(), key=lambda item: item["name"].lower())
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(rows, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{len(rows)} libraries from {', '.join(str(r) for r in roots)} -> {out}", file=sys.stderr)
    for row in rows:
        print(f"  {row['name']:<34} {row['path']}")


if __name__ == "__main__":
    main()
