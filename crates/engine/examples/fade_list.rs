//! Lists audio clips with fades in one FLP: channel, length, fades (ticks). Read-only.
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
    for c in &p.playlist.clips {
        let ClipSource::Audio(ch) = c.source else { continue };
        if c.fade_in > 0 || c.fade_out > 0 {
            println!("{:<28} start {:>6} len {:>6} fade in {:>5} out {:>5}{}", p.channels[ch].name.chars().take(28).collect::<String>(), c.start, c.length, c.fade_in, c.fade_out, if c.fade_in + c.fade_out > c.length { "  (fades longer than clip)" } else { "" });
        }
    }
}
