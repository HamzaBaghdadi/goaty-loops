//! DSP state for mixer insert effects.

use crate::dsp::{db_to_gain, gain_to_db, Biquad, BiquadKind, Svf};
use crate::model::{Effect, EqBand, EqBandKind, FilterKind};

pub enum FxState {
    Delay { l: Vec<f32>, r: Vec<f32>, pos: usize },
    Reverb(Box<Reverb>),
    Filter { l: Svf, r: Svf },
    Distortion,
    Compressor { env: f32 },
    Eq3 { bands: [[Biquad; 2]; 3] },
    Plugin(Option<std::sync::Arc<crate::plugin::PluginInstance>>),
    ParamEq(Box<ParamEqState>),
    Limiter { comp_env: f32, gain: f32 },
    Reverb2(Box<Reverb2State>),
    Balance,
    StereoEnhancer { l: Vec<f32>, r: Vec<f32>, pos: usize },
    SoftClipper,
    Soundgoodizer(Box<SoundgoodizerState>),
    MultiFilter { l: Svf, r: Svf },
    StereoShaper,
    AutoPan { phase: f32 },
    FastDist,
    Overdrive { tone: [Svf; 2] },
    Chorus { l: Vec<f32>, r: Vec<f32>, pos: usize, phase: f32 },
    TransientShaper { fast: f32, slow: f32 },
}

impl FxState {
    pub fn new(effect: &Effect, sr: f32) -> Self {
        match effect {
            Effect::Delay { .. } => {
                let n = (sr * 4.0) as usize;
                FxState::Delay { l: vec![0.0; n], r: vec![0.0; n], pos: 0 }
            }
            Effect::Reverb { .. } => FxState::Reverb(Box::new(Reverb::new(sr))),
            Effect::Filter { .. } => FxState::Filter { l: Svf::default(), r: Svf::default() },
            Effect::Distortion { .. } => FxState::Distortion,
            Effect::Compressor { .. } => FxState::Compressor { env: 0.0 },
            Effect::Eq3 { .. } => FxState::Eq3 { bands: Default::default() },
            Effect::Plugin(p) => FxState::Plugin(p.instance.clone()),
            Effect::ParamEq { .. } => FxState::ParamEq(Box::new(ParamEqState { filters: Default::default(), last: None })),
            Effect::Limiter { .. } => FxState::Limiter { comp_env: 0.0, gain: 1.0 },
            Effect::Reverb2 { .. } => FxState::Reverb2(Box::new(Reverb2State::new(sr))),
            Effect::Balance { .. } => FxState::Balance,
            Effect::StereoEnhancer { .. } => {
                let n = (sr * 0.05) as usize + 1;
                FxState::StereoEnhancer { l: vec![0.0; n], r: vec![0.0; n], pos: 0 }
            }
            Effect::SoftClipper { .. } => FxState::SoftClipper,
            Effect::Soundgoodizer { .. } => FxState::Soundgoodizer(Box::new(SoundgoodizerState { low: Default::default(), high: Default::default(), env: [[0.0; 2]; 3] })),
            Effect::MultiFilter { .. } => FxState::MultiFilter { l: Svf::default(), r: Svf::default() },
            Effect::StereoShaper { .. } => FxState::StereoShaper,
            Effect::AutoPan { .. } => FxState::AutoPan { phase: 0.0 },
            Effect::FastDist { .. } => FxState::FastDist,
            Effect::Overdrive { .. } => FxState::Overdrive { tone: Default::default() },
            Effect::Chorus { .. } => {
                let n = (sr * 0.06) as usize + 2;
                FxState::Chorus { l: vec![0.0; n], r: vec![0.0; n], pos: 0, phase: 0.0 }
            }
            Effect::TransientShaper { .. } => FxState::TransientShaper { fast: 0.0, slow: 0.0 },
        }
    }

