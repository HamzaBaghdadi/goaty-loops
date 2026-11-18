//! Real-time audio output (cpal, or WASAPI low-latency/exclusive on Windows)
//! and the state shared with the UI.

use crate::model::{Project, MAX_CHANNELS, MAX_MIXER_TRACKS};
use crate::render::Renderer;
use crate::sample::SampleData;
use anyhow::{anyhow, Result};
use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// Messages from the UI to the audio thread.
pub enum Command {
    NoteOn { channel: usize, key: u8, velocity: f32 },
    NoteOff { channel: usize, key: u8 },
    PreviewSample(Arc<SampleData>),
    StopPreview,
    AllNotesOff,
    /// Live pitch wheel for a channel, in semitones.
    /// `wheel` is the raw position (0..=1, 0.5 = centre) for plugin instruments.
    PitchBend { channel: usize, semis: f32, wheel: f32 },
    /// Overrides a parameter the way automation does (normalized 0..=1);
    /// for targets not stored in the project, e.g. FL effect parameters.
    SetParam { target: crate::model::AutoTarget, value: f32 },
}

const NO_SEEK: u64 = u64::MAX;

pub struct Transport {
    pub playing: AtomicBool,
    pub song_mode: AtomicBool,
    pub pattern: AtomicUsize,
    position: AtomicU64,
    seek: AtomicU64,
}

