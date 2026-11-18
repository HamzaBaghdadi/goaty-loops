//! Round-trip-safe FLP writer: applies a project's edits to the FLP it was
//! imported from, leaving every event Goaty Loops doesn't model untouched.
//!
//! Each piece of data is compared with what the importer produced from the
//! original; only real changes are written. An unmodified import therefore
//! saves byte-for-byte identical to the original file.

use crate::import_flp::{clip_from_flp, gain_to_fl_fader, note_from_flp, OffsetCodec};
use crate::model::*;
use goaty_flp::{
    encode_text, ids, parse_playlist, parse_version, playlist_item_size, write_item_fades, write_item_gain, write_playlist_item, Event, EventValue, FlpFile, FlpItemSource, FlpNote,
    FlpPlaylistItem, FlpProject, LEVEL_MAX, NOTE_SIZE, PAN_CENTER,
};
use std::collections::{HashMap, HashSet};

pub struct PatchResult {
    pub file: FlpFile,
    pub warnings: Vec<String>,
}

struct Ctx<'a> {
    project: &'a Project,
    /// FL channel iid -> Goaty Loops channel index.
    ch_by_iid: HashMap<u16, usize>,
    /// Goaty Loops pattern index -> FL pattern iid (existing or newly assigned).
    pat_iid: Vec<u16>,
    /// FL pattern iid -> Goaty Loops pattern index.
    pat_by_iid: HashMap<u16, usize>,
    scale_in: f64,
    /// Tempo of the original file (audio clip offsets are in milliseconds).
    tempo_in: f64,
    /// Channels added in Goaty Loops that get written into the file, with their new FL ids.
    new_ch_iid: HashMap<usize, u16>,
    ppq: u16,
    utf16: bool,
}

