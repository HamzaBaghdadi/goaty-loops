use crate::app::App;
use crate::{theme, widgets};
use goaty_engine::*;
use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Ui};

#[derive(Default)]
pub struct State {
    /// Value being painted while dragging across steps.
    paint: Option<bool>,
    preview: Option<usize>,
    action: Option<RowAction>,
}

enum RowAction {
    Delete(usize),
    Clone(usize),
    Fill(usize, u32),
    Clear(usize),
    Solo(usize),
    MoveUp(usize),
    MoveDown(usize),
}

const STEP_W: f32 = 15.0;
const STEP_H: f32 = 24.0;
const STEP_GAP: f32 = 2.0;
const GROUP_GAP: f32 = 4.0;

pub fn window(app: &mut App, ctx: &egui::Context) {
    let mut open = app.show_rack;
    let pat = app.project.patterns.get(app.cur_pattern).map(|p| p.name.clone()).unwrap_or_default();
    widgets::fl_window(ctx, "channel_rack", &format!("Channel rack  \u{203A}  {pat}"), &mut open, [420.0, 140.0], [720.0, 300.0], |ui| body(app, ui));
    app.show_rack = open;
}

fn body(app: &mut App, ui: &mut Ui) {
    let (primary_down, secondary_down) = ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
    if !primary_down && !secondary_down {
        app.rack.paint = None;
    }
    if !primary_down {
        if let Some(ch) = app.rack.preview.take() {
            app.preview_off(ch, 60);
        }
    }

    let steps = (app.project.pattern_length(app.cur_pattern) / STEP_TICKS).max(16);
    egui::ScrollArea::both().auto_shrink([false, true]).max_height(ui.available_height() - 28.0).show(ui, |ui| {
        if app.project.channels.is_empty() {
            ui.label(egui::RichText::new("No channels. Add one with the + button or drop a sample here.").color(theme::TEXT_DIM));
        }
        for ch in 0..app.project.channels.len() {
            row(app, ui, ch, steps);
        }
    });

    if let Some(a) = app.rack.action.take() {
        apply_action(app, a);
    }

    ui.horizontal(|ui| {
        ui.menu_button(egui::RichText::new(" + ").strong(), |ui| add_channel_menu(app, ui)).response.on_hover_text("Add channel");
        ui.separator();
        ui.label(egui::RichText::new("Pattern length").color(theme::TEXT_DIM).size(11.0));
        let bar = app.project.bar_ticks();
        let pat = app.cur_pattern;
        let current = app.project.patterns[pat].length;
        let label = |l: Option<u32>| match l {
            None => "Auto".to_string(),
            Some(t) => format!("{} bar{}", t / bar, if t / bar == 1 { "" } else { "s" }),
        };
        egui::ComboBox::from_id_salt("pattern_len").width(70.0).selected_text(label(current)).show_ui(ui, |ui| {
            for opt in [None, Some(bar), Some(bar * 2), Some(bar * 4), Some(bar * 8)] {
                if ui.selectable_label(current == opt, label(opt)).clicked() && current != opt {
                    app.checkpoint();
                    app.project.patterns[pat].length = opt;
                    app.changed();
                }
            }
        });
    });
}

