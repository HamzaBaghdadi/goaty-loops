use crate::app::{App, PROJECT_EXT};
use crate::theme;
use crate::widgets::{self, Icon};
use goaty_engine::sample::is_audio_file;
use goaty_engine::Command;
use egui::{pos2, vec2, Rect, Sense, Stroke, Ui};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const ROW_H: f32 = 24.0;
const INDENT: f32 = 18.0;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    All,
    Project,
    Plugins,
}

pub struct State {
    roots: Vec<(String, PathBuf)>,
    cache: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
    expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    tab: Tab,
    filter: String,
}

impl State {
    pub fn new() -> Self {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let mut roots = Vec::new();
        // FL Studio's factory content, if installed.
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            if let Ok(rd) = std::fs::read_dir(PathBuf::from(pf).join("Image-Line")) {
                let mut installs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("Data").is_dir()).collect();
                installs.sort();
                if let Some(fl) = installs.last() {
                    for (name, sub) in [("Packs", "Data/Patches/Packs"), ("Demo projects", "Data/Demo projects")] {
                        let p = fl.join(sub);
                        if p.is_dir() {
                            roots.push((name.to_string(), p));
                        }
                    }
                }
            }
        }
        let user_fl = home.join("Documents").join("Image-Line").join("FL Studio");
        for (name, sub) in [("My projects", "Projects"), ("Recorded", "Audio/Recorded"), ("Rendered", "Audio/Rendered")] {
            let p = user_fl.join(sub);
            if p.is_dir() {
                roots.push((name.to_string(), p));
            }
        }
        let recorded = crate::app::recordings_dir();
        if recorded.is_dir() {
            roots.push(("Recordings".to_string(), recorded));
        }
        let legacy = goaty_engine::paths::legacy_recordings_dir();
        if legacy.is_dir() {
            roots.push(("Recordings (BeatCraft)".to_string(), legacy));
        }
        for (name, p) in [("Music", home.join("Music")), ("Desktop", home.join("Desktop")), ("Documents", home.join("Documents"))] {
            if p.is_dir() {
                roots.push((name.to_string(), p));
            }
        }
        Self { roots, cache: HashMap::new(), expanded: HashSet::new(), selected: None, tab: Tab::All, filter: String::new() }
    }

    fn list(&mut self, dir: &Path) -> Vec<(PathBuf, bool)> {
        self.cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                let mut v: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| {
                                let p = e.path();
                                let is_dir = p.is_dir();
                                (p, is_dir)
                            })
                            .filter(|(p, is_dir)| {
                                let hidden = p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false);
                                !hidden && (*is_dir || is_audio_file(p) || is_project(p))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.file_name().cmp(&b.0.file_name())));
                v
            })
            .clone()
    }
}

fn is_project(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("flp") | Some(PROJECT_EXT))
}