impl Ctx<'_> {
    fn to_file_ticks(&self, t: u32) -> u32 {
        (t as f64 / self.scale_in).round() as u32
    }

    fn marker_from_flp(&self, m: &goaty_flp::FlpTimeMarker) -> TimeMarker {
        crate::import_flp::marker_from_flp(m, self.scale_in)
    }

    fn offsets(&self, source: ClipSource, tempo: f64) -> OffsetCodec {
        OffsetCodec::new(source, &self.project.channels, self.ppq as u32, tempo)
    }

    fn text(&self, s: &str) -> Vec<u8> {
        if self.utf16 {
            encode_text(s)
        } else {
            s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).chain(std::iter::once(0)).collect()
        }
    }

    fn channel_iid(&self, ch: usize) -> Option<u16> {
        self.project.channels.get(ch)?.fl_iid.or_else(|| self.new_ch_iid.get(&ch).copied())
    }

    /// Notes of a pattern as stored in the original, converted like the importer does.
    fn convert_notes(&self, notes: &[FlpNote]) -> Vec<Note> {
        notes
            .iter()
            .filter_map(|n| self.ch_by_iid.get(&n.rack_channel).map(|&c| note_from_flp(n, c, self.scale_in)))
            .collect()
    }

    fn encode_notes(&self, notes: &[Note], originals: &[FlpNote]) -> Vec<u8> {
        // Reuse the original note's extra fields (fine pitch, release, mod X/Y...) when it still exists.
        let mut by_key: HashMap<(usize, u32, u8), &FlpNote> = HashMap::new();
        for n in originals {
            if let Some(&c) = self.ch_by_iid.get(&n.rack_channel) {
                by_key.entry((c, note_from_flp(n, c, self.scale_in).start, n.key.min(127) as u8)).or_insert(n);
            }
        }
        let mut out = Vec::with_capacity(notes.len() * NOTE_SIZE);
        for n in notes {
            let Some(iid) = self.channel_iid(n.channel) else { continue };
            let base = by_key.get(&(n.channel, n.start, n.key)).map(|o| **o).unwrap_or(FlpNote {
                fine_pitch: 120,
                release: 64,
                mod_x: 128,
                mod_y: 128,
                ..Default::default()
            });
            let fl = FlpNote {
                position: self.to_file_ticks(n.start),
                flags: if n.length == 0 { base.flags | 0x4000 } else { base.flags & !0x4000 },
                rack_channel: iid,
                length: self.to_file_ticks(n.length),
                key: n.key as u16,
                pan: (n.pan * 64.0 + 64.0).round().clamp(0.0, 128.0) as u8,
                velocity: (n.velocity * 128.0).round().clamp(0.0, 128.0) as u8,
                ..base
            };
            fl.write(&mut out);
        }
        out
    }

    fn convert_clips(&self, items: &[FlpPlaylistItem]) -> Vec<Clip> {
        items
            .iter()
            .filter_map(|it| {
                let source = match it.source {
                    FlpItemSource::Pattern(iid) => ClipSource::Pattern(*self.pat_by_iid.get(&iid)?),
                    FlpItemSource::Channel(iid) => ClipSource::Audio(*self.ch_by_iid.get(&iid)?),
                };
                Some(clip_from_flp(it, source, self.scale_in, self.offsets(source, self.tempo_in)))
            })
            .collect()
    }

    fn encode_clips(&self, original: &[u8], major: u32, warnings: &mut Vec<String>) -> Vec<u8> {
        let size = playlist_item_size(original, major);
        let items = parse_playlist(original, major);
        // Raw bytes of each original item, keyed by its converted clip, to keep unknown fields.
        let mut raw: HashMap<(usize, u32, ClipSource), &[u8]> = HashMap::new();
        for (it, chunk) in items.iter().zip(original.chunks_exact(size)) {
            if let Some(c) = self.convert_clips(std::slice::from_ref(it)).first() {
                raw.entry((c.track, c.start, c.source)).or_insert(chunk);
            }
        }
        let tail_template: Vec<u8> = original.chunks_exact(size).next().map(|c| c[32.min(c.len())..].to_vec()).unwrap_or_else(|| vec![0; size.saturating_sub(32)]);
        let mut dropped = 0;
        let mut out = Vec::new();
        for c in &self.project.playlist.clips {
            let source = match c.source {
                ClipSource::Pattern(p) => FlpItemSource::Pattern(self.pat_iid[p]),
                ClipSource::Audio(ch) => match self.channel_iid(ch) {
                    Some(iid) => FlpItemSource::Channel(iid),
                    None => {
                        dropped += 1;
                        continue;
                    }
                },
            };
            let original = raw.get(&(c.track, c.start, c.source)).copied();
            let orig_item = original.map(|b| parse_playlist(b, major)[0]);
            // Keep the original offsets bit-exact while the trim is unchanged.
            let same_trim = orig_item.filter(|o| {
                let before = clip_from_flp(o, c.source, self.scale_in, self.offsets(c.source, self.tempo_in));
                (before.offset, before.length) == (c.offset, c.length)
            });
            let (start_offset, end_offset) = match same_trim {
                Some(o) => (o.start_offset, o.end_offset),
                None => self.offsets(c.source, self.project.tempo).encode_clip(c),
            };
            let item = FlpPlaylistItem {
                position: self.to_file_ticks(c.start),
                length: self.to_file_ticks(c.length),
                track: c.track.min(499) as u16,
                source,
                start_offset,
                end_offset,
                muted: c.muted,
                fade_in: 0.0,
                fade_out: 0.0,
                gain: c.gain,
            };
            let mut bytes = Vec::with_capacity(size);
            write_playlist_item(&item, &mut bytes);
            match original {
                // Keep the original's unknown bytes (group, flags, FL 21+ extras).
                Some(o) => {
                    let mut merged = o.to_vec();
                    merged[..4].copy_from_slice(&bytes[..4]);
                    merged[6..14].copy_from_slice(&bytes[6..14]);
                    merged[18..20].copy_from_slice(&bytes[18..20]);
                    merged[24..32].copy_from_slice(&bytes[24..32]);
                    let before = orig_item.map(|o| clip_from_flp(&o, c.source, self.scale_in, self.offsets(c.source, self.tempo_in)));
                    if before.is_some_and(|b| (b.fade_in, b.fade_out) != (c.fade_in, c.fade_out)) {
                        let codec = self.offsets(c.source, self.project.tempo);
                        write_item_fades(&mut merged, codec.ticks_to_ms(c.fade_in), codec.ticks_to_ms(c.fade_out));
                    }
                    if before.is_some_and(|b| (b.gain - c.gain).abs() > 1e-6) {
                        write_item_gain(&mut merged, c.gain);
                    }
                    out.extend_from_slice(&merged);
                }
                None => {
                    bytes.extend_from_slice(&tail_template[..size.saturating_sub(32).min(tail_template.len())]);
                    bytes.resize(size, 0);
                    // The tail comes from another item: set this clip's own fades.
                    let codec = self.offsets(c.source, self.project.tempo);
                    write_item_fades(&mut bytes, codec.ticks_to_ms(c.fade_in), codec.ticks_to_ms(c.fade_out));
                    if matches!(c.source, ClipSource::Audio(_)) {
                        write_item_gain(&mut bytes, c.gain);
                    }
                    out.extend_from_slice(&bytes);
                }
            }
        }
        if dropped > 0 {
            warnings.push(format!("{dropped} playlist clip(s) use channels added in Goaty Loops and were not saved"));
        }
        out
    }
}

