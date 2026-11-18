//! Runs every mixer effect of an FLP on noise and reports non-finite output.
use goaty_engine::effects::FxState;
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::{PluginDb, TimeInfo};
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let r = import_flp(path.as_ref(), &PluginDb::default())?;
    let time = TimeInfo { sample_rate: 44100.0, tempo: r.project.tempo, playing: true, ..Default::default() };
    let mut seed = 1u32;
    for t in &r.project.mixer {
        for s in &t.effects {
            let mut st = FxState::new(&s.effect, 44100.0);
            let mut bad_at = None;
            let mut peak = 0f32;
            for block in 0..(44100 * 20 / 512) {
                let mut l = vec![0f32; 512];
                let mut rr = vec![0f32; 512];
                for i in 0..512 {
                    seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
                    let v = (seed as f32 / u32::MAX as f32 - 0.5) * if block % 40 < 20 { 0.8 } else { 0.0 };
                    l[i] = v; rr[i] = -v * 0.5;
                }
                st.process(&s.effect, &mut l, &mut rr, 44100.0, &time);
                if l.iter().chain(&rr).any(|x| !x.is_finite()) { bad_at = Some(block); break; }
                peak = peak.max(l.iter().chain(&rr).fold(0f32, |m, x| m.max(x.abs())));
            }
            if bad_at.is_some() || peak > 8.0 {
                println!("{} on {}: non-finite at block {:?}, peak {peak:.1}\n   {:?}", s.effect.name(), t.name, bad_at, s.effect);
            }
        }
    }
    println!("done");
    Ok(())
}
