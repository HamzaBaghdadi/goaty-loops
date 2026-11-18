//! Checks the sampler reverse flag against FL-made files (read-only):
//! import both, then reverse the base in Goaty Loops and compare with FL's save.
use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};

fn reversed(p: &Project) -> Vec<String> {
    p.channels.iter().filter(|c| matches!(&c.instrument, Instrument::Sampler(s) if s.reverse)).map(|c| c.name.clone()).collect()
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let base = FlpFile::read(format!("{dir}/base.flp")).unwrap();
    let rev = FlpFile::read(format!("{dir}/reverse.flp")).unwrap();
    let db = PluginDb::default();
    let mut pb = convert(&FlpProject::from_file(&base), None, &db).project;
    let pr = convert(&FlpProject::from_file(&rev), None, &db).project;
    println!("base reversed: {:?}", reversed(&pb));
    println!("reverse.flp reversed: {:?}", reversed(&pr));
    // Turn reverse on in Goaty Loops for the channel FL reversed, then save.
    for (a, b) in pb.channels.iter_mut().zip(&pr.channels) {
        if let (Instrument::Sampler(x), Instrument::Sampler(y)) = (&mut a.instrument, &b.instrument) {
            x.reverse = y.reverse;
        }
    }
    let saved = patch_flp(&base, &pb).file;
    let fl70 = |f: &FlpFile| f.events.iter().filter(|e| e.id == 70).map(|e| e.as_u32().unwrap_or(0)).collect::<Vec<_>>();
    println!("event 70 FL: {:?}  Goaty Loops: {:?}  match: {}", fl70(&rev), fl70(&saved), fl70(&rev) == fl70(&saved));
}
