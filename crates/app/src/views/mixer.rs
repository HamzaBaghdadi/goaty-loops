use crate::app::App;
use crate::theme;
use crate::widgets::{self, Icon};
use goaty_engine::*;
use egui::{pos2, vec2, Align2, Color32, Id, Rect, Sense, Stroke, Ui};

pub struct State {
    pub selected: usize,
    wide: bool,
    open_fx: Vec<(usize, usize)>,
}

impl Default for State {
    fn default() -> Self {
        Self { selected: 0, wide: true, open_fx: Vec::new() }
    }
}

const SLOTS: usize = 10;
const SCALE_W: f32 = 44.0;
const PANEL_W: f32 = 270.0;
const HDR_H: f32 = 22.0;

pub fn window(app: &mut App, ctx: &egui::Context) {
    let mut open = app.show_mixer;
    let sel = app.mixer_ui.selected.min(app.project.mixer.len().saturating_sub(1));
    let title = format!("Mixer - {}", app.project.mixer.get(sel).map(|t| t.name.as_str()).unwrap_or(""));
    widgets::fl_window(ctx, "mixer", &title, &mut open, [260.0, 250.0], [1180.0, 500.0], |ui| body(app, ui));
    app.show_mixer = open;
    effect_windows(app, ctx);
}

fn body(app: &mut App, ui: &mut Ui) {
    // Toolbar.
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let r = widgets::icon_button(ui, Icon::Plus, theme::TEXT, 18.0, "Add insert track");
        if r.clicked() {
            app.add_mixer_track();
        }
        let label = if app.mixer_ui.wide { "Wide" } else { "Compact" };
        if ui.add(egui::Label::new(egui::RichText::new(label).font(theme::font_medium(15.0)).color(theme::TEXT)).sense(Sense::click())).on_hover_text("Toggle strip width").clicked() {
            app.mixer_ui.wide = !app.mixer_ui.wide;
        }
    });
    ui.add_space(2.0);
    let avail = ui.available_rect_before_wrap();
    let panel = Rect::from_min_max(pos2(avail.right() - PANEL_W, avail.top()), avail.max);
    let strips = Rect::from_min_max(avail.min, pos2(panel.left() - 4.0, avail.bottom()));
    ui.painter().rect_filled(avail, 0.0, theme::MIX_BG);

    let mut strips_ui = ui.new_child(egui::UiBuilder::new().max_rect(strips));
    egui::ScrollArea::horizontal().id_salt("mixer_strips").auto_shrink([false, false]).show(&mut strips_ui, |ui| {
        let strip_w = if app.mixer_ui.wide { 56.0 } else { 44.0 };
        let n = app.project.mixer.len();
        let total = SCALE_W + 4.0 + (strip_w + 6.0) + 10.0 + (n.saturating_sub(1)) as f32 * strip_w + 30.0;
        let h = strips.height() - 14.0;
        let (rect, _) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
        db_scale(ui, Rect::from_min_size(rect.min, vec2(SCALE_W, h)));
        let mut x = rect.left() + SCALE_W + 4.0;
        for t in 0..n {
            let w = if t == 0 { strip_w + 6.0 } else { strip_w };
            strip(app, ui, t, Rect::from_min_size(pos2(x, rect.top()), vec2(w, h)));
            x += w;
            if t == 0 {
                x += 10.0;
            }
        }
    });

    let mut panel_ui = ui.new_child(egui::UiBuilder::new().max_rect(panel));
    slot_panel(app, &mut panel_ui, panel);
    ui.allocate_rect(avail, Sense::hover());
}

fn db_scale(ui: &Ui, r: Rect) {
    let p = ui.painter();
    p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), HDR_H)), 0.0, theme::MIX_HDR);
    p.text(pos2(r.center().x, r.top() + HDR_H / 2.0), Align2::CENTER_CENTER, "C", theme::font(14.0), theme::TEXT_DIM);
    let body = Rect::from_min_max(pos2(r.left() + 8.0, r.top() + HDR_H + 8.0), pos2(r.right() - 8.0, r.top() + HDR_H + 8.0 + (r.height() - HDR_H) * 0.68));
    p.rect_filled(body, 2.0, Color32::from_rgb(0x18, 0x1c, 0x1e));
    for (i, db) in [3, 0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39].iter().enumerate() {
        let y = body.top() + 10.0 + i as f32 / 14.0 * (body.height() - 20.0);
        p.text(pos2(body.center().x, y), Align2::CENTER_CENTER, db.to_string(), theme::font(if i < 11 { 13.0 } else { 9.5 }), theme::TEXT);
        p.line_segment([pos2(body.left() + 6.0, y + 8.0), pos2(body.right() - 6.0, y + 8.0)], Stroke::new(1.0_f32, theme::TEXT_FAINT));
    }
}

