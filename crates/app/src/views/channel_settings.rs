use crate::app::App;
use crate::{theme, widgets};
use goaty_engine::*;
use egui::{pos2, vec2, Id, Rect, Sense, Stroke, Ui};
use std::ops::RangeInclusive;

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(ch) = app.settings_channel else { return };
    if ch >= app.project.channels.len() {
        app.settings_channel = None;
        return;
    }
    let mut open = true;
    let name = app.project.channels[ch].name.clone();
    widgets::fl_window(ctx, "channel_settings", &name, &mut open, [520.0, 160.0], [420.0, 420.0], |ui| body(app, ui, ch));
    if !open {
        app.settings_channel = None;
    }
}

struct Edit {
    dirty: bool,
    started: bool,
}

impl Edit {
    fn track(&mut self, r: &egui::Response) {
        self.dirty |= r.changed();
        self.started |= r.drag_started() || r.gained_focus() || (r.changed() && !r.dragged());
    }
}

fn slider(ui: &mut Ui, e: &mut Edit, v: &mut f32, range: RangeInclusive<f32>, text: &str, log: bool) {
    let r = ui.add(egui::Slider::new(v, range).logarithmic(log).text(text));
    e.track(&r);
}

fn body(app: &mut App, ui: &mut Ui, ch: usize) {
    let mut c = app.project.channels[ch].clone();
    let mut e = Edit { dirty: false, started: false };

    egui::Grid::new("ch_common").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        let r = ui.text_edit_singleline(&mut c.name);
        e.track(&r);
        ui.end_row();

        ui.label("Color");
        let mut col = [c.color.0, c.color.1, c.color.2];
        ui.color_edit_button_srgb(&mut col);
        if col != [c.color.0, c.color.1, c.color.2] {
            c.color = Rgb(col[0], col[1], col[2]);
            e.dirty = true;
            e.started = true;
        }
        ui.end_row();

        ui.label("Volume");
        slider(ui, &mut e, &mut c.volume, 0.0..=1.0, "", false);
        ui.end_row();
        ui.label("Pan");
        slider(ui, &mut e, &mut c.pan, -1.0..=1.0, "", false);
        ui.end_row();
        ui.label("Pitch");
        let range = c.pitch_range.max(1.0);
        slider(ui, &mut e, &mut c.pitch, -range..=range, "semitones", false);
        ui.end_row();
        ui.label("Cut / cut by");
        ui.horizontal(|ui| {
            let r = ui.add(egui::DragValue::new(&mut c.cut_group).range(0..=999)).on_hover_text("Group this channel cuts (0 = none)");
            e.track(&r);
            let r = ui.add(egui::DragValue::new(&mut c.cut_by).range(0..=999)).on_hover_text("Group that cuts this channel (0 = none)");
            e.track(&r);
        });
        ui.end_row();
        ui.label("Pitch range");
        let r = ui.add(egui::DragValue::new(&mut c.pitch_range).range(1.0..=48.0).speed(0.1).suffix(" st"));
        e.track(&r);
        ui.end_row();
        ui.label("Mixer track");
        let r = ui.add(egui::DragValue::new(&mut c.mixer_track).range(0..=app.project.mixer.len().saturating_sub(1)));
        e.track(&r);
        ui.end_row();
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(c.instrument.kind_name()).strong().color(theme::SELECT_GREEN));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Replace instrument", |ui| {
                if ui.button("Sampler").clicked() {
                    c.instrument = Instrument::Sampler(SamplerParams::default());
                    e.dirty = true;
                    e.started = true;
                    ui.close_menu();
                }
                if ui.button("Goaty Synth").clicked() {
                    c.instrument = Instrument::Synth(SynthParams::default());
                    e.dirty = true;
                    e.started = true;
                    ui.close_menu();
                }
                for k in DrumKind::ALL {
                    if ui.button(format!("Goaty Drum: {}", k.name())).clicked() {
                        c.instrument = Instrument::Drum(DrumParams::new(k));
                        e.dirty = true;
                        e.started = true;
                        ui.close_menu();
                    }
                }
            });
        });
    });

    let mut load_request = false;
    let mut open_editor = None;
    match &mut c.instrument {
        Instrument::Sampler(s) => sampler_ui(ui, &mut e, s, &mut load_request),
        Instrument::Synth(s) => synth_ui(ui, &mut e, s),
        Instrument::Drum(d) => {
            egui::ComboBox::from_id_salt("drum_kind").selected_text(d.kind.name()).show_ui(ui, |ui| {
                for k in DrumKind::ALL {
                    if ui.selectable_value(&mut d.kind, k, k.name()).changed() {
                        e.dirty = true;
                        e.started = true;
                    }
                }
            });
            slider(ui, &mut e, &mut d.tune, -24.0..=24.0, "Tune (semitones)", false);
            slider(ui, &mut e, &mut d.decay, 0.1..=4.0, "Decay", false);
        }
        Instrument::Plugin(p) => {
            ui.label(egui::RichText::new(&p.info.name).font(theme::font_medium(17.0)).color(theme::TEXT_BRIGHT));
            ui.label(egui::RichText::new(format!("{}  {}  ({})", p.info.vendor, p.info.version, p.info.sub_categories)).color(theme::TEXT_DIM));
            ui.label(egui::RichText::new(p.info.path.display().to_string()).size(12.0).color(theme::TEXT_FAINT));
            match &p.instance {
                Some(inst) => {
                    if inst.latency > 0 {
                        ui.label(egui::RichText::new(format!("Latency: {} samples", inst.latency)).color(theme::TEXT_DIM));
                    }
                    if ui.button("Show plugin editor").clicked() {
                        open_editor = Some(inst.clone());
                    }
                }
                None => {
                    ui.colored_label(theme::METER_HIGH, "The plugin could not be loaded.");
                }
            }
        }
        Instrument::Automation(a) => {
            ui.label(egui::RichText::new("Automation clip").font(theme::font_medium(17.0)).color(theme::TEXT_BRIGHT));
            ui.label(egui::RichText::new(format!("Controls: {}", super::describe_target(&app.project, a.target))).color(theme::TEXT));
            ui.label(egui::RichText::new(format!("{} points, {:.1} bars", a.points.len(), a.length_ticks() as f32 / app.project.bar_ticks() as f32)).color(theme::TEXT_DIM));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 80.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, theme::METER_BG);
            let len = a.length_ticks().max(1) as f32;
            super::paint_auto_curve(ui.painter(), rect.shrink(3.0), a, 0.0, len, theme::PR_NOTE);
        }
        Instrument::Silent { label } => {
            ui.label(format!("{label} channel (makes no sound)."));
        }
    }

    if let Some(inst) = open_editor {
        let title = c.name.clone();
        app.open_plugin_editor(inst, &title);
    }
    if load_request {
        let exts: Vec<&str> = sample::AUDIO_EXTENSIONS.to_vec();
        if let Some(path) = rfd::FileDialog::new().add_filter("Audio", &exts).pick_file() {
            match app.load_sample_cached(&path) {
                Ok(data) => {
                    if let Instrument::Sampler(s) = &mut c.instrument {
                        s.path = Some(path.clone());
                        s.data = Some(data);
                    }
                    if c.name.starts_with("Sampler") || c.name.is_empty() {
                        c.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    }
                    e.dirty = true;
                    e.started = true;
                }
                Err(err) => app.status = format!("Could not load {}: {err}", path.display()),
            }
        }
    }

    ui.separator();
    mini_keyboard(app, ui, ch);

    if e.dirty {
        if e.started {
            app.checkpoint();
        }
        app.project.channels[ch] = c;
        app.changed();
    }
}

