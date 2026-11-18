//! For every Fruity Limiter: whether its insert receives a level-0 route (a
//! sidechain) and the values of its first fields, to find the sidechain
//! selector. Read-only.
use goaty_flp::{FlpFile, FlpProject};
use std::collections::BTreeMap;
fn main() {
    // field index -> (values on sidechained inserts, values elsewhere)
    let mut stats: BTreeMap<usize, (BTreeMap<i32, usize>, BTreeMap<i32, usize>)> = BTreeMap::new();
    let (mut sc, mut plain) = (0, 0);
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        // Inserts that receive a route at level 0 from another insert.
        let mut fed: Vec<bool> = vec![false; p.inserts.len()];
        for ins in &p.inserts {
            for &d in &ins.routes {
                let level = ins.route_levels.iter().find(|(to, _)| *to == d).map(|(_, v)| *v);
                if level == Some(0) && d < fed.len() {
                    fed[d] = true;
                }
            }
        }
        for (i, ins) in p.inserts.iter().enumerate() {
            for s in ins.slots.iter().filter(|s| s.internal == "Fruity Limiter") {
                let fields: Vec<i32> = s.params.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                if fed[i] { sc += 1 } else { plain += 1 }
                for (k, v) in fields.iter().enumerate().skip(36).take(8) {
                    let e = stats.entry(k).or_default();
                    *if fed[i] { &mut e.0 } else { &mut e.1 }.entry(*v).or_default() += 1;
                }
            }
        }
    }
    println!("limiters on sidechained inserts: {sc}, others: {plain}");
    for (k, (a, b)) in stats {
        let top = |m: &BTreeMap<i32, usize>| { let mut v: Vec<_> = m.iter().collect(); v.sort_by(|x, y| y.1.cmp(x.1)); v.iter().take(5).map(|(k, c)| format!("{k}x{c}")).collect::<Vec<_>>().join(" ") };
        println!("[{k:>2}] sidechained: {:<40} others: {}", top(&a), top(&b));
    }
}