fn strip(app: &mut App, ui: &mut Ui, t: usize, r: Rect) {
    let selected = app.mixer_ui.selected.min(app.project.mixer.len() - 1) == t;
    let id = Id::new(("mix_strip", t));
    if ui.interact(r, id, Sense::click()).clicked() {
        app.mixer_ui.selected = t;
    }
    let track = app.project.mixer[t].clone();
    let custom = track.color != MixerTrack::new("").color;
    let tint = |c: Color32| if custom { theme::mix(c, theme::rgb(track.color), 0.22) } else { c };

    let p = ui.painter();
    let base = if t == 0 { theme::MIX_MASTER } else { theme::MIX_STRIP };
    p.rect_filled(r.shrink2(vec2(0.5, 0.0)), 0.0, tint(base));
    p.line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0_f32, theme::darken(base, 0.35)));
    // Header with the insert number.
    let hdr = Rect::from_min_size(r.min, vec2(r.width(), HDR_H));
    let hdr_col = if custom { theme::mix(theme::MIX_HDR, theme::rgb(track.color), 0.6) } else { theme::MIX_HDR };
    p.rect_filled(hdr, 0.0, hdr_col);
    let label = if t == 0 { "M".to_string() } else { t.to_string() };
    p.text(hdr.center(), Align2::CENTER_CENTER, label, theme::font(15.0), if selected { theme::TEXT_BRIGHT } else { theme::TEXT_DIM });

    // Dark card with name and meter.
    let card = Rect::from_min_max(pos2(r.left() + 2.0, hdr.bottom()), pos2(r.right() - 2.0, hdr.bottom() + 190.0));
    p.rect_filled(card, egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, tint(theme::MIX_CARD));
    let meter_box = Rect::from_min_max(pos2(card.right() - 20.0, card.top() + 6.0), pos2(card.right() - 5.0, card.top() + 128.0));
    p.rect_filled(meter_box, 1.0, theme::METER_BG);
    let (ml, mr) = app.mixer_meters.get(t).copied().unwrap_or((0.0, 0.0));
    let inner = meter_box.shrink(2.0);
    let half = inner.width() / 2.0;
    widgets::paint_meter(p, Rect::from_min_size(inner.min, vec2(half - 0.5, inner.height())), ml);
    widgets::paint_meter(p, Rect::from_min_size(inner.min + vec2(half + 0.5, 0.0), vec2(half - 0.5, inner.height())), mr);
    if selected {
        p.rect_stroke(meter_box.expand(1.0), 1.0, Stroke::new(1.5_f32, theme::SELECT_GREEN), egui::StrokeKind::Outside);
    }
    let name_font = theme::font(15.5);
    let name = widgets::fit_text(p, &track.name, &name_font, 118.0);
    let name_col = if track.name.starts_with("Insert ") && track.effects.is_empty() { theme::TEXT_FAINT } else { theme::TEXT };
    widgets::vertical_text(p, pos2(card.left() + (meter_box.left() - card.left()) / 2.0 - 8.0, card.top() + 128.0), &name, name_font, name_col);

    // Mute LED.
    let led_c = pos2(card.center().x, card.top() + 146.0);
    let led_r = Rect::from_center_size(led_c, vec2(16.0, 16.0));
    widgets::paint_led(p, led_c, 5.5, !track.muted);
    if ui.interact(led_r, id.with("led"), Sense::click()).on_hover_text("Mute").clicked() {
        app.checkpoint();
        app.project.mixer[t].muted = !track.muted;
        app.changed();
    }

    // Pan knob straddling the bottom of the card.
    let knob_r = Rect::from_center_size(pos2(card.center().x, card.bottom() - 6.0), vec2(36.0, 36.0));
    ui.painter().circle_filled(knob_r.center(), 21.0, tint(theme::MIX_CARD));
    let mut pan = track.pan;
    let resp = widgets::knob_at(ui, knob_r, id.with("pan"), &mut pan, -1.0..=1.0, 0.0, "Pan");
    resp.context_menu(|ui| app.midi_link_menu(ui, AutoTarget::MixerPan(t)));
    if resp.drag_started() {
        app.checkpoint();
    }
    if resp.changed() {
        app.project.mixer[t].pan = pan;
        app.changed();
    }

    // Fader.
    let fader_r = Rect::from_min_max(pos2(r.right() - 26.0, card.bottom() + 24.0), pos2(r.right() - 6.0, r.bottom() - 80.0));
    let mut gain = track.volume;
    let resp = widgets::fader(ui, &mut gain, fader_r, selected, id.with("fader"));
    resp.context_menu(|ui| app.midi_link_menu(ui, AutoTarget::MixerVolume(t)));
    if resp.drag_started() {
        app.checkpoint();
    }
    if resp.changed() {
        app.project.mixer[t].volume = gain;
        app.changed();
    }
    // Volume readout on the left column.
    let p = ui.painter();
    let db = 20.0 * track.volume.max(1e-6).log10();
    p.text(pos2(r.left() + 12.0, card.bottom() + 34.0), Align2::CENTER_CENTER, if db < -99.0 { "-inf".into() } else { format!("{db:.0}") }, theme::font(11.0), theme::TEXT_FAINT);

    // Routing: the arrow at the bottom sends the selected track here (FL style).
    let sel = app.mixer_ui.selected.min(app.project.mixer.len() - 1);
    let routed_here = sel != t && app.project.mixer[sel].output == t && sel > 0;
    let can_route = sel > t;
    let arrow = Rect::from_center_size(pos2(r.center().x, r.bottom() - 22.0), vec2(22.0, 18.0));
    let col = if routed_here { theme::ORANGE } else if can_route { theme::TEXT_DIM } else { theme::TEXT_FAINT.gamma_multiply(0.5) };
    let c = arrow.center();
    if routed_here {
        p.add(egui::Shape::convex_polygon(vec![c + vec2(-7.0, 5.0), c + vec2(0.0, -5.0), c + vec2(7.0, 5.0)], col, Stroke::NONE));
    } else {
        p.add(egui::Shape::closed_line(vec![c + vec2(-7.0, 5.0), c + vec2(0.0, -5.0), c + vec2(7.0, 5.0)], Stroke::new(1.2_f32, col)));
    }
    if t > 0 {
        let out_name = if track.output == 0 { "Master".to_string() } else { format!("Insert {}", track.output) };
        p.text(pos2(r.center().x, r.bottom() - 52.0), Align2::CENTER_CENTER, if track.output == 0 { "M".into() } else { track.output.to_string() }, theme::font(11.0), theme::ORANGE.gamma_multiply(0.8));
        let _ = out_name;
    }
    let resp = ui.interact(arrow, id.with("route"), Sense::click());
    if can_route && resp.on_hover_text("Route the selected track to this track").clicked() {
        app.checkpoint();
        let cur = app.project.mixer[sel].output;
        app.project.mixer[sel].output = if cur == t { 0 } else { t };
        app.changed();
    }
}

