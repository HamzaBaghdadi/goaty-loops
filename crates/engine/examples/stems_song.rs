//! Renders stems of the first 16 bars of an FLP into a folder and prints
//! each stem's peak (reads the FLP only).
use goaty_engine::export::{render_stems, ExportOptions};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (path, out) = (a.next().unwrap(), a.next().unwrap());
    let mut p = import_flp(path.as_ref(), &PluginDb::load())?.project;
    p.instantiate_plugins(48000.0);
    let limit = p.bar_ticks() * 16;
    p.playlist.clips.retain(|c| c.start < limit);
    let t = std::time::Instant::now();
    let files = render_stems(&p, out.as_ref(), &ExportOptions { sample_rate: 48000, tail_secs: 0.5, ..Default::default() }, |_| {})?;
    println!("{} files in {:.1}s", files.len(), t.elapsed().as_secs_f64());
    for f in &files {
        let peak = hound::WavReader::open(f)?.samples::<f32>().map(|s| s.unwrap().abs()).fold(0f32, f32::max);
        println!("  {:<40} peak {peak:.2}", f.file_name().unwrap().to_string_lossy());
    }
    Ok(())
}
