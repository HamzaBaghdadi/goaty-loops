//! Voices for the built-in instruments.

use crate::dsp::{key_to_freq, poly_blep, Adsr, Rng, Svf};
use crate::model::{DrumKind, Instrument, Wave};
use crate::sample::SampleData;
use std::f32::consts::TAU;
use std::sync::Arc;

/// Channel index used for browser previews; routed straight to the master.
pub const PREVIEW_CHANNEL: usize = usize::MAX;

pub struct Voice {
    pub channel: usize,
    pub key: u8,
    pub age: u64,
    /// Held by a live NoteOn command until a NoteOff arrives.
    pub held: bool,
    velocity: f32,
    pan_l: f32,
    pan_r: f32,
    /// Samples until note-off; negative = no scheduled note-off.
    gate: i64,
    /// Samples until a cut-group kill takes effect.
    cut_in: Option<u32>,
    /// Samples to wait before the voice starts (sample-accurate scheduling).
    delay: u32,
    env: Adsr,
    sr: f32,
    kind: VoiceKind,
    done: bool,
    fade: Option<ClipFade>,
}

enum VoiceKind {
    Synth {
        phase: [f32; 3],
        inc: [f32; 3],
        waves: [Wave; 3],
        levels: [f32; 3],
        cutoff: f32,
        resonance: f32,
        filter: Svf,
        rng: Rng,
        counter: u32,
    },
    Sample {
        data: Arc<SampleData>,
        /// Read position; with `grains`, the stretch playhead.
        pos: f64,
        /// Source frames per output sample (pitch, and speed unless stretching keeps pitch).
        step: f64,
        reverse: bool,
        use_env: bool,
        /// Pitch-preserving time stretch.
        grains: Option<Grains>,
    },
    Drum {
        kind: DrumKind,
        t: f32,
        phase: f32,
        tune: f32,
        decay: f32,
        rng: Rng,
        filter: Svf,
    },
}

pub struct NoteParams {
    pub key: u8,
    pub velocity: f32,
    pub pan: f32,
    /// Note length in samples, or `None` for a held note.
    pub gate: Option<u64>,
    pub delay: u32,
    /// Start offset into the sound, in seconds (used for audio clips).
    pub offset_secs: f64,
    /// Song tempo when the note starts (for time-stretched samples).
    pub tempo: f64,
    /// Fade of the audio clip this note plays, if any.
    pub fade: Option<ClipFade>,
}

/// Fade-in/out of an audio clip, in samples.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipFade {
    /// Samples of the clip already behind the voice start.
    pub elapsed: u64,
    pub length: u64,
    pub fade_in: u64,
    pub fade_out: u64,
}

impl ClipFade {
    fn gain(&self) -> f32 {
        let mut g = 1.0;
        if self.fade_in > 0 && self.elapsed < self.fade_in {
            g *= self.elapsed as f32 / self.fade_in as f32;
        }
        let left = self.length.saturating_sub(self.elapsed);
        if self.fade_out > 0 && left < self.fade_out {
            g *= left as f32 / self.fade_out as f32;
        }
        g
    }
}

