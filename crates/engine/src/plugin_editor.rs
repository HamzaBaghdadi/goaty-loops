//! Native windows hosting VST3 plugin editors (Windows only).
//!
//! Windows are created on the UI thread; their messages are dispatched by the
//! application's existing event loop.

#![allow(non_snake_case)]

use crate::plugin::PluginInstance;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: &[u16] = &[66, 101, 97, 116, 67, 114, 97, 102, 116, 80, 108, 117, 103, 105, 110, 0]; // "Goaty LoopsPlugin"

struct WinState {
    view: usize,
    closed: Arc<AtomicBool>,
}

fn windows() -> &'static Mutex<HashMap<isize, WinState>> {
    static W: OnceLock<Mutex<HashMap<isize, WinState>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

const STYLE: u32 = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            if let Some(s) = windows().lock().unwrap().get(&(hwnd as isize)) {
                s.closed.store(true, Ordering::Relaxed);
            }
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_SIZE => {
            let view = windows().lock().unwrap().get(&(hwnd as isize)).map(|s| s.view);
            if let Some(v) = view.filter(|v| *v != 0) {
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let mut vr = ViewRect { left: 0, top: 0, right: rc.right - rc.left, bottom: rc.bottom - rc.top };
                let view = v as *mut IPlugView;
                ((*(*view).vtbl).onSize)(view, &mut vr);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_class() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: CLASS_NAME.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
    });
}

/// Outer window size for a client area of `w` x `h`.
unsafe fn outer_size(w: i32, h: i32, style: u32) -> (i32, i32) {
    let mut rc = RECT { left: 0, top: 0, right: w, bottom: h };
    AdjustWindowRectEx(&mut rc, style, 0, 0);
    (rc.right - rc.left, rc.bottom - rc.top)
}

struct PlugFrame {
    hwnd: isize,
}

impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        if new_size.is_null() {
            return kInvalidArgument;
        }
        let r = *new_size;
        let hwnd = self.hwnd as HWND;
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let (w, h) = outer_size(r.right - r.left, r.bottom - r.top, style);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        if !view.is_null() {
            let mut vr = r;
            ((*(*view).vtbl).onSize)(view, &mut vr);
        }
        kResultOk
    }
}

pub struct PluginEditor {
    hwnd: isize,
    view: ComPtr<IPlugView>,
    _frame: ComWrapper<PlugFrame>,
    closed: Arc<AtomicBool>,
    pub instance: Arc<PluginInstance>,
}

impl PluginEditor {
    /// Opens the plugin's editor in a new top-level window owned by `owner`.
    pub fn open(instance: Arc<PluginInstance>, title: &str, owner: Option<isize>) -> Result<Self> {
        let view = instance.create_view().ok_or_else(|| anyhow!("{} has no editor", instance.info.name))?;
        unsafe {
            if view.isPlatformTypeSupported(kPlatformTypeHWND) != kResultOk {
                bail!("{} does not support Windows editors", instance.info.name);
            }
            register_class();
            let mut rect: ViewRect = std::mem::zeroed();
            view.getSize(&mut rect);
            let (cw, ch) = ((rect.right - rect.left).max(200), (rect.bottom - rect.top).max(100));
            let resizable = view.canResize() == kResultOk;
            let style = STYLE | if resizable { WS_THICKFRAME | WS_MAXIMIZEBOX } else { 0 };
            let (w, h) = outer_size(cw, ch, style);
            let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            let hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME,
                CLASS_NAME.as_ptr(),
                title_w.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                owner.map(|o| o as HWND).unwrap_or(std::ptr::null_mut()),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                bail!("could not create editor window");
            }
            let closed = Arc::new(AtomicBool::new(false));
            windows().lock().unwrap().insert(hwnd as isize, WinState { view: 0, closed: closed.clone() });

            let frame = ComWrapper::new(PlugFrame { hwnd: hwnd as isize });
            view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr());
            if view.attached(hwnd as *mut std::ffi::c_void, kPlatformTypeHWND) != kResultOk {
                windows().lock().unwrap().remove(&(hwnd as isize));
                DestroyWindow(hwnd);
                bail!("{} could not attach its editor", instance.info.name);
            }
            if let Some(s) = windows().lock().unwrap().get_mut(&(hwnd as isize)) {
                s.view = view.as_ptr() as usize;
            }
            ShowWindow(hwnd, SW_SHOW);
            Ok(Self { hwnd: hwnd as isize, view, _frame: frame, closed, instance })
        }
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// False once the user closed the window.
    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Relaxed) && unsafe { IsWindow(self.hwnd as HWND) != 0 }
    }

    pub fn focus(&self) {
        unsafe {
            ShowWindow(self.hwnd as HWND, SW_SHOW);
            SetForegroundWindow(self.hwnd as HWND);
        }
        self.closed.store(false, Ordering::Relaxed);
    }
}

impl Drop for PluginEditor {
    fn drop(&mut self) {
        unsafe {
            if let Some(s) = windows().lock().unwrap().get_mut(&self.hwnd) {
                s.view = 0;
            }
            self.view.removed();
            self.view.setFrame(std::ptr::null_mut());
            DestroyWindow(self.hwnd as HWND);
            windows().lock().unwrap().remove(&self.hwnd);
        }
    }
}
