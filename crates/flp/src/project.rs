use crate::event::*;
use std::collections::BTreeMap;

/// Known event ids. Names follow PyFLP where possible.
pub mod ids {
    use crate::event::{BYTE, DATA, DWORD, TEXT, WORD};

    // Project
    pub const LOOP_ACTIVE: u8 = BYTE + 9;
    pub const TIME_SIG_NUM: u8 = BYTE + 17;
    pub const TIME_SIG_BEAT: u8 = BYTE + 18;
    pub const TEMPO_COARSE: u8 = WORD + 2;
    pub const MAIN_PITCH: u8 = WORD + 16;
    pub const TEMPO_FINE: u8 = WORD + 29;
    pub const TEMPO: u8 = DWORD + 28;
    pub const FL_BUILD: u8 = DWORD + 31;
    pub const TITLE: u8 = TEXT + 2;
    pub const COMMENTS: u8 = TEXT + 3;
    pub const URL: u8 = TEXT + 5;
    pub const FL_VERSION: u8 = TEXT + 7;
    pub const GENRE: u8 = TEXT + 14;
    pub const ARTISTS: u8 = TEXT + 15;

    // Channel
    pub const CH_ENABLED: u8 = BYTE;
    pub const CH_VOL_BYTE: u8 = BYTE + 2;
    pub const CH_PAN_BYTE: u8 = BYTE + 3;
    pub const CH_TYPE: u8 = BYTE + 21;
    pub const CH_ROUTED_TO: u8 = BYTE + 22;
    /// Mixer insert the channel plays into (FL 20+; older files use CH_ROUTED_TO).
    pub const CH_INSERT: u8 = WORD + 40;
    /// Sampler effect flags: 1 fade stereo, 2 reverse, 4 clip, 256 swap stereo.
    pub const CH_FX_FLAGS: u8 = WORD + 6;
    /// Sampler preamp: gain = 1 + value / 16 (measured in FL).
    pub const CH_PREAMP: u8 = WORD + 10;
    /// Channel parameter block (sampler settings and more).
    pub const CH_PARAMS: u8 = DATA + 7;
    pub const CH_NEW: u8 = WORD;
    pub const CH_VOL_WORD: u8 = WORD + 8;
    pub const CH_PAN_WORD: u8 = WORD + 9;
    pub const CH_COLOR: u8 = DWORD;
    pub const CH_ROOT_NOTE: u8 = DWORD + 7;
    pub const CH_NAME_LEGACY: u8 = TEXT;
    pub const CH_SAMPLE_PATH: u8 = TEXT + 4;
    pub const CH_LEVELS: u8 = DATA + 11;
    pub const CH_AUTOMATION: u8 = DATA + 26;
    pub const CH_ENVELOPE: u8 = DATA + 10;
    pub const CH_CUT: u8 = DWORD + 4;
    pub const CONTROLLER: u8 = DATA + 19;

    // Plugins (shared by channels and mixer slots)
    pub const PLUGIN_INTERNAL_NAME: u8 = TEXT + 9;
    pub const PLUGIN_NAME: u8 = TEXT + 11;
    pub const PLUGIN_DATA: u8 = DATA + 5;

    // Patterns
    pub const PAT_NEW: u8 = WORD + 1;
    pub const PAT_COLOR: u8 = DWORD + 22;
    pub const PAT_NAME: u8 = TEXT + 1;
    pub const PAT_NOTES: u8 = DATA + 16;

    // Arrangements / playlist
    pub const ARR_NEW: u8 = WORD + 35;
    pub const ARR_CURRENT: u8 = WORD + 36;
    pub const ARR_PLAYLIST: u8 = DATA + 25;
    pub const ARR_NAME: u8 = TEXT + 49;
    pub const TRACK_DATA: u8 = DATA + 30;
    pub const TRACK_NAME: u8 = TEXT + 47;
    pub const MARKER_POS: u8 = DWORD + 20;
    pub const MARKER_NUM: u8 = BYTE + 33;
    pub const MARKER_DEN: u8 = BYTE + 34;
    pub const MARKER_NAME: u8 = TEXT + 13;
    /// Further marker fields FL writes between the signature and the name
    /// (meaning unknown; written as 0), in file order.
    pub const MARKER_EXTRA: [u8; 4] = [BYTE + 46, BYTE + 45, DWORD + 40, WORD + 37];

    // Mixer
    pub const MIXER_PARAMS: u8 = DATA + 17;
    pub const INS_ROUTING: u8 = DATA + 27;
    pub const INS_FLAGS: u8 = DATA + 28;
    pub const INS_NAME: u8 = TEXT + 12;
    pub const INS_COLOR: u8 = DWORD + 21;
    pub const INS_OUTPUT: u8 = DWORD + 19;
    pub const SLOT_INDEX: u8 = WORD + 34;
}

/// Channel volume/pan raw ranges used by FL.
/// Smallest channel parameter block (`ids::CH_PARAMS`) with the time-stretch fields.
pub const STRETCH_LAYOUT_LEN: usize = 168;

/// Reverse bit of `ids::CH_FX_FLAGS`.
pub const FX_REVERSE: u16 = 2;

pub const LEVEL_MAX: f32 = 12800.0;
pub const PAN_CENTER: f32 = 6400.0;

/// A third-party plugin hosted in FL's "Fruity Wrapper".
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpPluginWrapper {
    pub name: String,
    pub vendor: String,
    /// Path to the plugin binary as stored by FL.
    pub path: String,
    /// VST3 class id (absent for VST2 plugins).
    pub guid: Option<[u8; 16]>,
    pub instrument: bool,
    /// FL's state container (see [`FlpPluginWrapper::vst3_component_state`]).
    pub state: Vec<u8>,
}

