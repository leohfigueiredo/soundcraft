//! Scanning standard directories for VST2 plugins (.so, .dll).

use soundcraft_dsp::Category;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 3;
const MAX_PLUGINS: usize = 4096;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vst2Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub path: PathBuf,
    pub is_instrument: bool,
    pub category: Category,
    pub unique_id: i32,
}

pub fn default_search_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push(h.join(".vst"));
        v.push(h.join(".vst/yabridge"));
        v.push(h.join(".lxvst"));
    }
    let ext_plugins = PathBuf::from("/run/media/leonardo/Elements/Pianote/Plugins");
    if ext_plugins.join("VST").is_dir() {
        v.push(ext_plugins.join("VST"));
    }
    if ext_plugins.join("VST2").is_dir() {
        v.push(ext_plugins.join("VST2"));
    }

    v.push(PathBuf::from("/usr/lib/vst"));
    v.push(PathBuf::from("/usr/local/lib/vst"));
    v.push(PathBuf::from("/usr/lib/lxvst"));

    if let Some(p) = std::env::var_os("VST_PATH").or_else(|| std::env::var_os("VST2_PATH")) {
        v.extend(std::env::split_paths(&p).filter(|p| !p.as_os_str().is_empty()));
    }
    v
}

pub fn is_vst2_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if cfg!(target_os = "windows") {
        ext.as_deref() == Some("dll")
    } else {
        // On Linux, native VST2 and yabridge wrappers are .so files
        ext.as_deref() == Some("so")
    }
}

pub fn scan_paths(dirs: &[PathBuf]) -> Vec<Vst2Descriptor> {
    let mut found = Vec::new();
    let mut visited = std::collections::HashSet::new();

    for d in dirs {
        if !d.exists() {
            continue;
        }
        walk_dir(d, 0, &mut found, &mut visited);
        if found.len() >= MAX_PLUGINS {
            break;
        }
    }
    found
}

fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<Vst2Descriptor>, visited: &mut std::collections::HashSet<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_PLUGINS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "SF2" || name_str == "Samples" {
                continue;
            }
            walk_dir(&path, depth + 1, out, visited);
        } else if is_vst2_file(&path) {
            let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
            if visited.insert(canon)
                && let Some(desc) = probe_vst2(&path)
            {
                out.push(desc);
            }
        }
    }
}

pub fn probe_vst2(path: &Path) -> Option<Vst2Descriptor> {
    let stem = path.file_stem()?.to_str()?;
    let name = stem.replace("_x64", "").replace("(64-bit)", "").trim().to_string();

    let name_lower = name.to_ascii_lowercase();
    let is_inst = name_lower.contains("pianoteq")
        || name_lower.contains("triton")
        || name_lower.contains("serenity")
        || name_lower.contains("bass")
        || name_lower.contains("drum")
        || name_lower.contains("piano")
        || name_lower.contains("string")
        || name_lower.contains("synth");

    let cat = if is_inst {
        Category::Instrument
    } else if name_lower.contains("reverb") {
        Category::Reverb
    } else if name_lower.contains("delay") || name_lower.contains("echo") {
        Category::Delay
    } else if name_lower.contains("eq") || name_lower.contains("filter") {
        Category::Eq
    } else if name_lower.contains("comp") || name_lower.contains("limit") || name_lower.contains("gate") {
        Category::Dynamics
    } else {
        Category::Other
    };

    let vendor = if name_lower.contains("pianoteq") {
        "Modartt".to_string()
    } else if name_lower.contains("triton") || name_lower.contains("korg") {
        "KORG".to_string()
    } else if name_lower.contains("valhalla") {
        "Valhalla DSP".to_string()
    } else if name_lower.contains("serenity") || name_lower.contains("kontakt") {
        "Native Instruments".to_string()
    } else if name_lower.contains("fabfilter") {
        "FabFilter".to_string()
    } else if name_lower.contains("soundtoys") {
        "Soundtoys".to_string()
    } else {
        "VST2".to_string()
    };

    // Format safe slug id: vst2:<clean_stem>
    let slug: String = stem.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let id = format!("vst2:{slug}");

    Some(Vst2Descriptor { id, name, vendor, path: path.to_path_buf(), is_instrument: is_inst, category: cat, unique_id: 0 })
}
