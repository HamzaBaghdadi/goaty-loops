//! Custom widgets drawn to match FL Studio's look: knobs, faders, meters,
//! LEDs, painted toolbar icons and frameless child windows.

use crate::theme;
use egui::{pos2, vec2, Color32, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};
use std::f32::consts::PI;
use std::ops::RangeInclusive;

/// A rotary knob. Drag vertically (Shift/Ctrl for fine), scroll, or double-click to reset.
pub fn knob(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, default: f32, size: f32, label: &str) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let id = ui.id().with(("knob", rect.min.x as i32, rect.min.y as i32));
    knob_at(ui, rect, id, value, range, default, label)
}

/// A knob at an absolute position.
pub fn knob_at(ui: &mut Ui, rect: Rect, id: Id, value: &mut f32, range: RangeInclusive<f32>, default: f32, label: &str) -> Response {
    let size = rect.width().min(rect.height());
    let mut resp = ui.interact(rect, id, Sense::click_and_drag());
    let (min, max) = (*range.start(), *range.end());
    let span = max - min;
    let old = *value;
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
        *value -= resp.drag_delta().y * span / if fine { 1000.0 } else { 150.0 };
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            *value += scroll.signum() * span / 50.0;
        }
    }
    if resp.double_clicked() {
        *value = default;
    }
    *value = value.clamp(min, max);
    if *value != old {
        resp.mark_changed();
    }
    paint_knob(ui.painter(), rect.center(), size * 0.5, (*value - min) / span.max(1e-9), resp.hovered() || resp.dragged());
    resp.on_hover_text(format!("{label}: {:.2}", *value))
}

pub fn paint_knob(p: &Painter, c: Pos2, r: f32, t: f32, hot: bool) {
    p.circle_filled(c, r, theme::KNOB_RIM);
    p.circle_filled(c, r - 1.5, if hot { theme::lighten(theme::KNOB, 0.06) } else { theme::KNOB });
    // Soft top highlight.
    p.circle_filled(c - vec2(0.0, r * 0.25), r * 0.55, Color32::from_white_alpha(6));
    let a = PI * 0.75 + t.clamp(0.0, 1.0) * PI * 1.5;
    let dir = vec2(a.cos(), a.sin());
    p.line_segment([c + dir * r * 0.35, c + dir * (r - 3.0)], Stroke::new(2.0_f32, theme::TEXT));
}

/// A small round LED toggle.
pub fn led(ui: &mut Ui, on: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    paint_led(ui.painter(), rect.center(), size * 0.42, on);
    resp
}

pub fn paint_led(p: &Painter, c: Pos2, r: f32, on: bool) {
    p.circle_filled(c, r + 1.0, Color32::from_rgb(0x15, 0x17, 0x18));
    let col = if on { theme::LED_ON } else { theme::LED_OFF };
    p.circle_filled(c, r, col);
    if on {
        p.circle_filled(c - vec2(r * 0.3, r * 0.3), r * 0.35, theme::lighten(col, 0.55));
    }
}

/// Vertical peak meter mapping -60..+6 dB to the height.
pub fn paint_meter(p: &Painter, rect: Rect, level: f32) {
    p.rect_filled(rect, 0.0, theme::METER_BG);
    let db = 20.0 * level.max(1e-6).log10();
    let t = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
    if t <= 0.0 {
        return;
    }
    let h = rect.height() * t;
    let unity = rect.bottom() - rect.height() * (60.0 / 66.0);
    let top = rect.bottom() - h;
    let green_top = top.max(unity + rect.height() * 0.1);
    p.rect_filled(Rect::from_min_max(pos2(rect.left(), green_top), rect.max), 0.0, theme::METER_LOW);
    if top < green_top {
        let yellow_top = top.max(unity);
        p.rect_filled(Rect::from_min_max(pos2(rect.left(), yellow_top), pos2(rect.right(), green_top)), 0.0, theme::METER_MID);
        if top < unity {
            p.rect_filled(Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), unity)), 0.0, theme::METER_HIGH);
        }
    }
}

/// Fader position (0..1) <-> linear gain. 0.8 = unity, 1.0 = +5.6 dB (FL's volume law).
pub fn fader_to_gain(p: f32) -> f32 {
    goaty_engine::dsp::fader_to_gain(p)
}

pub fn gain_to_fader(g: f32) -> f32 {
    goaty_engine::dsp::gain_to_fl_volume(g) * 0.8
}

