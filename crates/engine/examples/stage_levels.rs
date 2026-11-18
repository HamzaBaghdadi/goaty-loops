//! Renders the first seconds of an FLP keeping only the clips that start
//! within them, and prints the RMS of the output and of each mixer stem, to
//! locate level differences stage by stage. Reads the FLP only.
use goaty_engine::export::{render_stems, ExportOptions};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
fn rms_db(path: &std::path::Path) -> f32 {
    let s: Vec<f32> = hound::WavReader::open(path).unwrap().samples::<f32>().map(|v| v.unwrap()).collect();
    let n = (8 * 48000 * 2).min(s.len());
    20.0 * (s[..n].iter().map(|v| v * v).sum::<f32>() / n.max(1) as f32).sqrt().max(1e-9).log10()
}
fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (path, out) = (a.next().unwrap(), a.next().unwrap());
    let mut p = import_flp(path.as_ref(), &PluginDb::load())?.project;
    p.instantiate_plugins(48000.0);
    let limit = p.bar_ticks() * 4;
    p.playlist.clips.retain(|c| c.start < limit);
    for c in &p.playlist.clips {
        println!("clip {:?} start {} len {} gain {}", c.source, c.start, c.length, c.gain);
    }
    let files = render_stems(&p, out.as_ref(), &ExportOptions { sample_rate: 48000, tail_secs: 0.0, ..Default::default() }, |_| {})?;
    for f in files {
        println!("{:>7.1} dB  {}", rms_db(&f), f.file_name().unwrap().to_string_lossy());
    }
    Ok(())
}
