use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    println!("version {:?}, inserts parsed {}", p.version, p.inserts.len());
    for (i, ins) in p.inserts.iter().enumerate().take(20) {
        println!("  ins {i}: {:?} slots {}", ins.name, ins.slots.len());
    }
    for e in f.events.iter().filter(|e| e.id == ids::MIXER_PARAMS) {
        for c in e.blob().unwrap().chunks_exact(12) {
            if c[4] == 0xC0 || c[4] == 0xC1 {
                let cd = u16::from_le_bytes([c[6], c[7]]);
                println!("  id {:#x} b5 {:#x} cd {:#06x} msg {}", c[4], c[5], cd, i32::from_le_bytes(c[8..12].try_into().unwrap()));
            }
        }
    }
}
