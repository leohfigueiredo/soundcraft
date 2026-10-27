#!/usr/bin/env python3
"""Seed the Decent Sampler console with one real plugin state per library.

Why: the console restores a library by loading a saved plugin state. The states saved by hand
all ended up being the *Steinway* state (the plugin still showed the Steinway when "save" was
pressed), so Woven Strings, Rhodes, ... all loaded the piano while the yellow "bound" dot said
otherwise. A Decent Sampler state is the library's `.dspreset` XML expanded by the plugin
(absolute sample path, per-sample `_fileLength`, ...) inside a small JUCE header. This script
builds that state directly from each `.dspreset`, so every model loads its own library.

Usage:
  scripts/decent-seed-presets.py [--root "/path/Decent Samples"] [--win-root 'D:\\Pianote\\Plugins\\Decent Samples'] [--dry-run]

Only the user's own libraries are read; nothing is copied into the repository.
"""
import argparse
import base64
import json
import os
import struct
import sys
import urllib.parse
import re
import xml.etree.ElementTree as ET

try:  # Several community .dspreset files are not well-formed (the plugin is lenient); lxml can recover.
    from lxml import etree as LET
except ImportError:  # pragma: no cover
    LET = None
XM = LET if LET is not None else ET

# Console model name -> (library folder, .dspreset file name). Names match decent_console.rs.
MODELS = {
    "Steinway Grand": ("Steinway Grand  (DS)", "Steinway Grand.dspreset"),
    "Church Steinway": ("Church Steinway (DS)", "Church Steinway.dspreset"),
    "Manthey Klaviano": ("Manthey Klaviano", "Manthey Klaviano.dspreset"),
    "Legacy Knight": ("Legacy Knight Piano v1.0/Legacy Knight 1.0 (DS)", "Legacy Knight.dspreset"),
    "Woven Strings": ("Woven Strings PIANOBOOK/Woven Strings (DS)", "Woven Strings.dspreset"),
    "Beyond the Strings": ("Beyond the strings", "Beyond the Strings.dspreset"),
    "Matt's Fender Rhodes": ("Matt's Fender Rhodes  (DS)", "Matt's Fender Rhodes.dspreset"),
    "Electro Acoustic Piano": ("Electro Acoustic Piano (DS)", "Electro Acoustic Piano.dspreset"),
    # Shipped as a .dslibrary (zip); unzip it next to the folder first: `unzip "Dave Choir.dslibrary"`.
    "Dave Choir": ("Dave Choir", "Dave Choir.dspreset"),
}

# Constant JUCE trailer the plugin writes after the XML (taken from a state it saved itself).
TRAILER = (
    b"\x00" * 9 + b"JUCEPrivateData\x00\x01\x01Bypass\x00\x01\x01\x03\x00\x1d\x00\x00\x00\x00\x00\x00\x00JUCEPrivateData\x00\x00\x00\x00"
)


def wav_info(path):
    """(frames, sample_rate) from a RIFF/WAVE header; None when unreadable."""
    try:
        with open(path, "rb") as f:
            if f.read(4) != b"RIFF":
                return None
            f.read(4)
            if f.read(4) != b"WAVE":
                return None
            rate = align = None
            while True:
                head = f.read(8)
                if len(head) < 8:
                    return None
                tag, size = head[:4], struct.unpack("<I", head[4:])[0]
                if tag == b"fmt ":
                    body = f.read(size)
                    _fmt, _ch, rate, _br, align = struct.unpack("<HHIIH", body[:14])
                elif tag == b"data":
                    if not align or not rate:
                        return None
                    return size // align, rate
                else:
                    f.seek(size + (size & 1), 1)
    except (OSError, struct.error):
        return None


def file_url(win_path):
    drive, rest = win_path.split(":", 1)
    return "file:///" + drive + "%3A" + urllib.parse.quote(rest.replace("\\", "/"), safe="/()'")


