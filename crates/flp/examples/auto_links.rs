//! Lists automation channels (iid, name) and event-227 controller records (read-only).
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    for path in std::env::args().skip(1) {
        let f = FlpFile::read(&path).unwrap();
        let p = FlpProject::from_file(&f);
        println!("== {}", path.rsplit(['\\', '/']).next().unwrap());
        let names: Vec<String> = p.channels.iter().map(|c| format!("{}:{}{}", c.iid, if c.kind == 5 { "AUTO " } else { "" }, c.name.clone().unwrap_or_default())).collect();
        println!("channels: {}", names.join(" | "));
        println!("inserts named: {:?}", p.inserts.iter().enumerate().filter_map(|(i, x)| x.name.clone().map(|n| format!("{i}:{n}"))).collect::<Vec<_>>());
        for e in f.events.iter().filter(|e| e.id == 227) {
            let b = e.blob().unwrap();
            let w: Vec<String> = b.chunks(2).map(|c| format!("{:5}", u16::from_le_bytes([c[0], c.get(1).copied().unwrap_or(0)]))).collect();
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            println!("  227 {hex}  u16s {}", w.join(" "));
        }
    }
}
