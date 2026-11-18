//! Times an offline render of an FLP (read-only) to measure real-time load.
use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let r = import_flp(path.as_ref(), &PluginDb::load())?;
    let mut p = r.project;
    for (t, track) in p.mixer.iter().enumerate() {
        for e in &track.effects {
            if let goaty_engine::Effect::Plugin(pp) = &e.effect {
                if let Some(i) = pp.instance.as_ref().filter(|i| i.latency > 0) {
                    println!("latency: insert {t} {} = {} samples", pp.info.name, i.latency);
                }
            }
        }
    }
    // Render the first 16 bars.
    let limit = p.bar_ticks() * 16;
    if std::env::var("BENCH_FULL").is_err() {
        p.playlist.clips.retain(|c| c.start < limit);
    }
    let secs = p.song_length() as f64 * 60.0 / (p.tempo * goaty_engine::PPQ as f64);
    let out = std::env::temp_dir().join("bc_bench.wav");
    let t = std::time::Instant::now();
    render_to_wav(&p, &out, &ExportOptions { sample_rate: 48000, tail_secs: 0.0, ..Default::default() }, |_| {})?;
    let el = t.elapsed().as_secs_f64();
    println!("rendered {secs:.1}s of audio in {el:.2}s -> {:.0}% of real time", el / secs * 100.0);
    let mut rd = hound::WavReader::open(&out)?;
    let s: Vec<f32> = rd.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
    let seg = s.len() / 8;
    let parts: Vec<String> = s.chunks(seg.max(1)).take(8).map(|c| { let rms = (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt(); let pk = c.iter().fold(0f32, |m, x| m.max(x.abs())); format!("{:.2}/{:.2}", rms, pk) }).collect();
    println!("rms/peak per eighth: {}", parts.join("  "));
    Ok(())
}

#[allow(dead_code)]
fn unused() {}