def parse_lenient(path):
    with open(path, "rb") as f:
        raw = f.read()
    raw = re.sub(rb"<\?xml[^>]*\?>", b"", raw.lstrip(b"\xef\xbb\xbf"))
    if LET is not None:
        root = LET.fromstring(raw, LET.XMLParser(recover=True, remove_comments=True))
        if root is None:
            raise ET.ParseError("unrecoverable XML")
        return root
    return ET.fromstring(raw)


def build_state(lib_dir, preset_file, win_dir):
    root = parse_lenient(os.path.join(lib_dir, preset_file))
    url = file_url(win_dir + "\\" + preset_file)
    root.attrib.clear()
    root.attrib.update({
        "pluginVersion": "1",
        "_samplePath": win_dir,
        "_presetName": os.path.splitext(preset_file)[0],
        "_libraryUrl": url,
        "_libraryCanonicalUrl": url,
        "_sampleLibraryId": "-1",
        "_libraryBookmark": "0.",
        "_tuningA69Frequency": "440.0",
    })
    for el in root.iter():
        if el.text is not None and not el.text.strip():
            el.text = None
        if el.tail is not None and not el.tail.strip():
            el.tail = None
    missing = 0
    groups = root.find("groups")
    if groups is not None:
        for gi, group in enumerate(groups.findall("group")):
            if group.find("effects") is None:
                XM.SubElement(group, "effects")
            for sample in group.findall("sample"):
                rel = sample.get("path", "")
                info = wav_info(os.path.join(lib_dir, *rel.replace("\\", "/").split("/")))
                if info is None:
                    missing += 1
                else:
                    sample.set("_fileLength", str(info[0]))
                    sample.set("_fileSampleRate", f"{float(info[1]):.1f}")
                sample.set("path", rel.replace("/", "\\"))
                sample.set("_groupIndex", str(gi))
    for tag in ("tags", "midi", "modulators", "noteSequences"):
        if root.find(tag) is None:
            XM.SubElement(root, tag)
    xml = '<?xml version="1.0" encoding="UTF-8"?> ' + XM.tostring(root, encoding="unicode")
    xml = xml.replace(" />", "/>").encode("utf-8")
    blob = b"SCV3" + struct.pack("<I", len(xml) + len(TRAILER) + 4) + b"VC2!" + struct.pack("<I", len(xml)) + xml + TRAILER
    return blob, missing


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default="/run/media/leonardo/Elements/Pianote/Plugins/Decent Samples")
    ap.add_argument("--win-root", default="D:\\Pianote\\Plugins\\Decent Samples")
    ap.add_argument("--presets", default=os.path.expanduser("~/.config/soundcraft/presets/decent_sampler.json"))
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    try:
        with open(args.presets) as f:
            saved = json.load(f)
    except (OSError, ValueError):
        saved = {}
    ok = 0
    for name, (folder, preset) in MODELS.items():
        lib = os.path.join(args.root, *folder.split("/"))
        if not os.path.isfile(os.path.join(lib, preset)):
            print(f"  skip  {name}: {preset} not found under {lib}", file=sys.stderr)
            continue
        win = args.win_root + "\\" + folder.replace("/", "\\")
        try:
            blob, missing = build_state(lib, preset, win)
        except (ET.ParseError, ValueError) as e:
            print(f"  skip  {name}: {e}", file=sys.stderr)
            continue
        print(f"  ok    {name}: {len(blob)} bytes, {missing} sample(s) without wav header info")
        saved[name] = base64.b64encode(blob).decode("ascii")
        ok += 1
    if args.dry_run:
        print("dry run: nothing written")
        return 0
    os.makedirs(os.path.dirname(args.presets), exist_ok=True)
    tmp = args.presets + ".tmp"
    with open(tmp, "w") as f:
        json.dump(saved, f, indent=2)
    os.replace(tmp, args.presets)
    print(f"wrote {ok} library state(s) to {args.presets}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
