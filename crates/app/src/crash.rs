//! Crash reports: `%APPDATA%\Goaty Loops\Crashes\<session id>.txt`.
//!
//! - Rust panics: message, location and backtrace. A panic on the UI thread
//!   ends the app, so it also triggers an emergency save of the project.
//! - Native crashes (e.g. an access violation inside a plugin): exception and
//!   the module it happened in, so the next start can say "crash in X.vst3".
//!   That handler runs in a broken process, so it uses only stack buffers and
//!   raw Win32 calls (no allocation, no locks).
//!
//! [`crate::recovery`] shows the first lines at the next start.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static REPORT: OnceLock<PathBuf> = OnceLock::new();
static EMERGENCY_SAVE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
#[cfg(windows)]
static REPORT_WIDE: OnceLock<Vec<u16>> = OnceLock::new();

/// Crash reports kept; older ones are deleted at startup.
const KEEP_REPORTS: usize = 20;

pub fn crash_dir() -> PathBuf {
    goaty_engine::paths::app_data_dir().join("Crashes")
}

/// Installs the panic hook and the native crash handler for this session.
pub fn install(session_id: &str) {
    let dir = crash_dir();
    let _ = std::fs::create_dir_all(&dir);
    prune(&dir);
    let path = dir.join(format!("{session_id}.txt"));
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let _ = REPORT_WIDE.set(path.as_os_str().encode_wide().chain([0]).collect());
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(native_crash)) };
    }
    let _ = REPORT.set(path);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown cause".into());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let text = format!(
            "Goaty Loops panicked in thread '{name}': {msg}\nat {at}\nversion {}\n\n{}\n\n",
            env!("CARGO_PKG_VERSION"),
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(p) = REPORT.get() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        // A panic on the UI thread takes the app down: save what we can.
        if name == "main" {
            if let Some(save) = EMERGENCY_SAVE.get() {
                save();
            }
        }
        default_hook(info);
    }));
}

/// What to run when the UI thread panics (saves the latest project version).
pub fn set_emergency_save(f: impl Fn() + Send + Sync + 'static) {
    let _ = EMERGENCY_SAVE.set(Box::new(f));
}

/// Deletes all but the newest reports.
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(KEEP_REPORTS) {
        let _ = std::fs::remove_file(p);
    }
}

/// A fixed-size text buffer on the stack (the native handler can't allocate).
struct StackText<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }
    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> std::fmt::Write for StackText<N> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let mut n = s.len().min(N - self.len);
        while !s.is_char_boundary(n) {
            n -= 1; // never cut a character in half
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "access violation",
        0xC000_00FD => "stack overflow",
        0xC000_0094 => "integer division by zero",
        0xC000_001D => "illegal instruction",
        0xC000_0096 => "privileged instruction",
        0xC000_0374 => "heap corruption",
        0xE06D_7363 => "unhandled C++ exception",
        0x8000_0003 => "breakpoint",
        _ => "unhandled exception",
    }
}

/// Writes "<summary>\n<details>" for a native crash into `out`. `module` is
/// the full path of the module the crash happened in, as UTF-16.
fn describe_native(out: &mut impl std::fmt::Write, code: u32, address: usize, module: &[u16], info: &[usize]) {
    let mut name = StackText::<512>::new();
    let mut path = StackText::<1024>::new();
    let file_start = module.iter().rposition(|&c| c == b'\\' as u16 || c == b'/' as u16).map_or(0, |i| i + 1);
    for c in char::decode_utf16(module[file_start..].iter().copied()) {
        let _ = std::fmt::Write::write_char(&mut name, c.unwrap_or('?'));
    }
    for c in char::decode_utf16(module.iter().copied()) {
        let _ = std::fmt::Write::write_char(&mut path, c.unwrap_or('?'));
    }
    let name = std::str::from_utf8(name.bytes()).unwrap_or("?");
    let path = std::str::from_utf8(path.bytes()).unwrap_or("?");
    let module_name = if name.is_empty() { "an unknown module" } else { name };
    let _ = writeln!(out, "Crash in {module_name}: {} (0x{code:08x})", exception_name(code));
    let _ = writeln!(out, "module: {path}");
    let _ = write!(out, "address: 0x{address:x}");
    if code == 0xC000_0005 && info.len() >= 2 {
        let what = match info[0] {
            0 => "reading",
            1 => "writing",
            8 => "executing",
            _ => "accessing",
        };
        let _ = write!(out, ", {what} 0x{:x}", info[1]);
    }
    let _ = writeln!(out, "\nversion {}\n", env!("CARGO_PKG_VERSION"));
}

#[cfg(windows)]
unsafe extern "system" fn native_crash(ep: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, WriteFile, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_ALWAYS};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    let Some(path) = REPORT_WIDE.get() else { return EXCEPTION_CONTINUE_SEARCH };
    if ep.is_null() || (*ep).ExceptionRecord.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let rec = &*(*ep).ExceptionRecord;
    let address = rec.ExceptionAddress as usize;
    let mut module = [0u16; 520];
    let mut module_len = 0usize;
    let mut hmod = std::ptr::null_mut();
    if GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, address as *const u16, &mut hmod) != 0 {
        module_len = GetModuleFileNameW(hmod, module.as_mut_ptr(), module.len() as u32) as usize;
    }
    let params = (rec.NumberParameters as usize).min(rec.ExceptionInformation.len());
    let mut text = StackText::<2048>::new();
    describe_native(&mut text, rec.ExceptionCode as u32, address, &module[..module_len], &rec.ExceptionInformation[..params]);

    let file = CreateFileW(path.as_ptr(), FILE_APPEND_DATA, FILE_SHARE_READ, std::ptr::null(), OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut());
    if file != INVALID_HANDLE_VALUE {
        let mut written = 0u32;
        WriteFile(file, text.bytes().as_ptr(), text.len as u32, &mut written, std::ptr::null_mut());
        CloseHandle(file);
    }
    // Let Windows end the process as usual.
    EXCEPTION_CONTINUE_SEARCH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_report_names_the_module() {
        let module: Vec<u16> = r"C:\Program Files\Common Files\VST3\Auto-Tune Pro.vst3\Contents\x86_64-win\Auto-Tune Pro.vst3".encode_utf16().collect();
        let mut out = String::new();
        describe_native(&mut out, 0xC000_0005, 0x7ff6_1234, &module, &[1, 0x10]);
        let first = out.lines().next().unwrap();
        assert_eq!(first, "Crash in Auto-Tune Pro.vst3: access violation (0xc0000005)");
        assert!(out.contains("writing 0x10"));
        let mut out = String::new();
        describe_native(&mut out, 0xC000_00FD, 0, &[], &[]);
        assert!(out.starts_with("Crash in an unknown module: stack overflow"));
    }

    #[test]
    fn stack_text_truncates() {
        let mut t = StackText::<4>::new();
        let _ = std::fmt::Write::write_str(&mut t, "abcdef");
        assert_eq!(t.bytes(), b"abcd");
    }
}
