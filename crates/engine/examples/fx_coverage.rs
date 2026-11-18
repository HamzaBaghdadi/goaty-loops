//! Counts reproduced vs. missing mixer effects across FLPs (read-only).
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use std::collections::BTreeMap;
fn main() {
    let db = PluginDb::load();
    let (mut ok, mut missing) = (BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new());
    let (mut auto_ok, mut auto_total) = (0, 0);
    for path in std::env::args().skip(1) {
        let Ok(r) = import_flp(path.as_ref(), &db) else { continue };
        for c in &r.project.channels {
            if let goaty_engine::Instrument::Automation(a) = &c.instrument {
                auto_total += 1;
                if let Some(t) = a.target { auto_ok += 1; *ok.entry(format!("auto:{:?}", std::mem::discriminant(&t))).or_default() += 0; }
            }
        }
        for t in &r.project.mixer {
            for s in &t.effects {
                *ok.entry(s.effect.name()).or_default() += 1;
            }
        }
        for w in &r.warnings {
            if let Some(rest) = w.strip_prefix("FL effect \"") {
                *missing.entry(rest.split('"').next().unwrap_or("").to_string()).or_default() += 1;
            }
        }
    }
    let total_ok: usize = ok.values().sum();
    println!("automation clips: {auto_ok} mapped, {auto_total} total");
    let total_missing: usize = missing.values().sum();
    println!("reproduced {total_ok}, missing {total_missing} ({:.0}% coverage)", total_ok as f32 / (total_ok + total_missing) as f32 * 100.0);
    let mut v: Vec<_> = ok.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("reproduced: {:?}", &v[..v.len().min(14)]);
    let mut m: Vec<_> = missing.into_iter().collect();
    m.sort_by(|a, b| b.1.cmp(&a.1));
    println!("top missing: {:?}", &m[..m.len().min(12)]);
}
