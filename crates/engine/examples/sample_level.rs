//! RMS (dB) of the first N seconds of audio files, via Goaty Loops's loader (read-only).
fn main() {
    let mut a = std::env::args().skip(1);
    let secs: f64 = a.next().unwrap().parse().unwrap();
    for path in a {
        let d = goaty_engine::sample::load_audio(path.as_ref()).unwrap();
        let n = ((secs * d.sample_rate as f64) as usize * 2).min(d.samples.len());
        let rms = (d.samples[..n].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
        let peak = d.samples[..n].iter().fold(0f32, |m, v| m.max(v.abs()));
        println!("{:>7.1} dB RMS, peak {:.2}  {}", 20.0 * rms.max(1e-9).log10(), peak, path.rsplit(['/', '\\']).next().unwrap());
    }
}
