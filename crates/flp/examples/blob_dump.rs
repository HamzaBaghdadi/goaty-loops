//! Hex dump of one event (by id) of a named channel. Args: file name id
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = FlpFile::read(&a[0]).unwrap();
    let p = FlpProject::from_file(&f);
    let id: u8 = a[2].parse().unwrap();
    let iid = p.channels.iter().find(|c| c.name.as_deref().is_some_and(|n| n.contains(&a[1]))).map(|c| c.iid).unwrap();
    let mut cur = None;
    for e in &f.events {
        if e.id == ids::CH_NEW { cur = e.as_u32().map(|v| v as u16); continue; }
        if cur == Some(iid) && e.id == id {
            let b = e.blob().unwrap();
            for (k, c) in b.chunks(4).enumerate() {
                if c.len() == 4 { print!("{}:{} ", k * 4, i32::from_le_bytes([c[0], c[1], c[2], c[3]])); }
            }
            println!();
        }
    }
}
