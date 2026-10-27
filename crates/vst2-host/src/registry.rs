//! In-process registry and caching for hosted VST2 plugins.

use crate::ffi::Instance;
use crate::plugin::Vst2Plugin;
use crate::scan::{Vst2Descriptor, default_search_paths, scan_paths};
use soundcraft_dsp::{ParamInfo, Plugin, PluginInfo, Taper, Unit};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Registry {
    scanned: Option<Vec<Vst2Descriptor>>,
    extra_dirs: Vec<PathBuf>,
    infos: HashMap<String, &'static PluginInfo>,
    meta: HashMap<String, &'static PluginInfo>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

pub fn scan() -> Vec<Vst2Descriptor> {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(list) = &reg.scanned {
        return list.clone();
    }
    let mut dirs = default_search_paths();
    dirs.extend(reg.extra_dirs.clone());
    let list = scan_paths(&dirs);
    reg.scanned = Some(list.clone());
    list
}

pub fn rescan() -> Vec<Vst2Descriptor> {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut dirs = default_search_paths();
    dirs.extend(reg.extra_dirs.clone());
    let list = scan_paths(&dirs);
    reg.scanned = Some(list.clone());
    list
}

pub fn cached() -> Option<Vec<Vst2Descriptor>> {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if reg.scanned.is_none() {
        let mut dirs = default_search_paths();
        dirs.extend(reg.extra_dirs.clone());
        let list = scan_paths(&dirs);
        reg.scanned = Some(list);
    }
    reg.scanned.clone()
}

pub fn register_search_dir(dir: &std::path::Path) {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let pb = dir.to_path_buf();
    if !reg.extra_dirs.contains(&pb) {
        reg.extra_dirs.push(pb);
    }
}

pub fn adopt_scan(descriptors: Vec<Vst2Descriptor>) {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    reg.scanned = Some(descriptors);
}

pub fn descriptor(id: &str) -> Option<Vst2Descriptor> {
    if !id.starts_with("vst2:") && !id.to_ascii_lowercase().contains("pianoteq") {
        return None;
    }
    let list = scan();
    list.into_iter().find(|d| d.id == id || d.name == id || d.id.ends_with(id))
}

pub fn descriptor_info(id: &str) -> Option<&'static PluginInfo> {
    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(info) = reg.meta.get(id) {
        return Some(*info);
    }
    let desc = descriptor(id)?;
    let name: &'static str = Box::leak(desc.name.into_boxed_str());
    let id_str: &'static str = Box::leak(desc.id.into_boxed_str());
    let info = Box::leak(Box::new(PluginInfo {
        id: id_str,
        name,
        short_name: name,
        category: desc.category,
        params: &[],
        is_instrument: desc.is_instrument,
        audiosuite: false,
    }));
    reg.meta.insert(id_str.to_string(), info);
    Some(info)
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    let reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(info) = reg.infos.get(id) {
        return Some(*info);
    }
    drop(reg);

    let desc = descriptor(id)?;
    let inst = Instance::load(&desc.path, 48000.0).ok()?;

    let name: &'static str = Box::leak(desc.name.into_boxed_str());
    let id_str: &'static str = Box::leak(desc.id.into_boxed_str());

    let mut params = Vec::with_capacity(inst.num_params());
    for i in 0..inst.num_params() {
        let p_name = inst.param_name(i);
        let p_label = inst.param_label(i);
        let pid_str: &'static str = Box::leak(i.to_string().into_boxed_str());
        let pname_str: &'static str = Box::leak(p_name.into_boxed_str());
        let def = inst.get_param(i);
        let unit = if p_label.is_empty() { Unit::None } else { Unit::Percent };
        params.push(ParamInfo { id: pid_str, name: pname_str, min: 0.0, max: 1.0, default: def, unit, taper: Taper::Linear, choices: &[] });
    }

    let params_slice: &'static [ParamInfo] = Box::leak(params.into_boxed_slice());
    let info = Box::leak(Box::new(PluginInfo {
        id: id_str,
        name,
        short_name: name,
        category: desc.category,
        params: params_slice,
        is_instrument: desc.is_instrument,
        audiosuite: false,
    }));

    let mut reg = match registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    reg.infos.insert(id_str.to_string(), info);
    Some(info)
}

pub fn create(id: &str) -> Option<Box<dyn Plugin>> {
    let desc = descriptor(id)?;
    let inst = Instance::load(&desc.path, 48000.0).ok()?;
    let info = plugin_info(&desc.id).or_else(|| descriptor_info(&desc.id))?;
    let mut plug = Vst2Plugin::new(inst, info);
    plug.prepare(48000.0, 1024, 2);
    Some(Box::new(plug))
}
