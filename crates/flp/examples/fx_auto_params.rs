//! Parameter numbers automated on a named mixer effect, with counts. Read-only.
use goaty_flp::{FlpControlTarget, FlpFile, FlpProject};
use std::collections::BTreeMap;
fn main() {
    let mut args = std::env::args().skip(1);
    let want = args.next().unwrap();
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for path in args {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        for c in &p.controllers {
            if let FlpControlTarget::MixerPlugin { insert, slot, param } = c.target {
                let s = p.inserts.get(insert as usize).and_then(|i| i.slots.iter().find(|s| s.index as u16 == slot));
                if s.is_some_and(|s| s.internal == want) {
                    *counts.entry(param as u32).or_default() += 1;
                }
            }
        }
    }
    println!("{want}: {counts:?}");
}
