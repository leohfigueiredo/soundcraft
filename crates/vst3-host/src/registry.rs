//! The process-wide VST3 registry: scan cache, leaked `PluginInfo`s, instantiation.

use crate::ffi::{Bundle, Instance, PARAM_BYPASS, PARAM_HIDDEN, PARAM_READ_ONLY, RawParam};
use crate::plugin::Vst3Plugin;
use crate::scan as paths;
use crate::{AUDIO_MODULE_CLASS, Vst3Descriptor, Vst3Error, parse_id};
use soundcraft_dsp::{ParamInfo, Plugin, PluginInfo, Taper, Unit};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Most labels read for a stepped parameter (more steps are shown as a plain range).
const MAX_CHOICES: i32 = 128;

#[derive(Default)]
struct Registry {
    scanned: Option<Vec<Vst3Descriptor>>,
    extra_dirs: Vec<PathBuf>,
    /// One leaked `PluginInfo` per plugin type that was actually instantiated (bounded by the
    /// number of distinct installed plugins).
    infos: HashMap<String, &'static PluginInfo>,
    /// One leaked `PluginInfo` per catalogue entry, built from the descriptor alone. A command
    /// that only needs the id, the name and the kind uses these: loading a bridged Windows
    /// plugin to learn its parameters costs seconds under Wine, and one that hangs there would
    /// freeze the interface.
    meta: HashMap<String, &'static PluginInfo>,
    /// Ids that failed to instantiate (not retried every UI frame).
    failed: HashSet<String>,
}

