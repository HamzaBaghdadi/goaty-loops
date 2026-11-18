//! Event ids of each channel block (CH_NEW .. next CH_NEW) and what follows
//! the last one, with the channel type. Read-only.
use goaty_flp::{FlpFile, ids};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let starts: Vec<usize> = f.events.iter().enumerate().filter(|(_, e)| e.id == ids::CH_NEW).map(|(i, _)| i).collect();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or((s + 120).min(f.events.len()));
        let ty = f.events[s..end].iter().find(|e| e.id == ids::CH_TYPE).and_then(|e| e.as_u32());
        let idsv: Vec<String> = f.events[s..end].iter().map(|e| e.id.to_string()).collect();
        if k < 2 || k + 1 == starts.len() {
            println!("channel {k} (type {ty:?}) events {}: {}", end - s, idsv.join(" "));
        }
    }
}
