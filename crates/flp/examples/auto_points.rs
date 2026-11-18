//! Prints automation points (event 234) and candidate controller-link events.
use goaty_flp::{ids, FlpFile};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let mut iid = 0;
    let mut n = 0;
    for e in &f.events {
        if e.id == ids::CH_NEW { iid = e.as_u32().unwrap(); }
        if e.id == 234 && n < 3 {
            n += 1;
            let b = e.blob().unwrap();
            let count = u32::from_le_bytes(b[17..21].try_into().unwrap()) as usize;
            println!("channel {iid}: {count} points, len {}", b.len());
            for k in 0..count {
                let p = &b[21 + k * 24..21 + k * 24 + 24];
                let pos = f64::from_le_bytes(p[0..8].try_into().unwrap());
                let val = f64::from_le_bytes(p[8..16].try_into().unwrap());
                let ten = f32::from_le_bytes(p[16..20].try_into().unwrap());
                let rest: String = p[20..24].iter().map(|x| format!("{x:02x}")).collect();
                println!("   pos {pos:>10.4} val {val:.4} tension {ten:.3} {rest}");
            }
            let tail: String = b[21 + count * 24..].iter().map(|x| format!("{x:02x}")).collect();
            println!("   tail {tail}");
        }
    }
    // Candidate link events: project-level blobs whose content mentions small ints.
    for id in [227u8, 211, 225, 222, 216, 214, 210] {
        let evs: Vec<_> = f.events.iter().filter(|e| e.id == id).collect();
        if let Some(e) = evs.first() {
            let b = e.blob().unwrap();
            println!("event {id}: {} occurrences, first len {} = {}", evs.len(), b.len(), b.iter().take(40).map(|x| format!("{x:02x}")).collect::<String>());
        }
    }
}
