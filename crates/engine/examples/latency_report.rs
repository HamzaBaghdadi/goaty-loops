//! Prints each plugin's reported latency and the total compensation for an FLP.
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use goaty_engine::render::Renderer;
use goaty_engine::*;
use std::sync::atomic::Ordering;
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let mut p = import_flp(path.as_ref(), &PluginDb::load())?.project;
    p.instantiate_plugins(48000.0);
    for (t, track) in p.mixer.iter().enumerate() {
        for e in &track.effects {
            if let Effect::Plugin(pp) = &e.effect {
                if let Some(i) = pp.instance.as_ref().filter(|i| i.latency > 0) {
                    println!("insert {t} {:<14} {:<24} {} samples", track.name, pp.info.name, i.latency);
                }
            }
        }
    }
    let shared = Shared::new(p);
    shared.transport.playing.store(true, Ordering::Relaxed);
    let mut r = Renderer::new(48000, None);
    let mut buf = vec![0f32; 1024];
    r.process(&shared, &mut buf);
    let l = shared.latency_samples.load(Ordering::Relaxed);
    println!("total compensation: {l} samples ({:.1} ms)", l as f64 / 48.0);
    Ok(())
}
