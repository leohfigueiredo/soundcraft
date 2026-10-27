//! SoundFont 2 loader (`mix.soundfont_load`): loads an SF2 soundbank into an instrument track's
//! SoundFont Player (setting the track's instrument to "soundfont" if it is not already).

use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_model::{Insert, TrackKind};
use std::path::Path;

const SOUNDFONT: &str = "soundfont";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "mix.soundfont_load",
        "Load SoundFont",
        [],
        None,
        "{track?, path, bank?: 0..128, preset?: 0..127}: loads an SF2 SoundFont into the track's SoundFont Player (made its instrument if it is not already) → {track, path, bank, preset, size}",
        has_selection,
        load_soundfont
    )]
}

/// The instrument track a soundfont command acts on.
fn instrument_track(e: &Engine, id: &str, p: &Value) -> Result<TrackId> {
    let t = tracks_required(e, id, p)?.first().copied().ok_or_else(|| bad(id, "no track"))?;
    if e.session().track(t).map(|tr| tr.kind) != Some(TrackKind::Instrument) {
        return Err(bad(id, "only instrument tracks have a soundfont player"));
    }
    Ok(t)
}

fn load_soundfont(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.soundfont_load";
    let path = str_param(p, "path").ok_or_else(|| bad(id, "`path` required"))?.to_string();
    let file_path = Path::new(&path);
    if !crate::io::file_found(file_path) {
        return Err(bad(id, format!("file not found: `{path}`")));
    }

    let bank = p.get("bank").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let preset = p.get("preset").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

    let t = instrument_track(e, id, p)?;

    // Make SoundFont the instrument on this track if it is not already
    let s = e.session_mut();
    let tr = s.track_mut(t).ok_or_else(|| bad(id, "no track"))?;
    let keep = tr.instrument.as_ref().is_some_and(|i| i.plugin == SOUNDFONT);
    if !keep {
        let ins = Insert::new(SOUNDFONT.to_string());
        tr.instrument = Some(ins);
    }

    // Set the state
    let state_json = format!("{{\"path\":\"{path}\",\"bank\":{bank},\"preset\":{preset}}}");
    if let Some(ins) = tr.instrument.as_mut() {
        ins.set_state_bytes(state_json.as_bytes());
    }

    Ok(json!({
    "track": t.0,
    "path": path,
    "bank": bank,
    "preset": preset,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument_track(e: &mut Engine) -> TrackId {
        e.execute("track.new", &json!({"kind": "instrument", "format": "stereo"})).unwrap();
        e.session().tracks.last().unwrap().id
    }

    #[test]
    fn loading_soundfont_sets_track_instrument_and_state() {
        let mut e = Engine::default();
        let t = instrument_track(&mut e);
        let path = "/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Clavinet.sf2";
        if std::path::Path::new(path).exists() {
            let res = e.execute("mix.soundfont_load", &json!({"track": t.0, "path": path, "preset": 0})).unwrap();
            assert_eq!(res["track"], t.0);
            assert_eq!(res["path"], path);
            let tr = e.session().track(t).unwrap();
            assert_eq!(tr.instrument.as_ref().map(|i| i.plugin.as_str()), Some("soundfont"));
            let state = tr.instrument.as_ref().and_then(|i| i.state_bytes()).unwrap();
            let state_str = std::str::from_utf8(&state).unwrap();
            assert!(state_str.contains("Clavinet.sf2"));
        }
    }
}
