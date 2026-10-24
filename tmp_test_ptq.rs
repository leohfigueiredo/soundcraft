
use soundcraft_vst3_host::*;
fn main() {
    let p = std::path::Path::new("/home/leonardo/.vst3/Pianoteq 9.vst3");
    println!("Testing Pianoteq 9 at {:?}", p);
    let desc = scan_bundle(p);
    println!("Descriptors: {}", desc.len());
    for d in &desc {
        println!("  id: {}, name: {}, is_inst: {}", d.id, d.name, d.is_instrument);
    }
}
