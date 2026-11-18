//! In-app updates from GitHub Releases.
//!
//! Checks `releases/latest` of [`REPO`], downloads the release zip (or the
//! bare `goaty-loops.exe` asset) next to the running exe, verifies its SHA-256
//! against the digest GitHub publishes, unpacks it with Windows' `tar.exe`,
//! then swaps it in: a running exe can't be overwritten on
//! Windows but it can be renamed, so the old one becomes `goaty-loops.exe.old`
//! (deleted at the next start) and the new one takes its name. Restarting
//! runs the new version.
//!
//! HTTP goes through `curl.exe` and unpacking through `tar.exe`, both shipped
//! with Windows 10 and 11, so the app carries no TLS or zip code of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

/// GitHub repository releases are published to.
pub const REPO: &str = "HamzaBaghdadi/goaty-loops";
/// Release asset holding the app.
const ASSET: &str = "goaty-loops.exe";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    /// e.g. "0.2.0" (no leading v).
    pub version: String,
    pub notes: String,
    pub page: String,
    asset_url: String,
    sha256: Option<String>,
    /// The asset is the release zip (smaller) rather than the bare exe.
    zipped: bool,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    /// Installed; takes effect after a restart.
    Ready(Release),
    Failed(String),
}

enum Msg {
    Checked(Result<Option<Release>, String>),
    Installed(Result<Release, String>),
}

pub struct Updater {
    pub state: State,
    /// Show the window (always after a manual check, or when an update is found).
    pub open: bool,
    manual: bool,
    rx: Option<Receiver<Msg>>,
}

impl Updater {
    pub fn new() -> Self {
        Self { state: State::Idle, open: false, manual: false, rx: None }
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading(_))
    }

    /// Looks for a newer release in the background. `manual` shows the result
    /// even when there is nothing new.
    pub fn check(&mut self, manual: bool) {
        if self.busy() {
            return;
        }
        self.manual = manual;
        self.open |= manual;
        self.state = State::Checking;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(latest()));
        });
    }

    /// Downloads and installs the available release in the background.
    pub fn install(&mut self) {
        let State::Available(r) = self.state.clone() else { return };
        self.state = State::Downloading(r.clone());
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Installed(download_and_swap(&r).map(|_| r)));
        });
    }

    /// Picks up background results. Returns true when the state changed.
    pub fn poll(&mut self) -> bool {
        let Some(msg) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
        self.rx = None;
        self.state = match msg {
            Msg::Checked(Ok(Some(r))) => {
                self.open = true;
                State::Available(r)
            }
            Msg::Checked(Ok(None)) => State::UpToDate,
            Msg::Checked(Err(e)) => State::Failed(e),
            Msg::Installed(Ok(r)) => State::Ready(r),
            Msg::Installed(Err(e)) => State::Failed(e),
        };
        // Quiet startup checks don't pop up "up to date" or network errors.
        if !self.manual && matches!(self.state, State::UpToDate | State::Failed(_)) {
            self.open = false;
        }
        true
    }
}

/// Deletes the exe left over from the last update.
pub fn cleanup_previous() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
        let _ = std::fs::remove_file(new_path(&exe));
        let _ = std::fs::remove_file(exe.with_extension("update.zip"));
        let _ = std::fs::remove_dir_all(exe.with_extension("update"));
    }
}

/// Starts the (updated) exe; call on exit.
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

fn new_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.new")
}

/// The latest release, if newer than this build.
fn latest() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = curl(&["-H", "Accept: application/vnd.github+json", &url])?;
    let v: serde_json::Value = serde_json::from_slice(&out).map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("No release found")?;
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, CURRENT) {
        return Ok(None);
    }
    // The zip is about a third of the exe's size; fall back to the bare exe.
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let zip_name = format!("GoatyLoops-{tag}-windows-x64.zip");
    let (asset, zipped) = match assets.iter().find(|x| x["name"].as_str() == Some(zip_name.as_str())) {
        Some(a) => (a, true),
        None => (assets.iter().find(|x| x["name"].as_str() == Some(ASSET)).ok_or_else(|| format!("Release {tag} has no {ASSET}"))?, false),
    };
    Ok(Some(Release {
        version,
        notes: plain_text(v["body"].as_str().unwrap_or("")),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        asset_url: asset["browser_download_url"].as_str().ok_or("Missing download link")?.to_string(),
        sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_ascii_lowercase),
        zipped,
    }))
}

