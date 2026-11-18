//! Small DSP building blocks shared by instruments and effects.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Off,
}

/// Linear ADSR envelope.
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub stage: Stage,
    pub level: f32,
    attack_rate: f32,
    decay_rate: f32,
    sustain: f32,
    release_secs: f32,
    hold_samples: u32,
    hold_left: u32,
    release_rate: f32,
    sr: f32,
}

impl Adsr {
    pub fn new(env: &crate::model::Envelope, sr: f32) -> Self {
        let rate = |secs: f32| 1.0 / (secs.max(0.0005) * sr);
        Self {
            stage: Stage::Attack,
            level: 0.0,
            attack_rate: rate(env.attack),
            hold_samples: (env.hold.max(0.0) * sr) as u32,
            hold_left: 0,
            decay_rate: rate(env.decay),
            sustain: env.sustain.clamp(0.0, 1.0),
            release_secs: env.release.max(0.002),
            release_rate: 0.0,
            sr,
        }
    }

    /// An envelope that is fully open until released, then fades quickly.
    pub fn gate(sr: f32) -> Self {
        let mut a = Self::new(&crate::model::Envelope { attack: 0.0005, hold: 0.0, decay: 0.0, sustain: 1.0, release: 0.01 }, sr);
        a.stage = Stage::Sustain;
        a.level = 1.0;
        a
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Off {
            self.stage = Stage::Release;
            self.release_rate = self.level / (self.release_secs * self.sr);
        }
    }

    /// Quick fade used when a voice is stolen or playback stops.
    pub fn kill(&mut self) {
        self.stage = Stage::Release;
        self.release_rate = self.level / (0.005 * self.sr);
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += self.attack_rate;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.hold_left = self.hold_samples;
                    self.stage = if self.hold_samples > 0 { Stage::Hold } else { Stage::Decay };
                }
            }
            Stage::Hold => {
                self.hold_left = self.hold_left.saturating_sub(1);
                if self.hold_left == 0 {
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= self.decay_rate * (1.0 - self.sustain).max(0.001);
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                if self.level <= 0.0 {
                    self.stage = Stage::Off;
                }
            }
            Stage::Release => {
                self.level -= self.release_rate;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Off;
                }
            }
            Stage::Off => self.level = 0.0,
        }
        self.level
    }

    pub fn is_off(&self) -> bool {
        self.stage == Stage::Off
    }
}

/// Topology-preserving-transform state variable filter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    g: f32,
    k: f32,
}

pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl Svf {
    pub fn set(&mut self, cutoff: f32, resonance: f32, sr: f32) {
        let fc = cutoff.clamp(20.0, sr * 0.49);
        self.g = (PI * fc / sr).tan();
        self.k = 2.0 - 1.95 * resonance.clamp(0.0, 1.0);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> SvfOut {
        let a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        let a2 = self.g * a1;
        let a3 = self.g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { low: v2, band: v1, high: x - self.k * v1 - v2 }
    }
}

/// RBJ biquad, used for shelving/peaking EQ.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    LowShelf,
    Peak,
    HighShelf,
}

impl Biquad {
    pub fn set(&mut self, kind: BiquadKind, freq: f32, q: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq.clamp(10.0, sr * 0.49) / sr;
        let (cw, sw) = (w0.cos(), w0.sin());
        let alpha = sw / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::Peak => (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a),
            BiquadKind::LowPass => ((1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            BiquadKind::HighPass => ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            BiquadKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            BiquadKind::Notch => (1.0, -2.0 * cw, 1.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            BiquadKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cw + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                    a * ((a + 1.0) - (a - 1.0) * cw - s),
                    (a + 1.0) + (a - 1.0) * cw + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                    (a + 1.0) + (a - 1.0) * cw - s,
                )
            }
            BiquadKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cw + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                    a * ((a + 1.0) + (a - 1.0) * cw - s),
                    (a + 1.0) - (a - 1.0) * cw + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cw),
                    (a + 1.0) - (a - 1.0) * cw - s,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// xorshift32 noise source.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[inline]
pub fn key_to_freq(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// FL-style linear pan law: the centre is at full level on both sides.
#[inline]
/// FL Studio's volume law for mixer faders and sends, measured in FL:
/// `x` = raw value / 12800 (1.0 = 0 dB; the fader top is 1.25 = +5.6 dB);
/// gain = (e^(2.4x) - 1) / (e^2.4 - 1). E.g. x 0.748 = -6 dB, 0.524 = -12 dB.
pub fn fl_volume_to_gain(x: f32) -> f32 {
    const K: f32 = 2.4;
    ((K * x.max(0.0)).exp() - 1.0) / (K.exp() - 1.0)
}

/// Inverse of [`fl_volume_to_gain`].
pub fn gain_to_fl_volume(gain: f32) -> f32 {
    const K: f32 = 2.4;
    (gain.max(0.0) * (K.exp() - 1.0) + 1.0).ln() / K
}

/// Channel volume knob (0..=1) -> linear gain, measured in FL: the volume
/// law re-centred so the default 78% plays a sample at its own level
/// (FL's readout says -5.2 dB there; 52% plays -6.8 dB, 100% +5.2 dB).
pub fn channel_volume_to_gain(v: f32) -> f32 {
    fl_volume_to_gain(v) / fl_volume_to_gain(crate::model::DEFAULT_CHANNEL_VOLUME)
}

/// Fader position 0..=1 (top = 1, 0 dB at 0.8, as in FL) -> linear gain.
pub fn fader_to_gain(p: f32) -> f32 {
    fl_volume_to_gain(p.max(0.0) / 0.8)
}

#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

#[cfg(test)]
mod fl_volume_tests {
    use super::*;

    /// Raw values FL Studio saved for faders and sends typed in as dB.
    /// Channel volume levels measured from FL renders (sample in -> out).
    #[test]
    fn channel_volume_matches_fl_renders() {
        let db = |raw: f32| 20.0 * channel_volume_to_gain(raw / 12800.0).log10();
        assert!(db(10000.0).abs() < 0.05); // default 78%: unity
        assert!((db(6705.0) - -6.9).abs() < 0.15); // 52%
        assert!((db(2611.0) - -18.9).abs() < 0.15);
    }

    #[test]
    fn matches_values_measured_in_fl() {
        let db = |raw: i32| 20.0 * fl_volume_to_gain(raw as f32 / 12800.0).log10();
        for (raw, want) in [(12800, 0.0), (11153, -3.0), (9575, -6.0), (6705, -12.0), (2611, -24.0), (509, -40.0), (14500, 3.0), (16000, 5.6)] {
            assert!((db(raw) - want).abs() < 0.1, "raw {raw}: {} dB, FL says {want}", db(raw));
        }
        // The inverse gives FL's raw value back (within its rounding, ~0.01 dB).
        assert!(((gain_to_fl_volume(10f32.powf(-12.0 / 20.0)) * 12800.0).round() as i32 - 6705).abs() <= 4);
        assert_eq!(fl_volume_to_gain(0.0), 0.0);
    }
}