fn load_disk_cache_if_needed(r: &mut Registry) {
    if r.scanned.is_some() {
        return;
    }
    let mut descriptors: Vec<Vst3Descriptor> = Vec::new();
    let cache_path = std::env::var_os("SOUNDCRAFT_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("soundcraft")))
        .map(|d| d.join("plugins_cache.json"));
    if let Some(path) = cache_path
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(vst3_val) = val.get("vst3")
        && let Ok(cached) = serde_json::from_value::<Vec<Vst3Descriptor>>(vst3_val.clone())
    {
        descriptors.extend(cached);
    }
    if !r.extra_dirs.is_empty() {
        for d in scan_paths(&r.extra_dirs) {
            if !descriptors.iter().any(|o| o.id == d.id) {
                descriptors.push(d);
            }
        }
    }
    if !descriptors.is_empty() {
        r.scanned = Some(descriptors);
    }
}

fn scan_all() -> Vec<Vst3Descriptor> {
    scan_paths(&search_dirs())
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

/// Descriptors of every audio module class in one `.vst3`.
pub fn load_bundle(path: &Path) -> Result<Vec<Vst3Descriptor>, Vst3Error> {
    let b = Bundle::load(path)?;
    Ok(b.classes()
        .into_iter()
        .filter(|c| c.category == AUDIO_MODULE_CLASS)
        .map(|c| {
            let subs = paths::split_sub_categories(&c.sub_categories);
            let class_id = crate::ffi::hex(&c.cid);
            Vst3Descriptor {
                id: format!("{}{class_id}", crate::ID_PREFIX),
                category: paths::category_from_sub_categories(&subs),
                is_instrument: paths::is_instrument(&subs),
                class_id,
                name: if c.name.is_empty() { "VST3 plugin".to_string() } else { c.name },
                vendor: c.vendor,
                version: c.version,
                sdk_version: c.sdk_version,
                sub_categories: subs,
                path: path.to_path_buf(),
            }
        })
        .collect())
}

/// True for a bundle that yabridge generated (a Windows plugin run through Wine).
fn is_bridged(path: &Path) -> bool {
    path.to_string_lossy().contains("yabridge")
}

/// Whether `candidate` should replace an already-kept bundle with the same plugin id: the bridged
/// (Windows, normally licensed) build wins by default, the native one when `SOUNDCRAFT_PREFER_NATIVE`
/// is set.
fn candidate_wins(kept: &Vst3Descriptor, candidate: &Vst3Descriptor) -> bool {
    if prefer_native() { is_bridged(&kept.path) && !is_bridged(&candidate.path) } else { !is_bridged(&kept.path) && is_bridged(&candidate.path) }
}

/// Whether a native bundle should win over a bridged one with the same plugin id
/// (`SOUNDCRAFT_PREFER_NATIVE=1`). Off by default: a licensed Windows install normally beats a
/// native trial build.
fn prefer_native() -> bool {
    static PREFER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PREFER.get_or_init(|| {
        std::env::var("SOUNDCRAFT_PREFER_NATIVE").is_ok_and(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
    })
}

/// Scans `dirs` (uncached). Unloadable files are logged and skipped; when two bundles expose the
/// same plugin id the bridged one wins unless `SOUNDCRAFT_PREFER_NATIVE=1`.
pub fn scan_paths(dirs: &[PathBuf]) -> Vec<Vst3Descriptor> {
    let mut out: Vec<Vst3Descriptor> = Vec::new();
    for path in paths::find_bundles(dirs) {
        match load_bundle(&path) {
            Ok(ds) => {
                for d in ds {
                    match out.iter().position(|o| o.id == d.id) {
                        None => out.push(d),
                        // The same plugin installed twice (a native build and a bridged one) keeps
                        // one id; `candidate_wins` decides which copy is the one a session loads.
                        Some(i) if candidate_wins(&out[i], &d) => out[i] = d,
                        Some(_) => {}
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
pub fn adopt_scan(descriptors: Vec<Vst3Descriptor>) {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut list: Vec<Vst3Descriptor> = Vec::with_capacity(descriptors.len());
    for d in descriptors {
        match list.iter().position(|o| o.id == d.id) {
            None => list.push(d),
            Some(i) if candidate_wins(&list[i], &d) => list[i] = d,
            Some(_) => {}
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

/// The folders to scan: the standard locations plus every folder the user added. Reading the
/// extra folders is a short lock; if a scan holds it, fall back to the standard locations only
/// (the extra folders are merged as soon as that scan finishes).
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = paths::default_search_paths();
    if let Ok(r) = registry().try_lock() {
        dirs.extend(r.extra_dirs.iter().cloned());
    }
    dirs
}

/// Scans `dirs` (uncached) *outside* the registry lock, so a slow walk of a large or removable
/// drive never blocks the UI's catalogue readers.
pub fn scan() -> Vec<Vst3Descriptor> {
    // Already scanned: return the cache without rescanning.
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
    r.scanned.as_ref().and_then(|list| list.iter().find(|d| d.id == id).map(|d| d.path.clone()))
}

/// The cached catalogue. The lock is only ever held for fast in-memory merges (the disk walk and
/// `dlopen` happen outside it), so this never blocks on a slow scan of a removable drive.
pub fn cached() -> Option<Vec<Vst3Descriptor>> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    load_disk_cache_if_needed(&mut r);
    r.scanned.clone()
}

/// Forgets the cached catalogue and scans again. The walk and `dlopen` happen without holding
/// the registry lock; the lock is taken only to store the result.
pub fn rescan() -> Vec<Vst3Descriptor> {
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

/// Adds a folder to the search path and merges its plugins into the cache. The scan itself runs
/// without holding the lock. Call off the UI thread (it walks the folder and loads each plugin).
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

/// Maps a VST3 unit label to a SoundCraft display unit.
pub(crate) fn unit_from_label(units: &str) -> Unit {
    match units.trim().to_ascii_lowercase().as_str() {
        "db" => Unit::Db,
        "hz" => Unit::Hz,
        "ms" => Unit::Ms,
        "%" => Unit::Percent,
        "s" | "sec" | "secs" | "seconds" => Unit::Seconds,
        "st" | "semi" | "semitones" => Unit::Semitones,
        "ct" | "cent" | "cents" => Unit::Cents,
        _ => Unit::None,
    }
}

fn param_info(inst: &Instance, p: &RawParam) -> ParamInfo {
    let name = if p.title.is_empty() { format!("Param {}", p.id) } else { p.title.clone() };
    let base = ParamInfo {
        id: leak(p.id.to_string()),
        name: leak(name),
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        taper: Taper::Linear,
        choices: &[],
    };
    if p.step_count == 1 {
        return ParamInfo { unit: Unit::Toggle, default: p.default_normalized.round() as f32, ..base };
    }
    if (2..=MAX_CHOICES).contains(&p.step_count) {
        let steps = f64::from(p.step_count);
        let labels: Vec<&'static str> =
            (0..=p.step_count).map(|i| leak(inst.value_text(p.id, f64::from(i) / steps).unwrap_or_else(|| i.to_string()))).collect();
        return ParamInfo {
            max: p.step_count as f32,
            default: (p.default_normalized * steps).round() as f32,
            unit: Unit::Choice,
            choices: Box::leak(labels.into_boxed_slice()),
            ..base
        };
    }
    // Continuous (or too many steps for a menu): plain range from the controller.
    let unit = unit_from_label(&p.units);
    match (inst.to_plain(p.id, 0.0), inst.to_plain(p.id, 1.0)) {
        (Some(lo), Some(hi)) if hi > lo && (lo as f32).is_finite() && (hi as f32).is_finite() && (hi as f32) > (lo as f32) => {
            let default = inst.to_plain(p.id, p.default_normalized).map_or(lo, |d| d.clamp(lo, hi));
            // A logarithmic plugin mapping (frequency knobs) shows its geometric mean at mid travel.
            let taper = match inst.to_plain(p.id, 0.5) {
                Some(mid)
                    if lo > 0.0
                        && ((mid - (lo * hi).sqrt()).abs() <= 0.01 * (lo * hi).sqrt())
                        && (mid - (lo + hi) / 2.0).abs() > 0.01 * (hi - lo) =>
                {
                    Taper::Log
                }
                _ => Taper::Linear,
            };
            ParamInfo { min: lo as f32, max: hi as f32, default: default as f32, unit, taper, ..base }
        }
        _ => ParamInfo { default: p.default_normalized as f32, unit, ..base },
    }
}

/// Builds and leaks the `PluginInfo` for one plugin type. Called at most once per id (cached).
fn build_info(d: &Vst3Descriptor, inst: &Instance) -> &'static PluginInfo {
    let params: Vec<ParamInfo> =
        inst.params().iter().filter(|p| p.flags & (PARAM_HIDDEN | PARAM_READ_ONLY | PARAM_BYPASS) == 0).map(|p| param_info(inst, p)).collect();
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
fn lookup_descriptor(id: &str) -> Result<Vst3Descriptor, Vst3Error> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if r.failed.contains(id) {
        return Err(Vst3Error::Instantiate(id.to_string(), "failed before".into()));
    }
    load_disk_cache_if_needed(&mut r);
    r.scanned.as_ref().and_then(|s| s.iter().find(|d| d.id == id)).cloned().ok_or_else(|| Vst3Error::NotFound(id.to_string()))
}

/// Finds the descriptor, loads its binary and creates an instance plus its (cached) info.
fn instantiate_raw(id: &str) -> Result<(Instance, &'static PluginInfo), Vst3Error> {
    let cid = parse_id(id).ok_or_else(|| Vst3Error::NotFound(id.to_string()))?;
    let desc = lookup_descriptor(id)?;
    // A bridged plugin runs inside Wine and answers its own Win32 event loop at ~60 Hz; letting it
    // register with our run loop as well puts two threads behind one GUI.
    let bridged = desc.path.to_string_lossy().contains("yabridge");
    let made = Bundle::load(&desc.path).and_then(|b| b.instantiate(&cid));
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    match made {
        Ok(inst) => {
            if bridged {
                inst.set_run_loop_enabled(false);
            }
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

/// Diagnostic: how many parameters the edit controller reports before and after the component is
/// configured. Some plugins build their parameter list only on activation, which would leave them
/// with no parameters at all if the host read them once at instantiation time.
pub fn param_count_timeline(id: &str, sample_rate: f64, max_block: usize, mono: bool) -> Option<(usize, usize)> {
    let (mut inst, _) = instantiate_raw(id).ok()?;
    let before = inst.params().len();
    inst.configure(sample_rate, max_block, mono).ok()?;
    let after = inst.params().len();
    Some((before, after))
}

pub fn instantiate(id: &str) -> Result<Box<dyn Plugin>, Vst3Error> {
    Ok(Box::new(instantiate_plugin(id)?))
}

pub fn instantiate_plugin(id: &str) -> Result<Vst3Plugin, Vst3Error> {
    let (inst, info) = instantiate_raw(id)?;
    let mut p = Vst3Plugin::new(inst, info);
    p.prepare(48_000.0, 1024, 2);
    Ok(p)
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    let failed = match registry().try_lock() {
        Err(_) => return None, // a scan is running: the caller retries on a later repaint
        Ok(r) => {
            if let Some(i) = r.infos.get(id) {
                return Some(*i);
            }
            r.failed.contains(id)
        }
    };
    if failed {
        return None;
    }
    // Instantiate once to learn the parameters (VST3 exposes them only on a live controller).
    // This is the actual load; `lookup_descriptor` refuses to block on a concurrent scan.
    instantiate_raw(id).ok().map(|(_, info)| info)
}

/// What this process already knows about a plugin, loading nothing: its parameters once an instance
/// has been created, otherwise just the catalogue entry (empty `params`).
///
/// [`plugin_info`] instantiates the plugin to read its parameter list — seconds under Wine for a
/// bridged plugin, and a freeze if Wine is stuck — so anything drawn per UI frame must call this.
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
/// that only need the id, the name and the kind use this, so inserting a bridged Windows plugin
/// never pays for a Wine instantiation (and never hangs on one).
pub fn descriptor_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(info) = r.meta.get(id) {
        return Some(*info);
    }
    load_disk_cache_if_needed(&mut r);
    let d = r.scanned.as_ref()?.iter().find(|d| d.id == id)?.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_labels_map() {
        assert_eq!(unit_from_label("dB"), Unit::Db);
        assert_eq!(unit_from_label(" Hz "), Unit::Hz);
        assert_eq!(unit_from_label("%"), Unit::Percent);
        assert_eq!(unit_from_label("ms"), Unit::Ms);
        assert_eq!(unit_from_label("sec"), Unit::Seconds);
        assert_eq!(unit_from_label("cents"), Unit::Cents);
        assert_eq!(unit_from_label("st"), Unit::Semitones);
        assert_eq!(unit_from_label("furlongs"), Unit::None);
    }
}
