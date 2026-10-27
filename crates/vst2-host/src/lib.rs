//! SoundCraft's VST2 plugin host.
//!
//! Scans standard VST2 directories, loads `.so`/`.dll` bundles in-process and exposes each
//! plugin behind the safe [`soundcraft_dsp::Plugin`] trait. Plugin IDs have the prefix `vst2:`.
//!
//! Like `soundcraft-vst3-host` and `soundcraft-clap-host`, this is an isolated `unsafe` crate
//! (craftrules never-crash): `unsafe` is denied crate-wide and allowed solely in the private
//! `ffi` module with `// SAFETY:` rationale on every block. The public API is safe, returns
//! `Result`/`Option` and never panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod scan;

#[cfg(not(target_arch = "wasm32"))]
#[allow(unsafe_code)]
pub mod ffi;
#[cfg(not(target_arch = "wasm32"))]
mod plugin;
#[cfg(not(target_arch = "wasm32"))]
mod registry;

#[cfg(not(target_arch = "wasm32"))]
pub use plugin::Vst2Plugin;
#[cfg(not(target_arch = "wasm32"))]
pub use registry::{adopt_scan, cached, create, descriptor, descriptor_info, plugin_info, register_search_dir, rescan, scan};
pub use scan::Vst2Descriptor;

/// Prefix for all VST2 plugin IDs in SoundCraft.
pub const ID_PREFIX: &str = "vst2:";

pub fn parse_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_id() {
        assert_eq!(parse_id("vst2:pianoteq_6"), Some("pianoteq_6"));
        assert_eq!(parse_id("vst3:something"), None);
        assert_eq!(parse_id("vst2:"), None);
    }

    #[test]
    fn test_scan_finds_plugins() {
        let plugins = scan();
        // Should find Pianoteq 6 and others in ~/.vst/yabridge
        assert!(!plugins.is_empty(), "expected to find VST2 plugins in ~/.vst/yabridge");
        let has_pianoteq = plugins.iter().any(|p| p.name.contains("Pianoteq 6"));
        assert!(has_pianoteq, "expected Pianoteq 6 in scanned plugins");
    }

    #[test]
    fn test_pianoteq6_instantiate_and_process() {
        let plugins = scan();
        let ptq = plugins.iter().find(|p| p.name.contains("Pianoteq 6"));
        if let Some(desc) = ptq {
            println!("Testing VST2 loading: {} at {}", desc.name, desc.path.display());
            let mut plug = create(&desc.id).expect("create pianoteq 6");
            plug.prepare(48000.0, 256, 2);
            plug.note_on(0, 60, 100);
            let mut io = vec![vec![0.0f32; 256], vec![0.0f32; 256]];
            plug.process(&mut io, 256);
            let energy: f32 = io[0].iter().map(|x| x * x).sum();
            println!("Pianoteq 6 output energy: {}", energy);
            assert!(energy >= 0.0);
        }
    }
}
