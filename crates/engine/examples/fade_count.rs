//! Counts audio clips with fades and time-stretched sample channels (read-only).
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let (mut faded, mut clips, mut stretched) = (0, 0, 0);
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
        for c in p.playlist.clips.iter().filter(|c| matches!(c.source, ClipSource::Audio(_))) {
            clips += 1;
            if c.fade_in > 0 || c.fade_out > 0 { faded += 1; }
        }
        stretched += p.channels.iter().filter(|c| matches!(&c.instrument, Instrument::Sampler(s) if s.stretch_beats > 0.0)).count();
    }
    println!("audio clips {clips}, with fades {faded}; time-stretched sample channels {stretched}");
}