/// Applies `project`'s edits to `original` (the FLP it was imported from).
pub fn patch_flp(original: &FlpFile, project: &Project) -> PatchResult {
    let mut warnings = Vec::new();
    let flp = FlpProject::from_file(original);
    let origin = project.fl_origin.clone();
    let ppq = origin.as_ref().map(|o| o.ppq).unwrap_or(original.ppq).max(1);
    let (major, minor) = flp.version.as_deref().map(parse_version).unwrap_or((20, 0));
    let utf16 = flp.version.is_none() || (major, minor) >= (11, 5);

    let ch_by_iid: HashMap<u16, usize> = project.channels.iter().enumerate().filter_map(|(i, c)| c.fl_iid.map(|iid| (iid, i))).collect();
    // Channels added in Goaty Loops: samplers are written as copies of an FL
    // sampler channel from this file; other kinds can't be expressed yet.
    let template = sampler_template(original);
    let mut next_ch = original.events.iter().filter(|e| e.id == ids::CH_NEW).filter_map(|e| e.as_u32()).max().map(|v| v as u16 + 1).unwrap_or(0);
    let mut new_ch_iid: HashMap<usize, u16> = HashMap::new();
    let mut skipped = Vec::new();
    for (i, c) in project.channels.iter().enumerate().filter(|(_, c)| c.fl_iid.is_none()) {
        let writable = matches!(&c.instrument, Instrument::Sampler(s) if s.path.is_some()) && template.is_some() && channel_insert_point(original).is_some();
        if writable {
            new_ch_iid.insert(i, next_ch);
            next_ch += 1;
        } else {
            skipped.push(c.name.clone());
        }
    }
    if !skipped.is_empty() {
        warnings.push(format!(
            "{} channel(s) added in Goaty Loops can't be written into the FLP yet (only sample channels can); their notes and clips were skipped: {}",
            skipped.len(),
            skipped.join(", ")
        ));
    }
    // New pattern ids skip every id in use, including empty patterns FL doesn't store.
    let mut next_pat = flp.patterns.iter().map(|p| p.iid).chain(project.patterns.iter().filter_map(|p| p.fl_iid)).max().unwrap_or(0) + 1;
    let pat_iid: Vec<u16> = project
        .patterns
        .iter()
        .map(|p| {
            p.fl_iid.unwrap_or_else(|| {
                next_pat += 1;
                next_pat - 1
            })
        })
        .collect();
    let pat_by_iid: HashMap<u16, usize> = pat_iid.iter().enumerate().map(|(i, &iid)| (iid, i)).collect();
    let ctx = Ctx { project, ch_by_iid, pat_iid, pat_by_iid, scale_in: PPQ as f64 / ppq as f64, tempo_in: flp.tempo, new_ch_iid, ppq, utf16 };

    let original_patterns: HashMap<u16, &goaty_flp::FlpPattern> = flp.patterns.iter().map(|p| (p.iid, p)).collect();
    let target_arrangement = origin.as_ref().and_then(|o| o.arrangement);

    // Patterns that have notes now but no notes event in the original get one after their first PAT_NEW.
    let had_notes: HashSet<u16> = {
        let mut s = HashSet::new();
        let mut cur = None;
        for e in &original.events {
            match e.id {
                ids::PAT_NEW => cur = e.as_u32().map(|v| v as u16),
                ids::PAT_NOTES => {
                    if let Some(c) = cur {
                        s.insert(c);
                    }
                }
                ids::CH_NEW | ids::ARR_NEW => cur = None,
                _ => {}
            }
        }
        s
    };
    // New patterns go after the last pattern event, or before the first channel if there are none.
    let last_pattern_event = original
        .events
        .iter()
        .rposition(|e| matches!(e.id, ids::PAT_NEW | ids::PAT_NOTES | ids::PAT_NAME | ids::PAT_COLOR))
        .or_else(|| original.events.iter().position(|e| e.id == ids::CH_NEW).map(|i| i.saturating_sub(1)));

    let mut out: Vec<Event> = Vec::with_capacity(original.events.len() + 16);
    let mut cur_ch: Option<usize> = None;
    let mut cur_pat: Option<u16> = None;
    let mut cur_arr: Option<u16> = None;
    let mut seen_pat_new: HashSet<u16> = HashSet::new();
    let mut playlist_done = false;
    let mut pending_markers = false;
    // Time markers: rewritten as a set (after the playlist event) only when edited.
    let target_iid = target_arrangement.unwrap_or(0);
    let original_markers: Vec<TimeMarker> = flp
        .arrangements
        .iter()
        .find(|a| a.iid == target_iid)
        .or(flp.arrangements.first())
        .map(|a| a.markers.iter().map(|m| ctx.marker_from_flp(m)).collect())
        .unwrap_or_default();
    let markers_changed = original_markers != project.playlist.markers;
    let mut in_marker = false;

    let mut insert_idx = 0usize;

    // Mixer routing: inserts whose outputs or sends changed get new route flags
    // and send levels; untouched inserts keep their bytes.
    let mut route_bytes: Vec<Option<Vec<u8>>> = vec![None; flp.inserts.len()];
    let mut send_levels: Vec<Option<Vec<(usize, i32)>>> = vec![None; flp.inserts.len()];
    for (t, ins) in flp.inserts.iter().enumerate().skip(1) {
        let Some(track) = project.mixer.get(t) else { continue };
        let raw = |sends: &[MixerSend]| -> Vec<(usize, i32)> {
            let mut v: Vec<(usize, i32)> = sends.iter().map(|s| (s.to, gain_to_fl_fader(s.gain))).collect();
            v.sort_unstable(); // order doesn't matter to FL
            v
        };
        let (out0, sends0) = crate::import_flp::routing_from_flp(t, ins).unwrap_or((0, Vec::new()));
        if (out0, raw(&sends0)) == (track.output, raw(&track.sends)) {
            continue;
        }
        let dests: Vec<usize> = std::iter::once(track.output).chain(track.sends.iter().map(|s| s.to)).filter(|&d| d != t && d < flp.inserts.len()).collect();
        let mut bytes = vec![0u8; dests.iter().max().map(|m| m + 1).unwrap_or(1)];
        for d in dests {
            bytes[d] = 1;
        }
        route_bytes[t] = Some(bytes);
        // The main output is at full level (it may have been a send before).
        let mut levels = raw(&track.sends);
        levels.push((track.output, gain_to_fl_fader(1.0)));
        send_levels[t] = Some(levels);
    }

    let channel_at = if ctx.new_ch_iid.is_empty() { None } else { channel_insert_point(original) };

    for (i, e) in original.events.iter().enumerate() {
        let mut ev = e.clone();
        if Some(i) == channel_at {
            let template = template.as_ref().expect("checked when ids were assigned");
            let mut added: Vec<(&usize, &u16)> = ctx.new_ch_iid.iter().collect();
            added.sort();
            for (&ch, &iid) in added {
                out.extend(new_channel_block(&ctx, template, &project.channels[ch], iid));
            }
        }
        // Old files keep every marker in their one implicit arrangement.
        let in_target = if flp.has_arrangements { cur_arr == Some(target_iid) } else { true };
        if markers_changed && in_target {
            // Drop the original markers: a position event and the fields after it.
            let field = in_marker && (matches!(e.id, ids::MARKER_NUM | ids::MARKER_DEN | ids::MARKER_NAME) || ids::MARKER_EXTRA.contains(&e.id));
            in_marker = e.id == ids::MARKER_POS || field;
            if in_marker {
                continue;
            }
        }
        match e.id {
            ids::CH_NEW => {
                cur_ch = e.as_u32().and_then(|iid| ctx.ch_by_iid.get(&(iid as u16)).copied());
                cur_pat = None;
            }
            ids::PAT_NEW => {
                cur_pat = e.as_u32().map(|v| v as u16);
                cur_ch = None;
            }
            ids::ARR_NEW => {
                cur_arr = e.as_u32().map(|v| v as u16);
                cur_ch = None;
                cur_pat = None;
            }
            ids::TEMPO => {
                if (e.as_u32().unwrap_or(0) as f64 / 1000.0 - project.tempo).abs() > 1e-6 {
                    ev.value = EventValue::Dword((project.tempo * 1000.0).round() as u32);
                }
            }
            ids::CH_LEVELS => {
                if let (Some(c), Some(b)) = (cur_ch.and_then(|c| project.channels.get(c)), e.blob()) {
                    if b.len() >= 12 {
                        let mut nb = b.to_vec();
                        let rd = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                        let pan = (c.pan * PAN_CENTER + PAN_CENTER).round() as i32;
                        let vol = (c.volume * LEVEL_MAX).round() as i32;
                        let pitch = (c.pitch * 100.0).round() as i32;
                        // Keep originals that convert to the same values (avoids rounding churn).
                        if ((rd(0) as f32 - PAN_CENTER) / PAN_CENTER - c.pan).abs() > 1e-4 {
                            nb[0..4].copy_from_slice(&pan.to_le_bytes());
                        }
                        if (rd(4) as f32 / LEVEL_MAX - c.volume).abs() > 1e-4 {
                            nb[4..8].copy_from_slice(&vol.to_le_bytes());
                        }
                        if rd(8) != pitch && ((rd(8) as f32 / 100.0) - c.pitch).abs() > 1e-4 {
                            nb[8..12].copy_from_slice(&pitch.to_le_bytes());
                        }
                        ev.value = EventValue::Blob(nb);
                    }
                }
            }
            ids::CH_ENABLED => {
                if let Some(c) = cur_ch.and_then(|c| project.channels.get(c)) {
                    let enabled = !c.muted;
                    if (e.as_u32().unwrap_or(1) != 0) != enabled {
                        ev.value = EventValue::Byte(enabled as u8);
                    }
                }
            }
            ids::CH_PARAMS => {
                let blob = e.blob().filter(|b| b.len() >= goaty_flp::STRETCH_LAYOUT_LEN);
                if let (Some(Instrument::Sampler(s)), Some(b)) = (cur_ch.and_then(|c| project.channels.get(c)).map(|c| &c.instrument), blob) {
                    if let Some(w) = b.get(96..100) {
                        let v = f32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                        // Same reading as the importer; only rewrite when the stretch changed.
                        let before = if v.is_normal() && v > 0.0 { v / 192.0 } else { 0.0 };
                        let mut nb = b.to_vec();
                        if (before - s.stretch_beats).abs() > 1e-4 {
                            nb[96..100].copy_from_slice(&(s.stretch_beats.max(0.0) * 192.0).to_le_bytes());
                        }
                        let i32_at = |o: usize| b.get(o..o + 4).map(|w| i32::from_le_bytes([w[0], w[1], w[2], w[3]]));
                        if i32_at(100).is_some_and(|v| v.clamp(-4800, 4800) != s.stretch_pitch.round() as i32) {
                            nb[100..104].copy_from_slice(&(s.stretch_pitch.round() as i32).to_le_bytes());
                        }
                        // Keep pitch = any mode but resample (0); turning it on picks "Auto" (5).
                        if i32_at(108).is_some_and(|m| (m != 0) != s.keep_pitch) {
                            nb[108..112].copy_from_slice(&(if s.keep_pitch { 5i32 } else { 0 }).to_le_bytes());
                        }
                        if nb != b {
                            ev.value = EventValue::Blob(nb);
                        }
                    }
                }
            }
            ids::CH_PREAMP => {
                if let Some(Instrument::Sampler(s)) = cur_ch.and_then(|c| project.channels.get(c)).map(|c| &c.instrument) {
                    let before = 1.0 + e.as_u32().unwrap_or(0) as f32 / 16.0;
                    if (before - s.preamp).abs() > 1e-4 {
                        ev.value = EventValue::Word(((s.preamp - 1.0) * 16.0).round().max(0.0) as u16);
                    }
                }
            }
            ids::CH_FX_FLAGS => {
                if let Some(Instrument::Sampler(s)) = cur_ch.and_then(|c| project.channels.get(c)).map(|c| &c.instrument) {
                    let v = e.as_u32().unwrap_or(0) as u16;
                    if (v & goaty_flp::FX_REVERSE != 0) != s.reverse {
                        ev.value = EventValue::Word(v ^ goaty_flp::FX_REVERSE);
                    }
                }
            }
            ids::CH_INSERT | ids::CH_ROUTED_TO => {
                if let Some(c) = cur_ch.and_then(|c| project.channels.get(c)) {
                    // FL 20+ writes a word (CH_INSERT), older files a byte (CH_ROUTED_TO).
                    let v = e.as_u32().unwrap_or(0);
                    let signed = if e.id == ids::CH_INSERT { v as u16 as i16 as i32 } else { v as u8 as i8 as i32 };
                    // Same clamping as the importer, so an untouched channel stays byte-identical.
                    let orig = signed.clamp(0, project.mixer.len() as i32 - 1) as usize;
                    if orig != c.mixer_track {
                        ev.value = if e.id == ids::CH_INSERT { EventValue::Word(c.mixer_track as u16) } else { EventValue::Byte(c.mixer_track as u8) };
                    }
                }
            }
            ids::PAT_NOTES => {
                if let (Some(iid), Some(b)) = (cur_pat, e.blob()) {
                    if let Some(&pi) = ctx.pat_by_iid.get(&iid) {
                        let originals: Vec<FlpNote> = b.chunks_exact(NOTE_SIZE).map(FlpNote::parse).collect();
                        let model = &project.patterns[pi].notes;
                        let saveable: Vec<Note> = model.iter().filter(|n| ctx.channel_iid(n.channel).is_some()).copied().collect();
                        if ctx.convert_notes(&originals) != saveable {
                            ev.value = EventValue::Blob(ctx.encode_notes(&saveable, &originals));
                        }
                    }
                }
            }
            ids::PAT_NAME => {
                if let Some(pi) = cur_pat.and_then(|iid| ctx.pat_by_iid.get(&iid)) {
                    let name = &project.patterns[*pi].name;
                    let orig = cur_pat.and_then(|iid| original_patterns.get(&iid)).and_then(|p| p.name.clone());
                    if orig.as_deref() != Some(name.as_str()) {
                        ev.value = EventValue::Blob(ctx.text(name));
                    }
                }
            }
            ids::ARR_PLAYLIST => {
                let is_target = match target_arrangement {
                    // Files before FL 12.9 have no arrangement events: that playlist is arrangement 0.
                    Some(a) => cur_arr.unwrap_or(0) == a,
                    None => !playlist_done,
                };
                if is_target && !playlist_done {
                    playlist_done = true;
                    if markers_changed {
                        pending_markers = true;
                    }
                    if let Some(b) = e.blob() {
                        let items = parse_playlist(b, major);
                        if ctx.convert_clips(&items) != project.playlist.clips {
                            ev.value = EventValue::Blob(ctx.encode_clips(b, major, &mut warnings));
                        }
                    }
                }
            }
            ids::INS_ROUTING => {
                if let Some(Some(bytes)) = route_bytes.get(insert_idx) {
                    ev.value = EventValue::Blob(bytes.clone());
                }
            }
            ids::INS_OUTPUT => insert_idx += 1,
            ids::MIXER_PARAMS => {
                if let Some(b) = e.blob() {
                    ev.value = EventValue::Blob(patch_mixer_params(b, project, &send_levels));
                }
            }
            _ => {}
        }
        out.push(ev);
        if pending_markers {
            pending_markers = false;
            for m in &project.playlist.markers {
                out.push(Event::new(ids::MARKER_POS, EventValue::Dword((m.kind as u32) << 24 | (ctx.to_file_ticks(m.tick) & 0x00ff_ffff))));
                let (num, den) = m.signature().unwrap_or(project.time_sig);
                out.push(Event::new(ids::MARKER_NUM, EventValue::Byte(num)));
                out.push(Event::new(ids::MARKER_DEN, EventValue::Byte(den)));
                out.push(Event::new(ids::MARKER_EXTRA[0], EventValue::Byte(0)));
                out.push(Event::new(ids::MARKER_EXTRA[1], EventValue::Byte(0)));
                out.push(Event::new(ids::MARKER_EXTRA[2], EventValue::Dword(0)));
                out.push(Event::new(ids::MARKER_EXTRA[3], EventValue::Word(0)));
                out.push(Event::new(ids::MARKER_NAME, EventValue::Blob(ctx.text(&m.name))));
            }
        }

        // A pattern that gained notes needs a notes event after its first PAT_NEW.
        if e.id == ids::PAT_NEW {
            if let Some(iid) = cur_pat {
                if seen_pat_new.insert(iid) && !had_notes.contains(&iid) {
                    if let Some(&pi) = ctx.pat_by_iid.get(&iid) {
                        let notes: Vec<Note> = project.patterns[pi].notes.iter().filter(|n| ctx.channel_iid(n.channel).is_some()).copied().collect();
                        if !notes.is_empty() {
                            out.push(Event::new(ids::PAT_NOTES, EventValue::Blob(ctx.encode_notes(&notes, &[]))));
                        }
                    }
                }
            }
        }
        // New patterns go after the original pattern section.
        if Some(i) == last_pattern_event {
            let used = |pi: usize| project.playlist.clips.iter().any(|c| c.source == ClipSource::Pattern(pi));
            // New patterns, and empty FL patterns (which FL doesn't store) once they get notes.
            let write = |i: usize, p: &Pattern| match p.fl_iid {
                None => !p.notes.is_empty() || used(i),
                Some(iid) => !original_patterns.contains_key(&iid) && !p.notes.is_empty(),
            };
            for (pi, pat) in project.patterns.iter().enumerate().filter(|(i, p)| write(*i, p)) {
                let iid = ctx.pat_iid[pi];
                out.push(Event::new(ids::PAT_NEW, EventValue::Word(iid)));
                out.push(Event::new(ids::PAT_NAME, EventValue::Blob(ctx.text(&pat.name))));
                let notes: Vec<Note> = pat.notes.iter().filter(|n| ctx.channel_iid(n.channel).is_some()).copied().collect();
                if !notes.is_empty() {
                    out.push(Event::new(ids::PAT_NOTES, EventValue::Blob(ctx.encode_notes(&notes, &[]))));
                }
            }
        }
    }
    let _ = (insert_idx, ctx.ppq);
    let channel_count = original.channel_count + ctx.new_ch_iid.len() as u16;
    PatchResult { file: FlpFile { format: original.format, channel_count, ppq: original.ppq, events: out }, warnings }
}