/// One tree row. Returns the response for the whole row.
fn row(ui: &mut Ui, depth: usize, icon: Icon, icon_col: egui::Color32, text: &str, selected: bool, open: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, theme::BROWSER_SEL);
    } else if open {
        p.rect_filled(rect, 0.0, theme::BROWSER_OPEN);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, theme::lighten(theme::BROWSER_BG, 0.04));
    }
    let x = rect.left() + 10.0 + depth as f32 * INDENT;
    widgets::paint_icon(p, Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(15.0, 15.0)), icon, icon_col);
    let font = theme::font(15.5);
    let text = widgets::fit_text(p, text, &font, rect.right() - x - 26.0);
    let col = if selected { theme::TEXT_BRIGHT } else { theme::BROWSER_TEXT };
    p.text(pos2(x + 20.0, rect.center().y), egui::Align2::LEFT_CENTER, text, font, col);
    resp
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    // Header line.
    let (hdr, _) = ui.allocate_exact_size(vec2(full.width(), 22.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(hdr, 0.0, theme::TITLE_BG);
    widgets::paint_icon(p, Rect::from_center_size(pos2(hdr.left() + 10.0, hdr.center().y), vec2(10.0, 10.0)), Icon::ArrowRight, theme::TEXT_DIM);
    p.text(pos2(hdr.left() + 22.0, hdr.center().y), egui::Align2::LEFT_CENTER, "Browser", theme::font(13.5), theme::TEXT_DIM);

    // Tab icons.
    let (tabs, _) = ui.allocate_exact_size(vec2(full.width(), 38.0), Sense::hover());
    ui.painter().rect_filled(tabs, 0.0, theme::darken(theme::BROWSER_BG, 0.15));
    let tab_defs = [(Tab::All, Icon::Folder, "All"), (Tab::Project, Icon::Project, "Current project"), (Tab::Plugins, Icon::Preset, "Plugin database")];
    for (i, (tab, icon, tip)) in tab_defs.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(tabs.left() + 6.0 + i as f32 * 46.0, tabs.top() + 5.0), vec2(40.0, 28.0));
        let resp = ui.interact(r, ui.id().with(("browser_tab", i)), Sense::click()).on_hover_text(tip);
        let active = app.browser.tab == tab;
        let p = ui.painter();
        if active {
            p.rect_filled(r, 3.0, theme::BUTTON);
            p.rect_stroke(r, 3.0, Stroke::new(1.0_f32, theme::TEXT_DIM), egui::StrokeKind::Inside);
        }
        widgets::paint_icon(p, r.shrink(6.0), icon, if active { theme::TEXT_BRIGHT } else { theme::TEXT_DIM });
        if resp.clicked() {
            app.browser.tab = tab;
        }
    }
    let add = Rect::from_min_size(pos2(tabs.right() - 34.0, tabs.top() + 5.0), vec2(28.0, 28.0));
    let resp = ui.interact(add, ui.id().with("browser_add"), Sense::click()).on_hover_text("Add a folder to the browser");
    widgets::paint_icon(ui.painter(), add.shrink(7.0), Icon::Plus, if resp.hovered() { theme::TEXT_BRIGHT } else { theme::TEXT_DIM });
    if resp.clicked() {
        if let Some(d) = rfd::FileDialog::new().pick_folder() {
            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| d.display().to_string());
            app.browser.roots.push((name, d));
        }
    }

    if app.browser.tab == Tab::All {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(egui::TextEdit::singleline(&mut app.browser.filter).hint_text("Search").desired_width(ui.available_width() - 64.0));
            if ui.small_button("Refresh").clicked() {
                app.browser.cache.clear();
            }
        });
        ui.add_space(2.0);
    }

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match app.browser.tab {
            Tab::All => {
                let roots = app.browser.roots.clone();
                for (name, path) in roots {
                    let open = app.browser.expanded.contains(&path);
                    let r = row(ui, 0, Icon::Folder, theme::BROWSER_ICON, &name, app.browser.selected.as_ref() == Some(&path), open);
                    if r.clicked() {
                        toggle(app, &path);
                    }
                    r.on_hover_text(path.display().to_string());
                    if open {
                        tree(app, ui, &path, 1);
                    }
                }
            }
            Tab::Project => project_tree(app, ui),
            Tab::Plugins => plugin_tree(app, ui),
        }
        ui.add_space(20.0);
    });
}

fn toggle(app: &mut App, path: &Path) {
    if !app.browser.expanded.remove(path) {
        app.browser.expanded.insert(path.to_path_buf());
    }
    app.browser.selected = Some(path.to_path_buf());
}

fn tree(app: &mut App, ui: &mut Ui, dir: &Path, depth: usize) {
    if depth > 14 {
        return;
    }
    let filter = app.browser.filter.to_lowercase();
    for (path, is_dir) in app.browser.list(dir) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let selected = app.browser.selected.as_ref() == Some(&path);
        if is_dir {
            let open = app.browser.expanded.contains(&path);
            if row(ui, depth, Icon::Folder, theme::BROWSER_ICON, &name, selected, open).clicked() {
                toggle(app, &path);
            }
            if open {
                tree(app, ui, &path, depth + 1);
            }
            continue;
        }
        if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
            continue;
        }
        let project = is_project(&path);
        let (icon, col) = if project { (Icon::Project, theme::ORANGE) } else { (Icon::Audio, theme::BROWSER_ICON) };
        let display = if project { name.clone() } else { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name.clone()) };
        let r = row(ui, depth, icon, col, &display, selected, false);
        if r.double_clicked() {
            app.engine.send(Command::StopPreview);
            if project {
                app.request(crate::app::Pending::Open(Some(path.clone())));
            } else {
                app.open_path(&path);
            }
        } else if r.clicked() {
            app.browser.selected = Some(path.clone());
            if !project {
                match app.load_sample_cached(&path) {
                    Ok(d) => {
                        app.status = format!("{name}  ({:.2} s)", d.duration_secs());
                        app.engine.send(Command::PreviewSample(d));
                    }
                    Err(e) => app.status = format!("Could not load {name}: {e}"),
                }
            }
        }
        r.on_hover_text("Click to preview, double-click to add to the channel rack / open");
    }
}

