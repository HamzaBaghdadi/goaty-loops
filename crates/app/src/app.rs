use crate::views::{browser, channel_rack, channel_settings, mixer, piano_roll, playlist};
use crate::{theme, widgets};
use goaty_engine::export::{render_to_wav, ExportOptions};
use goaty_engine::import_flp::{import_flp, to_flp};
use goaty_engine::sample::{is_audio_file, load_audio};
use goaty_engine::*;
use egui::{pos2, vec2, Color32, Key, Rect, RichText, Sense, Stroke};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

const UNDO_LIMIT: usize = 200;
pub const PROJECT_EXT: &str = "goaty";
/// Project extension used before the rename (BeatCraft); still opened.
pub const LEGACY_PROJECT_EXT: &str = "bcproj";

/// Actions that replace or close the project; asked about first when the
/// project has unsaved changes.
#[derive(Clone, Debug)]
pub enum Pending {
    New,
    /// Open this file, or show the open dialog.
    Open(Option<PathBuf>),
    /// Recover the autosave of a crashed session (by session id).
    Recover(String),
    Close,
    /// Close and start the updated exe.
    Restart,
}

/// Files that replace the open project when opened.
pub fn is_project_path(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == PROJECT_EXT || ext == LEGACY_PROJECT_EXT || ext == "flp"
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Playlist,
    PianoRoll,
}

/// What the record button captures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecMode {
    /// MIDI notes into the current pattern.
    Notes,
    /// The audio input into a new playlist clip.
    Audio,
}

enum ExportMsg {
    Progress(f32),
    Done(Result<PathBuf, String>),
}

pub struct App {
    pub project: Project,
    pub engine: AudioEngine,
    pub file_path: Option<PathBuf>,
    pub modified: bool,
    needs_publish: bool,
    undo: Vec<Project>,
    redo: Vec<Project>,

    pub sel_channel: usize,
    pub cur_pattern: usize,
    pub song_mode: bool,

    pub show_rack: bool,
    pub show_piano: bool,
    pub show_mixer: bool,
    pub show_browser: bool,
    pub settings_channel: Option<usize>,
    show_about: bool,
    show_shortcuts: bool,
    show_info: bool,
    warnings: Vec<String>,

    pub piano: piano_roll::State,
    pub playlist_ui: playlist::State,
    pub mixer_ui: mixer::State,
    pub browser: browser::State,
    pub rack: channel_rack::State,
    pub automation_ui: crate::views::automation::State,

    pub status: String,
    /// Smoothed meter values for display.
    pub mixer_meters: Vec<(f32, f32)>,
    pub channel_meters: Vec<f32>,

    export: Option<(Receiver<ExportMsg>, f32)>,
    typing_held: HashMap<Key, (usize, u8)>,
    pub typing_octave: i32,
    pub typing_enabled: bool,
    time_bst: bool,
    renaming_pattern: Option<String>,
    last_title: String,
    startup_frames: u32,
    pub sample_cache: HashMap<PathBuf, Arc<sample::SampleData>>,
    /// Which editor keyboard shortcuts (delete, copy, paste...) apply to.
    pub focus: Focus,
    /// Notes copied from the piano roll (positions relative to the first note).
    pub note_clipboard: Vec<Note>,
    /// Clips copied from the playlist (positions relative to the first clip).
    pub clip_clipboard: Vec<Clip>,
    pub midi: crate::midi::Midi,
    /// Parameter waiting for a MIDI controller to be moved (MIDI learn).
    pub midi_learn: Option<AutoTarget>,
    /// Record armed: MIDI notes played while the pattern plays are recorded.
    pub recording: bool,
    pub rec_mode: RecMode,
    pub settings: crate::settings::Settings,
    /// OPTIONS > Audio settings window.
    pub audio_ui: crate::views::audio_settings::AudioSettingsUi,
    /// Plugins loaded so far and the total, while a project's plugins load.
    plugin_load: Option<(usize, usize)>,
    /// Audio take in progress and the song tick it started at.
    audio_rec: Option<(goaty_engine::record::AudioRecorder, f64)>,
    rec_held: HashMap<u8, (f64, f32)>,
    rec_take_started: bool,
    /// Playlist clip receiving notes recorded in song mode.
    rec_clip: Option<usize>,
    pub plugins: crate::plugins::PluginHost,
    /// The FLP the current project was imported from (for round-trip saving).
    flp_original: Option<goaty_flp::FlpFile>,
    /// Autosave and crash-recovery session for this run.
    session: crate::recovery::Session,
    /// Bumped on every edit; autosave runs when it differs from `autosaved_gen`.
    edit_gen: u64,
    autosaved_gen: u64,
    /// Sessions that crashed or were killed, offered for recovery at startup.
    recoverable: Vec<crate::recovery::Recoverable>,
    /// An action waiting for the "save changes?" answer.
    unsaved_prompt: Option<Pending>,
    /// Closing was confirmed; the window closes on the next frame.
    close_now: bool,
    allow_close: bool,
    /// In-app updates from GitHub Releases.
    pub updater: crate::update::Updater,
    /// Start the (updated) app again after this one exits.
    restart_on_exit: bool,
}

impl App {
    pub fn new(file: Option<PathBuf>) -> Self {
        let project = Project::new_template();
        let settings = crate::settings::Settings::load();
        let session = crate::recovery::Session::start();
        crate::crash::install(&session.id);
        let recoverable = crate::recovery::find_recoverable();
        let engine = AudioEngine::new(project.clone(), &settings.audio);
        let status = match &engine.error {
            Some(e) => format!("Audio: {e}"),
            None => format!("Audio: {} @ {} Hz", engine.device_name, engine.sample_rate),
        };
        let mut app = Self {
            project,
            engine,
            file_path: None,
            modified: false,
            needs_publish: false,
            undo: Vec::new(),
            redo: Vec::new(),
            sel_channel: 0,
            cur_pattern: 0,
            song_mode: false,
            show_rack: true,
            show_piano: false,
            show_mixer: false,
            show_browser: true,
            settings_channel: None,
            show_about: false,
            show_shortcuts: false,
            show_info: false,
            warnings: Vec::new(),
            piano: Default::default(),
            playlist_ui: Default::default(),
            mixer_ui: Default::default(),
            browser: browser::State::new(),
            rack: Default::default(),
            automation_ui: Default::default(),
            status,
            mixer_meters: Vec::new(),
            channel_meters: Vec::new(),
            export: None,
            typing_held: HashMap::new(),
            typing_octave: 5,
            typing_enabled: true,
            time_bst: false,
            renaming_pattern: None,
            last_title: String::new(),
            startup_frames: 0,
            sample_cache: HashMap::new(),
            focus: Focus::Playlist,
            note_clipboard: Vec::new(),
            clip_clipboard: Vec::new(),
            midi: crate::midi::Midi::new(),
            midi_learn: None,
            recording: false,
            rec_mode: RecMode::Notes,
            settings,
            audio_ui: Default::default(),
            plugin_load: None,
            audio_rec: None,
            rec_held: HashMap::new(),
            rec_take_started: false,
            rec_clip: None,
            plugins: crate::plugins::PluginHost::new(),
            flp_original: None,
            session,
            edit_gen: 0,
            autosaved_gen: 0,
            recoverable,
            unsaved_prompt: None,
            close_now: false,
            allow_close: false,
            updater: crate::update::Updater::new(),
            restart_on_exit: false,
        };
        if app.settings.check_updates.unwrap_or(true) {
            app.updater.check(false);
        }
        {
            // If the UI thread panics, save the latest project version.
            let (shared, path) = (app.engine.shared.clone(), app.session.autosave_path());
            crate::crash::set_emergency_save(move || crate::recovery::emergency_save(&path, &shared));
        }
        if !app.settings.midi_off {
            let (tx, shared) = (app.engine.command_sender(), app.engine.shared.clone());
            app.midi.connect(app.settings.midi_input.as_deref(), tx.clone(), shared.clone());
            if app.midi.port_name.is_none() && app.settings.midi_input.is_some() {
                // The saved device is gone; fall back to the first one.
                app.midi.connect(None, tx, shared);
            }
        }
        if let Some(f) = file {
            app.open_path(&f);
        }
        app
    }

    // ---- editing helpers -------------------------------------------------

