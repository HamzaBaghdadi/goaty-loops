use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let file = FlpFile::read(&path).unwrap();
    let project = convert(&FlpProject::from_file(&file), None, &PluginDb::default()).project;
    let patched = patch_flp(&file, &project).file;
    println!("events {} vs {}", file.events.len(), patched.events.len());
    for (i, (a, b)) in file.events.iter().zip(&patched.events).enumerate() {
        if a != b {
            let la = a.blob().map(|x| x.len()); let lb = b.blob().map(|x| x.len());
            println!("event {i} id {} differs: len {:?} vs {:?}; int {:?} vs {:?}", a.id, la, lb, a.as_u32(), b.as_u32());
            if let (Some(x), Some(y)) = (a.blob(), b.blob()) {
                let pos = x.iter().zip(y).position(|(p, q)| p != q);
                println!("   first byte diff at {pos:?}");
            }
        }
    }
}