fn row(app: &mut App, ui: &mut Ui, ch: usize, steps: u32) {
    // Rounded row panel behind the controls, like FL's rack rows.
    let top = ui.cursor().min;
    let width = 6.0 + 16.0 + 26.0 * 2.0 + 34.0 + 140.0 + 12.0 + steps_width(steps) + 26.0;
    ui.painter().rect_filled(Rect::from_min_size(top - vec2(2.0, 1.0), vec2(width, STEP_H + 4.0)), 4.0, theme::RACK_ROW);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.add_space(4.0);
        let c = app.project.channels[ch].clone();

        let r = widgets::led(ui, !c.muted, 14.0).on_hover_text("Mute (right-click: solo)");
        if r.clicked() {
            app.checkpoint();
            app.project.channels[ch].muted = !c.muted;
            app.changed();
        }
        if r.secondary_clicked() {
            app.rack.action = Some(RowAction::Solo(ch));
        }

        let mut pan = c.pan;
        let r = widgets::knob(ui, &mut pan, -1.0..=1.0, 0.0, 21.0, "Pan");
        r.context_menu(|ui| app.midi_link_menu(ui, AutoTarget::ChannelPan(ch)));
        if r.drag_started() {
            app.checkpoint();
        }
        if r.changed() {
            app.project.channels[ch].pan = pan;
            app.changed();
        }
        let mut vol = c.volume;
        let r = widgets::knob(ui, &mut vol, 0.0..=1.0, DEFAULT_CHANNEL_VOLUME, 21.0, "Volume");
        r.context_menu(|ui| app.midi_link_menu(ui, AutoTarget::ChannelVolume(ch)));
        if r.drag_started() {
            app.checkpoint();
        }
        if r.changed() {
            app.project.channels[ch].volume = vol;
            app.changed();
        }

        let max_track = app.project.mixer.len().saturating_sub(1);
        let mut mt = c.mixer_track;
        let r = ui.add_sized(vec2(28.0, 18.0), egui::DragValue::new(&mut mt).range(0..=max_track).speed(0.05).custom_formatter(|v, _| if v == 0.0 { "---".into() } else { format!("{v:.0}") })).on_hover_text("Mixer track (--- = master)");
        if r.drag_started() || r.gained_focus() {
            app.checkpoint();
        }
        if mt != c.mixer_track {
            app.project.channels[ch].mixer_track = mt;
            app.changed();
        }

        name_button(app, ui, ch, &c);

        if let Instrument::Automation(a) = &c.instrument {
            let (rect, resp) = ui.allocate_exact_size(vec2(steps_width(steps), STEP_H), Sense::click());
            ui.painter().rect_filled(rect, 3.0, theme::PR_ROW_BLACK);
            let len = a.length_ticks().max(1) as f32;
            super::paint_auto_curve(ui.painter(), rect.shrink(3.0), a, 0.0, len, theme::lighten(theme::rgb(c.color), 0.4));
            if resp.on_hover_text(super::describe_target(&app.project, a.target)).double_clicked() {
                app.automation_ui.open = Some(ch);
                app.playlist_ui.paint_auto = Some(ch);
            }
            return;
        }
        let pat = &app.project.patterns[app.cur_pattern];
        if pat.is_step_only(ch) {
            step_row(app, ui, ch, steps);
        } else {
            mini_roll(app, ui, ch, steps);
        }
    });
}

fn name_button(app: &mut App, ui: &mut Ui, ch: usize, c: &Channel) {
    let (rect, resp) = ui.allocate_exact_size(vec2(132.0, STEP_H), Sense::click());
    let selected = app.sel_channel == ch;
    let p = ui.painter();
    let base = theme::mix(theme::RACK_NAME, theme::rgb(c.color), 0.18);
    let bg = if resp.hovered() { theme::lighten(base, 0.07) } else { base };
    p.rect_filled(rect, 4.0, bg);
    p.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.5)), egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, Color32::from_white_alpha(6));
    let text_col = if c.muted { theme::TEXT_DIM } else { theme::TEXT_BRIGHT };
    let font = theme::font(15.0);
    let name = crate::widgets::fit_text(p, &c.name, &font, rect.width() - 14.0);
    p.text(pos2(rect.left() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, name, font, text_col);
    // Activity meter.
    let level = app.channel_meters.get(ch).copied().unwrap_or(0.0).min(1.0);
    if level > 0.01 {
        let h = (rect.height() - 6.0) * level.sqrt();
        p.rect_filled(Rect::from_min_max(pos2(rect.right() - 5.0, rect.bottom() - 3.0 - h), pos2(rect.right() - 2.5, rect.bottom() - 3.0)), 0.0, theme::LED_ON);
    }
    // Selection indicator to the right of the name, green when selected.
    let (sel_rect, sel_resp) = ui.allocate_exact_size(vec2(8.0, STEP_H), Sense::click());
    let p = ui.painter();
    p.rect_filled(sel_rect.shrink2(vec2(0.0, 2.0)), 2.0, if selected { theme::SELECT_GREEN } else { Color32::from_rgb(0x1b, 0x1e, 0x20) });
    if sel_resp.clicked() {
        app.sel_channel = ch;
    }

    if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        app.sel_channel = ch;
        app.preview_on(ch, 60);
        app.rack.preview = Some(ch);
    }
    if resp.double_clicked() {
        match &c.instrument {
            Instrument::Plugin(PluginParams { instance: Some(inst), .. }) => app.open_plugin_editor(inst.clone(), &c.name),
            _ => app.settings_channel = Some(ch),
        }
    }
    let kind = c.instrument.kind_name();
    let hover = match &c.instrument {
        Instrument::Synth(SynthParams { replaces: Some(r), .. }) => format!("{} ({kind}, stand-in for {r})", c.name),
        _ => format!("{} ({kind})\nDouble-click for channel settings", c.name),
    };
    resp.on_hover_text(hover).context_menu(|ui| {
        if ui.button("Piano roll").clicked() {
            app.sel_channel = ch;
            app.show_piano = true;
            ui.close_menu();
        }
        if ui.button("Channel settings...").clicked() {
            app.settings_channel = Some(ch);
            ui.close_menu();
        }
        ui.separator();
        for (label, n) in [("Fill each 2 steps", 2), ("Fill each 4 steps", 4), ("Fill each 8 steps", 8)] {
            if ui.button(label).clicked() {
                app.rack.action = Some(RowAction::Fill(ch, n));
                ui.close_menu();
            }
        }
        if ui.button("Clear notes").clicked() {
            app.rack.action = Some(RowAction::Clear(ch));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Move up").clicked() {
            app.rack.action = Some(RowAction::MoveUp(ch));
            ui.close_menu();
        }
        if ui.button("Move down").clicked() {
            app.rack.action = Some(RowAction::MoveDown(ch));
            ui.close_menu();
        }
        if ui.button("Clone").clicked() {
            app.rack.action = Some(RowAction::Clone(ch));
            ui.close_menu();
        }
        if ui.button("Delete").clicked() {
            app.rack.action = Some(RowAction::Delete(ch));
            ui.close_menu();
        }
    });
}

