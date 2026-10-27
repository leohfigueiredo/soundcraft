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



def scan_plugins_folder():
    """Scan mounted Pianote/Plugins folders for Kontakt libraries (.nicnt and .nki)."""
    found = {}
    search_dirs = [
        Path("/run/media/leonardo/Elements/Pianote/Plugins"),
    ]
    for base in Path("/run/media").glob("*/*"):
        if base.name.lower() in ("plugins", "kontakt samples", "pianote"):
            if base not in search_dirs:
                search_dirs.append(base)

    for base in search_dirs:
        if not base.exists():
            continue
        for root, dirs, files in os.walk(base):
            nkis = [f for f in files if f.lower().endswith(".nki") and not f.startswith("._")]
            nicnts = [f for f in files if f.lower().endswith(".nicnt") and not f.startswith("._")]
            if nicnts:
                nicnt_path = Path(root) / nicnts[0]
                try:
                    import xml.etree.ElementTree as ET
                    raw = nicnt_path.read_bytes()
                    idx = raw.find(b"<?xml")
                    if idx != -1:
                        end_tag = "</ProductHints>"
                        end_idx = raw.find(end_tag.encode("utf-8"))
                        xml_data = raw[idx:end_idx + len(end_tag)]
                        root_elem = ET.fromstring(xml_data)
                        prod = root_elem.find("Product")
                        name = prod.findtext("Name") or nicnt_path.stem
                    else:
                        name = nicnt_path.stem
                except Exception:
                    name = nicnt_path.stem

                win_path = Path(root).as_posix().replace("/run/media/leonardo/Elements", "F:").replace("/", "\\")
                found[name.lower()] = {
                    "name": name,
                    "path": win_path,
                    "slug": nicnt_path.stem.lower()
                }
            elif nkis:
                dir_name = Path(root).name
                if dir_name.lower() not in ("instruments", "instrument", "samples", "sample", "multis"):
                    name = dir_name
                else:
                    name = Path(root).parent.name

                if name.lower() not in found and not name.lower().startswith("template"):
                    win_path = Path(root).as_posix().replace("/run/media/leonardo/Elements", "F:").replace("/", "\\")
                    found[name.lower()] = {
                        "name": name,
                        "path": win_path,
                        "slug": name.lower()
                    }
    return found

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
    # 1. First load plugins folder libraries (with real files on disk)
    for key, info in scan_plugins_folder().items():
        libraries[key] = info

    # 2. Add meta names and installed products from Windows / Wine
    for root in roots:
        for key, info in meta_names(root).items():
            libraries.setdefault(key, info)
        for key, content in installed_content(root).items():
            libraries.setdefault(key, {"name": key.title(), "path": "", "slug": key})
            if content:
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
