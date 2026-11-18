//! All mixer parameter entries (id, slot, value) of one insert. Read-only.
use goaty_flp::{ids, mixer_location, FlpFile};
fn main() {
    let mut a = std::env::args().skip(1);
    let (path, want): (String, usize) = (a.next().unwrap(), a.next().unwrap().parse().unwrap());
    let f = FlpFile::read(&path).unwrap();
    for e in f.events.iter().filter(|e| e.id == ids::MIXER_PARAMS) {
        for c in e.blob().unwrap().chunks_exact(12) {
            let loc = u16::from_le_bytes([c[6], c[7]]);
            if let Some((insert, slot)) = mixer_location(loc) {
                if insert == want {
                    println!("id {:>3} slot {slot:>2} raw {:02x?} value {}", c[4], &c[0..8], i32::from_le_bytes([c[8], c[9], c[10], c[11]]));
                }
            }
        }
    }
}
