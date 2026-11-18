//! Decoders for FL Studio's native effect parameters.
//!
//! The layouts were derived empirically: each blob is read as little-endian
//! i32 fields, and field meanings were inferred from value ranges and
//! defaults across thousands of instances in real projects. Unknown fields
//! are ignored, so decoded effects are close to, not identical with, FL's.

use crate::dsp::gain_to_db;
use crate::model::{eq_freq_from_norm, Effect, EqBand, EqBandKind};

/// The settings blob as little-endian i32 fields, read in place (decoding runs on
/// the audio thread while effects are automated, so it must not allocate).
#[derive(Clone, Copy)]
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn len(&self) -> usize {
        self.0.len() / 4
    }

    fn get(&self, i: usize) -> Option<i32> {
        self.0.get(i * 4..i * 4 + 4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
    }

    /// Field `i`; callers check `len` first.
    fn at(&self, i: usize) -> i32 {
        self.get(i).unwrap_or(0)
    }
}

fn fields(data: &[u8]) -> Fields<'_> {
    Fields(data)
}

/// Linear amplitude stored as value/1000 -> dB (0 maps to -60 dB).
fn milli_to_db(v: i32) -> f32 {
    if v <= 0 {
        -60.0
    } else {
        gain_to_db(v as f32 / 1000.0).max(-60.0)
    }
}

/// Bandwidth field (0..65536) -> Q, treating it as 0..4 octaves.
fn bandwidth_to_q(v: i32) -> f32 {
    let octaves = (v.clamp(0, 65536) as f32 / 65536.0 * 4.0).max(0.05);
    let x = std::f32::consts::LN_2 / 2.0 * octaves;
    (1.0 / (2.0 * x.sinh())).clamp(0.1, 30.0)
}

