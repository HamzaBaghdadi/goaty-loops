//! Survey of audio clip offsets in FLP files, to find their unit.
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::Instrument;
use goaty_flp::{FlpFile, FlpItemSource, FlpProject};
use std::path::Path;

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(file) = FlpFile::read(&path) else { continue };
        let flp = FlpProject::from_file(&file);
        let project = convert(&flp, Path::new(&path).parent(), &PluginDb::default()).project;
        for arr in &flp.arrangements {
            for it in &arr.items {
                let FlpItemSource::Channel(iid) = it.source else {
                    if it.start_offset >= 0.0 && std::env::var("PATS").is_ok() {
                        println!("PAT so={} eo={} len={}", it.start_offset, it.end_offset, it.length);
                    }
                    continue;
                };
                let ch = project.channels.iter().find(|c| c.fl_iid == Some(iid));
                let dur = ch.and_then(|c| match &c.instrument {
                    Instrument::Sampler(s) => s.data.as_ref().map(|d| d.duration_secs()),
                    _ => None,
                });
                let beat_secs = 60.0 / flp.tempo;
                let len_secs = it.length as f64 / flp.ppq as f64 * beat_secs;
                println!(
                    "{:<28} so={:>10.3} eo={:>10.3} len={:>6} ({:>7.3}s) dur={:?} ppq={} bpm={}",
                    Path::new(&path).file_stem().unwrap().to_string_lossy().chars().take(28).collect::<String>(),
                    it.start_offset, it.end_offset, it.length, len_secs, dur.map(|d| (d * 1000.0).round() / 1000.0), flp.ppq, flp.tempo
                );
            }
        }
    }
}
