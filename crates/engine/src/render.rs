//! The sequencer + mixer. Used both by the real-time audio thread and by
//! offline export.

use crate::audio::{Command, Shared};
use crate::dsp::pan_gains;
use crate::effects::FxState;
use crate::instruments::{ClipFade, NoteParams, Voice, PREVIEW_CHANNEL};
use crate::model::{ClipSource, Instrument, Project, SamplerParams, PPQ, STEP_TICKS};
use crate::plugin::{NoteEvent, TimeInfo};
use crossbeam_channel::Receiver;

const BLOCK: usize = 512;
const MAX_VOICES: usize = 256;
const MAX_PLUGIN_NOTES: usize = 1024;
const MAX_PLUGIN_EVENTS: usize = 256;
const MAX_AUTOS: usize = 512;

#[derive(Clone, Copy)]
struct PluginNote {
    channel: usize,
    key: u8,
    /// Samples until note-off, from the start of the current block.
    remaining: i64,
    held: bool,
}

#[derive(Default)]
struct Bus {
    l: Vec<f32>,
    r: Vec<f32>,
    /// Samples in a row with silent input and silent output after effects
    /// (smart disable: the track sleeps once this reaches [`SLEEP_AFTER_SECS`]).
    quiet: u32,
}

/// Peak level treated as silence (-120 dB).
const SILENCE: f32 = 1e-6;
/// A mixer track whose input and output stay silent this long stops running
/// its effects until audio arrives again. Long enough for delay echoes and
/// reverb tails, which can follow seconds of silence.
const SLEEP_AFTER_SECS: f32 = 3.0;

fn is_silent(l: &[f32], r: &[f32]) -> bool {
    l.iter().chain(r).all(|x| x.abs() < SILENCE)
}

pub struct Renderer {
    sr: f32,
    voices: Vec<Voice>,
    next_age: u64,
    seed: u32,
    buses: Vec<Bus>,
    fx: Vec<Vec<FxState>>,
    /// Playback position in ticks.
    pos: f64,
    /// Loop the song/pattern at its end (disabled for export).
    pub loop_enabled: bool,
    /// Mixer tracks whose processed output is copied out each block (stem export).
    pub stem_tracks: Vec<usize>,
    /// Interleaved stereo of each stem track for the last `process` call.
    pub stems: Vec<Vec<f32>>,
    was_playing: bool,
    just_started: bool,
    limiter_gain: f32,
    track_eq: Vec<FxState>,
    plugin_events: Vec<Vec<NoteEvent>>,
    plugin_notes: Vec<PluginNote>,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    time: TimeInfo,
    samples_played: i64,
    levels: Vec<usize>,
    autos: Vec<AutoValue>,
    /// Live pitch bend per channel, in semitones (MIDI pitch wheel).
    bend: Vec<f32>,
    /// Plugin delay compensation: effect latency per track, the planned
    /// input/output latencies, and delays for direct audio and sends.
    /// Mixer routes this block: (source, destination, gain).
    edges: Vec<(usize, usize, f32)>,
    /// Sidechain: the source track feeding each track's Limiter, and a copy of
    /// its output (taken before the track is processed).
    sc_src: Vec<Option<usize>>,
    sc_bufs: Vec<Bus>,
    pdc_own: Vec<usize>,
    pdc_in: Vec<usize>,
    pdc_out: Vec<usize>,
    pdc_direct: Vec<PdcDelay>,
    pdc_send: Vec<PdcDelay>,
    /// Instrument latency compensation: per-channel delays for plugin
    /// instruments and per-bus delays for the built-in voices.
    inst_pdc: Vec<PdcDelay>,
    voice_pdc: Vec<PdcDelay>,
    rx: Option<Receiver<Command>>,
    /// Runs the mixer tracks of one routing level in parallel.
    pool: crate::pool::WorkerPool,
    /// The project version being played. Replaced versions go back to the UI
    /// thread to be freed (see `Shared::retire`).
    current: Option<std::sync::Arc<Project>>,
    /// Tracks on the routing level being processed (reused, no allocation).
    level_tracks: Vec<usize>,
}

/// Per-track mixer state handed to the pool. Each task touches only its own
/// track index, so the mutable accesses never overlap.
struct TrackPtrs {
    bus: *mut Bus,
    fx: *mut Vec<FxState>,
    eq: *mut FxState,
}

unsafe impl Sync for TrackPtrs {}

impl TrackPtrs {
    /// # Safety
    /// `t` must be in bounds, and no two concurrent calls may use the same `t`.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self, t: usize) -> (&mut Bus, &mut Vec<FxState>, &mut FxState) {
        unsafe { (&mut *self.bus.add(t), &mut *self.fx.add(t), &mut *self.eq.add(t)) }
    }
}

