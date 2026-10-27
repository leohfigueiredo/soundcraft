//! SoundFont 2 (SF2) parser and multi-sample data structures.
//!
//! Parses RIFF sfbk files (SF2 2.04 specification): INFO metadata, sdta sample data
//! (16-bit PCM), and pdta hydra structures (presets, instruments, generators, sample headers).
//! Multi-sample zones are resolved into flattened, ready-to-play [`ResolvedZone`] records.

use std::sync::Arc;

/// A parsed SoundFont 2 soundbank with its presets and sample pool.
#[derive(Clone)]
pub struct SoundFont {
    pub name: String,
    pub presets: Vec<Sf2Preset>,
    pub samples: Vec<Arc<Sf2Sample>>,
}

/// A playable preset in an SF2 soundbank (e.g. Bank 0, Program 0 "Grand Piano").
#[derive(Clone, Debug)]
pub struct Sf2Preset {
    pub name: String,
    pub bank: u16,
    pub preset: u16,
    pub zones: Vec<ResolvedZone>,
}

/// A mono or stereo channel sample extracted from the SF2's sdta chunk.
pub struct Sf2Sample {
    pub name: String,
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub original_pitch: u8,
    pub pitch_correction: i8,
    pub loop_start: usize,
    pub loop_end: usize,
    pub sample_type: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopMode {
    Off,
    Continuous,
    LoopDuringHeldKey,
}

/// A flattened zone ready for voice mapping and note playback.
#[derive(Clone, Debug)]
pub struct ResolvedZone {
    pub key_lo: u8,
    pub key_hi: u8,
    pub vel_lo: u8,
    pub vel_hi: u8,
    pub sample_idx: usize,
    pub root_key: u8,
    pub tuning_semitones: f64,
    pub scale_tuning: f64,
    pub loop_mode: LoopMode,
    pub loop_start: usize,
    pub loop_end: usize,
    pub attenuation_gain: f32,
    pub pan: f32,
    pub attack_s: f32,
    pub hold_s: f32,
    pub decay_s: f32,
    pub sustain_gain: f32,
    pub release_s: f32,
}

impl SoundFont {
    /// Parses an SF2 file from bytes. Completely safe and bounded against malformed data.
    pub fn parse(bytes: &[u8]) -> Result<SoundFont, String> {
        if bytes.len() < 12 {
            return Err("file too short for RIFF header".to_string());
        }
        if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"sfbk" {
            return Err("not an SF2 (RIFF sfbk) file".to_string());
        }

        let mut name = String::from("SoundFont");
        let mut sample_data: &[u8] = &[];
        let mut pdta_chunks = PdtaChunks::default();

        let mut pos = 12usize;
        while pos + 8 <= bytes.len() {
            let chunk_id = &bytes[pos..pos + 4];
            let chunk_size = u32::from_le_bytes([
                bytes.get(pos + 4).copied().unwrap_or(0),
                bytes.get(pos + 5).copied().unwrap_or(0),
                bytes.get(pos + 6).copied().unwrap_or(0),
                bytes.get(pos + 7).copied().unwrap_or(0),
            ]) as usize;
            pos += 8;

            let end = pos.saturating_add(chunk_size).min(bytes.len());
            let chunk_bytes = bytes.get(pos..end).unwrap_or(&[]);

            if chunk_id == b"LIST" && chunk_bytes.len() >= 4 {
                let list_type = &chunk_bytes[0..4];
                let list_payload = &chunk_bytes[4..];
                match list_type {
                    b"INFO" => {
                        if let Some(inam) = parse_info_name(list_payload) {
                            name = inam;
                        }
                    }
                    b"sdta" => {
                        sample_data = parse_sdta_smpl(list_payload);
                    }
                    b"pdta" => {
                        pdta_chunks = parse_pdta(list_payload);
                    }
                    _ => {}
                }
            }

            // Word-aligned chunk padding
            pos = end + (chunk_size % 2);
        }

        if sample_data.is_empty() {
            return Err("SF2 has no sample data (sdta/smpl)".to_string());
        }

        build_soundfont(name, sample_data, pdta_chunks)
    }
}

fn parse_info_name(bytes: &[u8]) -> Option<String> {
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let cid = &bytes[pos..pos + 4];
        let csize = u32::from_le_bytes([
            bytes.get(pos + 4).copied().unwrap_or(0),
            bytes.get(pos + 5).copied().unwrap_or(0),
            bytes.get(pos + 6).copied().unwrap_or(0),
            bytes.get(pos + 7).copied().unwrap_or(0),
        ]) as usize;
        pos += 8;
        let end = pos.saturating_add(csize).min(bytes.len());
        if cid == b"INAM" {
            let s_bytes = bytes.get(pos..end)?;
            let nul_pos = s_bytes.iter().position(|&b| b == 0).unwrap_or(s_bytes.len());
            let s = String::from_utf8_lossy(&s_bytes[..nul_pos]).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        pos = end + (csize % 2);
    }
    None
}

fn parse_sdta_smpl(bytes: &[u8]) -> &[u8] {
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let cid = &bytes[pos..pos + 4];
        let csize = u32::from_le_bytes([
            bytes.get(pos + 4).copied().unwrap_or(0),
            bytes.get(pos + 5).copied().unwrap_or(0),
            bytes.get(pos + 6).copied().unwrap_or(0),
            bytes.get(pos + 7).copied().unwrap_or(0),
        ]) as usize;
        pos += 8;
        let end = pos.saturating_add(csize).min(bytes.len());
        if cid == b"smpl" {
            return bytes.get(pos..end).unwrap_or(&[]);
        }
        pos = end + (csize % 2);
    }
    &[]
}

