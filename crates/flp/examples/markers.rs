//! Dumps time-marker candidate events (148 position, 33/34 signature,
//! 205 name) with the arrangement they follow. Read-only.
use goaty_flp::FlpFile;
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        let mut arr = None;
        let mut out = Vec::new();
        for e in &f.events {
            match e.id {
                99 => arr = e.as_u32(),
                148 => out.push(format!("[arr {arr:?}] pos=0x{:08x}", e.as_u32().unwrap())),
                33 | 34 => out.push(format!("  id{} = {}", e.id, e.as_u32().unwrap())),
                205 => {
                    let b = e.blob().unwrap();
                    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                    out.push(format!("  name={:?}", String::from_utf16_lossy(&u)));
                }
                _ => {}
            }
        }
        if out.iter().any(|l| l.contains("pos=")) {
            println!("== {name}");
            for l in out.iter().take(24) {
                println!("{l}");
            }
        }
    }
}