impl Renderer {
    pub fn new(sample_rate: u32, rx: Option<Receiver<Command>>) -> Self {
        Self {
            sr: sample_rate as f32,
            voices: Vec::with_capacity(MAX_VOICES),
            next_age: 0,
            seed: 0x9e37_79b9,
            buses: Vec::new(),
            fx: Vec::new(),
            pos: 0.0,
            loop_enabled: true,
            stem_tracks: Vec::new(),
            stems: Vec::new(),
            was_playing: false,
            just_started: false,
            limiter_gain: 1.0,
            track_eq: Vec::new(),
            plugin_events: Vec::new(),
            plugin_notes: Vec::with_capacity(MAX_PLUGIN_NOTES),
            scratch_l: vec![0.0; BLOCK],
            scratch_r: vec![0.0; BLOCK],
            time: TimeInfo::default(),
            samples_played: 0,
            levels: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            autos: Vec::with_capacity(MAX_AUTOS),
            bend: vec![0.0; crate::model::MAX_CHANNELS],
            edges: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS * 4),
            sc_src: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            sc_bufs: Vec::new(),
            pdc_own: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            pdc_in: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            pdc_out: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            pdc_direct: Vec::new(),
            pdc_send: Vec::new(),
            inst_pdc: Vec::new(),
            voice_pdc: Vec::new(),
            rx,
            pool: crate::pool::WorkerPool::new(crate::pool::WorkerPool::default_workers()),
            level_tracks: Vec::with_capacity(crate::model::MAX_MIXER_TRACKS),
            current: None,
        }
    }

    pub fn position(&self) -> f64 {
        self.pos
    }

    fn samples_per_tick(&self, tempo: f64) -> f64 {
        self.sr as f64 * 60.0 / (tempo.max(1.0) * crate::model::PPQ as f64)
    }

    /// Renders stereo interleaved audio into `out`.
    pub fn process(&mut self, shared: &Shared, out: &mut [f32]) {
        self.stems.resize_with(self.stem_tracks.len(), Vec::new);
        for s in &mut self.stems {
            s.clear();
        }
        let project = shared.project.load_full();
        if !self.current.as_ref().is_some_and(|c| std::sync::Arc::ptr_eq(c, &project)) {
            if let Some(old) = self.current.replace(project.clone()) {
                shared.retire(crate::audio::Retired::Project(old));
            }
        }
        let t = &shared.transport;

        if let Some(rx) = self.rx.take() {
            while let Ok(cmd) = rx.try_recv() {
                self.handle_command(&project, cmd);
            }
            self.rx = Some(rx);
        }

        if let Some(seek) = t.take_seek() {
            self.pos = seek.max(0.0);
            self.kill_sequenced();
            self.just_started = true;
        }

        let playing = t.playing.load(std::sync::atomic::Ordering::Relaxed);
        if playing && !self.was_playing {
            self.just_started = true;
        }
        if !playing && self.was_playing {
            self.kill_sequenced();
        }
        self.was_playing = playing;

        let nbus = project.mixer.len().max(1);
        if self.buses.len() != nbus {
            self.buses.resize_with(nbus, || Bus { l: vec![0.0; BLOCK], r: vec![0.0; BLOCK], quiet: 0 });
        }
        self.sync_fx(&project, shared);
        if self.plugin_events.len() < project.channels.len() {
            self.plugin_events.resize_with(project.channels.len(), || Vec::with_capacity(MAX_PLUGIN_EVENTS));
        }

        let song = t.song_mode.load(std::sync::atomic::Ordering::Relaxed);
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            if playing && song {
                self.update_automation(&project);
            }
            let tempo = auto_lookup(&self.autos, crate::model::AutoTarget::Tempo).map(|v| 60.0 + v as f64 * 120.0).unwrap_or(project.tempo);
            let spt = self.samples_per_tick(tempo);
            for b in &mut self.buses {
                b.l[..n].fill(0.0);
                b.r[..n].fill(0.0);
            }
            if playing {
                self.advance(&project, shared, n as f64 / spt, spt);
            }
            self.time = TimeInfo {
                sample_rate: self.sr as f64,
                tempo,
                playing,
                ppq: self.pos / PPQ as f64,
                project_samples: (self.pos * spt) as i64,
                time_sig: project.time_sig,
            };
            self.samples_played += n as i64;
            // Built-in voices first, delayed to match the slowest plugin instrument;
            // then each plugin instrument, delayed by the rest.
            let inst_latency = instrument_latency(&project);
            self.render_voices(&project, shared, n);
            self.voice_pdc.resize_with(self.buses.len(), Default::default);
            for (b, d) in self.buses.iter_mut().zip(self.voice_pdc.iter_mut()) {
                d.set(inst_latency);
                d.process(&mut b.l[..n], &mut b.r[..n]);
            }
            self.render_plugins(&project, shared, n, inst_latency);
            self.mix(&project, shared, &mut out[done * 2..(done + n) * 2], n);
            let total = inst_latency + self.pdc_in.first().copied().unwrap_or(0);
            shared.latency_samples.store(total as u32, std::sync::atomic::Ordering::Relaxed);
            done += n;
        }
        t.set_position(self.pos);
    }

    fn handle_command(&mut self, project: &Project, cmd: Command) {
        match cmd {
            Command::NoteOn { channel, key, velocity } => {
                if let Some(ch) = project.channels.get(channel) {
                    let p = NoteParams { key, velocity, pan: 0.0, gate: None, delay: 0, offset_secs: 0.0, tempo: project.tempo, fade: None };
                    self.start_voice(channel, &ch.instrument, &p);
                }
            }
            Command::NoteOff { channel, key } => {
                for v in &mut self.voices {
                    if v.held && v.channel == channel && v.key == key {
                        v.note_off();
                    }
                }
                self.plugin_notes_off(|n| n.held && n.channel == channel && n.key == key);
            }
            Command::PreviewSample(data) => {
                self.stop_preview();
                let inst = Instrument::Sampler(SamplerParams { data: Some(data), ..Default::default() });
                let p = NoteParams { key: 60, velocity: 1.0, pan: 0.0, gate: None, delay: 0, offset_secs: 0.0, tempo: project.tempo, fade: None };
                self.start_voice(PREVIEW_CHANNEL, &inst, &p);
            }
            Command::StopPreview => self.stop_preview(),
            Command::SetParam { target, value } => {
                // Same path as automation: holds until automation moves it again.
                if let Some(a) = self.autos.iter_mut().find(|a| a.target == target) {
                    a.value = value;
                    a.changed = true;
                } else if self.autos.len() < MAX_AUTOS {
                    self.autos.push(AutoValue { target, value, changed: true });
                }
            }
            Command::PitchBend { channel, semis, wheel } => {
                if let Some(b) = self.bend.get_mut(channel) {
                    *b = semis;
                }
                if let Some(Instrument::Plugin(crate::model::PluginParams { instance: Some(inst), .. })) = project.channels.get(channel).map(|c| &c.instrument) {
                    inst.midi_controller(129, wheel as f64);
                }
            }
            Command::AllNotesOff => {
                for v in &mut self.voices {
                    v.kill();
                }
                self.plugin_notes_off(|_| true);
            }
        }
    }


    fn plugin_note_on(&mut self, channel: usize, p: &NoteParams) {
        if channel >= self.plugin_events.len() {
            return;
        }
        let ev = &mut self.plugin_events[channel];
        if ev.len() < MAX_PLUGIN_EVENTS {
            ev.push(NoteEvent::On { offset: p.delay, key: p.key, velocity: p.velocity.clamp(0.0, 1.0) });
        }
        if self.plugin_notes.len() < MAX_PLUGIN_NOTES {
            let remaining = p.gate.map(|g| p.delay as i64 + g as i64).unwrap_or(i64::MAX);
            self.plugin_notes.push(PluginNote { channel, key: p.key, remaining, held: p.gate.is_none() });
        }
    }

    /// Releases tracked plugin notes matching `pred` at the start of the next block.
    fn plugin_notes_off(&mut self, pred: impl Fn(&PluginNote) -> bool) {
        let mut i = 0;
        while i < self.plugin_notes.len() {
            let pn = self.plugin_notes[i];
            if pred(&pn) {
                if let Some(ev) = self.plugin_events.get_mut(pn.channel) {
                    ev.push(NoteEvent::Off { offset: 0, key: pn.key });
                }
                self.plugin_notes.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Runs every plugin instrument for `n` frames and mixes it into its bus.
    fn render_plugins(&mut self, project: &Project, shared: &Shared, n: usize, inst_latency: usize) {
        self.inst_pdc.resize_with(project.channels.len(), Default::default);
        // Scheduled note-offs that fall inside this block.
        let mut i = 0;
        while i < self.plugin_notes.len() {
            let pn = &mut self.plugin_notes[i];
            if pn.remaining < n as i64 {
                let (ch, key, off) = (pn.channel, pn.key, pn.remaining.max(0) as u32);
                if let Some(ev) = self.plugin_events.get_mut(ch) {
                    ev.push(NoteEvent::Off { offset: off, key });
                }
                self.plugin_notes.swap_remove(i);
            } else {
                if pn.remaining != i64::MAX {
                    pn.remaining -= n as i64;
                }
                i += 1;
            }
        }
        let last_bus = self.buses.len() - 1;
        for (ci, ch) in project.channels.iter().enumerate() {
            let Instrument::Plugin(pp) = &ch.instrument else { continue };
            let Some(inst) = &pp.instance else { continue };
            let Some(events) = self.plugin_events.get_mut(ci) else { continue };
            events.sort_unstable_by_key(|e| e.offset());
            self.scratch_l[..n].fill(0.0);
            self.scratch_r[..n].fill(0.0);
            let ok = inst.process(n, events, &mut self.scratch_l[..n], &mut self.scratch_r[..n], &self.time);
            if ok {
                events.clear();
            }
            // Line this instrument up with the slowest one.
            let pdc = &mut self.inst_pdc[ci];
            pdc.set(inst_latency.saturating_sub(inst.latency as usize));
            if ok {
                pdc.process(&mut self.scratch_l[..n], &mut self.scratch_r[..n]);
            }
            if !ok || ch.muted {
                continue;
            }
            let (gain, pan) = channel_levels(&self.autos, ci, ch);
            let (pl, pr) = pan_gains(pan);
            let bus = &mut self.buses[ch.mixer_track.min(last_bus)];
            let mut peak = 0f32;
            for k in 0..n {
                let (l, r) = (self.scratch_l[k] * gain * pl, self.scratch_r[k] * gain * pr);
                bus.l[k] += l;
                bus.r[k] += r;
                peak = peak.max(l.abs()).max(r.abs());
            }
            shared.meters.bump_channel(ci, peak);
        }
    }

    /// Evaluates automation clips at the current position. Values persist
    /// after a clip ends, like FL Studio.
    fn update_automation(&mut self, project: &Project) {
        use crate::model::{AutoTarget, Effect};
        for a in &mut self.autos {
            a.changed = false;
        }
        let pos = self.pos as f32;
        for clip in &project.playlist.clips {
            let ClipSource::Audio(ch) = clip.source else { continue };
            if clip.muted || pos < clip.start as f32 || pos >= clip.end() as f32 {
                continue;
            }
            let Some(Instrument::Automation(auto)) = project.channels.get(ch).map(|c| &c.instrument) else { continue };
            let Some(target) = auto.target else { continue };
            let Some(value) = auto.value_at(pos - clip.start as f32 + clip.offset as f32) else { continue };
            if let Some(a) = self.autos.iter_mut().find(|a| a.target == target) {
                if (a.value - value).abs() > 1e-5 {
                    a.value = value;
                    a.changed = true;
                }
            } else if self.autos.len() < MAX_AUTOS {
                self.autos.push(AutoValue { target, value, changed: true });
            }
        }
        // Forward changed values to VST3 plugins.
        for a in self.autos.iter().filter(|a| a.changed) {
            match a.target {
                AutoTarget::PluginParam { channel, param } => {
                    if let Some(Instrument::Plugin(p)) = project.channels.get(channel).map(|c| &c.instrument) {
                        if let Some(inst) = &p.instance {
                            inst.set_param_index(param, a.value as f64);
                        }
                    }
                }
                AutoTarget::EffectParam { track, slot, param } => {
                    let fx = project.mixer.get(track).and_then(|t| t.slot_by_id(slot));
                    if let Some(Effect::Plugin(p)) = fx.map(|e| &e.effect) {
                        if let Some(inst) = &p.instance {
                            inst.set_param_index(param, a.value as f64);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn stop_preview(&mut self) {
        for v in &mut self.voices {
            if v.channel == PREVIEW_CHANNEL {
                v.kill();
            }
        }
    }

    fn kill_sequenced(&mut self) {
        for v in &mut self.voices {
            if !v.held && v.channel != PREVIEW_CHANNEL {
                v.kill();
            }
        }
        self.plugin_notes_off(|n| !n.held);
    }

    fn start_voice(&mut self, channel: usize, inst: &Instrument, p: &NoteParams) {
        if let Instrument::Plugin(pp) = inst {
            if pp.instance.is_some() {
                self.plugin_note_on(channel, p);
            }
            return;
        }
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let Some(mut v) = Voice::new(channel, inst, p, self.sr, self.seed) else { return };
        v.age = self.next_age;
        self.next_age += 1;
        if self.voices.len() >= MAX_VOICES {
            // Steal the oldest voice.
            if let Some((i, _)) = self.voices.iter().enumerate().min_by_key(|(_, v)| v.age) {
                self.voices[i] = v;
            }
        } else {
            self.voices.push(v);
        }
    }

    /// Moves the play position forward by `dur` ticks, triggering notes.
    fn advance(&mut self, project: &Project, shared: &Shared, dur: f64, spt: f64) {
        let t = &shared.transport;
        let song = t.song_mode.load(std::sync::atomic::Ordering::Relaxed);
        let pattern = t.pattern.load(std::sync::atomic::Ordering::Relaxed);
        let len = if song { project.song_length() } else { project.pattern_length(pattern) } as f64;

        if self.pos >= len && self.loop_enabled {
            self.pos %= len.max(1.0);
        }
        let mut a = self.pos;
        let mut remaining = dur;
        let mut elapsed = 0.0;
        loop {
            let b = if self.loop_enabled { (a + remaining).min(len) } else { a + remaining };
            if a < len {
                self.trigger_range(project, song, pattern, a, b.min(len), elapsed, spt);
            }
            self.just_started = false;
            elapsed += b - a;
            remaining -= b - a;
            a = b;
            if remaining <= 1e-9 {
                break;
            }
            a = 0.0;
        }
        self.pos = if self.loop_enabled && a >= len { 0.0 } else { a };
    }

    #[allow(clippy::too_many_arguments)]
    fn trigger_range(&mut self, project: &Project, song: bool, pattern: usize, a: f64, b: f64, elapsed: f64, spt: f64) {
        // Tempo in effect (samples per tick already includes tempo automation).
        let tempo = 60.0 * self.sr as f64 / (spt * PPQ as f64);
        let delay_of = |tick: f64| ((elapsed + tick - a) * spt).max(0.0) as u32;
        let gate_of = |len: u32| ((if len == 0 { STEP_TICKS } else { len }) as f64 * spt) as u64;
        let in_range = |tick: f64| tick >= a && tick < b;

        if !song {
            let Some(pat) = project.patterns.get(pattern) else { return };
            for n in &pat.notes {
                if in_range(n.start as f64) {
                    let p = NoteParams {
                        key: n.key,
                        velocity: n.velocity,
                        pan: n.pan,
                        gate: Some(gate_of(n.length)),
                        delay: delay_of(n.start as f64),
                        offset_secs: 0.0,
                        tempo,
                        fade: None,
                    };
                    self.note_on(project, n.channel, &p);
                }
            }
            return;
        }

        for clip in &project.playlist.clips {
            if clip.muted || project.playlist.tracks.get(clip.track).map(|t| t.muted).unwrap_or(false) {
                continue;
            }
            let (cs, ce) = (clip.start as f64, clip.end() as f64);
            if ce <= a || cs >= b {
                continue;
            }
            match clip.source {
                ClipSource::Pattern(pi) => {
                    let Some(pat) = project.patterns.get(pi) else { continue };
                    for n in &pat.notes {
                        if n.start < clip.offset || n.start >= clip.offset + clip.length {
                            continue;
                        }
                        let tick = (clip.start + n.start - clip.offset) as f64;
                        if in_range(tick) {
                            let len = n.length.max(1).min(clip.end() - tick as u32);
                            let p = NoteParams {
                                key: n.key,
                                velocity: n.velocity,
                                pan: n.pan,
                                gate: Some(gate_of(len)),
                                delay: delay_of(tick),
                                offset_secs: 0.0,
                                tempo,
                                fade: None,
                            };
                            self.note_on(project, n.channel, &p);
                        }
                    }
                }
                ClipSource::Audio(ch) => {
                    let start = if in_range(cs) {
                        Some((cs, 0.0))
                    } else if self.just_started && cs < a {
                        Some((a, a - cs))
                    } else {
                        None
                    };
                    if let Some((tick, into)) = start {
                        let secs_per_tick = spt / self.sr as f64;
                        let p = NoteParams {
                            key: sampler_root(project, ch),
                            // The clip's own volume (FL 21+), linear.
                            velocity: clip.gain,
                            pan: 0.0,
                            gate: Some(((ce - tick) * spt) as u64),
                            delay: delay_of(tick),
                            offset_secs: (into + clip.offset as f64) * secs_per_tick,
                            tempo,
                            fade: (clip.fade_in > 0 || clip.fade_out > 0).then(|| ClipFade {
                                elapsed: (into * spt) as u64,
                                length: (clip.length as f64 * spt) as u64,
                                fade_in: (clip.fade_in as f64 * spt) as u64,
                                fade_out: (clip.fade_out as f64 * spt) as u64,
                            }),
                        };
                        self.note_on(project, ch, &p);
                    }
                }
            }
        }
    }

    fn note_on(&mut self, project: &Project, channel: usize, p: &NoteParams) {
        let Some(ch) = project.channels.get(channel) else { return };
        if ch.muted {
            return;
        }
        if ch.cut_group != 0 {
            let g = ch.cut_group;
            for v in &mut self.voices {
                if v.channel != PREVIEW_CHANNEL && project.channels.get(v.channel).is_some_and(|c| c.cut_by == g) {
                    v.cut_at(p.delay);
                }
            }
        }
        self.start_voice(channel, &ch.instrument, p);
    }

    fn render_voices(&mut self, project: &Project, shared: &Shared, n: usize) {
        let last_bus = self.buses.len() - 1;
        for v in &mut self.voices {
            let (gain, pan, track) = if v.channel == PREVIEW_CHANNEL {
                (1.0, 0.0, 0)
            } else {
                match project.channels.get(v.channel) {
                    Some(ch) => {
                        let (g, p) = channel_levels(&self.autos, v.channel, ch);
                        (g, p, ch.mixer_track.min(last_bus))
                    }
                    None => {
                        v.kill();
                        (0.0, 0.0, 0)
                    }
                }
            };
            let (gl, gr) = pan_gains(pan);
            let bus = &mut self.buses[track];
            let bend = self.bend.get(v.channel).copied().unwrap_or(0.0);
            let pitch = project.channels.get(v.channel).map(|ch| channel_pitch(&self.autos, v.channel, ch, bend)).unwrap_or(1.0);
            let peak = v.render(&mut bus.l[..n], &mut bus.r[..n], gl * gain, gr * gain, pitch);
            if v.channel != PREVIEW_CHANNEL {
                shared.meters.bump_channel(v.channel, peak);
            }
        }
        self.voices.retain(|v| !v.is_done());
    }

    fn sync_fx(&mut self, project: &Project, shared: &Shared) {
        // Old effect states may hold the last reference to a plugin: retire them
        // rather than dropping them here.
        while self.fx.len() > project.mixer.len() {
            if let Some(v) = self.fx.pop() {
                shared.retire(crate::audio::Retired::Effects(v));
            }
        }
        self.fx.resize_with(project.mixer.len(), Vec::new);
        let sr = self.sr;
        self.track_eq.resize_with(project.mixer.len(), || FxState::new(&crate::model::Effect::Eq3 { low_db: 0.0, mid_db: 0.0, high_db: 0.0 }, sr));
        for (t, track) in project.mixer.iter().enumerate() {
            let states = &mut self.fx[t];
            let ok = states.len() == track.effects.len()
                && states.iter().zip(&track.effects).all(|(s, e)| s.matches(&e.effect));
            if !ok {
                let new = track.effects.iter().map(|e| FxState::new(&e.effect, self.sr)).collect();
                shared.retire(crate::audio::Retired::Effects(std::mem::replace(states, new)));
            }
        }
    }

    fn mix(&mut self, project: &Project, shared: &Shared, out: &mut [f32], n: usize) {
        let nt = self.buses.len().min(project.mixer.len());

        // Routing graph: each track's main output plus its sends (FL routes an
        // insert to any number of others, in either direction).
        self.edges.clear();
        for (t, track) in project.mixer.iter().enumerate().take(nt).skip(1) {
            let main = if track.output < nt && track.output != t { track.output } else { 0 };
            self.edges.push((t, main, 1.0));
            for s in &track.sends {
                if s.to < nt && s.to != t && s.to != main {
                    self.edges.push((t, s.to, s.gain));
                }
            }
        }
        // Levels: a track runs after every track feeding it (longest path).
        // Edges that would close a loop never satisfy this and are skipped.
        route_levels(nt, &self.edges, &mut self.levels);
        let max_level = self.levels.iter().copied().max().unwrap_or(0);

        // Sidechains: a Limiter with sidechain input n listens to the n-th track
        // (by index) routed into its track, which is processed earlier.
        self.sc_src.clear();
        for t in 0..nt {
            let input = project.mixer[t].effects.iter().find_map(|e| match e.effect {
                crate::model::Effect::Limiter { sidechain, .. } if sidechain > 0 && e.enabled => Some(sidechain as usize),
                _ => None,
            });
            let edges = &self.edges;
            let levels = &self.levels;
            let src = input.and_then(|k| (1..nt).filter(|&s| s != t && levels[s] < levels[t] && edges.iter().any(|&(a, b, _)| a == s && b == t)).nth(k - 1));
            self.sc_src.push(src);
        }
        if self.sc_src.iter().any(Option::is_some) && self.sc_bufs.len() < nt {
            self.sc_bufs.resize_with(nt, || Bus { l: vec![0.0; BLOCK], r: vec![0.0; BLOCK], quiet: 0 });
        }

        // Plugin delay compensation: delay each track's direct audio, and each
        // route, so all paths reach their destination in step.
        self.pdc_own.clear();
        self.pdc_own.extend(project.mixer[..nt].iter().map(track_latency));
        pdc_plan(&self.pdc_own, &self.levels, &self.edges, &mut self.pdc_in, &mut self.pdc_out);
        self.pdc_direct.resize_with(nt, Default::default);
        self.pdc_send.resize_with(self.edges.len(), Default::default);
        for t in 0..nt {
            self.pdc_direct[t].set(self.pdc_in[t]);
            let b = &mut self.buses[t];
            self.pdc_direct[t].process(&mut b.l[..n], &mut b.r[..n]);
        }
        for (k, &(s, d, _)) in self.edges.iter().enumerate() {
            self.pdc_send[k].set(self.pdc_in[d].saturating_sub(self.pdc_out[s]));
        }

        let (sr, time) = (self.sr, self.time);
        let playing = self.was_playing;
        let autos = &self.autos[..];

        for level in 0..=max_level {
            // Effects, EQ, volume and pan for every track on this level, in parallel.
            // Copy each sidechain source (already processed) for the tracks on this level.
            for t in 0..nt {
                if let (true, Some(s)) = (self.levels[t] == level, self.sc_src[t]) {
                    let (src, dst) = (&self.buses[s], &mut self.sc_bufs[t]);
                    dst.l[..n].copy_from_slice(&src.l[..n]);
                    dst.r[..n].copy_from_slice(&src.r[..n]);
                }
            }
            let (sc_src, sc_bufs) = (&self.sc_src, &self.sc_bufs);
            // Iterated in place (no per-block job list), so the audio thread doesn't allocate.
            let levels = &self.levels;
            let heavy = (0..nt).filter(|&t| levels[t] == level && !project.mixer[t].effects.is_empty()).count() > 1;
            let run = |(t, ((bus, fx), eq)): (usize, ((&mut Bus, &mut Vec<FxState>), &mut FxState))| {
                if levels[t] != level {
                    return;
                }
                let sc = sc_src[t].map(|_| (&sc_bufs[t].l[..n], &sc_bufs[t].r[..n]));
                process_track(t, &project.mixer[t], bus, fx, eq, n, sr, &time, autos, sc, playing);
                let (pl, pr) = bus.l[..n].iter().zip(&bus.r[..n]).fold((0f32, 0f32), |(a, b), (l, r)| (a.max(l.abs()), b.max(r.abs())));
                shared.meters.bump_mixer(t, pl, pr);
            };
            if heavy {
                self.level_tracks.clear();
                self.level_tracks.extend((0..nt).filter(|&t| levels[t] == level));
                let ptrs = TrackPtrs { bus: self.buses.as_mut_ptr(), fx: self.fx.as_mut_ptr(), eq: self.track_eq.as_mut_ptr() };
                let tracks = &self.level_tracks;
                self.pool.run(tracks.len(), &|k| {
                    let t = tracks[k];
                    // SAFETY: t < nt <= every vector's length, and each index runs once.
                    let (bus, fx, eq) = unsafe { ptrs.get(t) };
                    run((t, ((bus, fx), eq)));
                });
            } else {
                let (buses, fx, eq) = (&mut self.buses[..nt], &mut self.fx[..nt], &mut self.track_eq[..nt]);
                buses.iter_mut().zip(fx.iter_mut()).zip(eq.iter_mut()).enumerate().for_each(run);
            }
            // Stem export: copy each requested track after its effects and fader.
            for (k, &t) in self.stem_tracks.iter().enumerate() {
                if t < nt && self.levels[t] == level {
                    let (b, s) = (&self.buses[t], &mut self.stems[k]);
                    for i in 0..n {
                        s.push(b.l[i]);
                        s.push(b.r[i]);
                    }
                }
            }
            // Sum this level into its destinations, through each route's delay and gain.
            for (k, &(s, d, gain)) in self.edges.iter().enumerate() {
                if self.levels[s] != level || self.levels[d] <= level {
                    continue;
                }
                let (sl, sr_) = (&mut self.scratch_l[..n], &mut self.scratch_r[..n]);
                sl.copy_from_slice(&self.buses[s].l[..n]);
                sr_.copy_from_slice(&self.buses[s].r[..n]);
                self.pdc_send[k].process(sl, sr_);
                let dst = &mut self.buses[d];
                for i in 0..n {
                    dst.l[i] += sl[i] * gain;
                    dst.r[i] += sr_[i] * gain;
                }
            }
        }
        // Safety limiter: instant attack, ~150 ms release, ceiling just below 0 dBFS.
        const CEILING: f32 = 0.97;
        let release = 1.0 - (-1.0 / (0.15 * self.sr)).exp();
        let main = auto_lookup(&self.autos, crate::model::AutoTarget::MainVolume).map(|v| { let g = v / crate::model::DEFAULT_CHANNEL_VOLUME; g * g }).unwrap_or(1.0);
        let master = &mut self.buses[0];
        if main != 1.0 {
            for i in 0..n {
                master.l[i] *= main;
                master.r[i] *= main;
            }
        }
        for i in 0..n {
            let (l, r) = (master.l[i], master.r[i]);
            let peak = l.abs().max(r.abs());
            let target = if peak * self.limiter_gain > CEILING { CEILING / peak } else { 1.0 };
            if target < self.limiter_gain {
                self.limiter_gain = target;
            } else {
                self.limiter_gain += (target.min(1.0) - self.limiter_gain) * release;
            }
            out[i * 2] = (l * self.limiter_gain).clamp(-1.0, 1.0);
            out[i * 2 + 1] = (r * self.limiter_gain).clamp(-1.0, 1.0);
        }
    }
}

fn sampler_root(project: &Project, ch: usize) -> u8 {
    match project.channels.get(ch).map(|c| &c.instrument) {
        Some(Instrument::Sampler(s)) => s.root_key,
        _ => 60,
    }
}

/// Effects, built-in EQ, volume and pan for one mixer track.
///
/// Smart disable: a track that has had silent input and output for a while
/// skips its effects. Built-in effects resume exactly as if they had kept
/// running. VST3 effects may keep adaptive state (pitch trackers, de-clickers),
/// so tracks holding one only sleep while the transport is stopped; playback
/// and exports then match a render without smart disable sample for sample.
#[allow(clippy::too_many_arguments)]
fn process_track(t: usize, track: &crate::model::MixerTrack, bus: &mut Bus, fx: &mut [FxState], eq: &mut FxState, n: usize, sr: f32, time: &TimeInfo, autos: &[AutoValue], sc: Option<(&[f32], &[f32])>, playing: bool) {
    use crate::model::AutoTarget;
    let silent_in = is_silent(&bus.l[..n], &bus.r[..n]);
    let may_sleep = !playing || !track.effects.iter().any(|e| matches!(e.effect, crate::model::Effect::Plugin(_)));
    if silent_in && may_sleep && bus.quiet as f32 >= SLEEP_AFTER_SECS * sr {
        return; // asleep: silence in, silence out, no effect processing
    }
    let (l, r) = (&mut bus.l[..n], &mut bus.r[..n]);
    for (slot, state) in track.effects.iter().zip(fx.iter_mut()) {
        let fl_slot = slot.fl.as_ref().map(|f| f.slot);
        let enabled = match fl_slot.and_then(|s| auto_lookup(autos, AutoTarget::SlotEnabled { track: t, slot: s })) {
            Some(v) => v >= 0.5,
            None => slot.enabled,
        };
        if !enabled {
            continue;
        }
        let mix = fl_slot.and_then(|s| auto_lookup(autos, AutoTarget::SlotMix { track: t, slot: s })).unwrap_or(slot.mix).clamp(0.0, 1.0);
        if mix <= 0.001 {
            continue;
        }
        let blend = mix < 0.999;
        let (mut dry_l, mut dry_r) = ([0f32; BLOCK], [0f32; BLOCK]);
        if blend {
            dry_l[..n].copy_from_slice(l);
            dry_r[..n].copy_from_slice(r);
        }
        process_slot(t, slot, state, l, r, sc, sr, time, autos);
        if blend {
            for i in 0..n {
                l[i] = dry_l[i] + (l[i] - dry_l[i]) * mix;
                r[i] = dry_r[i] + (r[i] - dry_r[i]) * mix;
            }
        }
    }
    bus.quiet = if silent_in && is_silent(l, r) { bus.quiet.saturating_add(n as u32) } else { 0 };
    if track.eq.iter().any(|g| g.abs() > 0.01) {
        let e = crate::model::Effect::Eq3 { low_db: track.eq[0], mid_db: track.eq[1], high_db: track.eq[2] };
        eq.process(&e, l, r, sr, time);
    }
    let vol = auto_lookup(autos, AutoTarget::MixerVolume(t)).map(crate::dsp::fader_to_gain).unwrap_or(track.volume);
    let pan = auto_lookup(autos, AutoTarget::MixerPan(t)).map(|v| v * 2.0 - 1.0).unwrap_or(track.pan);
    let g = if track.muted { 0.0 } else { vol };
    let (pl, pr) = pan_gains(pan);
    for i in 0..n {
        l[i] *= g * pl;
        r[i] *= g * pr;
    }
}


/// Runs one effect slot, re-decoding automated FL effects from their raw parameters.
#[allow(clippy::too_many_arguments)]
fn process_slot(t: usize, slot: &crate::model::EffectSlot, state: &mut FxState, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>, sr: f32, time: &TimeInfo, autos: &[AutoValue]) {
    use crate::model::{AutoTarget, Effect};
    if let Some(info) = slot.fl.as_ref().filter(|_| !matches!(slot.effect, Effect::Plugin(_))) {
        let params = || {
            autos.iter().filter_map(|a| match a.target {
                AutoTarget::EffectParam { track, slot: s, param } if track == t && s == info.slot => Some((param, a.value)),
                _ => None,
            })
        };
        if params().next().is_some() {
            if let Some(e) = crate::fl_effects::decode_with(&info.internal, &info.data, params()).filter(|e| state.matches(e)) {
                state.process_sc(&e, l, r, sc, sr, time);
                return;
            }
        }
    }
    state.process_sc(&slot.effect, l, r, sc, sr, time);
}
/// Current automated value (normalized) for a target.
#[derive(Clone, Copy)]
pub(crate) struct AutoValue {
    target: crate::model::AutoTarget,
    value: f32,
    changed: bool,
}

fn auto_lookup(autos: &[AutoValue], target: crate::model::AutoTarget) -> Option<f32> {
    autos.iter().find(|a| a.target == target).map(|a| a.value)
}

/// Channel volume (FL 0..1 scale) and pan with automation applied.
fn channel_levels(autos: &[AutoValue], ci: usize, ch: &crate::model::Channel) -> (f32, f32) {
    use crate::model::AutoTarget;
    let vol = auto_lookup(autos, AutoTarget::ChannelVolume(ci)).unwrap_or(ch.volume);
    let pan = auto_lookup(autos, AutoTarget::ChannelPan(ci)).map(|v| v * 2.0 - 1.0).unwrap_or(ch.pan);
    (if ch.muted { 0.0 } else { crate::dsp::channel_volume_to_gain(vol) }, pan)
}

/// Channel pitch as a frequency ratio, with automation applied.
fn channel_pitch(autos: &[AutoValue], ci: usize, ch: &crate::model::Channel, bend: f32) -> f32 {
    let semis = auto_lookup(autos, crate::model::AutoTarget::ChannelPitch(ci)).map(|v| (v * 2.0 - 1.0) * ch.pitch_range).unwrap_or(ch.pitch) + bend;
    if semis.abs() < 1e-4 {
        1.0
    } else {
        2f32.powf(semis / 12.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    /// Two hits on a self-cutting drum channel: the first must stop when the second starts.
    #[test]
    fn cut_group_stops_previous_voice() {
        let mut p = Project::empty();
        let mut ch = Channel::new("hat", Instrument::Drum(DrumParams { kind: DrumKind::OpenHat, tune: 0.0, decay: 4.0 }));
        ch.cut_group = 1;
        ch.cut_by = 1;
        p.channels.push(ch);
        p.patterns[0].notes.push(Note { channel: 0, key: 60, start: 0, length: STEP_TICKS, velocity: 1.0, pan: 0.0 });
        p.patterns[0].notes.push(Note { channel: 0, key: 60, start: PPQ, length: STEP_TICKS, velocity: 1.0, pan: 0.0 });
        let shared = Shared::new(p);
        shared.transport.playing.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut r = Renderer::new(48000, None);
        let mut buf = vec![0f32; 48000];
        // Render past the second hit (one beat at 130 BPM is ~22k samples).
        r.process(&shared, &mut buf);
        assert_eq!(r.voices.len(), 1, "the first open hat should have been cut");
    }
}

/// Most plugin delay compensated, per path (1 s at 48 kHz).
const MAX_PDC: usize = 48_000;

/// Processing order of the mixer graph: each track's level is one more than
/// the highest level feeding it (longest path), so it runs after all its
/// sources. A route that would close a loop can never be satisfied; levels
/// stop growing past the track count and such routes are skipped when mixing.
fn route_levels(nt: usize, edges: &[(usize, usize, f32)], levels: &mut Vec<usize>) {
    levels.clear();
    levels.resize(nt, 0);
    for _ in 0..nt {
        let mut changed = false;
        for &(s, d, _) in edges {
            if levels[d] <= levels[s] && levels[s] < nt {
                levels[d] = levels[s] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Plugin delay compensation plan for the mixer graph. `own[t]` is track t's
/// effect latency. Fills the input and output latency of each track: the
/// input latency is the slowest route arriving at the track, so every other
/// arrival must be delayed up to it.
fn pdc_plan(own: &[usize], levels: &[usize], edges: &[(usize, usize, f32)], input: &mut Vec<usize>, output: &mut Vec<usize>) {
    let n = own.len();
    input.clear();
    input.resize(n, 0);
    output.clear();
    output.resize(n, 0);
    let max_level = levels.iter().copied().max().unwrap_or(0);
    for level in 0..=max_level {
        for t in (0..n).filter(|&t| levels[t] == level) {
            output[t] = (input[t] + own[t]).min(MAX_PDC);
        }
        for &(s, d, _) in edges {
            if levels[s] == level && levels[d] > level {
                input[d] = input[d].max(output[s]);
            }
        }
    }
}

/// Fixed stereo delay used to line up paths for plugin delay compensation.
#[derive(Default)]
struct PdcDelay {
    buf: Vec<f32>,
    pos: usize,
    samples: usize,
}

impl PdcDelay {
    fn set(&mut self, samples: usize) {
        if samples != self.samples {
            // Only reallocates when a plugin's latency changes the plan.
            self.samples = samples;
            self.buf = vec![0.0; samples * 2];
            self.pos = 0;
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.samples == 0 {
            return;
        }
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let j = self.pos * 2;
            let (dl, dr) = (self.buf[j], self.buf[j + 1]);
            self.buf[j] = *a;
            self.buf[j + 1] = *b;
            *a = dl;
            *b = dr;
            self.pos = (self.pos + 1) % self.samples;
        }
    }
}

#[cfg(test)]
mod pdc_tests {
    use super::*;

    #[test]
    fn sends_go_up_and_loops_are_cut() {
        // Vocals (2) go to the master and send *up* to a reverb bus (7), as in FL.
        let edges = [(2, 0, 1.0), (2, 7, 0.2), (7, 0, 1.0)];
        let mut levels = Vec::new();
        route_levels(8, &edges, &mut levels);
        assert!(levels[2] < levels[7] && levels[7] < levels[0]);
        // A loop (3 -> 4 -> 3) terminates; one of its routes can't be satisfied.
        let looped = [(3, 4, 1.0), (4, 3, 1.0), (4, 0, 1.0)];
        route_levels(5, &looped, &mut levels);
        assert!(levels[3] < levels[4] || levels[4] < levels[3]);
        assert!(levels.iter().all(|&l| l <= 5));
    }

    #[test]
    fn plan_aligns_paths() {
        // 1 and 2 feed the master; 3 feeds 1. Track 2 has a 100-sample limiter,
        // track 3 a 30-sample plugin.
        let own = [0, 0, 100, 30];
        let edges = [(1, 0, 1.0), (2, 0, 1.0), (3, 1, 1.0)];
        let mut levels = Vec::new();
        route_levels(4, &edges, &mut levels);
        let (mut input, mut output) = (Vec::new(), Vec::new());
        pdc_plan(&own, &levels, &edges, &mut input, &mut output);
        assert_eq!(output[3], 30);
        assert_eq!(input[1], 30); // track 1's direct audio waits for track 3
        assert_eq!(output[1], 30);
        assert_eq!(input[0], 100); // the master waits for the slowest path (track 2)
        // Each source is delayed by input[dest] - output[source]: 1 by 70, 2 by 0.
        assert_eq!(input[0] - output[1], 70);
        assert_eq!(input[0] - output[2], 0);
    }

    #[test]
    fn delay_shifts_by_samples() {
        let mut d = PdcDelay::default();
        d.set(2);
        let (mut l, mut r) = ([1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]);
        d.process(&mut l, &mut r);
        assert_eq!(l, [0.0, 0.0, 1.0, 2.0]);
        assert_eq!(r, [0.0, 0.0, 5.0, 6.0]);
    }
}

/// Latency of a mixer track's plugin effects, in samples. Bypassed plugins
/// count too, so toggling one doesn't shift the timing.
fn track_latency(track: &crate::model::MixerTrack) -> usize {
    track
        .effects
        .iter()
        .filter_map(|e| match &e.effect {
            crate::model::Effect::Plugin(p) => p.instance.as_ref().map(|i| i.latency as usize),
            _ => None,
        })
        .sum()
}

/// Largest latency among plugin instruments, in samples.
fn instrument_latency(project: &Project) -> usize {
    project
        .channels
        .iter()
        .filter_map(|c| match &c.instrument {
            Instrument::Plugin(p) => p.instance.as_ref().map(|i| i.latency as usize),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .min(MAX_PDC)
}