/// FL-style vertical fader controlling a linear gain.
pub fn fader(ui: &mut Ui, gain: &mut f32, rect: Rect, selected: bool, id: Id) -> Response {
    let mut resp = ui.interact(rect, id, Sense::click_and_drag());
    let mut pos = gain_to_fader(*gain);
    let old = pos;
    let travel = rect.height() - 26.0;
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
        pos -= resp.drag_delta().y / travel * if fine { 0.2 } else { 1.0 };
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            pos += scroll.signum() * 0.02;
        }
    }
    if resp.double_clicked() {
        pos = 0.8;
    }
    pos = pos.clamp(0.0, 1.0);
    if pos != old {
        *gain = fader_to_gain(pos);
        resp.mark_changed();
    }
    let p = ui.painter();
    let cx = rect.center().x;
    let top = rect.top() + 13.0;
    let track = Rect::from_min_max(pos2(cx - 1.5, rect.top()), pos2(cx + 1.5, rect.bottom()));
    p.rect_filled(track, 1.0, theme::FADER_TRACK);
    let y = top + travel * (1.0 - pos);
    if selected {
        p.rect_filled(Rect::from_min_max(pos2(cx - 1.0, y), pos2(cx + 1.0, rect.bottom())), 0.0, theme::SELECT_GREEN);
    }
    let handle = Rect::from_center_size(pos2(cx, y), vec2(17.0, 26.0));
    let (fill, groove) = if selected { (theme::SELECT_GREEN, theme::darken(theme::SELECT_GREEN, 0.45)) } else { (theme::FADER_HANDLE, Color32::from_rgb(0x2a, 0x2b, 0x2d)) };
    p.rect_filled(handle.translate(vec2(0.0, 2.0)), 3.0, Color32::from_black_alpha(90));
    p.rect_filled(handle, 3.0, fill);
    p.rect_filled(Rect::from_min_size(handle.min, vec2(handle.width(), 9.0)), 3.0, theme::lighten(fill, 0.12));
    for dy in [1.0, 5.0] {
        p.rect_filled(Rect::from_center_size(pos2(cx, y + dy), vec2(13.0, 2.0)), 0.0, groove);
    }
    let db = 20.0 * gain.max(1e-6).log10();
    resp.on_hover_text(format!("{db:+.1} dB"))
}

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    Record,
    Playlist,
    PianoRoll,
    ChannelRack,
    Mixer,
    Browser,
    Plus,
    ArrowRight,
    ArrowLeft,
    Folder,
    Audio,
    Project,
    Preset,
    Keyboard,
    Magnet,
    Close,
    Minimize,
    Pattern,
}