    pub fn matches(&self, effect: &Effect) -> bool {
        if let (FxState::Plugin(a), Effect::Plugin(p)) = (self, effect) {
            return match (a, &p.instance) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
        }
        matches!(
            (self, effect),
            (FxState::Delay { .. }, Effect::Delay { .. })
                | (FxState::Reverb(_), Effect::Reverb { .. })
                | (FxState::Filter { .. }, Effect::Filter { .. })
                | (FxState::Distortion, Effect::Distortion { .. })
                | (FxState::Compressor { .. }, Effect::Compressor { .. })
                | (FxState::Eq3 { .. }, Effect::Eq3 { .. })
                | (FxState::Plugin(_), Effect::Plugin(_))
                | (FxState::ParamEq(_), Effect::ParamEq { .. })
                | (FxState::Limiter { .. }, Effect::Limiter { .. })
                | (FxState::Reverb2(_), Effect::Reverb2 { .. })
                | (FxState::Balance, Effect::Balance { .. })
                | (FxState::StereoEnhancer { .. }, Effect::StereoEnhancer { .. })
                | (FxState::SoftClipper, Effect::SoftClipper { .. })
                | (FxState::Soundgoodizer(_), Effect::Soundgoodizer { .. })
                | (FxState::MultiFilter { .. }, Effect::MultiFilter { .. })
                | (FxState::StereoShaper, Effect::StereoShaper { .. })
                | (FxState::AutoPan { .. }, Effect::AutoPan { .. })
                | (FxState::FastDist, Effect::FastDist { .. })
                | (FxState::Overdrive { .. }, Effect::Overdrive { .. })
                | (FxState::Chorus { .. }, Effect::Chorus { .. })
                | (FxState::TransientShaper { .. }, Effect::TransientShaper { .. })
        )
    }

    pub fn process(&mut self, effect: &Effect, l: &mut [f32], r: &mut [f32], sr: f32, time: &crate::plugin::TimeInfo) {
        self.process_sc(effect, l, r, None, sr, time);
    }

