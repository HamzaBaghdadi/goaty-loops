//! Lists mixer routing (main output and sends with gain in dB) of an FLP. Read-only.
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let f = FlpFile::read(&path).unwrap();
    let p = convert(&FlpProject::from_file(&f), None, &PluginDb::default()).project;
    for (i, t) in p.mixer.iter().enumerate() {
        if t.sends.is_empty() && t.output == 0 {
            continue;
        }
        let sends: Vec<String> = t.sends.iter().map(|s| format!("{} ({:.1} dB)", p.mixer[s.to].name, 20.0 * s.gain.max(1e-9).log10())).collect();
        println!("{i:>3} {:<16} -> {:<12} sends: {}", t.name, p.mixer[t.output].name, sends.join(", "));
    }
}