fn sampler_ui(ui: &mut Ui, e: &mut Edit, s: &mut SamplerParams, load_request: &mut bool) {
    ui.horizontal(|ui| {
        if ui.button("Load sample...").clicked() {
            *load_request = true;
        }
        let path = s.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(no sample)".into());
        ui.label(egui::RichText::new(path).size(10.5).color(theme::TEXT_DIM));
    });
    if let Some(data) = &s.data {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, theme::PR_ROW_BLACK);
        let n = data.peaks.len().max(1);
        let mid = rect.center().y;
        for x in 0..rect.width() as usize {
            let i = x * n / rect.width().max(1.0) as usize;
            let peak = data.peaks.get(i).copied().unwrap_or(0.0).min(1.0);
            let h = peak * rect.height() * 0.48;
            let xx = rect.left() + x as f32;
            p.line_segment([pos2(xx, mid - h), pos2(xx, mid + h)], Stroke::new(1.0_f32, theme::METER_LOW));
        }
        ui.label(egui::RichText::new(format!("{:.2} s, {} Hz", data.duration_secs(), data.sample_rate)).size(10.5).color(theme::TEXT_DIM));
    } else if s.path.is_some() {
        ui.colored_label(theme::METER_HIGH, "Sample file is missing.");
    }
    ui.horizontal(|ui| {
        ui.label("Root key");
        let r = ui.add(egui::DragValue::new(&mut s.root_key).range(0..=127).custom_formatter(|v, _| widgets::key_name(v as u8)));
        e.track(&r);
        let r = ui.checkbox(&mut s.reverse, "Reverse");
        e.track(&r);
        let r = ui.checkbox(&mut s.use_envelope, "Volume envelope");
        e.track(&r);
    });
    ui.horizontal(|ui| {
        ui.label("Preamp");
        let mut db = 20.0 * s.preamp.max(1e-6).log10();
        let r = ui.add(egui::DragValue::new(&mut db).range(0.0..=24.6).speed(0.1).suffix(" dB")).on_hover_text("Boosts the sample before the channel volume (FL's sampler preamp)");
        if r.changed() {
            // FL stores the preamp in 1/16 steps of linear gain.
            s.preamp = ((10f32.powf(db / 20.0) - 1.0) * 16.0).round().max(0.0) / 16.0 + 1.0;
        }
        e.track(&r);
    });
    ui.horizontal(|ui| {
        ui.label("Time stretch");
        let r = ui
            .add(egui::DragValue::new(&mut s.stretch_beats).range(0.0..=1024.0).speed(0.25).custom_formatter(|v, _| if v <= 0.0 { "off".into() } else { format!("{v:.2} beats") }))
            .on_hover_text("Play the whole sample over this many beats. 0 = off");
        e.track(&r);
        let r = ui
            .add_enabled(s.stretch_beats > 0.0, egui::Checkbox::new(&mut s.keep_pitch, "Keep pitch"))
            .on_hover_text("On: stretch without changing pitch. Off: resample, so the pitch follows the speed (FL's default)");
        e.track(&r);
    });
    if s.use_envelope {
        envelope_ui(ui, e, &mut s.env);
    }
}

