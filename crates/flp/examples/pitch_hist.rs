use goaty_flp::{FlpFile, FlpProject};
use std::collections::BTreeMap;
fn main() {
    let mut h: BTreeMap<i32, usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        for c in FlpProject::from_file(&f).channels {
            *h.entry(c.pitch_cents).or_default() += 1;
        }
    }
    let mut v: Vec<_> = h.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("{:?}", &v[..v.len().min(30)]);
}