/// Converts a native FL effect into the closest Goaty Loops effect.
/// Returns `None` for effects Goaty Loops can't reproduce yet.
pub fn decode(internal: &str, data: &[u8]) -> Option<Effect> {
    let f = fields(data);
    let get = |i: usize| f.get(i);
    match internal {
        "Fruity Parametric EQ 2" => {
            if f.len() < 37 {
                return None;
            }
            let mut bands = EqBand::defaults();
            for (i, b) in bands.iter_mut().enumerate() {
                b.gain_db = f.at(1 + i) as f32 / 100.0;
                b.freq = eq_freq_from_norm(f.at(8 + i) as f32 / 65536.0);
                b.q = bandwidth_to_q(f.at(15 + i));
                b.kind = EqBandKind::ALL.get(f.at(22 + i).clamp(0, 7) as usize).copied().unwrap_or(EqBandKind::Peak);
            }
            Some(Effect::ParamEq { bands, main_db: f.at(36) as f32 / 100.0 })
        }
        "Fruity Limiter" => {
            if f.len() < 14 {
                return None;
            }
            let ratio_field = (f.at(13) as f32 / 10000.0).clamp(0.0, 0.95);
            Some(Effect::Limiter {
                gain_db: milli_to_db(f.at(1)),
                ceiling_db: milli_to_db(f.at(3)).min(6.0),
                release_ms: (f.at(6) as f32 / 10.0).clamp(1.0, 2000.0),
                comp_threshold_db: milli_to_db(f.at(9)),
                comp_ratio: 1.0 / (1.0 - ratio_field),
                // Field 41: the sidechain input number (0 = none), confirmed in FL.
                sidechain: f.get(41).unwrap_or(0).clamp(0, 255) as u8,
            })
        }
        "Fruity Compressor" => {
            if f.len() < 6 {
                return None;
            }
            Some(Effect::Compressor {
                threshold_db: f.at(1) as f32 / 10.0,
                ratio: (f.at(2) as f32 / 10.0).max(1.0),
                attack_ms: (f.at(4) as f32 / 10.0).max(0.1),
                release_ms: (f.at(5) as f32).max(1.0),
                makeup_db: f.at(3) as f32 / 10.0,
            })
        }
        "Fruity Delay 2" => {
            if f.len() < 8 {
                return None;
            }
            Some(Effect::Delay {
                beats: (f.at(4).max(1) as f32 / 96.0).min(8.0),
                feedback: (f.at(3) as f32 / 128.0).clamp(0.0, 0.95),
                mix: (f.at(1) as f32 / 128.0).clamp(0.0, 1.2),
            })
        }
        "Fruity Reeverb 2" => {
            if f.len() < 14 || !(10000..=10001).contains(&f.at(0)) {
                return None;
            }
            Some(Effect::Reverb2 {
                low_cut: f.at(1).max(10) as f32,
                high_cut: (f.at(2) as f32 * 100.0).clamp(500.0, 22100.0),
                predelay_ms: f.at(3).max(0) as f32,
                size: (f.at(4) as f32 / 100.0).clamp(0.01, 1.0),
                decay_s: (f.at(6) as f32 / 10.0).clamp(0.1, 20.0),
                damping_hz: (f.at(7) as f32 * 100.0).clamp(500.0, 22100.0),
                width: (1.0 + f.at(10) as f32 / 64.0).clamp(0.0, 2.0),
                dry: f.at(11).max(0) as f32 / 128.0,
                early: f.at(12).max(0) as f32 / 128.0,
                wet: f.at(13).max(0) as f32 / 128.0,
            })
        }
        "Fruity Balance" => Some(Effect::Balance { pan: (get(0)? as f32 / 128.0).clamp(-1.0, 1.0), gain: get(1)?.max(0) as f32 / 256.0 }),
        "Fruity Soft Clipper" => Some(Effect::SoftClipper { threshold: (get(0)? as f32 / 127.0).clamp(0.01, 1.0), post_gain: get(1)?.max(0) as f32 / 128.0 }),
        "Fruity Stereo Enhancer" => Some(Effect::StereoEnhancer {
            pan: (get(0)? as f32 / 128.0).clamp(-1.0, 1.0),
            gain: get(1)?.max(0) as f32 / 256.0,
            separation: (get(2)? as f32 / 96.0).clamp(-1.0, 1.0),
            phase_offset_ms: get(3)? as f32 / 512.0 * 20.0,
            invert: get(5).unwrap_or(0).clamp(0, 2) as u8,
        }),
        "Soundgoodizer" => Some(Effect::Soundgoodizer { mode: get(1)?.clamp(0, 3) as u8, amount: (get(2)? as f32 / 1000.0).clamp(0.0, 1.0) }),
        "Fruity Filter" => Some(Effect::MultiFilter {
            cutoff: eq_freq_from_norm(get(1)? as f32 / 1024.0),
            resonance: (get(2)? as f32 / 1024.0).clamp(0.0, 1.0),
            low: get(3)? as f32 / 1024.0,
            band: get(4)? as f32 / 1024.0,
            high: get(5)? as f32 / 1024.0,
        }),
        // Attack, release and gain are centred on 80; drive and the band split
        // aren't reproduced (reported as an approximation).
        "Transient Processor" => Some(Effect::TransientShaper {
            attack: ((get(1)? - 80) as f32 / 80.0).clamp(-1.0, 1.0),
            release: ((get(3)? - 80) as f32 / 80.0).clamp(-1.0, 1.0),
            gain_db: ((get(4)? - 80) as f32 * 0.15).clamp(-12.0, 12.0),
        }),
        // A one-knob lowpass: the multi-filter with only its lowpass output.
        "Fruity Fast LP" => Some(Effect::MultiFilter {
            cutoff: eq_freq_from_norm(get(1)? as f32 / 1024.0),
            resonance: (get(2)? as f32 / 1024.0).clamp(0.0, 1.0),
            low: 1.0,
            band: 0.0,
            high: 0.0,
        }),
        "Fruity Stereo Shaper" => Some(Effect::StereoShaper {
            l_from_r: get(1)? as f32 / 12800.0,
            l_from_l: get(2)? as f32 / 12800.0,
            r_from_r: get(3)? as f32 / 12800.0,
            r_from_l: get(4)? as f32 / 12800.0,
        }),
        "Fruity PanOMatic" => {
            let lfo_on = get(2)? != 0;
            Some(Effect::AutoPan {
                pan: ((get(0)? - 128) as f32 / 128.0).clamp(-1.0, 1.0),
                gain: get(1)?.max(0) as f32 / 256.0,
                lfo_amount: if lfo_on { (get(4)? as f32 / 128.0).clamp(0.0, 1.0) } else { 0.0 },
                lfo_hz: 0.05 * 400f32.powf(get(5)? as f32 / 65536.0),
            })
        }
        "Fruity Fast Dist" => Some(Effect::FastDist {
            pre: get(0)?.max(0) as f32 / 128.0,
            threshold: (get(1)? as f32 / 10.0).clamp(0.05, 1.0),
            kind: get(2)?.clamp(0, 1) as u8,
            mix: (get(3)? as f32 / 128.0).clamp(0.0, 1.0),
            post: get(4)?.max(0) as f32 / 128.0,
        }),
        "Fruity Blood Overdrive" => {
            let x100 = get(4)? != 0;
            Some(Effect::Overdrive {
                drive: 1.0 + get(3)?.max(0) as f32 / 10000.0 * if x100 { 90.0 } else { 9.0 },
                color: (get(2)? as f32 / 10000.0).clamp(0.0, 1.0),
                post: get(6)?.max(0) as f32 / 10000.0,
            })
        }
        "Fruity Chorus" => Some(Effect::Chorus {
            delay_ms: 1.0 + get(1)?.max(0) as f32 / 1024.0 * 20.0,
            depth_ms: get(2)?.max(0) as f32 / 4336.0 * 5.0,
            rate_hz: (get(4)?.max(0) as f32 / 1000.0).clamp(0.01, 10.0),
            mix: (get(11).unwrap_or(2500) as f32 / 5000.0).clamp(0.0, 1.0),
        }),
        "Fruity Delay 3" => Some(Effect::Delay {
            beats: if get(5)? > 0 { (get(5)? as f32 / 96.0).min(8.0) } else { 0.75 },
            feedback: (get(9)? as f32 / 6000.0 * 0.95).clamp(0.0, 0.95),
            mix: (get(1)? as f32 / 6000.0 * 0.5).clamp(0.0, 1.0),
        }),
        // Not decoded yet: a neutral limiter keeps the signal path and level sane.
        "Maximus" | "Fruity Multiband Compressor" => Some(Effect::Limiter { gain_db: 0.0, ceiling_db: 0.0, release_ms: 200.0, comp_threshold_db: 0.0, comp_ratio: 1.0, sidechain: 0 }),
        _ => None,
    }
}


