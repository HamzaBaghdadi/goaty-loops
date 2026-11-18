//! App-side plugin management: the scanned plugin database, background
//! scanning in child processes, and plugin editor windows.

use crate::{theme, widgets};
use goaty_engine::plugin::{default_vst3_dirs, PluginDb, PluginInfo, PluginInstance};
use goaty_engine::Project;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SCAN_TIMEOUT: Duration = Duration::from_secs(45);

enum ScanMsg {
    Progress(String),
    Done(PluginDb, usize),
}

pub struct PluginHost {
    pub db: PluginDb,
    scan: Option<Receiver<ScanMsg>>,
    pub scan_status: Option<String>,
    pub show_manager: bool,
    pub main_hwnd: Option<isize>,
    #[cfg(windows)]
    editors: Vec<goaty_engine::plugin_editor::PluginEditor>,
}

/// Scans one bundle in a child process so a crashing plugin can't take the app down.
fn scan_in_child(path: &Path) -> anyhow::Result<Vec<PluginInfo>> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe).arg("--scan-plugin").arg(path).stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null()).spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > SCAN_TIMEOUT {
            let _ = child.kill();
            anyhow::bail!("timed out while scanning");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let out = reader.join().unwrap_or_default();
    for line in out.lines() {
        if let Some(json) = line.strip_prefix("BCSCAN:") {
            return Ok(serde_json::from_str(json)?);
        }
        if let Some(err) = line.strip_prefix("BCERR:") {
            anyhow::bail!("{err}");
        }
    }
    anyhow::bail!("the plugin crashed while being scanned")
}

impl PluginHost {
    pub fn new() -> Self {
        let mut h = Self {
            db: PluginDb::load(),
            scan: None,
            scan_status: None,
            show_manager: false,
            main_hwnd: None,
            #[cfg(windows)]
            editors: Vec::new(),
        };
        h.start_scan(false);
        h
    }

    /// Scans new/changed plugins in the background (`full` rescans everything).
    pub fn start_scan(&mut self, full: bool) {
        if self.scan.is_some() {
            return;
        }
        let mut db = if full { PluginDb::default() } else { self.db.clone() };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let dirs = default_vst3_dirs();
            let n = db.refresh(&dirs, |p| {
                let _ = tx.send(ScanMsg::Progress(p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()));
                scan_in_child(p)
            });
            if n > 0 {
                let _ = db.save();
            }
            let _ = tx.send(ScanMsg::Done(db, n));
        });
        self.scan = Some(rx);
    }

    /// True while any plugin editor window is open (its edits are polled each frame).
    pub fn has_open_editors(&self) -> bool {
        !self.editors.is_empty()
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// Call once per frame. Returns a status message when a scan finishes.
    pub fn poll(&mut self) -> Option<String> {
        let rx = self.scan.as_ref()?;
        let mut result = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                ScanMsg::Progress(name) => self.scan_status = Some(format!("Scanning {name}...")),
                ScanMsg::Done(db, n) => result = Some((db, n)),
            }
        }
        let (db, n) = result?;
        self.db = db;
        self.scan = None;
        self.scan_status = None;
        Some(if n > 0 {
            format!("Plugin scan finished: {} plugins ({} failed)", self.db.plugins.len(), self.db.failed.len())
        } else {
            format!("{} plugins available", self.db.plugins.len())
        })
    }

    /// Opens (or focuses) the editor for a plugin instance.
    pub fn open_editor(&mut self, inst: Arc<PluginInstance>, title: &str) -> Result<(), String> {
        #[cfg(windows)]
        {
            if let Some(e) = self.editors.iter().find(|e| Arc::ptr_eq(&e.instance, &inst)) {
                e.focus();
                return Ok(());
            }
            let e = goaty_engine::plugin_editor::PluginEditor::open(inst, title, self.main_hwnd).map_err(|e| e.to_string())?;
            self.editors.push(e);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (inst, title);
            Err("Plugin editors are only supported on Windows so far".into())
        }
    }

    /// Closes editors whose window was closed or whose plugin left the project.
    pub fn sync_editors(&mut self, project: &mut Project) {
        #[cfg(windows)]
        {
            let live: Vec<*const PluginInstance> = project.plugins_mut().iter().filter_map(|p| p.instance.as_ref().map(Arc::as_ptr)).collect();
            self.editors.retain(|e| e.is_open() && live.contains(&Arc::as_ptr(&e.instance)));
        }
        #[cfg(not(windows))]
        let _ = project;
    }

    pub fn manager_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_manager;
        let mut rescan = false;
        let db = &self.db;
        let status = self.scan_status.clone();
        widgets::fl_window(ctx, "plugin_manager", "Plugin manager", &mut open, [380.0, 140.0], [620.0, 460.0], |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} instruments, {} effects", db.instruments().count(), db.effects().count())).color(theme::TEXT_BRIGHT));
                if let Some(s) = &status {
                    ui.spinner();
                    ui.label(egui::RichText::new(s).color(theme::HINT));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Rescan all").clicked() {
                        rescan = true;
                    }
                });
            });
            ui.label(egui::RichText::new(format!("Folders: {}", default_vst3_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(";  "))).size(12.5).color(theme::TEXT_DIM));
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("plugin_list").striped(true).num_columns(3).show(ui, |ui| {
                    for p in &db.plugins {
                        ui.label(egui::RichText::new(&p.name).color(theme::TEXT_BRIGHT));
                        ui.label(if p.instrument { "Instrument" } else { "Effect" });
                        ui.label(egui::RichText::new(&p.vendor).color(theme::TEXT_DIM));
                        ui.end_row();
                    }
                });
                if !db.failed.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Could not be loaded:").color(theme::METER_HIGH));
                    for (p, e) in &db.failed {
                        ui.label(egui::RichText::new(format!("{}  -  {e}", p.display())).size(12.5).color(theme::TEXT_DIM));
                    }
                }
            });
        });
        self.show_manager = open;
        if rescan {
            self.start_scan(true);
        }
    }
}
