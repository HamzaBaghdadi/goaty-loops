//! Scans a VST3 bundle, instantiates its first class, plays a note and
//! round-trips the state.
//!
//! cargo run -p goaty-engine --example vst3_probe -- "C:\Program Files\Common Files\VST3\Vital.vst3"

use goaty_engine::plugin::{scan_module, NoteEvent, PluginInstance, TimeInfo};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("path to .vst3");
    let infos = scan_module(path.as_ref())?;
    for i in &infos {
        println!("class {} | {} | {} | instrument={} | {}", i.class_id, i.name, i.vendor, i.instrument, i.sub_categories);
    }
    let info = infos.first().ok_or_else(|| anyhow::anyhow!("no classes"))?;
    let inst = PluginInstance::new(info, 48000.0)?;
    println!("instantiated {} (editor: {}, latency {})", info.name, inst.has_editor(), inst.latency);

    let time = TimeInfo { sample_rate: 48000.0, tempo: 120.0, playing: true, ppq: 0.0, project_samples: 0, time_sig: (4, 4) };
    let mut peak_in = 0f32;
    let mut peak = 0f32;
    let (mut l, mut r) = (vec![0f32; 64], vec![0f32; 64]);
    for block in 0..750 {
        // Feed a test tone to effects; instruments ignore the input.
        for i in 0..64 {
            let t = (block * 64 + i) as f32 / 48000.0;
            l[i] = (t * 440.0 * std::f32::consts::TAU).sin() * 0.3;
            r[i] = l[i];
            peak_in = peak_in.max(l[i].abs());
        }
        let ev: Vec<NoteEvent> = match block {
            10 => vec![NoteEvent::On { offset: 3, key: 60, velocity: 0.8 }],
            500 => vec![NoteEvent::Off { offset: 0, key: 60 }],
            _ => vec![],
        };
        let ok = inst.process(64, &ev, &mut l, &mut r, &time);
        assert!(ok);
        peak = peak.max(l.iter().chain(r.iter()).fold(0f32, |m, v| m.max(v.abs())));
    }
    println!("output peak over 1 s: {peak:.3} (input peak {peak_in:.3})");

    let state = inst.get_state();
    println!("state: {} bytes", state.len());
    println!("restore accepted: {}", inst.set_state(&state));
    Ok(())
}