/// Writes mixer volume/pan and slot on/off/mix back into FL's mixer parameter table.
fn patch_mixer_params(b: &[u8], project: &Project, send_levels: &[Option<Vec<(usize, i32)>>]) -> Vec<u8> {
    // The importer uses the last entry for each (insert, slot, id); only rewrite
    // a parameter when the project's value differs from that one.
    let entry = |c: &[u8]| -> Option<(usize, u8, u8, i32)> {
        if c[5] == goaty_flp::ROUTE_LEVEL_MARK {
            return None; // send levels: handled below
        }
        let channel_data = u16::from_le_bytes([c[6], c[7]]);
        let (insert, slot) = goaty_flp::mixer_location(channel_data)?;
        Some((insert, slot, c[4], i32::from_le_bytes([c[8], c[9], c[10], c[11]])))
    };
    let mut last: HashMap<(usize, u8, u8), i32> = HashMap::new();
    for c in b.chunks_exact(12) {
        if let Some((insert, slot, id, msg)) = entry(c) {
            last.insert((insert, slot, id), msg);
        }
    }
    let mut out = b.to_vec();
    for chunk in out.chunks_exact_mut(12) {
        let Some((insert, slot, id, _)) = entry(chunk) else { continue };
        let msg = last[&(insert, slot, id)];
        let Some(track) = project.mixer.get(insert) else { continue };
        let fx = track.effects.iter().find(|e| e.fl.as_ref().is_some_and(|f| f.slot == slot));
        let new = match id {
            0xC0 => {
                let g = crate::import_flp::fl_fader_to_gain(msg);
                ((g - track.volume).abs() > 1e-4 * track.volume.max(1e-3)).then(|| gain_to_fl_fader(track.volume))
            }
            0xC1 => ((msg as f32 / 6400.0).clamp(-1.0, 1.0) - track.pan).abs().gt(&1e-4).then(|| (track.pan * 6400.0).round() as i32),
            0x00 => fx.and_then(|e| ((msg != 0) != e.enabled).then_some(e.enabled as i32)),
            0x01 => fx.and_then(|e| ((msg as f32 / 12800.0).clamp(0.0, 1.0) - e.mix).abs().gt(&1e-4).then(|| (e.mix * 12800.0).round() as i32)),
            _ => None,
        };
        if let Some(v) = new {
            chunk[8..12].copy_from_slice(&v.to_le_bytes());
        }
    }
    patch_send_levels(&mut out, send_levels);
    out
}

