use super::{note_tools, snap_combo, snap_floor, snap_round};
use crate::app::{App, Focus};
use crate::{theme, widgets};
use goaty_engine::*;
use egui::{pos2, vec2, Align2, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Ui};

const KB_W: f32 = 58.0;
const RULER_H: f32 = 18.0;
const VEL_H: f32 = 64.0;

pub struct State {
    scroll_x: f32,
    scroll_row: f32,
    zoom: f32,
    row_h: f32,
    pub snap: u32,
    last_len: u32,
    drag: Option<Drag>,
    preview: Option<(usize, u8)>,
    vel_active: bool,
    deleting: bool,
    /// Indices of selected notes in the current pattern.
    pub selected: std::collections::BTreeSet<usize>,
    /// Ctrl+drag selection box start (screen position).
    marquee: Option<Pos2>,
    /// (pattern, channel) the selection belongs to.
    sel_owner: (usize, usize),
}

impl Default for State {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_row: (127 - 84) as f32,
            zoom: 0.9,
            row_h: 14.0,
            snap: STEP_TICKS,
            last_len: STEP_TICKS,
            drag: None,
            preview: None,
            vel_active: false,
            deleting: false,
            selected: Default::default(),
            marquee: None,
            sel_owner: (usize::MAX, usize::MAX),
        }
    }
}

#[derive(Clone)]
enum DragMode {
    Move { grab: f32 },
    Resize,
    /// Moves every selected note; `origins` holds (index, start, key) at drag start.
    MoveGroup { grab_tick: f32, grab_key: i32, origins: Vec<(usize, u32, u8)> },
}

#[derive(Clone)]
struct Drag {
    idx: usize,
    mode: DragMode,
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let mut open = app.show_piano;
    let pat = app.project.patterns.get(app.cur_pattern).map(|p| p.name.clone()).unwrap_or_default();
    let ch = app.project.channels.get(app.sel_channel).map(|c| c.name.clone()).unwrap_or_else(|| "no channel".into());
    widgets::fl_window(ctx, "piano_roll", &format!("Piano roll - {ch}  \u{203A}  {pat}"), &mut open, [340.0, 170.0], [980.0, 520.0], |ui| body(app, ui));
    app.show_piano = open;
}

fn body(app: &mut App, ui: &mut Ui) {
    if app.project.channels.is_empty() {
        ui.label("Add a channel first.");
        return;
    }
    toolbar(app, ui);
    canvas(app, ui);
}

fn toolbar(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let sel = app.sel_channel;
        egui::ComboBox::from_id_salt("pr_channel").width(130.0).selected_text(app.project.channels[sel].name.clone()).show_ui(ui, |ui| {
            for (i, c) in app.project.channels.iter().enumerate() {
                ui.selectable_value(&mut app.sel_channel, i, &c.name);
            }
        });
        ui.label(egui::RichText::new("Snap").color(theme::TEXT_DIM).size(11.0));
        snap_combo(ui, "pr_snap", &mut app.piano.snap);
        ui.menu_button("Tools", |ui| {
            let hint = "Acts on the selection, or all notes of this channel";
            for (tool, label, keys) in [
                (Tool::Quantize, "Quick quantize", "Ctrl+Q"),
                (Tool::QuantizeAll, "Quantize starts and lengths", "Ctrl+Shift+Q"),
                (Tool::Chop, "Chop by snap", "Alt+U"),
                (Tool::Glue, "Glue", "Alt+G"),
                (Tool::Legato, "Legato", "Ctrl+L"),
                (Tool::VelUp, "Velocity +10%", ""),
                (Tool::VelDown, "Velocity -10%", ""),
            ] {
                if ui.add(egui::Button::new(label).shortcut_text(keys)).on_hover_text(hint).clicked() {
                    apply_tool(app, tool);
                    ui.close_menu();
                }
            }
        });
        if ui.button("Clear").on_hover_text("Delete this channel's notes in the pattern").clicked() {
            app.checkpoint();
            let ch = app.sel_channel;
            app.project.patterns[app.cur_pattern].notes.retain(|n| n.channel != ch);
            app.changed();
        }
        ui.separator();
        if ui.small_button("-").on_hover_text("Zoom out (Ctrl+wheel)").clicked() {
            app.piano.zoom = (app.piano.zoom / 1.25).max(0.1);
        }
        if ui.small_button("+").on_hover_text("Zoom in (Ctrl+wheel)").clicked() {
            app.piano.zoom = (app.piano.zoom * 1.25).min(8.0);
        }
        ui.label(egui::RichText::new("Left: add/move/resize  Right: delete").color(theme::TEXT_DIM).size(11.0));
    });
}

