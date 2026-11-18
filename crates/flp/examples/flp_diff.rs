//! Event-level diff of two FLPs (read-only): which events changed, with the
//! channel/insert context and the differing byte offsets of blobs.
use goaty_flp::{ids, Event, FlpFile};

fn context(events: &[Event]) -> Vec<String> {
    let mut ctx = String::from("-");
    events
        .iter()
        .map(|e| {
            match e.id {
                ids::CH_NEW => ctx = format!("channel {}", e.as_u32().unwrap_or(0)),
                ids::PAT_NEW => ctx = format!("pattern {}", e.as_u32().unwrap_or(0)),
                ids::ARR_NEW => ctx = format!("arrangement {}", e.as_u32().unwrap_or(0)),
                ids::INS_FLAGS => ctx = "mixer".into(),
                _ => {}
            }
            ctx.clone()
        })
        .collect()
}

fn main() {
    let mut a = std::env::args().skip(1);
    let (pa, pb) = (a.next().unwrap(), a.next().unwrap());
    let (fa, fb) = (FlpFile::read(&pa).unwrap(), FlpFile::read(&pb).unwrap());
    println!("events: {} -> {}", fa.events.len(), fb.events.len());
    if fa.events.len() != fb.events.len() {
        // Report the first divergence only; insertions shift everything after.
        let i = fa.events.iter().zip(&fb.events).position(|(x, y)| x != y).unwrap_or(fa.events.len().min(fb.events.len()));
        let ca = context(&fa.events);
        println!("first difference at event {i} ({}): {:?} vs {:?}", ca.get(i).cloned().unwrap_or_default(), fa.events.get(i).map(|e| e.id), fb.events.get(i).map(|e| e.id));
        return;
    }
    let ctx = context(&fa.events);
    for (i, (x, y)) in fa.events.iter().zip(&fb.events).enumerate() {
        if x == y {
            continue;
        }
        match (x.blob(), y.blob()) {
            (Some(bx), Some(by)) => {
                let offs: Vec<String> = (0..bx.len().max(by.len()))
                    .filter(|&k| bx.get(k) != by.get(k))
                    .take(12)
                    .map(|k| format!("@{k}:{:?}->{:?}", bx.get(k), by.get(k)))
                    .collect();
                println!("{i:>6} [{}] id {} blob {}->{} bytes: {}", ctx[i], x.id, bx.len(), by.len(), offs.join(" "));
            }
            _ => println!("{i:>6} [{}] id {}: {:?} -> {:?}", ctx[i], x.id, x.as_u32(), y.as_u32()),
        }
    }
}
