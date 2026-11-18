//! Round-trip check of the FLP patch writer (read-only on the input files).
//! 1. Unmodified import -> patch must be byte-identical.
//! 2. Edited project -> patch -> re-import must show exactly the edits.
use goaty_engine::flp_patch::patch_flp;
use goaty_engine::import_flp::convert;
use goaty_engine::plugin::PluginDb;
use goaty_engine::*;
use goaty_flp::{FlpFile, FlpProject};

fn main() {
    let db = PluginDb::default();
    let (mut identical, mut edits_ok, mut total) = (0, 0, 0);
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(file) = FlpFile::parse(&bytes) else { continue };
        total += 1;
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
        let mut project = convert(&FlpProject::from_file(&file), None, &db).project;

        let same = patch_flp(&file, &project).file.to_bytes() == bytes;
        if same {
            identical += 1;
        } else {
            println!("NOT IDENTICAL: {name}");
            let orig = FlpProject::from_file(&file);
            let cur = convert(&orig, None, &db).project;
            println!("  markers in file arrangements: {:?}", orig.arrangements.iter().map(|a| (a.iid, a.markers.len())).collect::<Vec<_>>());
            println!("  imported markers: {}, current arrangement {:?}", cur.playlist.markers.len(), orig.current_arrangement);
        }

        // Apply edits.
        project.tempo = (project.tempo + 3.5).min(500.0);
        let ch = project.channels.iter().position(|c| c.fl_iid.is_some() && !matches!(c.instrument, Instrument::Automation(_)));
        if let Some(ch) = ch {
            project.channels[ch].pan = -0.5;
            project.channels[ch].mixer_track = (project.channels[ch].mixer_track + 1) % project.mixer.len().max(1);
            if let Some(p) = project.patterns.first_mut() {
                p.notes.push(Note { channel: ch, key: 72, start: PPQ * 3, length: PPQ, velocity: 0.5, pan: 0.0 });
            }
        }
        if let Some(c) = project.playlist.clips.first_mut() {
            c.start += PPQ * 4;
        }
        // Split the longest audio clip (exercises millisecond offsets).
        let audio = project.playlist.clips.iter().enumerate().filter(|(_, c)| matches!(c.source, ClipSource::Audio(a) if matches!(project.channels[a].instrument, Instrument::Sampler(_))) && c.length > 2 * PPQ).max_by_key(|(_, c)| c.length).map(|(i, _)| i);
        if let Some(i) = audio {
            let c = project.playlist.clips[i];
            let half = (c.length / 2 / PPQ).max(1) * PPQ; // on a beat, like a snapped split
            project.playlist.clips[i].length = half;
            project.playlist.clips.push(Clip { start: c.start + half, length: c.length - half, offset: c.offset + half, ..c });
        }
        if project.mixer.len() > 1 {
            project.mixer[1].volume = 0.5;
        }
        // Clip gain on the first audio clip, where the file's playlist items have room for it (FL 21+).
        let flp_meta = FlpProject::from_file(&file);
        let major = flp_meta.version.as_deref().map(|v| goaty_flp::parse_version(v).0).unwrap_or(20);
        let item_size = file.events.iter().find(|e| e.id == 233).and_then(|e| e.blob()).map(|b| goaty_flp::playlist_item_size(b, major)).unwrap_or(32);
        if let Some(c) = project.playlist.clips.iter_mut().find(|c| item_size >= 60 && matches!(c.source, ClipSource::Audio(_))) {
            c.gain *= 0.5;
        }
        // Routing: a new send on insert 1, and a changed level on an existing send.
        if project.mixer.len() > 3 && project.mixer[1].output != 3 && !project.mixer[1].sends.iter().any(|s| s.to == 3) {
            project.mixer[1].sends.push(MixerSend { to: 3, gain: 0.5 });
        }
        if let Some(s) = project.mixer.iter_mut().skip(2).find_map(|t| t.sends.first_mut()) {
            s.gain *= 0.5;
        }
        project.playlist.markers.push(TimeMarker { tick: PPQ * 8, name: "Goaty Loops marker".into(), kind: 0, numerator: 0, denominator: 0 });
        // A sample channel added in Goaty Loops, with a note and an audio clip.
        let sample = project.channels.iter().find_map(|c| match &c.instrument {
            Instrument::Sampler(s) => s.path.clone(),
            _ => None,
        });
        let added = sample.map(|path| {
            let mut c = Channel::new("Goaty Loops sampler", Instrument::Sampler(SamplerParams { path: Some(path), ..Default::default() }));
            c.volume = 0.6;
            project.channels.push(c);
            let idx = project.channels.len() - 1;
            if let Some(p) = project.patterns.first_mut() {
                p.notes.push(Note { channel: idx, key: 64, start: PPQ * 2, length: PPQ, velocity: 0.7, pan: 0.0 });
            }
            project.playlist.clips.push(Clip { track: 0, start: PPQ * 64, length: PPQ * 4, offset: 0, source: ClipSource::Audio(idx), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
            idx
        });
        let mut np = Pattern::new("Goaty Loops pattern", PALETTE[0]);
        if let Some(ch) = ch {
            np.notes.push(Note::step(ch, 0));
        }
        project.patterns.push(np);

        let res = patch_flp(&file, &project);
        let re = convert(&FlpProject::from_file(&FlpFile::parse(&res.file.to_bytes()).unwrap()), None, &db).project;
        let mut problems = Vec::new();
        if (re.tempo - project.tempo).abs() > 1e-3 {
            problems.push(format!("tempo {} vs {}", re.tempo, project.tempo));
        }
        if let Some(ch) = ch {
            if re.channels[ch].mixer_track != project.channels[ch].mixer_track {
                problems.push(format!("mixer track {} vs {}", re.channels[ch].mixer_track, project.channels[ch].mixer_track));
            }
            if (re.channels[ch].pan + 0.5).abs() > 1e-3 {
                problems.push(format!("pan {}", re.channels[ch].pan));
            }
            if re.patterns.first().map(|p| p.notes.len()) != project.patterns.first().map(|p| p.notes.len()) {
                problems.push("pattern 0 note count".into());
            }
        }
        if re.playlist.clips != project.playlist.clips {
            let ratio = re.tempo / (re.tempo - 3.5);
            let only_offsets = re.playlist.clips.len() == project.playlist.clips.len()
                && re.playlist.clips.iter().zip(&project.playlist.clips).all(|(a, b)| {
                    same_but_tempo(a, b, ratio)
                });
            if !only_offsets {
                if let Some((a, b)) = re.playlist.clips.iter().zip(&project.playlist.clips).find(|(a, b)| !same_but_tempo(a, b, ratio)) {
                    println!("  first diff: {a:?}\n          vs: {b:?}");
                }
                problems.push(format!("clips differ ({} vs {})", re.playlist.clips.len(), project.playlist.clips.len()));
            }
        }
        if project.mixer.len() > 1 && (re.mixer[1].volume - 0.5).abs() > 0.01 {
            problems.push(format!("mixer volume {}", re.mixer[1].volume));
        }
        if let Some(idx) = added {
            match re.channels.iter().position(|c| c.name == "Goaty Loops sampler") {
                None => problems.push("new channel missing".into()),
                Some(ri) => {
                    let same_sample = matches!((&re.channels[ri].instrument, &project.channels[idx].instrument), (Instrument::Sampler(a), Instrument::Sampler(b)) if a.path == b.path);
                    if !same_sample || (re.channels[ri].volume - 0.6).abs() > 1e-3 {
                        problems.push("new channel settings".into());
                    }
                    if !re.patterns.first().is_some_and(|p| p.notes.iter().any(|n| n.channel == ri && n.key == 64)) {
                        problems.push("new channel note".into());
                    }
                    if !re.playlist.clips.iter().any(|c| c.source == ClipSource::Audio(ri) && c.start == PPQ * 64) {
                        problems.push("new channel clip".into());
                    }
                }
            }
            if res.warnings.iter().any(|w| w.contains("can't be written")) {
                problems.push(format!("warning: {:?}", res.warnings));
            }
        }
        // Routing as a set of (destination, level); the main output is at full level.
        let routing = |p: &Project| p.mixer.iter().map(|t| { let mut s: Vec<_> = t.sends.iter().map(|s| (s.to, (s.gain * 1000.0).round() as i32)).chain([(t.output, 1000)]).collect(); s.sort(); s }).collect::<Vec<_>>();
        if routing(&re) != routing(&project) {
            let d = routing(&re).into_iter().zip(routing(&project)).enumerate().find(|(_, (a, b))| a != b);
            problems.push(format!("routing differs: {d:?}"));
        }
        if re.playlist.markers != project.playlist.markers {
            let d = re.playlist.markers.iter().zip(&project.playlist.markers).find(|(a, b)| a != b);
            problems.push(format!("markers {} vs {}: {d:?}", re.playlist.markers.len(), project.playlist.markers.len()));
        }
        if !re.patterns.iter().any(|p| p.name == "Goaty Loops pattern") {
            problems.push("new pattern missing".into());
        }
        if problems.is_empty() {
            edits_ok += 1;
        } else {
            println!("EDIT PROBLEMS in {name}: {}", problems.join("; "));
        }
    }
    println!("\n{total} files: {identical} byte-identical when unmodified, {edits_ok} with all edits preserved");
}

/// Equal clips, allowing millisecond-based fields (audio offsets and fades)
/// to scale with the test's tempo change: FL keeps them in real time.
fn same_but_tempo(a: &Clip, b: &Clip, ratio: f64) -> bool {
    let close = |x: u32, y: u32| x == y || (x as f64 - y as f64 * ratio).abs() <= 1.5;
    Clip { offset: b.offset, fade_in: b.fade_in, fade_out: b.fade_out, ..*a } == *b && close(a.offset, b.offset) && close(a.fade_in, b.fade_in) && close(a.fade_out, b.fade_out)
}
