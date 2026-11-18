//! Histogram of mixer slot params (id 0 = enabled, id 1 = mix) (read-only).
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let mut h: BTreeMap<(u8, i32), usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        for e in f.events.iter().filter(|e| e.id == ids::MIXER_PARAMS) {
            for c in e.blob().unwrap().chunks_exact(12) {
                let id = c[4];
                if id <= 1 {
                    *h.entry((id, i32::from_le_bytes(c[8..12].try_into().unwrap()))).or_default() += 1;
                }
            }
        }
    }
    for id in 0..=1u8 {
        let mut v: Vec<_> = h.iter().filter(|((i, _), _)| *i == id).map(|((_, val), n)| (*val, *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        println!("id {id}: {} distinct, top {:?}, min {:?} max {:?}", v.len(), &v[..v.len().min(8)], v.iter().map(|x| x.0).min(), v.iter().map(|x| x.0).max());
    }
}