    /// Like `process`, with an optional sidechain signal (left, right) that
    /// effects with a sidechain input (the Limiter's compressor) listen to.
    pub fn process_sc(&mut self, effect: &Effect, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>, sr: f32, time: &crate::plugin::TimeInfo) {
        let tempo = time.tempo;
        match (self, effect) {
            (FxState::Delay { l: bl, r: br, pos }, Effect::Delay { beats, feedback, mix }) => {
                let len = bl.len();
                let d = ((*beats as f64 * 60.0 / tempo) * sr as f64).clamp(1.0, (len - 1) as f64) as usize;
                let fb = feedback.clamp(0.0, 0.95);
                for i in 0..l.len() {
                    let rd = (*pos + len - d) % len;
                    let (yl, yr) = (bl[rd], br[rd]);
                    bl[*pos] = l[i] + yr * fb;
                    br[*pos] = r[i] + yl * fb;
                    l[i] += yl * mix;
                    r[i] += yr * mix;
                    *pos = (*pos + 1) % len;
                }
            }
            (FxState::Reverb(rv), Effect::Reverb { room, damping, mix }) => rv.process(l, r, *room, *damping, *mix),
            (FxState::Filter { l: fl, r: fr }, Effect::Filter { kind, cutoff, resonance }) => {
                fl.set(*cutoff, *resonance, sr);
                fr.set(*cutoff, *resonance, sr);
                for i in 0..l.len() {
                    let (a, b) = (fl.process(l[i]), fr.process(r[i]));
                    (l[i], r[i]) = match kind {
                        FilterKind::Lowpass => (a.low, b.low),
                        FilterKind::Highpass => (a.high, b.high),
                        FilterKind::Bandpass => (a.band, b.band),
                    };
                }
            }
            (FxState::Distortion, Effect::Distortion { drive, mix }) => {
                let dr = drive.max(1.0);
                let norm = 1.0 / dr.tanh();
                for s in l.iter_mut().chain(r.iter_mut()) {
                    let wet = (*s * dr).tanh() * norm;
                    *s += (wet - *s) * mix;
                }
            }
            (FxState::Compressor { env }, Effect::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup_db }) => {
                let att = (-1.0 / (attack_ms.max(0.1) * 0.001 * sr)).exp();
                let rel = (-1.0 / (release_ms.max(1.0) * 0.001 * sr)).exp();
                let makeup = db_to_gain(*makeup_db);
                let slope = 1.0 - 1.0 / ratio.max(1.0);
                for i in 0..l.len() {
                    let x = l[i].abs().max(r[i].abs());
                    let c = if x > *env { att } else { rel };
                    *env = c * *env + (1.0 - c) * x;
                    let over = gain_to_db(*env) - threshold_db;
                    let g = if over > 0.0 { db_to_gain(-over * slope) } else { 1.0 } * makeup;
                    l[i] *= g;
                    r[i] *= g;
                }
            }
            (FxState::Eq3 { bands }, Effect::Eq3 { low_db, mid_db, high_db }) => {
                for ch in 0..2 {
                    bands[0][ch].set(BiquadKind::LowShelf, 200.0, 0.707, *low_db, sr);
                    bands[1][ch].set(BiquadKind::Peak, 1000.0, 0.7, *mid_db, sr);
                    bands[2][ch].set(BiquadKind::HighShelf, 5000.0, 0.707, *high_db, sr);
                }
                for i in 0..l.len() {
                    for b in bands.iter_mut() {
                        l[i] = b[0].process(l[i]);
                        r[i] = b[1].process(r[i]);
                    }
                }
            }
            (FxState::ParamEq(st), Effect::ParamEq { bands, main_db }) => st.process(bands, *main_db, l, r, sr),
            (FxState::Limiter { comp_env, gain }, Effect::Limiter { gain_db, ceiling_db, release_ms, comp_threshold_db, comp_ratio, sidechain }) => {
                // With a sidechain the compressor follows that signal (e.g. a kick ducking a bass).
                let detector = sc.filter(|_| *sidechain > 0);
                let pre = db_to_gain(*gain_db);
                let ceil = db_to_gain(*ceiling_db);
                let rel = 1.0 - (-1.0 / (release_ms.max(1.0) * 0.001 * sr)).exp();
                let comp = *comp_threshold_db < -0.1 && *comp_ratio > 1.0;
                let (ca, cr) = ((-1.0 / (0.005 * sr)).exp(), (-1.0 / (0.1 * sr)).exp());
                let slope = 1.0 - 1.0 / comp_ratio.max(1.0);
                for i in 0..l.len() {
                    let (mut a, mut b) = (l[i] * pre, r[i] * pre);
                    if comp {
                        let x = match detector {
                            Some((sl, sr_)) => sl.get(i).map_or(0.0, |v| v.abs()).max(sr_.get(i).map_or(0.0, |v| v.abs())),
                            None => a.abs().max(b.abs()),
                        };
                        let c = if x > *comp_env { ca } else { cr };
                        *comp_env = c * *comp_env + (1.0 - c) * x;
                        let over = gain_to_db(*comp_env) - comp_threshold_db;
                        if over > 0.0 {
                            let g = db_to_gain(-over * slope);
                            a *= g;
                            b *= g;
                        }
                    }
                    let peak = a.abs().max(b.abs());
                    let target = if peak * *gain > ceil { ceil / peak } else { 1.0 };
                    if target < *gain {
                        *gain = target;
                    } else {
                        *gain += (target - *gain) * rel;
                    }
                    l[i] = a * *gain;
                    r[i] = b * *gain;
                }
            }
            (FxState::Reverb2(st), e @ Effect::Reverb2 { .. }) => st.process(e, l, r, sr),
            (FxState::Balance, Effect::Balance { gain, pan }) => {
                let (pl, pr) = crate::dsp::pan_gains(*pan);
                for i in 0..l.len() {
                    l[i] *= gain * pl;
                    r[i] *= gain * pr;
                }
            }
            (FxState::StereoEnhancer { l: bl, r: br, pos }, Effect::StereoEnhancer { gain, pan, separation, phase_offset_ms, invert }) => {
                let (pl, pr) = crate::dsp::pan_gains(*pan);
                let len = bl.len();
                let d = ((phase_offset_ms.abs() * 0.001 * sr) as usize).min(len - 1);
                let side = (1.0 + separation).clamp(0.0, 2.0);
                for i in 0..l.len() {
                    let (m, s) = ((l[i] + r[i]) * 0.5, (l[i] - r[i]) * 0.5 * side);
                    let (mut a, mut b) = (m + s, m - s);
                    bl[*pos] = a;
                    br[*pos] = b;
                    let rd = (*pos + len - d) % len;
                    if *phase_offset_ms > 0.0 {
                        b = br[rd];
                    } else if *phase_offset_ms < 0.0 {
                        a = bl[rd];
                    }
                    *pos = (*pos + 1) % len;
                    if *invert == 1 {
                        a = -a;
                    } else if *invert == 2 {
                        b = -b;
                    }
                    l[i] = a * gain * pl;
                    r[i] = b * gain * pr;
                }
            }
            (FxState::TransientShaper { fast, slow }, Effect::TransientShaper { attack, release, gain_db }) => {
                // A fast and a slow envelope: the fast one leads at attacks and
                // trails in decaying tails; their dB difference drives the gain.
                let coef = |secs: f32| 1.0 - (-1.0 / (secs * sr)).exp();
                let (fast_up, fast_down, slow_up, slow_down) = (coef(0.0005), coef(0.03), coef(0.02), coef(0.3));
                let out = db_to_gain(*gain_db);
                for i in 0..l.len() {
                    let x = l[i].abs().max(r[i].abs());
                    *fast += (x - *fast) * if x > *fast { fast_up } else { fast_down };
                    *slow += (x - *slow) * if x > *slow { slow_up } else { slow_down };
                    let d = gain_to_db(fast.max(1e-6)) - gain_to_db(slow.max(1e-6));
                    let g_db = if d > 0.0 { attack * d } else { -release * d };
                    let g = db_to_gain(g_db.clamp(-18.0, 18.0)) * out;
                    l[i] *= g;
                    r[i] *= g;
                }
            }
            (FxState::SoftClipper, Effect::SoftClipper { threshold, post_gain }) => {
                let t = threshold.clamp(0.01, 1.0);
                for s in l.iter_mut().chain(r.iter_mut()) {
                    *s = soft_clip(*s, t) * post_gain;
                }
            }
            (FxState::Soundgoodizer(st), Effect::Soundgoodizer { mode, amount }) => st.process(*mode, *amount, l, r, sr),
            (FxState::MultiFilter { l: fl, r: fr }, Effect::MultiFilter { cutoff, resonance, low, band, high }) => {
                fl.set(*cutoff, *resonance, sr);
                fr.set(*cutoff, *resonance, sr);
                for i in 0..l.len() {
                    let (a, b) = (fl.process(l[i]), fr.process(r[i]));
                    l[i] = a.low * low + a.band * band + a.high * high;
                    r[i] = b.low * low + b.band * band + b.high * high;
                }
            }
            (FxState::StereoShaper, Effect::StereoShaper { l_from_l, l_from_r, r_from_l, r_from_r }) => {
                for i in 0..l.len() {
                    let (a, b) = (l[i], r[i]);
                    l[i] = a * l_from_l + b * l_from_r;
                    r[i] = a * r_from_l + b * r_from_r;
                }
            }
            (FxState::AutoPan { phase }, Effect::AutoPan { gain, pan, lfo_amount, lfo_hz }) => {
                let inc = lfo_hz.max(0.0) / sr;
                for i in 0..l.len() {
                    let p = (pan + (*phase * std::f32::consts::TAU).sin() * lfo_amount).clamp(-1.0, 1.0);
                    let (pl, pr) = crate::dsp::pan_gains(p);
                    l[i] *= gain * pl;
                    r[i] *= gain * pr;
                    *phase = (*phase + inc) % 1.0;
                }
            }
            (FxState::FastDist, Effect::FastDist { pre, threshold, kind, mix, post }) => {
                let t = threshold.clamp(0.01, 1.0);
                for s in l.iter_mut().chain(r.iter_mut()) {
                    let x = *s * pre;
                    let wet = if *kind == 0 {
                        x.clamp(-t, t) / t
                    } else {
                        // Type B folds back above the threshold.
                        let y = x / t;
                        if y.abs() <= 1.0 { y } else { y.signum() * (2.0 - y.abs()).clamp(-1.0, 1.0) }
                    };
                    *s = (*s + (wet - *s) * mix) * post;
                }
            }
            (FxState::Overdrive { tone }, Effect::Overdrive { drive, color, post }) => {
                let fc = 300.0 * 2f32.powf(color.clamp(0.0, 1.0) * 6.0);
                for t in tone.iter_mut() {
                    t.set(fc, 0.1, sr);
                }
                let d = drive.max(1.0);
                let norm = 1.0 / d.tanh();
                for i in 0..l.len() {
                    let a = tone[0].process((l[i] * d).tanh() * norm).low;
                    let b = tone[1].process((r[i] * d).tanh() * norm).low;
                    l[i] = a * post;
                    r[i] = b * post;
                }
            }
            (FxState::Chorus { l: bl, r: br, pos, phase }, Effect::Chorus { delay_ms, depth_ms, rate_hz, mix }) => {
                let len = bl.len();
                let inc = rate_hz.max(0.0) / sr;
                for i in 0..l.len() {
                    bl[*pos] = l[i];
                    br[*pos] = r[i];
                    let m = (*phase * std::f32::consts::TAU).sin();
                    let m2 = ((*phase + 0.25) * std::f32::consts::TAU).sin();
                    let tap = |buf: &[f32], ms: f32| {
                        let d = (ms * 0.001 * sr).clamp(1.0, (len - 2) as f32);
                        let rd = (*pos as f32 + len as f32 - d) % len as f32;
                        let k = rd as usize;
                        let f = rd - k as f32;
                        buf[k] + (buf[(k + 1) % len] - buf[k]) * f
                    };
                    let wl = tap(bl, delay_ms + depth_ms * m);
                    let wr = tap(br, delay_ms + depth_ms * m2);
                    l[i] += (wl - l[i]) * mix * 0.5 + wl * mix * 0.5;
                    r[i] += (wr - r[i]) * mix * 0.5 + wr * mix * 0.5;
                    *pos = (*pos + 1) % len;
                    *phase = (*phase + inc) % 1.0;
                }
            }
            (FxState::Plugin(Some(inst)), Effect::Plugin(_)) => {
                inst.process(l.len(), &[], l, r, time);
            }
            _ => {}
        }
    }
}

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }
    #[inline]
    fn process(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * fb;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        b - x
    }
}