/// Sets the send-level entries of inserts whose routing changed
/// (`send_levels[insert]` = wanted (destination, raw level)); entries that
/// don't exist yet are appended, located like the insert's other entries.
fn patch_send_levels(out: &mut Vec<u8>, send_levels: &[Option<Vec<(usize, i32)>>]) {
    let loc_of = |c: &[u8]| u16::from_le_bytes([c[6], c[7]]);
    // FL 24+ locates entries at 0x7000 + insert * 64, older files at 0x2000 + insert * 64.
    let modern = out.chunks_exact(12).any(|c| loc_of(c) >= 0x7000);
    for (insert, wanted) in send_levels.iter().enumerate() {
        let Some(wanted) = wanted else { continue };
        for &(dest, raw) in wanted {
            let existing = out.chunks_exact_mut(12).find(|c| {
                c[5] == goaty_flp::ROUTE_LEVEL_MARK && c[4] as usize == dest && goaty_flp::mixer_location(loc_of(c)).is_some_and(|(i, _)| i == insert)
            });
            match existing {
                Some(c) => c[8..12].copy_from_slice(&raw.to_le_bytes()),
                None => {
                    let loc = (if modern { 0x7000u16 } else { 0x2000 }) + ((insert as u16) << 6);
                    out.extend_from_slice(&[0, 0, 0, 0, dest as u8, goaty_flp::ROUTE_LEVEL_MARK]);
                    out.extend_from_slice(&loc.to_le_bytes());
                    out.extend_from_slice(&raw.to_le_bytes());
                }
            }
        }
    }
}

