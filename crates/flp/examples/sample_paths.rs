//! Prints each channel's stored sample path (read-only).
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let p = FlpProject::from_file(&FlpFile::read(std::env::args().nth(1).unwrap()).unwrap());
    for c in &p.channels {
        if let Some(s) = &c.sample_path {
            println!("{:<30} {}", c.name.clone().unwrap_or_default().chars().take(30).collect::<String>(), s);
        }
    }
}
