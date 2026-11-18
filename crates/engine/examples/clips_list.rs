//! Lists imported patterns and playlist clips of an FLP (read-only).
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let flp = FlpProject::from_file(&FlpFile::read(&path).unwrap());
    println!("FL patterns: {:?}", flp.patterns.iter().map(|p| (p.iid, p.notes.len())).collect::<Vec<_>>());
    let p = convert(&flp, None, &PluginDb::default()).project;
    println!("patterns: {:?}", p.patterns.iter().map(|x| (x.name.clone(), x.fl_iid)).collect::<Vec<_>>());
    for c in &p.playlist.clips {
        println!("clip {c:?}");
    }
}
