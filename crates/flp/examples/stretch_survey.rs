//! Values of the channel parameter blob (event 215) at bytes 96..100 as f32,
//! per channel type, to validate the sampler "time" (stretch) field. Read-only.
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let mut zero: BTreeMap<u32, usize> = BTreeMap::new();
    let mut nonzero = Vec::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let mut ty = 99;
        for e in &f.events {
            match e.id {
                ids::CH_TYPE => ty = e.as_u32().unwrap_or(99),
                215 => {
                    let Some(b) = e.blob().filter(|b| b.len() >= 100) else { continue };
                    let v = f32::from_le_bytes([b[96], b[97], b[98], b[99]]);
                    if v == 0.0 {
                        *zero.entry(ty).or_default() += 1;
                    } else {
                        nonzero.push(format!("type {ty}: {v} ({} beats)", v / 192.0));
                    }
                }
                _ => {}
            }
        }
    }
    println!("zero by type: {zero:?}");
    println!("non-zero: {}", nonzero.len());
    for n in nonzero.iter().take(25) {
        println!("  {n}");
    }
}
