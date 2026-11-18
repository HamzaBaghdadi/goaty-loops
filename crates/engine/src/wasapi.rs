//! Low-latency audio output on Windows, straight on WASAPI.
//!
//! cpal only offers WASAPI shared mode at the system period (about 10 ms of
//! buffering). This module adds:
//! - **low-latency shared mode** (`IAudioClient3`): the smallest period the
//!   driver allows (often 128-144 samples), other apps keep playing;
//! - **exclusive mode**: the app owns the device, with its own sample rate and
//!   buffer size, bypassing the Windows mixer (what "FL ASIO"-style drivers give).
//!
//! Every stream runs on its own thread, registered with MMCSS as "Pro Audio",
//! woken by the device's event. All COM objects live on that thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use windows::core::{Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::Properties;
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{self, StructuredStorage, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_LPWSTR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    LowLatency,
    Exclusive,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub mode: Mode,
    /// Device friendly name; `None` = the Windows default output.
    pub device: Option<String>,
    /// Exclusive mode only; `None` = the device's current rate.
    pub sample_rate: Option<u32>,
    /// Wanted buffer in frames; `None` = the smallest the device allows.
    pub buffer_frames: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Opened {
    pub device_name: String,
    pub sample_rate: u32,
    /// Frames per device period (what each wake-up renders).
    pub buffer_frames: u32,
    /// Output latency reported by the device plus one period, in frames.
    pub latency_frames: u32,
    /// Sample format sent to the device, e.g. "32-bit float".
    pub format: &'static str,
}

/// What a device offers in a mode, for the settings UI.
#[derive(Clone, Debug, Default)]
pub struct Caps {
    /// The device's current rate (what "device default" means).
    pub current_rate: u32,
    pub sample_rates: Vec<u32>,
    pub buffer_frames: Vec<u32>,
    pub default_buffer: u32,
}

/// Fills an interleaved stereo buffer (`frames * 2` samples).
pub type Render = Box<dyn FnMut(&mut [f32]) + Send>;
/// Builds the render callback once the stream format is known (on the audio thread).
pub type MakeRender = Box<dyn FnOnce(&Opened) -> Render + Send>;
/// Called from the audio thread when the device goes away.
pub type OnError = Box<dyn Fn(String) + Send>;

pub struct Stream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub info: Opened,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Output devices by friendly name (the names cpal reports too).
pub fn output_devices() -> Vec<String> {
    on_com_thread(|| unsafe {
        let Ok(e) = enumerator() else { return Vec::new() };
        let Ok(list) = e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return Vec::new() };
        let n = list.GetCount().unwrap_or(0);
        (0..n).filter_map(|i| list.Item(i).ok()).filter_map(|d| device_name(&d)).collect()
    })
}

/// Sample rates and buffer sizes the device offers in `mode`; buffer sizes
/// are for `rate` (exclusive mode; `None` = the device's current rate).
pub fn caps(mode: Mode, device: Option<String>, rate: Option<u32>) -> Result<Caps, String> {
    on_com_thread(move || unsafe {
        let (dev, _) = find_device(device.as_deref())?;
        match mode {
            Mode::LowLatency => {
                let client: IAudioClient3 = dev.Activate(CLSCTX_ALL, None).map_err(|_| "Low-latency mode needs Windows 10 or later".to_string())?;
                let mix = MixFormat(client.GetMixFormat().map_err(err)?);
                let rate = (*mix.0).nSamplesPerSec;
                let (mut def, mut fund, mut min, mut max) = (0, 0, 0, 0);
                client.GetSharedModeEnginePeriod(mix.0, &mut def, &mut fund, &mut min, &mut max).map_err(err)?;
                Ok(Caps { current_rate: rate, sample_rates: vec![rate], buffer_frames: shared_periods(def, fund, min, max), default_buffer: min })
            }
            Mode::Exclusive => {
                let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(err)?;
                let mix = MixFormat(client.GetMixFormat().map_err(err)?);
                let mix_rate = (*mix.0).nSamplesPerSec;
                let rates: Vec<u32> = [44100, 48000, 88200, 96000, 176400, 192000]
                    .into_iter()
                    .filter(|&r| exclusive_format(&client, r).is_some())
                    .collect();
                let (mut def, mut min) = (0i64, 0i64);
                client.GetDevicePeriod(Some(&mut def), Some(&mut min)).map_err(err)?;
                // The minimum period is a duration, so in frames it grows with the rate.
                let min_frames = hns_to_frames(min, rate.unwrap_or(mix_rate));
                // The device minimum first, then common sizes above it.
                let buffers: Vec<u32> = std::iter::once(min_frames)
                    .chain([64, 96, 128, 192, 256, 384, 512, 768, 1024, 2048].into_iter().filter(|&f| f > min_frames))
                    .collect();
                Ok(Caps { current_rate: mix_rate, sample_rates: if rates.is_empty() { vec![mix_rate] } else { rates }, default_buffer: min_frames, buffer_frames: buffers })
            }
        }
    })
}

