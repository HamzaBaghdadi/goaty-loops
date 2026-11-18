//! Correlates the first seconds of a reference render with a source sample
//! (best lag within 300 ms) and prints the gain that maps one onto the other.
fn load(p: &str) -> (Vec<f32>, u32) {
    let d = goaty_engine::sample::load_audio(p.as_ref()).unwrap();
    (d.samples.chunks(2).map(|f| (f[0] + f[1]) * 0.5).collect(), d.sample_rate)
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (render, sr) = load(&a[0]);
    let (src, sr2) = load(&a[1]);
    assert_eq!(sr, sr2, "sample rates differ");
    let n = (6 * sr) as usize;
    let mut best = (0i64, 0f64);
    for lag in (-(sr as i64) * 3 / 10..(sr as i64) * 3 / 10).step_by(8) {
        let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
        for i in (0..n).step_by(4) {
            let j = i as i64 + lag;
            if j < 0 { continue; }
            let (x, y) = (render[i] as f64, src[j as usize] as f64);
            xy += x * y; xx += x * x; yy += y * y;
        }
        let c = xy / (xx * yy).sqrt();
        if c > best.1 { best = (lag, c); }
    }
    let lag = best.0;
    let (mut xy, mut yy) = (0f64, 0f64);
    for i in 0..n { let j = i as i64 + lag; if j < 0 { continue; } let (x, y) = (render[i] as f64, src[j as usize] as f64); xy += x * y; yy += y * y; }
    let g = xy / yy;
    println!("best lag {:.1} ms, correlation {:.3}, render = sample x {:.3} ({:+.1} dB)", lag as f64 * 1000.0 / sr as f64, best.1, g, 20.0 * g.abs().log10());
}
