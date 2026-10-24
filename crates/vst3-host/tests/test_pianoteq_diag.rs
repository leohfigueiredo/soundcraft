use soundcraft_dsp::Plugin;

#[test]
fn pianoteq_program_change_switches_sound() {
    let p = std::path::Path::new("/home/leonardo/.vst3/Pianoteq 9.vst3");
    if !p.exists() {
        return;
    }
    let desc = soundcraft_vst3_host::scan_paths(&[p.to_path_buf()]);
    for d in &desc {
        if let Ok(mut inst) = soundcraft_vst3_host::instantiate_plugin(&d.id) {
            inst.prepare(44100.0, 512, 2);
            // Render a note with Program 0
            inst.select_program(0, 0);
            inst.note_on(0, 60, 100);
            let mut buf_prog0 = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
            inst.process(&mut buf_prog0, 512);

            // Now reset and render with Program 11
            inst.reset();
            inst.select_program(0, 11);
            inst.note_on(0, 60, 100);
            let mut buf_prog11 = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
            inst.process(&mut buf_prog11, 512);

            let diff: f32 = buf_prog0[0].iter().zip(&buf_prog11[0]).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 0.01, "Program 11 must sound different from Program 0 (diff={diff})");

            // Also verify MIDI 0xC0 program change
            inst.reset();
            inst.midi(0, [0xC0, 22, 0]);
            assert_eq!(inst.param("1886548852"), Some(22.0), "MIDI 0xC0 must select Program 22");
        }
    }
}
