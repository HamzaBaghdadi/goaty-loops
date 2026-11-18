//! Value distribution of channel event 70 and byte 20 of event 212, per
//! channel type, to identify the sampler reverse flag. Read-only.
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let mut e70: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut b20: BTreeMap<(u32, u8), usize> = BTreeMap::new();
    let mut pairs: BTreeMap<(u32, u8), usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let (mut ty, mut v70, mut v20) = (u32::MAX, None, None);
        let mut flush = |ty: u32, v70: Option<u32>, v20: Option<u8>| {
            if let (Some(a), Some(b)) = (v70, v20) {
                *pairs.entry((a, b)).or_default() += 1;
            }
            if let Some(a) = v70 { *e70.entry((ty, a)).or_default() += 1; }
            if let Some(b) = v20 { *b20.entry((ty, b)).or_default() += 1; }
        };
        for e in &f.events {
            match e.id {
                ids::CH_NEW => {
                    flush(ty, v70.take(), v20.take());
                    ty = u32::MAX;
                }
                ids::CH_TYPE => ty = e.as_u32().unwrap_or(99),
                70 => v70 = e.as_u32(),
                212 => v20 = e.blob().and_then(|b| b.get(20).copied()),
                ids::ARR_NEW => flush(ty, v70.take(), v20.take()),
                _ => {}
            }
        }
        flush(ty, v70, v20);
    }
    println!("event 70 by (type, value): {e70:?}");
    println!("event 212[20] by (type, value): {b20:?}");
    println!("pairs (ev70, 212[20]): {pairs:?}");
}
