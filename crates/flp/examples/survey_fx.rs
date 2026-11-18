//! Counts native FL effects in mixer slots and their parameter blob sizes (read-only).
use goaty_flp::{FlpFile, FlpProject};
use std::collections::BTreeMap;
fn main() {
    let mut count: BTreeMap<String, (usize, BTreeMap<usize, usize>)> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        // Native plugin data events (213) by internal name, in mixer context.
        let p = FlpProject::from_file(&f);
        for ins in &p.inserts {
            for s in &ins.slots {
                if s.wrapper.is_none() {
                    let e = count.entry(s.internal.clone()).or_default();
                    e.0 += 1;
                }
            }
        }
        let mut internal = String::new();
        let mut in_mixer = false;
        for e in &f.events {
            if e.id == 236 { in_mixer = true; }
            if e.id == 201 { internal = e.blob().map(|b| String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).trim_end_matches('\0').to_string()).unwrap_or_default(); }
            if e.id == 213 && in_mixer && internal != "Fruity Wrapper" {
                *count.entry(internal.clone()).or_default().1.entry(e.blob().unwrap().len()).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = count.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (name, (n, sizes)) in v {
        println!("{n:>5}  {name:<28} sizes {:?}", sizes);
    }
}