/// Opens the device and starts calling `render` from a real-time thread.
pub fn open(req: Request, make_render: MakeRender, on_error: OnError) -> Result<Stream, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let thread = std::thread::Builder::new()
        .name("audio output (WASAPI)".into())
        .spawn(move || {
            let _com = Com_::init();
            match unsafe { setup(&req) } {
                Ok(s) => {
                    let _ = tx.send(Ok(s.info.clone()));
                    let render = make_render(&s.info);
                    unsafe { run(s, render, &stop2, on_error) };
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    match rx.recv() {
        Ok(Ok(info)) => Ok(Stream { stop, thread: Some(thread), info }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err("The audio thread stopped while opening the device".into())
        }
    }
}

// ---- setup ---------------------------------------------------------------

struct Setup {
    client: IAudioClient,
    render_client: IAudioRenderClient,
    event: HANDLE,
    sample: Sample,
    channels: usize,
    buffer: u32,
    exclusive: bool,
    info: Opened,
}

unsafe fn setup(req: &Request) -> Result<Setup, String> {
    let (dev, name) = find_device(req.device.as_deref())?;
    let event = CreateEventW(None, false, false, PCWSTR::null()).map_err(err)?;
    let result = match req.mode {
        Mode::LowLatency => setup_low_latency(&dev, req, event),
        Mode::Exclusive => setup_exclusive(&dev, req, event),
    };
    match result {
        Ok(mut s) => {
            s.info.device_name = name;
            Ok(s)
        }
        Err(e) => {
            let _ = CloseHandle(event);
            Err(e)
        }
    }
}

unsafe fn setup_low_latency(dev: &IMMDevice, req: &Request, event: HANDLE) -> Result<Setup, String> {
    let client3: IAudioClient3 = dev.Activate(CLSCTX_ALL, None).map_err(|_| "Low-latency mode needs Windows 10 or later".to_string())?;
    let mix = MixFormat(client3.GetMixFormat().map_err(err)?);
    let (sample, channels, rate) = read_format(mix.0).ok_or("The device's mix format isn't supported")?;
    let (mut def, mut fund, mut min, mut max) = (0, 0, 0, 0);
    client3.GetSharedModeEnginePeriod(mix.0, &mut def, &mut fund, &mut min, &mut max).map_err(err)?;
    let period = pick_shared_period(req.buffer_frames.unwrap_or(min), def, fund, min, max);
    client3.InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, mix.0, None).map_err(err)?;
    let client: IAudioClient = client3.cast().map_err(err)?;
    finish(client, event, sample, channels, rate, period, false)
}

unsafe fn setup_exclusive(dev: &IMMDevice, req: &Request, event: HANDLE) -> Result<Setup, String> {
    let mut client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(err)?;
    let mix_rate = {
        let mix = MixFormat(client.GetMixFormat().map_err(err)?);
        (*mix.0).nSamplesPerSec
    };
    let rate = req.sample_rate.unwrap_or(mix_rate);
    let (sample, fmt) = exclusive_format(&client, rate).ok_or_else(|| format!("The device doesn't support {rate} Hz in exclusive mode"))?;
    let (mut def, mut min) = (0i64, 0i64);
    client.GetDevicePeriod(Some(&mut def), Some(&mut min)).map_err(err)?;
    let want = req.buffer_frames.map(|f| frames_to_hns(f, rate)).unwrap_or(min);
    let mut hns = want.max(min);
    let fmt_ptr = &fmt as *const WaveFormatExtensible as *const WAVEFORMATEX;
    if let Err(e) = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, hns, hns, fmt_ptr, None) {
        if e.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
            return Err(err(e));
        }
        // The driver wants a slightly different size: ask for exactly the
        // size it rounded to, on a fresh client.
        let frames = client.GetBufferSize().map_err(err)?;
        hns = frames_to_hns(frames, rate);
        client = dev.Activate(CLSCTX_ALL, None).map_err(err)?;
        client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, hns, hns, fmt_ptr, None).map_err(err)?;
    }
    let frames = client.GetBufferSize().map_err(err)?;
    finish(client, event, sample, 2, rate, frames, true)
}

