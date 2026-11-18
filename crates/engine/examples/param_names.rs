//! Prints parameter names and MIDI controller support (mod wheel, sustain,
//! pitch bend) of cached VST3 instruments.
use goaty_engine::plugin::{PluginDb, PluginInstance};

fn main() {
    let db = PluginDb::load();
    for info in db.plugins.iter().filter(|p| p.vendor != "Antares" && p.instrument) {
        let Ok(inst) = PluginInstance::new(info, 48000.0) else { continue };
        let names: Vec<String> = (0..3).filter_map(|i| inst.param_name(i)).collect();
        let cc = |c: u8| if inst.midi_controller(c, 0.5) { "yes" } else { "no" };
        println!("{:<24} modwheel={} sustain={} pitchbend={} params={names:?}", info.name, cc(1), cc(64), cc(129));
    }
}
