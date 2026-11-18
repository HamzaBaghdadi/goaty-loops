//! Per channel: every integer event id=value, plus named inserts, to find the
//! channel -> mixer insert field. Read-only.
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    let names: Vec<String> = p.inserts.iter().enumerate().filter_map(|(i, s)| s.name.as_ref().map(|n| format!("{i}:{n}"))).collect();
    println!("inserts: {}", names.join(", "));
    let mut ch: Option<u16> = None;
    let mut line = String::new();
    for e in &f.events {
        if e.id == ids::CH_NEW || e.id == ids::ARR_NEW {
            if let Some(c) = ch {
                let name = p.channels.iter().find(|x| x.iid == c).and_then(|x| x.name.clone()).unwrap_or_default();
                println!("{c:>3} {:<16} {line}", name.chars().take(16).collect::<String>());
            }
            line.clear();
            ch = if e.id == ids::CH_NEW { e.as_u32().map(|v| v as u16) } else { None };
            continue;
        }
        if ch.is_some() {
            if let Some(v) = e.as_u32() {
                line += &format!("{}={} ", e.id, v);
            }
        }
    }
}