unsafe fn finish(client: IAudioClient, event: HANDLE, sample: Sample, channels: usize, rate: u32, period: u32, exclusive: bool) -> Result<Setup, String> {
    client.SetEventHandle(event).map_err(err)?;
    let buffer = client.GetBufferSize().map_err(err)?;
    let render_client: IAudioRenderClient = client.GetService().map_err(err)?;
    let stream_latency = client.GetStreamLatency().map(|h| hns_to_frames(h, rate)).unwrap_or(0);
    let info = Opened { device_name: String::new(), sample_rate: rate, buffer_frames: period, latency_frames: period + stream_latency, format: sample.label() };
    Ok(Setup { client, render_client, event, sample, channels, buffer, exclusive, info })
}

/// Picks a supported exclusive format at `rate`: float, then 32/24/16-bit PCM.
unsafe fn exclusive_format(client: &IAudioClient, rate: u32) -> Option<(Sample, WaveFormatExtensible)> {
    [Sample::F32, Sample::I32, Sample::I24, Sample::I16].into_iter().find_map(|s| {
        let fmt = make_format(rate, 2, s);
        let ok = client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &fmt as *const WaveFormatExtensible as *const WAVEFORMATEX, None) == S_OK;
        ok.then_some((s, fmt))
    })
}

// ---- the real-time loop --------------------------------------------------

unsafe fn run(s: Setup, mut render: Render, stop: &AtomicBool, on_error: OnError) {
    let mut task_index = 0u32;
    let task = AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut task_index).ok();
    let mut tmp = vec![0f32; s.buffer as usize * 2];
    let lost = |e: windows::core::Error| on_error(describe(e.code()));

    // Start from a buffer of silence.
    if let Ok(p) = s.render_client.GetBuffer(s.buffer) {
        let _ = s.render_client.ReleaseBuffer(s.buffer, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
        let _ = p;
    }
    if let Err(e) = s.client.Start() {
        lost(e);
    } else {
        while !stop.load(Ordering::Acquire) {
            if WaitForSingleObject(s.event, 500) != WAIT_OBJECT_0 {
                // No wake-up for half a second: check the device still exists.
                if let Err(e) = s.client.GetCurrentPadding() {
                    lost(e);
                    break;
                }
                continue;
            }
            let frames = if s.exclusive {
                s.buffer
            } else {
                match s.client.GetCurrentPadding() {
                    Ok(p) => s.buffer.saturating_sub(p),
                    Err(e) => {
                        lost(e);
                        break;
                    }
                }
            };
            if frames == 0 {
                continue;
            }
            let data = match s.render_client.GetBuffer(frames) {
                Ok(p) => p,
                Err(e) => {
                    lost(e);
                    break;
                }
            };
            let out = &mut tmp[..frames as usize * 2];
            render(out);
            write_frames(s.sample, data, s.channels, out);
            if let Err(e) = s.render_client.ReleaseBuffer(frames, 0) {
                lost(e);
                break;
            }
        }
        let _ = s.client.Stop();
    }
    if let Some(t) = task {
        let _ = AvRevertMmThreadCharacteristics(t);
    }
    let _ = CloseHandle(s.event);
}

// ---- formats -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sample {
    F32,
    I32,
    I24,
    I16,
}

impl Sample {
    fn bytes(self) -> usize {
        match self {
            Sample::F32 | Sample::I32 => 4,
            Sample::I24 => 3,
            Sample::I16 => 2,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Sample::F32 => "32-bit float",
            Sample::I32 => "32-bit integer",
            Sample::I24 => "24-bit integer",
            Sample::I16 => "16-bit integer",
        }
    }
}

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WaveFormatExtensible {
    format: WAVEFORMATEX,
    valid_bits: u16,
    channel_mask: u32,
    sub_format: GUID,
}

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

fn make_format(rate: u32, channels: u16, s: Sample) -> WaveFormatExtensible {
    let container = s.bytes() as u16 * 8;
    let block = channels * s.bytes() as u16;
    WaveFormatExtensible {
        format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * block as u32,
            nBlockAlign: block,
            wBitsPerSample: container,
            cbSize: 22,
        },
        valid_bits: if s == Sample::I32 { 32 } else { container },
        channel_mask: 0x3, // front left | front right
        sub_format: if s == Sample::F32 { SUBTYPE_FLOAT } else { SUBTYPE_PCM },
    }
}

