//! Prints tempo automation points next to the project tempo (read-only).
use goaty_flp::{FlpControlTarget, FlpFile, FlpProject};
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        for c in p.controllers.iter().filter(|c| matches!(c.target, FlpControlTarget::Global { param: 5 })) {
            if let Some(ch) = p.channels.iter().find(|x| x.iid == c.source_channel) {
                let vals: Vec<String> = ch.automation.iter().flatten().map(|pt| format!("{:.4}", pt.value)).collect();
                println!("{:.2} BPM  {}  values {}", p.tempo, path.rsplit(['\\', '/']).next().unwrap(), vals.join(" "));
            }
        }
    }
}
