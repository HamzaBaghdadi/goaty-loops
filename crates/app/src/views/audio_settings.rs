//! OPTIONS > Audio settings: output driver, device, sample rate and buffer size.

use crate::app::App;
use crate::theme;
use crate::widgets;
use goaty_engine::audio::{driver_caps, output_devices, AudioConfig, Driver, DriverCaps};

/// Window state: the edited (not yet applied) settings and what the chosen
/// device offers, queried only when the driver or device changes.
#[derive(Default)]
pub struct AudioSettingsUi {
    pub open: bool,
    draft: Option<AudioConfig>,
    devices: Option<(Driver, Vec<String>)>,
    caps: Option<((Driver, Option<String>, Option<u32>), Result<DriverCaps, String>)>,
}

impl AudioSettingsUi {
    pub fn show(&mut self) {
        self.open = true;
        self.draft = None; // start from what is applied
        self.devices = None;
        self.caps = None;
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.audio_ui.open {
        return;
    }
    let mut open = true;
    let applied = app.settings.audio.clone();
    let mut draft = app.audio_ui.draft.take().unwrap_or_else(|| applied.clone());
    let mut refresh = false;

    // Device list and capabilities for the draft (blocking queries, so cached).
    if app.audio_ui.devices.as_ref().is_none_or(|(d, _)| *d != draft.driver) {
        app.audio_ui.devices = Some((draft.driver, output_devices(draft.driver)));
    }
    let key = (draft.driver, draft.device.clone(), draft.sample_rate);
    if app.audio_ui.caps.as_ref().is_none_or(|(k, _)| *k != key) {
        let caps = driver_caps(draft.driver, draft.device.as_deref(), draft.sample_rate);
        app.audio_ui.caps = Some((key, caps));
    }
    let devices = app.audio_ui.devices.as_ref().map(|(_, d)| d.clone()).unwrap_or_default();
    let caps = app.audio_ui.caps.as_ref().map(|(_, c)| c.clone()).unwrap_or_else(|| Ok(DriverCaps::default()));
    let mut apply = false;
    let recording = app.is_recording_audio();
    let engine = &app.engine;

    widgets::fl_window(ctx, "audio_settings", "Audio settings", &mut open, [420.0, 140.0], [470.0, 380.0], |ui| {
        egui::Grid::new("audio_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Driver");
            egui::ComboBox::from_id_salt("audio_driver").width(300.0).selected_text(draft.driver.label()).show_ui(ui, |ui| {
                for &d in Driver::all() {
                    if ui.selectable_label(draft.driver == d, d.label()).clicked() && draft.driver != d {
                        draft = AudioConfig { driver: d, device: draft.device.clone(), sample_rate: None, buffer_frames: None };
                    }
                }
            });
            ui.end_row();

            ui.label("Device");
            ui.horizontal(|ui| {
                let shown = draft.device.clone().unwrap_or_else(|| "System default".into());
                egui::ComboBox::from_id_salt("audio_device").width(266.0).selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(draft.device.is_none(), "System default").clicked() {
                        draft.device = None;
                        draft.sample_rate = None;
                    }
                    for name in &devices {
                        if ui.selectable_label(draft.device.as_deref() == Some(name), name).clicked() {
                            draft.device = Some(name.clone());
                            draft.sample_rate = None;
                        }
                    }
                });
                if ui.small_button("⟳").on_hover_text("Look for devices again").clicked() {
                    refresh = true;
                }
            });
            ui.end_row();

            let caps_ok = caps.as_ref().ok();
            ui.label("Sample rate");
            match (draft.driver, caps_ok) {
                (Driver::Exclusive, Some(c)) if !c.sample_rates.is_empty() => {
                    let default = format!("Device default ({} Hz)", c.current_rate);
                    let shown = draft.sample_rate.map_or(default.clone(), |r| format!("{r} Hz"));
                    egui::ComboBox::from_id_salt("audio_rate").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(draft.sample_rate.is_none(), default.as_str()).clicked() {
                            draft.sample_rate = None;
                        }
                        for &r in &c.sample_rates {
                            if ui.selectable_label(draft.sample_rate == Some(r), format!("{r} Hz")).clicked() {
                                draft.sample_rate = Some(r);
                                draft.buffer_frames = None; // sizes differ per rate
                            }
                        }
                    });
                }
                (_, Some(c)) => {
                    ui.label(format!("{} Hz (set in Windows Sound settings)", c.current_rate));
                }
                (_, None) => {
                    ui.label("-");
                }
            }
            ui.end_row();

            ui.label("Buffer size");
            let rate = draft.sample_rate.or(caps_ok.map(|c| c.current_rate)).filter(|&r| r > 0).unwrap_or(48000);
            let fmt = |f: u32| format!("{f} samples ({:.1} ms)", f as f32 * 1000.0 / rate as f32);
            match caps_ok {
                Some(c) if c.buffer_frames.len() > 1 => {
                    let smallest = c.default_buffer.unwrap_or(c.buffer_frames[0]);
                    let shown = draft.buffer_frames.map_or(format!("Smallest: {}", fmt(smallest)), fmt);
                    egui::ComboBox::from_id_salt("audio_buffer").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(draft.buffer_frames.is_none(), format!("Smallest: {}", fmt(smallest))).clicked() {
                            draft.buffer_frames = None;
                        }
                        for &f in &c.buffer_frames {
                            if ui.selectable_label(draft.buffer_frames == Some(f), fmt(f)).clicked() {
                                draft.buffer_frames = Some(f);
                            }
                        }
                    });
                }
                Some(c) if c.buffer_frames.len() == 1 => {
                    ui.label(fmt(c.buffer_frames[0]));
                }
                _ => {
                    ui.label("Chosen by Windows (about 10 ms)");
                }
            }
            ui.end_row();
        });

        let hint = match draft.driver {
            Driver::Shared => "Works with every device; other apps keep playing. Highest latency.",
            Driver::LowLatency => "Uses the smallest period your driver allows; other apps keep playing. Needs Windows 10.",
            Driver::Exclusive => "Lowest latency. Takes over the device: other apps can't play sound on it while Goaty Loops runs.",
        };
        ui.label(egui::RichText::new(hint).color(theme::TEXT_DIM).size(12.0));
        if let (Driver::LowLatency, Ok(c)) = (draft.driver, &caps) {
            if c.buffer_frames.len() <= 1 {
                ui.label(egui::RichText::new("This device's driver doesn't offer periods smaller than the default; try exclusive mode.").color(theme::TEXT_DIM).size(12.0));
            }
        }
        if let Err(e) = &caps {
            ui.colored_label(theme::METER_HIGH, e);
        }

        ui.separator();
        ui.label(egui::RichText::new("Now running").strong());
        let info = &engine.info;
        let mut line = format!("{} - {} Hz", engine.device_name, engine.sample_rate);
        if let Some(b) = info.buffer_frames {
            line += &format!(", buffer {b} samples");
        }
        if let Some(f) = info.format {
            line += &format!(", {f}");
        }
        ui.label(line);
        ui.label(format!(
            "{}   |   output latency {}   |   CPU {:.0}%",
            info.driver.label(),
            engine.output_latency_ms().map_or("measuring...".to_string(), |ms| format!("{ms:.1} ms")),
            engine.shared.cpu_load() * 100.0
        ));
        if let Some(e) = &engine.error {
            ui.colored_label(theme::METER_HIGH, e);
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let changed = draft != applied || engine.error.is_some();
            let r = ui.add_enabled(changed && !recording, egui::Button::new("Apply"));
            if recording {
                r.on_disabled_hover_text("Stop recording first");
            } else if r.clicked() {
                apply = true;
            }
            if ui.add_enabled(draft != applied, egui::Button::new("Revert")).clicked() {
                draft = applied.clone();
            }
            ui.label(egui::RichText::new("Sound stops for a moment while the device reopens.").color(theme::TEXT_DIM).size(11.0));
        });
    });

    if refresh {
        app.audio_ui.devices = None;
        app.audio_ui.caps = None;
    }
    if apply {
        app.apply_audio_config(draft.clone());
        app.audio_ui.caps = None; // the device's state changed (e.g. now in use)
    }
    app.audio_ui.open = open;
    app.audio_ui.draft = open.then_some(draft);
    // Keep the CPU and latency readouts moving while the window is open.
    if open {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
