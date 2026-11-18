//! Per channel: name and the bytes of its parameters blob (event 215), plus
//! the mixer insert names, to locate the channel -> insert routing. Read-only.
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    for (i, ins) in p.inserts.iter().enumerate().filter(|(_, i)| i.name.is_some()) {
        println!("insert {i}: {:?}", ins.name);
    }
    let mut ch = None;
    for e in &f.events {
        match e.id {
            ids::CH_NEW => ch = e.as_u32().map(|v| v as u16),
            215 if ch.is_some() => {
                let b = e.blob().unwrap();
                let name = p.channels.iter().find(|c| Some(c.iid) == ch).and_then(|c| c.name.clone()).unwrap_or_default();
                let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
                println!("{:>3} {:<20} len {} {}", ch.unwrap(), name.chars().take(20).collect::<String>(), b.len(), hex.join(""));
                ch = None;
            }
            ids::ARR_NEW => ch = None,
            _ => {}
        }
    }
}