/// Maps an FL plugin parameter index to its blob field and raw value range.
/// Plugins whose blob starts with a version field have parameters shifted by one.
pub fn param_field(internal: &str, param: u32) -> Option<(usize, i32, i32)> {
    let p = param as usize;
    let r = |field: usize, min: i32, max: i32| Some((field, min, max));
    match internal {
        "Fruity Parametric EQ 2" => match p {
            0..=6 => r(p + 1, -1800, 1800),
            7..=13 => r(p + 1, 0, 65536),
            14..=20 => r(p + 1, 0, 65536),
            35 => r(36, -1800, 1800),
            _ => None,
        },
        "Fruity Reeverb 2" => match p {
            0 => r(1, 20, 3000),
            1 => r(2, 5, 221),
            2 => r(3, 0, 300),
            3 => r(4, 1, 100),
            4 => r(5, 0, 100),
            5 => r(6, 1, 200),
            6 => r(7, 5, 221),
            9 => r(10, -64, 64),
            10..=12 => r(p + 1, 0, 160),
            _ => None,
        },
        "Fruity Limiter" => match p {
            0 => r(1, 0, 4000),
            2 => r(3, 0, 2000),
            5 => r(6, 0, 10000),
            8 => r(9, 0, 1000),
            _ => None,
        },
        "Fruity Compressor" => match p {
            0 => r(1, -600, 0),
            1 => r(2, 10, 300),
            2 => r(3, -40, 400),
            3 => r(4, 0, 10000),
            4 => r(5, 1, 5000),
            _ => None,
        },
        "Fruity Delay 2" => match p {
            0 => r(0, -128, 128),
            1..=2 => r(p, 0, 160),
            3 => r(3, 0, 128),
            4 => r(4, 1, 384),
            _ => None,
        },
        "Fruity Balance" => match p {
            0 => r(0, -128, 128),
            1 => r(1, 0, 320),
            _ => None,
        },
        "Fruity Soft Clipper" => match p {
            0 => r(0, 1, 127),
            1 => r(1, 0, 160),
            _ => None,
        },
        "Fruity Stereo Enhancer" => match p {
            0 => r(0, -128, 128),
            1 => r(1, 0, 320),
            2 => r(2, -96, 96),
            3 => r(3, -512, 512),
            _ => None,
        },
        "Soundgoodizer" => match p {
            1 => r(2, 0, 1000),
            _ => None,
        },
        // Cutoff, resonance (and Fruity Filter's low/band/high mix) follow the version field.
        "Fruity Fast LP" => match p {
            0..=1 => r(p + 1, 0, 1024),
            _ => None,
        },
        "Fruity Filter" => match p {
            0..=4 => r(p + 1, 0, 1024),
            _ => None,
        },
        _ => None,
    }
}

