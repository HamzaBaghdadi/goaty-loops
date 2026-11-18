//! Dumps FL's plugin wrapper data events (read-only).
//!
//! cargo run -p goaty-flp --example dump_wrapper -- song.flp

use goaty_flp::{ids, FlpFile};

fn main() {
    for path in std::env::args().skip(1) {
        let f = FlpFile::read(&path).expect("read");
        println!("== {path}");
        let mut internal = String::new();
        for e in &f.events {
            if e.id == ids::PLUGIN_INTERNAL_NAME {
                internal = e.blob().map(|b| String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())).unwrap_or_default();
            }
            if e.id == 213 {
                let b = e.blob().unwrap();
                let head: Vec<String> = b.iter().take(24).map(|x| format!("{x:02x}")).collect();
                println!("  [{}] 213 len {} head {}", internal.trim_end_matches('\0'), b.len(), head.join(" "));
                if b.len() >= 4 {
                    let kind = u32::from_le_bytes(b[0..4].try_into().unwrap());
                    let mut pos = 4;
                    let mut recs = Vec::new();
                    while pos + 12 <= b.len() {
                        let id = u32::from_le_bytes(b[pos..pos + 4].try_into().unwrap());
                        let size = u64::from_le_bytes(b[pos + 4..pos + 12].try_into().unwrap()) as usize;
                        if pos + 12 + size > b.len() {
                            recs.push(format!("BAD(id {id} size {size})"));
                            break;
                        }
                        let data = &b[pos + 12..pos + 12 + size];
                        let preview = if matches!(id, 54 | 55 | 56) {
                            format!("{:?}", String::from_utf8_lossy(data))
                        } else {
                            data.iter().take(16).map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
                        };
                        recs.push(format!("{id}:{size}={preview}"));
                        pos += 12 + size;
                    }
                    println!("      type {kind} records: {}", recs.join(" | "));
                }
            }
        }
    }
}