impl Voice {
    pub fn new(channel: usize, inst: &Instrument, p: &NoteParams, sr: f32, seed: u32) -> Option<Self> {
        let (kind, env) = match inst {
            Instrument::Silent { .. } | Instrument::Plugin(_) | Instrument::Automation(_) => return None,
            Instrument::Synth(s) => {
                let mut inc = [0.0; 3];
                for (i, o) in s.osc.iter().enumerate() {
                    let f = key_to_freq(p.key as f32 + o.coarse as f32 + o.fine / 100.0);
                    inc[i] = f / sr;
                }
                (
                    VoiceKind::Synth {
                        phase: [0.0, 0.33, 0.66],
                        inc,
                        waves: [s.osc[0].wave, s.osc[1].wave, s.osc[2].wave],
                        levels: [s.osc[0].level, s.osc[1].level, s.osc[2].level],
                        cutoff: s.cutoff,
                        resonance: s.resonance,
                        filter: Svf::default(),
                        rng: Rng::new(seed),
                        counter: 0,
                    },
                    Adsr::new(&s.env, sr),
                )
            }
            Instrument::Sampler(s) => {
                let data = s.data.clone()?;
                let ratio = 2f64.powf((p.key as f64 - s.root_key as f64) / 12.0);
                let frames = data.frames() as f64;
                // Time stretch: play the whole sample over `stretch_beats`. Resample
                // mode speeds the sample up (pitch follows); other modes keep the pitch
                // and move a grain playhead at that speed instead.
                let speed = if s.stretch_beats > 0.0 && p.tempo > 0.0 {
                    let target_secs = s.stretch_beats as f64 * 60.0 / p.tempo;
                    (frames / data.sample_rate.max(1) as f64 / target_secs).clamp(0.05, 20.0)
                } else {
                    1.0
                };
                let ratio = ratio * 2f64.powf(s.stretch_pitch as f64 / 1200.0);
                let base = data.sample_rate as f64 / sr as f64;
                let keep_pitch = s.keep_pitch && speed != 1.0;
                let step = if keep_pitch { ratio * base } else { ratio * speed * base };
                let grains = keep_pitch.then(|| Grains::new(speed * base, sr));
                let skip = p.offset_secs * speed * data.sample_rate as f64;
                let pos = if s.reverse { frames - 1.0 - skip } else { skip };
                let env = if s.use_envelope { Adsr::new(&s.env, sr) } else { Adsr::gate(sr) };
                (VoiceKind::Sample { data, pos, step, reverse: s.reverse, use_env: s.use_envelope, grains }, env)
            }
            Instrument::Drum(d) => (
                VoiceKind::Drum {
                    kind: d.kind,
                    t: 0.0,
                    phase: 0.0,
                    tune: 2f32.powf((d.tune + p.key as f32 - 60.0) / 12.0),
                    decay: d.decay.max(0.05),
                    rng: Rng::new(seed),
                    filter: Svf::default(),
                },
                Adsr::gate(sr),
            ),
        };
        let (pan_l, pan_r) = crate::dsp::pan_gains(p.pan);
        Some(Self {
            channel,
            key: p.key,
            age: 0,
            held: p.gate.is_none(),
            // Sample preamp (FL) scales the sound like a louder recording.
            velocity: p.velocity * match inst {
                Instrument::Sampler(s) => s.preamp.max(0.0),
                _ => 1.0,
            },
            pan_l,
            pan_r,
            gate: p.gate.map(|g| g.max(1) as i64).unwrap_or(-1),
            delay: p.delay,
            cut_in: None,
            env,
            sr,
            kind,
            done: false,
            fade: p.fade,
        })
    }

    pub fn note_off(&mut self) {
        self.held = false;
        self.gate = -1;
        match self.kind {
            // Drums and envelope-less samples are one-shots.
            VoiceKind::Drum { .. } => {}
            VoiceKind::Sample { use_env: false, .. } => {}
            _ => self.env.release(),
        }
    }

    pub fn kill(&mut self) {
        self.held = false;
        self.gate = -1;
        self.env.kill();
    }

