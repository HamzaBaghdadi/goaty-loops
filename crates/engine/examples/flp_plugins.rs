//! Imports FLPs (read-only), instantiates mapped VST3 plugins and reports
//! whether FL's saved plugin state was accepted.
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use goaty_engine::{Effect, Instrument};

fn main() -> anyhow::Result<()> {
    let db = PluginDb::load();
    for path in std::env::args().skip(1) {
        let mut r = import_flp(path.as_ref(), &db)?;
        println!("== {path}");
        let mut found = Vec::new();
        for c in &r.project.channels {
            if let Instrument::Plugin(p) = &c.instrument {
                found.push(format!("channel {} -> {} (state {} B)", c.name, p.info.name, p.state.len()));
            }
        }
        for t in &r.project.mixer {
            for s in &t.effects {
                if let Effect::Plugin(p) = &s.effect {
                    found.push(format!("mixer {} -> {} (state {} B)", t.name, p.info.name, p.state.len()));
                }
            }
        }
        for f in &found {
            println!("  {f}");
        }
        let errors = r.project.instantiate_plugins(48000.0);
        let loaded = r.project.plugins_mut().iter().filter(|p| p.instance.is_some()).count();
        println!("  loaded {loaded}/{} plugins", found.len());
        for e in errors {
            println!("  ! {e}");
        }
        let vst_warn: Vec<_> = r.warnings.iter().filter(|w| w.contains("Plugin") || w.contains("FL effect")).collect();
        println!("  {} other warnings, e.g. {:?}", vst_warn.len(), vst_warn.iter().take(3).collect::<Vec<_>>());
    }
    Ok(())
}
