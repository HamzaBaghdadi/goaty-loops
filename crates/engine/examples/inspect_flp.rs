//! Prints what Goaty Loops understands of one or more `.flp` files.
//!
//! cargo run -p goaty-engine --example inspect_flp -- [-v] [--wav out.wav] file.flp...

use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::import_flp::convert;
use goaty_engine::{ClipSource, Instrument};
use goaty_flp::{FlpFile, FlpProject};
use std::path::{Path, PathBuf};

fn main() {
    let mut verbose = false;
    let mut wav: Option<PathBuf> = None;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-v" => verbose = true,
            "--wav" => wav = args.next().map(PathBuf::from),
            _ => files.push(PathBuf::from(a)),
        }
    }
    let mut failures = 0;
    for f in &files {
        if let Err(e) = inspect(f, verbose, wav.as_deref()) {
            failures += 1;
            println!("FAIL {}: {e}", f.display());
        }
    }
    println!("\n{} files, {} failed", files.len(), failures);
    if std::env::var("FLP_WARN_SUMMARY").is_ok() {
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        for f in &files {
            let Ok(file) = FlpFile::read(f) else { continue };
            let r = convert(&FlpProject::from_file(&file), f.parent(), &goaty_engine::plugin::PluginDb::load());
            for w in r.warnings.iter().filter(|w| !w.starts_with("Plugin")) {
                let key = if w.contains("not found") {
                    let ext = w.rsplit('.').next().unwrap_or("").to_lowercase();
                    let prefix: String = w.split(['\\', '/']).take(2).collect::<Vec<_>>().join("\\");
                    format!("not found: {prefix} ... .{ext}")
                } else {
                    w.rsplit(": ").next().unwrap_or(w).to_string()
                };
                *reasons.entry(key).or_default() += 1;
            }
        }
        let mut v: Vec<_> = reasons.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in v.into_iter().take(25) {
            println!("{n:>5}  {k}");
        }
    }
}

fn inspect(path: &Path, verbose: bool, wav: Option<&Path>) -> anyhow::Result<()> {
    let bytes = std::fs::read(path)?;
    let file = FlpFile::parse(&bytes)?;
    let lossless = file.to_bytes() == bytes;
    let flp = FlpProject::from_file(&file);
    let result = convert(&flp, path.parent(), &goaty_engine::plugin::PluginDb::load());
    let p = &result.project;

    let notes: usize = p.patterns.iter().map(|x| x.notes.len()).sum();
    let audio_clips = p.playlist.clips.iter().filter(|c| matches!(c.source, ClipSource::Audio(_))).count();
    let stand_ins = p.channels.iter().filter(|c| matches!(&c.instrument, Instrument::Synth(s) if s.replaces.is_some())).count();
    let samplers = p.channels.iter().filter(|c| matches!(c.instrument, Instrument::Sampler(_))).count();
    let loaded = p.channels.iter().filter(|c| matches!(&c.instrument, Instrument::Sampler(s) if s.data.is_some())).count();
    println!(
        "{}\n  FL {} | ppq {} | {:.2} BPM | {}/{} | lossless={} | {} events\n  {} channels ({} samplers, {} loaded, {} plugin stand-ins) | {} patterns, {} notes | {} clips ({} audio) on {} tracks | {} mixer tracks | song {} bars",
        path.file_name().unwrap().to_string_lossy(),
        flp.version.as_deref().unwrap_or("?"),
        flp.ppq,
        p.tempo,
        p.time_sig.0,
        p.time_sig.1,
        lossless,
        file.events.len(),
        p.channels.len(),
        samplers,
        loaded,
        stand_ins,
        p.patterns.len(),
        notes,
        p.playlist.clips.len(),
        audio_clips,
        p.playlist.tracks.len(),
        p.mixer.len(),
        p.song_length() / p.bar_ticks(),
    );
    if std::env::var("FLP_ITEMS").is_ok() {
        for (ai, a) in flp.arrangements.iter().enumerate() {
            let max_pos = a.items.iter().map(|i| i.position).max().unwrap_or(0);
            println!("    arr{ai} iid={} name={:?} items={} tracks={} max_pos={max_pos}", a.iid, a.name, a.items.len(), a.tracks.len());
            for it in a.items.iter().filter(|i| i.position > 96 * 4 * 2000 || i.track > 400).take(8) {
                println!("      odd item {it:?}");
            }
        }
        if let Some(pat) = flp.patterns.first() {
            for n in pat.notes.iter().take(12) {
                println!("    note ch{} pos {} len {} key {} vel {} flags {:#x}", n.rack_channel, n.position, n.length, n.key, n.velocity, n.flags);
            }
        }
        let cols: Vec<_> = flp.inserts.iter().take(30).map(|i| i.color.map(|c| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]))).collect();
        println!("    insert colors: {cols:?}");
        let vols: Vec<_> = flp.inserts.iter().take(24).map(|i| (i.volume, i.pan, i.enabled)).collect();
        println!("    insert raw (vol, pan, enabled): {vols:?}");
        let chv: Vec<_> = flp.channels.iter().take(24).map(|c| (c.volume, c.mixer_insert)).collect();
        println!("    channel (vol, insert): {chv:?}");
        let raw: Vec<_> = file.events.iter().filter(|e| e.id == goaty_flp::ids::ARR_PLAYLIST).map(|e| e.blob().map(|b| b.len()).unwrap_or(0)).collect();
        println!("    playlist blob sizes {raw:?}");
    }
    if verbose {
        for (i, c) in p.channels.iter().enumerate() {
            let what = match &c.instrument {
                Instrument::Sampler(s) => format!("sampler {}", s.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()),
                Instrument::Synth(s) => format!("synth (for {})", s.replaces.as_deref().unwrap_or("-")),
                Instrument::Drum(d) => format!("drum {:?}", d.kind),
                Instrument::Silent { label } => label.clone(),
                Instrument::Plugin(p) => format!("VST3 {} (state {} bytes)", p.info.name, p.state.len()),
                Instrument::Automation(a) => format!("automation, {} points -> {:?}", a.points.len(), a.target),
            };
            println!("    ch{i:<3} {:<24} vol {:.2} pan {:+.2} -> ins {:<3} {what}", c.name, c.volume, c.pan, c.mixer_track);
        }
        for (i, pat) in p.patterns.iter().enumerate() {
            println!("    pat{i:<3} {:<24} {} notes, {} ticks", pat.name, pat.notes.len(), p.pattern_length(i));
        }
        for (i, m) in p.mixer.iter().enumerate().filter(|(_, m)| !m.name.starts_with("Insert ")) {
            println!("    ins{i:<3} {:<24} vol {:.2} pan {:+.2}", m.name, m.volume, m.pan);
        }
        for w in &result.warnings {
            println!("    ! {w}");
        }
    }
    if let Some(out) = wav {
        let opts = ExportOptions { song: !p.playlist.clips.is_empty(), ..Default::default() };
        render_to_wav(p, out, &opts, |_| {})?;
        println!("  rendered to {}", out.display());
    }
    Ok(())
}