fn slot_panel(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = app.mixer_ui.selected.min(app.project.mixer.len() - 1);
    let p = ui.painter();
    p.rect_filled(rect, 0.0, theme::MIX_PANEL);

    // Track name (editable).
    let name_r = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(rect.width() - 12.0, 26.0));
    let mut name = app.project.mixer[t].name.clone();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_r));
    let resp = child.add(egui::TextEdit::singleline(&mut name).font(theme::font(15.0)).desired_width(name_r.width()));
    if resp.gained_focus() {
        app.checkpoint();
    }
    if name != app.project.mixer[t].name {
        app.project.mixer[t].name = name;
        app.changed();
    }

    // Effect slots.
    let mut y = name_r.bottom() + 8.0;
    let mut action: Option<(usize, &'static str)> = None;
    let mut add: Option<(usize, Effect)> = None;
    let mut add_plugin: Option<plugin::PluginInfo> = None;
    for s in 0..SLOTS {
        let row = Rect::from_min_size(pos2(rect.left() + 4.0, y), vec2(rect.width() - 8.0, 24.0));
        let id = Id::new(("slot", t, s));
        let resp = ui.interact(Rect::from_min_max(row.min, pos2(row.right() - 50.0, row.bottom())), id, Sense::click());
        let slot = app.project.mixer[t].effects.get(s).cloned();
        let p = ui.painter();
        p.rect_filled(row, 1.0, if resp.hovered() { theme::lighten(theme::SLOT_ROW, 0.05) } else { theme::SLOT_ROW });
        widgets::paint_icon(p, Rect::from_center_size(pos2(row.left() + 10.0, row.center().y), vec2(9.0, 9.0)), Icon::ArrowRight, theme::TEXT_DIM);
        match &slot {
            Some(sl) => {
                p.text(pos2(row.left() + 22.0, row.center().y), Align2::LEFT_CENTER, sl.effect.name(), theme::font(15.0), if sl.enabled { theme::TEXT } else { theme::TEXT_DIM });
            }
            None => {
                p.text(pos2(row.left() + 22.0, row.center().y), Align2::LEFT_CENTER, format!("Slot {}", s + 1), egui::FontId::new(14.0, egui::FontFamily::Proportional), theme::TEXT_FAINT);
            }
        }
        let led_c = pos2(row.right() - 12.0, row.center().y);
        widgets::paint_led(p, led_c, 4.0, slot.as_ref().map(|s| s.enabled).unwrap_or(true));
        // Dry/wet mix knob.
        let knob_r = Rect::from_center_size(pos2(row.right() - 34.0, row.center().y), vec2(18.0, 18.0));
        match &slot {
            Some(sl) => {
                let mut m = sl.mix;
                let kr = widgets::knob_at(ui, knob_r, id.with("mix"), &mut m, 0.0..=1.0, 1.0, "Mix");
                if kr.drag_started() {
                    app.checkpoint();
                }
                if kr.changed() {
                    app.project.mixer[t].effects[s].mix = m;
                    app.changed();
                }
            }
            None => widgets::paint_knob(ui.painter(), knob_r.center(), 9.0, 1.0, false),
        }
        if slot.is_some() && ui.interact(Rect::from_center_size(led_c, vec2(16.0, 16.0)), id.with("led"), Sense::click()).clicked() {
            action = Some((s, "toggle"));
        }

        if slot.is_some() {
            if resp.clicked() {
                action = Some((s, "open"));
            }
            resp.context_menu(|ui| {
                for (label, a) in [("Open editor", "open"), ("Move up", "up"), ("Move down", "down"), ("Delete", "delete")] {
                    if ui.button(label).clicked() {
                        action = Some((s, a));
                        ui.close_menu();
                    }
                }
            });
        } else {
            let popup = id.with("add");
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup::popup_below_widget(ui, popup, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(160.0);
                for e in Effect::defaults() {
                    if ui.button(e.name()).clicked() {
                        add = Some((s, e));
                    }
                }
                let fx_plugins: Vec<plugin::PluginInfo> = app.plugins.db.effects().cloned().collect();
                if !fx_plugins.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("VST3").color(theme::TEXT_DIM));
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for info in &fx_plugins {
                            if ui.button(&info.name).on_hover_text(&info.vendor).clicked() {
                                add_plugin = Some(info.clone());
                            }
                        }
                    });
                }
            });
        }
        y += 25.0;
    }

    if let Some(info) = add_plugin {
        app.add_plugin_effect(t, &info);
    }
    if let Some((_, e)) = add {
        app.checkpoint();
        app.project.mixer[t].effects.push(EffectSlot { mix: 1.0, fl: None, enabled: true, effect: e });
        let idx = app.project.mixer[t].effects.len() - 1;
        app.mixer_ui.open_fx.push((t, idx));
        app.changed();
    }
    if let Some((s, a)) = action {
        match a {
            "open" => {
                let slot = app.project.mixer[t].effects.get(s).cloned();
                match slot.map(|sl| sl.effect) {
                    Some(Effect::Plugin(PluginParams { instance: Some(inst), info, .. })) => {
                        let title = format!("{} ({})", info.name, app.project.mixer[t].name);
                        app.open_plugin_editor(inst, &title);
                    }
                    _ => {
                        if !app.mixer_ui.open_fx.contains(&(t, s)) {
                            app.mixer_ui.open_fx.push((t, s));
                        }
                    }
                }
            }
            _ => {
                app.checkpoint();
                let fx = &mut app.project.mixer[t].effects;
                match a {
                    "toggle" => fx[s].enabled = !fx[s].enabled,
                    "up" if s > 0 => fx.swap(s, s - 1),
                    "down" if s + 1 < fx.len() => fx.swap(s, s + 1),
                    "delete" => {
                        fx.remove(s);
                        app.mixer_ui.open_fx.retain(|&(tt, _)| tt != t);
                    }
                    _ => {}
                }
                app.changed();
            }
        }
    }

    // Built-in EQ.
    let eq_r = Rect::from_min_max(pos2(rect.left() + 4.0, y + 6.0), pos2(rect.right() - 4.0, y + 96.0));
    let p = ui.painter();
    p.rect_filled(eq_r, 1.0, Color32::from_rgb(0x2a, 0x2d, 0x2f));
    let curve = Rect::from_min_max(eq_r.min + vec2(6.0, 6.0), pos2(eq_r.right() - 110.0, eq_r.bottom() - 26.0));
    p.rect_filled(curve, 1.0, theme::METER_BG);
    let eq = app.project.mixer[t].eq;
    let pts: Vec<egui::Pos2> = (0..=40)
        .map(|i| {
            let f = i as f32 / 40.0;
            let w = |c: f32, s: f32| (-((f - c) / s).powi(2)).exp();
            let g = eq[0] * (1.0 - f).powi(3) + eq[1] * w(0.5, 0.18) + eq[2] * f.powi(3);
            pos2(curve.left() + f * curve.width(), curve.center().y - g / 18.0 * curve.height() * 0.45)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5_f32, theme::TEXT)));
    p.text(pos2(eq_r.left() + 8.0, eq_r.bottom() - 12.0), Align2::LEFT_CENTER, "Equalizer", theme::font(16.0), theme::TEXT_DIM);
    let mut new_eq = eq;
    let mut started = false;
    for (i, label) in ["Low", "Mid", "High"].iter().enumerate() {
        let kr = Rect::from_center_size(pos2(eq_r.right() - 90.0 + i as f32 * 34.0, eq_r.top() + 30.0), vec2(26.0, 26.0));
        let resp = widgets::knob_at(ui, kr, Id::new(("eq", t, i)), &mut new_eq[i], -18.0..=18.0, 0.0, &format!("{label} (dB)"));
        started |= resp.drag_started();
        ui.painter().text(pos2(kr.center().x, kr.bottom() + 8.0), Align2::CENTER_CENTER, *label, theme::font(11.0), theme::TEXT_DIM);
    }
    if started {
        app.checkpoint();
    }
    if new_eq != eq {
        app.project.mixer[t].eq = new_eq;
        app.changed();
    }

    // Output.
    let out_r = Rect::from_min_max(pos2(rect.left() + 4.0, eq_r.bottom() + 8.0), pos2(rect.right() - 4.0, eq_r.bottom() + 32.0));
    if out_r.bottom() < rect.bottom() {
        let p = ui.painter();
        p.rect_filled(out_r, 1.0, theme::SLOT_ROW);
        let out = app.project.mixer[t].output;
        let text = if t == 0 { "Out 1 - Out 2".to_string() } else if out == 0 { "Master".to_string() } else { format!("Insert {out} - {}", app.project.mixer[out].name) };
        widgets::paint_icon(p, Rect::from_center_size(pos2(out_r.left() + 12.0, out_r.center().y), vec2(12.0, 12.0)), Icon::ArrowRight, theme::TEXT);
        p.text(pos2(out_r.left() + 26.0, out_r.center().y), Align2::LEFT_CENTER, text, theme::font(15.0), theme::TEXT);
    }

    // Sends: further destinations with their own level (FL routes to several inserts).
    let sends_r = Rect::from_min_max(pos2(rect.left() + 4.0, out_r.bottom() + 6.0), pos2(rect.right() - 4.0, rect.bottom()));
    if t > 0 && sends_r.height() > 24.0 {
        let mut sends = app.project.mixer[t].sends.clone();
        let mut remove = None;
        let mut edited = false;
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sends_r), |ui| {
            for (i, s) in sends.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let name = app.project.mixer.get(s.to).map(|m| m.name.clone()).unwrap_or_default();
                    ui.label(egui::RichText::new(format!("\u{2192} {name}")).color(theme::TEXT).size(13.0));
                    let mut db = 20.0 * s.gain.max(1e-6).log10();
                    let r = ui.add(egui::DragValue::new(&mut db).range(-60.0..=6.0).speed(0.2).suffix(" dB").custom_formatter(|v, _| if v <= -59.9 { "-inf".into() } else { format!("{v:.1} dB") }));
                    if r.changed() {
                        s.gain = if db <= -59.9 { 0.0 } else { 10f32.powf(db / 20.0) };
                        edited = true;
                    }
                    if ui.small_button("\u{2715}").on_hover_text("Remove send").clicked() {
                        remove = Some(i);
                    }
                });
            }
            ui.menu_button(egui::RichText::new("+ Send").color(theme::TEXT_DIM).size(13.0), |ui| {
                for (d, m) in app.project.mixer.iter().enumerate() {
                    let taken = d == t || d == app.project.mixer[t].output || sends.iter().any(|s| s.to == d);
                    if !taken && ui.button(format!("{d} - {}", m.name)).clicked() {
                        sends.push(MixerSend { to: d, gain: 1.0 });
                        edited = true;
                        ui.close_menu();
                    }
                }
            });
        });
        if let Some(i) = remove {
            sends.remove(i);
            edited = true;
        }
        if edited {
            app.checkpoint();
            app.project.mixer[t].sends = sends;
            app.changed();
        }
    }
}

