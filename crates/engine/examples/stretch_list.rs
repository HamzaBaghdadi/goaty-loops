//! Lists time-stretched sample channels with their sample length and speed (read-only).
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(r) = import_flp(path.as_ref(), &PluginDb::default()) else { continue };
        let p = r.project;
        for c in &p.channels {
            if let Instrument::Sampler(s) = &c.instrument {
                if s.stretch_beats > 0.0 {
                    let secs = s.data.as_ref().map(|d| d.duration_secs()).unwrap_or(0.0);
                    let target = s.stretch_beats as f64 * 60.0 / p.tempo;
                    println!("{} | {} | {:.1} beats, sample {secs:.1}s -> {target:.1}s (speed {:.2}x)", path.rsplit(['/', '\\']).next().unwrap(), c.name, s.stretch_beats, secs / target.max(1e-6));
                }
            }
        }
    }
}