/// A compact Freeverb.
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let comb_lens = [1116, 1188, 1277, 1356, 1422, 1491];
        let ap_lens = [556, 441, 341];
        let mk = |spread: usize| {
            (
                comb_lens.iter().map(|&n| Comb::new(((n + spread) as f32 * scale) as usize)).collect(),
                ap_lens.iter().map(|&n| Allpass::new(((n + spread) as f32 * scale) as usize)).collect(),
            )
        };
        let (cl, al) = mk(0);
        let (cr, ar) = mk(23);
        Self { combs: [cl, cr], allpasses: [al, ar] }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], room: f32, damping: f32, mix: f32) {
        let fb = room.clamp(0.0, 1.0) * 0.28 + 0.7;
        let damp = damping.clamp(0.0, 1.0) * 0.4;
        for i in 0..l.len() {
            let (wl, wr) = self.tick((l[i] + r[i]) * 0.015, fb, damp);
            l[i] += wl * mix * 3.0;
            r[i] += wr * mix * 3.0;
        }
    }

    /// One sample of the reverb tail for a mono input.
    #[inline]
    fn tick(&mut self, input: f32, fb: f32, damp: f32) -> (f32, f32) {
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            let mut acc = 0.0;
            for c in self.combs[ch].iter_mut() {
                acc += c.process(input, fb, damp);
            }
            for a in self.allpasses[ch].iter_mut() {
                acc = a.process(acc);
            }
            out[ch] = acc;
        }
        (out[0], out[1])
    }
}