fn step_rect(origin: egui::Pos2, i: u32) -> Rect {
    let x = origin.x + i as f32 * (STEP_W + STEP_GAP) + (i / 4) as f32 * GROUP_GAP;
    Rect::from_min_size(pos2(x, origin.y), vec2(STEP_W, STEP_H))
}

fn steps_width(steps: u32) -> f32 {
    steps as f32 * (STEP_W + STEP_GAP) + (steps / 4) as f32 * GROUP_GAP
}

fn current_step(app: &App) -> Option<u32> {
    if !app.is_playing() || app.song_mode {
        return None;
    }
    Some((app.play_position() / STEP_TICKS as f64) as u32)
}

fn step_row(app: &mut App, ui: &mut Ui, ch: usize, steps: u32) {
    let (rect, _) = ui.allocate_exact_size(vec2(steps_width(steps), STEP_H), Sense::click_and_drag());
    let playing_step = current_step(app);
    let pat_idx = app.cur_pattern;

    let pointer = ui.input(|i| i.pointer.interact_pos());
    let hovered_step = pointer
        .filter(|_| ui.rect_contains_pointer(rect))
        .and_then(|pos| (0..steps).find(|&i| step_rect(rect.min, i).expand2(vec2(STEP_GAP, 0.0)).contains(pos)));
    if let Some(i) = hovered_step {
        let (pp, sp, pd, sd) = ui.input(|inp| (inp.pointer.primary_pressed(), inp.pointer.secondary_pressed(), inp.pointer.primary_down(), inp.pointer.secondary_down()));
        let on = app.project.patterns[pat_idx].step_on(ch, i);
        if pp {
            app.checkpoint();
            app.rack.paint = Some(!on);
        } else if sp {
            app.checkpoint();
            app.rack.paint = Some(false);
        }
        if let Some(v) = app.rack.paint {
            if (pd || sd) && on != v {
                app.project.patterns[pat_idx].set_step(ch, i, v);
                app.changed();
            }
        }
    }

    let p = ui.painter();
    let pat = &app.project.patterns[pat_idx];
    for i in 0..steps {
        let r = step_rect(rect.min, i);
        let on = pat.step_on(ch, i);
        let base = if (i / 4) % 2 == 0 { theme::STEP_A } else { theme::STEP_B };
        let fill = if on { theme::STEP_ON } else { base };
        p.rect_filled(r, 3.0, fill);
        if on {
            p.rect_filled(Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(STEP_W - 4.0, 3.0)), 1.0, theme::lighten(fill, 0.6));
        } else {
            p.rect_filled(Rect::from_min_size(r.min + vec2(0.0, STEP_H - 3.0), vec2(STEP_W, 3.0)), 1.0, theme::darken(base, 0.25));
        }
        if playing_step == Some(i) {
            p.rect_stroke(r, 2.0, Stroke::new(1.5_f32, theme::PLAYHEAD), egui::StrokeKind::Outside);
        }
    }
}