/// Sample type, channel count and rate of a device format.
unsafe fn read_format(p: *const WAVEFORMATEX) -> Option<(Sample, usize, u32)> {
    let f = std::ptr::read_unaligned(p);
    let (tag, bits, channels, rate) = (f.wFormatTag, f.wBitsPerSample, f.nChannels as usize, f.nSamplesPerSec);
    let float = match tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_PCM => false,
        WAVE_FORMAT_EXTENSIBLE => {
            let sub = std::ptr::read_unaligned(p as *const WaveFormatExtensible).sub_format;
            if sub == SUBTYPE_FLOAT {
                true
            } else if sub == SUBTYPE_PCM {
                false
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let sample = match (float, bits) {
        (true, 32) => Sample::F32,
        (false, 32) => Sample::I32,
        (false, 24) => Sample::I24,
        (false, 16) => Sample::I16,
        _ => return None,
    };
    (channels > 0).then_some((sample, channels, rate))
}

/// Converts interleaved stereo `src` into the device's format and channel
/// layout (mono gets the average, extra channels silence).
unsafe fn write_frames(s: Sample, dst: *mut u8, channels: usize, src: &[f32]) {
    let size = s.bytes();
    let mut at = dst;
    for frame in src.chunks_exact(2) {
        let (l, r) = (frame[0], frame[1]);
        for c in 0..channels {
            let v = match (channels, c) {
                (1, _) => (l + r) * 0.5,
                (_, 0) => l,
                (_, 1) => r,
                _ => 0.0,
            };
            write_sample(s, at, v);
            at = at.add(size);
        }
    }
}

unsafe fn write_sample(s: Sample, at: *mut u8, v: f32) {
    let v = if v.is_finite() { v } else { 0.0 };
    match s {
        Sample::F32 => std::ptr::write_unaligned(at as *mut f32, v),
        Sample::I32 => std::ptr::write_unaligned(at as *mut i32, (v.clamp(-1.0, 1.0) as f64 * i32::MAX as f64) as i32),
        Sample::I24 => {
            let x = (v.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
            let b = x.to_le_bytes();
            std::ptr::copy_nonoverlapping(b.as_ptr(), at, 3);
        }
        Sample::I16 => std::ptr::write_unaligned(at as *mut i16, (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16),
    }
}

// ---- helpers -------------------------------------------------------------

/// Period choices in low-latency shared mode: multiples of the fundamental
/// period from the minimum up to the default (the normal shared period).
fn shared_periods(def: u32, fund: u32, min: u32, max: u32) -> Vec<u32> {
    if fund == 0 || min == 0 {
        return vec![def.max(min)];
    }
    let top = def.max(min).min(max.max(min));
    let mut v: Vec<u32> = (0..).map(|k| min + k * fund).take_while(|&p| p <= top).collect();
    // Keep the list short: thin it out evenly if the fundamental is tiny.
    while v.len() > 12 {
        v = v.iter().copied().step_by(2).collect();
    }
    if v.last() != Some(&top) {
        v.push(top);
    }
    v
}

fn pick_shared_period(want: u32, def: u32, fund: u32, min: u32, max: u32) -> u32 {
    if fund == 0 {
        return def.max(min);
    }
    let k = (want.saturating_sub(min) + fund / 2) / fund;
    (min + k * fund).clamp(min, max.max(min))
}

fn frames_to_hns(frames: u32, rate: u32) -> i64 {
    (10_000_000.0 * frames as f64 / rate.max(1) as f64 + 0.5) as i64
}

fn hns_to_frames(hns: i64, rate: u32) -> u32 {
    (hns.max(0) as f64 * rate as f64 / 10_000_000.0).round() as u32
}

fn err(e: windows::core::Error) -> String {
    describe(e.code())
}

fn describe(code: HRESULT) -> String {
    match code {
        AUDCLNT_E_DEVICE_INVALIDATED => "The audio device was disconnected".into(),
        AUDCLNT_E_DEVICE_IN_USE => "Another app is using the device in exclusive mode".into(),
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
            "Exclusive mode is turned off for this device (Windows Sound settings > device properties > Advanced)".into()
        }
        AUDCLNT_E_UNSUPPORTED_FORMAT => "The device doesn't support this format".into(),
        c => format!("{} (0x{:08x})", windows::core::Error::from(c).message(), c.0 as u32),
    }
}

/// The device with this friendly name, or the default output.
unsafe fn find_device(name: Option<&str>) -> Result<(IMMDevice, String), String> {
    let e = enumerator()?;
    if let Some(name) = name {
        let list = e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(err)?;
        for i in 0..list.GetCount().unwrap_or(0) {
            if let Ok(d) = list.Item(i) {
                if device_name(&d).as_deref() == Some(name) {
                    return Ok((d, name.to_string()));
                }
            }
        }
        return Err(format!("Audio device \"{name}\" was not found"));
    }
    let d = e.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|_| "No audio output device".to_string())?;
    let n = device_name(&d).unwrap_or_else(|| "Default device".into());
    Ok((d, n))
}

unsafe fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    Com::CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(err)
}

unsafe fn device_name(d: &IMMDevice) -> Option<String> {
    let store = d.OpenPropertyStore(STGM_READ).ok()?;
    let mut value = store.GetValue(&Properties::DEVPKEY_Device_FriendlyName as *const _ as *const _).ok()?;
    let raw = &value.as_raw().Anonymous.Anonymous;
    let name = if raw.vt == VT_LPWSTR.0 {
        let p = *(&raw.Anonymous as *const _ as *const *const u16);
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)))
    } else {
        None
    };
    let _ = StructuredStorage::PropVariantClear(&mut value);
    name
}