// ---------------------------------------------------------------------------
// FL-compatible effects
// ---------------------------------------------------------------------------

pub struct ParamEqState {
    filters: [[Biquad; 2]; 7],
    last: Option<[EqBand; 7]>,
}

impl ParamEqState {
    fn process(&mut self, bands: &[EqBand; 7], main_db: f32, l: &mut [f32], r: &mut [f32], sr: f32) {
        if self.last.as_ref() != Some(bands) {
            for (b, f) in bands.iter().zip(self.filters.iter_mut()) {
                let kind = match b.kind {
                    EqBandKind::Off => continue,
                    EqBandKind::LowPass => BiquadKind::LowPass,
                    EqBandKind::HighPass => BiquadKind::HighPass,
                    EqBandKind::BandPass => BiquadKind::BandPass,
                    EqBandKind::Notch => BiquadKind::Notch,
                    EqBandKind::LowShelf => BiquadKind::LowShelf,
                    EqBandKind::HighShelf => BiquadKind::HighShelf,
                    EqBandKind::Peak => BiquadKind::Peak,
                };
                let q = match b.kind {
                    EqBandKind::LowShelf | EqBandKind::HighShelf => b.q.clamp(0.3, 1.5),
                    _ => b.q.clamp(0.1, 30.0),
                };
                for ch in f.iter_mut() {
                    ch.set(kind, b.freq, q, b.gain_db, sr);
                }
            }
            self.last = Some(*bands);
        }
        let main = db_to_gain(main_db);
        for i in 0..l.len() {
            let (mut a, mut c) = (l[i], r[i]);
            for (b, f) in bands.iter().zip(self.filters.iter_mut()) {
                if b.kind == EqBandKind::Off || (matches!(b.kind, EqBandKind::Peak | EqBandKind::LowShelf | EqBandKind::HighShelf) && b.gain_db.abs() < 0.01) {
                    continue;
                }
                a = f[0].process(a);
                c = f[1].process(c);
            }
            l[i] = a * main;
            r[i] = c * main;
        }
    }
}

