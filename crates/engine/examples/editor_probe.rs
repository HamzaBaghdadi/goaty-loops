//! Opens a plugin editor for a few seconds and prints its window handle.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use goaty_engine::plugin::{scan_module, PluginInstance};
    use goaty_engine::plugin_editor::PluginEditor;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let path = std::env::args().nth(1).expect("plugin path");
    let info = scan_module(path.as_ref())?.remove(0);
    let inst = PluginInstance::new(&info, 48000.0)?;
    let ed = PluginEditor::open(inst, &info.name, None)?;
    println!("HWND {}", ed.hwnd());
    let start = std::time::Instant::now();
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while start.elapsed().as_secs() < 8 && ed.is_open() {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    drop(ed);
    println!("closed cleanly");
    Ok(())
}
#[cfg(not(windows))]
fn main() {}
