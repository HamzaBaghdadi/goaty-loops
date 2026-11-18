use crate::audio::Shared;
use crate::model::{Project, PPQ};
use crate::render::Renderer;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

pub struct ExportOptions {
    pub sample_rate: u32,
    /// Render the playlist (song); otherwise render `pattern` once.
    pub song: bool,
    pub pattern: usize,
    /// Extra seconds rendered after the end for reverb/delay tails.
    pub tail_secs: f64,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { sample_rate: 44100, song: true, pattern: 0, tail_secs: 2.0 }
    }
}

/// Renders the project offline to a 16-bit stereo WAV file.
/// `progress` receives values in 0..=1.
pub fn render_to_wav(project: &Project, path: &Path, opts: &ExportOptions, progress: impl FnMut(f32)) -> Result<()> {
    let spec = hound::WavSpec { channels: 2, sample_rate: opts.sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(path, spec)?;
    render_offline(project, opts, &[], progress, |mix, _| {
        for s in mix {
            writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        Ok(())
    })?;
    writer.finalize()?;
    Ok(())
}

/// Renders the project offline to a 24-bit stereo FLAC file (lossless).
pub fn render_to_flac(project: &Project, path: &Path, opts: &ExportOptions, progress: impl FnMut(f32)) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    const FULL_SCALE: f32 = 8_388_607.0; // 24-bit
    let mut samples: Vec<i32> = Vec::new();
    render_offline(project, opts, &[], progress, |mix, _| {
        samples.extend(mix.iter().map(|s| (s.clamp(-1.0, 1.0) * FULL_SCALE) as i32));
        Ok(())
    })?;
    let config = flacenc::config::Encoder::default().into_verified().map_err(|e| anyhow::anyhow!("FLAC config: {e:?}"))?;
    // Pad with silence to whole blocks: decoders (symphonia included) expect the
    // minimum block size in the stream header to match the fixed block size.
    let block = config.block_size * 2;
    samples.resize(samples.len().div_ceil(block) * block, 0);
    let source = flacenc::source::MemSource::from_samples(&samples, 2, 24, opts.sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).map_err(|e| anyhow::anyhow!("FLAC encoding failed: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| anyhow::anyhow!("FLAC writing failed: {e:?}"))?;
    std::fs::write(path, sink.as_slice())?;
    Ok(())
}

/// Renders one WAV per mixer insert that carries audio (after its effects and
/// fader, before the master), plus the full mix, into `dir`. Stems are 32-bit
/// float so their peaks above 0 dB survive. Returns the written files.
pub fn render_stems(project: &Project, dir: &Path, opts: &ExportOptions, progress: impl FnMut(f32)) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let tracks = stem_tracks(project);
    let float = hound::WavSpec { channels: 2, sample_rate: opts.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut paths = vec![dir.join(format!("00 {}.wav", file_name(&project.mixer[0].name)))];
    paths.extend(tracks.iter().map(|&t| dir.join(format!("{t:02} {}.wav", file_name(&project.mixer[t].name)))));
    let mut writers = paths.iter().map(|p| hound::WavWriter::create(p, float)).collect::<Result<Vec<_>, _>>()?;
    render_offline(project, opts, &tracks, progress, |mix, stems| {
        for s in mix {
            writers[0].write_sample(*s)?;
        }
        for (w, stem) in writers[1..].iter_mut().zip(stems) {
            // A track not processed in a block (e.g. no audio yet) stays silent.
            for i in 0..mix.len() {
                w.write_sample(stem.get(i).copied().unwrap_or(0.0))?;
            }
        }
        Ok(())
    })?;
    for w in writers {
        w.finalize()?;
    }
    Ok(paths)
}

/// Mixer inserts (not the master) that receive audio: from a channel, or
/// from another insert that does.
pub fn stem_tracks(project: &Project) -> Vec<usize> {
    let n = project.mixer.len();
    let mut used = vec![false; n];
    for c in &project.channels {
        if c.mixer_track < n {
            used[c.mixer_track] = true;
        }
    }
    // Carry usage along outputs and sends until nothing changes (routes can
    // point either way).
    loop {
        let mut changed = false;
        for t in 1..n {
            if !used[t] {
                continue;
            }
            let track = &project.mixer[t];
            for d in std::iter::once(track.output).chain(track.sends.iter().map(|s| s.to)) {
                if d < n && !used[d] {
                    used[d] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    (1..n).filter(|&t| used[t]).collect()
}

fn file_name(s: &str) -> String {
    let clean: String = s.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    if clean.is_empty() { "Insert".into() } else { clean }
}

/// The offline render loop: renders until the song (or pattern) ends plus the
/// tail, handing each block's master mix and stems to `sink`.
fn render_offline(
    project: &Project,
    opts: &ExportOptions,
    stem_tracks: &[usize],
    mut progress: impl FnMut(f32),
    mut sink: impl FnMut(&[f32], &[Vec<f32>]) -> Result<()>,
) -> Result<()> {
    // Plugins get their own instances so export never competes with live playback.
    let (project, _errors) = project.with_new_plugin_instances(opts.sample_rate as f64);
    let project = &project;
    let shared = Shared::new(project.clone());
    shared.transport.song_mode.store(opts.song, Ordering::Relaxed);
    shared.transport.pattern.store(opts.pattern, Ordering::Relaxed);
    shared.transport.playing.store(true, Ordering::Relaxed);

    let mut renderer = Renderer::new(opts.sample_rate, None);
    renderer.loop_enabled = false;
    renderer.stem_tracks = stem_tracks.to_vec();
    let ticks = if opts.song { project.song_length() } else { project.pattern_length(opts.pattern) } as f64;
    // Estimate for progress only; tempo automation can change the real length.
    let est_frames = (ticks * 60.0 / (project.tempo * PPQ as f64) * opts.sample_rate as f64).max(1.0);
    let tail_frames = (opts.tail_secs * opts.sample_rate as f64) as usize;
    // Safety cap: 4x the estimate or 2 hours.
    let max_frames = ((est_frames * 4.0) as usize).min(opts.sample_rate as usize * 7200);

    let mut buf = vec![0f32; 1024 * 2];
    let mut written = 0;
    let mut tail_left: Option<usize> = None;
    // Plugin delay compensation delays everything by the slowest path; like FL's
    // "Trim PDC silence" (on by default), drop that silence from the start and
    // render as much longer at the end.
    let mut trim: Option<usize> = None;
    while written < max_frames {
        let n = match tail_left {
            Some(0) => break,
            Some(t) => t.min(1024),
            None => 1024,
        };
        renderer.process(&shared, &mut buf[..n * 2]);
        let skip = trim.get_or_insert_with(|| shared.latency_samples.load(Ordering::Relaxed) as usize);
        let drop = (*skip).min(n);
        *skip -= drop;
        if drop < n {
            let stems: Vec<Vec<f32>> = if drop == 0 { Vec::new() } else { renderer.stems.iter().map(|s| s.get(drop * 2..).unwrap_or(&[]).to_vec()).collect() };
            sink(&buf[drop * 2..n * 2], if drop == 0 { &renderer.stems } else { &stems })?;
        }
        written += n;
        match &mut tail_left {
            Some(t) => *t -= n,
            None if renderer.position() >= ticks => tail_left = Some(tail_frames + shared.latency_samples.load(Ordering::Relaxed) as usize),
            None => {}
        }
        progress((written as f64 / (est_frames + tail_frames as f64)).min(1.0) as f32);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_default_template() {
        let mut p = Project::new_template();
        for s in [0, 4, 8, 12] {
            p.patterns[0].toggle_step(0, s);
        }
        let dir = std::env::temp_dir().join("goaty_export_test.wav");
        let opts = ExportOptions { song: false, tail_secs: 0.5, ..Default::default() };
        render_to_wav(&p, &dir, &opts, |_| {}).unwrap();
        let mut r = hound::WavReader::open(&dir).unwrap();
        let peak = r.samples::<i16>().map(|s| s.unwrap().unsigned_abs()).max().unwrap();
        assert!(peak > 1000, "expected audible kick, peak was {peak}");
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn flac_decodes_back() {
        let mut p = Project::new_template();
        for s in [0, 4, 8, 12] {
            p.patterns[0].toggle_step(0, s);
        }
        let path = std::env::temp_dir().join("goaty_export_test.flac");
        let opts = ExportOptions { song: false, tail_secs: 0.2, ..Default::default() };
        render_to_flac(&p, &path, &opts, |_| {}).unwrap();
        // Read it back with the same decoder used for samples.
        let back = crate::sample::load_audio(&path).unwrap();
        assert_eq!(back.sample_rate, 44_100);
        let peak = back.samples.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.05, "peak {peak}");
        let _ = std::fs::remove_file(&path);
    }

    /// Insert 2's level right after the kick vs. between kicks, with the
    /// Limiter's compressor listening to the given sidechain input.
    fn duck_ratio(sidechain: u8) -> f32 {
        let mut p = Project::new_template();
        // Kick (channel 0) on insert 1, sidechained (level 0) into insert 2.
        p.channels[0].mixer_track = 1;
        p.mixer[1].sends.push(crate::model::MixerSend { to: 2, gain: 0.0 });
        for s in [0, 4, 8, 12] {
            p.patterns[0].toggle_step(0, s);
        }
        // A sustained synth on insert 2.
        let mut pad = crate::model::Channel::new("Pad", crate::model::Instrument::Synth(Default::default()));
        pad.mixer_track = 2;
        p.channels.push(pad);
        let ch = p.channels.len() - 1;
        p.patterns[0].notes.push(crate::model::Note { channel: ch, key: 48, start: 0, length: PPQ * 4, velocity: 1.0, pan: 0.0 });
        p.mixer[2].effects.push(crate::model::EffectSlot {
            mix: 1.0,
            fl: None,
            enabled: true,
            effect: crate::model::Effect::Limiter { gain_db: 0.0, ceiling_db: 0.0, release_ms: 500.0, comp_threshold_db: -40.0, comp_ratio: 20.0, sidechain },
        });
        let dir = std::env::temp_dir().join(format!("goaty_sc_test_{sidechain}"));
        let opts = ExportOptions { song: false, tail_secs: 0.0, ..Default::default() };
        let files = render_stems(&p, &dir, &opts, |_| {}).unwrap();
        let pad_stem: Vec<f32> = hound::WavReader::open(files.iter().find(|f| f.to_string_lossy().contains("02 ")).unwrap()).unwrap().samples::<f32>().map(|s| s.unwrap()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        // At 130 BPM a beat is ~20 350 frames (stereo samples x2). Kick at beat 1;
        // compare 20-60 ms after it with the middle of the beat.
        let rms = |a: usize, b: usize| (pad_stem[a * 2..b * 2].iter().map(|x| x * x).sum::<f32>() / ((b - a) * 2) as f32).sqrt();
        let beat = (44_100.0 * 60.0 / 130.0) as usize;
        rms(beat + 900, beat + 2600) / rms(beat + beat / 2, beat + beat / 2 + 1700)
    }

    #[test]
    fn sidechain_ducks_on_the_kick() {
        let ducked = duck_ratio(1);
        let plain = duck_ratio(0);
        assert!(ducked < 0.5, "with sidechain the pad should dip after the kick: {ducked}");
        assert!(plain > 0.8, "without sidechain no dip: {plain}");
    }

    #[test]
    fn stems_hold_each_insert() {
        // Kick (channel 0) on insert 1, clap (channel 1) on insert 2.
        let mut p = Project::new_template();
        p.channels[0].mixer_track = 1;
        p.channels[1].mixer_track = 2;
        p.patterns[0].toggle_step(0, 0);
        p.patterns[0].toggle_step(1, 8);
        assert_eq!(stem_tracks(&p), vec![1, 2]);
        let dir = std::env::temp_dir().join("goaty_stems_test");
        let opts = ExportOptions { song: false, tail_secs: 0.2, ..Default::default() };
        let files = render_stems(&p, &dir, &opts, |_| {}).unwrap();
        assert_eq!(files.len(), 3);
        let read = |f: &std::path::Path| -> Vec<f32> { hound::WavReader::open(f).unwrap().samples::<f32>().map(|s| s.unwrap()).collect() };
        let (mix, kick, clap) = (read(&files[0]), read(&files[1]), read(&files[2]));
        assert_eq!(mix.len(), kick.len());
        let peak = |s: &[f32]| s.iter().fold(0f32, |m, x| m.max(x.abs()));
        // The kick hits at step 0, the clap at step 8 (half way): each stem only has its own.
        let half = kick.len() / 2;
        assert!(peak(&kick[..half / 2]) > 0.05 && peak(&clap[..half / 2]) < 1e-4);
        assert!(peak(&clap[half..]) > 0.05);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