/// Paints a simple line icon centred in `rect`.
pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon, col: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let u = s / 16.0;
    let st = Stroke::new((1.4 * u).max(1.2), col);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(c + vec2(x0 * u, y0 * u), c + vec2(x1 * u, y1 * u));
    match icon {
        Icon::Play => {
            p.add(Shape::convex_polygon(vec![c + vec2(-4.0, -6.0) * u, c + vec2(6.0, 0.0) * u, c + vec2(-4.0, 6.0) * u], col, Stroke::NONE));
        }
        Icon::Pause => {
            p.rect_filled(r(-5.0, -6.0, -1.5, 6.0), 0.0, col);
            p.rect_filled(r(1.5, -6.0, 5.0, 6.0), 0.0, col);
        }
        Icon::Stop => {
            p.rect_filled(r(-5.0, -5.0, 5.0, 5.0), 1.0, col);
        }
        Icon::Record => {
            p.circle_filled(c, 6.0 * u, col);
        }
        Icon::Playlist => {
            for (i, w) in [(0, 9.0), (1, 6.0), (2, 11.0)] {
                let y = -5.0 + i as f32 * 5.0;
                p.rect_filled(r(-7.0, y - 1.5, -7.0 + w, y + 1.5), 0.0, col);
            }
            p.line_segment([c + vec2(-7.0, -8.0) * u, c + vec2(-7.0, 8.0) * u], st);
        }
        Icon::PianoRoll => {
            p.rect_stroke(r(-7.0, -7.0, 7.0, 7.0), 1.0, st, egui::StrokeKind::Inside);
            p.rect_filled(r(-4.0, -4.0, 1.0, -1.5), 0.0, col);
            p.rect_filled(r(-1.0, 0.0, 5.0, 2.5), 0.0, col);
            p.rect_filled(r(-5.0, 3.5, -1.0, 5.5), 0.0, col);
        }
        Icon::ChannelRack => {
            for i in 0..3 {
                let y = -5.0 + i as f32 * 5.0;
                p.rect_filled(r(-7.0, y - 1.8, -3.0, y + 1.8), 0.0, col);
                for j in 0..3 {
                    let x = -1.0 + j as f32 * 3.0;
                    p.rect_filled(r(x, y - 1.8, x + 2.0, y + 1.8), 0.0, theme::darken(col, if (i + j) % 2 == 0 { 0.0 } else { 0.45 }));
                }
            }
        }
        Icon::Mixer => {
            for (i, h) in [(0, 2.0), (1, -3.0), (2, 1.0)] {
                let x = -5.0 + i as f32 * 5.0;
                p.line_segment([c + vec2(x, -7.0) * u, c + vec2(x, 7.0) * u], st);
                p.rect_filled(r(x - 2.0, h - 1.5, x + 2.0, h + 1.5), 0.5, col);
            }
        }
        Icon::Browser => {
            p.rect_stroke(r(-7.0, -6.0, 7.0, 6.0), 1.0, st, egui::StrokeKind::Inside);
            p.line_segment([c + vec2(-2.0, -6.0) * u, c + vec2(-2.0, 6.0) * u], st);
        }
        Icon::Plus => {
            p.line_segment([c + vec2(-5.0, 0.0) * u, c + vec2(5.0, 0.0) * u], st);
            p.line_segment([c + vec2(0.0, -5.0) * u, c + vec2(0.0, 5.0) * u], st);
        }
        Icon::ArrowRight => {
            p.add(Shape::convex_polygon(vec![c + vec2(-3.0, -5.0) * u, c + vec2(3.0, 0.0) * u, c + vec2(-3.0, 5.0) * u], col, Stroke::NONE));
        }
        Icon::ArrowLeft => {
            p.add(Shape::convex_polygon(vec![c + vec2(3.0, -5.0) * u, c + vec2(-3.0, 0.0) * u, c + vec2(3.0, 5.0) * u], col, Stroke::NONE));
        }
        Icon::Folder => {
            let pts = vec![
                c + vec2(-7.0, -5.0) * u,
                c + vec2(-2.5, -5.0) * u,
                c + vec2(-1.0, -3.5) * u,
                c + vec2(7.0, -3.5) * u,
                c + vec2(7.0, 6.0) * u,
                c + vec2(-7.0, 6.0) * u,
                c + vec2(-7.0, -5.0) * u,
            ];
            p.add(Shape::line(pts, st));
            p.line_segment([c + vec2(-7.0, -1.0) * u, c + vec2(7.0, -1.0) * u], st);
        }
        Icon::Audio => {
            let hs = [2.0, 5.0, 3.0, 7.0, 4.0, 6.0, 2.0];
            for (i, h) in hs.iter().enumerate() {
                let x = -6.0 + i as f32 * 2.0;
                p.line_segment([c + vec2(x, -h) * u, c + vec2(x, *h) * u], st);
            }
        }
        Icon::Project => {
            p.rect_stroke(r(-6.0, -7.0, 6.0, 7.0), 1.0, st, egui::StrokeKind::Inside);
            p.add(Shape::convex_polygon(vec![c + vec2(-2.0, -3.0) * u, c + vec2(3.0, 0.0) * u, c + vec2(-2.0, 3.0) * u], col, Stroke::NONE));
        }
        Icon::Preset => {
            p.circle_stroke(c, 4.5 * u, st);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * 5.0 * u, c + d * 7.5 * u], st);
            }
        }
        Icon::Keyboard => {
            p.rect_stroke(r(-8.0, -6.0, 8.0, 6.0), 1.0, st, egui::StrokeKind::Inside);
            for i in 0..4 {
                let x = -5.0 + i as f32 * 3.5;
                p.rect_filled(r(x - 0.8, -6.0, x + 0.8, 1.0), 0.0, col);
            }
        }
        Icon::Magnet => {
            let pts: Vec<Pos2> = (0..=12).map(|i| {
                let a = PI * i as f32 / 12.0;
                c + vec2(-a.cos() * 5.0, a.sin() * 5.0 - 1.0) * u
            }).collect();
            p.add(Shape::line(pts, Stroke::new(st.width * 2.0, col)));
            p.line_segment([c + vec2(-5.0, -1.0) * u, c + vec2(-5.0, -6.0) * u], Stroke::new(st.width * 2.0, col));
            p.line_segment([c + vec2(5.0, -1.0) * u, c + vec2(5.0, -6.0) * u], Stroke::new(st.width * 2.0, col));
        }
        Icon::Close => {
            p.line_segment([c + vec2(-4.0, -4.0) * u, c + vec2(4.0, 4.0) * u], st);
            p.line_segment([c + vec2(4.0, -4.0) * u, c + vec2(-4.0, 4.0) * u], st);
        }
        Icon::Minimize => {
            p.line_segment([c + vec2(-4.0, 3.0) * u, c + vec2(4.0, 3.0) * u], st);
        }
        Icon::Pattern => {
            for i in 0..3 {
                let y = -4.0 + i as f32 * 4.0;
                p.rect_filled(r(-6.0, y - 1.2, 6.0, y + 1.2), 0.0, col);
            }
        }
    }
}