pub struct Reverb2State {
    reverb: Reverb,
    pre: [Vec<f32>; 2],
    pos: usize,
    low_cut: [Biquad; 2],
    high_cut: [Biquad; 2],
    cuts: (f32, f32),
}

impl Reverb2State {
    fn new(sr: f32) -> Self {
        let n = (sr * 0.6) as usize;
        Self {
            reverb: Reverb::new(sr),
            pre: [vec![0.0; n], vec![0.0; n]],
            pos: 0,
            low_cut: Default::default(),
            high_cut: Default::default(),
            cuts: (-1.0, -1.0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process(&mut self, e: &Effect, l: &mut [f32], r: &mut [f32], sr: f32) {
        let Effect::Reverb2 { low_cut, high_cut, predelay_ms, size, decay_s, damping_hz, width, dry, early, wet } = *e else { return };
        if self.cuts != (low_cut, high_cut) {
            for ch in 0..2 {
                self.low_cut[ch].set(BiquadKind::HighPass, low_cut.max(10.0), 0.707, 0.0, sr);
                self.high_cut[ch].set(BiquadKind::LowPass, high_cut.min(sr * 0.45), 0.707, 0.0, sr);
            }
            self.cuts = (low_cut, high_cut);
        }
        // Feedback for the requested RT60 given the combs' average delay (~30 ms * size).
        let loop_secs = 0.03 * size.clamp(0.05, 1.0) + 0.005;
        let fb = 10f32.powf(-3.0 * loop_secs / decay_s.max(0.1)).clamp(0.0, 0.985);
        let damp = (1.0 - 2.0 * std::f32::consts::PI * damping_hz / sr).clamp(0.0, 0.95);
        let len = self.pre[0].len();
        let pre = ((predelay_ms * 0.001 * sr) as usize).min(len - 1);
        let taps = [(0.007, 0.8f32), (0.011, -0.6), (0.017, 0.5), (0.023, -0.4), (0.031, 0.3)];
        for i in 0..l.len() {
            self.pre[0][self.pos] = l[i];
            self.pre[1][self.pos] = r[i];
            let rd = (self.pos + len - pre) % len;
            let (pl, pr) = (self.pre[0][rd], self.pre[1][rd]);
            let (mut wl, mut wr) = self.reverb.tick((pl + pr) * 0.015, fb, damp);
            wl *= 3.0;
            wr *= 3.0;
            if early > 0.0 {
                for (k, &(t, g)) in taps.iter().enumerate() {
                    let d = (pre + (t * sr) as usize).min(len - 1);
                    let idx = (self.pos + len - d) % len;
                    let ch = k % 2;
                    let v = self.pre[ch][idx] * g * early * 0.35;
                    if ch == 0 { wl += v } else { wr += v }
                }
            }
            // Stereo width (0 = mono, 1 = normal, 2 = extra wide).
            let (m, s) = ((wl + wr) * 0.5, (wl - wr) * 0.5 * width);
            let (mut ol, mut or) = (m + s, m - s);
            ol = self.high_cut[0].process(self.low_cut[0].process(ol));
            or = self.high_cut[1].process(self.low_cut[1].process(or));
            l[i] = l[i] * dry + ol * wet;
            r[i] = r[i] * dry + or * wet;
            self.pos = (self.pos + 1) % len;
        }
    }
}

pub struct SoundgoodizerState {
    low: [Svf; 2],
    high: [Svf; 2],
    env: [[f32; 2]; 3],
}

impl SoundgoodizerState {
    fn process(&mut self, mode: u8, amount: f32, l: &mut [f32], r: &mut [f32], sr: f32) {
        let a = amount.clamp(0.0, 1.0);
        // Per-mode band emphasis (low, mid, high) and drive.
        let (w, drive) = match mode {
            1 => ([1.25, 0.95, 1.0], 1.2),
            2 => ([0.95, 1.0, 1.3], 1.2),
            3 => ([1.15, 1.05, 1.15], 1.8),
            _ => ([1.05, 1.0, 1.05], 1.1),
        };
        for ch in 0..2 {
            self.low[ch].set(200.0, 0.0, sr);
            self.high[ch].set(3000.0, 0.0, sr);
        }
        let att = (-1.0 / (0.005 * sr)).exp();
        let rel = (-1.0 / (0.12 * sr)).exp();
        let k = a * 6.0;
        let makeup = 1.0 + a * 2.0;
        for i in 0..l.len() {
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let lo = self.low[ch].process(*x).low;
                let hi = self.high[ch].process(*x).high;
                let mid = *x - lo - hi;
                let mut out = 0.0;
                for (b, v) in [lo, mid, hi].into_iter().enumerate() {
                    let e = &mut self.env[b][ch];
                    let lvl = v.abs();
                    let c = if lvl > *e { att } else { rel };
                    *e = c * *e + (1.0 - c) * lvl;
                    out += v / (1.0 + k * *e) * w[b];
                }
                let y = out * makeup;
                *x = *x * (1.0 - a) + ((y * drive).tanh() / drive.tanh()) * a;
            }
        }
    }
}

fn soft_clip(x: f32, t: f32) -> f32 {
    let ax = x.abs();
    if ax <= t || t >= 1.0 {
        x
    } else {
        let room = 1.0 - t;
        x.signum() * (t + room * ((ax - t) / room).tanh())
    }
}

#[cfg(test)]
mod transient_tests {
    use super::*;

    fn run(attack: f32, release: f32) -> Vec<f32> {
        let sr = 48_000.0;
        let fx = Effect::TransientShaper { attack, release, gain_db: 0.0 };
        let mut st = FxState::new(&fx, sr);
        // Silence, then a steady tone starting abruptly (a transient).
        let mut l: Vec<f32> = (0..9600).map(|i| if i < 1000 { 0.0 } else { 0.5 * ((i as f32) * 0.3).sin() }).collect();
        let mut r = l.clone();
        let time = crate::plugin::TimeInfo { sample_rate: sr as f64, tempo: 120.0, playing: true, ppq: 0.0, project_samples: 0, time_sig: (4, 4) };
        st.process(&fx, &mut l, &mut r, sr, &time);
        l
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn attack_boosts_onset_only() {
        let neutral = run(0.0, 0.0);
        let boosted = run(1.0, 0.0);
        // The first 10 ms after the onset get louder...
        assert!(peak(&boosted[1000..1480]) > peak(&neutral[1000..1480]) * 1.3);
        // ...while the settled tone near the end is close to unchanged.
        assert!((peak(&boosted[9000..]) / peak(&neutral[9000..]) - 1.0).abs() < 0.1);
        // Neutral settings don't change the sound.
        assert!((peak(&neutral[9000..]) - 0.5).abs() < 0.01);
    }
}