    /// Kills the voice `samples` from the start of the current block.
    pub fn cut_at(&mut self, samples: u32) {
        self.cut_in = Some(self.cut_in.map_or(samples, |c| c.min(samples)));
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Mixes this voice into `l`/`r` with the given channel gains.
    /// Returns the peak level produced.
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32], gain_l: f32, gain_r: f32, pitch: f32) -> f32 {
        let gl = gain_l * self.pan_l * self.velocity;
        let gr = gain_r * self.pan_r * self.velocity;
        let mut peak = 0f32;
        for i in 0..l.len() {
            if let Some(c) = self.cut_in {
                if c == 0 {
                    self.cut_in = None;
                    self.kill();
                } else {
                    self.cut_in = Some(c - 1);
                }
            }
            if self.delay > 0 {
                self.delay -= 1;
                continue;
            }
            if self.gate > 0 {
                self.gate -= 1;
                if self.gate == 0 {
                    self.note_off();
                }
            }
            let env = self.env.next();
            if self.env.is_off() {
                self.done = true;
                break;
            }
            let (sl, sr) = match self.next_sample(pitch) {
                Some(s) => s,
                None => {
                    self.done = true;
                    break;
                }
            };
            let fade = match &mut self.fade {
                Some(f) => {
                    let g = f.gain();
                    f.elapsed += 1;
                    g
                }
                None => 1.0,
            };
            let (ol, or) = (sl * env * fade * gl, sr * env * fade * gr);
            l[i] += ol;
            r[i] += or;
            peak = peak.max(ol.abs()).max(or.abs());
        }
        peak
    }

    #[inline]
    fn next_sample(&mut self, pitch: f32) -> Option<(f32, f32)> {
        let sr = self.sr;
        match &mut self.kind {
            VoiceKind::Synth { phase, inc, waves, levels, cutoff, resonance, filter, rng, counter } => {
                let mut s = 0.0;
                for i in 0..3 {
                    if levels[i] <= 0.0 {
                        continue;
                    }
                    let t = phase[i];
                    let dt = inc[i] * pitch;
                    let v = match waves[i] {
                        Wave::Sine => (t * TAU).sin(),
                        Wave::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                        Wave::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
                        Wave::Square => {
                            let sq = if t < 0.5 { 1.0 } else { -1.0 };
                            sq + poly_blep(t, dt) - poly_blep((t + 0.5) % 1.0, dt)
                        }
                        Wave::Noise => rng.next_f32(),
                    };
                    s += v * levels[i];
                    phase[i] = (t + dt) % 1.0;
                }
                if *counter % 32 == 0 {
                    filter.set(*cutoff, *resonance, sr);
                }
                *counter = counter.wrapping_add(1);
                let y = filter.process(s * 0.5).low;
                Some((y, y))
            }
            VoiceKind::Sample { data, pos, step, reverse, grains: Some(g), .. } => {
                let dir = if *reverse { -1.0 } else { 1.0 };
                if *pos < 0.0 || *pos >= data.frames().saturating_sub(1) as f64 {
                    return None;
                }
                let out = g.next(data, *pos, dir, *step * pitch as f64);
                *pos += dir * g.time_step;
                Some(out)
            }
            VoiceKind::Sample { data, pos, step, reverse, .. } => {
                let frames = data.frames();
                if *pos < 0.0 || *pos >= (frames.saturating_sub(1)) as f64 {
                    return None;
                }
                let i = *pos as usize;
                let frac = (*pos - i as f64) as f32;
                let s = &data.samples;
                let l = s[i * 2] + (s[i * 2 + 2] - s[i * 2]) * frac;
                let r = s[i * 2 + 1] + (s[i * 2 + 3] - s[i * 2 + 1]) * frac;
                if *reverse {
                    *pos -= *step * pitch as f64;
                } else {
                    *pos += *step * pitch as f64;
                }
                Some((l, r))
            }
            VoiceKind::Drum { kind, t, phase, tune, decay, rng, filter } => {
                let tt = *t;
                let d = *decay;
                let tune_val = *tune * pitch;
                let tune = &tune_val;
                let (s, end) = match kind {
                    DrumKind::Kick => {
                        let f = (48.0 + 140.0 * (-tt * 32.0).exp()) * *tune;
                        *phase = (*phase + f / sr) % 1.0;
                        let body = (*phase * TAU).sin() * (-tt * 5.5 / d).exp();
                        let click = rng.next_f32() * (-tt * 400.0).exp() * 0.25;
                        ((body + click) * 0.95, 1.4 * d)
                    }
                    DrumKind::Snare => {
                        if tt == 0.0 {
                            filter.set(1800.0 * *tune, 0.1, sr);
                        }
                        *phase = (*phase + 190.0 * *tune / sr) % 1.0;
                        let tone = (*phase * TAU).sin() * (-tt * 22.0 / d).exp() * 0.55;
                        let noise = filter.process(rng.next_f32()).high * (-tt * 13.0 / d).exp() * 0.75;
                        (tone + noise, 0.7 * d)
                    }
                    DrumKind::Clap => {
                        if tt == 0.0 {
                            filter.set(1300.0 * *tune, 0.55, sr);
                        }
                        let mut e = 0.0;
                        for o in [0.0f32, 0.010, 0.021] {
                            if tt >= o {
                                e += (-(tt - o) * 190.0).exp();
                            }
                        }
                        e += (-(tt - 0.03).max(0.0) * 11.0 / d).exp() * if tt > 0.03 { 0.6 } else { 0.0 };
                        (filter.process(rng.next_f32()).band * e * 2.2, 0.7 * d)
                    }
                    DrumKind::ClosedHat | DrumKind::OpenHat => {
                        if tt == 0.0 {
                            filter.set(7500.0 * *tune, 0.2, sr);
                        }
                        let rate = if *kind == DrumKind::ClosedHat { 45.0 } else { 6.5 };
                        let s = filter.process(rng.next_f32()).high * (-tt * rate / d).exp() * 0.5;
                        (s, if *kind == DrumKind::ClosedHat { 0.3 } else { 1.3 } * d)
                    }
                };
                *t += 1.0 / sr;
                if tt > end {
                    return None;
                }
                Some((s, s))
            }
        }
    }
}

#[cfg(test)]
mod fade_tests {
    use super::ClipFade;

