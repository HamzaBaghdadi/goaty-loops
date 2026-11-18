//! Lists mixer inserts with their effect slots and routes. Read-only.
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let p = FlpProject::from_file(&FlpFile::read(&path).unwrap());
    for (i, ins) in p.inserts.iter().enumerate() {
        let slots: Vec<String> = ins.slots.iter().map(|s| format!("{}:{}", s.index, if s.internal.is_empty() { s.name.clone() } else { s.internal.clone() })).collect();
        if !slots.is_empty() || i < 3 {
            println!("{i:>3} {:<14} routes {:?} slots {}", ins.name.clone().unwrap_or_default(), ins.routes, slots.join(", "));
        }
    }
}
