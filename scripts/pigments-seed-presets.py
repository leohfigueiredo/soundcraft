#!/usr/bin/env python3
"""Seed Arturia Pigments console with one real plugin state per model.

Builds VST3 SCV3 state blobs from real Pigments 6.0 factory presets so that clicking
any of the 13 models in the Pigments console actually changes the synth sound.
"""
import base64
import json
import os
import struct
import sys

FACTORY_DIR = "/run/media/leonardo/Windows/ProgramData/Arturia/Presets/Pigments/Factory/Pigments 6.0"
OUTPUT_FILE = os.path.expanduser("~/.config/soundcraft/presets/pigments.json")

MODEL_PRESETS = {
    "Warm Analog Pad": "Trap Sparkle Pad",
    "Glass Wavetable": "Bells Of Varanasi",
    "Granular Cloud": "Pleiades",
    "Solid Sub Bass": "Bass Ventura",
    "Acid Wavetable Bass": "Weird Harmonics Hard Bass",
    "Screaming Lead": "Chunk",
    "Soft Sine Lead": "Sine Dust",
    "Electric Keys": "Ambivalent String Keys",
    "Plucked Keys": "Temple Pluck",
    "Arp Sequence": "Thin Plucked Arp",
    "Rhythmic Pulse": "Candy Blade",
    "Slow Texture": "Polar Loss",
    "Metallic Texture": "The Wooden Oracle",
}

def make_scv3_blob(content: bytes) -> bytes:
    state = bytearray()
    state.extend(b"SCV3")
    state.extend(struct.pack("<I", len(content)))
    state.extend(content)
    state.extend(struct.pack("<I", len(content)))
    state.extend(content)
    return bytes(state)

def main():
    if not os.path.isdir(FACTORY_DIR):
        print(f"Factory dir not found: {FACTORY_DIR}", file=sys.stderr)
        sys.exit(1)

    os.makedirs(os.path.dirname(OUTPUT_FILE), exist_ok=True)

    out = {}
    for model_name, preset_name in MODEL_PRESETS.items():
        preset_path = os.path.join(FACTORY_DIR, preset_name)
        if not os.path.isfile(preset_path):
            print(f"Warning: {preset_name} not found at {preset_path}")
            continue
        with open(preset_path, "rb") as f:
            content = f.read()
        blob = make_scv3_blob(content)
        encoded = base64.b64encode(blob).decode("ascii")
        out[model_name] = encoded
        print(f"  ok  {model_name} -> {preset_name} ({len(blob)} bytes)")

    with open(OUTPUT_FILE, "w") as f:
        json.dump(out, f, indent=2)

    print(f"Wrote {len(out)} Pigments preset states to {OUTPUT_FILE}")

if __name__ == "__main__":
    main()