/// A square toolbar button with a painted icon.
pub fn tool_button(ui: &mut Ui, icon: Icon, active: bool, size: egui::Vec2, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let bg = if active {
        theme::lighten(theme::BUTTON, 0.12)
    } else if resp.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON
    };
    p.rect_filled(rect, 2.0, bg);
    p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
    let col = if active { theme::TEXT_BRIGHT } else { theme::TEXT };
    paint_icon(p, rect.shrink(rect.height() * 0.2), icon, col);
    resp.on_hover_text(tooltip)
}

/// A borderless icon-only button.
pub fn icon_button(ui: &mut Ui, icon: Icon, color: Color32, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let col = if resp.hovered() { theme::lighten(color, 0.3) } else { color };
    paint_icon(ui.painter(), rect, icon, col);
    resp.on_hover_text(tooltip)
}

/// Draws text rotated 90 degrees counter-clockwise, starting at `bottom_left`.
pub fn vertical_text(p: &Painter, bottom_left: Pos2, text: &str, font: egui::FontId, color: Color32) {
    let galley = p.layout_no_wrap(text.to_string(), font, color);
    p.add(egui::epaint::TextShape::new(bottom_left, galley, color).with_angle(-PI / 2.0));
}

/// Text truncated with an ellipsis to fit `max_width`.
pub fn fit_text(p: &Painter, text: &str, font: &egui::FontId, max_width: f32) -> String {
    let w = |s: &str| p.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    if w(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "..";
        if w(&s) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + ".."
}

/// A frameless child window with an FL-style title bar. Returns false when closed.
pub fn fl_window(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    open: &mut bool,
    default_pos: [f32; 2],
    default_size: [f32; 2],
    add_contents: impl FnOnce(&mut Ui),
) {
    if !*open {
        return;
    }
    let frame = egui::Frame::default()
        .fill(theme::WINDOW_BG)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x12, 0x14, 0x15)))
        .corner_radius(2)
        .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(130) });
    let mut close = false;
    egui::Window::new(id)
        .id(Id::new(id))
        .title_bar(false)
        .frame(frame)
        .default_pos(default_pos)
        .default_size(default_size)
        .resizable(true)
        .show(ctx, |ui| {
            let full = ui.max_rect();
            let bar = Rect::from_min_size(full.min, vec2(full.width(), 26.0));
            let p = ui.painter();
            p.rect_filled(bar, egui::CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, theme::TITLE_BG);
            p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0_f32, theme::TOP_SEP));
            paint_icon(p, Rect::from_center_size(pos2(bar.left() + 12.0, bar.center().y), vec2(12.0, 12.0)), Icon::ArrowRight, theme::TEXT_DIM);
            p.text(pos2(bar.left() + 24.0, bar.center().y), egui::Align2::LEFT_CENTER, title, theme::font(15.5), theme::TEXT);
            let close_rect = Rect::from_center_size(pos2(bar.right() - 14.0, bar.center().y), vec2(20.0, 20.0));
            let r = ui.interact(close_rect, Id::new((id, "close")), Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(close_rect, 2.0, theme::BUTTON_HOVER);
            }
            paint_icon(ui.painter(), close_rect, Icon::Close, theme::TEXT);
            if r.clicked() {
                close = true;
            }
            ui.allocate_space(vec2(full.width(), 26.0));
            egui::Frame::default().inner_margin(egui::Margin::same(4)).show(ui, add_contents);
        });
    if close {
        *open = false;
    }
}

pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(key % 12) as usize], key / 12)
}

pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
