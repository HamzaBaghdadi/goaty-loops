//! Dumps events inside automation channels (type 5) and any project-level
//! events that look like controller links (read-only).
use goaty_flp::{ids, FlpFile};
use std::collections::BTreeMap;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    // Find channel ranges: CH_NEW starts a channel; collect its events until the next CH_NEW / pattern / arrangement.
    let mut cur: Option<(u32, Vec<(u8, usize, Vec<u8>)>)> = None;
    let mut chans = Vec::new();
    for e in &f.events {
        if e.id == ids::CH_NEW {
            if let Some(c) = cur.take() { chans.push(c); }
            cur = Some((e.as_u32().unwrap(), Vec::new()));
            continue;
        }
        if matches!(e.id, ids::ARR_NEW | ids::INS_FLAGS) { if let Some(c) = cur.take() { chans.push(c); } }
        if let Some((_, evs)) = &mut cur {
            let (len, head) = match e.blob() { Some(b) => (b.len(), b.iter().take(48).copied().collect()), None => (4, e.as_u32().unwrap().to_le_bytes().to_vec()) };
            evs.push((e.id, len, head));
        }
    }
    let mut shown = 0;
    for (iid, evs) in &chans {
        let kind = evs.iter().find(|e| e.0 == ids::CH_TYPE).map(|e| e.2[0]);
        if kind != Some(5) || shown >= 2 { continue; }
        shown += 1;
        println!("== automation channel iid {iid}");
        for (id, len, head) in evs {
            let hex: String = head.iter().map(|b| format!("{b:02x}")).collect();
            println!("  ev {id:>3} len {len:>5}  {hex}");
        }
    }
    // Project-wide blob events by id, for spotting controller tables.
    let mut counts: BTreeMap<u8, (usize, usize)> = BTreeMap::new();
    for e in &f.events { if let Some(b) = e.blob() { let c = counts.entry(e.id).or_default(); c.0 += 1; c.1 = c.1.max(b.len()); } }
    println!("blob events (id: count, max len): {:?}", counts);
}
