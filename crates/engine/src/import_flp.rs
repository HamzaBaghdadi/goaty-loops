//! Converts FL Studio projects into Goaty Loops projects (and back).

use crate::model::*;
use anyhow::Result;
use crate::plugin::PluginDb;
use goaty_flp::{
    FlpControlTarget,
    FlpPluginWrapper,
    FlpArrangement, FlpChannel, FlpFile, FlpInsert, FlpItemSource, FlpNote, FlpPattern, FlpPlaylistItem, FlpProject, FlpTrack,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ImportResult {
    pub project: Project,
    pub warnings: Vec<String>,
}

fn rgb(c: Option<[u8; 3]>, fallback: Rgb) -> Rgb {
    c.map(|[r, g, b]| Rgb(r, g, b)).unwrap_or(fallback)
}

pub fn import_flp(path: &Path, db: &PluginDb) -> Result<ImportResult> {
    let file = FlpFile::read(path)?;
    let flp = FlpProject::from_file(&file);
    let mut r = convert(&flp, path.parent(), db);
    if let Some(o) = &mut r.project.fl_origin {
        o.path = path.to_path_buf();
    }
    Ok(r)
}

pub fn convert(flp: &FlpProject, base_dir: Option<&Path>, db: &PluginDb) -> ImportResult {
    let mut warnings = Vec::new();
    let scale = PPQ as f64 / flp.ppq.max(1) as f64;

    let mut p = Project::empty();
    p.name = if flp.title.is_empty() { "Imported project".into() } else { flp.title.clone() };
    p.tempo = if flp.tempo > 0.0 { flp.tempo } else { 140.0 };
    p.time_sig = flp.time_sig;
    p.info = ProjectInfo { artists: flp.artists.clone(), genre: flp.genre.clone(), comments: flp.comments.clone() };

    // Mixer first so channel routing can be clamped.
    if !flp.inserts.is_empty() {
        p.mixer = flp.inserts.iter().enumerate().map(|(i, ins)| mixer_track(i, ins)).collect();
        for (t, ins) in flp.inserts.iter().enumerate() {
            let name = p.mixer[t].name.clone();
            p.mixer[t].effects = convert_slots(ins, &name, db, &mut warnings);
        }
    }

    let mut channel_map = HashMap::new();
    for ch in &flp.channels {
        channel_map.insert(ch.iid, p.channels.len());
        let (instrument, warning) = instrument_for(ch, base_dir, db);
        if let Some(w) = warning {
            warnings.push(w);
        }
        let name = ch
            .name
            .clone()
            .or_else(|| ch.sample_path.as_deref().and_then(file_stem))
            .or_else(|| ch.plugin.clone())
            .unwrap_or_else(|| format!("Channel {}", p.channels.len() + 1));
        let mut c = Channel::new(name, instrument);
        c.fl_iid = Some(ch.iid);
        c.color = rgb(ch.color, PALETTE[p.channels.len() % PALETTE.len()]);
        c.volume = ch.volume;
        c.pan = ch.pan;
        c.muted = !ch.enabled;
        c.mixer_track = ch.mixer_insert.unwrap_or(0).clamp(0, p.mixer.len() as i32 - 1) as usize;
        c.pitch = ch.pitch_cents as f32 / 100.0;
        c.pitch_range = c.pitch.abs().max(2.0).ceil();
        c.cut_group = ch.cut;
        c.cut_by = ch.cut_by;
        p.channels.push(c);
    }

    // Automation targets.
    let mut unsupported_auto = 0;
    for c in &flp.controllers {
        let Some(&src) = channel_map.get(&c.source_channel) else { continue };
        let target = match c.target {
            FlpControlTarget::Channel { iid, param } => channel_map.get(&iid).and_then(|&ch| match param {
                0 | 17 => Some(AutoTarget::ChannelVolume(ch)),
                1 | 16 => Some(AutoTarget::ChannelPan(ch)),
                4 => Some(AutoTarget::ChannelPitch(ch)),
                _ => None,
            }),
            FlpControlTarget::Global { param } => match param {
                0 => Some(AutoTarget::MainVolume),
                5 => Some(AutoTarget::Tempo),
                _ => None,
            },
            FlpControlTarget::Mixer { insert, slot, param } => match param {
                0xc0 if (insert as usize) < p.mixer.len() => Some(AutoTarget::MixerVolume(insert as usize)),
                0xc1 if (insert as usize) < p.mixer.len() => Some(AutoTarget::MixerPan(insert as usize)),
                0x00 | 0x01 if p.mixer.get(insert as usize).is_some_and(|t| t.effects.iter().any(|e| e.fl.as_ref().is_some_and(|f| f.slot as u16 == slot))) => {
                    let (track, slot) = (insert as usize, slot as u8);
                    Some(if param == 1 { AutoTarget::SlotMix { track, slot } } else { AutoTarget::SlotEnabled { track, slot } })
                }
                _ => None,
            },
            FlpControlTarget::ChannelPlugin { iid, param } => channel_map
                .get(&iid)
                .filter(|&&ch| matches!(p.channels[ch].instrument, Instrument::Plugin(_)))
                .map(|&ch| AutoTarget::PluginParam { channel: ch, param: param as u32 }),
            FlpControlTarget::MixerPlugin { insert, slot, param } => {
                let track = insert as usize;
                let known = p.mixer.get(track).is_some_and(|t| {
                    t.effects.iter().any(|e| {
                        e.fl.as_ref().is_some_and(|f| f.slot as u16 == slot && (matches!(e.effect, Effect::Plugin(_)) || crate::fl_effects::param_field(&f.internal, param as u32).is_some()))
                    })
                });
                known.then_some(AutoTarget::EffectParam { track, slot: slot as u8, param: param as u32 })
            }
        };
        match (&mut p.channels[src].instrument, target) {
            (Instrument::Automation(a), Some(t)) => a.target = Some(t),
            (Instrument::Automation(_), None) => unsupported_auto += 1,
            _ => {}
        }
    }
    if unsupported_auto > 0 {
        warnings.push(format!("{unsupported_auto} automation clip(s) control parameters Goaty Loops can't automate yet"));
    }

    let mut pattern_map = HashMap::new();
    let mut patterns: Vec<&FlpPattern> = flp.patterns.iter().collect();
    patterns.sort_by_key(|p| p.iid);
    if !patterns.is_empty() {
        p.patterns.clear();
    }
    for (i, fp) in patterns.into_iter().enumerate() {
        pattern_map.insert(fp.iid, i);
        let mut pat = Pattern::new(fp.name.clone().unwrap_or_else(|| format!("Pattern {}", fp.iid)), rgb(fp.color, PALETTE[i % PALETTE.len()]));
        pat.fl_iid = Some(fp.iid);
        for n in &fp.notes {
            let Some(&channel) = channel_map.get(&n.rack_channel) else { continue };
            pat.notes.push(note_from_flp(n, channel, scale));
        }
        p.patterns.push(pat);
    }

    // FL doesn't store empty patterns, but playlist clips can still use them:
    // add those as empty patterns so the clips (and the saved file) keep them.
    let mut missing: Vec<u16> = flp
        .arrangements
        .iter()
        .flat_map(|a| a.items.iter())
        .filter_map(|it| match it.source {
            FlpItemSource::Pattern(iid) if !pattern_map.contains_key(&iid) => Some(iid),
            _ => None,
        })
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if !missing.is_empty() && flp.patterns.is_empty() {
        p.patterns.clear(); // drop the default placeholder pattern
    }
    if !missing.is_empty() {
        for iid in missing {
            let mut pat = Pattern::new(format!("Pattern {iid}"), PALETTE[iid as usize % PALETTE.len()]);
            pat.fl_iid = Some(iid);
            p.patterns.push(pat);
        }
        // Keep patterns in FL's pattern order, stored or not.
        p.patterns.sort_by_key(|pat| pat.fl_iid.unwrap_or(u16::MAX));
        pattern_map = p.patterns.iter().enumerate().filter_map(|(i, pat)| Some((pat.fl_iid?, i))).collect();
    }

    let arrangement = flp
        .current_arrangement
        .and_then(|cur| flp.arrangements.iter().find(|a| a.iid == cur))
        .filter(|a| !a.items.is_empty())
        .or_else(|| flp.arrangements.iter().find(|a| !a.items.is_empty()))
        .or(flp.arrangements.first());
    if let Some(arr) = arrangement {
        let used = arr.items.iter().map(|i| i.track as usize + 1).max().unwrap_or(0);
        let named = arr.tracks.iter().rposition(|t| t.name.is_some()).map(|i| i + 1).unwrap_or(0);
        p.playlist.tracks.clear();
        p.ensure_tracks(used.max(named).max(16));
        for (i, t) in arr.tracks.iter().enumerate().take(p.playlist.tracks.len()) {
            if let Some(n) = &t.name {
                p.playlist.tracks[i].name = n.clone();
            }
            p.playlist.tracks[i].muted = !t.enabled;
            if let Some([r, g, b]) = t.color {
                p.playlist.tracks[i].color = Rgb(r, g, b);
            }
        }
        p.playlist.markers = arr
            .markers
            .iter()
            .map(|m| marker_from_flp(m, scale))
            .collect();
        for it in &arr.items {
            let source = match it.source {
                FlpItemSource::Pattern(iid) => match pattern_map.get(&iid) {
                    Some(&i) => ClipSource::Pattern(i),
                    None => continue,
                },
                FlpItemSource::Channel(iid) => match channel_map.get(&iid) {
                    Some(&i) => ClipSource::Audio(i),
                    None => continue,
                },
            };
            let offsets = OffsetCodec::new(source, &p.channels, flp.ppq as u32, flp.tempo);
            p.playlist.clips.push(clip_from_flp(it, source, scale, offsets));
        }
    }

    p.fl_origin = Some(FlOrigin { path: PathBuf::new(), arrangement: arrangement.map(|a| a.iid), ppq: flp.ppq });
    for f in p.load_samples() {
        warnings.push(format!("Could not load sample {f}"));
    }
    ImportResult { project: p, warnings }
}

/// Maps a plugin wrapped by FL to an installed VST3 plugin, carrying over its state.
fn map_wrapper(w: &FlpPluginWrapper, db: &PluginDb) -> std::result::Result<PluginParams, String> {
    let Some(guid) = w.guid else {
        return Err(format!("Plugin \"{}\" is a VST2 plugin, which Goaty Loops doesn't host", w.name));
    };
    let class_id: String = guid.iter().map(|b| format!("{b:02X}")).collect();
    let info = db
        .plugins
        .iter()
        .find(|p| p.class_id == class_id)
        .or_else(|| db.find(Some(&w.bundle_path()), Some(&w.name)))
        .cloned()
        .ok_or_else(|| {
            if db.plugins.is_empty() {
                format!("Plugin \"{}\" ({}) can't be loaded until the plugin scan has finished", w.name, w.vendor)
            } else {
                format!("Plugin \"{}\" ({}) is not installed", w.name, w.vendor)
            }
        })?;
    let mut p = PluginParams::new(info, None);
    p.state = w.vst3_component_state().map(|s| s.to_vec()).unwrap_or_default();
    Ok(p)
}

/// Converts insert effect slots; returns warnings for effects that can't be reproduced.
fn convert_slots(ins: &FlpInsert, track_name: &str, db: &PluginDb, warnings: &mut Vec<String>) -> Vec<EffectSlot> {
    let mut out = Vec::new();
    for s in &ins.slots {
        match &s.wrapper {
            Some(w) => match map_wrapper(w, db) {
                Ok(p) => out.push(EffectSlot { mix: s.mix, fl: Some(FlSlotInfo { internal: s.internal.clone(), slot: s.index, data: Vec::new() }), enabled: s.enabled, effect: Effect::Plugin(p) }),
                Err(e) => warnings.push(format!("{e} (mixer track \"{track_name}\")")),
            },
            None => match crate::fl_effects::decode(&s.internal, &s.params) {
                Some(effect) => {
                    if crate::fl_effects::is_approximation(&s.internal) {
                        warnings.push(format!("FL effect \"{}\" on mixer track \"{track_name}\" is approximated by Goaty Limiter", s.internal));
                    }
                    out.push(EffectSlot { mix: s.mix, fl: Some(FlSlotInfo { internal: s.internal.clone(), slot: s.index, data: s.params.clone() }), enabled: s.enabled, effect })
                }
                None => {
                    let name = if s.name.is_empty() { &s.internal } else { &s.name };
                    warnings.push(format!("FL effect \"{name}\" on mixer track \"{track_name}\" isn't available"));
                }
            },
        }
    }
    out
}

fn mixer_track(i: usize, ins: &FlpInsert) -> MixerTrack {
    let default = if i == 0 { "Master".to_string() } else { format!("Insert {i}") };
    let mut t = MixerTrack::new(ins.name.clone().filter(|n| !n.is_empty()).unwrap_or(default));
    if let Some(c) = ins.color {
        t.color = Rgb(c[0], c[1], c[2]);
    }
    if let Some(v) = ins.volume {
        t.volume = fl_fader_to_gain(v);
    }
    if let Some(pan) = ins.pan {
        t.pan = (pan as f32 / 6400.0).clamp(-1.0, 1.0);
    }
    t.muted = !ins.enabled;
    if let Some((output, sends)) = routing_from_flp(i, ins) {
        t.output = output;
        t.sends = sends;
    }
    t
}

/// An FL insert's routing as (main output, sends): the master when routed
/// there, else a full-level route (else the first); the other routes become sends at their send
/// level (12800 = 0 dB, as on faders). `None` keeps the default (master).
pub(crate) fn routing_from_flp(i: usize, ins: &FlpInsert) -> Option<(usize, Vec<MixerSend>)> {
    let routes: Vec<usize> = ins.routes.iter().copied().filter(|&d| d != i).collect();
    if i == 0 || routes.is_empty() {
        return None;
    }
    let level = |d: usize| ins.route_levels.iter().find(|(to, _)| *to == d).map(|(_, v)| *v).unwrap_or(FL_UNITY as i32);
    // The main output plays at full level: prefer the master, then a full-level route.
    let output = if routes.contains(&0) { 0 } else { routes.iter().copied().find(|&d| level(d) == FL_UNITY as i32).unwrap_or(routes[0]) };
    let sends = routes.iter().filter(|&&d| d != output).map(|&d| MixerSend { to: d, gain: fl_fader_to_gain(level(d)) }).collect();
    Some((output, sends))
}

/// FL's raw insert volume: 12800 = 0 dB (the fader's 80% default position),
/// 16000 = +5.6 dB, following FL's exponential volume law (see `dsp::fl_volume_to_gain`).
const FL_UNITY: f32 = 12800.0;

pub fn fl_fader_to_gain(raw: i32) -> f32 {
    crate::dsp::fl_volume_to_gain(raw.max(0) as f32 / FL_UNITY)
}

pub fn gain_to_fl_fader(gain: f32) -> i32 {
    (crate::dsp::gain_to_fl_volume(gain) * FL_UNITY).round() as i32
}

fn file_stem(p: &str) -> Option<String> {
    let name = p.rsplit(['\\', '/']).next()?;
    Some(name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_string())
}

fn instrument_for(ch: &FlpChannel, base_dir: Option<&Path>, db: &PluginDb) -> (Instrument, Option<String>) {
    if let Some(w) = &ch.wrapper {
        return match map_wrapper(w, db) {
            Ok(p) => (Instrument::Plugin(p), None),
            Err(e) => {
                let synth = stand_in_synth(&w.name, ch.name.as_deref().unwrap_or(""));
                (Instrument::Synth(synth), Some(format!("{e}; channel \"{}\" uses Goaty Synth as a stand-in", ch.name.as_deref().unwrap_or(&w.name))))
            }
        };
    }
    if let Some(sp) = &ch.sample_path {
        let resolved = resolve_sample_path(sp, base_dir);
        let warning = resolved.is_none().then(|| format!("Sample not found: {sp}"));
        let params = SamplerParams {
            path: Some(resolved.unwrap_or_else(|| PathBuf::from(sp))),
            root_key: ch.root_note.map(|r| r.min(127) as u8).unwrap_or(60),
            use_envelope: ch.volume_envelope.is_some(),
            env: ch.volume_envelope.map(fl_envelope).unwrap_or_default(),
            reverse: ch.fx_flags & goaty_flp::FX_REVERSE != 0,
            preamp: 1.0 + ch.preamp as f32 / 16.0,
            stretch_beats: ch.stretch_beats,
            keep_pitch: ch.stretch_mode != 0,
            stretch_pitch: ch.stretch_pitch.clamp(-4800, 4800) as f32,
            ..Default::default()
        };
        return (Instrument::Sampler(params), warning);
    }
    if ch.kind == 5 {
        let points = ch.automation.as_deref().unwrap_or(&[]).iter().map(|p| AutoPoint { tick: (p.beats * PPQ as f64).round().max(0.0) as u32, value: p.value as f32, tension: p.tension.clamp(-1.0, 1.0) }).collect();
        return (Instrument::Automation(AutomationParams { points, target: None }), None);
    }
    match ch.plugin.as_deref() {
        Some(plugin) => {
            let lower = plugin.to_ascii_lowercase();
            let mut synth = stand_in_synth(plugin, ch.name.as_deref().unwrap_or(""));
            if lower.contains("3x osc") || lower.contains("3xosc") {
                // 3x Osc maps onto Goaty Synth's three oscillators; it has no filter.
                if let Some(osc) = three_osc(&ch.plugin_params) {
                    synth.osc = osc;
                }
                synth.cutoff = 20_000.0;
                synth.resonance = 0.0;
                // Without a channel envelope 3x Osc holds notes at full level.
                synth.env = ch.volume_envelope.map(fl_envelope).unwrap_or(Envelope { attack: 0.002, hold: 0.0, decay: 0.1, sustain: 1.0, release: 0.05 });
                (Instrument::Synth(synth), None)
            } else {
                let w = format!("Plugin \"{plugin}\" is not available; channel \"{}\" uses Goaty Synth as a stand-in", ch.name.as_deref().unwrap_or(plugin));
                (Instrument::Synth(synth), Some(w))
            }
        }
        None if ch.kind == 0 => (Instrument::Sampler(SamplerParams::default()), None),
        None => (Instrument::Synth(SynthParams::default()), None),
    }
}

/// Resolves FL-style sample paths (which may use `%FLStudioFactoryData%`)
/// to an existing file.
pub fn resolve_sample_path(raw: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(raw)];
    let lower = raw.to_ascii_lowercase();
    for (var, roots) in [
        ("%flstudiofactorydata%", factory_roots()),
        // Older projects use %FLStudioData%, which points at "<install>\Data".
        ("%flstudiodata%", factory_roots().into_iter().map(|r| r.join("Data")).collect()),
        ("%flstudiouserdata%", user_data_roots()),
    ] {
        if let Some(i) = lower.find(var) {
            let rest = raw[i + var.len()..].trim_start_matches(['\\', '/']);
            for r in roots {
                candidates.push(r.join(rest));
            }
        }
    }
    if let (Some(dir), Some(name)) = (base_dir, raw.rsplit(['\\', '/']).next()) {
        candidates.push(dir.join(name));
        candidates.push(dir.join("Samples").join(name));
    }
    candidates.into_iter().find(|c| c.is_file())
}

