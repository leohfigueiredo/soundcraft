# Implementation Plan for SoundCraft Feature Gaps

This document outlines the implementation approach for key gaps identified in comparing Ardour 9 with SoundCraft.

## Gap 1: Sidechain/Key Input Support (Compressor/Gate)

### Status: Partially Implemented
- Added `key_input` toggle parameter to Compressor plugin
- Parameter is recognized and can be set via UI

### Implementation Details:
```rust
// In dynamics.rs - added key_input param at index 11:
toggle("key_input", "Key Input", false),
```

### Future Work Required:
- Audio routing system changes to support sidechain inputs
- Plugin processing layer modifications in `process()` functions 
- MIDI-to-audio conversion for key input signals
- Cross-plugin sidechain connection framework

## Gap 2: Live MIDI Input Support

### Status: Not Implemented (G-02)
- No live MIDI input routing capability
- MIDI is only supported as file import/export

### Implementation Path:
- Add `MidiInput` trait to plugin system
- Implement MIDI event processing in engine layer
- Create MIDI-to-Audio bridge components

## Gap 3: ARA 2 Integration

### Status: Not Implemented  
- No ARA 2 menu integration (Track/Clip/Window → ARA)

### Implementation Path:
- Add ARA 2 library dependencies
- Implement `ARAHost` interface in core engine
- Create UI integration for ARA controls  

## Gap 4: Multiple Plugin Editor Pages

### Status: Partially Supported (🟡)
- CLAP editors work but multi-page support missing
- VST3 editors partially functional on some platforms

### Implementation Path:
- Extend editor framework to handle multiple pages
- Add page navigation UI elements in plugin windows

## Gap 5: Lua Automation Scripts 

### Status: Not Implemented  
- No equivalent of Ardour's `s_plugin_automation.lua`, etc.

### Implementation Path:
- Add Lua 5.3 runtime integration 
- Implement script loading and execution framework
- Create basic automation API for plugins

## Next Steps:

1. **Immediate**: Test current key_input parameter in Compressor
2. **Short-term (1-2 weeks)**: Implement audio routing infrastructure for sidechain inputs  
3. **Medium-term (1 month)**: Add MIDI live input support and Lua automation scripts
4. **Long-term (2+ months)**: Full ARA 2 integration and multi-page plugin editors

## Testing Approach:

For the current key_input implementation:
- Verify parameter appears in UI 
- Confirm parameter value is accessible via API
- Validate no runtime errors when enabled/disabled
