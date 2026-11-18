//! Patches an FLP without edits and prints the events that changed (read-only).
use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
    let out = patch_flp(&f, &p).file;
    println!("events {} -> {}", f.events.len(), out.events.len());
    for (i, (a, b)) in f.events.iter().zip(&out.events).enumerate() {
        if a != b {
            println!("event {i} id {}: {:?} -> {:?}", a.id, a.as_u32().map(|v| v.to_string()).unwrap_or_else(|| format!("{} bytes", a.blob().map(|b| b.len()).unwrap_or(0))), b.as_u32());
            if let (Some(x), Some(y)) = (a.blob(), b.blob()) {
                let d: Vec<usize> = (0..x.len()).filter(|&k| x.get(k) != y.get(k)).take(10).collect();
                println!("   differing bytes at {d:?}");
                for (k, (p, q)) in x.chunks(4).zip(y.chunks(4)).enumerate() {
                    if p != q {
                        println!("   word at byte {}: {:02x?} -> {:02x?}", k * 4, p, q);
                    }
                }
            }
        }
    }
}
