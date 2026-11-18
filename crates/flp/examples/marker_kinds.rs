//! Lists time markers whose action byte is non-zero (read-only).
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        for (a, m) in p.arrangements.iter().flat_map(|a| a.markers.iter().map(move |m| (a.iid, m))).chain(p.legacy_markers.iter().map(|m| (999, m))) {
            if m.kind != 0 {
                println!("kind {:>2} arr {a:>3} pos {:>6} sig {}/{} name {:?}  [{name}]", m.kind, m.position, m.numerator, m.denominator, m.name.as_deref().unwrap_or(""));
            }
        }
    }
}
