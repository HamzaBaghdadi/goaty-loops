//! Mixer parameter entries with ids 64..=191 (route levels) per insert. Read-only.
use goaty_flp::{ids, mixer_location, FlpFile};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    for e in f.events.iter().filter(|e| e.id == ids::MIXER_PARAMS) {
        for c in e.blob().unwrap().chunks_exact(12) {
            let id = c[4];
            let loc = u16::from_le_bytes([c[6], c[7]]);
            let msg = i32::from_le_bytes([c[8], c[9], c[10], c[11]]);
            if (64..192).contains(&id) {
                println!("id {id} (dest {}) at {:?} = {msg}", id - 64, mixer_location(loc));
            }
        }
    }
}