fn download_and_swap(r: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = new_path(&exe);
    let download = if r.zipped { exe.with_extension("update.zip") } else { new.clone() };
    let download_s = download.to_string_lossy().to_string();
    curl(&["--max-time", "1800", "-o", &download_s, &r.asset_url]).map_err(|e| {
        let _ = std::fs::remove_file(&download);
        if e.contains("Failed to open") || e.contains("denied") {
            format!("Can't write to {}. Download the update from the release page instead.", exe.parent().map(|p| p.display().to_string()).unwrap_or_default())
        } else {
            e
        }
    })?;
    let downloaded = std::fs::read(&download).map_err(|e| e.to_string())?;
    if let Some(want) = &r.sha256 {
        use sha2::Digest;
        let got: String = sha2::Sha256::digest(&downloaded).iter().map(|b| format!("{b:02x}")).collect();
        if &got != want {
            let _ = std::fs::remove_file(&download);
            return Err("The download is damaged (checksum mismatch). Try again later.".into());
        }
    }
    if r.zipped {
        let dir = exe.with_extension("update");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let unpacked = system_tool("tar.exe")
            .args(["-xf", &download_s, "-C", &dir.to_string_lossy(), ASSET])
            .output()
            .map_err(|e| format!("Can't unpack the update: {e}"))?;
        let _ = std::fs::remove_file(&download);
        if !unpacked.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("Can't unpack the update: {}", String::from_utf8_lossy(&unpacked.stderr).trim()));
        }
        let moved = std::fs::rename(dir.join(ASSET), &new);
        let _ = std::fs::remove_dir_all(&dir);
        moved.map_err(|e| format!("Can't unpack the update: {e}"))?;
    }
    let data = std::fs::read(&new).map_err(|e| e.to_string())?;
    if data.len() < 1_000_000 || !data.starts_with(b"MZ") {
        let _ = std::fs::remove_file(&new);
        return Err("The download isn't a valid program.".into());
    }
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("Can't replace the running app: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe); // put the current version back
        return Err(format!("Can't install the update: {e}"));
    }
    Ok(())
}

/// Runs curl.exe (no console window) and returns its output.
fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut cmd = system_tool("curl.exe");
    cmd.args(["-sSfL", "--max-time", "30", "-H", "User-Agent: goaty-loops-updater"]).args(args);
    let out = cmd.output().map_err(|e| format!("Can't run curl.exe: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { "Network error".into() } else { err })
    }
}

/// A tool shipped with Windows (System32, not whatever is first on PATH),
/// run without a console window.
fn system_tool(name: &str) -> std::process::Command {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:Windows"));
    let path = root.join("System32").join(name);
    let mut cmd = std::process::Command::new(if path.is_file() { path } else { PathBuf::from(name) });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Release notes without Markdown markup, for the plain-text update window.
fn plain_text(md: &str) -> String {
    md.lines()
        .map(|l| l.trim_start_matches('#').trim_start().replace("**", "").replace('`', ""))
        .collect::<Vec<_>>()
        .join("
")
}

/// Compares dotted versions numerically ("0.10.0" > "0.9.3"); pre-release
/// suffixes ("-beta") are ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('-').next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (mut a, mut b) = (parse(candidate), parse(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn notes_are_plain() {
        assert_eq!(super::plain_text("### Highlights
- **Bold** and `code`"), "Highlights
- Bold and code");
    }

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
    }
}
