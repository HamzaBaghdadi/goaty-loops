use crate::sample::SampleData;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Internal resolution: ticks per quarter note.
pub const PPQ: u32 = 96;
/// One step of the step sequencer (a 16th note).
pub const STEP_TICKS: u32 = PPQ / 4;
/// FL's default step velocity (100/128).
pub const DEFAULT_VELOCITY: f32 = 100.0 / 128.0;
/// FL's default channel volume (78%).
pub const DEFAULT_CHANNEL_VOLUME: f32 = 0.78125;
pub const MAX_MIXER_TRACKS: usize = 128;
pub const MAX_CHANNELS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const PALETTE: [Rgb; 8] = [
    Rgb(0x6f, 0x86, 0x9c),
    Rgb(0x8e, 0x6b, 0xa8),
    Rgb(0x5d, 0x9e, 0x7a),
    Rgb(0xc0, 0x7a, 0x4f),
    Rgb(0xb5, 0x5d, 0x6e),
    Rgb(0x4f, 0x8f, 0xb8),
    Rgb(0xa8, 0x9c, 0x4f),
    Rgb(0x7a, 0x7f, 0xc4),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub tempo: f64,
    pub time_sig: (u8, u8),
    #[serde(default)]
    pub info: ProjectInfo,
    pub channels: Vec<Channel>,
    pub patterns: Vec<Pattern>,
    pub playlist: Playlist,
    pub mixer: Vec<MixerTrack>,
    /// Set when the project was imported from an FLP; used to write changes back.
    #[serde(default)]
    pub fl_origin: Option<FlOrigin>,
    /// MIDI controllers linked to parameters (MIDI learn).
    #[serde(default)]
    pub midi_links: Vec<MidiLink>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub artists: String,
    pub genre: String,
    pub comments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub name: String,
    pub color: Rgb,
    pub instrument: Instrument,
    /// 0..=1, FL scale (0.78 = unity).
    pub volume: f32,
    /// -1..=1
    pub pan: f32,
    pub muted: bool,
    pub mixer_track: usize,
    /// FL channel id when imported from an FLP.
    #[serde(default)]
    pub fl_iid: Option<u16>,
    /// Cut group this channel triggers (0 = none).
    #[serde(default)]
    pub cut_group: u16,
    /// Group that cuts this channel's voices (0 = none).
    #[serde(default)]
    pub cut_by: u16,
    /// Pitch offset in semitones.
    #[serde(default)]
    pub pitch: f32,
    /// Range of the pitch knob / automation in semitones (FL default 2).
    #[serde(default = "default_pitch_range")]
    pub pitch_range: f32,
}

impl Channel {
    pub fn new(name: impl Into<String>, instrument: Instrument) -> Self {
        Self {
            name: name.into(),
            color: Rgb(0x6f, 0x86, 0x9c),
            instrument,
            volume: DEFAULT_CHANNEL_VOLUME,
            pan: 0.0,
            muted: false,
            mixer_track: 0,
            fl_iid: None,
            cut_group: 0,
            cut_by: 0,
            pitch: 0.0,
            pitch_range: 2.0,
        }
    }

    /// Linear gain of the volume knob (see [`crate::dsp::channel_volume_to_gain`]).
    pub fn gain(&self) -> f32 {
        crate::dsp::channel_volume_to_gain(self.volume)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Instrument {
    Sampler(SamplerParams),
    Synth(SynthParams),
    Drum(DrumParams),
    /// A hosted VST3 instrument.
    Plugin(PluginParams),
    /// An automation clip source.
    Automation(AutomationParams),
    /// Produces no sound.
    Silent { label: String },
}

impl Instrument {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Instrument::Sampler(_) => "Sampler",
            Instrument::Synth(_) => "Goaty Synth",
            Instrument::Drum(_) => "Goaty Drum",
            Instrument::Plugin(_) => "VST3",
            Instrument::Automation(_) => "Automation",
            Instrument::Silent { .. } => "Silent",
        }
    }
}


/// A VST3 plugin reference. `info` and `state` are saved with the project;
/// `instance` is the live plugin.
#[derive(Clone, Serialize, Deserialize)]
pub struct PluginParams {
    pub info: crate::plugin::PluginInfo,
    #[serde(with = "crate::plugin::state_serde", default)]
    pub state: Vec<u8>,
    #[serde(skip)]
    pub instance: Option<Arc<crate::plugin::PluginInstance>>,
    /// Set when loading the plugin failed, so it isn't retried.
    #[serde(skip)]
    pub load_failed: bool,
}

impl PluginParams {
    pub fn new(info: crate::plugin::PluginInfo, instance: Option<Arc<crate::plugin::PluginInstance>>) -> Self {
        Self { info, state: Vec::new(), instance, load_failed: false }
    }
}

impl std::fmt::Debug for PluginParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Plugin({}, loaded: {})", self.info.name, self.instance.is_some())
    }
}