#[derive(Default)]
struct PdtaChunks<'a> {
    phdr: &'a [u8],
    pbag: &'a [u8],
    pgen: &'a [u8],
    inst: &'a [u8],
    ibag: &'a [u8],
    igen: &'a [u8],
    shdr: &'a [u8],
}

fn parse_pdta(bytes: &[u8]) -> PdtaChunks<'_> {
    let mut res = PdtaChunks::default();
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let cid = &bytes[pos..pos + 4];
        let csize = u32::from_le_bytes([
            bytes.get(pos + 4).copied().unwrap_or(0),
            bytes.get(pos + 5).copied().unwrap_or(0),
            bytes.get(pos + 6).copied().unwrap_or(0),
            bytes.get(pos + 7).copied().unwrap_or(0),
        ]) as usize;
        pos += 8;
        let end = pos.saturating_add(csize).min(bytes.len());
        let payload = bytes.get(pos..end).unwrap_or(&[]);
        match cid {
            b"phdr" => res.phdr = payload,
            b"pbag" => res.pbag = payload,
            b"pgen" => res.pgen = payload,
            b"inst" => res.inst = payload,
            b"ibag" => res.ibag = payload,
            b"igen" => res.igen = payload,
            b"shdr" => res.shdr = payload,
            _ => {}
        }
        pos = end + (csize % 2);
    }
    res
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let b0 = bytes.get(offset).copied().unwrap_or(0);
    let b1 = bytes.get(offset + 1).copied().unwrap_or(0);
    u16::from_le_bytes([b0, b1])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let b0 = bytes.get(offset).copied().unwrap_or(0);
    let b1 = bytes.get(offset + 1).copied().unwrap_or(0);
    let b2 = bytes.get(offset + 2).copied().unwrap_or(0);
    let b3 = bytes.get(offset + 3).copied().unwrap_or(0);
    u32::from_le_bytes([b0, b1, b2, b3])
}

fn read_ascii_string(bytes: &[u8], offset: usize, max_len: usize) -> String {
    let slice = bytes.get(offset..offset + max_len).unwrap_or(&[]);
    let nul = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..nul]).trim().to_string()
}

