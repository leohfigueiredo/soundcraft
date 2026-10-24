# Pro Tools Session File Format (.ptf / .ptx) Reverse Engineering Specification
*Derived from REA binary inspection and disassembly of Ardour 9's `libptformat.so.0`*

## 1. Overview & Context

In SoundCraft's roadmap (`docs/file-format-parity.md` and `docs/gaps.md` under G-03), native session interchange with Avid Pro Tools (`.ptf` for Pro Tools 7–9 and `.ptx` for 10+) was flagged as:
> `Proprietary, undocumented. Owner decision needed on black-box format study (G-03)`

Through REA Studio and binary reverse engineering of Ardour's production parser (`libptformat.so.0`), we have extracted the exact parser architecture, data structures, chunk iteration logic, and byte-level rules necessary to implement native Pro Tools session import in pure Rust for SoundCraft (`soundcraft-model`).

---

## 2. Binary Architecture & Layout

### 2.1 File Header & Endianness
Pro Tools session files originate from PowerPC (Macintosh) architecture and maintain big-endian representations in older `.ptf` revisions, while supporting little-endian flags on modern platforms:

- **Endianness Flag (`is_big_endian`):** Located at header offset `0xCC`. If set to non-zero, every 32-bit integer (`uint32_t`) and 16-bit word must be byte-swapped (`bswap`).
- **Session Magic / Tag:** Found at offset `+0x08` of the primary root chunk: `0x1028`.
- **Sample Rate Validation:** Stored at offset `+0xA8`. Checked against standard sampling rates:
  - `44100 Hz` (`0xAC44`)
  - `48000 Hz` (`0xBB80`)
  - `88200 Hz` (`0x15888`)
  - `96000 Hz` (`0x17700`)
  - `192000 Hz` (`0x2EE00`)

### 2.2 Chunks and Block Layout
The session is composed of linked blocks (`block_t`):
- Each block descriptor occupies **40 bytes** (`sizeof(block_t) = 0x28`).
- Traversing blocks advances by `0x28` bytes:
  ```rust
  pub struct PtfBlock {
      pub block_type: u16,     // e.g., 0x1028 for header
      pub block_flags: u16,
      pub content_offset: u32, // Offset into payload
      pub content_length: u32,
      pub child_blocks: Vec<PtfBlock>,
  }
  ```

### 2.3 String Encoding
Strings in `.ptf` files are length-prefixed:
1. 32-bit Big-Endian integer specifying the byte length ($L$).
2. Followed immediately by $L$ bytes of UTF-8 / MacRoman character data.
3. Terminating null bytes are not guaranteed in the raw stream.

---

## 3. Data Structures for Rust Implementation (`soundcraft-model`)

```rust
/// Audio asset reference
pub struct PtfAudioClip {
    pub name: String,
    pub file_path: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
}

/// Region on timeline (matches PTFFormat::region_t)
pub struct PtfRegion {
    pub name: String,
    pub audio_file_id: u32,
    pub start_sample: u64,
    pub length_samples: u64,
    pub file_offset_samples: u64,
    pub fade_in_samples: u32,
    pub fade_out_samples: u32,
    pub muted: bool,
}

/// Track structure (matches PTFFormat::track_t)
pub struct PtfTrack {
    pub name: String,
    pub track_type: PtfTrackType, // Audio, MIDI, Aux, Master
    pub channels: u16,
    pub volume_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub regions: Vec<PtfRegion>,
    pub midi_events: Vec<PtfMidiEvent>,
}
```

---

## 4. De-obfuscation (`unxor`)
Certain metadata chunks in Pro Tools files apply a byte delta XOR rotation:
```rust
pub fn unxor_ptf_block(data: &mut [u8], delta: u8) {
    let mut key = delta;
    for b in data.iter_mut() {
        *b ^= key;
        key = key.wrapping_add(0x1F);
    }
}
```

---

## 5. Execution Pipeline
The parser runs in 5 distinct deterministic phases:
1. `parseheader()`: Checks endianness, header tag `0x1028`, and sets sample rate bounds.
2. `setrates()`: Validates session timeline clock.
3. `parseaudio()`: Scans wav pool (`wav_t`) and populates clip references.
4. `parserest()`: Reconstructs track lanes, volume/pan automation, and timeline placement.
5. `parsemidi()`: Extracts MIDI note lists and controller curves.

