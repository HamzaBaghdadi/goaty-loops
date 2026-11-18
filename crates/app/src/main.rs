#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crash;
mod midi;
mod plugins;
mod recovery;
mod settings;
mod theme;
mod update;
mod views;
mod widgets;

fn main() -> eframe::Result<()> {
    // Child-process plugin scan: keeps crashing plugins out of the main app.
    {
        let mut it = std::env::args_os().skip(1);
        if it.next().is_some_and(|a| a == "--scan-plugin") {
            let path = std::path::PathBuf::from(it.next().unwrap_or_default());
            match goaty_engine::plugin::scan_module(&path) {
                Ok(list) => println!("BCSCAN:{}", serde_json::to_string(&list).unwrap_or_default()),
                Err(e) => println!("BCERR:{e}"),
            }
            use std::io::Write;
            let _ = std::io::stdout().flush();
            // Exit without running plugin DLL teardown, which some plugins crash in.
            std::process::exit(0);
        }
    }
    if std::env::args_os().nth(1).is_some_and(|a| a == "--list-midi") {
        let ports = midi::Midi::ports();
        println!("{} MIDI input(s)", ports.len());
        for p in ports {
            println!("  {p}");
        }
        return Ok(());
    }
    // Remove the exe replaced by the last in-app update.
    update::cleanup_previous();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let show_mixer = args.iter().any(|a| a == "--mixer");
    let file = args.into_iter().find(|a| a != "--mixer").map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Goaty Loops")
            .with_icon(std::sync::Arc::new(eframe::icon_data::from_png_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/icon_256.png"))).expect("app icon")))
            .with_min_inner_size([900.0, 560.0])
            .with_maximized(true)
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Goaty Loops",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut app = app::App::new(file);
            app.show_mixer = show_mixer;
            Ok(Box::new(app))
        }),
    )
}
