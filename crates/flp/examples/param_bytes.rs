//! Prints bytes 72..128 of the channel parameter blob (event 215) for
//! channels whose name contains a filter string. Read-only.
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let mut a = std::env::args().skip(1);
    let (path, want) = (a.next().unwrap(), a.next().unwrap_or_default());
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    let mut ch = None;
    for e in &f.events {
        match e.id {
            ids::CH_NEW => ch = e.as_u32().map(|v| v as u16),
            215 => {
                let name = p.channels.iter().find(|c| Some(c.iid) == ch).and_then(|c| c.name.clone()).unwrap_or_default();
                if name.contains(&want) {
                    let b = e.blob().unwrap();
                    let words: Vec<String> = b[72..128.min(b.len())].chunks_exact(4).enumerate().map(|(i, w)| format!("{}:{}", 72 + i * 4, i32::from_le_bytes([w[0], w[1], w[2], w[3]]))).collect();
                    println!("{name:<20} {}", words.join(" "));
                }
            }
            _ => {}
        }
    }
}
