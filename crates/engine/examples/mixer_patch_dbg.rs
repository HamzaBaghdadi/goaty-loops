use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let file = FlpFile::read(&path).unwrap();
    let mut project = convert(&FlpProject::from_file(&file), None, &PluginDb::default()).project;
    println!("mixer[1] before {}", project.mixer[1].volume);
    project.mixer[1].volume = 0.5;
    let patched = patch_flp(&file, &project).file;
    for (a, b) in file.events.iter().zip(&patched.events).filter(|(a, _)| a.id == ids::MIXER_PARAMS) {
        let (a, b) = (a.blob().unwrap(), b.blob().unwrap());
        for (ca, cb) in a.chunks_exact(12).zip(b.chunks_exact(12)) {
            if ca != cb { println!("changed {:?} -> {:?}", ca, cb); }
        }
    }
    let fp = FlpProject::from_file(&patched);
    println!("reparsed insert 1 volume raw {:?}", fp.inserts.get(1).map(|i| i.volume));
    let re = convert(&fp, None, &PluginDb::default()).project;
    println!("reimported {}", re.mixer[1].volume);
}
