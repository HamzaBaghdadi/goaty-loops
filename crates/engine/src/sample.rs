use anyhow::{anyhow, Context, Result};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "ogg", "flac", "aif", "aiff", "m4a"];

/// Decoded audio, always stereo interleaved.
#[derive(Debug)]
pub struct SampleData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Peak per block of [`PEAK_BLOCK`] frames, for waveform drawing.
    pub peaks: Vec<f32>,
}

pub const PEAK_BLOCK: usize = 256;

impl SampleData {
    pub fn from_interleaved(mut data: Vec<f32>, channels: usize, sample_rate: u32) -> Self {
        let samples = match channels {
            2 => data,
            1 => data.iter().flat_map(|&s| [s, s]).collect(),
            n if n > 2 => data.chunks_exact(n).flat_map(|f| [f[0], f[1]]).collect(),
            _ => {
                data.clear();
                data
            }
        };
        let peaks = samples
            .chunks(PEAK_BLOCK * 2)
            .map(|c| c.iter().fold(0f32, |m, s| m.max(s.abs())))
            .collect();
        Self { samples, sample_rate, peaks }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1) as f64
    }
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn load_audio(path: &Path) -> Result<SampleData> {
    if let Some(ogg) = wav_wrapped_ogg(path) {
        let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(ogg)), Default::default());
        return decode(mss, Some("ogg"));
    }
    match load_with_symphonia(path) {
        Ok(d) => Ok(d),
        // hound handles some WAV variants (e.g. odd chunk layouts) symphonia rejects.
        Err(e) => load_with_hound(path).map_err(|_| e),
    }
}

fn load_with_hound(path: &Path) -> Result<SampleData> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()?
        }
    };
    Ok(SampleData::from_interleaved(data, spec.channels as usize, spec.sample_rate))
}

/// FL Studio ships many factory samples as Ogg Vorbis streams inside a WAV
/// container (format tags 0x674F-0x6771). Returns the embedded Ogg stream.
fn wav_wrapped_ogg(path: &Path) -> Option<Vec<u8>> {
    let is_wav = path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("wav")).unwrap_or(false);
    if !is_wav {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut ogg = false;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        match id {
            b"fmt " => {
                let tag = u16::from_le_bytes(bytes.get(body..body + 2)?.try_into().ok()?);
                ogg = matches!(tag, 0x674F | 0x6750 | 0x6751 | 0x676F | 0x6770 | 0x6771);
                if !ogg {
                    return None;
                }
            }
            b"data" if ogg => {
                let end = (body + len).min(bytes.len());
                return Some(bytes[body..end].to_vec());
            }
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    None
}

fn load_with_symphonia(path: &Path) -> Result<SampleData> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    decode(mss, path.extension().and_then(|e| e.to_str()))
}

fn decode(mss: MediaSourceStream, ext: Option<&str>) -> Result<SampleData> {
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("unsupported audio format")?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow!("no audio track"))?;
    let track_id = track.id;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut out = Vec::new();
    let mut channels = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                channels = spec.channels.count();
                sample_rate = spec.rate;
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                out.extend_from_slice(sb.samples());
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if channels == 0 {
        return Err(anyhow!("file contains no audio"));
    }
    Ok(SampleData::from_interleaved(out, channels, sample_rate))
}