fn mini_roll(app: &mut App, ui: &mut Ui, ch: usize, steps: u32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(steps_width(steps), STEP_H), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, theme::PR_ROW_BLACK);
    let pat = &app.project.patterns[app.cur_pattern];
    let notes: Vec<&Note> = pat.notes.iter().filter(|n| n.channel == ch).collect();
    let (lo, hi) = notes.iter().fold((127u8, 0u8), |(lo, hi), n| (lo.min(n.key), hi.max(n.key)));
    let range = (hi.saturating_sub(lo)).max(6) as f32;
    let total = (steps * STEP_TICKS) as f32;
    let col = theme::lighten(theme::rgb(app.project.channels[ch].color), 0.3);
    for n in notes {
        let x0 = rect.left() + n.start as f32 / total * rect.width();
        let x1 = rect.left() + n.end() as f32 / total * rect.width();
        let y = rect.bottom() - 3.0 - (n.key - lo) as f32 / range * (rect.height() - 6.0);
        p.line_segment([pos2(x0, y), pos2(x1.max(x0 + 2.0), y)], Stroke::new(2.0_f32, col));
    }
    if let Some(s) = current_step(app) {
        let x = rect.left() + (s * STEP_TICKS) as f32 / total * rect.width();
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, theme::PLAYHEAD));
    }
    if resp.on_hover_text("Click to edit in the piano roll").clicked() {
        app.sel_channel = ch;
        app.show_piano = true;
    }
}

fn apply_action(app: &mut App, a: RowAction) {
    app.checkpoint();
    let pat = app.cur_pattern;
    match a {
        RowAction::Delete(ch) => {
            app.project.remove_channel(ch);
            app.settings_channel = None;
        }
        RowAction::Clone(ch) => {
            let mut c = app.project.channels[ch].clone();
            c.name = format!("{} #2", c.name);
            app.project.channels.push(c);
        }
        RowAction::Fill(ch, every) => {
            let steps = app.project.pattern_length(pat) / STEP_TICKS;
            let p = &mut app.project.patterns[pat];
            for s in 0..steps {
                p.set_step(ch, s, s % every == 0);
            }
        }
        RowAction::Clear(ch) => app.project.patterns[pat].notes.retain(|n| n.channel != ch),
        RowAction::Solo(ch) => {
            let others_muted = app.project.channels.iter().enumerate().all(|(i, c)| i == ch || c.muted);
            for (i, c) in app.project.channels.iter_mut().enumerate() {
                c.muted = if others_muted { false } else { i != ch };
            }
        }
        RowAction::MoveUp(ch) if ch > 0 => swap_channels(&mut app.project, ch, ch - 1),
        RowAction::MoveDown(ch) if ch + 1 < app.project.channels.len() => swap_channels(&mut app.project, ch, ch + 1),
        _ => {}
    }
    app.changed();
}

fn swap_channels(p: &mut Project, a: usize, b: usize) {
    p.remap_automation_channels(|c| Some(if c == a { b } else if c == b { a } else { c }));
    p.channels.swap(a, b);
    let fix = |c: &mut usize| {
        if *c == a {
            *c = b
        } else if *c == b {
            *c = a
        }
    };
    for pat in &mut p.patterns {
        for n in &mut pat.notes {
            fix(&mut n.channel);
        }
    }
    for clip in &mut p.playlist.clips {
        if let ClipSource::Audio(c) = &mut clip.source {
            fix(c);
        }
    }
}

pub fn add_channel_menu(app: &mut App, ui: &mut Ui) {
    if ui.button("Sampler (load audio file)...").clicked() {
        ui.close_menu();
        let exts: Vec<&str> = sample::AUDIO_EXTENSIONS.to_vec();
        if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &exts).pick_files() {
            for f in files {
                app.add_sample_channel(&f);
            }
        }
    }
    let instruments: Vec<plugin::PluginInfo> = app.plugins.db.instruments().cloned().collect();
    ui.menu_button("VST3 instruments", |ui| {
        if instruments.is_empty() {
            ui.label(if app.plugins.scanning() { "Scanning plugins..." } else { "No VST3 instruments found" });
        }
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for info in &instruments {
                if ui.button(&info.name).on_hover_text(format!("{} - {}", info.vendor, info.path.display())).clicked() {
                    ui.close_menu();
                    app.add_plugin_channel(info);
                }
            }
        });
    });
    ui.separator();
    if ui.button("Goaty Synth").clicked() {
        app.add_channel(Channel::new("Goaty Synth", Instrument::Synth(SynthParams::default())));
        ui.close_menu();
    }
    ui.menu_button("Goaty Drum", |ui| {
        for k in DrumKind::ALL {
            if ui.button(k.name()).clicked() {
                app.add_channel(Channel::new(k.name(), Instrument::Drum(DrumParams::new(k))));
                ui.close_menu();
            }
        }
    });
}