/// A format returned by `GetMixFormat`, freed with `CoTaskMemFree`.
struct MixFormat(*mut WAVEFORMATEX);

impl Drop for MixFormat {
    fn drop(&mut self) {
        unsafe { Com::CoTaskMemFree(Some(self.0 as *const _)) };
    }
}

/// COM for the current thread (multithreaded apartment), released on drop.
#[allow(non_camel_case_types)]
struct Com_(bool);

impl Com_ {
    fn init() -> Self {
        Self(unsafe { Com::CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com_ {
    fn drop(&mut self) {
        if self.0 {
            unsafe { Com::CoUninitialize() };
        }
    }
}

/// Runs `f` on a short-lived thread with its own COM apartment, so queries
/// from the UI thread (already set up for drag and drop) don't clash.
fn on_com_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::spawn(move || {
        let _com = Com_::init();
        f()
    })
    .join()
    .expect("COM helper thread panicked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_period_choices() {
        // A typical Realtek driver: 10 ms default, 2.67 ms minimum at 48 kHz.
        let v = shared_periods(480, 48, 144, 480);
        assert_eq!(v.first(), Some(&144));
        assert_eq!(v.last(), Some(&480));
        assert!(v.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(pick_shared_period(200, 480, 48, 144, 480), 192);
        assert_eq!(pick_shared_period(1, 480, 48, 144, 480), 144);
        assert_eq!(pick_shared_period(9999, 480, 48, 144, 480), 480);
        // Drivers without low-latency support only offer the default.
        assert_eq!(shared_periods(480, 0, 480, 480), vec![480]);
    }

    #[test]
    fn writes_each_format() {
        let src = [0.5f32, -1.0, 2.0, f32::NAN];
        let mut b = [0u8; 4 * 4];
        unsafe { write_frames(Sample::I16, b.as_mut_ptr(), 2, &src) };
        let v: Vec<i16> = b[..8].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(v, vec![16383, -32767, 32767, 0]);

        let mut b = [0u8; 12];
        unsafe { write_frames(Sample::I24, b.as_mut_ptr(), 2, &src) };
        let v: Vec<i32> = b.chunks(3).map(|c| i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8).collect();
        assert_eq!(v, vec![4_194_303, -8_388_607, 8_388_607, 0]);

        // Mono gets the average; a third channel gets silence.
        let mut b = [0u8; 8];
        unsafe { write_frames(Sample::F32, b.as_mut_ptr(), 1, &[0.5, 0.25, 1.0, 0.0]) };
        assert_eq!(f32::from_le_bytes(b[..4].try_into().unwrap()), 0.375);
        let mut b = [0xffu8; 12];
        unsafe { write_frames(Sample::F32, b.as_mut_ptr(), 3, &[0.5, 0.25]) };
        assert_eq!(f32::from_le_bytes(b[8..].try_into().unwrap()), 0.0);
    }

    #[test]
    fn format_round_trip() {
        for s in [Sample::F32, Sample::I32, Sample::I24, Sample::I16] {
            let f = make_format(96000, 2, s);
            let read = unsafe { read_format(&f as *const WaveFormatExtensible as *const WAVEFORMATEX) };
            assert_eq!(read, Some((s, 2, 96000)));
        }
    }

    #[test]
    fn hns_conversions() {
        assert_eq!(frames_to_hns(480, 48000), 100_000);
        assert_eq!(hns_to_frames(30_000, 48000), 144);
    }
}
