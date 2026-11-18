//! Renders an FLP with the master effects bypassed vs. enabled and prints the
//! level of the first 16 s, to see how much the master chain contributes.
use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
fn level(path: &std::path::Path) -> f32 {
    let s: Vec<f32> = hound::WavReader::open(path).unwrap().samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
    let n = (16 * 48000 * 2).min(s.len());
    20.0 * (s[..n].iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt().log10()
}
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let mut p = import_flp(path.as_ref(), &PluginDb::load())?.project;
    p.instantiate_plugins(48000.0);
    let limit = p.bar_ticks() * 8;
    p.playlist.clips.retain(|c| c.start < limit);
    let out = std::env::temp_dir().join("bc_master_test.wav");
    let opts = ExportOptions { sample_rate: 48000, tail_secs: 0.0, ..Default::default() };
    render_to_wav(&p, &out, &opts, |_| {})?;
    println!("master effects on:  {:.1} dB", level(&out));
    for e in &mut p.mixer[0].effects {
        println!("  master slot: {} enabled {}", e.effect.name(), e.enabled);
        e.enabled = false;
    }
    render_to_wav(&p, &out, &opts, |_| {})?;
    println!("master effects off: {:.1} dB", level(&out));
    Ok(())
}
