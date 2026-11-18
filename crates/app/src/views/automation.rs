//! Automation clip editor: add, move and delete points; choose the target.

use super::{describe_target, snap_round};
use crate::app::App;
use crate::theme;
use crate::widgets;
use goaty_engine::*;
use egui::{pos2, vec2, Align2, Rect, Sense, Stroke, Ui};

pub struct State {
    pub open: Option<usize>,
    drag: Option<usize>,
    zoom: f32,
    scroll: f32,
}

impl Default for State {
    fn default() -> Self {
        Self { open: None, drag: None, zoom: 0.5, scroll: 0.0 }
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(ch) = app.automation_ui.open else { return };
    let is_auto = matches!(app.project.channels.get(ch).map(|c| &c.instrument), Some(Instrument::Automation(_)));
    if !is_auto {
        app.automation_ui.open = None;
        return;
    }
    let mut open = true;
    let title = format!("Automation - {}", app.project.channels[ch].name);
    widgets::fl_window(ctx, "automation_editor", &title, &mut open, [380.0, 220.0], [820.0, 380.0], |ui| body(app, ui, ch));
    if !open {
        app.automation_ui.open = None;
    }
}

/// Targets the user can pick for a clip.
fn target_options(p: &Project) -> Vec<(String, Option<AutoTarget>)> {
    let mut v = vec![("(none)".to_string(), None), ("Tempo".to_string(), Some(AutoTarget::Tempo)), ("Main volume".to_string(), Some(AutoTarget::MainVolume))];
    for (i, c) in p.channels.iter().enumerate() {
        if matches!(c.instrument, Instrument::Automation(_)) {
            continue;
        }
        v.push((format!("{} - Volume", c.name), Some(AutoTarget::ChannelVolume(i))));
        v.push((format!("{} - Panning", c.name), Some(AutoTarget::ChannelPan(i))));
        v.push((format!("{} - Pitch", c.name), Some(AutoTarget::ChannelPitch(i))));
    }
    for (i, t) in p.mixer.iter().enumerate() {
        v.push((format!("Mixer {} - Volume", t.name), Some(AutoTarget::MixerVolume(i))));
        v.push((format!("Mixer {} - Panning", t.name), Some(AutoTarget::MixerPan(i))));
    }
    v
}

fn body(app: &mut App, ui: &mut Ui, ch: usize) {
    let Instrument::Automation(auto) = &app.project.channels[ch].instrument else { return };
    let current = auto.target;

    // Target picker.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Controls").color(theme::TEXT_DIM));
        let mut chosen = None;
        egui::ComboBox::from_id_salt("auto_target").width(320.0).selected_text(describe_target(&app.project, current)).show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (label, t) in target_options(&app.project) {
                    if ui.selectable_label(t == current, label).clicked() {
                        chosen = Some(t);
                    }
                }
            });
        });
        if let Some(t) = chosen {
            if t != current {
                app.checkpoint();
                if let Instrument::Automation(a) = &mut app.project.channels[ch].instrument {
                    a.target = t;
                }
                app.changed();
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("Left: add / drag points   Right: delete   Ctrl+wheel: zoom").color(theme::TEXT_DIM).size(12.5));
    });

    let size = vec2(ui.available_width(), ui.available_height().max(180.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let grid = rect.shrink2(vec2(0.0, 8.0));
    let st = &mut app.automation_ui;
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.raw_scroll_delta, i.zoom_delta()));
        if zoom != 1.0 {
            st.zoom = (st.zoom * zoom).clamp(0.02, 8.0);
        }
        st.scroll = (st.scroll - (scroll.x + scroll.y) / st.zoom).max(0.0);
    }
    let (zoom, scroll) = (st.zoom, st.scroll);
    let x_of = |t: f32| grid.left() + (t - scroll) * zoom;
    let t_of = |x: f32| ((x - grid.left()) / zoom + scroll).max(0.0);
    let y_of = |v: f32| grid.bottom() - v * grid.height();
    let v_of = |y: f32| ((grid.bottom() - y) / grid.height()).clamp(0.0, 1.0);
    let snap = app.piano.snap;

    // Interaction.
    let (pointer, pp, pd, sp) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.secondary_pressed()));
    if let Some(pos) = pointer {
        let Instrument::Automation(a) = &app.project.channels[ch].instrument else { return };
        let hit = a.points.iter().position(|p| pos2(x_of(p.tick as f32), y_of(p.value)).distance(pos) < 8.0);
        if resp.hovered() && grid.expand(8.0).contains(pos) {
            if pp {
                app.checkpoint();
                let idx = match hit {
                    Some(i) => i,
                    None => {
                        let tick = snap_round(t_of(pos.x), snap);
                        let value = v_of(pos.y);
                        let Instrument::Automation(a) = &mut app.project.channels[ch].instrument else { return };
                        let i = a.points.iter().position(|p| p.tick > tick).unwrap_or(a.points.len());
                        a.points.insert(i, AutoPoint { tick, value, tension: 0.0 });
                        app.changed();
                        i
                    }
                };
                app.automation_ui.drag = Some(idx);
            }
            if sp {
                if let Some(i) = hit {
                    app.checkpoint();
                    if let Instrument::Automation(a) = &mut app.project.channels[ch].instrument {
                        if a.points.len() > 1 {
                            a.points.remove(i);
                        }
                    }
                    app.changed();
                }
            }
        }
        if let (Some(i), true) = (app.automation_ui.drag, pd) {
            if let Instrument::Automation(a) = &mut app.project.channels[ch].instrument {
                if i < a.points.len() {
                    // Keep points ordered: a point can't pass its neighbours.
                    let lo = if i > 0 { a.points[i - 1].tick } else { 0 };
                    let hi = a.points.get(i + 1).map(|p| p.tick).unwrap_or(u32::MAX);
                    let tick = if i == 0 { 0 } else { snap_round(t_of(pos.x), snap).clamp(lo, hi) };
                    let value = v_of(pos.y);
                    if a.points[i].tick != tick || (a.points[i].value - value).abs() > 1e-4 {
                        a.points[i].tick = tick;
                        a.points[i].value = value;
                        app.changed();
                    }
                }
            }
        }
    }
    if !pd {
        app.automation_ui.drag = None;
    }

    // Drawing.
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::GRID_B);
    let bar = app.project.bar_ticks();
    let beat = app.project.beat_ticks();
    let mut t = (scroll / beat as f32).floor() as u32 * beat;
    while x_of(t as f32) < rect.right() {
        let x = x_of(t as f32);
        let col = if t % bar == 0 { theme::GRID_BAR } else { theme::GRID_BEAT };
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, col));
        if t % bar == 0 {
            p.text(pos2(x + 3.0, rect.top() + 8.0), Align2::LEFT_CENTER, format!("{}", t / bar + 1), theme::font(12.0), theme::TEXT_DIM);
        }
        t += beat;
    }
    for v in [0.25, 0.5, 0.75] {
        p.line_segment([pos2(rect.left(), y_of(v)), pos2(rect.right(), y_of(v))], Stroke::new(1.0_f32, theme::GRID_LINE));
    }
    let Instrument::Automation(a) = &app.project.channels[ch].instrument else { return };
    let color = theme::PR_NOTE;
    let curve = Rect::from_min_max(pos2(grid.left(), grid.top()), pos2(grid.right(), grid.bottom()));
    super::paint_auto_curve(&p, curve, a, scroll, grid.width() / zoom, color);
    for (i, pt) in a.points.iter().enumerate() {
        let c = pos2(x_of(pt.tick as f32), y_of(pt.value));
        let active = app.automation_ui.drag == Some(i);
        p.circle_filled(c, if active { 6.0 } else { 5.0 }, theme::lighten(color, 0.3));
        p.circle_stroke(c, 5.0, Stroke::new(1.0_f32, theme::darken(color, 0.5)));
    }
    if let Some(i) = app.automation_ui.drag {
        if let Some(pt) = a.points.get(i) {
            let c = pos2(x_of(pt.tick as f32), y_of(pt.value));
            p.text(c + vec2(8.0, -10.0), Align2::LEFT_BOTTOM, format!("{:.0}%", pt.value * 100.0), theme::font(12.0), theme::TEXT_BRIGHT);
        }
    }
}

/// Creates a new automation channel with a flat curve and opens it.
pub fn create_clip(app: &mut App) {
    let len = app.project.bar_ticks() * 4;
    let auto = AutomationParams {
        points: vec![AutoPoint { tick: 0, value: 0.5, tension: 0.0 }, AutoPoint { tick: len, value: 0.5, tension: 0.0 }],
        target: None,
    };
    let n = app.project.channels.iter().filter(|c| matches!(c.instrument, Instrument::Automation(_))).count();
    let idx = app.add_channel(Channel::new(format!("Automation {}", n + 1), Instrument::Automation(auto)));
    app.automation_ui.open = Some(idx);
    app.playlist_ui.paint_auto = Some(idx);
}
