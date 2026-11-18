//! Distribution of one channel event id's values by channel type. Args: id files...
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let mut a = std::env::args().skip(1);
    let id: u8 = a.next().unwrap().parse().unwrap();
    let mut m: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for path in a {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let mut ty = 99;
        for e in &f.events {
            match e.id {
                ids::CH_NEW => ty = 99,
                ids::CH_TYPE => ty = e.as_u32().unwrap_or(99),
                x if x == id => *m.entry((ty, e.as_u32().unwrap_or(0))).or_default() += 1,
                _ => {}
            }
        }
    }
    println!("(type, value): count -> {m:?}");
}
