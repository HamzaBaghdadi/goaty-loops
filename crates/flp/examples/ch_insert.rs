//! Per channel: name, event 22 and event 155 values (mixer insert candidates). Read-only.
use goaty_flp::{ids, FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = FlpProject::from_file(&f);
    let mut ch = None;
    let mut rows: Vec<(u16, Option<u32>, Option<u32>)> = Vec::new();
    for e in &f.events {
        match e.id {
            ids::CH_NEW => {
                ch = e.as_u32().map(|v| v as u16);
                rows.push((ch.unwrap(), None, None));
            }
            22 if ch.is_some() => rows.last_mut().unwrap().1 = e.as_u32(),
            155 if ch.is_some() => rows.last_mut().unwrap().2 = e.as_u32(),
            ids::ARR_NEW => ch = None,
            _ => {}
        }
    }
    for (iid, e22, e155) in rows {
        let name = p.channels.iter().find(|c| c.iid == iid).and_then(|c| c.name.clone()).unwrap_or_default();
        let parsed = p.channels.iter().find(|c| c.iid == iid).and_then(|c| c.mixer_insert);
        println!("{iid:>3} {name:<24} ev22={e22:?} ev155={e155:?} parsed={parsed:?}");
    }
}
