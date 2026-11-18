//! Per-field statistics of a native effect's parameter blob, read as i32 (read-only).
use goaty_flp::FlpFile;
use std::collections::BTreeMap;
fn main() {
    let mut args = std::env::args().skip(1);
    let want = args.next().unwrap();
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    for path in args {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let mut internal = String::new();
        for e in &f.events {
            if e.id == 201 { internal = e.blob().map(|b| String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).trim_end_matches('\0').to_string()).unwrap_or_default(); }
            if e.id == 213 && internal == want { blobs.push(e.blob().unwrap().to_vec()); }
        }
    }
    let mut by_len: BTreeMap<usize, Vec<&Vec<u8>>> = BTreeMap::new();
    for b in &blobs { by_len.entry(b.len()).or_default().push(b); }
    for (len, group) in by_len {
        println!("== {want}: {} blobs of {len} bytes", group.len());
        let n = len / 4;
        for i in 0..n {
            let vals: Vec<i32> = group.iter().map(|b| i32::from_le_bytes(b[i*4..i*4+4].try_into().unwrap())).collect();
            let mut freq: BTreeMap<i32, usize> = BTreeMap::new();
            for v in &vals { *freq.entry(*v).or_default() += 1; }
            let mut top: Vec<_> = freq.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            let tops: Vec<String> = top.iter().take(4).map(|(v, c)| format!("{v}x{c}")).collect();
            println!("  [{i:>3}] min {:>11} max {:>11} distinct {:>4}  top {}", vals.iter().min().unwrap(), vals.iter().max().unwrap(), freq.len(), tops.join(" "));
        }
        if len % 4 != 0 { println!("  (+{} trailing bytes)", len % 4); }
    }
}
