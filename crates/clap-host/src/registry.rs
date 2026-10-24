//! The process-wide CLAP registry: scan cache, leaked `PluginInfo`s, instantiation.

use crate::ffi::{Bundle, Instance, RawParam};
use crate::plugin::ClapPlugin;
use crate::scan as paths;
use crate::{ClapDescriptor, ClapError, parse_id};
use clap_sys::ext::params::{CLAP_PARAM_IS_ENUM, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY, CLAP_PARAM_IS_STEPPED};
use soundcraft_dsp::{ParamInfo, Plugin, PluginInfo, Taper, Unit};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Most labels read for an enum parameter.
const MAX_CHOICES: f64 = 128.0;

#[derive(Default)]
struct Registry {
    scanned: Option<Vec<ClapDescriptor>>,
    extra_dirs: Vec<PathBuf>,
    /// One leaked `PluginInfo` per plugin type that was actually instantiated (bounded by the
    /// number of distinct installed plugins).
    infos: HashMap<String, &'static PluginInfo>,
    /// One leaked `PluginInfo` per catalogue entry, built from the descriptor alone: commands
    /// that only need the id, the name and the kind must never pay for a plugin load.
    meta: HashMap<String, &'static PluginInfo>,
    /// Ids that failed to instantiate (not retried every UI frame).
    failed: HashSet<String>,
}