fn envelope_ui(ui: &mut Ui, e: &mut Edit, env: &mut Envelope) {
    slider(ui, e, &mut env.attack, 0.0005..=5.0, "Attack (s)", true);
    slider(ui, e, &mut env.hold, 0.0..=5.0, "Hold (s)", false);
    slider(ui, e, &mut env.decay, 0.001..=5.0, "Decay (s)", true);
    slider(ui, e, &mut env.sustain, 0.0..=1.0, "Sustain", false);
    slider(ui, e, &mut env.release, 0.002..=8.0, "Release (s)", true);
}

fn synth_ui(ui: &mut Ui, e: &mut Edit, s: &mut SynthParams) {
    if let Some(r) = &s.replaces {
        ui.label(egui::RichText::new(format!("Stand-in for the plugin \"{r}\", which isn't available.")).size(11.0).color(theme::SELECT_GREEN));
    }
    for (i, o) in s.osc.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("OSC {}", i + 1)).strong());
            egui::ComboBox::from_id_salt(("osc_wave", i)).width(76.0).selected_text(o.wave.name()).show_ui(ui, |ui| {
                for w in Wave::ALL {
                    if ui.selectable_value(&mut o.wave, w, w.name()).changed() {
                        e.dirty = true;
                        e.started = true;
                    }
                }
            });
            let r = ui.add(egui::DragValue::new(&mut o.coarse).range(-36..=36).suffix(" st")).on_hover_text("Coarse tune");
            e.track(&r);
            let r = ui.add(egui::DragValue::new(&mut o.fine).range(-100.0..=100.0).speed(0.5).suffix(" ct")).on_hover_text("Fine tune");
            e.track(&r);
            let r = widgets::knob(ui, &mut o.level, 0.0..=1.0, 0.5, 20.0, "Level");
            e.track(&r);
        });
    }
    ui.add_space(4.0);
    envelope_ui(ui, e, &mut s.env);
    slider(ui, e, &mut s.cutoff, 40.0..=20000.0, "Cutoff (Hz)", true);
    slider(ui, e, &mut s.resonance, 0.0..=1.0, "Resonance", false);
}

fn mini_keyboard(app: &mut App, ui: &mut Ui, ch: usize) {
    let id = Id::new("settings_kb_note");
    let lo = 48u8;
    let keys = 36u8;
    let whites: Vec<u8> = (lo..lo + keys).filter(|k| !widgets::is_black_key(*k)).collect();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click_and_drag());
    let ww = rect.width() / whites.len() as f32;
    let p = ui.painter();
    let mut black_rects = Vec::new();
    for (i, k) in whites.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * ww, rect.top()), vec2(ww - 1.0, rect.height()));
        p.rect_filled(r, 1.0, theme::KEY_WHITE);
        if k % 12 == 0 {
            p.text(pos2(r.center().x, r.bottom() - 7.0), egui::Align2::CENTER_CENTER, widgets::key_name(*k), egui::FontId::proportional(8.5), theme::WINDOW_BG);
        }
        if k + 1 < lo + keys && widgets::is_black_key(k + 1) {
            let br = Rect::from_min_size(pos2(r.right() - ww * 0.3, rect.top()), vec2(ww * 0.6, rect.height() * 0.6));
            black_rects.push((k + 1, br));
        }
    }
    for (_, br) in &black_rects {
        p.rect_filled(*br, 1.0, theme::KEY_BLACK);
    }

    let held: Option<u8> = ui.data(|d| d.get_temp(id));
    let pressed = ui.input(|i| i.pointer.primary_down());
    let key_at = |pos: egui::Pos2| -> Option<u8> {
        if let Some((k, _)) = black_rects.iter().find(|(_, r)| r.contains(pos)) {
            return Some(*k);
        }
        let i = ((pos.x - rect.left()) / ww) as usize;
        whites.get(i).copied()
    };
    let want = if resp.is_pointer_button_down_on() && pressed { resp.interact_pointer_pos().and_then(key_at) } else { None };
    if want != held {
        if let Some(k) = held {
            app.preview_off(ch, k);
        }
        if let Some(k) = want {
            app.preview_on(ch, k);
        }
        ui.data_mut(|d| match want {
            Some(k) => d.insert_temp(id, k),
            None => d.remove::<u8>(id),
        });
    }
}