fn effect_windows(app: &mut App, ctx: &egui::Context) {
    let open = app.mixer_ui.open_fx.clone();
    let mut keep = Vec::new();
    for (t, s) in open {
        let Some(slot) = app.project.mixer.get(t).and_then(|m| m.effects.get(s)).cloned() else { continue };
        let mut is_open = true;
        let title = format!("{} ({} - Slot {})", slot.effect.name(), app.project.mixer[t].name, s + 1);
        let mut effect = slot.effect.clone();
        let mut started = false;
        widgets::fl_window(ctx, &format!("fx_{t}_{s}"), &title, &mut is_open, [420.0 + s as f32 * 20.0, 200.0 + s as f32 * 20.0], [360.0, 130.0], |ui| {
            started = effect_params(ui, &mut effect, t * 64 + s);
        });
        if effect != slot.effect {
            if started {
                app.checkpoint();
            }
            app.project.mixer[t].effects[s].effect = effect;
            app.changed();
        }
        if is_open {
            keep.push((t, s));
        }
    }
    app.mixer_ui.open_fx = keep;
}

/// A labelled knob for plugin windows. Returns true if an edit started.
fn param(ui: &mut Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32, label: &str, unit: &str) -> bool {
    let mut started = false;
    ui.vertical(|ui| {
        ui.set_width(62.0);
        ui.vertical_centered(|ui| {
            let r = widgets::knob(ui, v, range, default, 36.0, label);
            started = r.drag_started();
            ui.label(egui::RichText::new(label).font(theme::font(12.5)).color(theme::TEXT_DIM));
            ui.label(egui::RichText::new(format!("{:.2}{unit}", *v)).font(theme::font(12.0)).color(theme::TEXT));
        });
    });
    started
}

