//! Prints a channel's imported settings and its mixer insert chain (read-only).
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};
fn main() {
    let mut a = std::env::args().skip(1);
    let (path, name) = (a.next().unwrap(), a.next().unwrap());
    let p = convert(&FlpProject::from_file(&FlpFile::read(&path).unwrap()), std::path::Path::new(&path).parent(), &PluginDb::load()).project;
    for (i, c) in p.channels.iter().enumerate().filter(|(_, c)| c.name.contains(&name)) {
        println!("channel {i} {:?}: volume {:.3} ({:.1} dB) pan {} muted {} pitch {} insert {}", c.name, c.volume, 20.0 * c.gain().max(1e-9).log10(), c.pan, c.muted, c.pitch, c.mixer_track);
        if let Instrument::Sampler(s) = &c.instrument {
            println!("  sampler: reverse {} stretch {} keep_pitch {} env {} root {} data {:?}", s.reverse, s.stretch_beats, s.keep_pitch, s.use_envelope, s.root_key, s.data.as_ref().map(|d| (d.sample_rate, d.duration_secs())));
        }
        let mut t = c.mixer_track;
        for _ in 0..4 {
            let m = &p.mixer[t];
            println!("  insert {t} {:?}: volume {:.1} dB, pan {}, muted {}, out {} sends {:?}", m.name, 20.0 * m.volume.max(1e-9).log10(), m.pan, m.muted, m.output, m.sends.iter().map(|s| (s.to, (20.0 * s.gain.max(1e-9).log10() * 10.0).round() / 10.0)).collect::<Vec<_>>());
            for e in &m.effects {
                println!("     fx {} enabled {} mix {:.2} {:?}
        {:?}
        raw {:?}", e.effect.name(), e.enabled, e.mix, e.fl.as_ref().map(|f| &f.internal), e.effect, e.fl.as_ref().map(|f| f.data.chunks_exact(4).take(24).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect::<Vec<_>>()));
            }
            if t == 0 { break; }
            t = m.output;
        }
    }
    for c in p.playlist.clips.iter().filter(|c| c.start < 200) {
        println!("clip at {} track {} len {} {:?} gain {}", c.start, c.track, c.length, c.source, c.gain);
    }
}
