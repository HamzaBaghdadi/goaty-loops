//! Hex-dumps the state record (53) of the first wrapper whose name matches.
use goaty_flp::FlpFile;
fn main() {
    let mut a = std::env::args().skip(1);
    let (path, want) = (a.next().unwrap(), a.next().unwrap());
    let f = FlpFile::read(&path).unwrap();
    for e in f.events.iter().filter(|e| e.id == 213) {
        let b = e.blob().unwrap();
        if b.len() < 4 || u32::from_le_bytes(b[0..4].try_into().unwrap()) != 12 { continue; }
        let mut pos = 4; let mut name = String::new(); let mut state = None;
        while pos + 12 <= b.len() {
            let id = u32::from_le_bytes(b[pos..pos+4].try_into().unwrap());
            let size = u64::from_le_bytes(b[pos+4..pos+12].try_into().unwrap()) as usize;
            let d = &b[pos+12..pos+12+size];
            if id == 54 { name = String::from_utf8_lossy(d).to_string(); }
            if id == 53 { state = Some(d.to_vec()); }
            pos += 12 + size;
        }
        if name == want {
            let s = state.unwrap();
            println!("{name}: state {} bytes", s.len());
            for (i, c) in s.chunks(32).enumerate().take(12) {
                let hex: Vec<String> = c.iter().map(|x| format!("{x:02x}")).collect();
                let asc: String = c.iter().map(|&x| if (32..127).contains(&x) { x as char } else { '.' }).collect();
                println!("{:04x}: {} {}", i * 32, hex.join(""), asc);
            }
            walk(&s);
            let tail = &s[s.len().saturating_sub(96)..];
            println!("... tail: {}", tail.iter().map(|x| format!("{x:02x}")).collect::<String>());
            return;
        }
    }
}
#[allow(dead_code)]
pub fn walk(s: &[u8]) {
    let mut pos = 0x50;
    while pos + 12 <= s.len() {
        let id = u32::from_le_bytes(s[pos..pos + 4].try_into().unwrap());
        let size = u64::from_le_bytes(s[pos + 4..pos + 12].try_into().unwrap()) as usize;
        println!("  chunk id {id} size {size} at {pos:#x}");
        if pos + 12 + size > s.len() { println!("  (overruns)"); break; }
        pos += 12 + size;
    }
    println!("  end at {pos:#x} of {:#x}", s.len());
}
