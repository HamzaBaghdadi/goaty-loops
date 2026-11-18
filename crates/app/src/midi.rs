//! MIDI keyboard input: live playing and recording into patterns.
//!
//! Notes go from the MIDI callback straight to the audio thread (no UI-frame
//! latency); a copy goes to the UI thread for recording.

use goaty_engine::{Clip, ClipSource, Command, Shared};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};

pub enum MidiMsg {
    On { key: u8, velocity: f32, tick: f64 },
    Off { key: u8, tick: f64 },
    /// Control change; `channel` is the MIDI channel (0..=15).
    Cc { channel: u8, cc: u8, value: u8 },
}

pub struct Midi {
    conn: Option<midir::MidiInputConnection<()>>,
    pub port_name: Option<String>,
    pub error: Option<String>,
    /// Channel that receives MIDI notes (the selected channel).
    pub target: Arc<AtomicUsize>,
    rx: Option<Receiver<MidiMsg>>,
    /// The UI context, woken when MIDI arrives (the UI doesn't redraw while idle).
    pub wake: Arc<OnceLock<egui::Context>>,
}

impl Midi {
    pub fn new() -> Self {
        Self { conn: None, port_name: None, error: None, target: Arc::new(AtomicUsize::new(0)), rx: None, wake: Arc::new(OnceLock::new()) }
    }

    /// Names of the available MIDI inputs.
    pub fn ports() -> Vec<String> {
        let Ok(input) = midir::MidiInput::new("Goaty Loops") else { return Vec::new() };
        input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.conn.take() {
            c.close();
        }
        self.port_name = None;
        self.rx = None;
    }

    /// Connects to the named input (or the first one when `None`).
    pub fn connect(&mut self, name: Option<&str>, engine_tx: Sender<Command>, shared: Arc<Shared>) {
        self.disconnect();
        let mut input = match midir::MidiInput::new("Goaty Loops") {
            Ok(i) => i,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        input.ignore(midir::Ignore::All);
        let ports = input.ports();
        let port = match name {
            Some(n) => ports.iter().find(|p| input.port_name(p).ok().as_deref() == Some(n)),
            None => ports.first(),
        };
        let Some(port) = port.cloned() else {
            self.error = Some("No MIDI input devices found".into());
            return;
        };
        let port_name = input.port_name(&port).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let target = self.target.clone();
        let wake = self.wake.clone();
        let conn = input.connect(
            &port,
            "goaty-loops-in",
            move |_ts, msg, _| {
                let (status, key, vel) = match msg {
                    [s, k, v, ..] => (s & 0xf0, *k & 0x7f, *v & 0x7f),
                    _ => return,
                };
                let midi_channel = msg[0] & 0x0f;
                let channel = target.load(Ordering::Relaxed);
                let tick = shared.transport.position();
                match (status, vel) {
                    (0x90, v) if v > 0 => {
                        let velocity = v as f32 / 127.0;
                        let _ = engine_tx.try_send(Command::NoteOn { channel, key, velocity });
                        let _ = tx.send(MidiMsg::On { key, velocity, tick });
                    }
                    (0x80, _) | (0x90, 0) => {
                        let _ = engine_tx.try_send(Command::NoteOff { channel, key });
                        let _ = tx.send(MidiMsg::Off { key, tick });
                    }
                    (0xB0, value) => {
                        let _ = tx.send(MidiMsg::Cc { channel: midi_channel, cc: key, value });
                    }
                    (0xE0, msb) => {
                        // 14-bit wheel, centre 8192, scaled by the channel's pitch range.
                        let wheel = ((msb as i32) << 7 | key as i32) - 8192;
                        let range = shared.project.load().channels.get(channel).map(|c| c.pitch_range).unwrap_or(2.0);
                        let semis = wheel as f32 / 8192.0 * range;
                        let raw = (wheel + 8192) as f32 / 16383.0;
                        let _ = engine_tx.try_send(Command::PitchBend { channel, semis, wheel: raw });
                    }
                    _ => {}
                }
                if let Some(ctx) = wake.get() {
                    ctx.request_repaint();
                }
            },
            (),
        );
        match conn {
            Ok(c) => {
                self.conn = Some(c);
                self.port_name = Some(port_name);
                self.rx = Some(rx);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Messages received since the last call.
    pub fn drain(&self) -> Vec<MidiMsg> {
        self.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }
}

/// Start and length (ticks) of a note recorded from `on` to `off` while a
/// pattern of `pat_len` ticks loops.
pub fn recorded_span(on: f64, off: f64, pat_len: f64) -> (u32, u32) {
    let mut length = off - on;
    if length < 0.0 {
        length += pat_len; // the pattern looped while the key was held
    }
    (on.rem_euclid(pat_len) as u32, (length.round() as u32).max(1))
}

/// Song-mode recording: index of the clip of `pattern` that holds a note
/// played from song tick `tick` to `end`. Uses the clip under the playhead,
/// else keeps growing the take's clip (`last`), else adds one at the bar on
/// the first free track. The clip is lengthened (in whole bars) to fit.
pub fn song_record_clip(clips: &mut Vec<Clip>, pattern: usize, tick: u32, end: u32, bar: u32, last: Option<usize>) -> usize {
    let bar = bar.max(1);
    let src = ClipSource::Pattern(pattern);
    let i = match clips.iter().position(|c| c.source == src && c.start <= tick && tick < c.end()) {
        Some(i) => i,
        None => match last.filter(|&i| clips.get(i).is_some_and(|c| c.source == src && c.start <= tick)) {
            Some(i) => i,
            None => {
                let start = tick / bar * bar;
                let track = (0..).find(|&t| !clips.iter().any(|c| c.track == t && c.start < start + bar && c.end() > start)).unwrap_or(0);
                clips.push(Clip { track, start, length: bar, offset: 0, source: src, muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
                clips.len() - 1
            }
        },
    };
    let c = &mut clips[i];
    if end > c.end() {
        c.length = (end - c.start).div_ceil(bar) * bar;
    }
    i
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans() {
        assert_eq!(recorded_span(96.0, 192.0, 384.0), (96, 96));
        // Held across the loop point: starts near the end, ends after the wrap.
        assert_eq!(recorded_span(360.0, 24.0, 384.0), (360, 48));
        // Very short taps still produce a note.
        assert_eq!(recorded_span(10.0, 10.0, 384.0), (10, 1));
    }

    #[test]
    fn song_recording_places_clips() {
        let bar = 384;
        let other = Clip { track: 0, start: 0, length: 2 * bar, offset: 0, source: ClipSource::Pattern(1), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 };
        let mut clips = vec![other];
        // No clip of pattern 0: a new one at the bar, on the first free track.
        let i = song_record_clip(&mut clips, 0, bar + 10, bar + 50, bar, None);
        assert_eq!((clips[i].track, clips[i].start, clips[i].length), (1, bar, bar));
        // Playing past its end grows the same clip.
        let j = song_record_clip(&mut clips, 0, 2 * bar + 5, 3 * bar + 1, bar, Some(i));
        assert_eq!(i, j);
        assert_eq!(clips[i].length, 3 * bar);
        assert_eq!(clips.len(), 2);
    }
}