// SF2 Generator IDs
const GEN_START_ADDRS_OFFSET: u16 = 0;
const GEN_END_ADDRS_OFFSET: u16 = 1;
const GEN_STARTLOOP_ADDRS_OFFSET: u16 = 2;
const GEN_ENDLOOP_ADDRS_OFFSET: u16 = 3;
const GEN_PAN: u16 = 17;
const GEN_ATTACK_VOL_ENV: u16 = 25;
const GEN_HOLD_VOL_ENV: u16 = 26;
const GEN_DECAY_VOL_ENV: u16 = 27;
const GEN_SUSTAIN_VOL_ENV: u16 = 28;
const GEN_RELEASE_VOL_ENV: u16 = 29;
const GEN_INSTRUMENT: u16 = 41;
const GEN_KEY_RANGE: u16 = 43;
const GEN_VEL_RANGE: u16 = 44;
const GEN_INITIAL_ATTENUATION: u16 = 48;
const GEN_COARSE_TUNE: u16 = 51;
const GEN_FINE_TUNE: u16 = 52;
const GEN_SAMPLE_ID: u16 = 53;
const GEN_SAMPLE_MODES: u16 = 54;
const GEN_SCALE_TUNING: u16 = 56;
const GEN_OVERRIDING_ROOT_KEY: u16 = 58;

#[derive(Clone, Copy, Default)]
struct GenMap {
    key_range: Option<(u8, u8)>,
    vel_range: Option<(u8, u8)>,
    instrument: Option<u16>,
    sample_id: Option<u16>,
    sample_modes: Option<u16>,
    root_key: Option<u8>,
    coarse_tune: i16,
    fine_tune: i16,
    scale_tuning: Option<i16>,
    attenuation_cb: i16,
    pan: i16,
    attack_tc: Option<i16>,
    hold_tc: Option<i16>,
    decay_tc: Option<i16>,
    sustain_cb: Option<i16>,
    release_tc: Option<i16>,
    start_offset: i32,
    end_offset: i32,
    startloop_offset: i32,
    endloop_offset: i32,
}

fn parse_generators(pgen: &[u8], start_idx: usize, end_idx: usize) -> GenMap {
    let mut g = GenMap::default();
    for i in start_idx..end_idx {
        let off = i * 4;
        if off + 4 > pgen.len() {
            break;
        }
        let oper = read_u16(pgen, off);
        let val_u = read_u16(pgen, off + 2);
        let val_i = val_u as i16;
        let lo = (val_u & 0xFF) as u8;
        let hi = ((val_u >> 8) & 0xFF) as u8;

        match oper {
            GEN_START_ADDRS_OFFSET => g.start_offset = val_i as i32,
            GEN_END_ADDRS_OFFSET => g.end_offset = val_i as i32,
            GEN_STARTLOOP_ADDRS_OFFSET => g.startloop_offset = val_i as i32,
            GEN_ENDLOOP_ADDRS_OFFSET => g.endloop_offset = val_i as i32,
            GEN_PAN => g.pan = val_i,
            GEN_ATTACK_VOL_ENV => g.attack_tc = Some(val_i),
            GEN_HOLD_VOL_ENV => g.hold_tc = Some(val_i),
            GEN_DECAY_VOL_ENV => g.decay_tc = Some(val_i),
            GEN_SUSTAIN_VOL_ENV => g.sustain_cb = Some(val_i),
            GEN_RELEASE_VOL_ENV => g.release_tc = Some(val_i),
            GEN_INSTRUMENT => g.instrument = Some(val_u),
            GEN_KEY_RANGE => g.key_range = Some((lo, hi)),
            GEN_VEL_RANGE => g.vel_range = Some((lo, hi)),
            GEN_INITIAL_ATTENUATION => g.attenuation_cb = val_i,
            GEN_COARSE_TUNE => g.coarse_tune = val_i,
            GEN_FINE_TUNE => g.fine_tune = val_i,
            GEN_SAMPLE_ID => g.sample_id = Some(val_u),
            GEN_SAMPLE_MODES => g.sample_modes = Some(val_u),
            GEN_SCALE_TUNING => g.scale_tuning = Some(val_i),
            GEN_OVERRIDING_ROOT_KEY if (0..=127).contains(&val_i) => {
                g.root_key = Some(val_i as u8);
            }
            _ => {}
        }
    }
    g
}