struct View {
    grid: Rect,
    scroll_x: f32,
    scroll_row: f32,
    zoom: f32,
    row_h: f32,
}

impl View {
    fn x(&self, tick: f32) -> f32 {
        self.grid.left() + (tick - self.scroll_x) * self.zoom
    }
    fn tick_at(&self, x: f32) -> f32 {
        (x - self.grid.left()) / self.zoom + self.scroll_x
    }
    fn y(&self, key: u8) -> f32 {
        self.grid.top() + ((127 - key as i32) as f32 - self.scroll_row) * self.row_h
    }
    fn key_at(&self, y: f32) -> u8 {
        let row = ((y - self.grid.top()) / self.row_h + self.scroll_row).floor() as i32;
        (127 - row).clamp(0, 127) as u8
    }
    fn note_rect(&self, n: &Note) -> Rect {
        let len = if n.length == 0 { STEP_TICKS } else { n.length };
        Rect::from_min_max(pos2(self.x(n.start as f32), self.y(n.key)), pos2(self.x((n.start + len) as f32), self.y(n.key) + self.row_h))
    }
}

fn canvas(app: &mut App, ui: &mut Ui) {
    let size = vec2(ui.available_width(), ui.available_height().max(220.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let kb = Rect::from_min_max(pos2(rect.left(), rect.top() + RULER_H), pos2(rect.left() + KB_W, rect.bottom() - VEL_H));
    let ruler = Rect::from_min_max(pos2(rect.left() + KB_W, rect.top()), pos2(rect.right(), rect.top() + RULER_H));
    let grid = Rect::from_min_max(pos2(rect.left() + KB_W, rect.top() + RULER_H), pos2(rect.right(), rect.bottom() - VEL_H));
    let vel = Rect::from_min_max(pos2(rect.left() + KB_W, rect.bottom() - VEL_H + 4.0), rect.max);

    // Scrolling and zooming.
    let st = &mut app.piano;
    if resp.hovered() {
        let (scroll, zoom_delta, shift, pointer) = ui.input(|i| (i.raw_scroll_delta, i.zoom_delta(), i.modifiers.shift, i.pointer.hover_pos()));
        if zoom_delta != 1.0 {
            let anchor = pointer.map(|p| (p.x - grid.left()) / st.zoom + st.scroll_x).unwrap_or(st.scroll_x);
            st.zoom = (st.zoom * zoom_delta).clamp(0.1, 8.0);
            if let Some(p) = pointer {
                st.scroll_x = anchor - (p.x - grid.left()) / st.zoom;
            }
        }
        if shift {
            st.scroll_x -= (scroll.y + scroll.x) / st.zoom;
        } else {
            st.scroll_x -= scroll.x / st.zoom;
            st.scroll_row -= scroll.y / st.row_h;
        }
    }
    let visible_rows = grid.height() / st.row_h;
    st.scroll_row = st.scroll_row.clamp(0.0, (128.0 - visible_rows).max(0.0));
    st.scroll_x = st.scroll_x.max(0.0);

    let v = View { grid, scroll_x: st.scroll_x, scroll_row: st.scroll_row, zoom: st.zoom, row_h: st.row_h };
    interact(app, ui, &resp, &v, kb, ruler, vel);
    shortcuts(app, ui);
    paint(app, ui, rect, &v, kb, ruler, vel);
}

fn hit_note(notes: &[Note], ch: usize, v: &View, pos: Pos2) -> Option<(usize, bool)> {
    notes.iter().enumerate().rev().filter(|(_, n)| n.channel == ch).find_map(|(i, n)| {
        let r = v.note_rect(n);
        r.contains(pos).then(|| (i, pos.x > r.right() - 6.0_f32.min(r.width() * 0.3)))
    })
}

#[allow(clippy::too_many_arguments)]
fn interact(app: &mut App, ui: &Ui, resp: &egui::Response, v: &View, kb: Rect, ruler: Rect, vel: Rect) {
    let (pointer, pp, pd, sp, sd, mods) = ui.input(|i| {
        (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.secondary_pressed(), i.pointer.secondary_down(), i.modifiers)
    });
    let ch = app.sel_channel;
    let pat = app.cur_pattern;
    let snap = app.piano.snap;

    // The selection belongs to one pattern/channel; drop stale indices.
    if app.piano.sel_owner != (pat, ch) {
        app.piano.selected.clear();
        app.piano.sel_owner = (pat, ch);
    }
    let notes_len = app.project.patterns[pat].notes.len();
    let notes_ref = &app.project.patterns[pat].notes;
    app.piano.selected.retain(|&i| i < notes_len && notes_ref[i].channel == ch);

    if let Some(pos) = pointer {
        let hovered = resp.hovered();
        if hovered && (pp || sp) {
            app.focus = Focus::PianoRoll;
        }
        if hovered && v.grid.contains(pos) {
            let hit = hit_note(&app.project.patterns[pat].notes, ch, v, pos);
            match hit {
                Some((_, true)) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                Some(_) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
                None if mods.ctrl => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
                None => {}
            }
            if pp {
                if mods.ctrl && hit.is_none() {
                    // Start a selection box.
                    app.piano.marquee = Some(pos);
                    if !mods.shift {
                        app.piano.selected.clear();
                    }
                } else if let (true, Some((idx, _))) = (mods.shift, hit) {
                    if !app.piano.selected.remove(&idx) {
                        app.piano.selected.insert(idx);
                    }
                } else {
                    app.checkpoint();
                    let drag = match hit {
                        Some((idx, true)) => Drag { idx, mode: DragMode::Resize },
                        Some((idx, false)) if app.piano.selected.contains(&idx) && app.piano.selected.len() > 1 => {
                            let notes = &app.project.patterns[pat].notes;
                            let origins = app.piano.selected.iter().map(|&i| (i, notes[i].start, notes[i].key)).collect();
                            Drag { idx, mode: DragMode::MoveGroup { grab_tick: v.tick_at(pos.x), grab_key: v.key_at(pos.y) as i32, origins } }
                        }
                        Some((idx, false)) => {
                            app.piano.selected.clear();
                            let n = app.project.patterns[pat].notes[idx];
                            app.piano.last_len = n.length.max(1);
                            Drag { idx, mode: DragMode::Move { grab: v.tick_at(pos.x) - n.start as f32 } }
                        }
                        None => {
                            app.piano.selected.clear();
                            let start = snap_floor(v.tick_at(pos.x), snap);
                            let key = v.key_at(pos.y);
                            let notes = &mut app.project.patterns[pat].notes;
                            notes.push(Note { channel: ch, key, start, length: app.piano.last_len, velocity: DEFAULT_VELOCITY, pan: 0.0 });
                            let idx = notes.len() - 1;
                            app.changed();
                            Drag { idx, mode: DragMode::Move { grab: v.tick_at(pos.x) - start as f32 } }
                        }
                    };
                    let key = app.project.patterns[pat].notes[drag.idx].key;
                    app.preview_on(ch, key);
                    app.piano.preview = Some((ch, key));
                    app.piano.drag = Some(drag);
                }
            }
            if sp {
                app.checkpoint();
                app.piano.deleting = true;
                app.piano.selected.clear();
            }
        }
        if app.piano.deleting && sd && v.grid.contains(pos) {
            if let Some((idx, _)) = hit_note(&app.project.patterns[pat].notes, ch, v, pos) {
                app.project.patterns[pat].notes.remove(idx);
                app.changed();
            }
        }

        if let (Some(drag), true) = (app.piano.drag.clone(), pd) {
            match drag.mode {
                DragMode::Move { grab } => {
                    if let Some(n) = app.project.patterns[pat].notes.get_mut(drag.idx) {
                        let start = snap_round(v.tick_at(pos.x) - grab, snap);
                        let key = v.key_at(pos.y);
                        if start != n.start || key != n.key {
                            let old_key = n.key;
                            n.start = start;
                            n.key = key;
                            app.changed();
                            if key != old_key {
                                app.preview_off(ch, old_key);
                                app.preview_on(ch, key);
                                app.piano.preview = Some((ch, key));
                            }
                        }
                    }
                }
                DragMode::Resize => {
                    if let Some(n) = app.project.patterns[pat].notes.get_mut(drag.idx) {
                        let end = snap_round(v.tick_at(pos.x), snap);
                        let len = end.saturating_sub(n.start).max(snap.max(1));
                        if len != n.length {
                            n.length = len;
                            app.piano.last_len = len;
                            app.changed();
                        }
                    }
                }
                DragMode::MoveGroup { grab_tick, grab_key, origins } => {
                    let s = snap.max(1) as f32;
                    let min_start = origins.iter().map(|o| o.1).min().unwrap_or(0) as f32;
                    let dt = (((v.tick_at(pos.x) - grab_tick) / s).round() * s).max(-min_start) as i64;
                    let min_key = origins.iter().map(|o| o.2).min().unwrap_or(0) as i32;
                    let max_key = origins.iter().map(|o| o.2).max().unwrap_or(0) as i32;
                    let dk = (v.key_at(pos.y) as i32 - grab_key).clamp(-min_key, 127 - max_key);
                    let notes = &mut app.project.patterns[pat].notes;
                    let mut moved = false;
                    for &(i, start, key) in &origins {
                        if let Some(n) = notes.get_mut(i) {
                            let (ns, nk) = ((start as i64 + dt) as u32, (key as i32 + dk) as u8);
                            if n.start != ns || n.key != nk {
                                n.start = ns;
                                n.key = nk;
                                moved = true;
                            }
                        }
                    }
                    if moved {
                        app.changed();
                    }
                }
            }
        }

        // Keyboard: click to audition.
        if resp.hovered() && kb.contains(pos) && pp {
            let key = v.key_at(pos.y);
            app.preview_on(ch, key);
            app.piano.preview = Some((ch, key));
        }

        // Velocity lane (only selected notes when there is a selection).
        if resp.hovered() && vel.contains(pos) && pp {
            app.checkpoint();
            app.piano.vel_active = true;
        }
        if app.piano.vel_active && pd {
            let value = (1.0 - (pos.y - vel.top()) / vel.height()).clamp(0.0, 1.0);
            let sel = app.piano.selected.clone();
            let mut any = false;
            for (i, n) in app.project.patterns[pat].notes.iter_mut().enumerate().filter(|(_, n)| n.channel == ch) {
                if (sel.is_empty() || sel.contains(&i)) && (v.x(n.start as f32) - pos.x).abs() <= 5.0 {
                    n.velocity = value;
                    any = true;
                }
            }
            if any {
                app.changed();
            }
        }

        // Ruler: seek.
        if resp.hovered() && ruler.contains(pos) && pp && !app.song_mode {
            app.engine.transport().seek(snap_floor(v.tick_at(pos.x), snap) as f64);
        }
    }

    if !pd {
        if let (Some(start), Some(end)) = (app.piano.marquee.take(), pointer) {
            let r = Rect::from_two_pos(start, end);
            for (i, n) in app.project.patterns[pat].notes.iter().enumerate() {
                if n.channel == ch && v.note_rect(n).intersects(r) {
                    app.piano.selected.insert(i);
                }
            }
        }
        app.piano.drag = None;
        app.piano.vel_active = false;
        if let Some((c, k)) = app.piano.preview.take() {
            app.preview_off(c, k);
        }
    }
    if !sd {
        app.piano.deleting = false;
    }
}

/// Editing shortcuts while the piano roll has focus.
fn shortcuts(app: &mut App, ui: &Ui) {
    if app.focus != Focus::PianoRoll || ui.ctx().wants_keyboard_input() {
        return;
    }
    use egui::Key;
    let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
    let (pat, ch) = (app.cur_pattern, app.sel_channel);
    let sel: Vec<usize> = app.piano.selected.iter().copied().collect();
    let notes = &app.project.patterns[pat].notes;
    let targets: Vec<usize> = if sel.is_empty() { (0..notes.len()).filter(|&i| notes[i].channel == ch).collect() } else { sel.clone() };

    if cmd && pressed(Key::A) {
        app.piano.selected = (0..notes.len()).filter(|&i| notes[i].channel == ch).collect();
        return;
    }
    if cmd && (pressed(Key::C) || pressed(Key::X)) {
        app.note_clipboard = targets.iter().map(|&i| notes[i]).collect();
        let cut = pressed(Key::X);
        app.status = format!("{} {} note(s)", if cut { "Cut" } else { "Copied" }, targets.len());
        if cut {
            delete_notes(app, &targets);
        }
        return;
    }
    if cmd && pressed(Key::V) && !app.note_clipboard.is_empty() {
        app.checkpoint();
        let notes = &mut app.project.patterns[pat].notes;
        let first = notes.len();
        notes.extend(app.note_clipboard.iter().map(|n| Note { channel: ch, ..*n }));
        app.piano.selected = (first..notes.len()).collect();
        app.changed();
        return;
    }
    if cmd && pressed(Key::B) && !targets.is_empty() {
        // Duplicate right after the selection, rounded up to whole beats.
        app.checkpoint();
        let beat = app.project.beat_ticks();
        let notes = &mut app.project.patterns[pat].notes;
        let start = targets.iter().map(|&i| notes[i].start).min().unwrap_or(0);
        let end = targets.iter().map(|&i| notes[i].end()).max().unwrap_or(0);
        let span = (end - start).div_ceil(beat).max(1) * beat;
        let first = notes.len();
        let copies: Vec<Note> = targets.iter().map(|&i| Note { start: notes[i].start + span, ..notes[i] }).collect();
        notes.extend(copies);
        app.piano.selected = (first..notes.len()).collect();
        app.changed();
        return;
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let tool = if cmd && pressed(Key::Q) {
        Some(if shift { Tool::QuantizeAll } else { Tool::Quantize })
    } else if cmd && pressed(Key::L) {
        Some(Tool::Legato)
    } else if alt && pressed(Key::U) {
        Some(Tool::Chop)
    } else if alt && pressed(Key::G) {
        Some(Tool::Glue)
    } else {
        None
    };
    if let Some(tool) = tool {
        apply_tool(app, tool);
        return;
    }
    if (pressed(Key::Delete) || pressed(Key::Backspace)) && !sel.is_empty() {
        delete_notes(app, &sel);
        return;
    }
    // Arrows: transpose (Shift = octave) or move by the snap.
    let (up, down, left, right) = (pressed(Key::ArrowUp), pressed(Key::ArrowDown), pressed(Key::ArrowLeft), pressed(Key::ArrowRight));
    if !sel.is_empty() && (up || down || left || right) {
        app.checkpoint();
        let step = if shift { 12 } else { 1 };
        let dk: i32 = if up { step } else if down { -step } else { 0 };
        let snap = app.piano.snap.max(1) as i64;
        let dt: i64 = if right { snap } else if left { -snap } else { 0 };
        let notes = &mut app.project.patterns[pat].notes;
        let ok = sel.iter().all(|&i| {
            let n = notes[i];
            (0..=127).contains(&(n.key as i32 + dk)) && n.start as i64 + dt >= 0
        });
        if ok {
            for &i in &sel {
                notes[i].key = (notes[i].key as i32 + dk) as u8;
                notes[i].start = (notes[i].start as i64 + dt) as u32;
            }
            app.changed();
        }
    }
}

#[derive(Clone, Copy)]
enum Tool {
    Quantize,
    QuantizeAll,
    Chop,
    Glue,
    Legato,
    VelUp,
    VelDown,
}

/// Runs a note tool on the selection (or every note of the channel).
fn apply_tool(app: &mut App, tool: Tool) {
    let (pat, ch, snap) = (app.cur_pattern, app.sel_channel, app.piano.snap.max(1));
    let notes = &app.project.patterns[pat].notes;
    let had_selection = !app.piano.selected.is_empty();
    let targets: Vec<usize> = if had_selection {
        app.piano.selected.iter().copied().filter(|&i| i < notes.len()).collect()
    } else {
        (0..notes.len()).filter(|&i| notes[i].channel == ch).collect()
    };
    if targets.is_empty() {
        return;
    }
    app.checkpoint();
    let notes = &mut app.project.patterns[pat].notes;
    let new_idx = match tool {
        Tool::Quantize | Tool::QuantizeAll => {
            note_tools::quantize(notes, &targets, snap, 1.0, matches!(tool, Tool::QuantizeAll));
            None
        }
        Tool::Chop => Some(note_tools::chop(notes, &targets, snap)),
        Tool::Glue => Some(note_tools::glue(notes, &targets)),
        Tool::Legato => {
            note_tools::legato(notes, &targets);
            None
        }
        Tool::VelUp | Tool::VelDown => {
            note_tools::scale_velocity(notes, &targets, if matches!(tool, Tool::VelUp) { 1.1 } else { 1.0 / 1.1 });
            None
        }
    };
    // Chop and glue reorder notes, so the selection follows the new indices.
    if let Some(idx) = new_idx {
        app.piano.selected = if had_selection { idx.into_iter().collect() } else { Default::default() };
    }
    app.changed();
}

fn delete_notes(app: &mut App, indices: &[usize]) {
    app.checkpoint();
    let mut idx = indices.to_vec();
    idx.sort_unstable_by(|a, b| b.cmp(a));
    let notes = &mut app.project.patterns[app.cur_pattern].notes;
    for i in idx {
        if i < notes.len() {
            notes.remove(i);
        }
    }
    app.piano.selected.clear();
    app.changed();
}

#[allow(clippy::too_many_arguments)]
fn paint(app: &App, ui: &Ui, rect: Rect, v: &View, kb: Rect, ruler: Rect, vel: Rect) {
    let p = ui.painter_at(rect);
    let grid = v.grid;
    p.rect_filled(rect, 0.0, theme::PR_ROW_WHITE);

    // Rows + keyboard.
    let first = v.scroll_row.floor() as i32;
    let rows = (grid.height() / v.row_h).ceil() as i32 + 1;
    for r in first..first + rows {
        let key = 127 - r;
        if !(0..=127).contains(&key) {
            continue;
        }
        let key = key as u8;
        let y = v.y(key);
        let row = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + v.row_h));
        p.rect_filled(row, 0.0, if widgets::is_black_key(key) { theme::PR_ROW_BLACK } else { theme::PR_ROW_WHITE });
        if key % 12 == 0 {
            p.line_segment([pos2(grid.left(), row.bottom()), pos2(grid.right(), row.bottom())], Stroke::new(1.0_f32, theme::GRID_BAR));
        }
        let krect = Rect::from_min_max(pos2(kb.left(), y), pos2(kb.right(), y + v.row_h));
        if widgets::is_black_key(key) {
            p.rect_filled(krect, 0.0, theme::KEY_WHITE);
            p.rect_filled(Rect::from_min_max(krect.min, pos2(kb.left() + KB_W * 0.62, krect.bottom())), 1.0, theme::KEY_BLACK);
        } else {
            p.rect_filled(krect, 0.0, theme::KEY_WHITE);
            p.line_segment([pos2(kb.left(), krect.bottom()), pos2(kb.right(), krect.bottom())], Stroke::new(0.5_f32, theme::TEXT_DIM));
        }
        if key % 12 == 0 {
            p.text(pos2(kb.right() - 3.0, krect.center().y), Align2::RIGHT_CENTER, widgets::key_name(key), FontId::proportional(9.5), theme::WINDOW_BG);
        }
    }

    // Vertical grid lines.
    let bar = app.project.bar_ticks();
    let beat = app.project.beat_ticks();
    let t0 = (v.scroll_x / STEP_TICKS as f32).floor() as u32 * STEP_TICKS;
    let t1 = v.tick_at(grid.right()) as u32 + STEP_TICKS;
    let step_px = STEP_TICKS as f32 * v.zoom;
    let mut t = t0;
    while t <= t1 {
        let x = v.x(t as f32);
        let (col, show) = if t % bar == 0 {
            (theme::GRID_BAR, true)
        } else if t % beat == 0 {
            (theme::GRID_BEAT, true)
        } else {
            (theme::GRID_LINE, step_px >= 6.0)
        };
        if show {
            p.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0_f32, col));
        }
        t += STEP_TICKS;
    }
    p.rect_filled(ruler, 0.0, theme::darken(theme::RACK_NAME, 0.15));
    let mut t = t0 - t0 % bar;
    while t <= t1 {
        let x = v.x(t as f32);
        if x >= ruler.left() {
            p.text(pos2(x + 3.0, ruler.center().y), Align2::LEFT_CENTER, format!("{}", t / bar + 1), FontId::proportional(10.5), theme::TEXT);
        }
        t += bar;
    }

    // Area beyond the pattern end.
    let end_x = v.x(app.project.pattern_length(app.cur_pattern) as f32);
    if end_x < grid.right() {
        p.rect_filled(Rect::from_min_max(pos2(end_x.max(grid.left()), grid.top()), grid.max), 0.0, theme::darken(theme::PR_ROW_BLACK, 0.3).gamma_multiply(0.6));
    }

    // Notes.
    let pat = &app.project.patterns[app.cur_pattern];
    let ch = app.sel_channel;
    let note_col = theme::PR_NOTE;

    let grid_p = p.with_clip_rect(grid);
    for n in pat.notes.iter().filter(|n| n.channel != ch) {
        grid_p.rect_filled(v.note_rect(n).shrink(1.0), 2.0, theme::RACK_NAME);
    }
    for (i, n) in pat.notes.iter().enumerate().filter(|(_, n)| n.channel == ch) {
        let r = v.note_rect(n);
        let selected = app.piano.selected.contains(&i);
        let base = if selected { theme::lighten(note_col, 0.35) } else { note_col };
        let fill = theme::darken(base, 0.35 * (1.0 - n.velocity));
        grid_p.rect_filled(r.shrink(0.5), 2.0, fill);
        let outline = if selected { Stroke::new(1.5_f32, theme::TEXT_BRIGHT) } else { Stroke::new(1.0_f32, theme::darken(note_col, 0.5)) };
        grid_p.rect_stroke(r.shrink(0.5), 2.0, outline, egui::StrokeKind::Inside);
        if r.width() > 26.0 {
            grid_p.text(pos2(r.left() + 3.0, r.center().y), Align2::LEFT_CENTER, widgets::key_name(n.key), FontId::proportional(9.0), theme::WINDOW_BG);
        }
    }

    // Velocity lane.
    p.rect_filled(Rect::from_min_max(pos2(rect.left(), vel.top() - 4.0), rect.max), 0.0, theme::WINDOW_BG);
    p.rect_filled(vel, 0.0, theme::PR_ROW_BLACK);
    p.text(pos2(rect.left() + 4.0, vel.center().y), Align2::LEFT_CENTER, "Velocity", FontId::proportional(10.0), theme::TEXT_DIM);
    let vel_p = p.with_clip_rect(vel);
    for n in pat.notes.iter().filter(|n| n.channel == ch) {
        let x = v.x(n.start as f32);
        let top = vel.bottom() - n.velocity * (vel.height() - 4.0);
        vel_p.line_segment([pos2(x, vel.bottom()), pos2(x, top)], Stroke::new(2.0_f32, note_col));
        vel_p.circle_filled(pos2(x, top), 3.0, note_col);
    }

    // Selection box.
    if let (Some(start), Some(end)) = (app.piano.marquee, ui.input(|i| i.pointer.interact_pos())) {
        let r = Rect::from_two_pos(start, end);
        grid_p.rect_filled(r, 0.0, theme::PR_NOTE.gamma_multiply(0.12));
        grid_p.rect_stroke(r, 0.0, Stroke::new(1.0_f32, theme::PR_NOTE), egui::StrokeKind::Inside);
    }
    // Playhead.
    if app.is_playing() && !app.song_mode {
        let x = v.x(app.play_position() as f32);
        if x >= grid.left() && x <= grid.right() {
            p.line_segment([pos2(x, ruler.top()), pos2(x, grid.bottom())], Stroke::new(1.5_f32, theme::PLAYHEAD));
        }
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, theme::RACK_NAME), egui::StrokeKind::Inside);
}
