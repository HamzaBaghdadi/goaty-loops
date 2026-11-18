//! Autosave and crash recovery.
//!
//! Each running copy of the app is a *session* with a marker file in
//! `%APPDATA%\Goaty Loops\Autosave\<id>.json`. While the project has unsaved
//! changes it is autosaved next to it as `<id>.goaty` (the user's own file is
//! never touched). A clean exit removes both. A marker whose process is no
//! longer running therefore means the app crashed or was killed: on the next
//! start its autosave is offered for recovery, together with the crash report
//! written by [`crate::crash`], if any.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionInfo {
    pub pid: u32,
    /// Unix seconds.
    pub started: u64,
    pub project_name: String,
    /// Where the project was saved by the user, if anywhere.
    pub original_path: Option<PathBuf>,
    /// Unix seconds of the last autosave; `None` = nothing unsaved.
    pub autosaved: Option<u64>,
    /// The project had unsaved changes (possibly newer than the autosave).
    pub dirty: bool,
}

pub struct Session {
    pub id: String,
    dir: PathBuf,
    info: SessionInfo,
    last_autosave: Option<Instant>,
    /// Autosave at the next check regardless of the interval.
    due_now: bool,
    /// Background write of the last autosave.
    writer: Option<JoinHandle<()>>,
}

pub fn autosave_dir() -> PathBuf {
    goaty_engine::paths::app_data_dir().join("Autosave")
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Session {
    pub fn start() -> Self {
        Self::start_in(autosave_dir(), std::process::id())
    }

    pub fn start_in(dir: PathBuf, pid: u32) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let started = now_secs();
        let id = format!("{pid}-{started}");
        let s = Self { id, dir, info: SessionInfo { pid, started, ..Default::default() }, last_autosave: None, due_now: false, writer: None };
        s.write_info();
        s
    }

    fn info_path(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.id))
    }

    pub fn autosave_path(&self) -> PathBuf {
        self.dir.join(format!("{}.goaty", self.id))
    }

    fn write_info(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(&self.info) {
            let _ = write_atomic(&self.info_path(), &json);
        }
    }

    /// Time since the last autosave (or since the session started).
    pub fn since_autosave(&self) -> Duration {
        if self.due_now {
            return Duration::MAX;
        }
        self.last_autosave.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(now_secs().saturating_sub(self.info.started)))
    }

    /// Makes the next autosave check write immediately (e.g. after recovering).
    pub fn mark_due(&mut self) {
        self.due_now = true;
    }

    /// Records whether the project has unsaved changes (written only when it
    /// flips), so a crash notice can say whether recent work was lost.
    pub fn set_dirty(&mut self, dirty: bool) {
        if self.info.dirty != dirty {
            self.flush();
            self.info.dirty = dirty;
            self.write_info();
        }
    }

    /// Records which project is open (shown if recovery is offered later).
    pub fn set_project(&mut self, name: &str, original_path: Option<PathBuf>) {
        if self.info.project_name != name || self.info.original_path != original_path {
            self.flush();
            self.info.project_name = name.to_string();
            self.info.original_path = original_path;
            self.write_info();
        }
    }

    /// Writes `project_json` in the background (atomically), then the marker.
    pub fn autosave(&mut self, project_json: String, project_name: &str, original_path: Option<PathBuf>) {
        self.flush();
        self.last_autosave = Some(Instant::now());
        self.due_now = false;
        self.info.project_name = project_name.to_string();
        self.info.original_path = original_path;
        self.info.autosaved = Some(now_secs());
        let (path, info_path) = (self.autosave_path(), self.info_path());
        let info = serde_json::to_vec_pretty(&self.info).unwrap_or_default();
        self.writer = std::thread::Builder::new()
            .name("autosave".into())
            .spawn(move || {
                // Project first: a marker never points at a half-written file.
                if write_atomic(&path, project_json.as_bytes()).is_ok() {
                    let _ = write_atomic(&info_path, &info);
                }
            })
            .ok();
    }

    /// Nothing unsaved any more (saved, or the project was closed).
    pub fn clear_autosave(&mut self) {
        self.flush();
        self.last_autosave = Some(Instant::now());
        if self.info.autosaved.take().is_some() {
            let _ = std::fs::remove_file(self.autosave_path());
            self.write_info();
        }
    }

    /// Waits for a pending background write.
    pub fn flush(&mut self) {
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }

    /// Clean exit: the session leaves nothing to recover.
    pub fn end(&mut self) {
        self.flush();
        let _ = std::fs::remove_file(self.autosave_path());
        let _ = std::fs::remove_file(self.info_path());
    }
}

/// A session that ended without a clean exit.
#[derive(Clone, Debug)]
pub struct Recoverable {
    pub id: String,
    pub info: SessionInfo,
    /// The autosaved project, if there were unsaved changes.
    pub project: Option<PathBuf>,
    /// First lines of the crash report, if one was written.
    pub crash: Option<String>,
    pub crash_log: Option<PathBuf>,
    info_path: PathBuf,
}

