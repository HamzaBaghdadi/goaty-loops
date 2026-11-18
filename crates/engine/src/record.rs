//! Audio input recording (microphone / line in) via cpal.

use crate::sample::SampleData;
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Records the default input device until dropped or [`finish`](Self::finish)ed.
pub struct AudioRecorder {
    _stream: cpal::Stream,
    rx: Receiver<Vec<f32>>,
    data: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
    pub device_name: String,
    /// Input peak since the last [`take_peak`](Self::take_peak).
    peak: Arc<AtomicU32>,
    /// Engine whose output monitors this input (cleared when recording stops).
    monitor: Option<Arc<crate::audio::Shared>>,
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        if let Some(s) = &self.monitor {
            s.monitor.store(None);
        }
    }
}

impl AudioRecorder {
    /// Starts recording from the named input device, or the system default.
    /// With `monitor` (engine state and its output rate) the input is also
    /// heard through the output, when both run at the same sample rate.
    pub fn start(name: Option<&str>, monitor: Option<(Arc<crate::audio::Shared>, u32)>) -> Result<Self> {
        let host = cpal::default_host();
        let device = match name {
            Some(n) => host.input_devices()?.find(|d| d.name().ok().as_deref() == Some(n)).ok_or_else(|| anyhow!("input device \"{n}\" not found"))?,
            None => host.default_input_device().ok_or_else(|| anyhow!("no audio input device"))?,
        };
        let device_name = device.name().unwrap_or_else(|_| "unknown".into());
        let supported = device.default_input_config()?;
        let config: cpal::StreamConfig = supported.config();
        let (tx, rx) = crossbeam_channel::unbounded();
        let peak = Arc::new(AtomicU32::new(0));
        let monitor = monitor.filter(|(_, rate)| *rate == config.sample_rate.0).map(|(s, _)| s);
        let ring = monitor.as_ref().map(|_| Arc::new(MonitorRing::new(8192, config.sample_rate.0)));
        let channels = config.channels.max(1) as usize;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, tx, peak.clone(), ring.clone(), channels)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, tx, peak.clone(), ring.clone(), channels)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, tx, peak.clone(), ring.clone(), channels)?,
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, tx, peak.clone(), ring.clone(), channels)?,
            f => return Err(anyhow!("unsupported input sample format {f:?}")),
        };
        stream.play()?;
        if let Some(s) = &monitor {
            s.monitor.store(ring);
        }
        Ok(Self {
            _stream: stream,
            rx,
            data: Vec::new(),
            channels: config.channels.max(1) as usize,
            sample_rate: config.sample_rate.0,
            device_name,
            peak,
            monitor,
        })
    }

    /// Moves captured audio into the take; call regularly (e.g. every UI frame).
    pub fn poll(&mut self) {
        for chunk in self.rx.try_iter() {
            self.data.extend_from_slice(&chunk);
        }
    }

    pub fn seconds(&self) -> f64 {
        self.data.len() as f64 / (self.channels * self.sample_rate.max(1) as usize) as f64
    }

    /// Whether the input is heard through the output.
    pub fn monitoring(&self) -> bool {
        self.monitor.is_some()
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    /// Stops recording and returns the take as stereo sample data, with the
    /// first `latency_ms` dropped to line it up with what was playing.
    pub fn finish(mut self, latency_ms: f32) -> SampleData {
        self.poll();
        let skip = (latency_ms.max(0.0) as f64 / 1000.0 * self.sample_rate as f64) as usize * self.channels;
        let data = self.data.split_off(skip.min(self.data.len()));
        SampleData::from_interleaved(data, self.channels, self.sample_rate)
    }
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, tx: Sender<Vec<f32>>, peak: Arc<AtomicU32>, ring: Option<Arc<MonitorRing>>, channels: usize) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let chunk: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
            let p = chunk.iter().fold(0f32, |m, s| m.max(s.abs()));
            if p > f32::from_bits(peak.load(Ordering::Relaxed)) {
                peak.store(p.to_bits(), Ordering::Relaxed);
            }
            if let Some(r) = &ring {
                r.push(&chunk, channels);
            }
            let _ = tx.send(chunk);
        },
        |e| eprintln!("audio input error: {e}"),
        None,
    )?;
    Ok(stream)
}

/// Writes stereo sample data as a 32-bit float WAV file.
pub fn write_wav(path: &Path, data: &SampleData) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec { channels: 2, sample_rate: data.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in &data.samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip() {
        let dir = std::env::temp_dir().join("goaty_rec_test");
        let path = dir.join("take.wav");
        let data = SampleData::from_interleaved(vec![0.0, 0.5, -0.25, 1.0], 1, 44100);
        write_wav(&path, &data).unwrap();
        let back = crate::sample::load_audio(&path).unwrap();
        assert_eq!(back.sample_rate, 44100);
        assert_eq!(back.samples, data.samples);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Names of the available audio input devices.
pub fn input_devices() -> Vec<String> {
    cpal::default_host().input_devices().map(|ds| ds.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// Lock-free single-producer/single-consumer stereo ring that carries the
/// audio input to the output for monitoring.
pub struct MonitorRing {
    buf: Box<[std::sync::atomic::AtomicU32]>,
    write: std::sync::atomic::AtomicUsize,
    read: std::sync::atomic::AtomicUsize,
    pub sample_rate: u32,
}

impl MonitorRing {
    /// `frames` should cover a few buffers of latency (e.g. 8192).
    pub fn new(frames: usize, sample_rate: u32) -> Self {
        Self {
            buf: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(),
            write: Default::default(),
            read: Default::default(),
            sample_rate,
        }
    }

    fn cap(&self) -> usize {
        self.buf.len() / 2
    }

    /// Producer side: appends interleaved frames with `channels` channels.
    pub fn push(&self, data: &[f32], channels: usize) {
        let channels = channels.max(1);
        let mut w = self.write.load(Ordering::Relaxed);
        for f in data.chunks_exact(channels) {
            let (l, r) = (f[0], if channels > 1 { f[1] } else { f[0] });
            let i = (w % self.cap()) * 2;
            self.buf[i].store(l.to_bits(), Ordering::Relaxed);
            self.buf[i + 1].store(r.to_bits(), Ordering::Relaxed);
            w = w.wrapping_add(1);
        }
        self.write.store(w, Ordering::Release);
    }

    /// Consumer side: adds up to `out.len() / 2` frames into `out`.
    pub fn mix_into(&self, out: &mut [f32]) {
        let w = self.write.load(Ordering::Acquire);
        let mut r = self.read.load(Ordering::Relaxed);
        let available = w.wrapping_sub(r);
        if available > self.cap() / 2 {
            // Fell behind (or just started): skip to recent audio to keep latency low.
            r = w.wrapping_sub(out.len() / 2);
        }
        for f in out.chunks_exact_mut(2) {
            if r == w {
                break;
            }
            let i = (r % self.cap()) * 2;
            f[0] += f32::from_bits(self.buf[i].load(Ordering::Relaxed));
            f[1] += f32::from_bits(self.buf[i + 1].load(Ordering::Relaxed));
            r = r.wrapping_add(1);
        }
        self.read.store(r, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::MonitorRing;

    #[test]
    fn passes_audio_through() {
        let ring = MonitorRing::new(64, 48000);
        ring.push(&[0.5, 0.25], 1); // mono -> both sides
        let mut out = [0.0f32; 6];
        ring.mix_into(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.25, 0.25, 0.0, 0.0]);
        // Nothing new: output untouched.
        let mut out = [0.0f32; 2];
        ring.mix_into(&mut out);
        assert_eq!(out, [0.0, 0.0]);
    }
}
