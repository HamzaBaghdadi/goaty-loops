//! Compares Goaty Loops's render of an FLP with a reference WAV rendered by FL
//! Studio: overall level, level over time and per frequency band.
//! Usage: compare_render <song.flp> <fl_render.wav> <out_dir>   (reads the FLP only)
use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;

fn read_mono(path: &std::path::Path) -> (Vec<f32>, u32) {
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    let ch = spec.channels as usize;
    let s: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|v| v.unwrap()).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|v| v.unwrap() as f32 * scale).collect()
        }
    };
    (s.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect(), spec.sample_rate)
}

/// One-pole low-pass at `hz`.
fn lowpass(x: &[f32], hz: f32, sr: f32) -> Vec<f32> {
    let a = (-std::f32::consts::TAU * hz / sr).exp();
    let mut y = 0.0;
    x.iter().map(|&v| { y = (1.0 - a) * v + a * y; y }).collect()
}

fn db(rms: f32) -> f32 { 20.0 * rms.max(1e-9).log10() }

/// RMS per window of `win` seconds.
fn envelope(x: &[f32], sr: u32, win: f32) -> Vec<f32> {
    let n = (sr as f32 * win) as usize;
    x.chunks(n).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect()
}

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (flp, fl_wav, out) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let bc_wav = std::path::Path::new(&out).join("bc_render.wav");
    let mut p = import_flp(flp.as_ref(), &PluginDb::load())?.project;
    p.instantiate_plugins(48000.0);
    let (fl, sr) = read_mono(fl_wav.as_ref());
    render_to_wav(&p, &bc_wav, &ExportOptions { sample_rate: sr, tail_secs: 0.0, ..Default::default() }, |_| {})?;
    let (bc, _) = read_mono(&bc_wav);
    println!("lengths: FL {:.1}s, Goaty Loops {:.1}s (sample rate {sr})", fl.len() as f32 / sr as f32, bc.len() as f32 / sr as f32);

    // Align: find the lag (within +-0.5 s) that best matches the 1 ms energy envelopes.
    let e = |x: &[f32]| envelope(x, sr, 0.001);
    let (ef, eb) = (e(&fl), e(&bc));
    let best = (-500i32..=500).max_by(|&x, &y| {
        let score = |lag: i32| (0..ef.len().min(eb.len()) as i32 - 600).step_by(3).map(|i| { let j = i + lag; if j < 0 { 0.0 } else { ef[i as usize] * eb[j as usize] } }).sum::<f32>();
        score(x).partial_cmp(&score(y)).unwrap()
    }).unwrap();
    println!("Goaty Loops is offset by {best} ms relative to FL");
    let shift = (best as i64 * sr as i64 / 1000) as isize;
    let bc: Vec<f32> = (0..fl.len() as isize).map(|i| bc.get((i + shift).max(0) as usize).copied().unwrap_or(0.0)).collect();

    let n = fl.len().min(bc.len());
    let overall = |x: &[f32]| (x[..n].iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
    println!("overall RMS: FL {:.1} dB, Goaty Loops {:.1} dB (difference {:+.1} dB)", db(overall(&fl)), db(overall(&bc)), db(overall(&bc)) - db(overall(&fl)));
    let corr = { let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64); for i in 0..n { let (x, y) = (fl[i] as f64, bc[i] as f64); xy += x * y; xx += x * x; yy += y * y; } xy / (xx * yy).sqrt() };
    println!("waveform correlation: {corr:.3}");

    // Bands: low < 200 Hz, mid 200-2000 Hz, high > 2000 Hz.
    let bands = |x: &[f32]| { let lo = lowpass(x, 200.0, sr as f32); let lm = lowpass(x, 2000.0, sr as f32); let mid: Vec<f32> = lm.iter().zip(&lo).map(|(a, b)| a - b).collect(); let hi: Vec<f32> = x.iter().zip(&lm).map(|(a, b)| a - b).collect(); [lo, mid, hi] };
    let (bf, bb) = (bands(&fl[..n]), bands(&bc[..n]));
    for (k, name) in ["low (<200 Hz)", "mid", "high (>2 kHz)"].iter().enumerate() {
        println!("  {name:<14} FL {:.1} dB, Goaty Loops {:.1} dB ({:+.1} dB)", db(overall(&bf[k])), db(overall(&bb[k])), db(overall(&bb[k])) - db(overall(&bf[k])));
    }

    // Level over time in 8 s windows.
    let (wf, wb) = (envelope(&fl[..n], sr, 8.0), envelope(&bc[..n], sr, 8.0));
    println!("level per 8 s (FL dB / Goaty Loops dB / diff):");
    for (i, (x, y)) in wf.iter().zip(&wb).enumerate() {
        println!("  {:>3}s  {:>6.1}  {:>6.1}  {:+5.1}", i * 8, db(*x), db(*y), db(*y) - db(*x));
    }
    Ok(())
}