/// Converts timecents to seconds: seconds = 2^(timecents / 1200.0). Clamped to 0.0001s .. 100.0s.
fn timecents_to_seconds(tc: i16) -> f32 {
    let power = (f64::from(tc) / 1200.0).clamp(-10.0, 10.0);
    (2f64.powf(power) as f32).clamp(0.0001, 100.0)
}

fn build_soundfont(name: String, smpl_bytes: &[u8], p: PdtaChunks<'_>) -> Result<SoundFont, String> {
    // 1. Parse sample headers (46 bytes each)
    let num_samples = p.shdr.len() / 46;
    if num_samples == 0 {
        return Err("no sample headers in SF2".to_string());
    }

    // Convert raw 16-bit signed PCM in smpl to f32 (-1.0 .. 1.0)
    let total_pcm_points = smpl_bytes.len() / 2;
    let mut raw_pcm: Vec<f32> = Vec::with_capacity(total_pcm_points);
    for chunk in smpl_bytes.as_chunks::<2>().0 {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]);
        raw_pcm.push(f32::from(s) / 32768.0);
    }

    let mut samples = Vec::with_capacity(num_samples.saturating_sub(1));
    for i in 0..num_samples.saturating_sub(1) {
        let off = i * 46;
        let s_name = read_ascii_string(p.shdr, off, 20);
        let start = read_u32(p.shdr, off + 20) as usize;
        let end = read_u32(p.shdr, off + 24) as usize;
        let start_loop = read_u32(p.shdr, off + 28) as usize;
        let end_loop = read_u32(p.shdr, off + 32) as usize;
        let sample_rate = read_u32(p.shdr, off + 36);
        let original_pitch = p.shdr.get(off + 40).copied().unwrap_or(60);
        let pitch_correction = p.shdr.get(off + 41).copied().unwrap_or(0) as i8;
        let sample_type = read_u16(p.shdr, off + 44);

        let valid_end = end.min(raw_pcm.len());
        let valid_start = start.min(valid_end);
        let slice = raw_pcm.get(valid_start..valid_end).unwrap_or(&[]);
        let data = slice.to_vec();

        let l_start = start_loop.saturating_sub(valid_start).min(data.len());
        let l_end = end_loop.saturating_sub(valid_start).min(data.len());

        samples.push(Arc::new(Sf2Sample {
            name: s_name,
            data,
            sample_rate: sample_rate.max(1000),
            original_pitch,
            pitch_correction,
            loop_start: l_start,
            loop_end: l_end,
            sample_type,
        }));
    }

    // 2. Parse instruments (22 bytes each)
    let num_insts = p.inst.len() / 22;
    let num_ibags = p.ibag.len() / 4;

    struct InstZone {
        gens: GenMap,
    }
    struct InstDef {
        zones: Vec<InstZone>,
    }

    let mut instruments: Vec<InstDef> = Vec::with_capacity(num_insts.saturating_sub(1));
    for i in 0..num_insts.saturating_sub(1) {
        let off = i * 22;
        let bag_idx = read_u16(p.inst, off + 20) as usize;
        let next_bag_idx = read_u16(p.inst, off + 22 + 20) as usize;

        let b_start = bag_idx.min(num_ibags);
        let b_end = next_bag_idx.min(num_ibags).max(b_start);

        let mut zones = Vec::new();
        for b in b_start..b_end {
            let boff = b * 4;
            let g_idx = read_u16(p.ibag, boff) as usize;
            let next_g = read_u16(p.ibag, boff + 4).max(read_u16(p.ibag, boff)) as usize;
            let gens = parse_generators(p.igen, g_idx, next_g);
            zones.push(InstZone { gens });
        }
        instruments.push(InstDef { zones });
    }

    // 3. Parse presets (38 bytes each)
    let num_presets = p.phdr.len() / 38;
    let num_pbags = p.pbag.len() / 4;

    let mut presets = Vec::with_capacity(num_presets.saturating_sub(1));
    for i in 0..num_presets.saturating_sub(1) {
        let off = i * 38;
        let pr_name = read_ascii_string(p.phdr, off, 20);
        let preset_num = read_u16(p.phdr, off + 20);
        let bank_num = read_u16(p.phdr, off + 22);
        let bag_idx = read_u16(p.phdr, off + 24) as usize;
        let next_bag_idx = read_u16(p.phdr, off + 38 + 24) as usize;

        let b_start = bag_idx.min(num_pbags);
        let b_end = next_bag_idx.min(num_pbags).max(b_start);

        let mut preset_zones = Vec::new();
        let mut global_pgen: Option<GenMap> = None;

        for b in b_start..b_end {
            let boff = b * 4;
            let g_idx = read_u16(p.pbag, boff) as usize;
            let next_g = read_u16(p.pbag, boff + 4).max(read_u16(p.pbag, boff)) as usize;
            let gens = parse_generators(p.pgen, g_idx, next_g);

            if gens.instrument.is_none() {
                global_pgen = Some(gens);
                continue;
            }

            // Target instrument
            let inst_idx = gens.instrument.unwrap_or(0) as usize;
            let Some(inst) = instruments.get(inst_idx) else { continue };

            let mut global_igen: Option<GenMap> = None;
            for iz in &inst.zones {
                if iz.gens.sample_id.is_none() {
                    global_igen = Some(iz.gens);
                    continue;
                }

                let s_idx = iz.gens.sample_id.unwrap_or(0) as usize;
                let Some(sample) = samples.get(s_idx) else { continue };

                // Combine ranges
                let pk = gens.key_range.or_else(|| global_pgen.and_then(|g| g.key_range)).unwrap_or((0, 127));
                let ik = iz.gens.key_range.or_else(|| global_igen.and_then(|g| g.key_range)).unwrap_or((0, 127));
                let k_lo = pk.0.max(ik.0);
                let k_hi = pk.1.min(ik.1);
                if k_lo > k_hi {
                    continue;
                }

                let pv = gens.vel_range.or_else(|| global_pgen.and_then(|g| g.vel_range)).unwrap_or((1, 127));
                let iv = iz.gens.vel_range.or_else(|| global_igen.and_then(|g| g.vel_range)).unwrap_or((1, 127));
                let v_lo = pv.0.max(iv.0);
                let v_hi = pv.1.min(iv.1);
                if v_lo > v_hi {
                    continue;
                }

                // Root key
                let root_key = iz.gens.root_key.or(global_igen.and_then(|g| g.root_key)).or(gens.root_key).unwrap_or(sample.original_pitch);

                // Tuning
                let c_tune = (iz.gens.coarse_tune + global_igen.map_or(0, |g| g.coarse_tune) + gens.coarse_tune) as f64;
                let f_tune = (iz.gens.fine_tune + global_igen.map_or(0, |g| g.fine_tune) + gens.fine_tune + sample.pitch_correction as i16) as f64;
                let tuning_semis = c_tune + (f_tune / 100.0);

                let scale_tuning = iz.gens.scale_tuning.or(global_igen.and_then(|g| g.scale_tuning)).or(gens.scale_tuning).unwrap_or(100) as f64;

                // Loop
                let sm = iz.gens.sample_modes.or(global_igen.and_then(|g| g.sample_modes)).or(gens.sample_modes).unwrap_or(0);
                let loop_mode = match sm & 0x03 {
                    1 => LoopMode::Continuous,
                    3 => LoopMode::LoopDuringHeldKey,
                    _ => LoopMode::Off,
                };

                let l_start = sample.loop_start.saturating_add_signed(iz.gens.startloop_offset as isize);
                let l_end = sample.loop_end.saturating_add_signed(iz.gens.endloop_offset as isize);

                // Attenuation
                let att_cb = iz.gens.attenuation_cb + global_igen.map_or(0, |g| g.attenuation_cb) + gens.attenuation_cb;
                let att_gain = 10f32.powf(-f32::from(att_cb) / 200.0).clamp(0.0, 2.0);

                // Pan (-500 .. +500 => -1.0 .. +1.0)
                let pan_val = iz.gens.pan + global_igen.map_or(0, |g| g.pan) + gens.pan;
                let pan = (f32::from(pan_val) / 500.0).clamp(-1.0, 1.0);

                // Envelope
                let att_tc = iz.gens.attack_tc.or(global_igen.and_then(|g| g.attack_tc)).unwrap_or(-12000);
                let hold_tc = iz.gens.hold_tc.or(global_igen.and_then(|g| g.hold_tc)).unwrap_or(-12000);
                let dec_tc = iz.gens.decay_tc.or(global_igen.and_then(|g| g.decay_tc)).unwrap_or(0);
                let sus_cb = iz.gens.sustain_cb.or(global_igen.and_then(|g| g.sustain_cb)).unwrap_or(0);
                let rel_tc = iz.gens.release_tc.or(global_igen.and_then(|g| g.release_tc)).unwrap_or(-1200);

                let attack_s = timecents_to_seconds(att_tc);
                let hold_s = timecents_to_seconds(hold_tc);
                let decay_s = timecents_to_seconds(dec_tc);
                let sustain_gain = 10f32.powf(-f32::from(sus_cb) / 200.0).clamp(0.0, 1.0);
                let release_s = timecents_to_seconds(rel_tc);

                preset_zones.push(ResolvedZone {
                    key_lo: k_lo,
                    key_hi: k_hi,
                    vel_lo: v_lo,
                    vel_hi: v_hi,
                    sample_idx: s_idx,
                    root_key,
                    tuning_semitones: tuning_semis,
                    scale_tuning,
                    loop_mode,
                    loop_start: l_start,
                    loop_end: l_end,
                    attenuation_gain: att_gain,
                    pan,
                    attack_s,
                    hold_s,
                    decay_s,
                    sustain_gain,
                    release_s,
                });
            }
        }

        presets.push(Sf2Preset { name: pr_name, bank: bank_num, preset: preset_num, zones: preset_zones });
    }

    Ok(SoundFont { name, presets, samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parse_acoustic_piano() {
        let path = "/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Acoustic Piano.sf2";
        if Path::new(path).exists() {
            let bytes = std::fs::read(path).expect("read piano");
            let sf = SoundFont::parse(&bytes).expect("parse piano");
            assert!(!sf.presets.is_empty());
            assert!(!sf.samples.is_empty());
        }
    }

    #[test]
    fn test_parse_grand_upright() {
        let path = "/home/leonardo/Downloads/Grand_Upright__N.sf2";
        if Path::new(path).exists() {
            let bytes = std::fs::read(path).expect("read upright");
            let sf = SoundFont::parse(&bytes).expect("parse upright");
            assert!(!sf.presets.is_empty());
            assert!(!sf.samples.is_empty());
        }
    }

    #[test]
    fn test_parse_clavinet() {
        let path = "/run/media/leonardo/Elements/Pianote/Plugins/SF2/Pianos_Keys/Clavinet.sf2";
        if Path::new(path).exists() {
            let bytes = std::fs::read(path).expect("read clavinet");
            let sf = SoundFont::parse(&bytes).expect("parse clavinet");
            assert_eq!(sf.presets.len(), 1);
            assert!(sf.presets[0].name.contains("Clavinet"));
            assert!(!sf.presets[0].zones.is_empty());
            assert!(!sf.samples.is_empty());
        }
    }

    #[test]
    fn test_corrupt_data_never_panics() {
        assert!(SoundFont::parse(&[]).is_err());
        assert!(SoundFont::parse(b"RIFFxxxxsfbk").is_err());
        assert!(SoundFont::parse(&[0u8; 1000]).is_err());
    }
}
