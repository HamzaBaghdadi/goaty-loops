//! Adds a note to pattern 0 of an FLP, patches, and shows the pattern events
//! and re-imported notes (read-only on the input).
use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let mut p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
    println!("patterns {:?}", p.patterns.iter().map(|x| (x.name.clone(), x.fl_iid, x.notes.len())).collect::<Vec<_>>());
    let ch = p.channels.iter().position(|c| c.fl_iid.is_some()).unwrap();
    p.patterns[0].notes.push(Note { channel: ch, key: 72, start: 0, length: PPQ, velocity: 0.5, pan: 0.0 });
    let out = patch_flp(&f, &p).file;
    let pats: Vec<(usize, u8)> = out.events.iter().enumerate().filter(|(_, e)| matches!(e.id, ids::PAT_NEW | ids::PAT_NOTES | ids::PAT_NAME)).map(|(i, e)| (i, e.id)).collect();
    println!("pattern events in output: {pats:?}");
    let re = convert(&FlpProject::from_file(&out), None, &PluginDb::default()).project;
    println!("re-imported {:?}", re.patterns.iter().map(|x| (x.name.clone(), x.fl_iid, x.notes.len())).collect::<Vec<_>>());
}