impl FlpPluginWrapper {
    /// Parses the plugin data event (DATA+5) of a wrapped plugin.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 4 || u32::from_le_bytes([b[0], b[1], b[2], b[3]]) < 8 {
            return None;
        }
        let mut w = FlpPluginWrapper::default();
        let mut pos = 4;
        let mut any = false;
        while pos + 12 <= b.len() {
            let id = u32::from_le_bytes(b[pos..pos + 4].try_into().ok()?);
            let size = u64::from_le_bytes(b[pos + 4..pos + 12].try_into().ok()?) as usize;
            let data = b.get(pos + 12..pos + 12 + size)?;
            let text = || String::from_utf8_lossy(data).trim_end_matches('\0').to_string();
            match id {
                50 => w.instrument = data.first() == Some(&8),
                52 if size == 16 => w.guid = data.try_into().ok(),
                53 => w.state = data.to_vec(),
                54 => w.name = text(),
                55 => w.path = text(),
                56 => w.vendor = text(),
                _ => {}
            }
            any = true;
            pos += 12 + size;
        }
        (any && !w.name.is_empty()).then_some(w)
    }

    /// The plugin's own component state, extracted from FL's container
    /// (a 0x50-byte header followed by `(u32 id, u64 size, data)` chunks;
    /// chunk 3 holds the component state).
    pub fn vst3_component_state(&self) -> Option<&[u8]> {
        let s = &self.state;
        let mut pos = 0x50;
        while pos + 12 <= s.len() {
            let id = u32::from_le_bytes(s[pos..pos + 4].try_into().ok()?);
            let size = u64::from_le_bytes(s[pos + 4..pos + 12].try_into().ok()?) as usize;
            let data = s.get(pos + 12..pos + 12 + size)?;
            if id == 3 {
                return Some(data);
            }
            pos += 12 + size;
        }
        None
    }

    /// The bundle path (`...\X.vst3`) for paths pointing inside a bundle.
    pub fn bundle_path(&self) -> String {
        let lower = self.path.to_ascii_lowercase();
        match lower.find(".vst3\\contents\\").or_else(|| lower.find(".vst3/contents/")) {
            Some(i) => self.path[..i + 5].to_string(),
            None => self.path.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpSlot {
    /// Display name (or internal plugin name).
    pub name: String,
    /// FL's internal plugin name, e.g. "Fruity Limiter" or "Fruity Wrapper".
    pub internal: String,
    pub wrapper: Option<FlpPluginWrapper>,
    /// Slot position (0-9) in the insert.
    pub index: u8,
    /// Raw parameter data of a native FL plugin.
    pub params: Vec<u8>,
    /// Dry/wet mix, 0..=1 (FL stores 0..12800).
    pub mix: f32,
    /// False when the slot is bypassed in FL.
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlpChannel {
    pub iid: u16,
    /// 0 = sampler, 2 = native plugin, 3 = layer, 4 = instrument, 5 = automation.
    pub kind: u8,
    pub name: Option<String>,
    pub plugin: Option<String>,
    pub sample_path: Option<String>,
    pub color: Option<[u8; 3]>,
    /// 0..=1 where 1 is FL's 100% (raw 12800). FL's default is 0.78.
    pub volume: f32,
    /// -1..=1
    pub pan: f32,
    pub enabled: bool,
    pub mixer_insert: Option<i32>,
    pub root_note: Option<u32>,
    /// Sampler volume envelope, when enabled.
    pub volume_envelope: Option<FlpEnvelope>,
    /// Envelope events seen so far (panning, volume, mod X, mod Y, pitch).
    pub envelope_count: u8,
    /// Cut group this channel triggers, and the group that cuts it (0 = none).
    pub cut: u16,
    pub cut_by: u16,
    /// Channel pitch knob in cents.
    pub pitch_cents: i32,
    /// Sampler effect flags (see `ids::CH_FX_FLAGS`).
    pub fx_flags: u16,
    /// Sampler preamp (see `ids::CH_PREAMP`); 0 = none.
    pub preamp: u16,
    /// Sampler time stretch: the sample plays over this many beats (0 = off).
    pub stretch_beats: f32,
    /// Time-stretch mode: 0 resample (pitch follows speed); others keep the pitch.
    pub stretch_mode: i32,
    /// Time-stretch pitch shift in cents.
    pub stretch_pitch: i32,
    /// Raw parameter data of a native FL plugin.
    pub plugin_params: Vec<u8>,
    /// Points of an automation channel.
    pub automation: Option<Vec<FlpAutoPoint>>,
    /// Third-party plugin data when the channel hosts a VST.
    pub wrapper: Option<FlpPluginWrapper>,
}

impl FlpChannel {
    fn new(iid: u16) -> Self {
        Self {
            iid,
            kind: 0,
            name: None,
            plugin: None,
            sample_path: None,
            color: None,
            volume: 10000.0 / LEVEL_MAX,
            pan: 0.0,
            enabled: true,
            mixer_insert: None,
            root_note: None,
            pitch_cents: 0,
            fx_flags: 0,
            preamp: 0,
            stretch_beats: 0.0,
            stretch_mode: 0,
            stretch_pitch: 0,
            volume_envelope: None,
            envelope_count: 0,
            cut: 0,
            cut_by: 0,
            wrapper: None,
            plugin_params: Vec::new(),
            automation: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FlpNote {
    pub position: u32,
    pub flags: u16,
    pub rack_channel: u16,
    pub length: u32,
    /// 60 = C5 in FL's naming (MIDI middle C).
    pub key: u16,
    pub group: u16,
    pub fine_pitch: u8,
    pub release: u8,
    pub midi_channel: u8,
    /// 0..=128, 64 = centre.
    pub pan: u8,
    /// 0..=128, default 100.
    pub velocity: u8,
    pub mod_x: u8,
    pub mod_y: u8,
}

pub const NOTE_SIZE: usize = 24;

impl FlpNote {
    pub fn parse(c: &[u8]) -> Self {
        let u16_at = |i: usize| u16::from_le_bytes([c[i], c[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
        Self {
            position: u32_at(0),
            flags: u16_at(4),
            rack_channel: u16_at(6),
            length: u32_at(8),
            key: u16_at(12),
            group: u16_at(14),
            fine_pitch: c[16],
            release: c[18],
            midi_channel: c[19],
            pan: c[20],
            velocity: c[21],
            mod_x: c[22],
            mod_y: c[23],
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.position.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.rack_channel.to_le_bytes());
        out.extend_from_slice(&self.length.to_le_bytes());
        out.extend_from_slice(&self.key.to_le_bytes());
        out.extend_from_slice(&self.group.to_le_bytes());
        out.extend_from_slice(&[
            self.fine_pitch,
            0,
            self.release,
            self.midi_channel,
            self.pan,
            self.velocity,
            self.mod_x,
            self.mod_y,
        ]);
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpPattern {
    pub iid: u16,
    pub name: Option<String>,
    pub color: Option<[u8; 3]>,
    pub notes: Vec<FlpNote>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlpItemSource {
    Pattern(u16),
    Channel(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlpPlaylistItem {
    pub position: u32,
    pub length: u32,
    /// 0-based playlist track index.
    pub track: u16,
    pub source: FlpItemSource,
    pub start_offset: f32,
    pub end_offset: f32,
    /// Clip fade-in and fade-out in milliseconds (FL 21+ items; 0 = none).
    pub fade_in: f32,
    pub fade_out: f32,
    /// Linear clip gain (FL 21+ audio clips; 1.0 = 0 dB).
    pub gain: f32,
    pub muted: bool,
}

const PATTERN_BASE: u16 = 20480;
const MAX_TRACKS: u16 = 500;
const ITEM_MUTED: u16 = 1 << 13;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpTrack {
    pub iid: u32,
    pub name: Option<String>,
    pub color: Option<[u8; 3]>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpArrangement {
    pub iid: u16,
    pub name: Option<String>,
    pub items: Vec<FlpPlaylistItem>,
    pub tracks: Vec<FlpTrack>,
    pub markers: Vec<FlpTimeMarker>,
}

/// A playlist time marker.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpTimeMarker {
    /// Position in ticks.
    pub position: u32,
    /// Marker action from the position's top byte (0 = plain marker).
    pub kind: u8,
    pub name: Option<String>,
    /// Time signature, for signature-change markers.
    pub numerator: u8,
    pub denominator: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlpInsert {
    pub name: Option<String>,
    pub color: Option<[u8; 3]>,
    pub enabled: bool,
    /// FL's raw fader value: 0..=12800, 10000 = 0 dB.
    pub volume: Option<i32>,
    /// -6400..=6400
    pub pan: Option<i32>,
    /// Plugin names in the effect slots.
    pub slots: Vec<FlpSlot>,
    /// Destination inserts this insert is routed to (FL can route to several).
    pub routes: Vec<usize>,
    /// Send levels by destination (raw 0..=12800); routes without one are at 12800.
    pub route_levels: Vec<(usize, i32)>,
}

impl Default for FlpInsert {
    fn default() -> Self {
        Self { name: None, color: None, enabled: true, volume: None, pan: None, slots: Vec::new(), routes: Vec::new(), route_levels: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlpProject {
    pub version: Option<String>,
    pub ppq: u16,
    pub tempo: f64,
    pub time_sig: (u8, u8),
    pub title: String,
    pub artists: String,
    pub genre: String,
    pub comments: String,
    pub channels: Vec<FlpChannel>,
    pub patterns: Vec<FlpPattern>,
    pub arrangements: Vec<FlpArrangement>,
    pub current_arrangement: Option<u16>,
    /// Time markers stored before any arrangement event.
    pub legacy_markers: Vec<FlpTimeMarker>,
    /// Whether the file has arrangement events (FL 12.9+).
    pub has_arrangements: bool,
    pub inserts: Vec<FlpInsert>,
    pub controllers: Vec<FlpController>,
}

fn color_of(v: u32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

fn decode_text(b: &[u8], utf16: bool) -> String {
    // UTF-16 strings always have an even length; fall back otherwise.
    if utf16 && b.len() % 2 == 0 {
        let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u).trim_end_matches('\0').to_string()
    } else {
        // Pre-11.5 projects use the Windows ANSI code page; Latin-1 is a close match.
        b.iter().map(|&c| c as char).collect::<String>().trim_end_matches('\0').to_string()
    }
}

pub fn encode_text(s: &str) -> Vec<u8> {
    let mut out: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    out.extend_from_slice(&[0, 0]);
    out
}

/// Parses "20.8.4.2576" into (20, 8).
pub fn parse_version(v: &str) -> (u32, u32) {
    let mut it = v.split('.').map(|p| p.trim_end_matches('\0').parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

enum Ctx {
    None,
    Channel(usize),
    Pattern(usize),
    Mixer,
}

impl FlpProject {
    pub fn from_file(f: &FlpFile) -> Self {
        let mut p = FlpProject {
            ppq: f.ppq.max(1),
            tempo: 140.0,
            time_sig: (4, 4),
            ..Default::default()
        };

        p.version = f
            .events
            .iter()
            .find(|e| e.id == ids::FL_VERSION)
            .and_then(|e| e.blob())
            .map(|b| decode_text(b, false));
        let ver = p.version.as_deref().map(parse_version);
        let utf16 = match ver {
            Some(v) => v >= (11, 5),
            None => true,
        };
        let major = ver.map(|v| v.0).unwrap_or(20);

        let mut ctx = Ctx::None;
        let mut pattern_index: BTreeMap<u16, usize> = BTreeMap::new();
        let mut insert = FlpInsert::default();
        let mut slot = FlpSlot::default();
        let mut tempo_coarse: Option<u32> = None;
        let mut tempo_fine: Option<u32> = None;
        let mut has_tempo = false;
        let mut mixer_params: Option<&[u8]> = None;

        for e in &f.events {
            let int = e.as_u32().unwrap_or(0);
            let text = || e.blob().map(|b| decode_text(b, utf16)).unwrap_or_default();

            match e.id {
                ids::TEMPO => {
                    p.tempo = int as f64 / 1000.0;
                    has_tempo = true;
                }
                ids::TEMPO_COARSE => tempo_coarse = Some(int),
                ids::TEMPO_FINE => tempo_fine = Some(int),
                ids::TIME_SIG_NUM if int > 0 => p.time_sig.0 = int as u8,
                ids::TIME_SIG_BEAT if int > 0 => p.time_sig.1 = int as u8,
                ids::TITLE => p.title = text(),
                ids::ARTISTS => p.artists = text(),
                ids::GENRE => p.genre = text(),
                ids::COMMENTS => p.comments = text(),

                ids::CH_NEW => {
                    p.channels.push(FlpChannel::new(int as u16));
                    ctx = Ctx::Channel(p.channels.len() - 1);
                }
                ids::PAT_NEW => {
                    let iid = int as u16;
                    let idx = *pattern_index.entry(iid).or_insert_with(|| {
                        p.patterns.push(FlpPattern { iid, ..Default::default() });
                        p.patterns.len() - 1
                    });
                    ctx = Ctx::Pattern(idx);
                }
                ids::PAT_NAME => {
                    if let Ctx::Pattern(i) = ctx {
                        p.patterns[i].name = Some(text());
                    }
                }
                ids::PAT_COLOR => {
                    if let Ctx::Pattern(i) = ctx {
                        p.patterns[i].color = Some(color_of(int));
                    }
                }
                ids::PAT_NOTES => {
                    if let (Ctx::Pattern(i), Some(b)) = (&ctx, e.blob()) {
                        p.patterns[*i].notes.extend(b.chunks_exact(NOTE_SIZE).map(FlpNote::parse));
                    }
                }

                ids::CONTROLLER => {
                    if let Some(c) = e.blob().and_then(FlpController::parse) {
                        p.controllers.push(c);
                    }
                }
                ids::ARR_NEW => {
                    p.has_arrangements = true;
                    p.arrangements.push(FlpArrangement { iid: int as u16, ..Default::default() });
                }
                ids::ARR_CURRENT => p.current_arrangement = Some(int as u16),
                ids::ARR_NAME => {
                    if let Some(a) = p.arrangements.last_mut() {
                        a.name = Some(text());
                    }
                }
                ids::ARR_PLAYLIST => {
                    if p.arrangements.is_empty() {
                        p.arrangements.push(FlpArrangement::default());
                    }
                    if let Some(b) = e.blob() {
                        let items = parse_playlist(b, major);
                        p.arrangements.last_mut().unwrap().items.extend(items);
                    }
                }
                ids::TRACK_DATA => {
                    if p.arrangements.is_empty() {
                        p.arrangements.push(FlpArrangement::default());
                    }
                    let b = e.blob().unwrap_or(&[]);
                    let rd = |i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                    p.arrangements.last_mut().unwrap().tracks.push(FlpTrack {
                        iid: rd(0).unwrap_or(0),
                        color: rd(4).map(color_of),
                        enabled: b.get(12).map(|&v| v != 0).unwrap_or(true),
                        name: None,
                    });
                }
                ids::MARKER_POS => {
                    let m = FlpTimeMarker { position: int & 0x00ff_ffff, kind: (int >> 24) as u8, ..Default::default() };
                    match p.arrangements.last_mut() {
                        Some(a) => a.markers.push(m),
                        // Before any arrangement: pre-12.9 markers, or a legacy copy in newer files.
                        None => p.legacy_markers.push(m),
                    }
                }
                ids::MARKER_NUM => {
                    if let Some(m) = last_marker(&mut p) {
                        m.numerator = int as u8;
                    }
                }
                ids::MARKER_DEN => {
                    if let Some(m) = last_marker(&mut p) {
                        m.denominator = int as u8;
                    }
                }
                ids::MARKER_NAME => {
                    if let Some(m) = last_marker(&mut p) {
                        m.name = Some(text());
                    }
                }
                ids::TRACK_NAME => {
                    if let Some(t) = p.arrangements.last_mut().and_then(|a| a.tracks.last_mut()) {
                        t.name = Some(text());
                    }
                }

                ids::INS_FLAGS => {
                    ctx = Ctx::Mixer;
                    if let Some(b) = e.blob().filter(|b| b.len() >= 8) {
                        let flags = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                        insert.enabled = flags & 8 != 0;
                    }
                }
                ids::MIXER_PARAMS => {
                    ctx = Ctx::Mixer;
                    mixer_params = e.blob();
                }
                ids::INS_ROUTING => {
                    ctx = Ctx::Mixer;
                    // One byte per destination insert: non-zero = routed there.
                    insert.routes = e.blob().unwrap_or(&[]).iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, _)| i).collect();
                }
                ids::INS_NAME => insert.name = Some(text()),
                ids::INS_COLOR if matches!(ctx, Ctx::Mixer) => insert.color = Some(color_of(int)),
                ids::SLOT_INDEX if matches!(ctx, Ctx::Mixer) => {
                    let mut s = std::mem::take(&mut slot);
                    s.index = int as u8;
                    s.enabled = true;
                    s.mix = 1.0;
                    if !s.internal.is_empty() || s.wrapper.is_some() {
                        insert.slots.push(s);
                    }
                }
                ids::INS_OUTPUT => {
                    ctx = Ctx::Mixer;
                    p.inserts.push(std::mem::take(&mut insert));
                }

                ids::PLUGIN_INTERNAL_NAME => match ctx {
                    Ctx::Channel(i) => p.channels[i].plugin = Some(text()).filter(|s| !s.is_empty()),
                    Ctx::Mixer => slot.internal = text(),
                    _ => {}
                },
                ids::PLUGIN_NAME => match ctx {
                    Ctx::Channel(i) => p.channels[i].name = Some(text()).filter(|s| !s.is_empty()),
                    Ctx::Mixer => slot.name = text(),
                    _ => {}
                },
                ids::PLUGIN_DATA => {
                    let raw = e.blob().unwrap_or(&[]);
                    match ctx {
                        Ctx::Channel(i) => {
                            let ch = &mut p.channels[i];
                            if ch.plugin.as_deref() == Some("Fruity Wrapper") {
                                ch.wrapper = FlpPluginWrapper::parse(raw);
                            } else {
                                ch.plugin_params = raw.to_vec();
                            }
                        }
                        Ctx::Mixer => {
                            if slot.internal == "Fruity Wrapper" {
                                slot.wrapper = FlpPluginWrapper::parse(raw);
                            } else {
                                slot.params = raw.to_vec();
                            }
                        }
                        _ => {}
                    }
                }

                _ => {
                    if let Ctx::Channel(i) = ctx {
                        apply_channel_event(&mut p.channels[i], e, utf16);
                    }
                }
            }
        }

        if !has_tempo {
            if let Some(c) = tempo_coarse {
                p.tempo = c as f64 + tempo_fine.unwrap_or(0) as f64 / 1000.0;
            }
        }
        if let Some(b) = mixer_params {
            apply_mixer_params(&mut p.inserts, b);
        }
        // Files before FL 12.9 have one implicit arrangement that owns the markers.
        if !p.has_arrangements && !p.legacy_markers.is_empty() {
            if p.arrangements.is_empty() {
                p.arrangements.push(FlpArrangement::default());
            }
            let legacy = std::mem::take(&mut p.legacy_markers);
            p.arrangements[0].markers.splice(0..0, legacy);
        }
        p
    }

    /// Serialises this project into a minimal FL 20-style event stream.
    ///
    /// Experimental: only the data modelled by this struct is written, so
    /// plugin states, automation and most UI state are lost.
    pub fn to_flp_file(&self) -> FlpFile {
        use EventValue::*;
        let mut ev = Vec::new();
        let mut push = |id: u8, v: EventValue| ev.push(Event::new(id, v));

        push(ids::FL_VERSION, Blob(b"20.8.4.2576\0".to_vec()));
        push(ids::FL_BUILD, Dword(2576));
        push(ids::TEMPO, Dword((self.tempo * 1000.0).round() as u32));
        push(ids::TIME_SIG_NUM, Byte(self.time_sig.0));
        push(ids::TIME_SIG_BEAT, Byte(self.time_sig.1));
        if !self.title.is_empty() {
            push(ids::TITLE, Blob(encode_text(&self.title)));
        }
        if !self.artists.is_empty() {
            push(ids::ARTISTS, Blob(encode_text(&self.artists)));
        }
        if !self.genre.is_empty() {
            push(ids::GENRE, Blob(encode_text(&self.genre)));
        }

        for pat in &self.patterns {
            push(ids::PAT_NEW, Word(pat.iid));
            if let Some(n) = &pat.name {
                push(ids::PAT_NAME, Blob(encode_text(n)));
            }
            if let Some(c) = pat.color {
                push(ids::PAT_COLOR, Dword(u32::from_le_bytes([c[0], c[1], c[2], 0])));
            }
            if !pat.notes.is_empty() {
                let mut b = Vec::with_capacity(pat.notes.len() * NOTE_SIZE);
                for n in &pat.notes {
                    n.write(&mut b);
                }
                push(ids::PAT_NOTES, Blob(b));
            }
        }

        for ch in &self.channels {
            push(ids::CH_NEW, Word(ch.iid));
            push(ids::CH_TYPE, Byte(ch.kind));
            if let Some(pl) = &ch.plugin {
                push(ids::PLUGIN_INTERNAL_NAME, Blob(encode_text(pl)));
            }
            if let Some(n) = &ch.name {
                push(ids::PLUGIN_NAME, Blob(encode_text(n)));
            }
            if let Some(c) = ch.color {
                push(ids::CH_COLOR, Dword(u32::from_le_bytes([c[0], c[1], c[2], 0])));
            }
            push(ids::CH_ENABLED, Byte(ch.enabled as u8));
            if ch.fx_flags != 0 {
                push(ids::CH_FX_FLAGS, Word(ch.fx_flags));
            }
            if ch.preamp != 0 {
                push(ids::CH_PREAMP, Word(ch.preamp));
            }
            if let Some(m) = ch.mixer_insert {
                push(ids::CH_ROUTED_TO, Byte(m as i8 as u8));
                push(ids::CH_INSERT, Word(m as i16 as u16));
            }
            if let Some(sp) = &ch.sample_path {
                push(ids::CH_SAMPLE_PATH, Blob(encode_text(sp)));
            }
            let mut levels = Vec::with_capacity(24);
            levels.extend_from_slice(&((ch.pan * PAN_CENTER + PAN_CENTER).round() as u32).to_le_bytes());
            levels.extend_from_slice(&((ch.volume * LEVEL_MAX).round() as u32).to_le_bytes());
            levels.extend_from_slice(&[0u8; 16]);
            push(ids::CH_LEVELS, Blob(levels));
        }

        for arr in &self.arrangements {
            push(ids::ARR_NEW, Word(arr.iid));
            if let Some(n) = &arr.name {
                push(ids::ARR_NAME, Blob(encode_text(n)));
            }
            let mut b = Vec::with_capacity(arr.items.len() * 32);
            for it in &arr.items {
                write_playlist_item(it, &mut b);
            }
            push(ids::ARR_PLAYLIST, Blob(b));
            for m in &arr.markers {
                push(ids::MARKER_POS, Dword((m.kind as u32) << 24 | (m.position & 0x00ff_ffff)));
                push(ids::MARKER_NUM, Byte(m.numerator.max(1)));
                push(ids::MARKER_DEN, Byte(m.denominator.max(1)));
                push(ids::MARKER_EXTRA[0], Byte(0));
                push(ids::MARKER_EXTRA[1], Byte(0));
                push(ids::MARKER_EXTRA[2], Dword(0));
                push(ids::MARKER_EXTRA[3], Word(0));
                if let Some(n) = &m.name {
                    push(ids::MARKER_NAME, Blob(encode_text(n)));
                }
            }
            for t in &arr.tracks {
                let mut d = Vec::with_capacity(13);
                d.extend_from_slice(&t.iid.to_le_bytes());
                let c = t.color.unwrap_or([0x48, 0x51, 0x56]);
                d.extend_from_slice(&u32::from_le_bytes([c[0], c[1], c[2], 0]).to_le_bytes());
                d.extend_from_slice(&0u32.to_le_bytes());
                d.push(t.enabled as u8);
                push(ids::TRACK_DATA, Blob(d));
                if let Some(n) = &t.name {
                    push(ids::TRACK_NAME, Blob(encode_text(n)));
                }
            }
        }

        let mut params = Vec::new();
        for (i, ins) in self.inserts.iter().enumerate() {
            let mut flags = [0u8; 12];
            flags[4..8].copy_from_slice(&(if ins.enabled { 8u32 | 4 } else { 4 }).to_le_bytes());
            push(ids::INS_FLAGS, Blob(flags.to_vec()));
            if let Some(n) = &ins.name {
                push(ids::INS_NAME, Blob(encode_text(n)));
            }
            push(ids::INS_OUTPUT, Dword(if i == 0 { u32::MAX } else { 0 }));
            let channel_data = ((i as u16) & 0x7f) << 6;
            for (id, val) in [(0xC0u8, ins.volume), (0xC1u8, ins.pan)] {
                if let Some(v) = val {
                    params.extend_from_slice(&[0, 0, 0, 0, id, 0]);
                    params.extend_from_slice(&channel_data.to_le_bytes());
                    params.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        if !params.is_empty() {
            push(ids::MIXER_PARAMS, Blob(params));
        }

        FlpFile { format: 0, channel_count: self.channels.len() as u16, ppq: self.ppq, events: ev }
    }
}

fn apply_channel_event(ch: &mut FlpChannel, e: &Event, utf16: bool) {
    let int = e.as_u32().unwrap_or(0);
    match e.id {
        ids::CH_ENABLED => ch.enabled = int != 0,
        ids::CH_TYPE => ch.kind = int as u8,
        ids::CH_ROUTED_TO => ch.mixer_insert = Some(int as u8 as i8 as i32),
        ids::CH_INSERT => ch.mixer_insert = Some(int as u16 as i16 as i32),
        ids::CH_FX_FLAGS => ch.fx_flags = int as u16,
        ids::CH_PREAMP => ch.preamp = int as u16,
        ids::CH_PARAMS => {
            // Time-stretch fields of the current (168+ byte) layout: "time" as a
            // float in 1/192-beat units at 96, pitch in cents at 100, mode at 108.
            // Older, shorter layouts differ and are left alone.
            if let Some(b) = e.blob().filter(|b| b.len() >= STRETCH_LAYOUT_LEN) {
                let i32_at = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                let v = f32::from_le_bytes([b[96], b[97], b[98], b[99]]);
                ch.stretch_beats = if v.is_normal() && v > 0.0 { v / 192.0 } else { 0.0 };
                ch.stretch_pitch = i32_at(100);
                ch.stretch_mode = i32_at(108);
            }
        }
        ids::CH_VOL_BYTE => ch.volume = int as f32 / 128.0,
        ids::CH_PAN_BYTE => ch.pan = (int as f32 - 64.0) / 64.0,
        ids::CH_VOL_WORD => ch.volume = int as f32 / LEVEL_MAX,
        ids::CH_PAN_WORD => ch.pan = (int as f32 - PAN_CENTER) / PAN_CENTER,
        ids::CH_COLOR => ch.color = Some(color_of(int)),
        ids::CH_ROOT_NOTE => ch.root_note = Some(int),
        ids::CH_NAME_LEGACY => {
            if ch.name.is_none() {
                ch.name = e.blob().map(|b| decode_text(b, utf16)).filter(|s| !s.is_empty());
            }
        }
        ids::CH_SAMPLE_PATH => {
            ch.sample_path = e.blob().map(|b| decode_text(b, utf16)).filter(|s| !s.is_empty());
        }
        ids::CH_AUTOMATION => ch.automation = e.blob().and_then(parse_automation),
        ids::CH_ENVELOPE => {
            if ch.envelope_count == 1 {
                ch.volume_envelope = e.blob().and_then(FlpEnvelope::parse_enabled);
            }
            ch.envelope_count = ch.envelope_count.saturating_add(1);
        }
        ids::CH_CUT => {
            ch.cut = (int & 0xffff) as u16;
            ch.cut_by = (int >> 16) as u16;
        }
        ids::CH_LEVELS => {
            if let Some(b) = e.blob().filter(|b| b.len() >= 8) {
                let pan = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32;
                let vol = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as f32;
                ch.pan = ((pan - PAN_CENTER) / PAN_CENTER).clamp(-1.0, 1.0);
                ch.volume = (vol / LEVEL_MAX).clamp(0.0, 1.0);
                if let Some(p) = b.get(8..12) {
                    ch.pitch_cents = i32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                }
            }
        }
        _ => {}
    }
}

/// Writes the linear clip gain into one FL 21+ playlist item (>= 60 bytes).
pub fn write_item_gain(item: &mut [u8], gain: f32) {
    if item.len() >= 60 {
        item[52..56].copy_from_slice(&gain.max(0.0).to_le_bytes());
    }
}

/// Writes clip fades (milliseconds) into one FL 21+ playlist item of at
/// least 60 bytes; byte 56 flags the fades as set.
pub fn write_item_fades(item: &mut [u8], fade_in: f32, fade_out: f32) {
    if item.len() < 60 {
        return;
    }
    item[36..40].copy_from_slice(&fade_in.max(0.0).to_le_bytes());
    item[44..48].copy_from_slice(&fade_out.max(0.0).to_le_bytes());
    if fade_in > 0.0 || fade_out > 0.0 {
        item[56] |= 3;
    }
}

/// Item sizes seen so far: 32 bytes (<= FL 20.8), 60 (FL 20.99/21+), 64 (FL 25).
const PLAYLIST_ITEM_SIZES: [usize; 7] = [32, 60, 64, 68, 72, 76, 80];

/// Detects the playlist item size. Every item stores the constant
/// `PATTERN_BASE` at offset 4, which identifies the right stride regardless
/// of FL version.
pub fn playlist_item_size(b: &[u8], major: u32) -> usize {
    let valid = |size: usize| {
        b.len() % size == 0 && b.chunks_exact(size).all(|c| u16::from_le_bytes([c[4], c[5]]) == PATTERN_BASE)
    };
    if let Some(&s) = PLAYLIST_ITEM_SIZES.iter().find(|&&s| valid(s)) {
        return s;
    }
    match major {
        25.. if b.len() % 64 == 0 => 64,
        21.. if b.len() % 60 == 0 => 60,
        _ if b.len() % 32 == 0 => 32,
        _ => 60,
    }
}

pub fn parse_playlist(b: &[u8], major: u32) -> Vec<FlpPlaylistItem> {
    let size = playlist_item_size(b, major);
    b.chunks_exact(size)
        .map(|c| {
            let u16_at = |i: usize| u16::from_le_bytes([c[i], c[i + 1]]);
            let u32_at = |i: usize| u32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            let f32_at = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            let pattern_base = u16_at(4);
            let item_index = u16_at(6);
            let source = if pattern_base != 0 && item_index > pattern_base {
                FlpItemSource::Pattern(item_index - pattern_base)
            } else {
                FlpItemSource::Channel(item_index)
            };
            FlpPlaylistItem {
                position: u32_at(0),
                length: u32_at(8),
                track: (MAX_TRACKS - 1).saturating_sub(u16_at(12)),
                source,
                muted: u16_at(18) & ITEM_MUTED != 0,
                start_offset: f32_at(24),
                end_offset: f32_at(28),
                fade_in: if size >= 48 { f32_at(36).max(0.0) } else { 0.0 },
                fade_out: if size >= 48 { f32_at(44).max(0.0) } else { 0.0 },
                gain: if size >= 60 { Some(f32_at(52)).filter(|g| g.is_finite() && *g >= 0.0).unwrap_or(1.0) } else { 1.0 },
            }
        })
        .collect()
}

pub fn write_playlist_item(it: &FlpPlaylistItem, out: &mut Vec<u8>) {
    let (base, index) = match it.source {
        FlpItemSource::Pattern(p) => (PATTERN_BASE, PATTERN_BASE + p),
        FlpItemSource::Channel(c) => (PATTERN_BASE, c),
    };
    out.extend_from_slice(&it.position.to_le_bytes());
    out.extend_from_slice(&base.to_le_bytes());
    out.extend_from_slice(&index.to_le_bytes());
    out.extend_from_slice(&it.length.to_le_bytes());
    out.extend_from_slice(&((MAX_TRACKS - 1) - it.track.min(MAX_TRACKS - 1)).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&[120, 0]);
    out.extend_from_slice(&(64u16 | if it.muted { ITEM_MUTED } else { 0 }).to_le_bytes());
    out.extend_from_slice(&[64, 100, 128, 128]);
    out.extend_from_slice(&it.start_offset.to_le_bytes());
    out.extend_from_slice(&it.end_offset.to_le_bytes());
}

/// Marks a mixer parameter entry as a send level (byte 5 of the entry).
pub const ROUTE_LEVEL_MARK: u8 = 0x20;

fn apply_mixer_params(inserts: &mut [FlpInsert], b: &[u8]) {
    for c in b.chunks_exact(12) {
        let id = c[4];
        let channel_data = u16::from_le_bytes([c[6], c[7]]);
        let msg = i32::from_le_bytes([c[8], c[9], c[10], c[11]]);
        let Some((insert, slot_idx)) = mixer_location(channel_data) else { continue };
        let Some(ins) = inserts.get_mut(insert) else { continue };
        // Send levels are marked 0x20 in the byte after the id, which then names
        // the destination insert; regular parameters use 0x1f.
        if c[5] == ROUTE_LEVEL_MARK {
            ins.route_levels.retain(|(d, _)| *d != id as usize);
            ins.route_levels.push((id as usize, msg));
            continue;
        }
        match id {
            0xC0 => ins.volume = Some(msg),
            0xC1 => ins.pan = Some(msg),
            0x01 => {
                if let Some(s) = ins.slots.iter_mut().find(|s| s.index == slot_idx) {
                    s.mix = (msg as f32 / 12800.0).clamp(0.0, 1.0);
                }
            }
            0x00 => {
                if let Some(s) = ins.slots.iter_mut().find(|s| s.index == slot_idx) {
                    s.enabled = msg != 0;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project() -> FlpProject {
        FlpProject {
            version: None,
            ppq: 96,
            tempo: 128.5,
            time_sig: (4, 4),
            title: "Test Song".into(),
            artists: "Me".into(),
            genre: String::new(),
            comments: String::new(),
            channels: vec![FlpChannel {
                name: Some("Kick".into()),
                sample_path: Some(r"C:\Samples\kick.wav".into()),
                volume: 0.5,
                pan: -0.25,
                mixer_insert: Some(1),
                ..FlpChannel::new(0)
            }],
            patterns: vec![FlpPattern {
                iid: 1,
                name: Some("Pattern 1".into()),
                color: None,
                notes: vec![FlpNote { position: 96, rack_channel: 0, length: 24, key: 60, velocity: 100, pan: 64, ..Default::default() }],
            }],
            arrangements: vec![FlpArrangement {
                iid: 0,
                name: Some("Arrangement".into()),
                items: vec![FlpPlaylistItem {
                    position: 384,
                    length: 384,
                    track: 2,
                    source: FlpItemSource::Pattern(1),
                    start_offset: -1.0,
                    end_offset: -1.0,
                    muted: false,
                    fade_in: 0.0,
                    fade_out: 0.0,
                    gain: 1.0,
                }],
                tracks: vec![FlpTrack { iid: 1, name: Some("Drums".into()), color: None, enabled: true }],
                markers: vec![FlpTimeMarker { position: 768, kind: 0, name: Some("Chorus".into()), numerator: 4, denominator: 4 }],
            }],
            current_arrangement: None,
            legacy_markers: Vec::new(),
            has_arrangements: true,
            controllers: Vec::new(),
            inserts: vec![
                FlpInsert { name: Some("Master".into()), volume: Some(10000), pan: Some(0), ..Default::default() },
                FlpInsert { name: Some("Kick bus".into()), volume: Some(8000), pan: Some(-100), ..Default::default() },
            ],
        }
    }

    #[test]
    fn high_level_round_trip() {
        let p = sample_project();
        let bytes = p.to_flp_file().to_bytes();
        let q = FlpProject::from_file(&FlpFile::parse(&bytes).unwrap());
        assert_eq!(q.tempo, 128.5);
        assert_eq!(q.title, "Test Song");
        assert_eq!(q.channels.len(), 1);
        let ch = &q.channels[0];
        assert_eq!(ch.name.as_deref(), Some("Kick"));
        assert_eq!(ch.sample_path.as_deref(), Some(r"C:\Samples\kick.wav"));
        assert_eq!(ch.mixer_insert, Some(1));
        assert!((ch.volume - 0.5).abs() < 1e-3);
        assert!((ch.pan + 0.25).abs() < 1e-3);
        assert_eq!(q.patterns[0].notes, p.patterns[0].notes);
        assert_eq!(q.arrangements[0].items, p.arrangements[0].items);
        assert_eq!(q.arrangements[0].tracks[0].name.as_deref(), Some("Drums"));
        assert_eq!(q.inserts.len(), 2);
        assert_eq!(q.inserts[1].name.as_deref(), Some("Kick bus"));
        assert_eq!(q.inserts[1].volume, Some(8000));
        assert_eq!(q.inserts[1].pan, Some(-100));
    }

    #[test]
    fn wrapper_records() {
        fn rec(out: &mut Vec<u8>, id: u32, data: &[u8]) {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(data);
        }
        // FL's state container: 0x50-byte header, then chunk 3 (component state) and chunk 4.
        let mut state = vec![0u8; 0x50];
        rec(&mut state, 3, b"COMPONENT");
        rec(&mut state, 4, &[1, 2, 3]);
        let mut b = 12u32.to_le_bytes().to_vec();
        rec(&mut b, 50, &[8, 0, 0, 0]);
        rec(&mut b, 52, &[0xAB; 16]);
        rec(&mut b, 54, b"Vital");
        rec(&mut b, 55, br"C:\Program Files\Common Files\VST3\Vital.vst3\Contents\x86_64-win\Vital.vst3");
        rec(&mut b, 56, b"Vital Audio");
        rec(&mut b, 53, &state);
        let w = FlpPluginWrapper::parse(&b).unwrap();
        assert_eq!(w.name, "Vital");
        assert_eq!(w.vendor, "Vital Audio");
        assert!(w.instrument);
        assert_eq!(w.guid, Some([0xAB; 16]));
        assert_eq!(w.bundle_path(), r"C:\Program Files\Common Files\VST3\Vital.vst3");
        assert_eq!(w.vst3_component_state(), Some(&b"COMPONENT"[..]));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("20.8.4.2576"), (20, 8));
        assert_eq!(parse_version("11.5"), (11, 5));
    }
}

/// One automation point. `beats` is absolute from the clip start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlpAutoPoint {
    pub beats: f64,
    pub value: f64,
    pub tension: f32,
}

/// Parses an automation channel's point data (event DATA+26).
/// Points store beat deltas from the previous point.
pub fn parse_automation(b: &[u8]) -> Option<Vec<FlpAutoPoint>> {
    let count = u32::from_le_bytes(b.get(17..21)?.try_into().ok()?) as usize;
    let mut pos = 0.0;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let p = b.get(21 + k * 24..21 + k * 24 + 24)?;
        pos += f64::from_le_bytes(p[0..8].try_into().ok()?);
        out.push(FlpAutoPoint {
            beats: pos,
            value: f64::from_le_bytes(p[8..16].try_into().ok()?).clamp(0.0, 1.0),
            tension: f32::from_le_bytes(p[16..20].try_into().ok()?),
        });
    }
    Some(out)
}

/// What an automation clip (or other controller) drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlpControlTarget {
    /// A built-in channel parameter (FL's REC_ ids: 0 volume, 1 pan, 4 pitch, ...).
    Channel { iid: u16, param: u16 },
    /// A parameter of a channel's plugin.
    ChannelPlugin { iid: u16, param: u16 },
    /// A project-wide parameter: 0 main volume, 2 master pitch, 5 tempo.
    Global { param: u16 },
    /// A built-in mixer parameter (FL mixer ids: 0xC0 volume, 0xC1 pan, 0x01 slot mix, ...).
    Mixer { insert: u16, slot: u16, param: u16 },
    /// A parameter of the effect in a mixer slot.
    MixerPlugin { insert: u16, slot: u16, param: u16 },
}

/// A controller link (event DATA+19): source automation channel -> target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlpController {
    pub source_channel: u16,
    pub target: FlpControlTarget,
}

impl FlpController {
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 12 {
            return None;
        }
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        // Bytes 0-1 are non-zero for internal controllers (peak controllers etc.).
        if u16_at(0) != 0 {
            return None;
        }
        let source_channel = u16_at(2);
        let (lo, hi) = (u16_at(8), u16_at(10));
        // Mixer: 0x2000 + (insert << 6 | slot) in older files, 0x7000 + ... in FL 24+.
        let mixer = if hi >= 0x7000 { mixer_location(hi) } else if (0x2000..0x4000).contains(&hi) { Some((((hi - 0x2000) >> 6) as usize, (hi & 0x3f) as u8)) } else { None };
        let target = if let Some((insert, slot)) = mixer {
            let (insert, slot) = (insert as u16, slot as u16);
            if lo & 0xff00 == 0x1f00 {
                FlpControlTarget::Mixer { insert, slot, param: lo & 0xff }
            } else {
                FlpControlTarget::MixerPlugin { insert, slot, param: lo & 0x7fff }
            }
        } else if hi & 0x4000 != 0 {
            FlpControlTarget::Global { param: lo }
        } else if lo & 0x8000 != 0 {
            FlpControlTarget::ChannelPlugin { iid: hi, param: lo & 0x7fff }
        } else {
            FlpControlTarget::Channel { iid: hi, param: lo }
        };
        Some(Self { source_channel, target })
    }
}

#[cfg(test)]
mod controller_tests {
    use super::*;

    fn rec(lo: u16, hi: u16) -> Vec<u8> {
        let mut b = vec![0, 0, 7, 0, 0, 0, 0, 0];
        b.extend_from_slice(&lo.to_le_bytes());
        b.extend_from_slice(&hi.to_le_bytes());
        b.extend_from_slice(&[8, 0, 0, 0, 0xd5, 1, 0, 0]);
        b
    }

    #[test]
    fn targets() {
        let t = |lo, hi| FlpController::parse(&rec(lo, hi)).unwrap().target;
        assert_eq!(t(0x800c, 0x2341), FlpControlTarget::MixerPlugin { insert: 13, slot: 1, param: 12 });
        assert_eq!(t(0x1fc0, 0x72c0), FlpControlTarget::Mixer { insert: 11, slot: 0, param: 0xc0 });
        assert_eq!(t(5, 0x4000), FlpControlTarget::Global { param: 5 });
        assert_eq!(t(0x8036, 24), FlpControlTarget::ChannelPlugin { iid: 24, param: 0x36 });
        assert_eq!(t(4, 16), FlpControlTarget::Channel { iid: 16, param: 4 });
    }
}

/// A channel envelope (raw FL knob values; times are 100..65536 knob positions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FlpEnvelope {
    pub predelay: i32,
    pub attack: i32,
    pub hold: i32,
    pub decay: i32,
    /// 0..=128
    pub sustain: i32,
    pub release: i32,
}

impl FlpEnvelope {
    /// Parses an envelope/LFO event (DATA+10). Returns `None` when disabled.
    pub fn parse_enabled(b: &[u8]) -> Option<Self> {
        let f = |i: usize| b.get(i * 4..i * 4 + 4).map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]));
        if f(1)? == 0 {
            return None;
        }
        Some(Self { predelay: f(2)?, attack: f(3)?, hold: f(4)?, decay: f(5)?, sustain: f(6)?, release: f(7)? })
    }
}

/// Decodes a mixer location (insert << 6 | slot). Older files add 0x2000,
/// FL 24+ adds 0x7000. Returns `None` for out-of-range values.
pub fn mixer_location(v: u16) -> Option<(usize, u8)> {
    let x = if v >= 0x7000 { v - 0x7000 } else { v & 0x1fff };
    let insert = (x >> 6) as usize;
    (insert < 128).then_some((insert, (x & 0x3f) as u8))
}

#[cfg(test)]
mod mixer_location_tests {
    use super::mixer_location;

    #[test]
    fn all_encodings() {
        assert_eq!(mixer_location(0x0040), Some((1, 0)));
        assert_eq!(mixer_location(0x2041), Some((1, 1)));
        assert_eq!(mixer_location(0x7440), Some((17, 0)));
        assert_eq!(mixer_location(0x72c3), Some((11, 3)));
    }
}

/// The marker that MARKER_NUM/DEN/NAME events refer to: the last one parsed.
fn last_marker(p: &mut FlpProject) -> Option<&mut FlpTimeMarker> {
    match p.arrangements.last_mut() {
        Some(a) => a.markers.last_mut(),
        None => p.legacy_markers.last_mut(),
    }
}