fn effect_params(ui: &mut Ui, e: &mut Effect, id: usize) -> bool {
    let mut s = false;
    ui.horizontal(|ui| match e {
        Effect::Delay { beats, feedback, mix } => {
            s |= param(ui, beats, 0.0625..=2.0, 0.75, "Time", " beats");
            s |= param(ui, feedback, 0.0..=0.95, 0.4, "Feedback", "");
            s |= param(ui, mix, 0.0..=1.0, 0.3, "Mix", "");
        }
        Effect::Reverb { room, damping, mix } => {
            s |= param(ui, room, 0.0..=1.0, 0.7, "Size", "");
            s |= param(ui, damping, 0.0..=1.0, 0.4, "Damping", "");
            s |= param(ui, mix, 0.0..=1.0, 0.25, "Mix", "");
        }
        Effect::Filter { kind, cutoff, resonance } => {
            let before = *kind;
            ui.vertical(|ui| {
                for k in [FilterKind::Lowpass, FilterKind::Highpass, FilterKind::Bandpass] {
                    ui.radio_value(kind, k, format!("{k:?}"));
                }
            });
            s |= *kind != before;
            let mut log = cutoff.log10();
            s |= param(ui, &mut log, 1.3..=4.3, 3.3, "Cutoff", "");
            *cutoff = 10f32.powf(log);
            s |= param(ui, resonance, 0.0..=1.0, 0.3, "Reso", "");
            let _ = id;
        }
        Effect::Distortion { drive, mix } => {
            s |= param(ui, drive, 1.0..=30.0, 4.0, "Drive", "");
            s |= param(ui, mix, 0.0..=1.0, 0.5, "Mix", "");
        }
        Effect::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup_db } => {
            s |= param(ui, threshold_db, -60.0..=0.0, -18.0, "Threshold", " dB");
            s |= param(ui, ratio, 1.0..=20.0, 4.0, "Ratio", "");
            s |= param(ui, attack_ms, 0.1..=200.0, 10.0, "Attack", " ms");
            s |= param(ui, release_ms, 5.0..=1000.0, 120.0, "Release", " ms");
            s |= param(ui, makeup_db, 0.0..=24.0, 4.0, "Gain", " dB");
        }
        Effect::ParamEq { bands, main_db } => {
            ui.vertical(|ui| {
                s |= eq_editor(ui, bands, main_db, id);
            });
        }
        Effect::Limiter { gain_db, ceiling_db, release_ms, comp_threshold_db, comp_ratio, sidechain } => {
            s |= param(ui, gain_db, -24.0..=24.0, 0.0, "Gain", " dB");
            s |= param(ui, ceiling_db, -24.0..=6.0, 0.0, "Ceiling", " dB");
            s |= param(ui, release_ms, 1.0..=2000.0, 500.0, "Release", " ms");
            s |= param(ui, comp_threshold_db, -60.0..=0.0, 0.0, "Comp thres", " dB");
            s |= param(ui, comp_ratio, 1.0..=20.0, 3.0, "Ratio", ":1");
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Sidechain").size(11.0));
                let r = ui
                    .add(egui::DragValue::new(sidechain).range(0..=16).speed(0.05).custom_formatter(|v, _| if v < 0.5 { "--".into() } else { format!("{v:.0}") }))
                    .on_hover_text("The compressor listens to the n-th mixer track routed into this one (-- = its own input)");
                s |= r.changed();
            });
        }
        Effect::Reverb2 { low_cut, high_cut, predelay_ms, size, decay_s, damping_hz, width, dry, early, wet } => {
            s |= param(ui, low_cut, 10.0..=2000.0, 75.0, "Low cut", " Hz");
            s |= param(ui, high_cut, 500.0..=22100.0, 22100.0, "High cut", " Hz");
            s |= param(ui, predelay_ms, 0.0..=300.0, 0.0, "Predelay", " ms");
            s |= param(ui, size, 0.01..=1.0, 1.0, "Size", "");
            s |= param(ui, decay_s, 0.1..=20.0, 1.5, "Decay", " s");
            s |= param(ui, damping_hz, 500.0..=22100.0, 4000.0, "Damping", " Hz");
            s |= param(ui, width, 0.0..=2.0, 1.0, "Width", "");
            s |= param(ui, dry, 0.0..=1.25, 1.0, "Dry", "");
            s |= param(ui, early, 0.0..=1.25, 0.5, "Early", "");
            s |= param(ui, wet, 0.0..=1.25, 0.5, "Wet", "");
        }
        Effect::Balance { gain, pan } => {
            s |= param(ui, gain, 0.0..=1.25, 1.0, "Volume", "");
            s |= param(ui, pan, -1.0..=1.0, 0.0, "Pan", "");
        }
        Effect::StereoEnhancer { gain, pan, separation, phase_offset_ms, invert } => {
            s |= param(ui, gain, 0.0..=1.25, 1.0, "Volume", "");
            s |= param(ui, pan, -1.0..=1.0, 0.0, "Pan", "");
            s |= param(ui, separation, -1.0..=1.0, 0.0, "Separation", "");
            s |= param(ui, phase_offset_ms, -20.0..=20.0, 0.0, "Phase", " ms");
            ui.vertical(|ui| {
                let before = *invert;
                for (v, label) in [(0u8, "No invert"), (1, "Invert left"), (2, "Invert right")] {
                    ui.radio_value(invert, v, label);
                }
                s |= *invert != before;
            });
        }
        Effect::TransientShaper { attack, release, gain_db } => {
            s |= param(ui, attack, -1.0..=1.0, 0.0, "Attack", "");
            s |= param(ui, release, -1.0..=1.0, 0.0, "Release", "");
            s |= param(ui, gain_db, -12.0..=12.0, 0.0, "Gain", " dB");
        }
        Effect::SoftClipper { threshold, post_gain } => {
            s |= param(ui, threshold, 0.01..=1.0, 100.0 / 127.0, "Threshold", "");
            s |= param(ui, post_gain, 0.0..=1.25, 1.0, "Post gain", "");
        }
        Effect::Soundgoodizer { mode, amount } => {
            ui.vertical(|ui| {
                let before = *mode;
                for (v, label) in [(0u8, "A"), (1, "B"), (2, "C"), (3, "D")] {
                    ui.radio_value(mode, v, label);
                }
                s |= *mode != before;
            });
            s |= param(ui, amount, 0.0..=1.0, 0.5, "Amount", "");
        }
        Effect::MultiFilter { cutoff, resonance, low, band, high } => {
            s |= param(ui, cutoff, 20.0..=20000.0, 2000.0, "Cutoff", " Hz");
            s |= param(ui, resonance, 0.0..=1.0, 0.2, "Reso", "");
            s |= param(ui, low, 0.0..=1.0, 1.0, "Low pass", "");
            s |= param(ui, band, 0.0..=1.0, 0.0, "Band pass", "");
            s |= param(ui, high, 0.0..=1.0, 0.0, "High pass", "");
        }
        Effect::StereoShaper { l_from_l, l_from_r, r_from_l, r_from_r } => {
            s |= param(ui, l_from_l, -1.0..=1.0, 1.0, "L from L", "");
            s |= param(ui, l_from_r, -1.0..=1.0, 0.0, "L from R", "");
            s |= param(ui, r_from_l, -1.0..=1.0, 0.0, "R from L", "");
            s |= param(ui, r_from_r, -1.0..=1.0, 1.0, "R from R", "");
        }
        Effect::AutoPan { gain, pan, lfo_amount, lfo_hz } => {
            s |= param(ui, gain, 0.0..=1.25, 1.0, "Volume", "");
            s |= param(ui, pan, -1.0..=1.0, 0.0, "Pan", "");
            s |= param(ui, lfo_amount, 0.0..=1.0, 0.5, "LFO amount", "");
            s |= param(ui, lfo_hz, 0.01..=20.0, 1.0, "LFO speed", " Hz");
        }
        Effect::FastDist { pre, threshold, kind, mix, post } => {
            s |= param(ui, pre, 0.0..=2.0, 1.0, "Pre", "");
            s |= param(ui, threshold, 0.05..=1.0, 0.5, "Threshold", "");
            ui.vertical(|ui| {
                let before = *kind;
                ui.radio_value(kind, 0, "Type A");
                ui.radio_value(kind, 1, "Type B");
                s |= *kind != before;
            });
            s |= param(ui, mix, 0.0..=1.0, 1.0, "Mix", "");
            s |= param(ui, post, 0.0..=1.5, 1.0, "Post", "");
        }
        Effect::Overdrive { drive, color, post } => {
            s |= param(ui, drive, 1.0..=91.0, 4.0, "Drive", "");
            s |= param(ui, color, 0.0..=1.0, 0.5, "Color", "");
            s |= param(ui, post, 0.0..=1.5, 1.0, "Post gain", "");
        }
        Effect::Chorus { delay_ms, depth_ms, rate_hz, mix } => {
            s |= param(ui, delay_ms, 1.0..=40.0, 10.0, "Delay", " ms");
            s |= param(ui, depth_ms, 0.0..=10.0, 3.0, "Depth", " ms");
            s |= param(ui, rate_hz, 0.01..=10.0, 0.8, "Speed", " Hz");
            s |= param(ui, mix, 0.0..=1.0, 0.5, "Mix", "");
        }
        Effect::Plugin(p) => {
            ui.label(format!("{} by {} - open its editor from the mixer slot.", p.info.name, p.info.vendor));
        }
        Effect::Eq3 { low_db, mid_db, high_db } => {
            s |= param(ui, low_db, -18.0..=18.0, 0.0, "Low", " dB");
            s |= param(ui, mid_db, -18.0..=18.0, 0.0, "Mid", " dB");
            s |= param(ui, high_db, -18.0..=18.0, 0.0, "High", " dB");
        }
    });
    s
}

