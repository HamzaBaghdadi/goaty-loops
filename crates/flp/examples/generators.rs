//! Counts native generator plugins (channel internal names) and blob sizes. Read-only.
use goaty_flp::{FlpFile, FlpProject};
use std::collections::BTreeMap;
fn main() {
    let mut counts: BTreeMap<String, (usize, BTreeMap<usize, usize>)> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        for c in &p.channels {
            if let Some(pl) = &c.plugin {
                if c.wrapper.is_some() || pl.is_empty() {
                    continue;
                }
                let e = counts.entry(pl.clone()).or_default();
                e.0 += 1;
                *e.1.entry(c.plugin_params.len()).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (k, (n, sizes)) in v.iter().take(20) {
        println!("{n:>5} {k}  sizes {sizes:?}");
    }
}