/// Events of each channel block: CH_NEW up to the next CH_NEW (the last
/// block ends at the arrangement section).
fn channel_blocks(f: &FlpFile) -> Vec<std::ops::Range<usize>> {
    let starts: Vec<usize> = f.events.iter().enumerate().filter(|(_, e)| e.id == ids::CH_NEW).map(|(i, _)| i).collect();
    let end = channel_insert_point(f).unwrap_or(f.events.len());
    starts.iter().enumerate().map(|(k, &s)| s..starts.get(k + 1).copied().unwrap_or(end)).collect()
}

/// Where new channel blocks go: right after the last channel, which is
/// followed by the arrangement section (ARR_NEW, or the playlist in files
/// before FL 12.9).
fn channel_insert_point(f: &FlpFile) -> Option<usize> {
    let last = f.events.iter().rposition(|e| e.id == ids::CH_NEW)?;
    f.events.iter().skip(last).position(|e| matches!(e.id, ids::ARR_NEW | ids::ARR_PLAYLIST)).map(|i| i + last)
}

/// Channel types that play a sample: FL's sampler, then its audio clip channel.
const SAMPLE_CHANNEL_TYPES: [u32; 2] = [0, 4];

/// An FL sample channel's events to copy for new sample channels. Preference:
/// a sampler over an audio clip channel, one that has a sample, and blocks
/// between two channels over the last one (which ends at the arrangement
/// section, so it could carry stray events).
fn sampler_template(f: &FlpFile) -> Option<Vec<Event>> {
    let blocks = channel_blocks(f);
    let n = blocks.len();
    let kind = |r: &std::ops::Range<usize>| f.events[r.clone()].iter().find(|e| e.id == ids::CH_TYPE).and_then(|e| e.as_u32());
    let has_sample = |r: &std::ops::Range<usize>| f.events[r.clone()].iter().any(|e| e.id == ids::CH_SAMPLE_PATH && e.blob().is_some_and(|b| b.len() > 2));
    let pick = SAMPLE_CHANNEL_TYPES.iter().find_map(|&t| {
        let order = (0..n.saturating_sub(1)).chain(n.checked_sub(1));
        let of_type: Vec<&std::ops::Range<usize>> = order.map(|i| &blocks[i]).filter(|r| kind(r) == Some(t)).collect();
        of_type.iter().find(|r| has_sample(r)).or(of_type.first()).copied()
    })?;
    Some(f.events[pick.clone()].to_vec())
}

