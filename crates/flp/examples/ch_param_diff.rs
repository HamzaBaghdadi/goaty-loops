//! Compares the channel parameter blob (event 215) and FX flags (event 70) of
//! a channel in two FLPs (read-only). Args: a.flp nameA b.flp nameB
use goaty_flp::{ids, FlpFile, FlpProject};
fn find(path: &str, name: &str) -> (Vec<u8>, Option<u32>, Vec<(u8, String)>) {
    let f = FlpFile::read(path).unwrap();
    let p = FlpProject::from_file(&f);
    let iid = p.channels.iter().find(|c| c.name.as_deref().is_some_and(|n| n.contains(name))).map(|c| c.iid).unwrap();
    let (mut cur, mut params, mut fx, mut ints) = (None, Vec::new(), None, Vec::new());
    for e in &f.events {
        if e.id == ids::CH_NEW { cur = e.as_u32().map(|v| v as u16); continue; }
        if e.id == ids::ARR_NEW { cur = None; }
        if cur != Some(iid) { continue; }
        match e.id {
            215 => params = e.blob().unwrap().to_vec(),
            70 => fx = e.as_u32(),
            _ => if let Some(v) = e.as_u32() { ints.push((e.id, v.to_string())); } else { ints.push((e.id, format!("blob {}", e.blob().map(|b| b.len()).unwrap_or(0)))); },
        }
    }
    (params, fx, ints)
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (pa, fa, ia) = find(&a[0], &a[1]);
    let (pb, fb, ib) = find(&a[2], &a[3]);
    println!("fx flags: {fa:?} vs {fb:?}");
    for (k, (x, y)) in pa.chunks(4).zip(pb.chunks(4)).enumerate() {
        if x != y {
            let v = |c: &[u8]| if c.len() == 4 { format!("{} / {:.4}", i32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[0], c[1], c[2], c[3]])) } else { format!("{c:?}") };
            println!("params @{:>3}: {:<28} vs {}", k * 4, v(x), v(y));
        }
    }
    println!("other events A: {ia:?}");
    println!("other events B: {ib:?}");
}
