//! Field stats of channel envelope events (DATA+10), by envelope slot, for enabled ones.
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let mut slots: BTreeMap<usize, Vec<Vec<i32>>> = BTreeMap::new();
    let mut cuts: BTreeMap<(u16, u16), usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let mut k = 0;
        for e in &f.events {
            if e.id == ids::CH_NEW { k = 0; }
            if e.id == 218 {
                let b = e.blob().unwrap();
                let v: Vec<i32> = b.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
                slots.entry(k).or_default().push(v);
                k += 1;
            }
            if e.id == 132 {
                let v = e.as_u32().unwrap();
                *cuts.entry(((v & 0xffff) as u16, (v >> 16) as u16)).or_default() += 1;
            }
        }
    }
    for (k, rows) in &slots {
        let enabled: Vec<&Vec<i32>> = rows.iter().collect();
        println!("== envelope slot {k}: {} total, {} enabled", rows.len(), enabled.len());
        if enabled.is_empty() { continue; }
        for i in 0..enabled[0].len() {
            let vals: Vec<i32> = enabled.iter().map(|r| r[i]).collect();
            let mut fr: BTreeMap<i32, usize> = BTreeMap::new();
            for v in &vals { *fr.entry(*v).or_default() += 1; }
            let mut top: Vec<_> = fr.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            println!("  [{i:>2}] min {:>7} max {:>7} top {:?}", vals.iter().min().unwrap(), vals.iter().max().unwrap(), &top[..top.len().min(5)]);
        }
    }
    let mut c: Vec<_> = cuts.into_iter().collect();
    c.sort_by(|a, b| b.1.cmp(&a.1));
    println!("cut/cutby: {:?}", &c[..c.len().min(12)]);
}
