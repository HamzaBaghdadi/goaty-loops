//! Prints insert routing and Limiter sidechain settings of an FLP (read-only).
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let p = convert(&FlpProject::from_file(&FlpFile::read(&path).unwrap()), None, &PluginDb::default()).project;
    for (i, t) in p.mixer.iter().enumerate().take(4) {
        let lim: Vec<String> = t.effects.iter().filter_map(|e| match e.effect { Effect::Limiter { sidechain, .. } => Some(format!("Limiter(sidechain {sidechain})")), _ => None }).collect();
        println!("{i}: out {} sends {:?} {}", t.output, t.sends.iter().map(|s| (s.to, s.gain)).collect::<Vec<_>>(), lim.join(" "));
    }
}