impl Transport {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            song_mode: AtomicBool::new(false),
            pattern: AtomicUsize::new(0),
            position: AtomicU64::new(0f64.to_bits()),
            seek: AtomicU64::new(NO_SEEK),
        }
    }

    /// Current position in ticks.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }

    pub(crate) fn set_position(&self, ticks: f64) {
        self.position.store(ticks.to_bits(), Ordering::Relaxed);
    }

    pub fn seek(&self, ticks: f64) {
        self.seek.store(ticks.max(0.0).to_bits(), Ordering::Relaxed);
        self.set_position(ticks.max(0.0));
    }

    pub(crate) fn take_seek(&self) -> Option<f64> {
        let v = self.seek.swap(NO_SEEK, Ordering::Relaxed);
        (v != NO_SEEK).then(|| f64::from_bits(v))
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

/// Peak meters written by the audio thread and drained by the UI.
pub struct Meters {
    mixer: Vec<[AtomicU32; 2]>,
    channels: Vec<AtomicU32>,
}

fn bump(a: &AtomicU32, v: f32) {
    if v > f32::from_bits(a.load(Ordering::Relaxed)) {
        a.store(v.to_bits(), Ordering::Relaxed);
    }
}

impl Meters {
    fn new() -> Self {
        Self {
            mixer: (0..MAX_MIXER_TRACKS).map(|_| [AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            channels: (0..MAX_CHANNELS).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    pub(crate) fn bump_mixer(&self, track: usize, l: f32, r: f32) {
        if let Some(m) = self.mixer.get(track) {
            bump(&m[0], l);
            bump(&m[1], r);
        }
    }

    pub(crate) fn bump_channel(&self, ch: usize, v: f32) {
        if let Some(m) = self.channels.get(ch) {
            bump(m, v);
        }
    }

    /// Returns and resets the peak since the last call.
    pub fn take_mixer(&self, track: usize) -> (f32, f32) {
        match self.mixer.get(track) {
            Some(m) => (f32::from_bits(m[0].swap(0, Ordering::Relaxed)), f32::from_bits(m[1].swap(0, Ordering::Relaxed))),
            None => (0.0, 0.0),
        }
    }

    pub fn take_channel(&self, ch: usize) -> f32 {
        self.channels.get(ch).map(|m| f32::from_bits(m.swap(0, Ordering::Relaxed))).unwrap_or(0.0)
    }
}

pub struct Shared {
    pub project: ArcSwap<Project>,
    pub transport: Transport,
    pub meters: Meters,
    /// Fraction of the buffer period spent rendering.
    cpu: AtomicU32,
    /// Audio input routed to the output while recording (input monitoring).
    pub monitor: arc_swap::ArcSwapOption<crate::record::MonitorRing>,
    /// Total plugin delay compensation applied to the output, in samples.
    pub latency_samples: AtomicU32,
    /// Set by the audio backend when the output device disappears.
    device_lost: AtomicBool,
    /// Output latency measured by cpal (callback to playback), in microseconds.
    measured_latency_us: AtomicU32,
    /// Wakes the UI (which doesn't redraw while idle) from the audio side.
    waker: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// What the audio thread stopped using, freed on the UI thread by
    /// [`Shared::collect_garbage`]: plugin teardown (which some plugins only
    /// allow on the main thread) and deallocation never run on the audio thread.
    retired: (Sender<Retired>, Receiver<Retired>),
}

/// Something the audio thread no longer uses (see `Shared::retired`).
pub(crate) enum Retired {
    Project(#[allow(dead_code)] Arc<Project>),
    Effects(#[allow(dead_code)] Vec<crate::effects::FxState>),
}

impl Shared {
    pub fn new(project: Project) -> Self {
        Self {
            project: ArcSwap::from_pointee(project),
            transport: Transport::new(),
            meters: Meters::new(),
            cpu: AtomicU32::new(0),
            monitor: Default::default(),
            latency_samples: AtomicU32::new(0),
            device_lost: AtomicBool::new(false),
            measured_latency_us: AtomicU32::new(0),
            waker: OnceLock::new(),
            retired: crossbeam_channel::bounded(256),
        }
    }

    pub fn cpu_load(&self) -> f32 {
        f32::from_bits(self.cpu.load(Ordering::Relaxed))
    }

    /// Sets the function that wakes the UI (only the first call has effect).
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        let _ = self.waker.set(Box::new(wake));
    }

    pub fn has_waker(&self) -> bool {
        self.waker.get().is_some()
    }

    /// Hands something the audio thread no longer uses to the UI thread. The
    /// queue is preallocated; only if the UI stops draining it is the item
    /// dropped here.
    pub(crate) fn retire(&self, item: Retired) {
        let _ = self.retired.0.try_send(item);
    }

    /// Frees what the audio thread retired. Call regularly from the UI thread.
    pub fn collect_garbage(&self) {
        while self.retired.1.try_recv().is_ok() {}
    }

    fn report_device_lost(&self) {
        self.device_lost.store(true, Ordering::Release);
        if let Some(w) = self.waker.get() {
            w();
        }
    }
}

/// How audio reaches the sound card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Driver {
    /// The system's shared output (Windows: WASAPI shared mode at the system
    /// period, about 10 ms). Works everywhere; other apps keep playing.
    #[default]
    Shared,
    /// WASAPI shared mode at the smallest period the driver allows (Windows 10+).
    LowLatency,
    /// WASAPI exclusive mode: own sample rate and buffer size; other apps
    /// can't play sound on that device meanwhile.
    Exclusive,
}

impl Driver {
    pub fn all() -> &'static [Driver] {
        if cfg!(windows) {
            &[Driver::Shared, Driver::LowLatency, Driver::Exclusive]
        } else {
            &[Driver::Shared]
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Driver::Shared if cfg!(windows) => "Windows audio (shared)",
            Driver::Shared => "System audio",
            Driver::LowLatency => "Windows audio, low latency (shared)",
            Driver::Exclusive => "Windows audio, exclusive (WASAPI)",
        }
    }
}

/// The user's audio output choices (persisted by the app).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub driver: Driver,
    /// Output device name; `None` = the system default.
    pub device: Option<String>,
    /// Exclusive mode only; `None` = the device's current rate.
    pub sample_rate: Option<u32>,
    /// Buffer size in frames (low-latency and exclusive modes); `None` = smallest.
    pub buffer_frames: Option<u32>,
}

/// What is actually running.
#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    pub driver: Driver,
    pub device_name: String,
    pub sample_rate: u32,
    /// Frames rendered per wake-up, when the driver lets us choose it.
    pub buffer_frames: Option<u32>,
    /// Output latency known when opening (WASAPI backends), in frames.
    pub latency_frames: Option<u32>,
    /// Sample format sent to the device, when known.
    pub format: Option<&'static str>,
}

/// Sample rates and buffer sizes a device offers with a driver.
#[derive(Clone, Debug, Default)]
pub struct DriverCaps {
    /// The device's current rate (what "device default" means).
    pub current_rate: u32,
    pub sample_rates: Vec<u32>,
    /// Empty when the system picks the buffer size.
    pub buffer_frames: Vec<u32>,
    pub default_buffer: Option<u32>,
}

/// Output device names for `driver`.
pub fn output_devices(driver: Driver) -> Vec<String> {
    #[cfg(windows)]
    if driver != Driver::Shared {
        return crate::wasapi::output_devices();
    }
    let _ = driver;
    cpal::default_host().output_devices().map(|d| d.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// What `device` (or the default) offers with `driver`. Opens the device
/// briefly, so call it when the choice changes, not every frame.
pub fn driver_caps(driver: Driver, device: Option<&str>, sample_rate: Option<u32>) -> Result<DriverCaps, String> {
    #[cfg(windows)]
    if driver != Driver::Shared {
        let mode = if driver == Driver::Exclusive { crate::wasapi::Mode::Exclusive } else { crate::wasapi::Mode::LowLatency };
        let c = crate::wasapi::caps(mode, device.map(str::to_string), sample_rate)?;
        return Ok(DriverCaps { current_rate: c.current_rate, sample_rates: c.sample_rates, buffer_frames: c.buffer_frames, default_buffer: Some(c.default_buffer) });
    }
    let _ = (driver, sample_rate);
    let dev = cpal_device(device).map_err(|e| e.to_string())?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    Ok(DriverCaps { current_rate: cfg.sample_rate().0, sample_rates: vec![cfg.sample_rate().0], buffer_frames: Vec::new(), default_buffer: None })
}

enum Backend {
    Cpal(#[allow(dead_code)] cpal::Stream),
    #[cfg(windows)]
    Wasapi(#[allow(dead_code)] crate::wasapi::Stream),
}

pub struct AudioEngine {
    pub shared: Arc<Shared>,
    pub sample_rate: u32,
    pub device_name: String,
    pub error: Option<String>,
    pub info: StreamInfo,
    tx: Sender<Command>,
    rx: Receiver<Command>,
    /// The running stream; dropping it stops the audio.
    backend: Option<Backend>,
}

impl AudioEngine {
    /// Opens the output described by `config`, falling back to the system
    /// default (shared) if that fails; `error` then says why.
    pub fn new(project: Project, config: &AudioConfig) -> Self {
        let shared = Arc::new(Shared::new(project));
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let mut engine = Self { shared, sample_rate: 44100, device_name: "none".into(), error: None, info: StreamInfo::default(), tx, rx, backend: None };
        let _ = engine.reopen(config);
        engine
    }

    /// Replaces the running stream with `config`. On failure the system
    /// default output (shared) is opened instead and the error is returned.
    pub fn reopen(&mut self, config: &AudioConfig) -> Result<(), String> {
        // Stop the old stream first: its renderer must not run alongside the
        // new one (they share plugin instances).
        self.backend = None;
        self.shared.collect_garbage();
        self.shared.device_lost.store(false, Ordering::Release);
        self.shared.measured_latency_us.store(0, Ordering::Relaxed);
        // The new renderer starts where playback was.
        let t = &self.shared.transport;
        t.seek(t.position());
        match self.open(config) {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(e) => {
                // Same driver on the default device (the chosen one may be
                // unplugged), then the plain system output.
                let mut fallbacks = Vec::new();
                if config.device.is_some() {
                    fallbacks.push((AudioConfig { device: None, ..config.clone() }, "the default device"));
                }
                fallbacks.push((AudioConfig::default(), "the system default output"));
                for (f, what) in fallbacks {
                    if f != *config && self.open(&f).is_ok() {
                        self.error = Some(format!("{e}. Using {what} instead."));
                        return Err(e);
                    }
                }
                self.error = Some(e.clone());
                Err(e)
            }
        }
    }

    fn open(&mut self, config: &AudioConfig) -> Result<(), String> {
        let (backend, info) = match config.driver {
            #[cfg(windows)]
            Driver::LowLatency | Driver::Exclusive => open_wasapi(self.shared.clone(), self.rx.clone(), config)?,
            _ => open_cpal(self.shared.clone(), self.rx.clone(), config.device.as_deref()).map_err(|e| e.to_string())?,
        };
        self.sample_rate = info.sample_rate;
        self.device_name = info.device_name.clone();
        self.info = info;
        self.backend = Some(backend);
        Ok(())
    }

    /// True once after the output device disappeared (unplugged, disabled).
    pub fn take_device_lost(&self) -> bool {
        self.shared.device_lost.swap(false, Ordering::AcqRel)
    }

    /// Output latency in milliseconds: from the driver when it reports it,
    /// otherwise as measured from the stream's timestamps.
    pub fn output_latency_ms(&self) -> Option<f32> {
        if let Some(f) = self.info.latency_frames {
            return Some(f as f32 * 1000.0 / self.sample_rate.max(1) as f32);
        }
        let us = self.shared.measured_latency_us.load(Ordering::Relaxed);
        (us > 0 && self.backend.is_some()).then(|| us as f32 / 1000.0)
    }

    /// A sender for other threads (e.g. MIDI input) to reach the audio thread.
    pub fn command_sender(&self) -> Sender<Command> {
        self.tx.clone()
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.try_send(cmd);
    }

    /// Makes `project` the version the audio thread plays.
    pub fn publish(&self, project: &Project) {
        self.shared.project.store(Arc::new(project.clone()));
    }

    pub fn transport(&self) -> &Transport {
        &self.shared.transport
    }
}

/// The renderer plus what every backend's callback needs.
struct Output {
    renderer: Renderer,
    shared: Arc<Shared>,
    sample_rate: u32,
}

impl Output {
    fn new(shared: Arc<Shared>, rx: Receiver<Command>, sample_rate: u32) -> Self {
        Self { renderer: Renderer::new(sample_rate, Some(rx)), shared, sample_rate }
    }

    /// Renders interleaved stereo into `out` and records the CPU load.
    fn render(&mut self, out: &mut [f32]) {
        let start = Instant::now();
        self.renderer.process(&self.shared, out);
        if let Some(m) = self.shared.monitor.load().as_ref() {
            m.mix_into(out);
        }
        let period = (out.len() / 2) as f32 / self.sample_rate as f32;
        let load = start.elapsed().as_secs_f32() / period.max(1e-6);
        self.shared.cpu.store(load.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn open_wasapi(shared: Arc<Shared>, rx: Receiver<Command>, config: &AudioConfig) -> Result<(Backend, StreamInfo), String> {
    use crate::wasapi;
    let mode = if config.driver == Driver::Exclusive { wasapi::Mode::Exclusive } else { wasapi::Mode::LowLatency };
    let req = wasapi::Request { mode, device: config.device.clone(), sample_rate: config.sample_rate, buffer_frames: config.buffer_frames };
    let for_render = shared.clone();
    // Built on the audio thread once the device's sample rate is known.
    let make_render: wasapi::MakeRender = Box::new(move |info: &wasapi::Opened| {
        let mut output = Output::new(for_render, rx, info.sample_rate);
        Box::new(move |out: &mut [f32]| output.render(out))
    });
    let on_error: wasapi::OnError = Box::new(move |e: String| {
        eprintln!("audio output: {e}");
        shared.report_device_lost();
    });
    let stream = wasapi::open(req, make_render, on_error)?;
    let i = &stream.info;
    let info = StreamInfo {
        driver: config.driver,
        device_name: i.device_name.clone(),
        sample_rate: i.sample_rate,
        buffer_frames: Some(i.buffer_frames),
        latency_frames: Some(i.latency_frames),
        format: Some(i.format),
    };
    Ok((Backend::Wasapi(stream), info))
}

fn cpal_device(name: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match name {
        Some(n) => host
            .output_devices()?
            .find(|d| d.name().ok().as_deref() == Some(n))
            .ok_or_else(|| anyhow!("Audio device \"{n}\" was not found")),
        None => host.default_output_device().ok_or_else(|| anyhow!("no audio output device")),
    }
}

fn open_cpal(shared: Arc<Shared>, rx: Receiver<Command>, device: Option<&str>) -> Result<(Backend, StreamInfo)> {
    let device = cpal_device(device)?;
    let name = device.name().unwrap_or_else(|_| "unknown".into());
    let supported = device.default_output_config()?;
    let config: cpal::StreamConfig = supported.config();
    let sr = config.sample_rate.0;
    let output = Output::new(shared.clone(), rx, sr);
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, output, shared)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, output, shared)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, output, shared)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, output, shared)?,
        f => return Err(anyhow!("unsupported sample format {f:?}")),
    };
    stream.play()?;
    let info = StreamInfo { driver: Driver::Shared, device_name: name, sample_rate: sr, buffer_frames: None, latency_frames: None, format: None };
    Ok((Backend::Cpal(stream), info))
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut output: Output, shared: Arc<Shared>) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut buf = vec![0f32; 8192 * 2];
    let latency = shared.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;
            if buf.len() < frames * 2 {
                buf.resize(frames * 2, 0.0);
            }
            let buf = &mut buf[..frames * 2];
            output.render(buf);
            for (i, frame) in data.chunks_mut(channels).enumerate() {
                let (l, r) = (buf[i * 2], buf[i * 2 + 1]);
                for (c, s) in frame.iter_mut().enumerate() {
                    let v = match (channels, c) {
                        (1, _) => (l + r) * 0.5,
                        (_, 0) => l,
                        (_, 1) => r,
                        _ => 0.0,
                    };
                    *s = T::from_sample(v);
                }
            }
            let ts = info.timestamp();
            if let Some(d) = ts.playback.duration_since(&ts.callback) {
                latency.measured_latency_us.store(d.as_micros().min(u32::MAX as u128) as u32, Ordering::Relaxed);
            }
        },
        move |e| match e {
            cpal::StreamError::DeviceNotAvailable => shared.report_device_lost(),
            e => eprintln!("audio stream error: {e}"),
        },
        None,
    )?;
    Ok(stream)
}
