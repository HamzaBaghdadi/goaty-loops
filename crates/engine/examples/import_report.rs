//! Prints Goaty Loops's import report for an FLP (read-only).
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let mut r = import_flp(path.as_ref(), &PluginDb::load())?;
    let p = &mut r.project;
    let kinds = p.channels.iter().fold(std::collections::BTreeMap::<String, usize>::new(), |mut m, c| {
        *m.entry(c.instrument.kind_name().to_string()).or_default() += 1;
        m
    });
    println!("channels {} {kinds:?}, patterns {}, clips {}, markers {}, plugins {}", p.channels.len(), p.patterns.len(), p.playlist.clips.len(), p.playlist.markers.len(), p.plugins_mut().len());
    let routed = p.channels.iter().filter(|c| c.mixer_track > 0).count();
    println!("channels routed to an insert: {routed}");
    for w in &r.warnings {
        println!("- {w}");
    }
    Ok(())
}