/// Decodes an FL effect after applying automated parameter values (0..=1).
pub fn decode_with(internal: &str, data: &[u8], overrides: impl Iterator<Item = (u32, f32)>) -> Option<Effect> {
    // Runs on the audio thread every block while automation is active: reuse a
    // per-thread buffer instead of allocating a copy of the settings each time.
    thread_local! {
        static SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    SCRATCH.with(|s| {
        let mut d = s.borrow_mut();
        d.clear();
        d.extend_from_slice(data);
        for (param, value) in overrides {
            if let Some((field, min, max)) = param_field(internal, param) {
                let raw = (min as f32 + (max - min) as f32 * value.clamp(0.0, 1.0)).round() as i32;
                if let Some(bytes) = d.get_mut(field * 4..field * 4 + 4) {
                    bytes.copy_from_slice(&raw.to_le_bytes());
                }
            }
        }
        decode(internal, &d)
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn blob(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn fast_lp_and_its_automation() {
        // Fully open by default; automating the cutoff to 0 closes it.
        let data = blob(&[1, 1024, 0]);
        let Some(Effect::MultiFilter { cutoff: open, low, high, .. }) = decode("Fruity Fast LP", &data) else { panic!() };
        assert_eq!((low, high), (1.0, 0.0));
        let Some(Effect::MultiFilter { cutoff: closed, .. }) = decode_with("Fruity Fast LP", &data, [(0, 0.0)].into_iter()) else { panic!() };
        assert!(open > 15_000.0 && closed < open / 10.0, "open {open} closed {closed}");
    }

    #[test]
    fn eq2_defaults() {
        let mut f = vec![6i32];
        f.extend([0; 7]);
        f.extend([10923, 18204, 25486, 32768, 40050, 47332, 54613]);
        f.extend([40124, 25412, 25412, 25412, 25412, 25412, 40124]);
        f.extend([5, 6, 6, 6, 6, 6, 7]);
        f.extend([0; 7]);
        f.push(-300);
        let Some(Effect::ParamEq { bands, main_db }) = decode("Fruity Parametric EQ 2", &blob(&f)) else { panic!() };
        assert_eq!(main_db, -3.0);
        assert_eq!(bands[0].kind, EqBandKind::LowShelf);
        assert_eq!(bands[6].kind, EqBandKind::HighShelf);
        assert!((bands[3].freq - 632.5).abs() < 2.0, "{}", bands[3].freq);
        assert!(bands[1].q > 0.5 && bands[1].q < 1.5);
    }

    #[test]
    fn limiter_unity_defaults() {
        let mut f = vec![7, 1000, 1000, 1000, 1115, 3, 5000, 3, 2882, 1000, 0, 0, 0, 6666];
        f.extend([0; 28]);
        let Some(Effect::Limiter { gain_db, ceiling_db, release_ms, comp_threshold_db, comp_ratio, .. }) = decode("Fruity Limiter", &blob(&f)) else { panic!() };
        assert!(gain_db.abs() < 1e-3 && ceiling_db.abs() < 1e-3);
        assert_eq!(release_ms, 500.0);
        assert!(comp_threshold_db.abs() < 1e-3);
        assert!((comp_ratio - 3.0).abs() < 0.01);
    }

    #[test]
    fn simple_effects() {
        assert_eq!(decode("Fruity Balance", &blob(&[0, 256])), Some(Effect::Balance { gain: 1.0, pan: 0.0 }));
        assert_eq!(decode("Soundgoodizer", &blob(&[3, 2, 500])), Some(Effect::Soundgoodizer { mode: 2, amount: 0.5 }));
        assert_eq!(decode("Unknown FX", &blob(&[1, 2, 3])), None);
    }
}

/// Effects that are imported as a rough stand-in rather than decoded.
pub fn is_approximation(internal: &str) -> bool {
    matches!(internal, "Maximus" | "Fruity Multiband Compressor" | "Transient Processor")
}
