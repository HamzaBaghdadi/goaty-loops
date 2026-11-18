use super::{snap_floor, snap_round};
use crate::app::{App, Focus};
use crate::theme;
use crate::widgets::{self, Icon};
use goaty_engine::*;
use egui::{pos2, vec2, Align2, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Ui};

const TITLE_H: f32 = 30.0;
const PICKER_W: f32 = 158.0;
const HEADER_W: f32 = 128.0;
const OVERVIEW_H: f32 = 22.0;
const RULER_H: f32 = 22.0;
const CLIP_TITLE_H: f32 = 17.0;

pub struct State {
    scroll_x: f32,
    scroll_y: f32,
    zoom: f32,
    track_h: f32,
    pub snap: u32,
    drag: Option<Drag>,
    deleting: bool,
    /// When set, clicking the playlist places this automation channel instead of the pattern.
    pub paint_auto: Option<usize>,
    /// Indices of selected clips.
    pub selected: std::collections::BTreeSet<usize>,
    marquee: Option<Pos2>,
    /// Right-clicked ruler position and the marker under it, for the marker menu.
    marker_menu: Option<(u32, Option<usize>)>,
}

impl Default for State {
    fn default() -> Self {
        Self { scroll_x: 0.0, scroll_y: 0.0, zoom: 0.16, track_h: 56.0, snap: PPQ, drag: None, deleting: false, paint_auto: None, selected: Default::default(), marquee: None, marker_menu: None }
    }
}

#[derive(Clone)]
enum DragMode {
    Move { grab: f32 },
    Resize,
    /// Left-edge trim; the clip's start, offset and end at drag start.
    TrimStart { start: u32, offset: u32, end: u32 },
    /// Moves every selected clip; `origins` holds (index, start, track) at drag start.
    MoveGroup { grab_tick: f32, grab_track: i64, origins: Vec<(usize, u32, usize)> },
}

#[derive(Clone)]
struct Drag {
    idx: usize,
    mode: DragMode,
}

struct View {
    area: Rect,
    scroll_x: f32,
    scroll_y: f32,
    zoom: f32,
    track_h: f32,
}

impl View {
    fn x(&self, t: f32) -> f32 {
        self.area.left() + (t - self.scroll_x) * self.zoom
    }
    fn tick_at(&self, x: f32) -> f32 {
        (x - self.area.left()) / self.zoom + self.scroll_x
    }
    fn y(&self, track: usize) -> f32 {
        self.area.top() + track as f32 * self.track_h - self.scroll_y
    }
    fn track_at(&self, y: f32) -> usize {
        ((y - self.area.top() + self.scroll_y) / self.track_h).floor().max(0.0) as usize
    }
    fn clip_rect(&self, c: &Clip) -> Rect {
        Rect::from_min_max(pos2(self.x(c.start as f32), self.y(c.track) + 1.0), pos2(self.x(c.end() as f32), self.y(c.track) + self.track_h - 1.0))
    }
}

/// Colour of a clip: its pattern colour, or FL's audio clip default.
fn clip_color(app: &App, c: &Clip) -> Color32 {
    match c.source {
        ClipSource::Pattern(i) => app.project.patterns.get(i).map(|p| theme::rgb(p.color)).unwrap_or(theme::CLIP_DEFAULT),
        ClipSource::Audio(_) => theme::AUDIO_CLIP_DEFAULT,
    }
}

fn clip_name(app: &App, c: &Clip) -> String {
    match c.source {
        ClipSource::Pattern(i) => app.project.patterns.get(i).map(|p| p.name.clone()).unwrap_or_default(),
        ClipSource::Audio(i) => app.project.channels.get(i).map(|ch| ch.name.clone()).unwrap_or_default(),
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 2.0, theme::WINDOW_BG);

    title_bar(app, ui, Rect::from_min_size(full.min, vec2(full.width(), TITLE_H)));

    let body = Rect::from_min_max(pos2(full.left(), full.top() + TITLE_H), full.max);
    let picker = Rect::from_min_max(body.min, pos2(body.left() + PICKER_W, body.bottom()));
    pattern_picker(app, ui, picker);

    let left = picker.right() + 2.0;
    let corner = Rect::from_min_max(pos2(left, body.top()), pos2(left + HEADER_W, body.top() + OVERVIEW_H + RULER_H));
    let headers = Rect::from_min_max(pos2(left, corner.bottom()), pos2(left + HEADER_W, body.bottom()));
    let overview = Rect::from_min_max(pos2(headers.right(), body.top()), pos2(body.right(), body.top() + OVERVIEW_H));
    let ruler = Rect::from_min_max(pos2(headers.right(), overview.bottom()), pos2(body.right(), corner.bottom()));
    let area = Rect::from_min_max(pos2(headers.right(), corner.bottom()), body.max);

    let canvas = Rect::from_min_max(pos2(left, body.top()), body.max);
    let resp = ui.interact(canvas, ui.id().with("playlist_canvas"), Sense::click_and_drag());
    // The ruler sits on top of the canvas: seeking and time markers.
    let ruler_resp = ui.interact(ruler, ui.id().with("playlist_ruler"), Sense::click_and_drag());

    let st = &mut app.playlist_ui;
    if resp.hovered() || ruler_resp.hovered() {
        let (scroll, zoom_delta, shift, alt, pointer) =
            ui.input(|i| (i.raw_scroll_delta, i.zoom_delta(), i.modifiers.shift, i.modifiers.alt, i.pointer.hover_pos()));
        if zoom_delta != 1.0 {
            let anchor = pointer.map(|p| (p.x - area.left()) / st.zoom + st.scroll_x).unwrap_or(st.scroll_x);
            st.zoom = (st.zoom * zoom_delta).clamp(0.01, 3.0);
            if let Some(p) = pointer {
                st.scroll_x = anchor - (p.x - area.left()) / st.zoom;
            }
        } else if alt && scroll.y != 0.0 {
            st.track_h = (st.track_h + scroll.y.signum() * 6.0).clamp(26.0, 160.0);
        } else if shift {
            st.scroll_x -= (scroll.y + scroll.x) / st.zoom;
        } else {
            st.scroll_x -= scroll.x / st.zoom;
            st.scroll_y -= scroll.y;
        }
    }
    let max_y = (app.project.playlist.tracks.len() as f32 * st.track_h - area.height() + 40.0).max(0.0);
    st.scroll_y = st.scroll_y.clamp(0.0, max_y);
    st.scroll_x = st.scroll_x.max(0.0);

    let v = View { area, scroll_x: st.scroll_x, scroll_y: st.scroll_y, zoom: st.zoom, track_h: st.track_h };
    interact(app, ui, &resp, &v, headers, overview, corner);
    ruler_interact(app, ui, &ruler_resp, &v);
    shortcuts(app, ui);
    paint(app, ui, &v, headers, ruler, overview, corner);
}

