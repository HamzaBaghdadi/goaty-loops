//! User preferences that persist between sessions (%APPDATA%/Goaty Loops/settings.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// MIDI input port name; `None` = first available.
    pub midi_input: Option<String>,
    /// MIDI input turned off by the user.
    pub midi_off: bool,
    /// Audio input device name; `None` = the system default.
    pub audio_input: Option<String>,
    /// Recorded audio is shifted this much earlier to make up for
    /// input + output latency.
    pub rec_latency_ms: f32,
    /// Hear the audio input through the output while recording.
    pub monitor_input: bool,
    /// Minutes between autosaves of unsaved changes; `None` = default, 0 = off.
    pub autosave_minutes: Option<f32>,
    /// Look for a new release at startup; `None` = yes.
    pub check_updates: Option<bool>,
    /// Audio output: driver, device, sample rate, buffer size.
    pub audio: goaty_engine::AudioConfig,
}

/// Autosave interval when the user hasn't chosen one.
pub const DEFAULT_AUTOSAVE_MINUTES: f32 = 2.0;

fn path() -> PathBuf {
    goaty_engine::paths::app_data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        // Tolerate a UTF-8 byte-order mark (some editors add one).
        const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(b.strip_prefix(BOM).unwrap_or(&b)).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }
}