fn load_disk_cache_if_needed(r: &mut Registry) {
    if r.scanned.is_some() {
        return;
    }
    let Some(path) = std::env::var_os("SOUNDCRAFT_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("soundcraft")))
        .map(|d| d.join("plugins_cache.json"))
    else {
        return;
    };
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(clap_val) = val.get("clap")
        && let Ok(descriptors) = serde_json::from_value::<Vec<ClapDescriptor>>(clap_val.clone())
        && !descriptors.is_empty()
    {
        log::info!("clap-host: loaded {} descriptors from disk cache ({})", descriptors.len(), path.display());
        r.scanned = Some(descriptors);
    }
}

fn scan_all() -> Vec<ClapDescriptor> {
    scan_paths(&search_dirs())
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

/// Descriptors of every plugin in one `.clap` file or bundle.
pub fn load_bundle(path: &Path) -> Result<Vec<ClapDescriptor>, ClapError> {
    let b = Bundle::load(path)?;
    Ok(b.descriptors()
        .into_iter()
        .map(|d| ClapDescriptor {
            id: format!("{}{}", crate::ID_PREFIX, d.id),
            category: paths::category_from_features(&d.features),
            is_instrument: paths::is_instrument(&d.features),
            plugin_id: d.id,
            name: d.name,
            vendor: d.vendor,
            version: d.version,
            description: d.description,
            features: d.features,
            path: path.to_path_buf(),
        })
        .collect())
}

/// Scans `dirs` (uncached). Unloadable files are logged and skipped; the first plugin with a
/// given id wins.
pub fn scan_paths(dirs: &[PathBuf]) -> Vec<ClapDescriptor> {
    let mut out: Vec<ClapDescriptor> = Vec::new();
    for path in paths::find_bundles(dirs) {
        match load_bundle(&path) {
            Ok(ds) => {
                for d in ds {
                    if !out.iter().any(|o| o.id == d.id) {
                        out.push(d);
                    }
                }
            }
            Err(e) => log::warn!("{e}"),
        }
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

/// Replaces the catalogue with a scan produced by another process (the `--scan-plugins` helper).
/// No plugin code runs here: only the descriptor list is stored.
pub fn adopt_scan(descriptors: Vec<ClapDescriptor>) {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut list: Vec<ClapDescriptor> = Vec::with_capacity(descriptors.len());
    for d in descriptors {
        if !list.iter().any(|o| o.id == d.id) {
            list.push(d);
        }
    }
    list.sort_by_key(|d| d.name.to_lowercase());
    r.scanned = Some(list);
    r.failed.clear();
    r.meta.clear();
}

/// The folders registered for scanning (the standard locations are not included).
pub fn extra_dirs() -> Vec<PathBuf> {
    registry().lock().unwrap_or_else(PoisonError::into_inner).extra_dirs.clone()
}

/// The folders to scan: the standard locations plus every folder the user added.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = paths::default_search_paths();
    if let Ok(r) = registry().try_lock() {
        dirs.extend(r.extra_dirs.iter().cloned());
    }
    dirs
}

/// Scans `dirs` (uncached) *outside* the registry lock, so a slow walk of a large or removable
/// drive never blocks the UI's catalogue readers.
pub fn scan() -> Vec<ClapDescriptor> {
    if let Ok(r) = registry().try_lock()
        && let Some(list) = &r.scanned
    {
        return list.clone();
    }
    let found = scan_all();
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if r.scanned.is_none() {
        r.scanned = Some(found.clone());
    }
    r.scanned.as_ref().cloned().unwrap_or(found)
}

/// The bundle a plugin id lives in, from the catalogue. Cheap: no plugin code is loaded.
pub fn bundle_path(id: &str) -> Option<PathBuf> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    load_disk_cache_if_needed(&mut r);
    if let Some(p) = r.scanned.as_ref().and_then(|list| list.iter().find(|d| d.id == id).map(|d| d.path.clone())) {
        return Some(p);
    }
    let found = scan_all();
    let p = found.iter().find(|d| d.id == id).map(|d| d.path.clone());
    if let Some(list) = r.scanned.as_mut() {
        for item in found {
            if !list.iter().any(|o| o.id == item.id) {
                list.push(item);
            }
        }
    } else {
        r.scanned = Some(found);
    }
    p
}

/// The cached catalogue. The lock is only ever held for fast in-memory merges (the disk walk and
/// `dlopen` happen outside it), so this never blocks on a slow scan of a removable drive.
pub fn cached() -> Option<Vec<ClapDescriptor>> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    load_disk_cache_if_needed(&mut r);
    r.scanned.clone()
}

/// Forgets the cached catalogue and scans again, holding the lock only to store the result.
pub fn rescan() -> Vec<ClapDescriptor> {
    let found = scan_all();
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    r.scanned = Some(found.clone());
    r.failed.clear();
    r.meta.clear();
    found
}

/// Registers a folder in the search path without scanning it. The next (background) `rescan`
/// picks it up; registering is O(1), so it is safe to call from the UI thread.
pub fn register_search_dir(dir: &Path) {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if !r.extra_dirs.iter().any(|d| d == dir) {
        r.extra_dirs.push(dir.to_path_buf());
    }
}

/// Adds a folder to the search path and merges its plugins into the cache. The scan runs
/// without holding the lock. Call off the UI thread.
pub fn add_search_dir(dir: &Path) -> usize {
    {
        let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
        if !r.extra_dirs.iter().any(|d| d == dir) {
            r.extra_dirs.push(dir.to_path_buf());
        }
    }
    let found = scan_paths(&[dir.to_path_buf()]);
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let list = r.scanned.get_or_insert_with(Vec::new);
    let mut n = 0;
    for d in found {
        if !list.iter().any(|o| o.id == d.id) {
            list.push(d);
            n += 1;
        }
    }
    n
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn param_info(inst: &Instance, p: &RawParam) -> ParamInfo {
    let stepped = p.flags & CLAP_PARAM_IS_STEPPED != 0;
    let is_enum = p.flags & CLAP_PARAM_IS_ENUM != 0;
    let (mut unit, mut choices): (Unit, &'static [&'static str]) = (Unit::None, &[]);
    if stepped && p.min == 0.0 && p.max == 1.0 && !is_enum {
        unit = Unit::Toggle;
    } else if stepped && p.min == 0.0 && p.max >= 1.0 && p.max <= MAX_CHOICES {
        let labels: Vec<&'static str> =
            (0..=p.max as u32).map(|i| leak(inst.value_text(p.id, f64::from(i)).unwrap_or_else(|| i.to_string()))).collect();
        unit = Unit::Choice;
        choices = Box::leak(labels.into_boxed_slice());
    }
    let name = match (p.module.is_empty(), p.name.is_empty()) {
        (_, true) => format!("Param {}", p.id),
        (true, false) => p.name.clone(),
        (false, false) => format!("{}/{}", p.module, p.name),
    };
    ParamInfo {
        id: leak(p.id.to_string()),
        name: leak(name),
        min: p.min as f32,
        max: p.max as f32,
        default: p.default as f32,
        unit,
        taper: Taper::Linear,
        choices,
    }
}

/// Builds and leaks the `PluginInfo` for one plugin type. Called at most once per id (cached).
fn build_info(d: &ClapDescriptor, inst: &Instance) -> &'static PluginInfo {
    let params: Vec<ParamInfo> =
        inst.params().iter().filter(|p| p.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY) == 0).map(|p| param_info(inst, p)).collect();
    let short: String = d.name.chars().take(8).collect();
    Box::leak(Box::new(PluginInfo {
        id: leak(d.id.clone()),
        name: leak(d.name.clone()),
        short_name: leak(short),
        category: d.category,
        params: Box::leak(params.into_boxed_slice()),
        is_instrument: d.is_instrument,
        audiosuite: false,
    }))
}

