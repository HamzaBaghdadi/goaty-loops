use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let mut h: BTreeMap<u8, usize> = BTreeMap::new();
    let mut vol_inserts = Vec::new();
    for e in f.events.iter().filter(|e| e.id == ids::MIXER_PARAMS) {
        for c in e.blob().unwrap().chunks_exact(12) {
            *h.entry(c[4]).or_default() += 1;
            if c[4] == 0xC0 { vol_inserts.push((u16::from_le_bytes([c[6], c[7]]) >> 6) & 0x7f); }
        }
    }
    println!("ids: {:?}", h);
    println!("volume entries for inserts: {:?}", &vol_inserts[..vol_inserts.len().min(20)]);
    println!("first entries: {:?}", f.events.iter().find(|e| e.id == ids::MIXER_PARAMS).map(|e| e.blob().unwrap()[..36].to_vec()));
}