fn project_tree(app: &mut App, ui: &mut Ui) {
    let sections = [("Channels", Icon::ChannelRack), ("Patterns", Icon::Pattern), ("Mixer", Icon::Mixer)];
    for (name, icon) in sections {
        let key = PathBuf::from(format!("::project::{name}"));
        let open = !app.browser.expanded.contains(&key); // open by default
        if row(ui, 0, icon, theme::BROWSER_ICON, name, false, open).clicked() {
            if open {
                app.browser.expanded.insert(key.clone());
            } else {
                app.browser.expanded.remove(&key);
            }
        }
        if !open {
            continue;
        }
        match name {
            "Channels" => {
                for i in 0..app.project.channels.len() {
                    let ch = &app.project.channels[i];
                    let label = format!("{}  ({})", ch.name, ch.instrument.kind_name());
                    let r = row(ui, 1, Icon::Preset, theme::rgb(ch.color), &label, app.sel_channel == i, false);
                    if r.clicked() {
                        app.sel_channel = i;
                    }
                    if r.double_clicked() {
                        app.settings_channel = Some(i);
                    }
                }
            }
            "Patterns" => {
                for i in 0..app.project.patterns.len() {
                    let pat = &app.project.patterns[i];
                    let label = format!("{}  ({} notes)", pat.name, pat.notes.len());
                    if row(ui, 1, Icon::Pattern, theme::rgb(pat.color), &label, app.cur_pattern == i, false).clicked() {
                        app.cur_pattern = i;
                    }
                }
            }
            _ => {
                for i in 0..app.project.mixer.len() {
                    let t = &app.project.mixer[i];
                    if t.effects.is_empty() && t.name.starts_with("Insert ") {
                        continue;
                    }
                    let label = format!("{}{}", t.name, if t.effects.is_empty() { String::new() } else { format!("  ({} fx)", t.effects.len()) });
                    if row(ui, 1, Icon::Mixer, theme::BROWSER_ICON, &label, app.mixer_ui.selected == i && app.show_mixer, false).clicked() {
                        app.mixer_ui.selected = i;
                        app.show_mixer = true;
                    }
                }
            }
        }
    }
}

fn plugin_tree(app: &mut App, ui: &mut Ui) {
    if app.plugins.scanning() {
        ui.label(egui::RichText::new(app.plugins.scan_status.clone().unwrap_or_else(|| "Scanning...".into())).color(theme::HINT));
    }
    let sections: [(&str, bool); 2] = [("Installed instruments", true), ("Installed effects", false)];
    for (name, instruments) in sections {
        let key = PathBuf::from(format!("::plugins::{name}"));
        let open = !app.browser.expanded.contains(&key);
        if row(ui, 0, Icon::Folder, theme::BROWSER_ICON, name, false, open).clicked() {
            if open {
                app.browser.expanded.insert(key.clone());
            } else {
                app.browser.expanded.remove(&key);
            }
        }
        if !open {
            continue;
        }
        let list: Vec<_> = app.plugins.db.plugins.iter().filter(|p| p.instrument == instruments).cloned().collect();
        for info in list {
            let r = row(ui, 1, Icon::Preset, if instruments { theme::ORANGE } else { theme::BROWSER_ICON }, &info.name, false, false)
                .on_hover_text(format!("{}\nDouble-click to add {}", info.vendor, if instruments { "to the channel rack" } else { "to the selected mixer track" }));
            if r.double_clicked() {
                if instruments {
                    app.add_plugin_channel(&info);
                } else {
                    let t = app.mixer_ui.selected.min(app.project.mixer.len().saturating_sub(1));
                    app.add_plugin_effect(t, &info);
                    app.show_mixer = true;
                }
            }
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        if ui.button("Plugin manager...").clicked() {
            app.plugins.show_manager = true;
        }
    });
}