/// Finds a plugin's descriptor in the catalogue. Discovery is explicit because scanning loads
/// third-party modules and must not be triggered by a render or a UI lookup.
fn lookup_descriptor(id: &str) -> Result<ClapDescriptor, ClapError> {
    let plugin_id = parse_id(id).ok_or_else(|| ClapError::NotFound(id.to_string()))?;
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if r.failed.contains(id) {
        return Err(ClapError::Instantiate(plugin_id.to_string(), "failed before".into()));
    }
    load_disk_cache_if_needed(&mut r);
    if let Some(d) = r.scanned.as_ref().and_then(|s| s.iter().find(|d| d.id == id)).cloned() {
        return Ok(d);
    }
    let found = scan_all();
    let d = found.iter().find(|d| d.id == id).cloned();
    if let Some(list) = r.scanned.as_mut() {
        for item in found {
            if !list.iter().any(|o| o.id == item.id) {
                list.push(item);
            }
        }
    } else {
        r.scanned = Some(found);
    }
    d.ok_or_else(|| ClapError::NotFound(id.to_string()))
}

/// Finds the descriptor, loads its binary and creates an instance plus its (cached) info.
fn instantiate_raw(id: &str) -> Result<(Instance, &'static PluginInfo), ClapError> {
    let plugin_id = parse_id(id).ok_or_else(|| ClapError::NotFound(id.to_string()))?;
    let desc = lookup_descriptor(id)?;
    let made = Bundle::load(&desc.path).and_then(|b| b.instantiate(plugin_id));
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    match made {
        Ok(inst) => {
            let info = match r.infos.get(id) {
                Some(i) => *i,
                None => {
                    let i = build_info(&desc, &inst);
                    r.infos.insert(id.to_string(), i);
                    i
                }
            };
            Ok((inst, info))
        }
        Err(e) => {
            r.failed.insert(id.to_string());
            Err(e)
        }
    }
}

pub fn instantiate(id: &str) -> Result<Box<dyn Plugin>, ClapError> {
    let (inst, info) = instantiate_raw(id)?;
    let mut p = ClapPlugin::new(inst, info);
    p.prepare(48_000.0, 1024, 2);
    Ok(Box::new(p))
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    {
        let r = registry().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = r.infos.get(id) {
            return Some(*i);
        }
        if r.failed.contains(id) {
            return None;
        }
    }
    // Instantiate once to learn the parameters (CLAP exposes them only on an instance).
    instantiate_raw(id).ok().map(|(_, info)| info)
}

/// What this process already knows about a plugin, loading nothing: its parameters once an instance
/// has been created, otherwise just the catalogue entry (empty `params`).
///
/// [`plugin_info`] loads the module to read its parameter list. That is seconds for a bridged plugin
/// and a stall if the bridge is stuck, so anything that runs per UI frame must call this instead.
pub fn cached_plugin_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    if let Ok(r) = registry().try_lock()
        && let Some(info) = r.infos.get(id)
    {
        return Some(*info);
    }
    descriptor_info(id)
}

/// A catalogue entry as [`PluginInfo`], built from its descriptor: no module is loaded. Commands
/// that only need the id, the name and the kind use this instead of [`plugin_info`].
pub fn descriptor_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(info) = r.meta.get(id) {
        return Some(*info);
    }
    load_disk_cache_if_needed(&mut r);
    let d = if let Some(d) = r.scanned.as_ref().and_then(|s| s.iter().find(|d| d.id == id)).cloned() {
        d
    } else {
        let found = scan_all();
        let d = found.iter().find(|d| d.id == id).cloned()?;
        if let Some(list) = r.scanned.as_mut() {
            for item in found {
                if !list.iter().any(|o| o.id == item.id) {
                    list.push(item);
                }
            }
        } else {
            r.scanned = Some(found);
        }
        d
    };
    let info: &'static PluginInfo = Box::leak(Box::new(PluginInfo {
        id: leak(d.id.clone()),
        name: leak(d.name.clone()),
        short_name: leak(d.name.chars().take(8).collect()),
        category: d.category,
        params: &[],
        is_instrument: d.is_instrument,
        audiosuite: false,
    }));
    r.meta.insert(id.to_string(), info);
    Some(info)
}