    #[test]
    fn fades_ramp_in_and_out() {
        let f = |elapsed| ClipFade { elapsed, length: 1000, fade_in: 100, fade_out: 200 }.gain();
        assert_eq!(f(0), 0.0);
        assert_eq!(f(50), 0.5);
        assert_eq!(f(500), 1.0);
        assert_eq!(f(900), 0.5);
        assert_eq!(f(1000), 0.0);
    }
}

/// Pitch-preserving time stretch by overlap-add: two Hann-windowed grains,
/// half a grain apart, read the sample at the note's pitch while their start
/// points follow a playhead that moves at the stretch speed.
#[derive(Clone, Debug)]
pub(crate) struct Grains {
    /// Grain length in output samples.
    len: f64,
    /// Source frames the playhead moves per output sample.
    time_step: f64,
    /// Position inside each grain, in output samples.
    k: [f64; 2],
    /// Source position each grain started at.
    start: [f64; 2],
    started: bool,
}

impl Grains {
    fn new(time_step: f64, sr: f32) -> Self {
        let len = (sr as f64 * 0.06).max(64.0);
        Self { len, time_step, k: [0.0, len / 2.0], start: [0.0; 2], started: false }
    }

    /// Next output frame; `pos` is the playhead, `step` the pitch step.
    fn next(&mut self, data: &SampleData, pos: f64, dir: f64, step: f64) -> (f32, f32) {
        if !self.started {
            self.start = [pos, pos];
            self.started = true;
        }
        let (mut l, mut r) = (0.0, 0.0);
        for g in 0..2 {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * self.k[g] / self.len).cos();
            let (a, b) = read_frame(data, self.start[g] + dir * self.k[g] * step);
            l += a * w as f32;
            r += b * w as f32;
            self.k[g] += 1.0;
            if self.k[g] >= self.len {
                self.k[g] -= self.len;
                self.start[g] = pos;
            }
        }
        (l, r)
    }
}

/// Linearly interpolated stereo frame at a fractional position (silence outside).
fn read_frame(data: &SampleData, p: f64) -> (f32, f32) {
    let frames = data.frames();
    if p < 0.0 || p >= frames.saturating_sub(1) as f64 {
        return (0.0, 0.0);
    }
    let i = p as usize;
    let frac = (p - i as f64) as f32;
    let s = &data.samples;
    (s[i * 2] + (s[i * 2 + 2] - s[i * 2]) * frac, s[i * 2 + 1] + (s[i * 2 + 3] - s[i * 2 + 1]) * frac)
}

#[cfg(test)]
mod stretch_tests {
    use super::*;
    use crate::model::{Instrument, SamplerParams};
    use crate::sample::SampleData;

    /// Renders a 1 s, 440 Hz sine stretched over 4 beats at 120 BPM (2 s);
    /// returns (seconds of sound, frequency from zero crossings).
    fn stretch(keep_pitch: bool) -> (f32, f32) {
        let sr = 48_000.0;
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 / sr * 440.0 * std::f32::consts::TAU).sin() * 0.5).collect();
        let data = Arc::new(SampleData::from_interleaved(tone, 1, 48_000));
        let s = SamplerParams { data: Some(data), stretch_beats: 4.0, keep_pitch, ..Default::default() };
        let p = NoteParams { key: 60, velocity: 1.0, pan: 0.0, gate: None, delay: 0, offset_secs: 0.0, tempo: 120.0, fade: None };
        let mut v = Voice::new(0, &Instrument::Sampler(s), &p, sr, 1).unwrap();
        let (mut l, mut r) = (vec![0.0; 150_000], vec![0.0; 150_000]);
        v.render(&mut l, &mut r, 1.0, 1.0, 1.0);
        let len = l.iter().rposition(|x| x.abs() > 1e-4).unwrap_or(0) as f32 / sr;
        // Zero crossings in a steady middle section.
        let mid = &l[20_000..40_000];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        (len, crossings as f32 / (mid.len() as f32 / sr))
    }

    #[test]
    fn stretch_modes() {
        let (len, freq) = stretch(true);
        assert!((len - 2.0).abs() < 0.1, "keep-pitch length {len}");
        assert!((freq - 440.0).abs() < 15.0, "keep-pitch frequency {freq}");
        let (len, freq) = stretch(false);
        assert!((len - 2.0).abs() < 0.1, "resample length {len}");
        assert!((freq - 220.0).abs() < 10.0, "resample frequency {freq}");
    }
}
