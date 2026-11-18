//! Prints event ids (with small values) around the first occurrence of an id.
use goaty_flp::FlpFile;
fn main() {
    let mut a = std::env::args().skip(1);
    let (path, id): (String, u8) = (a.next().unwrap(), a.next().unwrap().parse().unwrap());
    let f = FlpFile::read(&path).unwrap();
    let first = f.events.iter().position(|e| e.id == id).unwrap_or(0);
    let last = f.events.iter().rposition(|e| e.id == id).unwrap_or(0);
    for e in &f.events[first.saturating_sub(6)..(last + 8).min(f.events.len())] {
        match e.as_u32() {
            Some(v) => println!("{} = {v:#x}", e.id),
            None => println!("{} blob {} bytes", e.id, e.blob().map(|b| b.len()).unwrap_or(0)),
        }
    }
}
