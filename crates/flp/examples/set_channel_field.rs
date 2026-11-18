//! Writes a copy of an FLP with one channel field changed (for experiments).
//! Args: in.flp out.flp channel-name (vol <raw 0..12800> | ev <id> <value>)
use goaty_flp::{ids, EventValue, FlpFile, FlpProject};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut f = FlpFile::read(&a[0]).unwrap();
    let p = FlpProject::from_file(&f);
    let iid = p.channels.iter().find(|c| c.name.as_deref() == Some(a[2].as_str())).map(|c| c.iid).expect("channel");
    let mut cur = None;
    let mut done = false;
    for e in &mut f.events {
        if e.id == ids::CH_NEW { cur = e.as_u32().map(|v| v as u16); continue; }
        if cur != Some(iid) { continue; }
        match a[3].as_str() {
            "vol" if e.id == ids::CH_LEVELS => {
                let mut b = e.blob().unwrap().to_vec();
                b[4..8].copy_from_slice(&a[4].parse::<i32>().unwrap().to_le_bytes());
                e.value = EventValue::Blob(b);
                done = true;
            }
            "ev" if e.id == a[4].parse::<u8>().unwrap() => {
                let v: u32 = a[5].parse().unwrap();
                e.value = match e.value { EventValue::Byte(_) => EventValue::Byte(v as u8), EventValue::Word(_) => EventValue::Word(v as u16), _ => EventValue::Dword(v) };
                done = true;
            }
            _ => {}
        }
    }
    assert!(done, "field not found");
    std::fs::write(&a[1], f.to_bytes()).unwrap();
    println!("wrote {}", a[1]);
}