fn title_bar(app: &mut App, ui: &mut Ui, bar: Rect) {
    let p = ui.painter();
    p.rect_filled(bar, egui::CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, theme::TITLE_BG);
    p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0_f32, theme::TOP_SEP));
    let x = bar.left() + 8.0;
    widgets::paint_icon(p, Rect::from_center_size(pos2(x + 5.0, bar.center().y), vec2(12.0, 12.0)), Icon::ArrowRight, theme::TEXT_DIM);

    let tools = Rect::from_min_max(pos2(x + 22.0, bar.top()), pos2(x + 360.0, bar.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tools).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let (mr, _) = child.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    widgets::paint_icon(child.painter(), mr, Icon::Magnet, theme::LED_ON);
    super::snap_combo(&mut child, "pl_snap", &mut app.playlist_ui.snap);
    if widgets::icon_button(&mut child, Icon::Minimize, theme::TEXT_DIM, 20.0, "Zoom out (Ctrl+wheel)").clicked() {
        app.playlist_ui.zoom = (app.playlist_ui.zoom / 1.25).max(0.01);
    }
    if widgets::icon_button(&mut child, Icon::Plus, theme::TEXT_DIM, 20.0, "Zoom in (Ctrl+wheel)").clicked() {
        app.playlist_ui.zoom = (app.playlist_ui.zoom * 1.25).min(3.0);
    }
    let used = child.min_rect().right();

    let pat = app.project.patterns.get(app.cur_pattern).map(|p| p.name.clone()).unwrap_or_default();
    let p = ui.painter();
    let tx = used + 18.0;
    widgets::paint_icon(p, Rect::from_center_size(pos2(tx, bar.center().y), vec2(14.0, 14.0)), Icon::Playlist, theme::TEXT_DIM);
    p.text(pos2(tx + 14.0, bar.center().y), Align2::LEFT_CENTER, format!("Playlist - Arrangement  \u{203A}  {pat}"), theme::font(17.0), theme::TEXT);
}

fn pattern_picker(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, theme::PATPICK_BG);
    // Category icons along the top (patterns / audio / automation).
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), OVERVIEW_H + RULER_H));
    for (i, icon) in [Icon::PianoRoll, Icon::Audio, Icon::Magnet].into_iter().enumerate() {
        let r = Rect::from_center_size(pos2(top.left() + 26.0 + i as f32 * 50.0, top.center().y), vec2(18.0, 18.0));
        widgets::paint_icon(p, r, icon, if i == 0 { theme::TEXT } else { theme::TEXT_FAINT });
    }
    let list = Rect::from_min_max(pos2(rect.left(), top.bottom()), rect.max);
    let item_h = 34.0;
    let mut y = list.top() + 4.0;
    let mut clicked = None;
    let mut open_roll = None;
    for i in 0..app.project.patterns.len() {
        if y > list.bottom() {
            break;
        }
        let r = Rect::from_min_size(pos2(list.left() + 8.0, y), vec2(list.width() - 12.0, item_h - 4.0));
        let resp = ui.interact(r, ui.id().with(("pat_pick", i)), Sense::click()).on_hover_text("Click to select, double-click to open in the piano roll");
        let pat = &app.project.patterns[i];
        let p = ui.painter();
        p.rect_filled(r, 2.0, if resp.hovered() { theme::lighten(theme::PATPICK_ITEM, 0.04) } else { theme::PATPICK_ITEM });
        // Faint note preview behind the name.
        if !pat.notes.is_empty() {
            let (lo, hi) = pat.notes.iter().fold((127u8, 0u8), |(lo, hi), n| (lo.min(n.key), hi.max(n.key)));
            let len = app.project.pattern_length(i) as f32;
            let preview = Rect::from_min_max(pos2(r.left() + r.width() * 0.45, r.top() + 4.0), r.max - vec2(4.0, 4.0));
            for n in pat.notes.iter().take(400) {
                let x0 = preview.left() + n.start as f32 / len * preview.width();
                let x1 = preview.left() + n.end() as f32 / len * preview.width();
                let y = if hi == lo { preview.center().y } else { preview.bottom() - (n.key - lo) as f32 / (hi - lo) as f32 * preview.height() };
                p.line_segment([pos2(x0, y), pos2(x1.max(x0 + 2.0), y)], Stroke::new(1.5_f32, theme::TEXT_FAINT));
            }
        }
        if app.cur_pattern == i {
            let a = pos2(r.left() - 6.0, r.center().y);
            p.add(egui::Shape::convex_polygon(vec![a + vec2(0.0, -7.0), a + vec2(9.0, 0.0), a + vec2(0.0, 7.0)], Color32::from_rgb(0xe8, 0x3a, 0x4c), Stroke::NONE));
        }
        widgets::paint_icon(p, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(12.0, 12.0)), Icon::Pattern, theme::rgb(pat.color));
        let name = widgets::fit_text(p, &pat.name, &theme::font(16.0), r.width() - 30.0);
        p.text(pos2(r.left() + 24.0, r.center().y), Align2::LEFT_CENTER, name, theme::font(16.0), theme::TEXT);
        if resp.double_clicked() {
            open_roll = Some(i);
        } else if resp.clicked() {
            clicked = Some(i);
        }
        y += item_h;
    }
    // Automation clips, below the patterns.
    let autos: Vec<(usize, String)> = app
        .project
        .channels
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.instrument, Instrument::Automation(_)))
        .map(|(i, c)| (i, c.name.clone()))
        .collect();
    for (ch, name) in autos {
        if y > list.bottom() {
            break;
        }
        let r = Rect::from_min_size(pos2(list.left() + 8.0, y), vec2(list.width() - 12.0, item_h - 4.0));
        let resp = ui.interact(r, ui.id().with(("auto_pick", ch)), Sense::click()).on_hover_text("Click to paint this automation clip, double-click to edit");
        let p = ui.painter();
        p.rect_filled(r, 2.0, if resp.hovered() { theme::lighten(theme::PATPICK_ITEM, 0.04) } else { theme::PATPICK_ITEM });
        if let Instrument::Automation(a) = &app.project.channels[ch].instrument {
            let preview = Rect::from_min_max(pos2(r.left() + r.width() * 0.45, r.top() + 4.0), r.max - vec2(4.0, 4.0));
            super::paint_auto_curve(p, preview, a, 0.0, a.length_ticks().max(1) as f32, theme::TEXT_FAINT);
        }
        if app.playlist_ui.paint_auto == Some(ch) {
            let a = pos2(r.left() - 6.0, r.center().y);
            p.add(egui::Shape::convex_polygon(vec![a + vec2(0.0, -7.0), a + vec2(9.0, 0.0), a + vec2(0.0, 7.0)], Color32::from_rgb(0xe8, 0x3a, 0x4c), Stroke::NONE));
        }
        widgets::paint_icon(p, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(12.0, 12.0)), Icon::Magnet, theme::PR_NOTE);
        let label = widgets::fit_text(p, &name, &theme::font(16.0), r.width() - 30.0);
        p.text(pos2(r.left() + 24.0, r.center().y), Align2::LEFT_CENTER, label, theme::font(16.0), theme::TEXT);
        if resp.double_clicked() {
            app.automation_ui.open = Some(ch);
        } else if resp.clicked() {
            app.playlist_ui.paint_auto = Some(ch);
        }
        y += item_h;
    }
    if let Some(i) = clicked {
        app.playlist_ui.paint_auto = None;
        app.cur_pattern = i;
    }
    if let Some(i) = open_roll {
        app.cur_pattern = i;
        app.show_piano = true;
    }
}