fn factory_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for pf in [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)")].into_iter().flatten() {
        let il = PathBuf::from(pf).join("Image-Line");
        if let Ok(rd) = std::fs::read_dir(&il) {
            v.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    v.push(PathBuf::from("/Applications/FL Studio.app/Contents/Resources/FL"));
    v
}

fn user_data_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let docs = PathBuf::from(home).join("Documents").join("Image-Line");
        v.push(docs.join("FL Studio"));
        v.push(docs.join("Data").join("FL Studio"));
    }
    v
}

/// Converts a Goaty Loops project into the experimental FLP writer's model.
pub fn to_flp(p: &Project) -> FlpProject {
    let flp_ticks = |t: u32| t; // Goaty Loops uses FL's default PPQ of 96.
    let channels = p
        .channels
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (kind, plugin, sample_path) = match &c.instrument {
                Instrument::Sampler(s) => (0, None, s.path.as_ref().map(|p| p.display().to_string())),
                Instrument::Synth(s) => (2, Some(s.replaces.clone().unwrap_or_else(|| "3x Osc".into())), None),
                Instrument::Drum(_) => (2, Some("3x Osc".into()), None),
                Instrument::Silent { .. } | Instrument::Automation(_) => (5, None, None),
                Instrument::Plugin(p) => (2, Some(p.info.name.clone()), None),
            };
            FlpChannel {
                iid: i as u16,
                kind,
                name: Some(c.name.clone()),
                plugin,
                sample_path,
                color: Some([c.color.0, c.color.1, c.color.2]),
                volume: c.volume,
                pan: c.pan,
                enabled: !c.muted,
                mixer_insert: Some(c.mixer_track as i32),
                root_note: None,
                pitch_cents: (c.pitch * 100.0).round() as i32,
                stretch_beats: 0.0,
                stretch_mode: 0,
                stretch_pitch: 0,
                preamp: match &c.instrument {
                    Instrument::Sampler(s) => ((s.preamp - 1.0) * 16.0).round().max(0.0) as u16,
                    _ => 0,
                },
                fx_flags: match &c.instrument {
                    Instrument::Sampler(s) if s.reverse => goaty_flp::FX_REVERSE,
                    _ => 0,
                },
                volume_envelope: None,
                envelope_count: 0,
                cut: c.cut_group,
                cut_by: c.cut_by,
                wrapper: None,
                plugin_params: Vec::new(),
                automation: None,
            }
        })
        .collect();
    let patterns = p
        .patterns
        .iter()
        .enumerate()
        .map(|(i, pat)| FlpPattern {
            iid: i as u16 + 1,
            name: Some(pat.name.clone()),
            color: Some([pat.color.0, pat.color.1, pat.color.2]),
            notes: pat
                .notes
                .iter()
                .map(|n| FlpNote {
                    position: flp_ticks(n.start),
                    rack_channel: n.channel as u16,
                    length: flp_ticks(n.length),
                    key: n.key as u16,
                    velocity: (n.velocity * 128.0).round() as u8,
                    pan: (n.pan * 64.0 + 64.0).round() as u8,
                    fine_pitch: 120,
                    mod_x: 128,
                    mod_y: 128,
                    ..Default::default()
                })
                .collect(),
        })
        .collect();
    let items = p
        .playlist
        .clips
        .iter()
        .map(|c| {
            let (start_offset, end_offset) = OffsetCodec::new(c.source, &p.channels, PPQ, p.tempo).encode_clip(c);
            FlpPlaylistItem {
                position: flp_ticks(c.start),
                length: flp_ticks(c.length),
                track: c.track as u16,
                source: match c.source {
                    ClipSource::Pattern(i) => FlpItemSource::Pattern(i as u16 + 1),
                    ClipSource::Audio(i) => FlpItemSource::Channel(i as u16),
                },
                start_offset,
                end_offset,
                muted: c.muted,
                fade_in: 0.0,
                fade_out: 0.0,
                gain: c.gain,
            }
        })
        .collect();
    let markers = p
        .playlist
        .markers
        .iter()
        .map(|m| {
            let (numerator, denominator) = m.signature().unwrap_or(p.time_sig);
            goaty_flp::FlpTimeMarker { position: flp_ticks(m.tick), kind: m.kind, name: Some(m.name.clone()), numerator, denominator }
        })
        .collect();
    let tracks = p
        .playlist
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| FlpTrack { iid: i as u32 + 1, name: Some(t.name.clone()), color: Some([t.color.0, t.color.1, t.color.2]), enabled: !t.muted })
        .collect();
    let inserts = p
        .mixer
        .iter()
        .map(|m| FlpInsert {
            name: Some(m.name.clone()),
            color: Some([m.color.0, m.color.1, m.color.2]),
            enabled: !m.muted,
            volume: Some(gain_to_fl_fader(m.volume)),
            pan: Some((m.pan * 6400.0).round() as i32),
            slots: Vec::new(),
            routes: std::iter::once(m.output).chain(m.sends.iter().map(|s| s.to)).collect(),
            route_levels: m.sends.iter().map(|s| (s.to, gain_to_fl_fader(s.gain))).collect(),
        })
        .collect();
    FlpProject {
        version: Some("20.8.4.2576".into()),
        ppq: PPQ as u16,
        tempo: p.tempo,
        time_sig: p.time_sig,
        title: p.name.clone(),
        artists: p.info.artists.clone(),
        genre: p.info.genre.clone(),
        comments: p.info.comments.clone(),
        channels,
        patterns,
        arrangements: vec![FlpArrangement { iid: 0, name: Some("Arrangement".into()), items, tracks, markers }],
        current_arrangement: Some(0),
        legacy_markers: Vec::new(),
        has_arrangements: true,
        inserts,
        controllers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flp_round_trip_preserves_song() {
        let mut p = Project::new_template();
        p.tempo = 97.0;
        p.patterns[0].toggle_step(0, 0);
        p.patterns[0].toggle_step(2, 6);
        p.playlist.clips.push(Clip { track: 3, start: PPQ * 8, length: PPQ * 4, offset: 0, source: ClipSource::Pattern(0), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
        p.channels[1].mixer_track = 2;

        let bytes = to_flp(&p).to_flp_file().to_bytes();
        let flp = FlpProject::from_file(&FlpFile::parse(&bytes).unwrap());
        let back = convert(&flp, None, &PluginDb::default()).project;

        assert_eq!(back.tempo, 97.0);
        assert_eq!(back.channels.len(), 4);
        assert_eq!(back.channels[1].mixer_track, 2);
        assert_eq!(back.patterns[0].notes.len(), 2);
        assert!(back.patterns[0].step_on(2, 6));
        assert_eq!(back.playlist.clips, p.playlist.clips);
    }

    #[test]
    fn stems() {
        assert_eq!(file_stem(r"C:\a\b\Kick 01.wav").as_deref(), Some("Kick 01"));
    }
}

/// FL envelope knob position (100..=65536) -> seconds. Approximation of FL's
/// curve: cubic over 0..10 s.
fn fl_env_secs(v: i32) -> f32 {
    let x = ((v - 100) as f32 / (65536.0 - 100.0)).clamp(0.0, 1.0);
    (10.0 * x * x * x).max(0.001)
}

fn fl_envelope(e: goaty_flp::FlpEnvelope) -> Envelope {
    Envelope {
        attack: fl_env_secs(e.attack),
        hold: if e.hold <= 100 { 0.0 } else { fl_env_secs(e.hold) },
        decay: fl_env_secs(e.decay),
        sustain: (e.sustain as f32 / 128.0).clamp(0.0, 1.0),
        release: fl_env_secs(e.release),
    }
}

/// FL note -> Goaty Loops note. Shared with the FLP writer to detect edits.
pub(crate) fn note_from_flp(n: &FlpNote, channel: usize, scale: f64) -> Note {
    let ticks = |t: u32| (t as f64 * scale).round() as u32;
    Note {
        channel,
        key: n.key.min(127) as u8,
        start: ticks(n.position),
        length: ticks(n.length),
        velocity: (n.velocity as f32 / 128.0).clamp(0.0, 1.0),
        pan: ((n.pan as f32 - 64.0) / 64.0).clamp(-1.0, 1.0),
    }
}

/// Converts a playlist item's start/end offsets, whose unit depends on the
/// source: pattern clips store integer ticks in the f32 field's bits, audio
/// clips milliseconds and automation clips beats. -1 means "not set".
#[derive(Clone, Copy, Debug)]
pub(crate) struct OffsetCodec {
    unit: OffsetUnit,
    /// File ticks per beat.
    ppq: u32,
    tempo: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OffsetUnit {
    Ticks,
    Millis,
    Beats,
}

impl OffsetCodec {
    pub(crate) fn new(source: ClipSource, channels: &[Channel], ppq: u32, tempo: f64) -> Self {
        let unit = match source {
            ClipSource::Pattern(_) => OffsetUnit::Ticks,
            ClipSource::Audio(c) => match channels.get(c).map(|c| &c.instrument) {
                Some(Instrument::Automation(_)) => OffsetUnit::Beats,
                _ => OffsetUnit::Millis,
            },
        };
        Self { unit, ppq: ppq.max(1), tempo: if tempo > 0.0 { tempo } else { 140.0 } }
    }

    /// File value -> Goaty Loops ticks (`None` when unset).
    pub(crate) fn decode(&self, v: f32) -> Option<u32> {
        let ticks = match self.unit {
            OffsetUnit::Ticks => {
                let bits = v.to_bits();
                // -1.0 and other negative floats have the sign bit set.
                (bits < 0x8000_0000).then(|| bits as f64 * PPQ as f64 / self.ppq as f64)?
            }
            OffsetUnit::Millis => (v >= 0.0).then(|| v as f64 / 1000.0 * self.tempo / 60.0 * PPQ as f64)?,
            OffsetUnit::Beats => (v >= 0.0).then(|| v as f64 * PPQ as f64)?,
        };
        Some(ticks.round() as u32)
    }

    /// Goaty Loops ticks -> file value.
    pub(crate) fn encode(&self, ticks: u32) -> f32 {
        let beats = ticks as f64 / PPQ as f64;
        match self.unit {
            OffsetUnit::Ticks => f32::from_bits((beats * self.ppq as f64).round() as u32),
            OffsetUnit::Millis => (beats * 60.0 / self.tempo * 1000.0) as f32,
            OffsetUnit::Beats => beats as f32,
        }
    }

    /// Milliseconds (clip fades) -> Goaty Loops ticks, at the codec's tempo.
    pub(crate) fn ms_to_ticks(&self, ms: f32) -> u32 {
        (ms.max(0.0) as f64 / 1000.0 * self.tempo / 60.0 * PPQ as f64).round() as u32
    }

    pub(crate) fn ticks_to_ms(&self, ticks: u32) -> f32 {
        (ticks as f64 / PPQ as f64 * 60.0 / self.tempo * 1000.0) as f32
    }

    /// Start and end offsets for a clip; untrimmed pattern clips stay unset.
    pub(crate) fn encode_clip(&self, c: &Clip) -> (f32, f32) {
        if self.unit == OffsetUnit::Ticks && c.offset == 0 {
            return (-1.0, -1.0);
        }
        (self.encode(c.offset), self.encode(c.offset + c.length))
    }
}

/// FL playlist item -> clip. Shared with the FLP writer to detect edits.
pub(crate) fn clip_from_flp(it: &FlpPlaylistItem, source: ClipSource, scale: f64, offsets: OffsetCodec) -> Clip {
    let ticks = |t: u32| (t as f64 * scale).round() as u32;
    let offset = offsets.decode(it.start_offset).unwrap_or(0);
    Clip { track: it.track as usize, start: ticks(it.position), length: ticks(it.length).max(1), offset, source, muted: it.muted, fade_in: offsets.ms_to_ticks(it.fade_in), fade_out: offsets.ms_to_ticks(it.fade_out), gain: if matches!(source, ClipSource::Audio(_)) { it.gain } else { 1.0 } }
}

#[cfg(test)]
mod offset_tests {
    use super::*;

    #[test]
    fn offsets_per_source() {
        let mut auto = Channel::new("auto", Instrument::Automation(Default::default()));
        auto.fl_iid = Some(1);
        let channels = vec![Channel::new("audio", Instrument::Sampler(Default::default())), auto];
        // Pattern clips: integer ticks in the float's bits, at the file's PPQ.
        let pat = OffsetCodec::new(ClipSource::Pattern(0), &channels, 48, 120.0);
        assert_eq!(pat.decode(f32::from_bits(24)), Some(48));
        assert_eq!(pat.decode(-1.0), None);
        assert_eq!(pat.encode(48).to_bits(), 24);
        // Audio clips: milliseconds (500 ms = one beat at 120 BPM).
        let audio = OffsetCodec::new(ClipSource::Audio(0), &channels, 96, 120.0);
        assert_eq!(audio.decode(500.0), Some(PPQ));
        assert_eq!(audio.encode(PPQ), 500.0);
        // Automation clips: beats.
        let a = OffsetCodec::new(ClipSource::Audio(1), &channels, 96, 120.0);
        assert_eq!(a.decode(35.0), Some(35 * PPQ));
        assert_eq!(a.encode(2 * PPQ), 2.0);
    }
}

/// Decodes 3x Osc settings: after the version, 7 fields per oscillator
/// (pan, shape, coarse semitones, fine cents, phase, detune, mix level).
/// Oscillators 2 and 3 are mixed relative to oscillator 1 (level 0..=128).
pub(crate) fn three_osc(data: &[u8]) -> Option<[Osc; 3]> {
    let get = |i: usize| data.get(i * 4..i * 4 + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let wave = |v: i32| match v {
        1 => Wave::Triangle,
        2 => Wave::Square,
        3 | 4 => Wave::Saw, // saw, rounded saw
        5 => Wave::Noise,
        _ => Wave::Sine, // sine, custom sample
    };
    let osc = |k: usize, level: f32| -> Option<Osc> {
        let base = 1 + k * 7;
        Some(Osc { wave: wave(get(base + 1)?), coarse: get(base + 2)?.clamp(-48, 48), fine: get(base + 3)?.clamp(-100, 100) as f32, level })
    };
    const BASE_LEVEL: f32 = 0.5;
    let mix = |field: usize| Some(BASE_LEVEL * (get(field)? as f32 / 128.0).clamp(0.0, 1.0));
    Some([osc(0, BASE_LEVEL)?, osc(1, mix(14)?)?, osc(2, mix(21)?)?])
}

#[cfg(test)]
mod three_osc_tests {
    use super::*;

    #[test]
    fn stand_ins_follow_the_instrument_role() {
        let piano = stand_in_synth("FLEX", "Acoustic Piano");
        let bass = stand_in_synth("FLEX", "FLEX Bass");
        assert_eq!(piano.env.sustain, 0.0); // decays like a struck string
        assert!(bass.cutoff < 1_500.0 && bass.osc[0].coarse == -12);
        assert_eq!(piano.replaces.as_deref(), Some("FLEX"));
        let unknown = stand_in_synth("Mystery", "Channel 7");
        assert_eq!(unknown.cutoff, SynthParams::default().cutoff);
    }

    #[test]
    fn decodes_oscillators() {
        // Version, then osc 1: saw at -24; osc 2: square at -12, +5 cents, full mix;
        // osc 3: sine at 0, half mix.
        let mut f = vec![14i32, 0, 3, -24, 0, 0, 0, 0, 0, 2, -12, 5, 0, 0, 128, 0, 0, 0, 0, 0, 0, 64, 0];
        f.resize(23, 0);
        let data: Vec<u8> = f.iter().flat_map(|v| v.to_le_bytes()).collect();
        let [a, b, c] = three_osc(&data).unwrap();
        assert_eq!((a.wave, a.coarse), (Wave::Saw, -24));
        assert_eq!((b.wave, b.coarse, b.fine, b.level), (Wave::Square, -12, 5.0, 0.5));
        assert_eq!((c.wave, c.coarse, c.level), (Wave::Sine, 0, 0.25));
        assert!(three_osc(&data[..40]).is_none());
    }
}

/// A Goaty Synth voiced for the role of an instrument that can't be loaded,
/// guessed from the plugin and channel names (e.g. a FLEX "Acoustic Piano").
pub(crate) fn stand_in_synth(plugin: &str, channel: &str) -> SynthParams {
    let name = format!("{plugin} {channel}").to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    let osc = |wave, coarse, fine, level| Osc { wave, coarse, fine, level };
    let env = |attack, decay, sustain, release| Envelope { attack, hold: 0.0, decay, sustain, release };
    let (osc, env, cutoff, resonance) = if has(&["808", "sub"]) {
        ([osc(Wave::Sine, -12, 0.0, 0.8), osc(Wave::Sine, 0, 0.0, 0.15), osc(Wave::Triangle, -12, 0.0, 0.1)], env(0.003, 1.2, 0.0, 0.3), 2_000.0, 0.0)
    } else if has(&["bass"]) {
        ([osc(Wave::Saw, -12, 0.0, 0.5), osc(Wave::Square, -12, 6.0, 0.3), osc(Wave::Sine, -24, 0.0, 0.5)], env(0.003, 0.4, 0.6, 0.12), 900.0, 0.25)
    } else if has(&["piano", "keys", "rhodes", "epiano"]) {
        ([osc(Wave::Triangle, 0, 0.0, 0.5), osc(Wave::Sine, 12, 0.0, 0.25), osc(Wave::Saw, 0, 3.0, 0.08)], env(0.002, 1.6, 0.0, 0.35), 4_000.0, 0.05)
    } else if has(&["guitar", "pluck", "harp", "koto", "nylon"]) {
        ([osc(Wave::Saw, 0, 0.0, 0.35), osc(Wave::Triangle, 0, 4.0, 0.35), osc(Wave::Square, 12, 0.0, 0.08)], env(0.001, 0.7, 0.0, 0.25), 3_000.0, 0.15)
    } else if has(&["bell", "mallet", "marimba", "vibra", "glock"]) {
        ([osc(Wave::Sine, 0, 0.0, 0.5), osc(Wave::Sine, 24, 0.0, 0.25), osc(Wave::Triangle, 19, 0.0, 0.1)], env(0.001, 1.2, 0.0, 0.8), 8_000.0, 0.0)
    } else if has(&["pad", "string", "choir", "atmos", "ambient"]) {
        ([osc(Wave::Saw, 0, 0.0, 0.3), osc(Wave::Saw, 0, 9.0, 0.3), osc(Wave::Triangle, -12, 0.0, 0.25)], env(0.35, 0.5, 0.8, 1.2), 2_500.0, 0.1)
    } else if has(&["lead", "synth", "saw"]) {
        ([osc(Wave::Saw, 0, 0.0, 0.45), osc(Wave::Saw, 0, 8.0, 0.35), osc(Wave::Square, -12, 0.0, 0.15)], env(0.005, 0.3, 0.75, 0.2), 6_000.0, 0.2)
    } else {
        let d = SynthParams::default();
        (d.osc, d.env, d.cutoff, d.resonance)
    };
    SynthParams { osc, env, cutoff, resonance, replaces: Some(plugin.to_string()) }
}

/// FL time marker -> Goaty Loops marker; the signature is kept for signature
/// markers only (FL writes the song's signature on every marker).
pub(crate) fn marker_from_flp(m: &goaty_flp::FlpTimeMarker, scale: f64) -> TimeMarker {
    let sig = m.kind == TimeMarker::SIGNATURE && m.numerator > 0 && m.denominator > 0;
    TimeMarker {
        tick: (m.position as f64 * scale).round() as u32,
        name: m.name.clone().unwrap_or_default(),
        kind: m.kind,
        numerator: if sig { m.numerator } else { 0 },
        denominator: if sig { m.denominator } else { 0 },
    }
}