impl Recoverable {
    /// Forgets this session (the crash report itself is kept).
    pub fn discard(&self) {
        if let Some(p) = &self.project {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_file(&self.info_path);
    }
}

pub fn find_recoverable() -> Vec<Recoverable> {
    find_recoverable_in(&autosave_dir(), &crate::crash::crash_dir(), is_app_running)
}

/// Sessions in `dir` whose process isn't running, newest first.
pub fn find_recoverable_in(dir: &Path, crash_dir: &Path, running: impl Fn(u32) -> bool) -> Vec<Recoverable> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        let Some(info) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<SessionInfo>(&b).ok()) else {
            let _ = std::fs::remove_file(&path); // unreadable marker
            continue;
        };
        if running(info.pid) && info.pid != 0 {
            continue;
        }
        let project = Some(dir.join(format!("{id}.goaty"))).filter(|p| p.is_file());
        let crash_log = Some(crash_dir.join(format!("{id}.txt"))).filter(|p| p.is_file());
        let crash = crash_log.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| s.lines().take(3).collect::<Vec<_>>().join("\n"));
        found.push(Recoverable { id, info, project, crash, crash_log, info_path: path });
    }
    found.sort_by(|a, b| b.info.started.cmp(&a.info.started));
    found
}

/// Set while the project has unsaved changes (read by the emergency save).
pub static UNSAVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Saves the version of the project the audio engine has (the latest edits)
/// into the session's autosave file. Called from the panic hook when the UI
/// thread panics; does nothing if there were no unsaved changes.
pub fn emergency_save(path: &Path, shared: &goaty_engine::Shared) {
    if !UNSAVED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let project = shared.project.load();
    if let Ok(json) = serde_json::to_vec(&**project) {
        let _ = write_atomic(path, &json);
    }
}

/// Writes through a temporary file and a rename, so a crash mid-write never
/// leaves a truncated file behind.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    goaty_engine::model::write_atomic(path, data)
}

/// Is `pid` a running copy of this app (not some other process that reused the id)?
#[cfg(windows)]
pub fn is_app_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let alive = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let named = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if !alive || !named {
            return alive;
        }
        let image = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
        let ours = std::env::current_exe().ok();
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
        ours.as_deref().map(name) == Some(name(&image))
    }
}

#[cfg(not(windows))]
pub fn is_app_running(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// "3 minutes ago" style age of a Unix time.
pub fn ago(secs: u64) -> String {
    let d = now_secs().saturating_sub(secs);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goaty_recovery_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn clean_exit_leaves_nothing() {
        let dir = temp_dir("clean");
        let mut s = Session::start_in(dir.clone(), 4242);
        s.autosave("{}".into(), "Song", None);
        s.flush();
        assert!(s.autosave_path().is_file());
        s.end();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn crashed_session_is_offered() {
        let (dir, crashes) = (temp_dir("crash"), temp_dir("crash_logs"));
        let mut s = Session::start_in(dir.clone(), 4243);
        s.autosave("{\"name\":\"x\"}".into(), "Beat 1", Some("C:/beats/beat1.goaty".into()));
        s.flush();
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(crashes.join(format!("{}.txt", s.id)), "Crash in Plugin.vst3\nmore\nmore\nignored").unwrap();
        // Process 4243 "died" without `end()`.
        let found = find_recoverable_in(&dir, &crashes, |_| false);
        assert_eq!(found.len(), 1);
        let r = &found[0];
        assert_eq!(r.info.project_name, "Beat 1");
        assert_eq!(r.info.original_path.as_deref(), Some(Path::new("C:/beats/beat1.goaty")));
        assert_eq!(std::fs::read_to_string(r.project.as_ref().unwrap()).unwrap(), "{\"name\":\"x\"}");
        assert_eq!(r.crash.as_deref(), Some("Crash in Plugin.vst3\nmore\nmore"));
        // A still-running session is left alone.
        assert!(find_recoverable_in(&dir, &crashes, |pid| pid == 4243).is_empty());
        r.discard();
        assert!(find_recoverable_in(&dir, &crashes, |_| false).is_empty());
    }

    #[test]
    fn saving_clears_the_autosave() {
        let dir = temp_dir("saved");
        let mut s = Session::start_in(dir.clone(), 4244);
        s.autosave("{}".into(), "Song", None);
        s.clear_autosave();
        assert!(!s.autosave_path().exists());
        // A crash now has nothing unsaved to offer, only the notice.
        let found = find_recoverable_in(&dir, &dir, |_| false);
        assert_eq!(found.len(), 1);
        assert!(found[0].project.is_none());
        assert_eq!(found[0].info.autosaved, None);
    }

    #[test]
    fn this_process_is_running() {
        assert!(is_app_running(std::process::id()));
        assert!(!is_app_running(0xFFFF_FFF0));
    }
}
