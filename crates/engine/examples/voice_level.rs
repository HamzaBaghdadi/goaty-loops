//! Plays one sample channel's voice directly (no mixer) for a few seconds
//! and prints its level next to the sample's own level. Reads files only.
use goaty_engine::import_flp::import_flp;
use goaty_engine::instruments::{NoteParams, Voice};
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (path, name) = (a.next().unwrap(), a.next().unwrap());
    let p = import_flp(path.as_ref(), &PluginDb::default())?.project;
    let ch = p.channels.iter().find(|c| c.name.contains(&name)).unwrap();
    let Instrument::Sampler(s) = &ch.instrument else { panic!("not a sampler") };
    let d = s.data.as_ref().unwrap();
    let n = 8 * 48000;
    let src_rms = (d.samples[..(8 * d.sample_rate as usize * 2).min(d.samples.len())].iter().map(|v| v * v).sum::<f32>() / (8 * d.sample_rate as usize * 2) as f32).sqrt();
    let np = NoteParams { key: s.root_key, velocity: 1.0, pan: 0.0, gate: None, delay: 0, offset_secs: 0.0, tempo: p.tempo, fade: None };
    let mut v = Voice::new(0, &ch.instrument, &np, 48000.0, 1).unwrap();
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    v.render(&mut l, &mut r, 1.0, 1.0, 1.0);
    let out_rms = (l.iter().chain(&r).map(|v| v * v).sum::<f32>() / (2 * n) as f32).sqrt();
    println!("sample {:.1} dB, voice output {:.1} dB (sample rate {}, root {}, use_env {})", 20.0 * src_rms.log10(), 20.0 * out_rms.log10(), d.sample_rate, s.root_key, s.use_envelope);
    Ok(())
}