/// Where a clip was grabbed: its edges trim, its body moves.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Left,
    Body,
    Right,
}

fn hit_clip(clips: &[Clip], v: &View, pos: Pos2) -> Option<(usize, Part)> {
    clips.iter().enumerate().rev().find_map(|(i, c)| {
        let r = v.clip_rect(c);
        let edge = 6.0_f32.min(r.width() * 0.3);
        let part = if pos.x > r.right() - edge {
            Part::Right
        } else if pos.x < r.left() + edge {
            Part::Left
        } else {
            Part::Body
        };
        r.contains(pos).then_some((i, part))
    })
}

#[allow(clippy::too_many_arguments)]
fn interact(app: &mut App, ui: &Ui, resp: &egui::Response, v: &View, headers: Rect, overview: Rect, corner: Rect) {
    let (pointer, pp, pd, sp, sd, mods) = ui.input(|i| {
        (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.secondary_pressed(), i.pointer.secondary_down(), i.modifiers)
    });
    let shift = mods.shift;
    let snap = app.playlist_ui.snap;
    let n_clips = app.project.playlist.clips.len();
    app.playlist_ui.selected.retain(|&i| i < n_clips);

    if let Some(pos) = pointer {
        let hovered = resp.hovered();
        if hovered && (pp || sp) && v.area.contains(pos) {
            app.focus = Focus::Playlist;
        }
        if hovered && v.area.contains(pos) {
            let hit = hit_clip(&app.project.playlist.clips, v, pos);
            match hit {
                Some(_) if mods.alt => ui.ctx().set_cursor_icon(CursorIcon::VerticalText),
                Some((_, Part::Left | Part::Right)) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                Some(_) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
                None if mods.ctrl => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
                None => {}
            }
            if pp && mods.ctrl {
                match hit {
                    // Ctrl+click toggles a clip; Ctrl+drag on empty space draws a selection box.
                    Some((idx, _)) => {
                        if !app.playlist_ui.selected.remove(&idx) {
                            app.playlist_ui.selected.insert(idx);
                        }
                    }
                    None => {
                        app.playlist_ui.marquee = Some(pos);
                        if !shift {
                            app.playlist_ui.selected.clear();
                        }
                    }
                }
            } else if pp && mods.alt {
                // Alt+click slices the clip at the snapped position.
                if let Some((idx, _)) = hit {
                    let at = snap_round(v.tick_at(pos.x), snap);
                    let c = app.project.playlist.clips[idx];
                    if at > c.start && at < c.end() {
                        app.checkpoint();
                        split_clip(&mut app.project.playlist.clips, idx, at);
                        app.playlist_ui.selected.clear();
                        app.changed();
                    }
                }
            } else if pp {
                app.checkpoint();
                let drag = match hit {
                    Some((idx, Part::Right)) => Drag { idx, mode: DragMode::Resize },
                    Some((idx, Part::Left)) => {
                        let c = app.project.playlist.clips[idx];
                        Drag { idx, mode: DragMode::TrimStart { start: c.start, offset: c.offset, end: c.end() } }
                    }
                    Some((idx, Part::Body)) if !shift && app.playlist_ui.selected.contains(&idx) && app.playlist_ui.selected.len() > 1 => {
                        let clips = &app.project.playlist.clips;
                        let origins = app.playlist_ui.selected.iter().map(|&i| (i, clips[i].start, clips[i].track)).collect();
                        Drag { idx, mode: DragMode::MoveGroup { grab_tick: v.tick_at(pos.x), grab_track: v.track_at(pos.y) as i64, origins } }
                    }
                    Some((idx, Part::Body)) => {
                        app.playlist_ui.selected.clear();
                        if ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
                            if let ClipSource::Audio(c) = app.project.playlist.clips[idx].source {
                                if matches!(app.project.channels.get(c).map(|c| &c.instrument), Some(Instrument::Automation(_))) {
                                    app.automation_ui.open = Some(c);
                                }
                            }
                        }
                        let clip = app.project.playlist.clips[idx];
                        if let ClipSource::Pattern(p) = clip.source {
                            app.cur_pattern = p;
                        }
                        let idx = if shift {
                            app.project.playlist.clips.push(clip);
                            app.changed();
                            app.project.playlist.clips.len() - 1
                        } else {
                            idx
                        };
                        Drag { idx, mode: DragMode::Move { grab: v.tick_at(pos.x) - clip.start as f32 } }
                    }
                    None => {
                        app.playlist_ui.selected.clear();
                        let track = v.track_at(pos.y);
                        app.project.ensure_tracks(track + 1);
                        let start = snap_floor(v.tick_at(pos.x), snap);
                        let (source, length) = match app.playlist_ui.paint_auto.filter(|&c| matches!(app.project.channels.get(c).map(|c| &c.instrument), Some(Instrument::Automation(_)))) {
                            Some(c) => {
                                let len = match &app.project.channels[c].instrument {
                                    Instrument::Automation(a) => a.length_ticks(),
                                    _ => 0,
                                };
                                (ClipSource::Audio(c), len.max(app.project.bar_ticks()))
                            }
                            None => (ClipSource::Pattern(app.cur_pattern), app.project.pattern_length(app.cur_pattern)),
                        };
                        app.project.playlist.clips.push(Clip { track, start, length, offset: 0, source, muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
                        app.changed();
                        Drag { idx: app.project.playlist.clips.len() - 1, mode: DragMode::Move { grab: v.tick_at(pos.x) - start as f32 } }
                    }
                };
                app.playlist_ui.drag = Some(drag);
            }
            if sp {
                app.checkpoint();
                app.playlist_ui.deleting = true;
            }
        }
        if app.playlist_ui.deleting && sd && v.area.contains(pos) {
            if let Some((idx, _)) = hit_clip(&app.project.playlist.clips, v, pos) {
                app.project.playlist.clips.remove(idx);
                app.changed();
            }
        }
        if let (Some(Drag { mode: DragMode::MoveGroup { grab_tick, grab_track, origins }, .. }), true) = (app.playlist_ui.drag.clone(), pd) {
            let s = snap.max(1) as f32;
            let min_start = origins.iter().map(|o| o.1).min().unwrap_or(0) as f32;
            let dt = (((v.tick_at(pos.x) - grab_tick) / s).round() * s).max(-min_start) as i64;
            let ntracks = app.project.playlist.tracks.len() as i64;
            let (min_tr, max_tr) = (origins.iter().map(|o| o.2).min().unwrap_or(0) as i64, origins.iter().map(|o| o.2).max().unwrap_or(0) as i64);
            let dtr = (v.track_at(pos.y) as i64 - grab_track).clamp(-min_tr, ntracks - 1 - max_tr);
            let mut moved = false;
            for &(i, start, track) in &origins {
                if let Some(c) = app.project.playlist.clips.get_mut(i) {
                    let (ns, nt) = ((start as i64 + dt) as u32, (track as i64 + dtr) as usize);
                    if c.start != ns || c.track != nt {
                        c.start = ns;
                        c.track = nt;
                        moved = true;
                    }
                }
            }
            if moved {
                app.changed();
            }
        } else if let (Some(drag), true) = (app.playlist_ui.drag.clone(), pd) {
            let ntracks = app.project.playlist.tracks.len();
            if let Some(c) = app.project.playlist.clips.get_mut(drag.idx) {
                match drag.mode {
                    DragMode::MoveGroup { .. } => {}
                    DragMode::Move { grab } => {
                        let start = snap_round(v.tick_at(pos.x) - grab, snap);
                        let track = v.track_at(pos.y).min(ntracks.saturating_sub(1));
                        if start != c.start || track != c.track {
                            c.start = start;
                            c.track = track;
                            app.changed();
                        }
                    }
                    DragMode::Resize => {
                        let end = snap_round(v.tick_at(pos.x), snap);
                        let len = end.saturating_sub(c.start).max(snap.max(1));
                        if len != c.length {
                            c.length = len;
                            app.changed();
                        }
                    }
                    DragMode::TrimStart { start, offset, end } => {
                        // The end stays put; the source can't start before its beginning.
                        let earliest = start.saturating_sub(offset);
                        let latest = end.saturating_sub(snap.max(1)).max(earliest);
                        let new_start = snap_round(v.tick_at(pos.x), snap).clamp(earliest, latest);
                        if new_start != c.start {
                            c.offset = (offset as i64 + new_start as i64 - start as i64).max(0) as u32;
                            c.start = new_start;
                            c.length = end - new_start;
                            app.changed();
                        }
                    }
                }
            }
        }

        // Overview: jump the view.
        if hovered && overview.contains(pos) && pd {
            let total = overview_ticks(app, v) as f32;
            let visible = v.area.width() / v.zoom;
            let t = (pos.x - overview.left()) / overview.width() * total;
            app.playlist_ui.scroll_x = (t - visible / 2.0).max(0.0);
        }
        if hovered && headers.contains(pos) && pp {
            let t = v.track_at(pos.y);
            if t < app.project.playlist.tracks.len() {
                let led = pos2(headers.right() - 14.0, v.y(t) + v.track_h - 14.0);
                if pos.distance(led) < 9.0 {
                    app.checkpoint();
                    app.project.playlist.tracks[t].muted = !app.project.playlist.tracks[t].muted;
                    app.changed();
                }
            }
        }
        if hovered && pp && Rect::from_center_size(pos2(corner.left() + 18.0, corner.bottom() - 12.0), vec2(20.0, 20.0)).contains(pos) {
            app.checkpoint();
            let n = app.project.playlist.tracks.len() + 1;
            app.project.ensure_tracks(n);
            app.changed();
        }
    }
    if !pd {
        if let (Some(start), Some(end)) = (app.playlist_ui.marquee.take(), pointer) {
            let r = Rect::from_two_pos(start, end);
            for (i, c) in app.project.playlist.clips.iter().enumerate() {
                if v.clip_rect(c).intersects(r) {
                    app.playlist_ui.selected.insert(i);
                }
            }
        }
        app.playlist_ui.drag = None;
    }
    if !sd {
        app.playlist_ui.deleting = false;
    }
}


/// Editing shortcuts while the playlist has focus.
fn shortcuts(app: &mut App, ui: &Ui) {
    if app.focus != Focus::Playlist || ui.ctx().wants_keyboard_input() {
        return;
    }
    use egui::Key;
    let cmd = ui.input(|i| i.modifiers.command);
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
    let sel: Vec<usize> = app.playlist_ui.selected.iter().copied().collect();
    let clips = &app.project.playlist.clips;

    if ui.input(|i| i.modifiers.alt) && pressed(Key::T) {
        // Alt+T: time marker at the song position (FL shortcut).
        let at = snap_floor(app.play_position() as f32, app.playlist_ui.snap);
        add_marker(app, at);
        return;
    }
    if cmd && pressed(Key::A) {
        app.playlist_ui.selected = (0..clips.len()).collect();
        return;
    }
    if cmd && (pressed(Key::C) || pressed(Key::X)) && !sel.is_empty() {
        app.clip_clipboard = sel.iter().map(|&i| clips[i]).collect();
        let cut = pressed(Key::X);
        app.status = format!("{} {} clip(s)", if cut { "Cut" } else { "Copied" }, sel.len());
        if cut {
            delete_clips(app, &sel);
        }
        return;
    }
    if cmd && pressed(Key::V) && !app.clip_clipboard.is_empty() {
        // Paste at the song position, keeping the clips' relative layout.
        app.checkpoint();
        let bar = app.project.bar_ticks();
        let at = (app.play_position() as u32 / bar) * bar;
        let first_start = app.clip_clipboard.iter().map(|c| c.start).min().unwrap_or(0);
        let max_track = app.clip_clipboard.iter().map(|c| c.track).max().unwrap_or(0);
        app.project.ensure_tracks(max_track + 1);
        let clips = &mut app.project.playlist.clips;
        let first = clips.len();
        clips.extend(app.clip_clipboard.iter().map(|c| Clip { start: c.start - first_start + at, ..*c }));
        app.playlist_ui.selected = (first..clips.len()).collect();
        app.changed();
        return;
    }
    if cmd && pressed(Key::B) && !sel.is_empty() {
        // Duplicate right after the selection, rounded up to whole bars.
        app.checkpoint();
        let bar = app.project.bar_ticks();
        let clips = &mut app.project.playlist.clips;
        let start = sel.iter().map(|&i| clips[i].start).min().unwrap_or(0);
        let end = sel.iter().map(|&i| clips[i].end()).max().unwrap_or(0);
        let span = (end - start).div_ceil(bar).max(1) * bar;
        let first = clips.len();
        let copies: Vec<Clip> = sel.iter().map(|&i| Clip { start: clips[i].start + span, ..clips[i] }).collect();
        clips.extend(copies);
        app.playlist_ui.selected = (first..clips.len()).collect();
        app.changed();
        return;
    }
    if (pressed(Key::Delete) || pressed(Key::Backspace)) && !sel.is_empty() {
        delete_clips(app, &sel);
        return;
    }
    let (left, right) = (pressed(Key::ArrowLeft), pressed(Key::ArrowRight));
    if !sel.is_empty() && (left || right) {
        let snap = app.playlist_ui.snap.max(1) as i64;
        let dt = if right { snap } else { -snap };
        let clips = &mut app.project.playlist.clips;
        if sel.iter().all(|&i| clips[i].start as i64 + dt >= 0) {
            app.checkpoint();
            let clips = &mut app.project.playlist.clips;
            for &i in &sel {
                clips[i].start = (clips[i].start as i64 + dt) as u32;
            }
            app.changed();
        }
    }
}

/// Ruler: click or drag to seek; right-click for time markers.
fn ruler_interact(app: &mut App, ui: &Ui, resp: &egui::Response, v: &View) {
    let snap = app.playlist_ui.snap;
    if let Some(pos) = resp.interact_pointer_pos() {
        if ui.input(|i| i.pointer.primary_down()) {
            app.focus = Focus::Playlist;
            app.set_song_mode(true);
            app.engine.transport().seek(snap_floor(v.tick_at(pos.x), snap) as f64);
        }
        if resp.secondary_clicked() {
            let near = app.project.playlist.markers.iter().position(|m| (v.x(m.tick as f32) - pos.x).abs() < 8.0 || (pos.x >= v.x(m.tick as f32) && pos.x < v.x(m.tick as f32) + 90.0));
            app.playlist_ui.marker_menu = Some((snap_floor(v.tick_at(pos.x), snap), near));
        }
    }
    resp.context_menu(|ui| {
        let Some((tick, near)) = app.playlist_ui.marker_menu else { return };
        if ui.button("Add time marker here").clicked() {
            add_marker(app, tick);
            ui.close_menu();
        }
        ui.menu_button("Time signature from here", |ui| {
            for (n, d) in [(2u8, 4u8), (3, 4), (4, 4), (5, 4), (6, 4), (7, 4), (6, 8), (7, 8), (9, 8), (12, 8)] {
                if ui.button(format!("{n}/{d}")).clicked() {
                    // FL signature markers sit on bar lines.
                    let at = app.project.bars(tick, tick).first().map(|b| b.0).unwrap_or(tick);
                    app.checkpoint();
                    let markers = &mut app.project.playlist.markers;
                    let pos = markers.iter().position(|m| m.tick > at).unwrap_or(markers.len());
                    markers.insert(pos, TimeMarker { tick: at, name: format!("{n}/{d}"), kind: TimeMarker::SIGNATURE, numerator: n, denominator: d });
                    app.changed();
                    ui.close_menu();
                }
            }
        });
        if let Some(i) = near.filter(|&i| i < app.project.playlist.markers.len()) {
            ui.separator();
            let mut name = app.project.playlist.markers[i].name.clone();
            let r = ui.text_edit_singleline(&mut name);
            if r.gained_focus() {
                app.checkpoint();
            }
            if r.changed() {
                app.project.playlist.markers[i].name = name;
                app.changed();
            }
            if ui.button("Delete marker").clicked() {
                app.checkpoint();
                app.project.playlist.markers.remove(i);
                app.changed();
                ui.close_menu();
            }
        }
    });
}

/// Adds a time marker at `tick`, keeping markers in time order.
pub fn add_marker(app: &mut App, tick: u32) {
    app.checkpoint();
    let markers = &mut app.project.playlist.markers;
    let name = format!("Marker {}", markers.len() + 1);
    let at = markers.iter().position(|m| m.tick > tick).unwrap_or(markers.len());
    markers.insert(at, TimeMarker { tick, name, kind: 0, numerator: 0, denominator: 0 });
    app.changed();
}

/// Splits clip `idx` at song tick `at` (inside the clip); the right part
/// continues from the same point in the source.
fn split_clip(clips: &mut Vec<Clip>, idx: usize, at: u32) {
    let c = clips[idx];
    let left = at - c.start;
    clips[idx].length = left;
    clips.push(Clip { start: at, length: c.length - left, offset: c.offset + left, ..c });
}

fn delete_clips(app: &mut App, indices: &[usize]) {
    app.checkpoint();
    let mut idx = indices.to_vec();
    idx.sort_unstable_by(|a, b| b.cmp(a));
    for i in idx {
        if i < app.project.playlist.clips.len() {
            app.project.playlist.clips.remove(i);
        }
    }
    app.playlist_ui.selected.clear();
    app.changed();
}
fn overview_ticks(app: &App, v: &View) -> u32 {
    let visible_end = (v.scroll_x + v.area.width() / v.zoom) as u32;
    (app.project.song_length() + app.project.bar_ticks() * 8).max(visible_end)
}

#[allow(clippy::too_many_arguments)]
fn paint(app: &App, ui: &Ui, v: &View, headers: Rect, ruler: Rect, overview: Rect, corner: Rect) {
    let area = v.area;
    let p = ui.painter();
    let bar = app.project.bar_ticks();
    let beat = app.project.beat_ticks();
    let tracks = &app.project.playlist.tracks;

    // Grid: alternating bars, beat and bar lines (bars follow signature markers).
    let gp = p.with_clip_rect(area);
    let bars = app.project.bars(v.scroll_x.max(0.0) as u32, v.tick_at(area.right()) as u32 + bar);
    let beat_px = beat as f32 * v.zoom;
    for &(t, n, len, beat) in &bars {
        let (x0, x1) = (v.x(t as f32), v.x((t + len) as f32));
        let fill = if n % 2 == 1 { theme::GRID_A } else { theme::GRID_B };
        gp.rect_filled(Rect::from_min_max(pos2(x0, area.top()), pos2(x1, area.bottom())), 0.0, fill);
        let x = x0.round() + 0.5;
        gp.line_segment([pos2(x, area.top()), pos2(x, area.bottom())], Stroke::new(1.0_f32, theme::GRID_BAR));
        if beat_px >= 6.0 {
            for b in (beat..len).step_by(beat.max(1) as usize) {
                let x = v.x((t + b) as f32).round() + 0.5;
                gp.line_segment([pos2(x, area.top()), pos2(x, area.bottom())], Stroke::new(1.0_f32, theme::GRID_BEAT));
            }
        }
    }
    let first = (v.scroll_y / v.track_h).floor() as usize;
    let last = ((v.scroll_y + area.height()) / v.track_h).ceil() as usize;
    for tr in first..=last {
        let y = v.y(tr + 1).round() - 0.5;
        gp.line_segment([pos2(area.left(), y), pos2(area.right(), y)], Stroke::new(1.0_f32, theme::GRID_BAR));
    }

    // Clips.
    for (i, c) in app.project.playlist.clips.iter().enumerate() {
        let r = v.clip_rect(c);
        if r.right() < area.left() || r.left() > area.right() || r.bottom() < area.top() || r.top() > area.bottom() {
            continue;
        }
        paint_clip(app, &gp, c, r, v, area);
        if app.playlist_ui.selected.contains(&i) {
            gp.rect_stroke(r, 2.0, Stroke::new(2.0_f32, theme::TEXT_BRIGHT), egui::StrokeKind::Inside);
            gp.rect_filled(r, 2.0, Color32::from_white_alpha(18));
        }
    }

    // Selection box.
    if let (Some(start), Some(end)) = (app.playlist_ui.marquee, ui.input(|i| i.pointer.interact_pos())) {
        let r = Rect::from_two_pos(start, end);
        gp.rect_filled(r, 0.0, theme::PR_NOTE.gamma_multiply(0.12));
        gp.rect_stroke(r, 0.0, Stroke::new(1.0_f32, theme::PR_NOTE), egui::StrokeKind::Inside);
    }
    // Playhead.
    if app.song_mode {
        let x = v.x(app.play_position() as f32);
        if x >= area.left() && x <= area.right() {
            gp.line_segment([pos2(x, area.top()), pos2(x, area.bottom())], Stroke::new(1.0_f32, theme::PLAYHEAD));
        }
    }

    // Overview.
    p.rect_filled(overview, 0.0, theme::OVERVIEW_BG);
    let total = overview_ticks(app, v) as f32;
    let ntr = tracks.len().max(1) as f32;
    for c in &app.project.playlist.clips {
        let x0 = overview.left() + c.start as f32 / total * overview.width();
        let x1 = overview.left() + c.end() as f32 / total * overview.width();
        let y = overview.top() + 3.0 + c.track as f32 / ntr * (overview.height() - 6.0);
        p.line_segment([pos2(x0, y), pos2(x1.max(x0 + 1.0), y)], Stroke::new(1.0_f32, theme::lighten(clip_color(app, c), 0.4)));
    }
    let vx0 = overview.left() + v.scroll_x / total * overview.width();
    let vx1 = overview.left() + (v.scroll_x + area.width() / v.zoom) / total * overview.width();
    p.rect_filled(Rect::from_min_max(pos2(vx0, overview.top()), pos2(vx1.min(overview.right()), overview.bottom())), 0.0, Color32::from_white_alpha(14));

    // Ruler.
    p.rect_filled(ruler, 0.0, theme::RULER_BG);
    let bar_px = bar as f32 * v.zoom;
    let label_every = if bar_px < 18.0 { 8 } else if bar_px < 30.0 { 4 } else if bar_px < 55.0 { 2 } else { 1 };
    let rp = p.with_clip_rect(ruler);
    for &(t, n, _, _) in &bars {
        let x = v.x(t as f32);
        let big = (n - 1) % label_every == 0;
        rp.line_segment([pos2(x, ruler.bottom() - if big { 7.0 } else { 3.0 }), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, theme::TEXT_DIM));
        if big {
            rp.text(pos2(x + 3.0, ruler.center().y - 1.0), Align2::LEFT_CENTER, format!("{n}"), theme::font(14.0), theme::TEXT);
        }
    }
    // Time markers: a line through the playlist and a name tag in the ruler.
    let ap = p.with_clip_rect(area);
    for m in &app.project.playlist.markers {
        let x = v.x(m.tick as f32);
        if x < ruler.left() - 120.0 || x > ruler.right() {
            continue;
        }
        let col = if m.kind == 0 { theme::TEXT_BRIGHT } else { theme::ORANGE };
        ap.line_segment([pos2(x, area.top()), pos2(x, area.bottom())], Stroke::new(1.0_f32, col.gamma_multiply(0.25)));
        let label = if m.name.trim().is_empty() { "\u{25B8}".to_string() } else { m.name.clone() };
        let galley = rp.layout_no_wrap(label, theme::font(13.0), col);
        let tag = Rect::from_min_size(pos2(x, ruler.top() + 1.0), vec2(galley.size().x + 8.0, ruler.height() - 2.0));
        rp.rect_filled(tag, 2.0, theme::WINDOW_BG);
        rp.line_segment([pos2(x, ruler.top()), pos2(x, ruler.bottom())], Stroke::new(2.0_f32, col));
        rp.galley(pos2(x + 5.0, ruler.center().y - galley.size().y / 2.0), galley, col);
    }
    let end_x = v.x(app.project.song_length() as f32);
    if end_x > ruler.left() && end_x < ruler.right() {
        rp.line_segment([pos2(end_x, ruler.top()), pos2(end_x, ruler.bottom())], Stroke::new(2.0_f32, theme::ORANGE));
    }
    if app.song_mode {
        let x = v.x(app.play_position() as f32);
        if x >= ruler.left() - 6.0 && x <= ruler.right() {
            let tri = vec![pos2(x - 6.0, ruler.bottom() - 9.0), pos2(x + 6.0, ruler.bottom() - 9.0), pos2(x, ruler.bottom())];
            rp.add(egui::Shape::convex_polygon(tri, theme::PLAYHEAD, Stroke::NONE));
        }
    }

    // Corner and track headers.
    p.rect_filled(corner, 0.0, theme::WINDOW_BG);
    widgets::paint_icon(p, Rect::from_center_size(pos2(corner.left() + 18.0, corner.bottom() - 12.0), vec2(16.0, 16.0)), Icon::Plus, theme::TEXT);
    p.rect_filled(headers, 0.0, theme::WINDOW_BG);
    let hp = p.with_clip_rect(headers);
    for t in first..=last.min(tracks.len().saturating_sub(1)) {
        let y = v.y(t);
        let r = Rect::from_min_max(pos2(headers.left(), y + 1.0), pos2(headers.right() - 2.0, y + v.track_h - 1.0));
        hp.rect_filled(r, 2.0, theme::TRACK_HDR);
        let used = app.project.playlist.clips.iter().any(|c| c.track == t);
        let name_col = if used { theme::TEXT } else { theme::TEXT_DIM };
        hp.text(pos2(r.left() + 6.0, r.top() + 4.0), Align2::LEFT_TOP, &tracks[t].name, theme::font(16.0), name_col);
        if v.track_h >= 40.0 {
            hp.text(pos2(r.left() + 14.0, r.bottom() - 7.0), Align2::LEFT_CENTER, "...", theme::font(12.0), theme::TEXT_FAINT);
        }
        let led = pos2(headers.right() - 14.0, y + v.track_h - 14.0);
        widgets::paint_led(&hp, led, 5.0, !tracks[t].muted);
    }
    // Separators.
    p.line_segment([pos2(area.left(), overview.top()), pos2(area.left(), area.bottom())], Stroke::new(1.0_f32, theme::GRID_BAR));
    p.line_segment([pos2(ruler.left(), ruler.bottom()), pos2(ruler.right(), ruler.bottom())], Stroke::new(1.0_f32, theme::GRID_BAR));
}

fn paint_clip(app: &App, p: &egui::Painter, c: &Clip, r: Rect, v: &View, area: Rect) {
    let color = clip_color(app, c);
    let muted = c.muted || app.project.playlist.tracks.get(c.track).map(|t| t.muted).unwrap_or(false);
    let alpha = if muted { 0.45 } else { 1.0 };
    let title = Rect::from_min_size(r.min, vec2(r.width(), CLIP_TITLE_H.min(r.height())));
    let body = Rect::from_min_max(pos2(r.left(), title.bottom()), r.max);

    p.rect_filled(body, 0.0, theme::mix(theme::GRID_B, color, 0.22 * alpha));
    p.rect_filled(title, 0.0, theme::mix(theme::GRID_B, color, 0.95 * alpha));
    p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0_f32, theme::lighten(color, 0.2).gamma_multiply(alpha)));

    let tp = p.with_clip_rect(title.intersect(area));
    widgets::paint_icon(&tp, Rect::from_center_size(pos2(title.left() + 8.0, title.center().y), vec2(9.0, 9.0)), Icon::ArrowRight, theme::TEXT_BRIGHT);
    let name = clip_name(app, c);
    tp.text(pos2(title.left() + 15.0, title.center().y), Align2::LEFT_CENTER, name, theme::font(14.0), theme::TEXT_BRIGHT.gamma_multiply(if muted { 0.6 } else { 1.0 }));

    let content = Rect::from_min_max(pos2(body.left() + 1.0, body.top() + 2.0), pos2(body.right() - 1.0, body.bottom() - 2.0));
    let cp = p.with_clip_rect(content.intersect(area));
    let fg = theme::lighten(color, 0.55).gamma_multiply(alpha);
    match c.source {
        ClipSource::Pattern(i) => paint_pattern_preview(app, &cp, i, c, content, v, fg),
        ClipSource::Audio(ch) => match app.project.channels.get(ch).map(|c| &c.instrument) {
            Some(Instrument::Automation(a)) => {
                let r = Rect::from_min_max(pos2(v.x(c.start as f32), content.top() + 2.0), pos2(v.x(c.end() as f32), content.bottom() - 2.0));
                super::paint_auto_curve(&cp, r, a, c.offset as f32, c.length as f32, fg);
            }
            _ => paint_waveform(app, &cp, ch, c, content, v, fg),
        },
    }
}

