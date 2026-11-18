use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let file = FlpFile::read(&path).unwrap();
    let flp = FlpProject::from_file(&file);
    println!("version {:?}", flp.version);
    let mut project = convert(&flp, None, &PluginDb::default()).project;
    project.playlist.clips[0].start += 384;
    let patched = patch_flp(&file, &project).file;
    let re = convert(&FlpProject::from_file(&patched), None, &PluginDb::default()).project;
    for (i, (a, b)) in project.playlist.clips.iter().zip(&re.playlist.clips).enumerate() {
        if a != b { println!("clip {i}: {a:?}\n     -> {b:?}"); break; }
    }
}
