//! Checks clip fades and sampler time stretch against FL-made files
//! (read-only): import each, then repeat the edit on base.flp in Goaty Loops
//! and compare the saved values with FL's.
use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};

fn load(dir: &str, name: &str) -> (FlpFile, Project) {
    let f = FlpFile::read(format!("{dir}/{name}")).unwrap();
    let p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
    (f, p)
}

fn summary(p: &Project) -> String {
    let c = p.playlist.clips.iter().find(|c| matches!(c.source, ClipSource::Audio(_)));
    let stretch: Vec<f32> = p.channels.iter().filter_map(|c| match &c.instrument {
        Instrument::Sampler(s) if s.stretch_beats > 0.0 => Some(s.stretch_beats),
        _ => None,
    }).collect();
    format!("clip {:?}  stretch {stretch:?}", c.map(|c| (c.start, c.length, c.fade_in, c.fade_out)))
}

/// Bytes FL writes for the edits: item fade fields and the channel "time" field.
fn edit_bytes(f: &FlpFile) -> (Vec<u8>, Vec<Vec<u8>>) {
    let item = f.events.iter().find(|e| e.id == 233).and_then(|e| e.blob()).map(|b| [&b[36..40], &b[44..48], &b[56..57]].concat()).unwrap_or_default();
    let params = f.events.iter().filter(|e| e.id == 215).filter_map(|e| e.blob().and_then(|b| b.get(96..100)).map(|b| b.to_vec())).collect();
    (item, params)
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let (base_file, base) = load(&dir, "base.flp");
    println!("base:     {}", summary(&base));
    for name in ["fade in.flp", "fade out.flp", "time.flp"] {
        let (fl_file, fl) = load(&dir, name);
        println!("{name:<9} {}", summary(&fl));
        // Make the same edit in Goaty Loops: copy the clip and sampler settings.
        let mut edited = base.clone();
        edited.playlist.clips = fl.playlist.clips.clone();
        for (a, b) in edited.channels.iter_mut().zip(&fl.channels) {
            if let (Instrument::Sampler(x), Instrument::Sampler(y)) = (&mut a.instrument, &b.instrument) {
                x.stretch_beats = y.stretch_beats;
            }
        }
        let saved = patch_flp(&base_file, &edited).file;
        let (fl_bytes, ours) = (edit_bytes(&fl_file), edit_bytes(&saved));
        println!("          bytes match FL: item {} params {}", fl_bytes.0 == ours.0, fl_bytes.1 == ours.1);
        if fl_bytes != ours {
            println!("          FL {:?}\n          BC {:?}", fl_bytes, ours);
        }
    }
}