/// Approximate magnitude response (dB) of an EQ band at `f`.
fn band_response(b: &EqBand, f: f32) -> f32 {
    let x = (f / b.freq).log2();
    let width = (1.0 / b.q.max(0.1)).clamp(0.05, 6.0);
    match b.kind {
        EqBandKind::Off => 0.0,
        EqBandKind::Peak => b.gain_db * (-(x / width * 1.8).powi(2)).exp(),
        EqBandKind::LowShelf => b.gain_db / (1.0 + (x * 3.0).exp()),
        EqBandKind::HighShelf => b.gain_db / (1.0 + (-x * 3.0).exp()),
        EqBandKind::LowPass => if x > 0.0 { -12.0 * x } else { 0.0 },
        EqBandKind::HighPass => if x < 0.0 { 12.0 * x } else { 0.0 },
        EqBandKind::BandPass => -6.0 * x.abs() / width,
        EqBandKind::Notch => -30.0 * (-(x / width * 4.0).powi(2)).exp(),
    }
}

/// Response curve plus a table of bands. Returns true if an edit started.
fn eq_editor(ui: &mut Ui, bands: &mut [EqBand; 7], main_db: &mut f32, id: usize) -> bool {
    let mut started = false;
    let (rect, _) = ui.allocate_exact_size(vec2(560.0, 150.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, theme::METER_BG);
    let fx = |f: f32| rect.left() + ((f / 20.0).ln() / 1000f32.ln()) * rect.width();
    let dby = |db: f32| rect.center().y - db / 18.0 * rect.height() * 0.45;
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        p.line_segment([pos2(fx(f), rect.top()), pos2(fx(f), rect.bottom())], Stroke::new(1.0_f32, theme::GRID_A));
        let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(pos2(fx(f) + 2.0, rect.bottom() - 8.0), Align2::LEFT_CENTER, label, theme::font(10.0), theme::TEXT_FAINT);
    }
    p.line_segment([pos2(rect.left(), dby(0.0)), pos2(rect.right(), dby(0.0))], Stroke::new(1.0_f32, theme::TEXT_FAINT));
    let pts: Vec<egui::Pos2> = (0..=200)
        .map(|i| {
            let f = 20.0 * 1000f32.powf(i as f32 / 200.0);
            let db: f32 = bands.iter().map(|b| band_response(b, f)).sum::<f32>() + *main_db;
            pos2(fx(f), dby(db.clamp(-24.0, 24.0)))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, theme::PR_NOTE)));
    for (i, b) in bands.iter().enumerate() {
        if b.kind != EqBandKind::Off {
            let c = pos2(fx(b.freq), dby(if matches!(b.kind, EqBandKind::Peak | EqBandKind::LowShelf | EqBandKind::HighShelf) { b.gain_db } else { 0.0 }));
            p.circle_filled(c, 7.0, theme::BUTTON);
            p.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), theme::font(11.0), theme::TEXT_BRIGHT);
        }
    }

    egui::Grid::new(("eq_bands", id)).num_columns(5).spacing([8.0, 2.0]).show(ui, |ui| {
        for h in ["Band", "Type", "Freq", "Gain", "Q"] {
            ui.label(egui::RichText::new(h).color(theme::TEXT_DIM));
        }
        ui.end_row();
        for (i, b) in bands.iter_mut().enumerate() {
            ui.label(format!("{}", i + 1));
            let before = b.kind;
            egui::ComboBox::from_id_salt(("eq_kind", id, i)).width(90.0).selected_text(b.kind.name()).show_ui(ui, |ui| {
                for k in EqBandKind::ALL {
                    ui.selectable_value(&mut b.kind, k, k.name());
                }
            });
            started |= b.kind != before;
            let speed = b.freq * 0.01;
            let r = ui.add(egui::DragValue::new(&mut b.freq).range(20.0..=20000.0).speed(speed).suffix(" Hz").max_decimals(0));
            started |= r.drag_started() || r.gained_focus();
            let r = ui.add(egui::DragValue::new(&mut b.gain_db).range(-18.0..=18.0).speed(0.1).suffix(" dB").max_decimals(1));
            started |= r.drag_started() || r.gained_focus();
            let r = ui.add(egui::DragValue::new(&mut b.q).range(0.1..=30.0).speed(0.01).max_decimals(2));
            started |= r.drag_started() || r.gained_focus();
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        ui.label("Main level");
        let r = ui.add(egui::DragValue::new(main_db).range(-18.0..=18.0).speed(0.1).suffix(" dB"));
        started |= r.drag_started() || r.gained_focus();
    });
    started
}