fn paint_pattern_preview(app: &App, p: &egui::Painter, idx: usize, c: &Clip, content: Rect, v: &View, col: Color32) {
    let Some(pat) = app.project.patterns.get(idx) else { return };
    if pat.notes.is_empty() || content.height() < 4.0 {
        return;
    }
    let (lo, hi) = pat.notes.iter().fold((127u8, 0u8), |(lo, hi), n| (lo.min(n.key), hi.max(n.key)));
    let range = (hi - lo).max(1) as f32;
    let plen = app.project.pattern_length(idx).max(1);
    let thick = (content.height() / (range + 2.0)).clamp(1.5, 4.0);
    for n in &pat.notes {
        if n.start < c.offset || n.start >= c.offset + c.length {
            continue;
        }
        let t = c.start + n.start - c.offset;
        let x0 = v.x(t as f32);
        let x1 = v.x((t + (n.end() - n.start)) as f32).min(v.x(c.end() as f32));
        let y = if hi == lo { content.center().y } else { content.bottom() - 2.0 - (n.key - lo) as f32 / range * (content.height() - 4.0) };
        p.line_segment([pos2(x0, y), pos2(x1.max(x0 + 1.5), y)], Stroke::new(thick, col));
    }
    if c.length + c.offset > plen {
        let x = v.x((c.start + plen.saturating_sub(c.offset)) as f32);
        p.line_segment([pos2(x, content.top()), pos2(x, content.bottom())], Stroke::new(1.0_f32, theme::darken(col, 0.4)));
    }
}