/// A channel added in Goaty Loops as FL events: a copy of the sampler
/// template with this channel's id, name, sample and settings.
fn new_channel_block(ctx: &Ctx, template: &[Event], ch: &Channel, iid: u16) -> Vec<Event> {
    let Instrument::Sampler(s) = &ch.instrument else { return Vec::new() };
    let path = s.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let mut out = Vec::with_capacity(template.len() + 1);
    let mut has_path = template.iter().any(|e| e.id == ids::CH_SAMPLE_PATH);
    for e in template {
        let mut ev = e.clone();
        match e.id {
            ids::CH_NEW => ev.value = EventValue::Word(iid),
            ids::PLUGIN_NAME | ids::CH_NAME_LEGACY => ev.value = EventValue::Blob(ctx.text(&ch.name)),
            ids::CH_SAMPLE_PATH => ev.value = EventValue::Blob(ctx.text(&path)),
            ids::CH_ENABLED => ev.value = EventValue::Byte(!ch.muted as u8),
            ids::CH_COLOR => ev.value = EventValue::Dword(u32::from_le_bytes([ch.color.0, ch.color.1, ch.color.2, 0])),
            ids::CH_INSERT => ev.value = EventValue::Word(ch.mixer_track as u16),
            ids::CH_ROUTED_TO => ev.value = EventValue::Byte(ch.mixer_track as u8),
            ids::CH_CUT => ev.value = EventValue::Dword(ch.cut_group as u32 | (ch.cut_by as u32) << 16),
            ids::CH_ROOT_NOTE => ev.value = EventValue::Dword(s.root_key as u32),
            ids::CH_PREAMP => ev.value = EventValue::Word(((s.preamp - 1.0) * 16.0).round().max(0.0) as u16),
            ids::CH_FX_FLAGS => {
                let v = e.as_u32().unwrap_or(0) as u16 & !goaty_flp::FX_REVERSE;
                ev.value = EventValue::Word(if s.reverse { v | goaty_flp::FX_REVERSE } else { v });
            }
            ids::CH_LEVELS => {
                if let Some(b) = e.blob().filter(|b| b.len() >= 12) {
                    let mut nb = b.to_vec();
                    nb[0..4].copy_from_slice(&((ch.pan * PAN_CENTER + PAN_CENTER).round() as i32).to_le_bytes());
                    nb[4..8].copy_from_slice(&((ch.volume * LEVEL_MAX).round() as i32).to_le_bytes());
                    nb[8..12].copy_from_slice(&((ch.pitch * 100.0).round() as i32).to_le_bytes());
                    ev.value = EventValue::Blob(nb);
                }
            }
            _ => {}
        }
        let is_name = matches!(e.id, ids::PLUGIN_NAME | ids::CH_NAME_LEGACY);
        out.push(ev);
        // A template without a sample gets the path right after its name.
        if is_name && !has_path {
            out.push(Event::new(ids::CH_SAMPLE_PATH, EventValue::Blob(ctx.text(&path))));
            has_path = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_flp::{convert, to_flp};
    use crate::plugin::PluginDb;

    fn sample() -> FlpFile {
        let mut p = Project::new_template();
        p.patterns[0].toggle_step(0, 0);
        p.patterns[0].toggle_step(1, 4);
        p.playlist.clips.push(Clip { track: 0, start: 0, length: PPQ * 4, offset: 0, source: ClipSource::Pattern(0), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
        let bytes = to_flp(&p).to_flp_file().to_bytes();
        FlpFile::parse(&bytes).unwrap()
    }

    fn import(f: &FlpFile) -> Project {
        convert(&FlpProject::from_file(f), None, &PluginDb::default()).project
    }

    #[test]
    fn unmodified_is_byte_identical() {
        let f = sample();
        let p = import(&f);
        assert_eq!(patch_flp(&f, &p).file.to_bytes(), f.to_bytes());
    }

    #[test]
    fn edits_survive_round_trip() {
        let f = sample();
        let mut p = import(&f);
        p.tempo = 150.0;
        p.patterns[0].toggle_step(2, 8);
        p.playlist.clips[0].start = PPQ * 8;
        p.mixer[0].volume = 0.5;
        p.patterns.push(Pattern::new("New one", PALETTE[1]));
        p.patterns[1].toggle_step(0, 3);
        let out = patch_flp(&f, &p).file;
        let q = import(&FlpFile::parse(&out.to_bytes()).unwrap());
        assert_eq!(q.tempo, 150.0);
        assert!(q.patterns[0].step_on(2, 8));
        assert_eq!(q.playlist.clips[0].start, PPQ * 8);
        assert!((q.mixer[0].volume - 0.5).abs() < 0.01);
        assert!(q.patterns.iter().any(|x| x.name == "New one" && x.step_on(0, 3)));
    }
}
