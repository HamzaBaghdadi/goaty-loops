//! Renders a one-bar pattern through a VST3 instrument to check the
//! sequencer -> plugin path end to end.
//!
//! cargo run -p goaty-engine --example plugin_song -- "C:\...\Vital.vst3" out.wav

use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::plugin::scan_module;
use goaty_engine::*;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("plugin path");
    let out = args.next().unwrap_or_else(|| "plugin_song.wav".into());
    let info = scan_module(path.as_ref())?.into_iter().next().expect("no plugin class");
    let mut p = Project::empty();
    p.channels.push(Channel::new(info.name.clone(), Instrument::Plugin(PluginParams::new(info, None))));
    for (i, key) in [60u8, 64, 67, 72].iter().enumerate() {
        p.patterns[0].notes.push(Note { channel: 0, key: *key, start: i as u32 * PPQ, length: PPQ / 2, velocity: 0.8, pan: 0.0 });
    }
    let opts = ExportOptions { song: false, tail_secs: 1.0, ..Default::default() };
    render_to_wav(&p, out.as_ref(), &opts, |_| {})?;
    let mut r = hound::WavReader::open(&out)?;
    let samples: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
    let quarter = samples.len() / 5;
    for (i, chunk) in samples.chunks(quarter.max(1)).enumerate() {
        let peak = chunk.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        println!("segment {i}: peak {:.3}", peak as f32 / 32768.0);
    }
    Ok(())
}