fn paint_waveform(app: &App, p: &egui::Painter, ch: usize, c: &Clip, content: Rect, v: &View, col: Color32) {
    let Some(Instrument::Sampler(s)) = app.project.channels.get(ch).map(|c| &c.instrument) else { return };
    let Some(data) = &s.data else { return };
    let secs_per_tick = 60.0 / (app.project.tempo as f32 * PPQ as f32);
    let blocks_per_sec = data.sample_rate as f32 / sample::PEAK_BLOCK as f32;
    let mid = content.center().y;
    let x_start = v.x(c.start as f32).max(content.left()).max(p.clip_rect().left());
    let x_end = v.x(c.end() as f32).min(content.right()).min(p.clip_rect().right());
    let mut x = x_start;
    while x < x_end {
        let t0 = v.tick_at(x) - c.start as f32 + c.offset as f32;
        let t1 = v.tick_at(x + 1.0) - c.start as f32 + c.offset as f32;
        let (mut b0, mut b1) = ((t0 * secs_per_tick * blocks_per_sec) as usize, (t1 * secs_per_tick * blocks_per_sec) as usize);
        if s.reverse {
            // A reversed sample plays from its end: mirror the peak index.
            let n = data.peaks.len();
            (b0, b1) = (n.saturating_sub(b1), n.saturating_sub(b0));
        }
        let b1 = b1.max(b0 + 1);
        let peak = data.peaks.get(b0.min(data.peaks.len())..b1.min(data.peaks.len())).map(|s| s.iter().fold(0f32, |m, v| m.max(*v))).unwrap_or(0.0);
        if peak > 0.0 {
            let h = peak.min(1.0) * content.height() * 0.5;
            p.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(1.0_f32, col));
        }
        x += 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_source_position() {
        let c = Clip { track: 0, start: 96, length: 384, offset: 48, source: ClipSource::Audio(0), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 };
        let mut clips = vec![c];
        split_clip(&mut clips, 0, 192);
        assert_eq!((clips[0].start, clips[0].length, clips[0].offset), (96, 96, 48));
        assert_eq!((clips[1].start, clips[1].length, clips[1].offset), (192, 288, 144));
    }
}
