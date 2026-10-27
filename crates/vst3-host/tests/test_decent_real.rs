
use soundcraft_dsp::Plugin;

#[test]
fn test_decent_presets_from_json() {
    let p = std::path::Path::new("/home/leonardo/.vst3/yabridge/DecentSampler.vst3");
    if !p.exists() { return; }
    let desc = soundcraft_vst3_host::scan_paths(&[p.to_path_buf()]);
    let mut inst = soundcraft_vst3_host::instantiate_plugin(&desc[0].id).expect("instantiate");
    inst.prepare(44100.0, 512, 2);
    
    let decode_b64 = |s: &str| -> Vec<u8> {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0;
        for &byte in b {
            let val = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => continue,
            };
            buf = (buf << 6) | (val as u32);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        out
    };

    let json_str = std::fs::read_to_string("/home/leonardo/.config/soundcraft/presets/decent_sampler.json").unwrap();
    let map: std::collections::HashMap<String, String> = serde_json::from_str(&json_str).unwrap();
    
    for (name, b64) in &map {
        let state = decode_b64(b64);
        let ok = inst.load_state(&state);
        println!("Preset '{}': len={} load_state={}", name, state.len(), ok);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        inst.reset();
        inst.note_on(0, 60, 100);
        let mut total_energy = 0.0f32;
        for _ in 0..10 {
            let mut buf = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
            inst.process(&mut buf, 512);
            total_energy += buf[0].iter().map(|x| x.abs()).sum::<f32>();
        }
        println!("Preset '{}': total_energy={}", name, total_energy);
    }
}
