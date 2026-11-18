//! Dumps playlist items (event 233) as u32/f32 per 4 bytes, and the channel
//! parameter bytes 92..104 (event 215). Read-only.
use goaty_flp::FlpFile;
fn main() {
    for path in std::env::args().skip(1) {
        let f = FlpFile::read(&path).unwrap();
        println!("== {}", path.rsplit(['/', '\\']).next().unwrap());
        for e in f.events.iter().filter(|e| e.id == 233) {
            let b = e.blob().unwrap();
            for c in b.chunks(80) {
                let words: Vec<String> = c.chunks_exact(4).enumerate().map(|(i, w)| {
                    let u = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                    let fl = f32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                    format!("{}:{u}/{fl:.3}", i * 4)
                }).collect();
                println!("  item {}", words.join(" "));
            }
        }
        for e in f.events.iter().filter(|e| e.id == 215) {
            let b = e.blob().unwrap();
            if b.len() >= 104 {
                let w = &b[96..100];
                println!("  params[96..100] u32 {} f32 {}", u32::from_le_bytes([w[0], w[1], w[2], w[3]]), f32::from_le_bytes([w[0], w[1], w[2], w[3]]));
            }
        }
    }
}