impl PartialEq for PluginParams {
    fn eq(&self, other: &Self) -> bool {
        self.info == other.info
            && match (&self.instance, &other.instance) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SamplerParams {
    pub path: Option<PathBuf>,
    pub root_key: u8,
    /// When false the sample plays to the end regardless of note length,
    /// like FL's sampler with the volume envelope disabled.
    pub use_envelope: bool,
    pub env: Envelope,
    pub reverse: bool,
    /// Time stretch: the whole sample plays over this many beats (0 = off).
    /// Like FL's default "resample" mode, this also changes the pitch.
    /// Sample preamp, linear (FL: 1 + value / 16); 1 = none.
    #[serde(default = "unity_gain")]
    pub preamp: f32,
    #[serde(default)]
    pub stretch_beats: f32,
    /// Keep the pitch while stretching (FL's stretch modes other than resample).
    #[serde(default)]
    pub keep_pitch: bool,
    /// Extra pitch shift of the time-stretch section, in cents.
    #[serde(default)]
    pub stretch_pitch: f32,
    #[serde(skip)]
    pub data: Option<Arc<SampleData>>,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self { path: None, root_key: 60, use_envelope: false, env: Envelope::default(), reverse: false, preamp: 1.0, stretch_beats: 0.0, keep_pitch: false, stretch_pitch: 0.0, data: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub attack: f32,
    /// Time at full level after the attack, in seconds.
    #[serde(default)]
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self { attack: 0.002, hold: 0.0, decay: 0.3, sustain: 0.7, release: 0.15 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
    Noise,
}

impl Wave {
    pub const ALL: [Wave; 5] = [Wave::Sine, Wave::Triangle, Wave::Saw, Wave::Square, Wave::Noise];
    pub fn name(self) -> &'static str {
        match self {
            Wave::Sine => "Sine",
            Wave::Triangle => "Triangle",
            Wave::Saw => "Saw",
            Wave::Square => "Square",
            Wave::Noise => "Noise",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Osc {
    pub wave: Wave,
    /// Semitones.
    pub coarse: i32,
    /// Cents.
    pub fine: f32,
    pub level: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthParams {
    pub osc: [Osc; 3],
    pub env: Envelope,
    /// Hz
    pub cutoff: f32,
    /// 0..1
    pub resonance: f32,
    /// Name of the original plugin when imported from another DAW.
    #[serde(default)]
    pub replaces: Option<String>,
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            osc: [
                Osc { wave: Wave::Saw, coarse: 0, fine: 0.0, level: 0.6 },
                Osc { wave: Wave::Saw, coarse: 0, fine: 7.0, level: 0.4 },
                Osc { wave: Wave::Square, coarse: -12, fine: 0.0, level: 0.3 },
            ],
            env: Envelope::default(),
            cutoff: 6000.0,
            resonance: 0.2,
            replaces: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrumKind {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
}

impl DrumKind {
    pub const ALL: [DrumKind; 5] = [DrumKind::Kick, DrumKind::Snare, DrumKind::Clap, DrumKind::ClosedHat, DrumKind::OpenHat];
    pub fn name(self) -> &'static str {
        match self {
            DrumKind::Kick => "Kick",
            DrumKind::Snare => "Snare",
            DrumKind::Clap => "Clap",
            DrumKind::ClosedHat => "Hat",
            DrumKind::OpenHat => "Open Hat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrumParams {
    pub kind: DrumKind,
    /// Semitones.
    pub tune: f32,
    /// Multiplier on the decay time.
    pub decay: f32,
}

impl DrumParams {
    pub fn new(kind: DrumKind) -> Self {
        Self { kind, tune: 0.0, decay: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub channel: usize,
    /// MIDI key, 60 = C5 in FL naming.
    pub key: u8,
    pub start: u32,
    pub length: u32,
    /// 0..=1
    pub velocity: f32,
    /// -1..=1
    pub pan: f32,
}

impl Note {
    pub fn step(channel: usize, step: u32) -> Self {
        Self { channel, key: 60, start: step * STEP_TICKS, length: STEP_TICKS, velocity: DEFAULT_VELOCITY, pan: 0.0 }
    }
    /// End tick; zero-length (FL step) notes count as one step.
    pub fn end(&self) -> u32 {
        self.start + if self.length == 0 { STEP_TICKS } else { self.length }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub color: Rgb,
    pub notes: Vec<Note>,
    /// Fixed length in ticks; `None` means "auto" (rounded up to whole bars).
    #[serde(default)]
    pub length: Option<u32>,
    /// FL pattern id when imported from an FLP.
    #[serde(default)]
    pub fl_iid: Option<u16>,
}

impl Pattern {
    pub fn new(name: impl Into<String>, color: Rgb) -> Self {
        Self { name: name.into(), color, notes: Vec::new(), length: None, fl_iid: None }
    }

    pub fn length_ticks(&self, bar: u32) -> u32 {
        if let Some(l) = self.length {
            return l.max(STEP_TICKS);
        }
        let end = self.notes.iter().map(|n| n.end()).max().unwrap_or(0);
        end.div_ceil(bar).max(1) * bar
    }

    /// Whether this channel's notes can be shown as plain steps
    /// (all on the default key, one step long, on the step grid).
    /// FL stores step-sequencer notes with a length of 0.
    pub fn is_step_only(&self, channel: usize) -> bool {
        self.notes
            .iter()
            .filter(|n| n.channel == channel)
            .all(|n| n.key == 60 && (n.length == STEP_TICKS || n.length == 0) && n.start % STEP_TICKS == 0)
    }

    pub fn step_on(&self, channel: usize, step: u32) -> bool {
        let t = step * STEP_TICKS;
        self.notes.iter().any(|n| n.channel == channel && n.start == t)
    }

    pub fn set_step(&mut self, channel: usize, step: u32, on: bool) {
        if self.step_on(channel, step) != on {
            self.toggle_step(channel, step);
        }
    }

    pub fn toggle_step(&mut self, channel: usize, step: u32) {
        let t = step * STEP_TICKS;
        let before = self.notes.len();
        self.notes.retain(|n| !(n.channel == channel && n.start == t));
        if self.notes.len() == before {
            self.notes.push(Note::step(channel, step));
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub tracks: Vec<PlaylistTrack>,
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub markers: Vec<TimeMarker>,
}

/// A named position in the song (FL time marker).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeMarker {
    pub tick: u32,
    pub name: String,
    /// FL marker action (0 = plain marker); kept so it round-trips.
    #[serde(default)]
    pub kind: u8,
    /// Time signature from this marker on, for signature markers (`kind` 8).
    #[serde(default)]
    pub numerator: u8,
    #[serde(default)]
    pub denominator: u8,
}

impl TimeMarker {
    /// FL marker action of a time-signature change.
    pub const SIGNATURE: u8 = 8;

    /// The signature this marker sets, if it is a signature marker.
    pub fn signature(&self) -> Option<(u8, u8)> {
        (self.kind == Self::SIGNATURE && self.numerator > 0 && self.denominator > 0).then_some((self.numerator, self.denominator))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistTrack {
    pub name: String,
    pub color: Rgb,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipSource {
    Pattern(usize),
    /// Plays a channel's sample from the start of the clip (an audio clip).
    Audio(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub track: usize,
    pub start: u32,
    pub length: u32,
    /// Offset into the source, in ticks.
    pub offset: u32,
    pub source: ClipSource,
    pub muted: bool,
    /// Fade-in and fade-out lengths in ticks (audio clips).
    #[serde(default)]
    pub fade_in: u32,
    #[serde(default)]
    pub fade_out: u32,
    /// Linear gain of the clip (FL 21+ audio clip volume; 1.0 = 0 dB).
    #[serde(default = "unity")]
    pub gain: f32,
}

fn unity() -> f32 {
    1.0
}

impl Clip {
    pub fn end(&self) -> u32 {
        self.start + self.length
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixerTrack {
    pub name: String,
    pub color: Rgb,
    /// Linear gain; 1.0 = 0 dB.
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub effects: Vec<EffectSlot>,
    /// Main destination mixer track (any track but this one; routes that
    /// would form a loop are ignored).
    pub output: usize,
    /// Further destinations (FL sends, e.g. to reverb and delay buses).
    #[serde(default)]
    pub sends: Vec<MixerSend>,
    /// Built-in 3-band EQ gains in dB (low shelf, peak, high shelf).
    #[serde(default)]
    pub eq: [f32; 3],
}

/// A send from one mixer track to another, at a linear gain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerSend {
    pub to: usize,
    pub gain: f32,
}

impl MixerTrack {
    /// The effect addressed by `slot` in automation targets: its FL slot
    /// number when imported from FL, otherwise its position.
    pub fn slot_by_id(&self, slot: u8) -> Option<&EffectSlot> {
        self.effects
            .iter()
            .find(|e| e.fl.as_ref().is_some_and(|f| f.slot == slot))
            .or_else(|| self.effects.get(slot as usize).filter(|e| e.fl.is_none()))
    }

    /// Inverse of [`slot_by_id`](Self::slot_by_id).
    pub fn slot_id(&self, index: usize) -> u8 {
        self.effects.get(index).and_then(|e| e.fl.as_ref()).map(|f| f.slot).unwrap_or(index as u8)
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), color: Rgb(0x55, 0x5f, 0x66), volume: 1.0, pan: 0.0, muted: false, effects: Vec::new(), output: 0, sends: Vec::new(), eq: [0.0; 3] }
    }
}


/// One automation point; `tick` is relative to the clip start.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoPoint {
    pub tick: u32,
    /// 0..=1
    pub value: f32,
    /// Curve shape towards the next point (-1..=1, 0 = linear).
    pub tension: f32,
}

/// A MIDI controller (CC) linked to a parameter by MIDI learn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiLink {
    /// MIDI channel 0..=15.
    pub channel: u8,
    pub cc: u8,
    pub target: AutoTarget,
}

/// What an automation clip controls. Values are normalized 0..=1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoTarget {
    ChannelVolume(usize),
    ChannelPan(usize),
    ChannelPitch(usize),
    MixerVolume(usize),
    MixerPan(usize),
    /// A parameter of the effect in mixer `track`, FL slot `slot`.
    EffectParam { track: usize, slot: u8, param: u32 },
    /// Song tempo: 60 + value * 120 BPM (FL's mapping).
    Tempo,
    /// Main output volume, after the master insert.
    MainVolume,
    /// Dry/wet mix of the effect in a mixer slot.
    SlotMix { track: usize, slot: u8 },
    /// On/off of the effect in a mixer slot (>= 0.5 = on).
    SlotEnabled { track: usize, slot: u8 },
    /// A parameter (by index) of a channel's VST3 instrument.
    PluginParam { channel: usize, param: u32 },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutomationParams {
    pub points: Vec<AutoPoint>,
    pub target: Option<AutoTarget>,
}

impl AutomationParams {
    /// Interpolated value at `tick` (relative to the clip start).
    pub fn value_at(&self, tick: f32) -> Option<f32> {
        let pts = &self.points;
        let first = pts.first()?;
        if tick <= first.tick as f32 {
            return Some(first.value);
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if tick < b.tick as f32 {
                let span = (b.tick - a.tick).max(1) as f32;
                let x = ((tick - a.tick as f32) / span).clamp(0.0, 1.0);
                // Tension bends the curve: positive = slow start, negative = fast start.
                let shaped = if a.tension.abs() < 1e-3 {
                    x
                } else if a.tension > 0.0 {
                    x.powf(1.0 + a.tension * 4.0)
                } else {
                    1.0 - (1.0 - x).powf(1.0 - a.tension * 4.0)
                };
                return Some(a.value + (b.value - a.value) * shaped);
            }
        }
        pts.last().map(|p| p.value)
    }

    pub fn length_ticks(&self) -> u32 {
        self.points.last().map(|p| p.tick).unwrap_or(0)
    }
}

fn default_pitch_range() -> f32 {
    2.0
}

fn full_mix() -> f32 {
    1.0
}

/// Where an effect came from in an imported FL project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlSlotInfo {
    /// FL's internal plugin name, e.g. "Fruity Reeverb 2".
    pub internal: String,
    /// FL slot index (0-9) in the insert.
    pub slot: u8,
    /// Raw FL parameters, used to re-decode the effect when automated.
    #[serde(with = "crate::plugin::state_serde", default)]
    pub data: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectSlot {
    /// Dry/wet mix (1 = fully processed).
    #[serde(default = "full_mix")]
    pub mix: f32,
    #[serde(default)]
    pub fl: Option<FlSlotInfo>,
    pub enabled: bool,
    pub effect: Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EqBandKind {
    Off,
    LowPass,
    BandPass,
    HighPass,
    Notch,
    LowShelf,
    Peak,
    HighShelf,
}

impl EqBandKind {
    pub const ALL: [EqBandKind; 8] = [
        EqBandKind::Off,
        EqBandKind::LowPass,
        EqBandKind::BandPass,
        EqBandKind::HighPass,
        EqBandKind::Notch,
        EqBandKind::LowShelf,
        EqBandKind::Peak,
        EqBandKind::HighShelf,
    ];
    pub fn name(self) -> &'static str {
        match self {
            EqBandKind::Off => "Off",
            EqBandKind::LowPass => "Low pass",
            EqBandKind::BandPass => "Band pass",
            EqBandKind::HighPass => "High pass",
            EqBandKind::Notch => "Notch",
            EqBandKind::LowShelf => "Low shelf",
            EqBandKind::Peak => "Peaking",
            EqBandKind::HighShelf => "High shelf",
        }
    }
}

/// Normalized 0..1 position -> frequency on a 20 Hz..20 kHz log scale.
pub fn eq_freq_from_norm(x: f32) -> f32 {
    20.0 * 1000f32.powf(x.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub kind: EqBandKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl EqBand {
    /// FL Parametric EQ 2's default band layout.
    pub fn defaults() -> [EqBand; 7] {
        let kinds = [
            EqBandKind::LowShelf,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::Peak,
            EqBandKind::HighShelf,
        ];
        let mut out = [EqBand { kind: EqBandKind::Peak, freq: 1000.0, gain_db: 0.0, q: 0.9 }; 7];
        for (i, b) in out.iter_mut().enumerate() {
            b.kind = kinds[i];
            b.freq = eq_freq_from_norm((i as f32 + 1.5) / 9.0);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Delay { beats: f32, feedback: f32, mix: f32 },
    Reverb { room: f32, damping: f32, mix: f32 },
    Filter { kind: FilterKind, cutoff: f32, resonance: f32 },
    Distortion { drive: f32, mix: f32 },
    Compressor { threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, makeup_db: f32 },
    Eq3 { low_db: f32, mid_db: f32, high_db: f32 },
    /// A hosted VST3 effect.
    Plugin(PluginParams),
    /// 7-band parametric EQ (compatible with Fruity Parametric EQ 2).
    ParamEq { bands: [EqBand; 7], main_db: f32 },
    /// Gain, optional compressor and a brickwall limiter (Fruity Limiter).
    /// `sidechain`: 0 = the compressor listens to its own input; n = the n-th
    /// mixer track routed into this one (FL's sidechain input number).
    Limiter { gain_db: f32, ceiling_db: f32, release_ms: f32, comp_threshold_db: f32, comp_ratio: f32, #[serde(default)] sidechain: u8 },
    /// Algorithmic room/hall reverb (Fruity Reeverb 2).
    Reverb2 {
        low_cut: f32,
        high_cut: f32,
        predelay_ms: f32,
        size: f32,
        decay_s: f32,
        damping_hz: f32,
        width: f32,
        dry: f32,
        early: f32,
        wet: f32,
    },
    /// Volume and pan (Fruity Balance).
    Balance { gain: f32, pan: f32 },
    /// Stereo width, phase offset and polarity (Fruity Stereo Enhancer).
    StereoEnhancer { gain: f32, pan: f32, separation: f32, phase_offset_ms: f32, invert: u8 },
    /// Soft saturation above a threshold (Fruity Soft Clipper).
    SoftClipper { threshold: f32, post_gain: f32 },
    /// Multiband "make it louder" processor (Soundgoodizer).
    Soundgoodizer { mode: u8, amount: f32 },
    /// State-variable filter with mixable LP/BP/HP outputs (Fruity Filter).
    MultiFilter { cutoff: f32, resonance: f32, low: f32, band: f32, high: f32 },
    /// Left/right channel matrix (Fruity Stereo Shaper).
    StereoShaper { l_from_l: f32, l_from_r: f32, r_from_l: f32, r_from_r: f32 },
    /// Pan/volume with an optional pan LFO (Fruity PanOMatic).
    AutoPan { gain: f32, pan: f32, lfo_amount: f32, lfo_hz: f32 },
    /// Hard/soft clipping distortion (Fruity Fast Dist).
    FastDist { pre: f32, threshold: f32, kind: u8, mix: f32, post: f32 },
    /// Tube-style overdrive (Fruity Blood Overdrive).
    Overdrive { drive: f32, color: f32, post: f32 },
    /// Modulated short delay (Fruity Chorus).
    Chorus { delay_ms: f32, depth_ms: f32, rate_hz: f32, mix: f32 },
    /// Boosts or softens attacks and tails (Transient Processor). `attack`
    /// and `release` are -1..=1; 0 leaves the sound unchanged.
    TransientShaper { attack: f32, release: f32, gain_db: f32 },
}

impl Effect {
    pub fn defaults() -> Vec<Effect> {
        vec![
            Effect::Delay { beats: 0.75, feedback: 0.4, mix: 0.3 },
            Effect::Reverb { room: 0.7, damping: 0.4, mix: 0.25 },
            Effect::Filter { kind: FilterKind::Lowpass, cutoff: 2000.0, resonance: 0.3 },
            Effect::Distortion { drive: 4.0, mix: 0.5 },
            Effect::Compressor { threshold_db: -18.0, ratio: 4.0, attack_ms: 10.0, release_ms: 120.0, makeup_db: 4.0 },
            Effect::Eq3 { low_db: 0.0, mid_db: 0.0, high_db: 0.0 },
            Effect::ParamEq { bands: EqBand::defaults(), main_db: 0.0 },
            Effect::Limiter { gain_db: 0.0, ceiling_db: 0.0, release_ms: 500.0, comp_threshold_db: 0.0, comp_ratio: 3.0, sidechain: 0 },
            Effect::Reverb2 { low_cut: 75.0, high_cut: 22100.0, predelay_ms: 0.0, size: 1.0, decay_s: 1.5, damping_hz: 4000.0, width: 1.0, dry: 1.0, early: 0.5, wet: 0.5 },
            Effect::Balance { gain: 1.0, pan: 0.0 },
            Effect::StereoEnhancer { gain: 1.0, pan: 0.0, separation: 0.0, phase_offset_ms: 0.0, invert: 0 },
            Effect::SoftClipper { threshold: 100.0 / 127.0, post_gain: 1.0 },
            Effect::Soundgoodizer { mode: 0, amount: 0.5 },
            Effect::MultiFilter { cutoff: 2000.0, resonance: 0.2, low: 1.0, band: 0.0, high: 0.0 },
            Effect::StereoShaper { l_from_l: 1.0, l_from_r: 0.0, r_from_l: 0.0, r_from_r: 1.0 },
            Effect::AutoPan { gain: 1.0, pan: 0.0, lfo_amount: 0.5, lfo_hz: 1.0 },
            Effect::FastDist { pre: 1.0, threshold: 0.5, kind: 0, mix: 1.0, post: 1.0 },
            Effect::Overdrive { drive: 4.0, color: 0.5, post: 1.0 },
            Effect::Chorus { delay_ms: 10.0, depth_ms: 3.0, rate_hz: 0.8, mix: 0.5 },
            Effect::TransientShaper { attack: 0.5, release: 0.0, gain_db: 0.0 },
        ]
    }

    pub fn name(&self) -> String {
        match self {
            Effect::Delay { .. } => "Goaty Delay".into(),
            Effect::Reverb { .. } => "Goaty Reverb".into(),
            Effect::Filter { .. } => "Goaty Filter".into(),
            Effect::Distortion { .. } => "Goaty Distortion".into(),
            Effect::Compressor { .. } => "Goaty Compressor".into(),
            Effect::Eq3 { .. } => "Goaty EQ".into(),
            Effect::Plugin(p) => p.info.name.clone(),
            Effect::ParamEq { .. } => "Goaty Parametric EQ".into(),
            Effect::Limiter { .. } => "Goaty Limiter".into(),
            Effect::Reverb2 { .. } => "Goaty Reverb 2".into(),
            Effect::Balance { .. } => "Goaty Balance".into(),
            Effect::StereoEnhancer { .. } => "Goaty Stereo Enhancer".into(),
            Effect::SoftClipper { .. } => "Goaty Soft Clipper".into(),
            Effect::Soundgoodizer { .. } => "Goaty Loudener".into(),
            Effect::MultiFilter { .. } => "Goaty Multi Filter".into(),
            Effect::StereoShaper { .. } => "Goaty Stereo Shaper".into(),
            Effect::AutoPan { .. } => "Goaty Auto Pan".into(),
            Effect::FastDist { .. } => "Goaty Fast Dist".into(),
            Effect::Overdrive { .. } => "Goaty Overdrive".into(),
            Effect::Chorus { .. } => "Goaty Chorus".into(),
            Effect::TransientShaper { .. } => "Goaty Transient Shaper".into(),
        }
    }

    pub fn same_kind(&self, other: &Effect) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new_template()
    }
}

impl Project {
    /// Sets a parameter from a normalized 0..=1 value, using the same scale
    /// as automation. Returns false when the target isn't stored in the
    /// project (plugin and FL effect parameters) or doesn't exist.
    pub fn set_target(&mut self, target: AutoTarget, v: f32) -> bool {
        let v = v.clamp(0.0, 1.0);
        match target {
            AutoTarget::ChannelVolume(c) | AutoTarget::ChannelPan(c) | AutoTarget::ChannelPitch(c) => {
                let Some(ch) = self.channels.get_mut(c) else { return false };
                match target {
                    AutoTarget::ChannelVolume(_) => ch.volume = v,
                    AutoTarget::ChannelPan(_) => ch.pan = v * 2.0 - 1.0,
                    _ => ch.pitch = (v * 2.0 - 1.0) * ch.pitch_range,
                }
            }
            AutoTarget::MixerVolume(t) | AutoTarget::MixerPan(t) => {
                let Some(tr) = self.mixer.get_mut(t) else { return false };
                if matches!(target, AutoTarget::MixerVolume(_)) {
                    tr.volume = crate::dsp::fader_to_gain(v);
                } else {
                    tr.pan = v * 2.0 - 1.0;
                }
            }
            AutoTarget::Tempo => self.tempo = 60.0 + v as f64 * 120.0,
            AutoTarget::SlotMix { track, slot: s } => match fl_slot_mut(&mut self.mixer, track, s) {
                Some(e) => e.mix = v,
                None => return false,
            },
            AutoTarget::SlotEnabled { track, slot: s } => match fl_slot_mut(&mut self.mixer, track, s) {
                Some(e) => e.enabled = v >= 0.5,
                None => return false,
            },
            AutoTarget::MainVolume | AutoTarget::EffectParam { .. } | AutoTarget::PluginParam { .. } => return false,
        }
        true
    }

    pub fn empty() -> Self {
        let mut mixer = vec![MixerTrack::new("Master")];
        for i in 1..=16 {
            mixer.push(MixerTrack::new(format!("Insert {i}")));
        }
        Self {
            name: "Untitled".into(),
            tempo: 130.0,
            time_sig: (4, 4),
            info: ProjectInfo::default(),
            channels: Vec::new(),
            patterns: vec![Pattern::new("Pattern 1", PALETTE[0])],
            playlist: Playlist { tracks: default_tracks(), clips: Vec::new(), markers: Vec::new() },
            fl_origin: None,
            midi_links: Vec::new(),
            mixer,
        }
    }

    /// The default template: four drum channels, like FL's default project.
    pub fn new_template() -> Self {
        let mut p = Self::empty();
        let kinds = [DrumKind::Kick, DrumKind::Clap, DrumKind::ClosedHat, DrumKind::Snare];
        for (i, k) in kinds.into_iter().enumerate() {
            let mut ch = Channel::new(k.name(), Instrument::Drum(DrumParams::new(k)));
            ch.color = PALETTE[i % PALETTE.len()];
            p.channels.push(ch);
        }
        p
    }

    /// Bars between `t0` and `t1` (ticks), following time-signature markers:
    /// (start tick, bar number from 1, bar length, beat length).
    pub fn bars(&self, t0: u32, t1: u32) -> Vec<(u32, u32, u32, u32)> {
        let lens = |n: u8, d: u8| {
            let beat = PPQ * 4 / d.max(1) as u32;
            (beat * n.max(1) as u32, beat)
        };
        // Segments of constant signature: (start, bar length, beat length, first bar number).
        let (bar0, beat0) = lens(self.time_sig.0, self.time_sig.1);
        let mut segs = vec![(0u32, bar0, beat0, 1u32)];
        let mut changes: Vec<(u32, (u8, u8))> = self.playlist.markers.iter().filter_map(|m| Some((m.tick, m.signature()?))).collect();
        changes.sort_by_key(|c| c.0);
        for (tick, (n, d)) in changes {
            let (bar, beat) = lens(n, d);
            let &(start, len, _, first) = segs.last().expect("never empty");
            if tick <= start {
                // A change at the segment start replaces its signature.
                *segs.last_mut().unwrap() = (start, bar, beat, first);
            } else {
                segs.push((tick, bar, beat, first + (tick - start).div_ceil(len)));
            }
        }
        let mut out = Vec::new();
        for (i, &(start, len, beat, first)) in segs.iter().enumerate() {
            let end = segs.get(i + 1).map(|s| s.0).unwrap_or(u32::MAX);
            if end <= t0 || start > t1 {
                continue;
            }
            let skip = t0.saturating_sub(start) / len;
            let mut t = start + skip * len;
            let mut n = first + skip;
            while t < end && t <= t1 {
                out.push((t, n, len.min(end - t), beat));
                t += len;
                n += 1;
            }
        }
        out
    }

    pub fn bar_ticks(&self) -> u32 {
        // Beats are counted in quarter-note PPQ scaled by the denominator.
        let beat = PPQ * 4 / self.time_sig.1.max(1) as u32;
        beat * self.time_sig.0.max(1) as u32
    }

    pub fn beat_ticks(&self) -> u32 {
        PPQ * 4 / self.time_sig.1.max(1) as u32
    }

    /// End of the last clip, rounded up to a bar (at least one bar).
    pub fn song_length(&self) -> u32 {
        let bar = self.bar_ticks();
        let end = self.playlist.clips.iter().map(|c| c.end()).max().unwrap_or(0);
        end.div_ceil(bar).max(1) * bar
    }

    pub fn pattern_length(&self, idx: usize) -> u32 {
        let bar = self.bar_ticks();
        self.patterns.get(idx).map(|p| p.length_ticks(bar)).unwrap_or(bar)
    }

    /// Removes a channel and fixes up every reference to it.
    pub fn remove_channel(&mut self, idx: usize) {
        if idx >= self.channels.len() {
            return;
        }
        self.channels.remove(idx);
        self.remap_automation_channels(|c| if c == idx { None } else if c > idx { Some(c - 1) } else { Some(c) });
        for p in &mut self.patterns {
            p.notes.retain(|n| n.channel != idx);
            for n in &mut p.notes {
                if n.channel > idx {
                    n.channel -= 1;
                }
            }
        }
        self.playlist.clips.retain(|c| c.source != ClipSource::Audio(idx));
        for c in &mut self.playlist.clips {
            if let ClipSource::Audio(ch) = &mut c.source {
                if *ch > idx {
                    *ch -= 1;
                }
            }
        }
    }

    /// Re-points automation targets after channels were removed or reordered.
    /// `f` maps an old channel index to its new one (`None` = removed).
    pub fn remap_automation_channels(&mut self, f: impl Fn(usize) -> Option<usize>) {
        for ch in &mut self.channels {
            if let Instrument::Automation(a) = &mut ch.instrument {
                a.target = match a.target {
                    Some(AutoTarget::ChannelVolume(c)) => f(c).map(AutoTarget::ChannelVolume),
                    Some(AutoTarget::ChannelPan(c)) => f(c).map(AutoTarget::ChannelPan),
                    Some(AutoTarget::ChannelPitch(c)) => f(c).map(AutoTarget::ChannelPitch),
                    Some(AutoTarget::PluginParam { channel, param }) => f(channel).map(|channel| AutoTarget::PluginParam { channel, param }),
                    other => other,
                };
            }
        }
    }

    pub fn remove_pattern(&mut self, idx: usize) {
        if idx >= self.patterns.len() || self.patterns.len() == 1 {
            return;
        }
        self.patterns.remove(idx);
        self.playlist.clips.retain(|c| c.source != ClipSource::Pattern(idx));
        for c in &mut self.playlist.clips {
            if let ClipSource::Pattern(p) = &mut c.source {
                if *p > idx {
                    *p -= 1;
                }
            }
        }
    }

    pub fn ensure_tracks(&mut self, n: usize) {
        while self.playlist.tracks.len() < n {
            let i = self.playlist.tracks.len();
            self.playlist.tracks.push(PlaylistTrack { name: format!("Track {}", i + 1), color: Rgb(0x48, 0x51, 0x56), muted: false });
        }
    }

    /// Loads sample data for every sampler channel. Returns paths that failed.
    pub fn load_samples(&mut self) -> Vec<String> {
        let mut failed = Vec::new();
        for ch in &mut self.channels {
            if let Instrument::Sampler(s) = &mut ch.instrument {
                if s.data.is_some() {
                    continue;
                }
                if let Some(path) = &s.path {
                    match crate::sample::load_audio(path) {
                        Ok(d) => s.data = Some(Arc::new(d)),
                        Err(e) => failed.push(format!("{}: {e}", path.display())),
                    }
                }
            }
        }
        failed
    }

    /// Every plugin reference in the project (channels and mixer slots).
    /// The plugin instance and parameter index a target refers to.
    pub fn plugin_param(&self, target: AutoTarget) -> Option<(Arc<crate::plugin::PluginInstance>, u32)> {
        let (params, param) = match target {
            AutoTarget::PluginParam { channel, param } => match &self.channels.get(channel)?.instrument {
                Instrument::Plugin(p) => (p, param),
                _ => return None,
            },
            AutoTarget::EffectParam { track, slot, param } => match &self.mixer.get(track)?.slot_by_id(slot)?.effect {
                Effect::Plugin(p) => (p, param),
                _ => return None,
            },
            _ => return None,
        };
        Some((params.instance.clone()?, param))
    }

    /// The plugin parameter most recently moved in any plugin editor.
    pub fn last_tweaked(&self) -> Option<AutoTarget> {
        let mut best: Option<(u32, AutoTarget)> = None;
        let mut consider = |inst: &Option<Arc<crate::plugin::PluginInstance>>, mk: &dyn Fn(u32) -> AutoTarget| {
            if let Some((seq, index)) = inst.as_ref().and_then(|i| i.last_touched()) {
                if best.is_none_or(|(s, _)| seq > s) {
                    best = Some((seq, mk(index)));
                }
            }
        };
        for (channel, ch) in self.channels.iter().enumerate() {
            if let Instrument::Plugin(p) = &ch.instrument {
                consider(&p.instance, &|param| AutoTarget::PluginParam { channel, param });
            }
        }
        for (track, t) in self.mixer.iter().enumerate() {
            for (i, s) in t.effects.iter().enumerate() {
                if let Effect::Plugin(p) = &s.effect {
                    let slot = t.slot_id(i);
                    consider(&p.instance, &|param| AutoTarget::EffectParam { track, slot, param });
                }
            }
        }
        best.map(|(_, t)| t)
    }

    pub fn plugins_mut(&mut self) -> Vec<&mut PluginParams> {
        let mut v = Vec::new();
        for ch in &mut self.channels {
            if let Instrument::Plugin(p) = &mut ch.instrument {
                v.push(p);
            }
        }
        for t in &mut self.mixer {
            for s in &mut t.effects {
                if let Effect::Plugin(p) = &mut s.effect {
                    v.push(p);
                }
            }
        }
        v
    }

    /// Loads every plugin that has no live instance and restores its state.
    /// Returns messages for plugins that failed.
    pub fn instantiate_plugins(&mut self, sample_rate: f64) -> Vec<String> {
        let mut errors = Vec::new();
        for p in self.plugins_mut() {
            if p.instance.is_some() {
                continue;
            }
            match crate::plugin::PluginInstance::new(&p.info, sample_rate) {
                Ok(inst) => {
                    if !p.state.is_empty() && !inst.set_state(&p.state) {
                        errors.push(format!("{}: the saved settings were not accepted", p.info.name));
                    }
                    p.instance = Some(inst);
                }
                Err(e) => errors.push(format!("{}: {e}", p.info.name)),
            }
        }
        errors
    }

    /// Drops every live plugin instance, keeping its current settings, so the
    /// plugins can be loaded again (e.g. at a new sample rate).
    pub fn unload_plugins(&mut self) {
        for p in self.plugins_mut() {
            if let Some(inst) = p.instance.take() {
                p.state = inst.get_state();
            }
            p.load_failed = false;
        }
    }

    /// Plugins that still need loading.
    pub fn pending_plugins(&mut self) -> usize {
        self.plugins_mut().iter().filter(|p| p.instance.is_none() && !p.load_failed).count()
    }

    /// Loads one pending plugin (so the UI can stay responsive between
    /// plugins). Returns its name, plus an error message if it failed.
    pub fn instantiate_next_plugin(&mut self, sample_rate: f64) -> Option<(String, Option<String>)> {
        let p = self.plugins_mut().into_iter().find(|p| p.instance.is_none() && !p.load_failed)?;
        let name = p.info.name.clone();
        let error = match crate::plugin::PluginInstance::new(&p.info, sample_rate) {
            Ok(inst) => {
                let rejected = !p.state.is_empty() && !inst.set_state(&p.state);
                p.instance = Some(inst);
                rejected.then(|| format!("{name}: the saved settings were not accepted"))
            }
            Err(e) => {
                p.load_failed = true;
                Some(format!("{name}: {e}"))
            }
        };
        Some((name, error))
    }

    /// Copies each live plugin's current state into the saved state.
    pub fn capture_plugin_states(&mut self) {
        for p in self.plugins_mut() {
            if let Some(inst) = &p.instance {
                p.state = inst.get_state();
            }
        }
    }

    /// A copy with fresh, independent plugin instances (for offline rendering).
    pub fn with_new_plugin_instances(&self, sample_rate: f64) -> (Project, Vec<String>) {
        let mut p = self.clone();
        p.capture_plugin_states();
        for pp in p.plugins_mut() {
            pp.instance = None;
        }
        let errors = p.instantiate_plugins(sample_rate);
        (p, errors)
    }

    pub fn save_json(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let s = serde_json::to_string_pretty(self)?;
        write_atomic(path, s.as_bytes())?;
        Ok(())
    }

    pub fn load_json(path: &std::path::Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        // Tolerate a UTF-8 byte-order mark (some editors add one).
        Ok(serde_json::from_str(s.strip_prefix('\u{feff}').unwrap_or(&s))?)
    }
}

fn default_tracks() -> Vec<PlaylistTrack> {
    (1..=24)
        .map(|i| PlaylistTrack { name: format!("Track {i}"), color: Rgb(0x48, 0x51, 0x56), muted: false })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_follow_signature_markers() {
        let mut p = Project::empty();
        let sig = |tick, n, d| TimeMarker { tick, name: format!("{n}/{d}"), kind: TimeMarker::SIGNATURE, numerator: n, denominator: d };
        p.playlist.markers = vec![sig(768, 7, 4), sig(768 + 672, 3, 4)];
        let bars = p.bars(0, 2400);
        let starts: Vec<(u32, u32, u32)> = bars.iter().map(|b| (b.0, b.1, b.2)).collect();
        assert_eq!(&starts[..5], &[(0, 1, 384), (384, 2, 384), (768, 3, 672), (1440, 4, 288), (1728, 5, 288)]);
        // Starting mid-song gives the right bar numbers.
        assert_eq!(p.bars(1500, 1800)[0], (1440, 4, 288, 96));
        // Plain markers don't change the grid.
        p.playlist.markers = vec![TimeMarker { tick: 100, name: "Verse".into(), kind: 0, numerator: 0, denominator: 0 }];
        assert!(p.bars(0, 2000).iter().all(|b| b.2 == 384));
    }

    #[test]
    fn set_target_uses_automation_scale() {
        let mut p = Project::new_template();
        assert!(p.set_target(AutoTarget::ChannelPan(0), 0.0));
        assert_eq!(p.channels[0].pan, -1.0);
        assert!(p.set_target(AutoTarget::MixerVolume(1), 0.8));
        assert!((p.mixer[1].volume - 1.0).abs() < 1e-6); // fader 0.8 = 0 dB
        assert!(p.set_target(AutoTarget::Tempo, 0.5));
        assert_eq!(p.tempo, 120.0);
        assert!(!p.set_target(AutoTarget::ChannelVolume(999), 0.5));
        assert!(!p.set_target(AutoTarget::PluginParam { channel: 0, param: 0 }, 0.5));
    }

    #[test]
    fn step_toggle() {
        let mut p = Pattern::new("p", PALETTE[0]);
        p.toggle_step(0, 4);
        assert!(p.step_on(0, 4));
        p.toggle_step(0, 4);
        assert!(!p.step_on(0, 4));
    }

    #[test]
    fn fl_zero_length_steps() {
        let mut p = Pattern::new("p", PALETTE[0]);
        p.notes.push(Note { channel: 0, key: 60, start: STEP_TICKS * 4, length: 0, velocity: 0.78, pan: 0.0 });
        assert!(p.is_step_only(0));
        assert!(p.step_on(0, 4));
        assert_eq!(p.notes[0].end(), STEP_TICKS * 5);
    }


    #[test]
    fn automation_interpolation() {
        let a = AutomationParams {
            points: vec![
                AutoPoint { tick: 0, value: 0.0, tension: 0.0 },
                AutoPoint { tick: 100, value: 1.0, tension: 0.0 },
                AutoPoint { tick: 200, value: 1.0, tension: 0.5 },
            ],
            target: Some(AutoTarget::ChannelVolume(0)),
        };
        assert_eq!(a.value_at(-5.0), Some(0.0));
        assert!((a.value_at(50.0).unwrap() - 0.5).abs() < 1e-4);
        assert_eq!(a.value_at(150.0), Some(1.0));
        assert_eq!(a.value_at(500.0), Some(1.0));
    }
    #[test]
    fn remove_channel_reindexes() {
        let mut p = Project::new_template();
        p.patterns[0].toggle_step(1, 0);
        p.patterns[0].toggle_step(3, 2);
        p.remove_channel(1);
        assert_eq!(p.patterns[0].notes.len(), 1);
        assert_eq!(p.patterns[0].notes[0].channel, 2);
    }

    #[test]
    fn lengths() {
        let mut p = Project::empty();
        assert_eq!(p.pattern_length(0), PPQ * 4);
        p.patterns[0].notes.push(Note { channel: 0, key: 60, start: PPQ * 5, length: PPQ, velocity: 1.0, pan: 0.0 });
        assert_eq!(p.pattern_length(0), PPQ * 8);
    }
}

/// Where an imported project came from, so edits can be written back into
/// the original FLP without losing data Goaty Loops doesn't model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlOrigin {
    pub path: PathBuf,
    /// The arrangement imported into the playlist.
    pub arrangement: Option<u16>,
    /// The file's PPQ (Goaty Loops uses 96 internally).
    pub ppq: u16,
}

/// The effect in FL slot `slot` of mixer `track`.
fn fl_slot_mut(mixer: &mut [MixerTrack], track: usize, slot: u8) -> Option<&mut EffectSlot> {
    mixer.get_mut(track)?.effects.iter_mut().find(|s| s.fl.as_ref().map(|f| f.slot) == Some(slot))
}

fn unity_gain() -> f32 {
    1.0
}

/// Writes `data` to a temporary file next to `path`, flushes it to disk, then
/// renames it over `path`. A crash or power loss mid-save leaves either the
/// old file or the new one, never a truncated mix.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod write_tests {
    #[test]
    fn atomic_write_replaces() {
        let p = std::env::temp_dir().join(format!("goaty_atomic_{}.goaty", std::process::id()));
        super::write_atomic(&p, b"first").unwrap();
        super::write_atomic(&p, b"second").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"second");
        let mut tmp = p.clone().into_os_string();
        tmp.push(".tmp");
        assert!(!std::path::Path::new(&tmp).exists());
        let _ = std::fs::remove_file(p);
    }
}