    /// Records an undo point. Call once before a user edit begins.
    pub fn checkpoint(&mut self) {
        self.undo.push(self.project.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Marks the project as changed so the audio thread gets the new version.
    pub fn changed(&mut self) {
        self.modified = true;
        self.edit_gen += 1;
        self.needs_publish = true;
    }

    pub fn undo(&mut self) {
        if let Some(p) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.project, p));
            self.after_replace();
            self.status = "Undo".into();
        }
    }

    pub fn redo(&mut self) {
        if let Some(p) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.project, p));
            self.after_replace();
            self.status = "Redo".into();
        }
    }

    fn after_replace(&mut self) {
        self.sel_channel = self.sel_channel.min(self.project.channels.len().saturating_sub(1));
        self.cur_pattern = self.cur_pattern.min(self.project.patterns.len().saturating_sub(1));
        if self.settings_channel.is_some_and(|c| c >= self.project.channels.len()) {
            self.settings_channel = None;
        }
        self.changed();
    }

    pub fn preview_on(&self, channel: usize, key: u8) {
        self.engine.send(Command::NoteOn { channel, key, velocity: DEFAULT_VELOCITY });
    }

    pub fn preview_off(&self, channel: usize, key: u8) {
        self.engine.send(Command::NoteOff { channel, key });
    }

    pub fn play_position(&self) -> f64 {
        self.engine.transport().position()
    }

    pub fn is_playing(&self) -> bool {
        self.engine.transport().is_playing()
    }

    pub fn set_song_mode(&mut self, song: bool) {
        if self.song_mode != song {
            self.song_mode = song;
            self.engine.transport().seek(0.0);
        }
    }

    pub fn toggle_play(&mut self) {
        let t = self.engine.transport();
        if t.is_playing() {
            self.stop();
        } else {
            t.playing.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn stop(&mut self) {
        let t = self.engine.transport();
        t.playing.store(false, std::sync::atomic::Ordering::Relaxed);
        t.seek(0.0);
    }

    pub fn pause(&mut self) {
        self.engine.transport().playing.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn add_channel(&mut self, mut ch: Channel) -> usize {
        self.checkpoint();
        ch.color = PALETTE[self.project.channels.len() % PALETTE.len()];
        self.project.channels.push(ch);
        self.sel_channel = self.project.channels.len() - 1;
        self.changed();
        self.sel_channel
    }

    pub fn add_sample_channel(&mut self, path: &Path) {
        match self.load_sample_cached(path) {
            Ok(data) => {
                let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sample".into());
                let params = SamplerParams { path: Some(path.to_path_buf()), data: Some(data), ..Default::default() };
                self.add_channel(Channel::new(name, Instrument::Sampler(params)));
                self.status = format!("Added {}", path.display());
            }
            Err(e) => self.status = format!("Could not load {}: {e}", path.display()),
        }
    }

    pub fn load_sample_cached(&mut self, path: &Path) -> anyhow::Result<Arc<sample::SampleData>> {
        if let Some(d) = self.sample_cache.get(path) {
            return Ok(d.clone());
        }
        let d = Arc::new(load_audio(path)?);
        self.sample_cache.insert(path.to_path_buf(), d.clone());
        Ok(d)
    }

    pub fn new_pattern(&mut self) {
        self.checkpoint();
        let n = self.project.patterns.len();
        self.project.patterns.push(Pattern::new(format!("Pattern {}", n + 1), PALETTE[n % PALETTE.len()]));
        self.cur_pattern = n;
        self.changed();
    }

    pub fn clone_pattern(&mut self) {
        self.checkpoint();
        let mut p = self.project.patterns[self.cur_pattern].clone();
        p.name = format!("{} (copy)", p.name);
        self.project.patterns.insert(self.cur_pattern + 1, p);
        // Clips after the insertion point now refer to shifted indices.
        for c in &mut self.project.playlist.clips {
            if let ClipSource::Pattern(i) = &mut c.source {
                if *i > self.cur_pattern {
                    *i += 1;
                }
            }
        }
        self.cur_pattern += 1;
        self.changed();
    }

    pub fn delete_pattern(&mut self) {
        if self.project.patterns.len() <= 1 {
            self.status = "A project needs at least one pattern".into();
            return;
        }
        self.checkpoint();
        self.project.remove_pattern(self.cur_pattern);
        self.cur_pattern = self.cur_pattern.min(self.project.patterns.len() - 1);
        self.changed();
    }

    // ---- files -----------------------------------------------------------

    fn replace_project(&mut self, p: Project, path: Option<PathBuf>) {
        self.flp_original = None;
        self.stop();
        self.engine.send(Command::AllNotesOff);
        self.project = p;
        self.file_path = path;
        self.undo.clear();
        self.redo.clear();
        self.sel_channel = 0;
        self.cur_pattern = 0;
        self.settings_channel = None;
        self.needs_publish = true;
        self.modified = false;
        self.song_mode = !self.project.playlist.clips.is_empty();
        // A different project: the previous one's autosave is no longer wanted.
        self.session.clear_autosave();
        self.autosaved_gen = self.edit_gen;
        let name = self.project.name.clone();
        self.session.set_project(&name, self.file_path.clone());
    }

    pub fn new_project(&mut self) {
        self.replace_project(Project::new_template(), None);
        self.status = "New project".into();
    }

    pub fn open_dialog(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("All supported projects", &[PROJECT_EXT, LEGACY_PROJECT_EXT, "flp"])
            .add_filter("Goaty Loops project", &[PROJECT_EXT, LEGACY_PROJECT_EXT])
            .add_filter("FL Studio project", &["flp"])
            .pick_file();
        if let Some(f) = file {
            self.open_path(&f);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "flp" {
            let original = goaty_flp::FlpFile::read(path).ok();
            match import_flp(path, &self.plugins.db) {
                Ok(mut r) => {
                    let n = (r.project.channels.len(), r.project.patterns.len(), r.project.playlist.clips.len());
                    let plugins = r.project.plugins_mut().len();
                    self.replace_project(r.project, None);
                    self.flp_original = original;
                    self.status = format!("Imported {} ({} channels, {} patterns, {} clips, {} VST3 plugins)", path.display(), n.0, n.1, n.2, plugins);
                    if !r.warnings.is_empty() {
                        self.warnings = r.warnings;
                    }
                }
                Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
            }
        } else if ext == PROJECT_EXT || ext == LEGACY_PROJECT_EXT {
            match Project::load_json(path) {
                Ok(mut p) => {
                    let warnings: Vec<String> = p.load_samples().into_iter().map(|f| format!("Could not load sample {f}")).collect();
                    self.replace_project(p, Some(path.to_path_buf()));
                    self.status = format!("Opened {}", path.display());
                    if !warnings.is_empty() {
                        self.warnings = warnings;
                    }
                }
                Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
            }
        } else if is_audio_file(path) {
            self.add_sample_channel(path);
        } else {
            self.status = format!("Unsupported file: {}", path.display());
        }
    }

    pub fn save(&mut self) {
        match self.file_path.clone() {
            Some(p) => self.save_to(&p),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let name = format!("{}.{PROJECT_EXT}", self.project.name);
        if let Some(mut p) = rfd::FileDialog::new().add_filter("Goaty Loops project", &[PROJECT_EXT]).set_file_name(&name).save_file() {
            if p.extension().is_none() {
                p.set_extension(PROJECT_EXT);
            }
            self.save_to(&p);
        }
    }

    fn save_to(&mut self, path: &Path) {
        if let Some(stem) = path.file_stem() {
            self.project.name = stem.to_string_lossy().to_string();
        }
        self.project.capture_plugin_states();
        match self.project.save_json(path) {
            Ok(()) => {
                self.file_path = Some(path.to_path_buf());
                self.modified = false;
                self.session.clear_autosave();
                self.autosaved_gen = self.edit_gen;
                let name = self.project.name.clone();
                self.session.set_project(&name, Some(path.to_path_buf()));
                self.status = format!("Saved {}", path.display());
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    // ---- unsaved changes, autosave, recovery -------------------------------

    /// Runs `action`, first asking to save if the project has unsaved changes.
    pub fn request(&mut self, action: Pending) {
        if self.modified {
            self.unsaved_prompt = Some(action);
        } else {
            self.run_pending(action);
        }
    }

    fn run_pending(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_project(),
            Pending::Open(Some(p)) => self.open_path(&p),
            Pending::Open(None) => self.open_dialog(),
            Pending::Recover(id) => self.recover(&id),
            Pending::Close => {
                self.allow_close = true;
                self.close_now = true;
            }
            Pending::Restart => {
                self.restart_on_exit = true;
                self.allow_close = true;
                self.close_now = true;
            }
        }
    }

    fn unsaved_prompt_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.unsaved_prompt.clone() else { return };
        let name = self.file_path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.project.name.clone());
        let before = match action {
            Pending::New => "starting a new project",
            Pending::Open(_) => "opening another project",
            Pending::Recover(_) => "recovering the other project",
            Pending::Close => "closing",
            Pending::Restart => "restarting",
        };
        let (mut open, mut choice) = (true, None);
        widgets::fl_window(ctx, "unsaved_changes", "Unsaved changes", &mut open, [520.0, 260.0], [400.0, 100.0], |ui| {
            ui.label(format!("Save changes to \"{name}\" before {before}?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(2);
                }
            });
        });
        if !open {
            choice = Some(2);
        }
        match choice {
            Some(0) => {
                self.unsaved_prompt = None;
                self.save();
                // Only go on if it was saved (Save As can be cancelled).
                if !self.modified {
                    self.run_pending(action);
                }
            }
            Some(1) => {
                self.unsaved_prompt = None;
                self.run_pending(action);
            }
            Some(_) => self.unsaved_prompt = None,
            None => {}
        }
    }

    /// Autosaves into the session folder when there are unsaved edits and the
    /// interval has passed (and wakes the idle UI when it will be due).
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        crate::recovery::UNSAVED.store(self.modified, std::sync::atomic::Ordering::Relaxed);
        self.session.set_dirty(self.modified);
        let minutes = self.settings.autosave_minutes.unwrap_or(crate::settings::DEFAULT_AUTOSAVE_MINUTES);
        if minutes <= 0.0 || !self.modified || self.edit_gen == self.autosaved_gen {
            return;
        }
        let every = std::time::Duration::from_secs_f32(minutes * 60.0);
        let since = self.session.since_autosave();
        if since < every {
            ctx.request_repaint_after(every - since);
            return;
        }
        self.project.capture_plugin_states();
        match serde_json::to_string(&self.project) {
            Ok(json) => {
                let name = self.project.name.clone();
                self.session.autosave(json, &name, self.file_path.clone());
                self.autosaved_gen = self.edit_gen;
            }
            Err(e) => self.status = format!("Autosave failed: {e}"),
        }
    }

    /// Opens a crashed session's autosave as the current (unsaved) project.
    fn recover(&mut self, id: &str) {
        let Some(i) = self.recoverable.iter().position(|r| r.id == id) else { return };
        let r = self.recoverable.remove(i);
        let Some(path) = r.project.clone() else {
            r.discard();
            return;
        };
        match Project::load_json(&path) {
            Ok(mut p) => {
                let warnings: Vec<String> = p.load_samples().into_iter().map(|f| format!("Could not load sample {f}")).collect();
                // Saving goes back to the user's own file when it was a native project.
                let original = r.info.original_path.clone().filter(|o| {
                    let ext = o.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    ext == PROJECT_EXT || ext == LEGACY_PROJECT_EXT
                });
                self.replace_project(p, original);
                self.modified = true;
                self.edit_gen += 1;
                self.session.mark_due(); // keep it safe in this session right away
                r.discard();
                self.status = format!("Recovered \"{}\". Save it to keep the changes.", self.project.name);
                if !warnings.is_empty() {
                    self.warnings = warnings;
                }
            }
            // Keep the files: maybe a later version can read them.
            Err(e) => self.status = format!("Could not recover {}: {e}", path.display()),
        }
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        if self.recoverable.is_empty() {
            return;
        }
        let (mut open, mut action) = (true, None);
        let list = &self.recoverable;
        widgets::fl_window(ctx, "recovery", "Recover unsaved work", &mut open, [420.0, 170.0], [560.0, 160.0], |ui| {
            ui.label("Goaty Loops didn't close properly last time.");
            for r in list {
                ui.separator();
                let name = if r.info.project_name.is_empty() { "Untitled" } else { r.info.project_name.as_str() };
                let what = match (&r.project, r.info.autosaved) {
                    (Some(_), Some(t)) => format!("{name}: unsaved changes, autosaved {}", crate::recovery::ago(t)),
                    (Some(_), None) => format!("{name}: unsaved changes, saved when it crashed"),
                    (None, _) if r.info.dirty => format!("{name}: recent changes were lost (they hadn't been autosaved yet)"),
                    (None, _) => format!("{name}: no unsaved changes were lost"),
                };
                ui.label(RichText::new(what).strong());
                if let Some(o) = &r.info.original_path {
                    ui.label(RichText::new(o.display().to_string()).color(theme::TEXT_DIM).size(12.0));
                }
                if let Some(c) = &r.crash {
                    ui.label(RichText::new(c).color(theme::METER_HIGH).size(12.0));
                    if let Some(log) = &r.crash_log {
                        ui.label(RichText::new(format!("Report: {}", log.display())).color(theme::TEXT_DIM).size(11.0));
                    }
                }
                ui.horizontal(|ui| {
                    if r.project.is_some() {
                        if ui.button("Recover").clicked() {
                            action = Some((r.id.clone(), true));
                        }
                        if ui.button("Discard").on_hover_text("Delete the autosaved copy").clicked() {
                            action = Some((r.id.clone(), false));
                        }
                    } else if ui.button("OK").clicked() {
                        action = Some((r.id.clone(), false));
                    }
                });
            }
        });
        match action {
            Some((id, true)) => self.request(Pending::Recover(id)),
            Some((id, false)) => {
                if let Some(i) = self.recoverable.iter().position(|r| r.id == id) {
                    self.recoverable.remove(i).discard();
                }
            }
            None => {}
        }
        // Closing the window keeps the files: offered again next time.
        if !open {
            self.recoverable.clear();
        }
    }

    /// The update notice: shown when a newer release exists or after a manual check.
    fn update_window(&mut self, ctx: &egui::Context) {
        use crate::update::{State, CURRENT, REPO};
        self.updater.poll();
        if self.updater.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !self.updater.open {
            return;
        }
        let state = self.updater.state.clone();
        let (mut open, mut action) = (true, None);
        let mut at_startup = self.settings.check_updates.unwrap_or(true);
        let mut startup_changed = false;
        widgets::fl_window(ctx, "updates", "Updates", &mut open, [460.0, 180.0], [480.0, 150.0], |ui| {
            match &state {
                State::Idle | State::Checking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking for updates...");
                    });
                }
                State::UpToDate => {
                    ui.label(format!("Goaty Loops {CURRENT} is the latest version."));
                }
                State::Available(r) => {
                    ui.label(RichText::new(format!("Goaty Loops {} is available", r.version)).strong());
                    ui.label(RichText::new(format!("You have {CURRENT}.")).color(theme::TEXT_DIM));
                    if !r.notes.trim().is_empty() {
                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                            ui.label(r.notes.trim());
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Download and install").clicked() {
                            action = Some(0);
                        }
                        if ui.button("Later").clicked() {
                            action = Some(2);
                        }
                        if !r.page.is_empty() {
                            ui.hyperlink_to("Release page", &r.page);
                        }
                    });
                }
                State::Downloading(r) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Downloading Goaty Loops {}...", r.version));
                    });
                }
                State::Ready(r) => {
                    ui.label(format!("Goaty Loops {} is installed. Restart to start using it.", r.version));
                    ui.horizontal(|ui| {
                        if ui.button("Restart now").clicked() {
                            action = Some(1);
                        }
                        if ui.button("Later").clicked() {
                            action = Some(2);
                        }
                    });
                }
                State::Failed(e) => {
                    ui.colored_label(theme::METER_HIGH, e);
                    ui.hyperlink_to("Download page", format!("https://github.com/{REPO}/releases/latest"));
                }
            }
            ui.separator();
            startup_changed = ui.checkbox(&mut at_startup, "Check for updates when Goaty Loops starts").changed();
        });
        if startup_changed {
            self.settings.check_updates = Some(at_startup);
            self.settings.save();
        }
        match action {
            Some(0) => self.updater.install(),
            Some(1) => {
                self.updater.open = false;
                self.request(Pending::Restart);
            }
            Some(_) => self.updater.open = false,
            None => {}
        }
        if !open {
            self.updater.open = false;
        }
    }

    fn export_flp(&mut self) {
        // The FLP this project came from: kept from import, or re-read from its path.
        let original = self.flp_original.clone().or_else(|| {
            let path = &self.project.fl_origin.as_ref()?.path;
            goaty_flp::FlpFile::read(path).ok()
        });
        let base = self.project.fl_origin.as_ref().and_then(|o| o.path.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| self.project.name.clone());
        let name = format!("{base} (Goaty Loops).flp");
        let Some(p) = rfd::FileDialog::new().add_filter("FL Studio project", &["flp"]).set_file_name(&name).save_file() else { return };
        let (file, warnings, how) = match &original {
            Some(orig) => {
                let r = goaty_engine::flp_patch::patch_flp(orig, &self.project);
                (r.file, r.warnings, "changes written into the original project")
            }
            None => (to_flp(&self.project).to_flp_file(), Vec::new(), "experimental writer: only patterns, playlist and basic mixer settings"),
        };
        // Never silently replace the original: keep a backup if overwriting it.
        if self.project.fl_origin.as_ref().is_some_and(|o| o.path == p) && p.exists() {
            let mut bak = p.clone();
            bak.set_extension("flp.bak");
            if let Err(e) = std::fs::copy(&p, &bak) {
                self.status = format!("Could not back up the original ({e}); nothing was written");
                return;
            }
        }
        match file.write(&p) {
            Ok(()) => {
                self.status = format!("Saved {} ({how})", p.display());
                if !warnings.is_empty() {
                    self.warnings = warnings;
                }
            }
            Err(e) => self.status = format!("FLP export failed: {e}"),
        }
    }

    fn export_wav(&mut self) {
        self.export_audio(false);
    }

    /// Renders the song (or current pattern) to WAV, or to lossless FLAC.
    fn export_audio(&mut self, flac: bool) {
        if self.export.is_some() {
            return;
        }
        let (ext, kind) = if flac { ("flac", "FLAC audio") } else { ("wav", "WAV audio") };
        let name = format!("{}.{ext}", self.project.name);
        let Some(path) = rfd::FileDialog::new().add_filter(kind, &[ext]).set_file_name(&name).save_file() else { return };
        let project = self.project.clone();
        let opts = ExportOptions {
            sample_rate: 44100,
            song: self.song_mode && !project.playlist.clips.is_empty(),
            pattern: self.cur_pattern,
            tail_secs: 2.0,
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut last = 0.0;
            let report = |p: f32| {
                if p - last >= 0.01 {
                    last = p;
                    let _ = tx.send(ExportMsg::Progress(p));
                }
            };
            let r = if flac { goaty_engine::export::render_to_flac(&project, &path, &opts, report) } else { render_to_wav(&project, &path, &opts, report) };
            let _ = tx.send(ExportMsg::Done(r.map(|_| path).map_err(|e| e.to_string())));
        });
        self.export = Some((rx, 0.0));
    }

    /// One WAV per mixer insert plus the full mix, into a folder the user picks.
    fn export_stems(&mut self) {
        if self.export.is_some() {
            return;
        }
        let Some(parent) = rfd::FileDialog::new().set_title("Folder for the stems").pick_folder() else { return };
        let dir = parent.join(format!("{} stems", self.project.name));
        let project = self.project.clone();
        let opts = ExportOptions { sample_rate: 44100, song: self.song_mode && !project.playlist.clips.is_empty(), pattern: self.cur_pattern, tail_secs: 2.0 };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut last = 0.0;
            let r = goaty_engine::export::render_stems(&project, &dir, &opts, |p| {
                if p - last >= 0.01 {
                    last = p;
                    let _ = tx.send(ExportMsg::Progress(p));
                }
            });
            let _ = tx.send(ExportMsg::Done(r.map(|_| dir).map_err(|e| e.to_string())));
        });
        self.export = Some((rx, 0.0));
    }

    fn poll_export(&mut self) {
        let Some((rx, progress)) = &mut self.export else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                ExportMsg::Progress(p) => *progress = p,
                ExportMsg::Done(r) => finished = Some(r),
            }
        }
        if let Some(r) = finished {
            self.status = match r {
                Ok(p) => format!("Exported {}", p.display()),
                Err(e) => format!("Export failed: {e}"),
            };
            self.export = None;
        }
    }

    // ---- input -----------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for e in &events {
            let egui::Event::Key { key, pressed, repeat, modifiers, .. } = e else { continue };
            if *pressed && modifiers.command {
                match key {
                    Key::S if modifiers.shift => self.save_as(),
                    Key::S => self.save(),
                    Key::O => self.request(Pending::Open(None)),
                    Key::N => self.request(Pending::New),
                    Key::Z if modifiers.shift => self.redo(),
                    Key::Z => self.undo(),
                    Key::Y => self.redo(),
                    Key::R => self.export_wav(),
                    _ => {}
                }
                continue;
            }
            if typing {
                continue;
            }
            if *pressed && !*repeat {
                match key {
                    Key::Space => self.toggle_play(),
                    Key::F4 => self.new_pattern(),
                    Key::F5 => self.status = "The playlist is always visible in the main area".into(),
                    Key::F6 => self.show_rack = !self.show_rack,
                    Key::F7 => self.show_piano = !self.show_piano,
                    Key::F8 => self.show_browser = !self.show_browser,
                    Key::F9 => self.show_mixer = !self.show_mixer,
                    Key::L => self.set_song_mode(!self.song_mode),
                    Key::Plus | Key::Equals => {
                        self.cur_pattern = (self.cur_pattern + 1).min(self.project.patterns.len() - 1)
                    }
                    Key::Minus => self.cur_pattern = self.cur_pattern.saturating_sub(1),
                    _ => {}
                }
            }
            if self.typing_enabled && !modifiers.command && !modifiers.alt {
                self.typing_keyboard(*key, *pressed, *repeat);
            }
        }
    }

    /// Computer keyboard as a piano: Z-row = lower octave, Q-row = upper.
    fn typing_keyboard(&mut self, key: Key, pressed: bool, repeat: bool) {
        const LOWER: [Key; 13] = [Key::Z, Key::S, Key::X, Key::D, Key::C, Key::V, Key::G, Key::B, Key::H, Key::N, Key::J, Key::M, Key::Comma];
        const UPPER: [Key; 13] = [Key::Q, Key::Num2, Key::W, Key::Num3, Key::E, Key::R, Key::Num5, Key::T, Key::Num6, Key::Y, Key::Num7, Key::U, Key::I];
        let semis = LOWER.iter().position(|k| *k == key).or_else(|| UPPER.iter().position(|k| *k == key).map(|i| i + 12));
        let Some(semi) = semis else { return };
        if repeat {
            return;
        }
        if pressed {
            if self.project.channels.is_empty() {
                return;
            }
            let note = (self.typing_octave * 12 - 12 + semi as i32).clamp(0, 127) as u8;
            self.preview_on(self.sel_channel, note);
            self.typing_held.insert(key, (self.sel_channel, note));
        } else if let Some((ch, note)) = self.typing_held.remove(&key) {
            self.preview_off(ch, note);
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            if is_project_path(&p) {
                self.request(Pending::Open(Some(p)));
            } else {
                self.open_path(&p);
            }
        }
    }

    /// True while a meter still shows a level (sound playing or falling back).
    fn meters_active(&self) -> bool {
        self.mixer_meters.iter().any(|&(l, r)| l.max(r) > 1e-3) || self.channel_meters.iter().any(|&v| v > 1e-3)
    }

    fn update_meters(&mut self) {
        let m = &self.engine.shared.meters;
        self.mixer_meters.resize(self.project.mixer.len(), (0.0, 0.0));
        for (i, v) in self.mixer_meters.iter_mut().enumerate() {
            let (l, r) = m.take_mixer(i);
            *v = (l.max(v.0 * 0.85), r.max(v.1 * 0.85));
        }
        self.channel_meters.resize(self.project.channels.len(), 0.0);
        for (i, v) in self.channel_meters.iter_mut().enumerate() {
            *v = m.take_channel(i).max(*v * 0.8);
        }
    }

    // ---- top-level UI ----------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.visuals_mut().button_frame = false;
            ui.spacing_mut().item_spacing.x = 11.0;
            ui.spacing_mut().button_padding = egui::vec2(4.5, 2.0);
            ui.menu_button(RichText::new("FILE").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                if ui.button("New                    Ctrl+N").clicked() {
                    self.request(Pending::New);
                    ui.close_menu();
                }
                if ui.button("Open...                Ctrl+O").clicked() {
                    ui.close_menu();
                    self.request(Pending::Open(None));
                }
                if ui.button("Save                   Ctrl+S").clicked() {
                    ui.close_menu();
                    self.save();
                }
                if ui.button("Save as...       Ctrl+Shift+S").clicked() {
                    ui.close_menu();
                    self.save_as();
                }
                ui.separator();
                ui.menu_button("Export", |ui| {
                    if ui.button("WAV file...            Ctrl+R").clicked() {
                        ui.close_menu();
                        self.export_wav();
                    }
                    if ui.button("FL Studio project (.flp)...").clicked() {
                        ui.close_menu();
                        self.export_flp();
                    }
                });
                ui.separator();
                if ui.button("Project info...").clicked() {
                    self.show_info = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(RichText::new("EDIT").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo      Ctrl+Z")).clicked() {
                    self.undo();
                    ui.close_menu();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo      Ctrl+Y")).clicked() {
                    self.redo();
                    ui.close_menu();
                }
            });
            ui.menu_button(RichText::new("ADD").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                channel_rack::add_channel_menu(self, ui);
                ui.separator();
                if ui.button("Automation clip").clicked() {
                    crate::views::automation::create_clip(self);
                    ui.close_menu();
                }
                if ui.button("Mixer track").clicked() {
                    self.add_mixer_track();
                    ui.close_menu();
                }
            });
            ui.menu_button(RichText::new("PATTERNS").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                if ui.button("New pattern          F4").clicked() {
                    self.new_pattern();
                    ui.close_menu();
                }
                if ui.button("Clone pattern").clicked() {
                    self.clone_pattern();
                    ui.close_menu();
                }
                if ui.button("Delete pattern").clicked() {
                    self.delete_pattern();
                    ui.close_menu();
                }
            });
            ui.menu_button(RichText::new("VIEW").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                ui.checkbox(&mut self.show_rack, "Channel rack       F6");
                ui.checkbox(&mut self.show_piano, "Piano roll           F7");
                ui.checkbox(&mut self.show_browser, "Browser              F8");
                ui.checkbox(&mut self.show_mixer, "Mixer                 F9");
            });
            ui.menu_button(RichText::new("OPTIONS").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                ui.menu_button("MIDI input", |ui| {
                    let current = self.midi.port_name.clone();
                    if ui.selectable_label(current.is_none(), "(none)").clicked() {
                        self.midi.disconnect();
                        self.settings.midi_off = true;
                        self.settings.save();
                        ui.close_menu();
                    }
                    for name in crate::midi::Midi::ports() {
                        if ui.selectable_label(current.as_deref() == Some(name.as_str()), &name).clicked() {
                            self.midi.connect(Some(&name), self.engine.command_sender(), self.engine.shared.clone());
                            self.settings.midi_off = false;
                            self.settings.midi_input = Some(name.clone());
                            self.settings.save();
                            self.status = match &self.midi.error {
                                Some(e) => format!("MIDI: {e}"),
                                None => format!("MIDI input: {name}"),
                            };
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Controller links (right-click a knob to add)").color(theme::TEXT_DIM).size(11.0));
                    let mut remove = None;
                    for (i, l) in self.project.midi_links.iter().enumerate() {
                        let name = crate::views::describe_target(&self.project, Some(l.target));
                        if ui.button(format!("CC {} (ch {}) → {name}   ✕", l.cc, l.channel + 1)).on_hover_text("Click to remove").clicked() {
                            remove = Some(i);
                        }
                    }
                    if let Some(i) = remove {
                        self.checkpoint();
                        self.project.midi_links.remove(i);
                        self.changed();
                    }
                });
                ui.menu_button("Audio input", |ui| {
                    let current = self.settings.audio_input.clone();
                    let mut chosen = None;
                    if ui.selectable_label(current.is_none(), "System default").clicked() {
                        chosen = Some(None);
                    }
                    for name in goaty_engine::record::input_devices() {
                        if ui.selectable_label(current.as_deref() == Some(name.as_str()), &name).clicked() {
                            chosen = Some(Some(name));
                        }
                    }
                    if let Some(c) = chosen {
                        self.settings.audio_input = c;
                        self.settings.save();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.settings.monitor_input, "Monitor input while recording").on_hover_text("Hear the input through the speakers. Use headphones to avoid feedback.").changed() {
                        self.settings.save();
                    }
                    ui.horizontal(|ui| {
                        ui.label("Latency compensation");
                        let r = ui.add(egui::DragValue::new(&mut self.settings.rec_latency_ms).range(0.0..=500.0).speed(0.5).suffix(" ms"));
                        if r.changed() {
                            self.settings.save();
                        }
                    })
                    .response
                    .on_hover_text("Recorded audio is moved this much earlier so it lines up with the beat");
                });
                if ui.button("Plugin manager...").clicked() {
                    self.plugins.show_manager = true;
                    ui.close_menu();
                }
                if ui.add_enabled(!self.plugins.scanning(), egui::Button::new("Scan for new plugins")).clicked() {
                    self.plugins.start_scan(false);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Audio settings...").clicked() {
                    self.audio_ui.show();
                    ui.close_menu();
                }
                let latency = self.engine.output_latency_ms().map(|ms| format!(", {ms:.1} ms")).unwrap_or_default();
                ui.label(format!("Audio: {} @ {} Hz{latency}", self.engine.device_name, self.engine.sample_rate));
                let pdc = self.engine.shared.latency_samples.load(std::sync::atomic::Ordering::Relaxed);
                ui.label(format!("Plugin delay compensation: {:.1} ms", pdc as f64 * 1000.0 / self.engine.sample_rate.max(1) as f64))
                    .on_hover_text("Extra delay added so plugins that look ahead stay in time with everything else");
                if let Some(e) = &self.engine.error {
                    ui.colored_label(theme::METER_HIGH, e);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Typing keyboard octave");
                    ui.add(egui::DragValue::new(&mut self.typing_octave).range(1..=9));
                });
                ui.horizontal(|ui| {
                    ui.label("Autosave every");
                    let mut m = self.settings.autosave_minutes.unwrap_or(crate::settings::DEFAULT_AUTOSAVE_MINUTES);
                    let r = ui.add(egui::DragValue::new(&mut m).range(0.0..=60.0).speed(0.1).suffix(" min"));
                    if r.on_hover_text("Unsaved changes are kept safe in case of a crash (your project file isn't touched). 0 turns it off.").changed() {
                        self.settings.autosave_minutes = Some(m);
                        self.settings.save();
                    }
                });
            });
            ui.menu_button(RichText::new("TOOLS").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                if ui.button("Render to WAV...").clicked() {
                    ui.close_menu();
                    self.export_wav();
                }
                if ui.button("Render to FLAC...").clicked() {
                    ui.close_menu();
                    self.export_audio(true);
                }
                if ui.button("Export stems (one WAV per mixer track)...").clicked() {
                    ui.close_menu();
                    self.export_stems();
                }
                if ui.button("Project info...").clicked() {
                    self.show_info = true;
                    ui.close_menu();
                }
                if ui.button("Rename current pattern...").clicked() {
                    self.renaming_pattern = Some(self.project.patterns[self.cur_pattern].name.clone());
                    ui.close_menu();
                }
                ui.separator();
                match self.project.last_tweaked() {
                    Some(t) => {
                        ui.label(RichText::new(format!("Last tweaked: {}", crate::views::describe_target(&self.project, Some(t)))).color(theme::TEXT_DIM));
                        self.midi_link_menu(ui, t);
                    }
                    None => {
                        ui.label(RichText::new("Last tweaked: move a control in a plugin window to link it").color(theme::TEXT_DIM));
                    }
                }
            });
            ui.menu_button(RichText::new("HELP").font(theme::font(17.0)).color(theme::TEXT), |ui| {
                if ui.add_enabled(!self.updater.busy(), egui::Button::new("Check for updates...")).clicked() {
                    self.updater.check(true);
                    ui.close_menu();
                }
                if ui.button("Keyboard shortcuts").clicked() {
                    self.show_shortcuts = true;
                    ui.close_menu();
                }
                if ui.button("About Goaty Loops").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
        });
    }

    // ---- MIDI input / recording ------------------------------------------

    fn process_midi(&mut self) {
        self.midi.target.store(self.sel_channel, std::sync::atomic::Ordering::Relaxed);
        let msgs = self.midi.drain();
        let active = self.recording && self.rec_mode == RecMode::Notes && self.is_playing() && !self.project.channels.is_empty();
        if !active {
            self.rec_take_started = false;
            self.rec_held.clear();
            self.rec_clip = None;
        }
        let mut moved = false;
        for m in msgs {
            match m {
                crate::midi::MidiMsg::Cc { channel, cc, value } => {
                    if let Some(target) = self.midi_learn.take() {
                        self.checkpoint();
                        // One controller per parameter and one parameter per controller.
                        self.project.midi_links.retain(|l| l.target != target && (l.channel, l.cc) != (channel, cc));
                        self.project.midi_links.push(MidiLink { channel, cc, target });
                        self.status = format!("Linked CC {cc} to {}", crate::views::describe_target(&self.project, Some(target)));
                        self.changed();
                    }
                    let targets: Vec<AutoTarget> = self.project.midi_links.iter().filter(|l| (l.channel, l.cc) == (channel, cc)).map(|l| l.target).collect();
                    let v = value as f32 / 127.0;
                    if targets.is_empty() {
                        // Unlinked controllers (mod wheel, sustain...) go to the selected plugin instrument.
                        if let Some(Instrument::Plugin(PluginParams { instance: Some(inst), .. })) = self.project.channels.get(self.sel_channel).map(|c| &c.instrument) {
                            inst.midi_controller(cc, v as f64);
                        }
                    }
                    for t in targets {
                        if self.project.set_target(t, v) {
                            moved = true;
                        } else if let Some((inst, param)) = self.project.plugin_param(t) {
                            inst.set_param_from_ui(param, v as f64);
                        } else {
                            self.engine.send(Command::SetParam { target: t, value: v });
                        }
                    }
                }
                crate::midi::MidiMsg::On { key, velocity, tick } => {
                    if active {
                        self.rec_held.insert(key, (tick, velocity));
                    }
                }
                crate::midi::MidiMsg::Off { key, tick } => {
                    let Some((t0, velocity)) = self.rec_held.remove(&key) else { continue };
                    if !active {
                        continue;
                    }
                    if !self.rec_take_started {
                        // One undo step per recording take.
                        self.checkpoint();
                        self.rec_take_started = true;
                    }
                    let (start, length) = if self.song_mode {
                        // Song position: put the note in the pattern clip under the playhead.
                        let (t0, t1) = (t0.max(0.0) as u32, (tick.max(t0) as u32).max(t0 as u32 + 1));
                        let bar = self.project.bar_ticks();
                        let i = crate::midi::song_record_clip(&mut self.project.playlist.clips, self.cur_pattern, t0, t1, bar, self.rec_clip);
                        self.rec_clip = Some(i);
                        let c = self.project.playlist.clips[i];
                        self.project.ensure_tracks(c.track + 1);
                        (t0 - c.start + c.offset, t1 - t0)
                    } else {
                        let pat_len = self.project.pattern_length(self.cur_pattern) as f64;
                        crate::midi::recorded_span(t0, tick, pat_len)
                    };
                    let note = Note { channel: self.sel_channel, key, start, length, velocity, pan: 0.0 };
                    self.project.patterns[self.cur_pattern].notes.push(note);
                    self.changed();
                }
            }
        }
        // Publish controller moves once per frame, however many messages arrived.
        if moved {
            self.changed();
        }
    }

    /// Context-menu entries to link a parameter to a MIDI controller.
    pub fn midi_link_menu(&mut self, ui: &mut egui::Ui, target: AutoTarget) {
        let waiting = self.midi_learn == Some(target);
        let label = if waiting { "Waiting for a MIDI controller..." } else { "Link to MIDI controller" };
        if ui.button(label).clicked() {
            self.midi_learn = if waiting { None } else { Some(target) };
            self.status = if waiting { "MIDI learn cancelled".into() } else { "Move a knob or fader on your MIDI controller".into() };
            if self.midi.port_name.is_none() {
                self.status = "No MIDI input connected (OPTIONS > MIDI input)".into();
            }
            ui.close_menu();
        }
        if let Some(i) = self.project.midi_links.iter().position(|l| l.target == target) {
            let l = self.project.midi_links[i];
            if ui.button(format!("Remove link (CC {}, ch {})", l.cc, l.channel + 1)).clicked() {
                self.checkpoint();
                self.project.midi_links.remove(i);
                self.changed();
                ui.close_menu();
            }
        }
    }

    // ---- plugin loading --------------------------------------------------

    /// Loads one pending plugin per frame, so opening a project with many
    /// plugins shows progress instead of freezing the window.
    fn load_plugins_step(&mut self, ctx: &egui::Context) {
        let pending = self.project.pending_plugins();
        if pending == 0 {
            if let Some((done, _)) = self.plugin_load.take() {
                self.status = format!("Loaded {done} plugin(s)");
            }
            return;
        }
        let (done, total) = self.plugin_load.get_or_insert((0, pending));
        *total = (*done + pending).max(*total);
        let Some((name, error)) = self.project.instantiate_next_plugin(self.engine.sample_rate as f64) else { return };
        let (done, total) = self.plugin_load.as_mut().map(|(d, t)| {
            *d += 1;
            (*d, *t)
        }).unwrap_or((1, 1));
        if let Some(e) = error {
            self.warnings.push(format!("Plugin {e}"));
        }
        // Make it audible without marking the project as edited.
        self.engine.publish(&self.project);
        self.status = format!("Loading plugins {done}/{total}: {name}");
        ctx.request_repaint();
    }

    // ---- audio output ----------------------------------------------------

    pub fn is_recording_audio(&self) -> bool {
        self.audio_rec.is_some()
    }

    /// Switches the audio output; saves the choice if the device opened.
    pub fn apply_audio_config(&mut self, config: goaty_engine::AudioConfig) {
        let result = self.reopen_audio(&config);
        match result {
            Ok(()) => {
                self.settings.audio = config;
                self.settings.save();
                self.status = self.audio_status();
            }
            Err(e) => self.status = format!("Audio: {e}"),
        }
    }

    /// Reopens the output; plugins are reloaded if the sample rate changed
    /// (they are set up for one rate).
    fn reopen_audio(&mut self, config: &goaty_engine::AudioConfig) -> Result<(), String> {
        let old_rate = self.engine.sample_rate;
        let result = self.engine.reopen(config);
        if self.engine.sample_rate != old_rate {
            self.project.unload_plugins();
            self.plugin_load = None;
            self.engine.publish(&self.project);
        }
        result
    }

    fn audio_status(&self) -> String {
        let e = &self.engine;
        let latency = e.output_latency_ms().map(|ms| format!(", {ms:.1} ms latency")).unwrap_or_default();
        format!("Audio: {} @ {} Hz via {}{latency}", e.device_name, e.sample_rate, e.info.driver.label())
    }

    /// The output device went away: carry on with the same driver on the
    /// default device (the saved choice is kept for next time).
    fn handle_device_lost(&mut self) {
        if !self.engine.take_device_lost() {
            return;
        }
        let gone = self.engine.device_name.clone();
        let config = goaty_engine::AudioConfig { device: None, ..self.settings.audio.clone() };
        let _ = self.reopen_audio(&config);
        self.status = format!("Audio device \"{gone}\" disconnected. {}", self.audio_status());
    }

    // ---- audio recording -------------------------------------------------

    /// Starts a take when armed for audio and playing; finishes it on stop.
    fn process_audio_rec(&mut self) {
        let want = self.recording && self.rec_mode == RecMode::Audio && self.is_playing();
        match (&mut self.audio_rec, want) {
            (None, true) => {
                // Audio goes into the playlist, so record against the song.
                self.set_song_mode(true);
                let monitor = self.settings.monitor_input.then(|| (self.engine.shared.clone(), self.engine.sample_rate));
                match goaty_engine::record::AudioRecorder::start(self.settings.audio_input.as_deref(), monitor) {
                    Ok(r) => {
                        self.status = format!("Recording audio from {}", r.device_name);
                        if self.settings.monitor_input && !r.monitoring() {
                            self.status += " (monitoring off: input and output sample rates differ)";
                        }
                        self.audio_rec = Some((r, self.play_position()));
                    }
                    Err(e) => {
                        self.status = format!("Audio recording failed: {e}");
                        self.recording = false;
                    }
                }
            }
            (Some((r, _)), true) => {
                r.poll();
                let mon = if r.monitoring() { ", monitoring" } else { "" };
                self.status = format!("Recording audio from {}: {:.1} s{mon}", r.device_name, r.seconds());
            }
            (Some(_), false) => self.finish_audio_take(),
            (None, false) => {}
        }
    }

    fn finish_audio_take(&mut self) {
        let Some((rec, start)) = self.audio_rec.take() else { return };
        let data = rec.finish(self.settings.rec_latency_ms);
        if data.frames() == 0 {
            self.status = "Audio recording: nothing was captured".into();
            return;
        }
        let dir = recordings_dir();
        let base = sanitize_file_name(&self.project.name);
        let (name, path) = (1..)
            .map(|n| {
                let name = format!("{base} take {n}");
                let path = dir.join(format!("{name}.wav"));
                (name, path)
            })
            .find(|(_, p)| !p.exists())
            .expect("unbounded range");
        if let Err(e) = goaty_engine::record::write_wav(&path, &data) {
            self.status = format!("Could not save the recording: {e}");
            return;
        }
        let secs = data.duration_secs();
        let data = Arc::new(data);
        self.sample_cache.insert(path.clone(), data.clone());
        let params = SamplerParams { path: Some(path.clone()), data: Some(data), ..Default::default() };
        let ch = self.add_channel(Channel::new(name, Instrument::Sampler(params)));
        // Place the clip on the first track that is free over the take's span.
        let start = start.max(0.0) as u32;
        let length = ((secs * self.project.tempo / 60.0 * PPQ as f64).round() as u32).max(1);
        let clips = &self.project.playlist.clips;
        let track = (0..).find(|&t| !clips.iter().any(|c| c.track == t && c.start < start + length && c.end() > start)).unwrap_or(0);
        self.project.ensure_tracks(track + 1);
        self.project.playlist.clips.push(Clip { track, start, length, offset: 0, source: ClipSource::Audio(ch), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
        self.changed();
        self.status = format!("Recorded {secs:.1} s to {}", path.display());
    }

    // ---- plugins ---------------------------------------------------------

    fn plugin_frame(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.plugins.main_hwnd.is_none() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    self.plugins.main_hwnd = Some(w.hwnd.get());
                }
            }
        }
        if let Some(msg) = self.plugins.poll() {
            self.status = msg;
        } else if let Some(s) = &self.plugins.scan_status {
            self.status = s.clone();
        }
        self.plugins.sync_editors(&mut self.project);
        self.plugins.manager_window(ctx);
        if self.plugins.scanning() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn load_plugin(&mut self, info: &plugin::PluginInfo) -> Option<Arc<plugin::PluginInstance>> {
        match plugin::PluginInstance::new(info, self.engine.sample_rate as f64) {
            Ok(inst) => Some(inst),
            Err(e) => {
                self.status = format!("Could not load {}: {e}", info.name);
                None
            }
        }
    }

    /// Adds a VST3 instrument as a new channel and opens its editor.
    pub fn add_plugin_channel(&mut self, info: &plugin::PluginInfo) {
        let Some(inst) = self.load_plugin(info) else { return };
        let ch = Channel::new(info.name.clone(), Instrument::Plugin(PluginParams::new(info.clone(), Some(inst.clone()))));
        self.add_channel(ch);
        self.status = format!("Added {}", info.name);
        self.open_plugin_editor(inst, &info.name);
    }

    /// Adds a VST3 effect to a mixer track and opens its editor.
    pub fn add_plugin_effect(&mut self, track: usize, info: &plugin::PluginInfo) {
        if track >= self.project.mixer.len() {
            return;
        }
        let Some(inst) = self.load_plugin(info) else { return };
        self.checkpoint();
        self.project.mixer[track].effects.push(EffectSlot { mix: 1.0, fl: None, enabled: true, effect: Effect::Plugin(PluginParams::new(info.clone(), Some(inst.clone()))) });
        self.changed();
        let title = format!("{} ({})", info.name, self.project.mixer[track].name);
        self.open_plugin_editor(inst, &title);
    }

    pub fn open_plugin_editor(&mut self, inst: Arc<plugin::PluginInstance>, title: &str) {
        if let Err(e) = self.plugins.open_editor(inst, title) {
            self.status = e;
        }
    }

    pub fn add_mixer_track(&mut self) {
        if self.project.mixer.len() >= MAX_MIXER_TRACKS {
            return;
        }
        self.checkpoint();
        let n = self.project.mixer.len();
        self.project.mixer.push(MixerTrack::new(format!("Insert {n}")));
        self.changed();
    }

    /// Row 1 of the top bar: menus, transport, tempo, time display, meters.
    fn top_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            self.menu_bar(ui);
            ui.add_space(8.0);
            self.transport_panel(ui);
            ui.add_space(4.0);
            self.tempo_box(ui);
            ui.add_space(10.0);
            self.time_display(ui);
            ui.add_space(8.0);
            let r = widgets::tool_button(ui, widgets::Icon::Keyboard, self.typing_enabled, vec2(32.0, 28.0), "Typing keyboard to piano keyboard");
            if r.clicked() {
                self.typing_enabled = !self.typing_enabled;
            }
            ui.add_space(8.0);
            self.master_display(ui);
            ui.add_space(4.0);
            self.cpu_display(ui);
        });
    }

    fn transport_panel(&mut self, ui: &mut egui::Ui) {
        // PAT / SONG stacked toggle.
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, theme::BUTTON);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let top = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(36.0, 14.0));
        let bottom = Rect::from_min_size(rect.min + vec2(2.0, 18.0), vec2(36.0, 14.0));
        for (r, label, on) in [(top, "PAT", !self.song_mode), (bottom, "SONG", self.song_mode)] {
            if on {
                p.rect_filled(r, 2.0, theme::SONG_ON);
            }
            let col = if on { Color32::from_rgb(0x2a, 0x2e, 0x10) } else { theme::TEXT_FAINT };
            p.text(r.center(), egui::Align2::CENTER_CENTER, label, theme::font_medium(11.5), col);
        }
        if resp.clicked() {
            let y = resp.interact_pointer_pos().map(|p| p.y).unwrap_or(rect.center().y);
            self.set_song_mode(y > rect.center().y);
        }
        resp.on_hover_text("Pattern / song mode (L)");

        let playing = self.is_playing();
        let icon = if playing { widgets::Icon::Pause } else { widgets::Icon::Play };
        if widgets::tool_button(ui, icon, playing, vec2(34.0, 34.0), "Play / pause (Space)").clicked() {
            if playing {
                self.pause();
            } else {
                self.toggle_play();
            }
        }
        if widgets::tool_button(ui, widgets::Icon::Stop, false, vec2(34.0, 34.0), "Stop").clicked() {
            self.stop();
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
        let p = ui.painter();
        let armed = self.recording;
        let bg = if armed { theme::darken(theme::RECORD, 0.45) } else if resp.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON };
        p.rect_filled(rect, 2.0, bg);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let col = if armed { theme::lighten(theme::RECORD, 0.25) } else { theme::RECORD };
        p.circle_filled(rect.center(), 7.5, col);
        p.circle_filled(rect.center() - vec2(2.0, 2.0), 2.5, theme::lighten(col, 0.4));
        let tip = match (self.rec_mode, &self.midi.port_name) {
            (RecMode::Audio, _) => "Record audio input into the playlist. Right-click to choose what to record".to_string(),
            (RecMode::Notes, Some(n)) => format!("Record MIDI into the current pattern (input: {n}). Right-click to choose what to record"),
            (RecMode::Notes, None) => "Record MIDI into the current pattern (no MIDI input connected). Right-click to choose what to record".to_string(),
        };
        let resp = resp.on_hover_text(tip);
        if resp.clicked() {
            self.recording = !self.recording;
            self.status = match (self.recording, self.rec_mode) {
                (false, _) => "Recording off".into(),
                (true, RecMode::Notes) => "Recording armed: press play and play your MIDI keyboard".into(),
                (true, RecMode::Audio) => "Audio recording armed: press play to start, stop to finish the take".into(),
            };
        }
        resp.context_menu(|ui| {
            ui.label(RichText::new("Record").color(theme::TEXT_DIM));
            ui.radio_value(&mut self.rec_mode, RecMode::Notes, "Notes (MIDI into the current pattern)");
            ui.radio_value(&mut self.rec_mode, RecMode::Audio, "Audio (input into the playlist)");
            ui.separator();
            if ui.button("Open recordings folder").clicked() {
                let dir = recordings_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                ui.close_menu();
            }
        });
    }

    fn tempo_box(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(vec2(70.0, 28.0), Sense::click_and_drag());
        if resp.drag_started() {
            self.checkpoint();
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
            let d = -resp.drag_delta().y * if fine { 0.01 } else { 0.25 };
            self.project.tempo = (self.project.tempo + d as f64).clamp(10.0, 522.0);
            self.changed();
        }
        if resp.drag_stopped() && !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl) {
            self.project.tempo = self.project.tempo.round();
            self.changed();
        }
        let p = ui.painter();
        p.rect_filled(rect, 4.0, Color32::from_rgb(0x4d, 0x53, 0x56));
        p.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let whole = format!("{}", self.project.tempo.trunc() as i64);
        let frac = format!(".{:03}", ((self.project.tempo.fract()) * 1000.0).round() as i64 % 1000);
        let g1 = p.layout_no_wrap(whole, theme::font(21.0), theme::TEXT_BRIGHT);
        let g2 = p.layout_no_wrap(frac, theme::font(14.0), theme::TEXT);
        let total = g1.size().x + g2.size().x;
        let x = rect.center().x - total / 2.0;
        let y1 = rect.center().y - g1.size().y / 2.0;
        p.galley(pos2(x, y1), g1.clone(), theme::TEXT_BRIGHT);
        p.galley(pos2(x + g1.size().x, y1 + g1.size().y - g2.size().y - 2.0), g2, theme::TEXT);
        resp.on_hover_text("Tempo in BPM. Drag to change (Shift for fine), right-click to type.").context_menu(|ui| {
            let mut t = self.project.tempo;
            ui.add(egui::DragValue::new(&mut t).range(10.0..=522.0).speed(0.1).fixed_decimals(3).suffix(" BPM"));
            if t != self.project.tempo {
                self.project.tempo = t;
                self.changed();
            }
            ui.separator();
            self.midi_link_menu(ui, AutoTarget::Tempo);
        });
    }

    fn time_display(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(vec2(150.0, 38.0), Sense::click());
        if resp.clicked() {
            self.time_bst = !self.time_bst;
        }
        let pos = self.play_position();
        let (text, label) = if self.time_bst {
            let bar = self.project.bar_ticks() as f64;
            let beat = self.project.beat_ticks() as f64;
            let b = (pos / bar).floor() as u32 + 1;
            let bt = ((pos % bar) / beat).floor() as u32 + 1;
            let st = ((pos % beat) / STEP_TICKS as f64).floor() as u32 + 1;
            (format!("{b}:{bt:02}:{st:02}"), "B:S:T")
        } else {
            let secs = pos * 60.0 / (self.project.tempo * PPQ as f64);
            let m = (secs / 60.0).floor() as u32;
            let s = (secs % 60.0).floor() as u32;
            let cs = ((secs.fract()) * 100.0).floor() as u32;
            (format!("{m}:{s:02}:{cs:02}"), "M:S:CS")
        };
        let p = ui.painter();
        p.text(pos2(rect.left() + 4.0, rect.center().y + 1.0), egui::Align2::LEFT_CENTER, text, theme::font(34.0), theme::TEXT_BRIGHT);
        p.text(pos2(rect.right() - 2.0, rect.top() + 4.0), egui::Align2::RIGHT_TOP, label, theme::font_medium(9.0), theme::TEXT);
        resp.on_hover_text("Song position. Click to switch between time and bars.");
    }

    fn master_display(&mut self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(200.0, 34.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, Color32::from_rgb(0x1f, 0x24, 0x26));
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let (l, r) = self.mixer_meters.first().copied().unwrap_or((0.0, 0.0));
        for (i, v) in [l, r].into_iter().enumerate() {
            let y = rect.top() + 9.0 + i as f32 * 10.0;
            let bar = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, 6.0));
            p.rect_filled(bar, 1.0, theme::METER_BG);
            let db = 20.0 * v.max(1e-6).log10();
            let t = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let col = if v >= 0.97 { theme::METER_HIGH } else { theme::TEXT };
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * t, bar.height())), 1.0, col);
        }
    }

    fn cpu_display(&mut self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(170.0, 34.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, Color32::from_rgb(0x1f, 0x24, 0x26));
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let cpu = self.engine.shared.cpu_load().clamp(0.0, 1.0);
        let bar = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(26.0, 12.0));
        p.rect_filled(bar, 1.0, theme::METER_BG);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * cpu.max(0.05), bar.height())), 1.0, theme::KEY_WHITE);
        p.text(pos2(rect.left() + 60.0, rect.top() + 12.0), egui::Align2::RIGHT_CENTER, format!("{:.0}", cpu * 100.0), theme::font(14.0), theme::TEXT);
        p.text(pos2(rect.right() - 6.0, rect.top() + 12.0), egui::Align2::RIGHT_CENTER, format!("{:.1} kHz", self.engine.sample_rate as f32 / 1000.0), theme::font(14.0), theme::TEXT);
        p.text(pos2(rect.left() + 6.0, rect.bottom() - 9.0), egui::Align2::LEFT_CENTER, "CPU", theme::font_medium(10.0), theme::TEXT_DIM);
        p.text(pos2(rect.right() - 6.0, rect.bottom() - 9.0), egui::Align2::RIGHT_CENTER, format!("{} ch", self.project.channels.len()), theme::font(12.0), theme::TEXT_DIM);
    }

    /// Row 2 of the top bar: hint panel, window buttons, snap, pattern selector.
    fn second_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            self.hint_panel(ui);
            ui.add_space(6.0);
            let size = vec2(34.0, 30.0);
            if widgets::tool_button(ui, widgets::Icon::Playlist, true, size, "Playlist (F5)").clicked() {
                self.status = "The playlist fills the main area".into();
            }
            if widgets::tool_button(ui, widgets::Icon::PianoRoll, self.show_piano, size, "Piano roll (F7)").clicked() {
                self.show_piano = !self.show_piano;
            }
            if widgets::tool_button(ui, widgets::Icon::ChannelRack, self.show_rack, size, "Channel rack (F6)").clicked() {
                self.show_rack = !self.show_rack;
            }
            if widgets::tool_button(ui, widgets::Icon::Mixer, self.show_mixer, size, "Mixer (F9)").clicked() {
                self.show_mixer = !self.show_mixer;
            }
            if widgets::tool_button(ui, widgets::Icon::Browser, self.show_browser, size, "Browser (F8)").clicked() {
                self.show_browser = !self.show_browser;
            }
            ui.add_space(8.0);
            let (mrect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            widgets::paint_icon(ui.painter(), mrect, widgets::Icon::Magnet, theme::TEXT_DIM);
            let mut snap = self.playlist_ui.snap;
            crate::views::snap_combo(ui, "global_snap", &mut snap);
            if snap != self.playlist_ui.snap {
                self.playlist_ui.snap = snap;
                self.piano.snap = snap.min(PPQ);
            }
            ui.add_space(8.0);
            self.pattern_selector(ui);
        });
    }

    fn hint_panel(&mut self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(330.0, 34.0), Sense::hover());
        let p = ui.painter();
        let name = match &self.file_path {
            Some(f) => f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            None => format!("{}{}", self.project.name, if self.modified { " *" } else { "" }),
        };
        p.text(pos2(rect.left() + 4.0, rect.top() + 8.0), egui::Align2::LEFT_CENTER, format!("[ ]  {name}"), theme::font(13.5), theme::TEXT);
        let hint = match &self.export {
            Some((_, prog)) => format!("Exporting... {:.0}%", prog * 100.0),
            None => self.status.clone(),
        };
        let hint = widgets::fit_text(p, &hint, &theme::font(13.0), rect.width() - 8.0);
        p.text(pos2(rect.left() + 4.0, rect.bottom() - 8.0), egui::Align2::LEFT_CENTER, hint, theme::font(13.0), theme::HINT);
    }

    fn pattern_selector(&mut self, ui: &mut egui::Ui) {
        let popup_id = ui.make_persistent_id("pattern_popup");
        let arrow = widgets::tool_button(ui, widgets::Icon::ArrowRight, false, vec2(18.0, 30.0), "Pattern list");
        let (rect, resp) = ui.allocate_exact_size(vec2(130.0, 30.0), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, 2.0, if resp.hovered() { Color32::from_rgb(0x4a, 0x51, 0x54) } else { Color32::from_rgb(0x43, 0x49, 0x4c) });
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::TOP_SEP), egui::StrokeKind::Inside);
        let name = self.project.patterns.get(self.cur_pattern).map(|p| p.name.clone()).unwrap_or_default();
        let name = widgets::fit_text(p, &name, &theme::font(19.0), rect.width() - 10.0);
        p.text(rect.center(), egui::Align2::CENTER_CENTER, name, theme::font(19.0), theme::TEXT);
        if resp.clicked() || arrow.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        let resp = resp.on_hover_text("Current pattern. Click to choose, double-click to rename.");
        if resp.double_clicked() {
            ui.memory_mut(|m| m.close_popup());
            self.renaming_pattern = Some(self.project.patterns[self.cur_pattern].name.clone());
        }
        egui::popup::popup_below_widget(ui, popup_id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(180.0);
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for i in 0..self.project.patterns.len() {
                    let name = self.project.patterns[i].name.clone();
                    if ui.selectable_label(self.cur_pattern == i, name).clicked() {
                        self.cur_pattern = i;
                    }
                }
            });
        });
        if widgets::tool_button(ui, widgets::Icon::Plus, false, vec2(20.0, 30.0), "New pattern (F4)").clicked() {
            self.new_pattern();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if !self.warnings.is_empty() {
            let mut open = true;
            // Group warnings into categories with counts; details are collapsed.
            let categories: [(&str, fn(&str) -> bool); 6] = [
                ("FL effects not available", |w| w.starts_with("FL effect") && w.contains("isn't available")),
                ("FL effects approximated", |w| w.starts_with("FL effect") && w.contains("approximated")),
                ("Plugins replaced by stand-ins", |w| w.starts_with("Plugin") && w.contains("stand-in")),
                ("Plugins not installed or not loaded", |w| w.starts_with("Plugin") && !w.contains("stand-in")),
                ("Samples missing", |w| w.contains("ample")),
                ("Other", |_| true),
            ];
            let mut groups: Vec<(&str, Vec<&String>)> = categories.iter().map(|(n, _)| (*n, Vec::new())).collect();
            for w in &self.warnings {
                let i = categories.iter().position(|(_, f)| f(w)).unwrap_or(categories.len() - 1);
                groups[i].1.push(w);
            }
            widgets::fl_window(ctx, "import_report", "Import report", &mut open, [900.0, 520.0], [480.0, 230.0], |ui| {
                ui.label(RichText::new("Opened. Some parts can't be reproduced exactly yet:").color(theme::TEXT));
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(400.0).show(ui, |ui| {
                    for (name, items) in &groups {
                        if items.is_empty() {
                            continue;
                        }
                        egui::CollapsingHeader::new(RichText::new(format!("{name}  ({})", items.len())).color(theme::HINT)).id_salt(name).show(ui, |ui| {
                            for w in items {
                                ui.label(RichText::new(format!("- {w}")).size(13.0).color(theme::TEXT_DIM));
                            }
                        });
                    }
                });
            });
            if !open {
                self.warnings.clear();
            }
        }
        let mut open = self.show_about;
        widgets::fl_window(ctx, "about", "About Goaty Loops", &mut open, [520.0, 240.0], [380.0, 150.0], |ui| {
            ui.heading("Goaty Loops");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("An open-source, pattern-based digital audio workstation written in Rust.");
            ui.label("Opens .flp project files. Not affiliated with Image-Line; FL Studio is a trademark of Image-Line Software.");
            ui.label("Licensed under the Apache License 2.0.");
            ui.hyperlink_to("github.com/HamzaBaghdadi/goaty-loops", "https://github.com/HamzaBaghdadi/goaty-loops");
        });
        self.show_about = open;
        let mut open = self.show_shortcuts;
        widgets::fl_window(ctx, "shortcuts", "Keyboard shortcuts", &mut open, [480.0, 180.0], [460.0, 380.0], |ui| {
            egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                for (k, d) in [
                    ("Space", "Play / stop"),
                    ("L", "Toggle pattern / song mode"),
                    ("F4", "New pattern"),
                    ("F6 / F7 / F8 / F9", "Channel rack / piano roll / browser / mixer"),
                    ("+ / -", "Next / previous pattern"),
                    ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
                    ("Ctrl+N / O / S", "New / open / save"),
                    ("Ctrl+R", "Export WAV"),
                    ("Z..M, Q..I", "Play the selected channel from the computer keyboard"),
                    ("Piano roll: left click / drag", "Add, move, resize notes"),
                    ("Piano roll: right click", "Delete notes"),
                    ("Ctrl + wheel", "Zoom horizontally"),
                    ("Shift + wheel", "Scroll horizontally"),
                    ("Ctrl + drag", "Select notes / clips with a box (Ctrl+click toggles a clip)"),
                    ("Shift + click", "Toggle a note's selection"),
                    ("Ctrl+A / C / X / V", "Select all, copy, cut, paste (in the editor you last clicked)"),
                    ("Ctrl+B", "Duplicate the selection after itself"),
                    ("Ctrl+Q / Ctrl+Shift+Q", "Piano roll: quantize starts (and lengths)"),
                    ("Ctrl+L", "Piano roll: legato"),
                    ("Alt+U / Alt+G", "Piano roll: chop by snap / glue"),
                    ("Alt + click", "Playlist: split a clip at the snapped position"),
                    ("Alt+T", "Playlist: add a time marker at the song position (right-click the ruler to rename/delete)"),
                    ("Right-click knob", "Link to MIDI controller (MIDI learn)"),
                    ("TOOLS > Last tweaked", "Link the plugin parameter you last moved to a MIDI controller"),
                    ("Delete", "Delete the selection"),
                    ("Arrow keys", "Move selection (up/down = transpose, Shift = octave)"),
                    ("Record button", "Arm recording (right-click: notes into the pattern, or audio into the playlist)"),
                ] {
                    ui.label(RichText::new(k).color(theme::TEXT_BRIGHT));
                    ui.label(d);
                    ui.end_row();
                }
            });
        });
        self.show_shortcuts = open;
        let mut open = self.show_info;
        let mut changed = false;
        let project = &mut self.project;
        widgets::fl_window(ctx, "project_info", "Project info", &mut open, [480.0, 200.0], [380.0, 220.0], |ui| {
            egui::Grid::new("info").num_columns(2).show(ui, |ui| {
                ui.label("Title");
                changed |= ui.text_edit_singleline(&mut project.name).changed();
                ui.end_row();
                ui.label("Artists");
                changed |= ui.text_edit_singleline(&mut project.info.artists).changed();
                ui.end_row();
                ui.label("Genre");
                changed |= ui.text_edit_singleline(&mut project.info.genre).changed();
                ui.end_row();
                ui.label("Comments");
                changed |= ui.text_edit_multiline(&mut project.info.comments).changed();
                ui.end_row();
            });
        });
        if changed {
            self.modified = true;
            self.edit_gen += 1;
        }
        self.show_info = open;

        if let Some(mut name) = self.renaming_pattern.take() {
            let mut open = true;
            let mut done = false;
            widgets::fl_window(ctx, "rename_pattern", "Rename pattern", &mut open, [600.0, 160.0], [260.0, 70.0], |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
                r.request_focus();
                done = ui.input(|i| i.key_pressed(Key::Enter));
            });
            if done {
                if self.project.patterns[self.cur_pattern].name != name {
                    self.checkpoint();
                    self.project.patterns[self.cur_pattern].name = name;
                    self.changed();
                }
            } else if open {
                self.renaming_pattern = Some(name);
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!("{}{} - Goaty Loops", self.project.name, if self.modified { " *" } else { "" });
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A clean exit: nothing to recover next time.
        self.session.end();
        if self.restart_on_exit {
            crate::update::relaunch();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.handle_device_lost();
        // Free what the audio thread stopped using (old project versions, removed
        // plugins), here rather than on the audio thread.
        self.engine.shared.collect_garbage();
        if ctx.input(|i| i.viewport().close_requested()) && self.modified && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.unsaved_prompt = Some(Pending::Close);
        }
        if std::mem::take(&mut self.close_now) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.plugin_frame(ctx, frame);
        self.process_midi();
        self.process_audio_rec();
        self.load_plugins_step(ctx);
        if self.startup_frames < 5 {
            self.startup_frames += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);
        self.poll_export();
        self.update_meters();
        self.sel_channel = self.sel_channel.min(self.project.channels.len().saturating_sub(1));
        self.cur_pattern = self.cur_pattern.min(self.project.patterns.len().saturating_sub(1));

        egui::TopBottomPanel::top("top_row")
            .exact_height(44.0)
            .frame(egui::Frame::default().fill(theme::TOP_BG).inner_margin(egui::Margin::symmetric(6, 0)))
            .show(ctx, |ui| self.top_row(ui));
        egui::TopBottomPanel::top("second_row")
            .exact_height(40.0)
            .frame(egui::Frame::default().fill(theme::TOP_BG2).inner_margin(egui::Margin::symmetric(6, 0)).stroke(Stroke::new(1.0_f32, theme::TOP_SEP)))
            .show(ctx, |ui| self.second_row(ui));
        if self.show_browser {
            egui::SidePanel::left("browser")
                .exact_width(250.0)
                .frame(egui::Frame::default().fill(theme::BROWSER_BG))
                .show(ctx, |ui| browser::show(self, ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::default().fill(theme::WORKSPACE).inner_margin(egui::Margin::same(3))).show(ctx, |ui| {
            playlist::show(self, ui);
        });

        channel_rack::window(self, ctx);
        piano_roll::window(self, ctx);
        mixer::window(self, ctx);
        crate::views::automation::window(self, ctx);
        crate::views::audio_settings::window(self, ctx);
        self.recovery_window(ctx);
        self.unsaved_prompt_window(ctx);
        self.update_window(ctx);
        self.autosave_tick(ctx);
        channel_settings::window(self, ctx);
        self.dialogs(ctx);

        // Keep the audio thread in sync with the UI.
        let t = self.engine.transport();
        t.song_mode.store(self.song_mode, std::sync::atomic::Ordering::Relaxed);
        t.pattern.store(self.cur_pattern, std::sync::atomic::Ordering::Relaxed);
        if self.needs_publish {
            self.engine.publish(&self.project);
            self.needs_publish = false;
        }
        self.update_title(ctx);

        // Redraw only while something changes on its own. Mouse, keyboard, file
        // drops and MIDI input wake the UI by themselves, so an idle window costs
        // no CPU (it used to redraw 20 times a second regardless).
        use std::time::Duration;
        let _ = self.midi.wake.set(ctx.clone());
        if !self.engine.shared.has_waker() {
            let ctx = ctx.clone();
            self.engine.shared.set_waker(move || ctx.request_repaint());
        }
        if self.is_playing() || self.export.is_some() {
            ctx.request_repaint_after(Duration::from_millis(15)); // ~60 fps
        } else if self.startup_frames < 5 || self.audio_rec.is_some() || self.plugins.has_open_editors() || self.meters_active() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

/// Where audio recordings are saved: Documents/Goaty Loops/Recorded.
pub fn recordings_dir() -> PathBuf {
    goaty_engine::paths::recordings_dir()
}

/// Replaces characters Windows doesn't allow in file names.
fn sanitize_file_name(s: &str) -> String {
    let clean: String = s.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    if clean.is_empty() { "Recording".into() } else { clean }
}
