
use soundcraft_dsp::Plugin;

#[test]
fn test_decent_programs() {
    let p = std::path::Path::new("/home/leonardo/.vst3/yabridge/DecentSampler.vst3");
    if !p.exists() { return; }
    let desc = soundcraft_vst3_host::scan_paths(&[p.to_path_buf()]);
    let inst = soundcraft_vst3_host::instantiate_plugin(&desc[0].id).expect("instantiate");
    let lists = inst.program_lists();
    println!("DecentSampler program lists: {}", lists.len());
    for l in &lists {
        println!("  List '{}': {} programs", l.name, l.programs.len());
        for p in &l.programs {
            println!("    {}", p);
        }
    }
}
