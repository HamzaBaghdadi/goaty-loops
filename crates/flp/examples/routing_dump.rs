//! Prints each mixer insert's routing event (DATA+27) as the list of
//! destination indices with non-zero bytes, with the insert name. Read-only.
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    let mut insert = 0usize;
    for e in &f.events {
        match e.id {
            ids::INS_ROUTING => {
                let b = e.blob().unwrap();
                let targets: Vec<String> = b.iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, v)| format!("{i}={v}")).collect();
                let name = p.inserts.get(insert).and_then(|i| i.name.clone()).unwrap_or_default();
                if insert < 30 {
                    println!("insert {insert:>3} {name:<16} len {} -> [{}]", b.len(), targets.join(" "));
                }
            }
            ids::INS_OUTPUT => insert += 1,
            _ => {}
        }
    }
}
