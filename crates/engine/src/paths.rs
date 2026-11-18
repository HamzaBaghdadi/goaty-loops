//! Where Goaty Loops keeps its files.

use std::path::PathBuf;

/// Folder name used before the app was renamed to Goaty Loops (it was BeatCraft).
const LEGACY_NAME: &str = "BeatCraft";
const NAME: &str = "Goaty Loops";

fn appdata_base() -> PathBuf {
    std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Settings and caches: `%APPDATA%\Goaty Loops`. A folder left by an older
/// (BeatCraft) version is moved here the first time.
pub fn app_data_dir() -> PathBuf {
    let base = appdata_base();
    let dir = base.join(NAME);
    let legacy = base.join(LEGACY_NAME);
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &dir);
    }
    dir
}

/// Where audio recordings are saved: `Documents\Goaty Loops\Recorded`.
pub fn recordings_dir() -> PathBuf {
    home().join("Documents").join(NAME).join("Recorded")
}

/// Recordings made by an older (BeatCraft) version, which projects may still
/// reference, so they are left where they are.
pub fn legacy_recordings_dir() -> PathBuf {
    home().join("Documents").join(LEGACY_NAME).join("Recorded")
}
