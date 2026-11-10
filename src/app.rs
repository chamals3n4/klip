//! App state and the non-UI logic: import pipeline, playback engine,
//! undo/redo, project I/O and keyboard shortcuts.

use crate::audio::AudioPreview;
use crate::export::{ExportJob, Quality};
use crate::media::{self, Frame, GrabRequest, Grabber, StreamDecoder};
use crate::model::*;
use crate::store::{self, Meta, Store};
use crate::theme;
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    None,
    Video(Id),
    Text(Id),
    Audio(Id),
    /// The transition after this main-track clip.
    Transition(Id),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoTab {
    Basic,
    Adjust,
    Audio,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Media,
    Audio,
    Text,
    Captions,
    Transitions,
    Filters,
}

/// Payload for dragging a library item onto the timeline.
#[derive(Clone, Copy)]
pub struct MediaDrag(pub Id);

/// Payload for dragging an item from the Audio tab (always lands on the music track).
#[derive(Clone, Copy)]
pub struct AudioDrag(pub Id);

#[derive(Default)]
pub struct Assets {
    pub thumb: Option<TextureHandle>,
    /// Ungraded thumbnail pixels, for building filter previews.
    pub thumb_raw: Option<Frame>,
    /// Thumbnail with each filter applied (built lazily by the Filters tab).
    pub filtered: HashMap<crate::fx::FilterPreset, TextureHandle>,
    pub strip: Vec<(f64, TextureHandle)>,
    pub wave: Option<Arc<Vec<f32>>>,
}

pub enum Job {
    Probed(Result<MediaInfo, String>),
    Thumb(Id, Frame),
    Strip(Id, Vec<(f64, Frame)>),
    Wave(Id, Vec<f32>),
    /// A project cover was (re)loaded: id, edit time it reflects, pixels.
    ProjectThumb(String, u64, Option<Frame>),
}

pub enum DialogResult {
    Import(Vec<PathBuf>),
    ImportSrt(PathBuf),
    ImportProject(PathBuf),
    Relink(Id, PathBuf),
    ExportTo(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HomeSort {
    Recent,
    Name,
}

/// The "New project" dialog.
pub struct NewProject {
    pub name: String,
    pub aspect: Aspect,
    /// Media to add once the project is created (files dropped on Home).
    pub media: Vec<PathBuf>,
}

/// State of the home screen (project grid).
pub struct HomeState {
    pub projects: Vec<Meta>,
    pub trash: Vec<Meta>,
    pub show_trash: bool,
    pub search: String,
    pub sort: HomeSort,
    /// Cover textures, keyed by project id, with the edit time they show.
    pub thumbs: HashMap<String, (u64, TextureHandle)>,
    pub thumbs_pending: std::collections::HashSet<String>,
    pub new_project: Option<NewProject>,
    /// Project being renamed: (id, new name).
    pub rename: Option<(String, String)>,
    /// Project whose "⋯" menu is open.
    pub menu: Option<String>,
    /// Project waiting for "Delete forever?" confirmation.
    pub confirm_delete: Option<Meta>,
    /// Last deleted project, offered for undo until the given time.
    pub undo_delete: Option<(Meta, f64)>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub msg: String,
    pub kind: ToastKind,
    pub until: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipKind {
    Video,
    Text,
    Audio,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragMode {
    Move,
    TrimL,
    TrimR,
}

#[derive(Clone, Copy, Debug)]
pub enum TlDrag {
    None,
    Scrub,
    Clip {
        kind: ClipKind,
        id: Id,
        mode: DragMode,
        t0: f64,
        start: f64,
        src_in: f64,
        len: f64,
    },
    Reorder {
        id: Id,
        grab_dx: f32,
    },
}

pub struct TimelineView {
    pub pps: f32,
    /// Smallest zoom: the whole project fits on screen (updated each frame).
    pub min_pps: f32,
    pub scroll: f64,
    pub drag: TlDrag,
    pub snap_line: Option<f64>,
}

pub struct ExportUi {
    pub open: bool,
    pub scale_1080: bool,
    pub quality: Quality,
    pub out: PathBuf,
    pub job: Option<ExportJob>,
}

/// One preview layer: the latest raw frame for a clip and its graded texture.
pub struct Layer {
    pub tex: Option<TextureHandle>,
    pub blur: Option<TextureHandle>,
    pub media: Option<Id>,
    pub clip: Option<Id>,
    raw: Option<Frame>,
    graded: Option<crate::fx::Adjust>,
    requested: Option<(Id, i64)>,
    grabber: Grabber,
}

impl Layer {
    fn new(ctx: egui::Context) -> Self {
        Self { tex: None, blur: None, media: None, clip: None, raw: None, graded: None, requested: None, grabber: Grabber::new(ctx) }
    }

    /// (Re)build the textures from the raw frame with `grade` applied.
    fn upload(&mut self, ctx: &egui::Context, grade: crate::fx::Adjust) {
        let Some(raw) = &self.raw else { return };
        let mut f = raw.clone();
        grade.apply(&mut f.rgba);
        let img = to_image(&f);
        match &mut self.tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("preview_layer", img, TextureOptions::LINEAR)),
        }
        let tiny = to_image(&media::tiny_blur(&f, 40));
        match &mut self.blur {
            Some(t) => t.set(tiny, TextureOptions::LINEAR),
            None => self.blur = Some(ctx.load_texture("preview_layer_blur", tiny, TextureOptions::LINEAR)),
        }
        self.graded = Some(grade);
    }
}

pub struct KlipApp {
    pub screen: Screen,
    pub store: Store,
    pub home: HomeState,
    /// Store id of the project open in the editor.
    pub project_id: Option<String>,
    pub project: Project,
    committed: Project,
    saved: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,

    pub selection: Selection,
    pub lib_selected: Option<Id>,
    pub left_tab: LeftTab,
    pub video_tab: VideoTab,
    pub playhead: f64,
    pub playing: bool,
    play_anchor: (Instant, f64),
    /// Play was pressed; waiting for the first frame before starting the clock.
    starting: Option<Instant>,
    first_frame: bool,

    pub assets: HashMap<Id, Assets>,
    jobs_tx: Sender<Job>,
    jobs_rx: Receiver<Job>,
    pub importing: usize,
    dialog_tx: Sender<DialogResult>,
    dialog_rx: Receiver<DialogResult>,

    /// Layer 0: clip under the playhead. Layer 1: incoming clip during a transition.
    pub layers: [Layer; 2],
    decoder: Option<StreamDecoder>,
    next_decoder: Option<StreamDecoder>,
    /// Audio voices of main-track clips currently playing.
    voices: Vec<String>,
    audio: AudioPreview,

    pub tl: TimelineView,
    /// Preview fills the whole screen (F / Esc).
    pub fullscreen: bool,
    /// Player view zoom (1 = canvas fits the stage) and pan, in screen px.
    pub view_zoom: f32,
    pub view_pan: egui::Vec2,
    pub export: ExportUi,
    pub toasts: Vec<Toast>,
    pub text_font: Option<PathBuf>,
    pub ctx: egui::Context,
    pub dev: crate::dev::Dev,
    /// When pending edits will be written to the store.
    autosave_due: Option<f64>,
    last_title: String,
}

fn default_export_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into());
    let videos = home.join("Videos");
    if videos.is_dir() { videos.join("Klip") } else { home.join("Klip") }
}

pub fn safe_file_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' })
        .collect();
    let n = n.trim().to_string();
    if n.is_empty() { "klip".into() } else { n }
}

impl KlipApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let text_font = theme::find_text_font();
        theme::install(&ctx, text_font.as_ref());
        let (jobs_tx, jobs_rx) = unbounded();
        let (dialog_tx, dialog_rx) = unbounded();
        let project = Project::default();
        let dev = crate::dev::Dev::from_env();
        // Automated runs get their own throwaway store unless told otherwise.
        let data = if dev.active() && std::env::var_os("KLIP_DATA_DIR").is_none() {
            std::env::temp_dir().join("klip_dev")
        } else {
            store::data_dir()
        };
        let mut app = Self {
            screen: Screen::Home,
            store: Store::at(data.join("projects")),
            home: HomeState {
                projects: vec![],
                trash: vec![],
                show_trash: false,
                search: String::new(),
                sort: HomeSort::Recent,
                thumbs: HashMap::new(),
                thumbs_pending: Default::default(),
                new_project: None,
                rename: None,
                menu: None,
                confirm_delete: None,
                undo_delete: None,
            },
            project_id: None,
            committed: project.clone(),
            saved: project.clone(),
            project,
            undo: vec![],
            redo: vec![],
            selection: Selection::None,
            lib_selected: None,
            left_tab: LeftTab::Media,
            video_tab: VideoTab::Basic,
            playhead: 0.0,
            playing: false,
            play_anchor: (Instant::now(), 0.0),
            starting: None,
            first_frame: false,
            assets: HashMap::new(),
            jobs_tx,
            jobs_rx,
            importing: 0,
            dialog_tx,
            dialog_rx,
            layers: [Layer::new(ctx.clone()), Layer::new(ctx.clone())],
            decoder: None,
            next_decoder: None,
            voices: vec![],
            audio: AudioPreview::default(),
            fullscreen: false,
            view_zoom: 1.0,
            view_pan: egui::Vec2::ZERO,
            tl: TimelineView { pps: 60.0, min_pps: 4.0, scroll: 0.0, drag: TlDrag::None, snap_line: None },
            export: ExportUi {
                open: false,
                scale_1080: true,
                quality: Quality::High,
                out: default_export_dir().join("Untitled reel.mp4"),
                job: None,
            },
            toasts: vec![],
            text_font,
            ctx,
            dev,
            autosave_due: None,
            last_title: String::new(),
        };
        app.migrate_v1_autosave(&data.join("autosave.klip"));
        app.store.purge_trash(store::TRASH_TTL);
        app.refresh_home();
        // `klip project.klip` imports and opens it; `klip a.mp4 b.mp4` starts
        // a new project with those clips.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        let (klips, media): (Vec<PathBuf>, Vec<PathBuf>) = args.into_iter().partition(|p| p.extension().is_some_and(|e| e == "klip"));
        if let Some(p) = klips.first() {
            app.import_project_file(p);
        } else if !media.is_empty() {
            let name = std::env::var("KLIP_DEMO_NAME").unwrap_or_else(|_| "Untitled reel".into());
            let aspect = std::env::var("KLIP_DEMO_ASPECT")
                .ok()
                .and_then(|a| Aspect::ALL.into_iter().find(|x| x.label() == a))
                .unwrap_or(Aspect::Reel);
            app.create_project(&name, aspect, media);
        }
        app
    }

    /// Klip 0.1 kept unsaved work in a single autosave file. Bring it into
    /// the store as a regular project so nothing is lost.
    fn migrate_v1_autosave(&mut self, path: &Path) {
        let Ok((mut p, _)) = store::read_project(path) else { return };
        if !p.video.is_empty() || !p.media.is_empty() {
            p.name = format!("{} (recovered)", p.name);
            if self.store.import_project(p).is_err() {
                return;
            }
            self.toast(ToastKind::Info, "Your last unsaved session was added to your projects");
        }
        let _ = std::fs::remove_file(path);
    }

    // ---------------------------------------------------------------- toasts

    pub fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push(Toast { msg: msg.into(), kind, until: now + 3.5 });
        self.ctx.request_repaint();
    }

    // --------------------------------------------------------------- dialogs

    fn spawn_dialog(&self, f: impl FnOnce() -> Option<DialogResult> + Send + 'static) {
        let tx = self.dialog_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            if let Some(r) = f() {
                let _ = tx.send(r);
                ctx.request_repaint();
            }
        });
    }

    pub fn dialog_import(&self) {
        self.spawn_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Import media")
                .add_filter(
                    "Media",
                    &["mp4", "mov", "mkv", "webm", "avi", "m4v", "png", "jpg", "jpeg", "webp", "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus"],
                )
                .add_filter("All files", &["*"])
                .pick_files()
                .map(DialogResult::Import)
        });
    }

    pub fn dialog_import_srt(&self) {
        self.spawn_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Import subtitles")
                .add_filter("Subtitles", &["srt"])
                .pick_file()
                .map(DialogResult::ImportSrt)
        });
    }

    pub fn dialog_import_project(&self) {
        self.spawn_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Import project")
                .add_filter("Klip project", &["klip"])
                .pick_file()
                .map(DialogResult::ImportProject)
        });
    }

    /// Point a missing media item at its new location.
    pub fn dialog_relink(&self, id: Id) {
        let Some(m) = self.project.media.iter().find(|m| m.id == id) else { return };
        let title = format!("Locate “{}”", m.name);
        let dir = m.path.parent().filter(|d| d.is_dir()).map(Path::to_path_buf);
        self.spawn_dialog(move || {
            let mut d = rfd::FileDialog::new().set_title(title);
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            d.pick_file().map(|p| DialogResult::Relink(id, p))
        });
    }

    pub fn dialog_export_to(&self) {
        let dir = self.export.out.parent().map(Path::to_path_buf).unwrap_or_else(default_export_dir);
        let _ = std::fs::create_dir_all(&dir);
        let name = self.export.out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.spawn_dialog(move || {
            rfd::FileDialog::new()
                .set_title("Export video")
                .set_directory(dir)
                .set_file_name(name)
                .add_filter("MP4 video", &["mp4"])
                .save_file()
                .map(DialogResult::ExportTo)
        });
    }

    // ---------------------------------------------------------------- import

    pub fn import(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if self.project.media.iter().any(|m| m.path == p) {
                continue;
            }
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("srt")) {
                self.import_srt(&p);
                continue;
            }
            if p.extension().is_some_and(|e| e == "klip") {
                self.import_project_file(&p);
                continue;
            }
            self.importing += 1;
            let tx = self.jobs_tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Job::Probed(media::probe(&p)));
                ctx.request_repaint();
            });
        }
    }

    fn add_media(&mut self, mut info: MediaInfo) {
        info.id = self.project.new_id();
        // Media isn't part of undo history: mirror it into the committed copy.
        self.committed.media.push(info.clone());
        self.committed.next_id = self.project.next_id;
        if self.project.video.is_empty() && self.project.media.iter().all(|m| m.kind == MediaKind::Audio) {
            self.lib_selected = Some(info.id);
        }
        self.project.media.push(info.clone());
        self.generate_assets(&info);
        if self.dev.demo {
            match info.kind {
                MediaKind::Audio => self.add_audio(info.id, 0.0),
                _ => self.add_to_main(info.id, None),
            }
        }
    }

    fn generate_assets(&mut self, info: &MediaInfo) {
        self.assets.entry(info.id).or_default();
        let ctx = self.ctx.clone();
        let tx = self.jobs_tx.clone();
        let info = info.clone();
        std::thread::spawn(move || {
            if info.kind != MediaKind::Audio {
                let (w, h) = media::fit_size(&info, 360, 360);
                let t = if info.kind == MediaKind::Video { (info.duration * 0.15).min(1.0) } else { 0.0 };
                if let Some(f) = media::grab_frame(&info.path, t, w, h) {
                    let _ = tx.send(Job::Thumb(info.id, f));
                    ctx.request_repaint();
                }
                let count = ((info.duration / 1.5).ceil() as usize).clamp(4, 64);
                media::filmstrip(&info, count, 96, |batch| {
                    let _ = tx.send(Job::Strip(info.id, batch));
                    ctx.request_repaint();
                });
            }
            if info.has_audio {
                let w = media::waveform(&info.path);
                let _ = tx.send(Job::Wave(info.id, w));
                ctx.request_repaint();
            }
        });
    }

    pub fn import_srt(&mut self, path: &Path) {
        let Ok(src) = std::fs::read_to_string(path) else {
            self.toast(ToastKind::Error, "Could not read subtitle file");
            return;
        };
        let cues = parse_srt(&src);
        if cues.is_empty() {
            self.toast(ToastKind::Error, "No subtitles found in that file");
            return;
        }
        let style = self.project.texts.iter().find(|t| t.is_caption).cloned();
        let n = cues.len();
        for (a, b, text) in cues {
            let id = self.project.new_id();
            let mut c = style.clone().unwrap_or_else(|| caption_template(0));
            c.id = id;
            c.text = text;
            c.start = a;
            c.dur = b - a;
            self.project.texts.push(c);
        }
        self.toast(ToastKind::Success, format!("Added {n} captions"));
    }

    pub fn remove_media(&mut self, id: Id) {
        self.project.media.retain(|m| m.id != id);
        self.committed.media.retain(|m| m.id != id);
        self.project.video.retain(|c| c.media != id);
        self.project.audio.retain(|c| c.media != id);
        self.assets.remove(&id);
        if self.lib_selected == Some(id) {
            self.lib_selected = None;
        }
        self.invalidate_preview();
    }

    fn process_jobs(&mut self) {
        while let Ok(job) = self.jobs_rx.try_recv() {
            match job {
                Job::Probed(r) => {
                    self.importing = self.importing.saturating_sub(1);
                    match r {
                        Ok(info) => self.add_media(info),
                        Err(e) => self.toast(ToastKind::Error, e),
                    }
                }
                Job::Thumb(id, f) => {
                    let tex = self.ctx.load_texture(format!("thumb{id}"), to_image(&f), TextureOptions::LINEAR);
                    let a = self.assets.entry(id).or_default();
                    a.thumb = Some(tex);
                    a.thumb_raw = Some(f);
                    a.filtered.clear();
                }
                Job::Strip(id, frames) => {
                    // Batches arrive coarse-to-fine; merge and keep sorted by time.
                    let a = self.assets.entry(id).or_default();
                    let base = a.strip.len();
                    for (i, (t, f)) in frames.into_iter().enumerate() {
                        let tex = self.ctx.load_texture(format!("strip{id}_{}", base + i), to_image(&f), TextureOptions::LINEAR);
                        a.strip.push((t, tex));
                    }
                    a.strip.sort_by(|x, y| x.0.total_cmp(&y.0));
                }
                Job::Wave(id, w) => {
                    self.assets.entry(id).or_default().wave = Some(Arc::new(w));
                }
                Job::ProjectThumb(id, edited, f) => {
                    self.home.thumbs_pending.remove(&id);
                    match f {
                        Some(f) => {
                            let tex = self.ctx.load_texture(format!("cover{id}"), to_image(&f), TextureOptions::LINEAR);
                            self.home.thumbs.insert(id, (edited, tex));
                        }
                        None => {
                            self.home.thumbs.remove(&id);
                        }
                    }
                }
            }
        }
        while let Ok(r) = self.dialog_rx.try_recv() {
            match r {
                DialogResult::Import(p) => self.import(p),
                DialogResult::ImportSrt(p) => self.import_srt(&p),
                DialogResult::ImportProject(p) => self.import_project_file(&p),
                DialogResult::Relink(id, p) => self.relink(id, p),
                DialogResult::ExportTo(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("mp4");
                    }
                    self.export.out = p;
                }
            }
        }
    }

    // -------------------------------------------------------------- projects

    /// Re-read the project lists shown on the home screen.
    pub fn refresh_home(&mut self) {
        self.home.projects = self.store.list();
        self.home.trash = self.store.list_trash();
    }

    /// Create a project in the store and open it, optionally with media.
    pub fn create_project(&mut self, name: &str, aspect: Aspect, media: Vec<PathBuf>) {
        match self.store.create(name, aspect) {
            Ok(meta) => {
                self.open_project(&meta.id);
                if !media.is_empty() {
                    self.import(media);
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Could not create project: {e}")),
        }
    }

    pub fn open_project(&mut self, id: &str) {
        self.leave_project();
        match self.store.load(id) {
            Ok(p) => {
                self.adopt_project(p);
                self.project_id = Some(id.to_string());
                self.screen = Screen::Editor;
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("Could not open project: {e}"));
                self.refresh_home();
            }
        }
    }

    /// Copy a `.klip` file into the store and open it.
    pub fn import_project_file(&mut self, path: &Path) {
        match self.store.import_file(path) {
            Ok(meta) => {
                self.open_project(&meta.id);
                self.toast(ToastKind::Success, format!("Imported “{}”", meta.name));
            }
            Err(e) => self.toast(ToastKind::Error, format!("Could not import project: {e}")),
        }
    }

    /// Save the open project and return to the project grid.
    pub fn go_home(&mut self) {
        self.leave_project();
        self.screen = Screen::Home;
        self.refresh_home();
    }

    /// Save the open project and refresh its cover.
    fn leave_project(&mut self) {
        self.stop();
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        self.export.open = false;
        self.save_now();
        if let Some(id) = self.project_id.take() {
            self.refresh_cover(&id);
        }
    }

    /// Re-render a project's cover in the background, then show it on Home.
    fn refresh_cover(&mut self, id: &str) {
        let p = self.project.clone();
        let path = self.store.thumb_path(id);
        let edited = self.store.meta(id).map(|m| m.edited).unwrap_or(0);
        let (tx, ctx, id) = (self.jobs_tx.clone(), self.ctx.clone(), id.to_string());
        self.home.thumbs_pending.insert(id.clone());
        std::thread::spawn(move || {
            store::write_thumb(&p, &path);
            let f = store::read_thumb(&path, p.aspect);
            let _ = tx.send(Job::ProjectThumb(id, edited, f));
            ctx.request_repaint();
        });
    }

    /// Load a cover image for the home grid (a few at a time).
    pub fn load_cover(&mut self, meta: &Meta) {
        if self.home.thumbs_pending.contains(&meta.id)
            || self.home.thumbs_pending.len() >= 4
            || self.home.thumbs.get(&meta.id).is_some_and(|t| t.0 == meta.edited)
        {
            return;
        }
        let path = self.store.cover_path(meta);
        if !path.exists() {
            return;
        }
        let (tx, ctx, id, edited, aspect) = (self.jobs_tx.clone(), self.ctx.clone(), meta.id.clone(), meta.edited, meta.aspect);
        self.home.thumbs_pending.insert(id.clone());
        std::thread::spawn(move || {
            let f = store::read_thumb(&path, aspect);
            let _ = tx.send(Job::ProjectThumb(id, edited, f));
            ctx.request_repaint();
        });
    }

    /// Write pending edits to the store now.
    pub fn save_now(&mut self) {
        self.autosave_due = None;
        let Some(id) = self.project_id.clone() else { return };
        if !self.is_dirty() {
            return;
        }
        match self.store.save(&id, &self.project) {
            Ok(_) => self.saved = self.project.clone(),
            Err(e) => {
                self.toast(ToastKind::Error, format!("Couldn't save project: {e}"));
                // Don't retry (and toast) every frame.
                self.autosave_due = Some(self.ctx.input(|i| i.time) + 30.0);
            }
        }
    }

    fn adopt_project(&mut self, p: Project) {
        self.stop();
        let missing = p.media.iter().filter(|m| !m.path.exists()).count();
        self.project = p.clone();
        self.committed = p.clone();
        self.saved = p;
        self.undo.clear();
        self.redo.clear();
        self.assets.clear();
        self.selection = Selection::None;
        self.lib_selected = None;
        self.playhead = 0.0;
        self.view_zoom = 1.0;
        self.view_pan = egui::Vec2::ZERO;
        self.export.out = default_export_dir().join(format!("{}.mp4", safe_file_name(&self.project.name)));
        for m in self.project.media.clone() {
            self.generate_assets(&m);
        }
        self.invalidate_preview();
        if missing > 0 {
            self.toast(ToastKind::Error, format!("{missing} media file(s) are missing. Click them in Media to locate."));
        }
    }

    /// Point a media item at a new file. Other missing files found in the
    /// same folder are relinked too.
    pub fn relink(&mut self, id: Id, path: PathBuf) {
        let mut fixed = 0;
        let dir = path.parent().map(Path::to_path_buf);
        for m in self.project.media.iter_mut() {
            let new = if m.id == id {
                Some(path.clone())
            } else if !m.path.exists() {
                dir.as_ref().and_then(|d| m.path.file_name().map(|n| d.join(n))).filter(|p| p.exists())
            } else {
                None
            };
            if let Some(new) = new {
                if let Ok(info) = media::probe(&new) {
                    m.width = info.width;
                    m.height = info.height;
                    m.duration = m.duration.max(info.duration);
                    m.has_audio = info.has_audio;
                }
                m.path = new;
                fixed += 1;
            }
        }
        for m in self.project.media.clone() {
            if m.path.exists() && self.assets.get(&m.id).is_none_or(|a| a.thumb.is_none()) {
                self.generate_assets(&m);
            }
        }
        self.invalidate_preview();
        self.toast(ToastKind::Success, if fixed == 1 { "Media relinked".to_string() } else { format!("{fixed} media files relinked") });
    }

    pub fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    /// Edits are written to the store a moment after they happen, and
    /// whenever the project is left or Klip quits.
    fn autosave(&mut self, ctx: &egui::Context) {
        if self.screen != Screen::Editor || !self.is_dirty() {
            self.autosave_due = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let busy = ctx.input(|i| i.pointer.any_down());
        match self.autosave_due {
            None => self.autosave_due = Some(now + 1.5),
            Some(t) if now >= t && !busy => self.save_now(),
            _ => {}
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    // ---------------------------------------------------------------- history

    fn commit_history(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.text_edit_focused();
        if !busy {
            self.commit_now();
        }
    }

    fn commit_now(&mut self) {
        if self.project != self.committed {
            let prev = std::mem::replace(&mut self.committed, self.project.clone());
            self.undo.push(prev);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn restore(&mut self, snapshot: Project) -> Project {
        let media = self.project.media.clone();
        let next_id = self.project.next_id;
        let cur = std::mem::replace(&mut self.project, snapshot);
        self.project.media = media;
        self.project.next_id = self.project.next_id.max(next_id);
        self.committed = self.project.clone();
        self.invalidate_preview();
        cur
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.project != self.committed
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.commit_now();
        if let Some(prev) = self.undo.pop() {
            let cur = self.restore(prev);
            self.redo.push(cur);
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let cur = self.restore(next);
            self.undo.push(cur);
        }
    }

    // ---------------------------------------------------------------- editing

    pub fn add_to_main(&mut self, media_id: Id, index: Option<usize>) {
        let Some(m) = self.project.media(media_id).cloned() else { return };
        if m.kind == MediaKind::Audio {
            self.add_audio(media_id, self.playhead);
            return;
        }
        let id = self.project.new_id();
        let dur = if m.kind == MediaKind::Image { 3.0 } else { m.duration };
        let fit = if (m.aspect() - aspect_of(self.project.aspect)).abs() < 0.08 { FitMode::Fill } else { FitMode::Blur };
        let clip = VideoClip::new(id, media_id, 0.0, dur, fit);
        let idx = index.unwrap_or(self.project.video.len()).min(self.project.video.len());
        self.project.video.insert(idx, clip);
        self.selection = Selection::Video(id);
        self.invalidate_preview();
    }

    pub fn add_audio(&mut self, media_id: Id, at: f64) {
        let Some(m) = self.project.media(media_id).cloned() else { return };
        if !m.has_audio {
            return;
        }
        let id = self.project.new_id();
        self.project.audio.push(AudioClip { id, media: media_id, start: at.max(0.0), src_in: 0.0, dur: m.duration, volume: 0.8 });
        self.selection = Selection::Audio(id);
    }

    pub fn add_text(&mut self, mut t: TextClip) {
        t.id = self.project.new_id();
        t.start = self.playhead;
        let remaining = self.project.duration() - self.playhead;
        t.dur = if remaining > 0.5 { remaining.min(3.0) } else { 3.0 };
        self.selection = Selection::Text(t.id);
        self.project.texts.push(t);
    }

    /// Main-track clip that clip-level actions (filters, …) apply to:
    /// the selected clip, else the clip under the playhead.
    pub fn target_clip(&self) -> Option<usize> {
        if let Selection::Video(id) = self.selection {
            return self.project.video_index(id);
        }
        self.project.video_at(self.playhead).map(|(i, _)| i)
    }

    /// Cut that a transition applies to (index of the outgoing clip):
    /// the selected transition, the cut after the selected clip, or the
    /// cut nearest the playhead.
    pub fn target_cut(&self) -> Option<usize> {
        let n = self.project.video.len();
        if n < 2 {
            return None;
        }
        match self.selection {
            Selection::Transition(id) | Selection::Video(id) => {
                if let Some(i) = self.project.video_index(id) {
                    return Some(i.min(n - 2));
                }
            }
            _ => {}
        }
        let starts = self.project.video_starts();
        (0..n - 1).min_by(|&a, &b| {
            let da = (starts[a + 1] - self.playhead).abs();
            let db = (starts[b + 1] - self.playhead).abs();
            da.total_cmp(&db)
        })
    }

    pub fn apply_transition(&mut self, kind: crate::fx::TransitionKind) {
        let Some(i) = self.target_cut() else {
            self.toast(ToastKind::Info, "Add at least two clips to use transitions");
            return;
        };
        let dur = self.project.video[i].transition.map_or(0.5, |t| t.dur);
        self.project.video[i].transition = Some(crate::fx::Transition { kind, dur });
        self.selection = Selection::Transition(self.project.video[i].id);
        let s = self.project.video_starts();
        self.seek((s[i + 1] - 0.2).max(0.0));
        self.invalidate_preview();
    }

    pub fn apply_transition_all(&mut self, kind: crate::fx::TransitionKind, dur: f64) {
        let n = self.project.video.len();
        for c in self.project.video.iter_mut().take(n.saturating_sub(1)) {
            c.transition = Some(crate::fx::Transition { kind, dur });
        }
        self.invalidate_preview();
    }

    pub fn apply_filter(&mut self, preset: crate::fx::FilterPreset) {
        let Some(i) = self.target_clip() else {
            self.toast(ToastKind::Info, "Add a clip to use filters");
            return;
        };
        self.project.video[i].filter = preset;
        self.selection = Selection::Video(self.project.video[i].id);
    }

    pub fn delete_selection(&mut self) {
        match self.selection {
            Selection::Video(id) => {
                self.project.video.retain(|c| c.id != id);
                self.invalidate_preview();
            }
            Selection::Text(id) => self.project.texts.retain(|c| c.id != id),
            Selection::Audio(id) => self.project.audio.retain(|c| c.id != id),
            Selection::Transition(id) => {
                if let Some(c) = self.project.video.iter_mut().find(|c| c.id == id) {
                    c.transition = None;
                }
                self.invalidate_preview();
            }
            Selection::None => return,
        }
        self.selection = Selection::None;
        self.playhead = self.playhead.min(self.project.duration());
    }

    pub fn duplicate_selection(&mut self) {
        match self.selection {
            Selection::Video(id) => {
                if let Some(i) = self.project.video_index(id) {
                    let mut c = self.project.video[i].clone();
                    c.id = self.project.new_id();
                    self.selection = Selection::Video(c.id);
                    self.project.video.insert(i + 1, c);
                }
            }
            Selection::Text(id) => {
                if let Some(mut c) = self.project.texts.iter().find(|c| c.id == id).cloned() {
                    c.id = self.project.new_id();
                    c.start = c.end();
                    self.selection = Selection::Text(c.id);
                    self.project.texts.push(c);
                }
            }
            Selection::Audio(id) => {
                if let Some(mut c) = self.project.audio.iter().find(|c| c.id == id).cloned() {
                    c.id = self.project.new_id();
                    c.start = c.end();
                    self.selection = Selection::Audio(c.id);
                    self.project.audio.push(c);
                }
            }
            Selection::None | Selection::Transition(_) => {}
        }
        self.invalidate_preview();
    }

    pub fn split(&mut self) {
        let t = self.playhead;
        let done = match self.selection {
            Selection::Text(id) => self.project.split_text(id, t).map(Selection::Text),
            Selection::Audio(id) => self.project.split_audio(id, t).map(Selection::Audio),
            _ => self.project.split_video(t).map(Selection::Video),
        };
        match done {
            Some(sel) => {
                self.selection = sel;
                self.invalidate_preview();
            }
            None => self.toast(ToastKind::Info, "Move the playhead over a clip to split it"),
        }
    }

    fn validate_selection(&mut self) {
        let ok = match self.selection {
            Selection::None => true,
            Selection::Video(id) => self.project.video.iter().any(|c| c.id == id),
            Selection::Text(id) => self.project.texts.iter().any(|c| c.id == id),
            Selection::Audio(id) => self.project.audio.iter().any(|c| c.id == id),
            Selection::Transition(id) => self
                .project
                .video_index(id)
                .is_some_and(|i| i + 1 < self.project.video.len() && self.project.video[i].transition.is_some()),
        };
        if !ok {
            self.selection = Selection::None;
        }
    }

    // --------------------------------------------------------------- playback

    pub fn toggle_play(&mut self) {
        if std::env::var_os("KLIP_DEBUG_PLAYBACK").is_some() {
            eprintln!("[play] toggle_play (playing={}, playhead={:.2}, clips={})", self.playing, self.playhead, self.project.video.len());
        }
        if self.playing {
            self.stop();
        } else {
            if self.project.video.is_empty() {
                return;
            }
            if self.playhead >= self.project.duration() - 0.02 {
                self.playhead = 0.0;
            }
            self.playing = true;
            self.begin_preroll();
        }
    }

    /// Hold the clock until decoders have produced the first frame, so video
    /// and audio (which start decoding at the same moment) begin together.
    fn begin_preroll(&mut self) {
        self.starting = Some(Instant::now());
        self.first_frame = false;
        self.play_anchor = (Instant::now(), self.playhead);
    }

    #[track_caller]
    pub fn stop(&mut self) {
        if std::env::var_os("KLIP_DEBUG_PLAYBACK").is_some() {
            eprintln!("[play] stop() from {} (playing={}, starting={})", std::panic::Location::caller(), self.playing, self.starting.is_some());
        }
        self.playing = false;
        self.starting = None;
        self.decoder = None;
        self.next_decoder = None;
        self.audio.stop_all();
        self.voices.clear();
        for l in &mut self.layers {
            l.requested = None;
        }
    }

    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.project.duration().max(0.0));
        if (t - self.playhead).abs() < 1e-9 {
            return;
        }
        self.playhead = t;
        if self.playing {
            self.decoder = None;
            self.next_decoder = None;
            self.audio.stop_all();
            self.voices.clear();
            self.begin_preroll();
        }
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    pub fn invalidate_preview(&mut self) {
        for l in &mut self.layers {
            l.requested = None;
        }
        if self.playing {
            self.decoder = None;
            self.next_decoder = None;
            self.audio.stop_all();
            self.voices.clear();
            self.begin_preroll();
        }
    }

    /// Upload a decoded frame into a preview layer, applying the clip's grade.
    fn show_frame(&mut self, slot: usize, clip_id: Id, media_id: Id, f: Frame) {
        let grade = self
            .project
            .video
            .iter()
            .find(|c| c.id == clip_id)
            .map(|c| c.grade())
            .unwrap_or_default();
        if slot == 0 {
            self.first_frame = true;
            if std::env::var_os("KLIP_DEBUG_PLAYBACK").is_some() && self.playing {
                let now = self.ctx.input(|i| i.time);
                let (n, since) = self.ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(egui::Id::new("fps_dbg"), || (0u32, now)));
                if now - since >= 1.0 {
                    eprintln!("[play] {n} frames/s shown, playhead {:.2}", self.playhead);
                    self.ctx.data_mut(|d| d.insert_temp(egui::Id::new("fps_dbg"), (1u32, now)));
                } else {
                    self.ctx.data_mut(|d| d.insert_temp(egui::Id::new("fps_dbg"), (n + 1, since)));
                }
            }
        }
        let l = &mut self.layers[slot];
        l.raw = Some(f);
        l.media = Some(media_id);
        l.clip = Some(clip_id);
        l.upload(&self.ctx, grade);
    }

    /// Keep layer `slot` showing main-track clip `idx` at the playhead.
    fn feed_layer(&mut self, slot: usize, idx: usize, clip_start: f64) {
        let t = self.playhead;
        let clip = self.project.video[idx].clone();
        let Some(m) = self.project.media(clip.media).cloned() else { return };
        let local = (t - clip_start).clamp(0.0, clip.dur());
        let (w, h) = media::fit_size(&m, 1280, 1280);
        self.layers[slot].clip = Some(clip.id);

        if self.playing && m.kind == MediaKind::Video {
            // Layer 0 uses `decoder` (adopting the pre-rolled one at a cut);
            // layer 1 reads from the pre-rolled `next_decoder`.
            if slot == 0 && self.decoder.as_ref().is_none_or(|d| d.clip_id != clip.id) {
                self.decoder = match self.next_decoder.take() {
                    Some(d) if d.clip_id == clip.id => Some(d),
                    other => {
                        self.next_decoder = other;
                        StreamDecoder::new(self.ctx.clone(), clip.id, &m.path, clip.src_in + local, clip.dur() - local, t, w, h)
                    }
                };
            }
            if slot == 1 && self.next_decoder.as_ref().is_none_or(|d| d.clip_id != clip.id) {
                self.next_decoder =
                    StreamDecoder::new(self.ctx.clone(), clip.id, &m.path, clip.src_in + local, clip.dur() - local, t, w, h);
            }
            let dec = if slot == 0 { self.decoder.as_mut() } else { self.next_decoder.as_mut() };
            if let Some(f) = dec.and_then(|d| d.frame_for(t)) {
                self.show_frame(slot, clip.id, m.id, f);
            }
        } else {
            let src_t = if m.kind == MediaKind::Image { 0.0 } else { clip.src_in + local };
            let key = (m.id, (src_t * FPS).round() as i64);
            let l = &mut self.layers[slot];
            if l.requested != Some(key) {
                l.requested = Some(key);
                l.grabber.request(GrabRequest { key, path: m.path.clone(), t: src_t, w, h });
            }
        }
        // Frames arriving from the scrub grabber.
        while let Ok((key, f)) = self.layers[slot].grabber.rx.try_recv() {
            if Some(key) == self.layers[slot].requested {
                self.show_frame(slot, clip.id, key.0, f);
            }
        }
        // Re-grade instantly when filter/adjust values change.
        let grade = clip.grade();
        if self.layers[slot].graded != Some(grade) {
            let ctx = self.ctx.clone();
            self.layers[slot].upload(&ctx, grade);
        }
    }

    fn update_playback(&mut self) {
        let total = self.project.duration();
        if let (true, Some(since)) = (self.playing, self.starting) {
            // Pre-roll: keep the playhead still until the first frame is in
            // (photos are instant), or give up waiting after a moment.
            let still_media = self
                .project
                .video_at(self.playhead)
                .and_then(|(i, _)| self.project.media(self.project.video[i].media))
                .is_some_and(|m| m.kind != MediaKind::Video);
            if self.first_frame || still_media || since.elapsed().as_secs_f64() > 2.0 {
                if std::env::var_os("KLIP_DEBUG_PLAYBACK").is_some() {
                    eprintln!("[play] clock started after {:.0} ms pre-roll (first frame: {})", since.elapsed().as_secs_f64() * 1000.0, self.first_frame);
                }
                self.starting = None;
                self.play_anchor = (Instant::now(), self.playhead);
            }
            self.ctx.request_repaint();
        } else if self.playing {
            self.audio.reap();
            let t = self.play_anchor.1 + self.play_anchor.0.elapsed().as_secs_f64();
            if t >= total {
                self.playhead = total;
                self.stop();
            } else {
                self.playhead = t;
                self.ctx.request_repaint();
            }
        }

        let t = self.playhead;
        let Some((i, start)) = self.project.video_at(t) else {
            for l in &mut self.layers {
                l.clip = None;
                l.media = None;
            }
            self.decoder = None;
            return;
        };
        let starts = self.project.video_starts();
        self.feed_layer(0, i, start);

        // Incoming clip during a transition.
        let in_transition = self.project.transition_at(t).is_some_and(|(j, _)| j == i);
        if in_transition {
            self.feed_layer(1, i + 1, starts[i + 1]);
        } else {
            self.layers[1].clip = None;
        }

        // Pre-roll the next clip so cuts and transitions start instantly.
        if self.playing && !in_transition && self.next_decoder.is_none() {
            if let (Some(next), Some(&ns)) = (self.project.video.get(i + 1).cloned(), starts.get(i + 1)) {
                if ns - t < 1.0 {
                    if let Some(nm) = self.project.media(next.media).cloned() {
                        if nm.kind == MediaKind::Video {
                            let (nw, nh) = media::fit_size(&nm, 1280, 1280);
                            self.next_decoder =
                                StreamDecoder::new(self.ctx.clone(), next.id, &nm.path, next.src_in, next.dur(), ns, nw, nh);
                        }
                    }
                }
            }
        }

        // ---- audio preview
        if self.playing {
            let mut want: Vec<(usize, f64)> = vec![(i, start)];
            if in_transition {
                want.push((i + 1, starts[i + 1]));
            }
            let keys: Vec<String> = want.iter().map(|(k, _)| format!("v{}", self.project.video[*k].id)).collect();
            for old in self.voices.clone() {
                if !keys.contains(&old) {
                    self.audio.stop(&old);
                    self.voices.retain(|v| v != &old);
                }
            }
            for ((k, s), key) in want.into_iter().zip(keys) {
                if self.voices.contains(&key) {
                    continue;
                }
                let c = self.project.video[k].clone();
                if let Some(m) = self.project.media(c.media).cloned() {
                    if m.has_audio {
                        let local = (t - s).clamp(0.0, c.dur());
                        self.audio.play(key.clone(), &m.path, c.src_in + local, c.dur() - local, c.volume);
                    }
                }
                self.voices.push(key);
            }
            for a in self.project.audio.clone() {
                let k = format!("a{}", a.id);
                if t >= a.start && t < a.end() && !self.audio.is_playing(&k) {
                    if let Some(am) = self.project.media(a.media) {
                        let off = t - a.start;
                        let len = (a.end() - t).min(total - t);
                        let path = am.path.clone();
                        self.audio.play(k, &path, a.src_in + off, len, a.volume);
                    }
                }
            }
        }
    }

    // --------------------------------------------------------------- keyboard

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        // Only a focused *text field* should swallow shortcuts. (egui also
        // focuses buttons/clips on click, which used to block Space.)
        let typing = ctx.text_edit_focused();
        if self.screen == Screen::Home {
            let (new, open) = ctx.input_mut(|i| (i.consume_key(Modifiers::COMMAND, Key::N), i.consume_key(Modifiers::COMMAND, Key::O)));
            if new && self.home.new_project.is_none() {
                self.home.new_project = Some(NewProject { name: self.store.unique_name("Untitled reel"), aspect: Aspect::Reel, media: vec![] });
            }
            if open {
                self.dialog_import_project();
            }
            return;
        }
        let mut act: Vec<&str> = vec![];
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y) {
                act.push("redo");
            }
            if !typing && i.consume_key(Modifiers::COMMAND, Key::Z) {
                act.push("undo");
            }
            if i.consume_key(Modifiers::COMMAND, Key::S) {
                act.push("save");
            }
            if i.consume_key(Modifiers::COMMAND, Key::I) {
                act.push("import");
            }
            if i.consume_key(Modifiers::COMMAND, Key::E) {
                act.push("export");
            }
            if i.consume_key(Modifiers::COMMAND, Key::O) {
                act.push("open");
            }
            if typing {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Space) {
                act.push("play");
            }
            if i.consume_key(Modifiers::NONE, Key::S) || i.consume_key(Modifiers::COMMAND, Key::B) {
                act.push("split");
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace) {
                act.push("delete");
            }
            if i.consume_key(Modifiers::COMMAND, Key::D) {
                act.push("dup");
            }
            if i.consume_key(Modifiers::SHIFT, Key::ArrowLeft) {
                act.push("back1s");
            }
            if i.consume_key(Modifiers::SHIFT, Key::ArrowRight) {
                act.push("fwd1s");
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                act.push("back");
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                act.push("fwd");
            }
            if i.consume_key(Modifiers::NONE, Key::Home) {
                act.push("home");
            }
            if i.consume_key(Modifiers::NONE, Key::End) {
                act.push("end");
            }
            if i.consume_key(Modifiers::NONE, Key::Equals) || i.consume_key(Modifiers::NONE, Key::Plus) {
                act.push("zoomin");
            }
            if i.consume_key(Modifiers::NONE, Key::Minus) {
                act.push("zoomout");
            }
            if i.consume_key(Modifiers::NONE, Key::F) || i.consume_key(Modifiers::NONE, Key::F11) {
                act.push("fullscreen");
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                act.push("deselect");
            }
        });
        for a in act {
            match a {
                "undo" => self.undo(),
                "redo" => self.redo(),
                "save" => self.save_now(),
                "import" => self.dialog_import(),
                "open" => self.dialog_import_project(),
                "export" => self.export.open = true,
                "play" => self.toggle_play(),
                "split" => self.split(),
                "delete" => self.delete_selection(),
                "dup" => self.duplicate_selection(),
                "back" => self.seek(self.playhead - 1.0 / FPS),
                "fwd" => self.seek(self.playhead + 1.0 / FPS),
                "back1s" => self.seek(self.playhead - 1.0),
                "fwd1s" => self.seek(self.playhead + 1.0),
                "home" => self.seek(0.0),
                "end" => self.seek(self.project.duration()),
                "zoomin" => self.tl.pps = (self.tl.pps * 1.25).min(crate::ui::MAX_PPS),
                "zoomout" => self.tl.pps = (self.tl.pps / 1.25).max(self.tl.min_pps),
                "deselect" => {
                    if self.fullscreen {
                        self.set_fullscreen(false);
                    } else {
                        self.selection = Selection::None;
                    }
                }
                "fullscreen" => self.set_fullscreen(!self.fullscreen),
                _ => {}
            }
        }
    }

    fn handle_os_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if dropped.is_empty() {
            return;
        }
        if self.screen == Screen::Home {
            // A project file opens; media starts a new project with it.
            if let Some(k) = dropped.iter().find(|p| p.extension().is_some_and(|e| e == "klip")) {
                self.import_project_file(&k.clone());
            } else {
                self.home.new_project = Some(NewProject { name: self.store.unique_name("Untitled reel"), aspect: Aspect::Reel, media: dropped });
            }
        } else {
            self.import(dropped);
        }
    }
}

impl eframe::App for KlipApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_jobs();
        self.handle_os_drops(&ctx);
        self.shortcuts(&ctx);
        self.validate_selection();
        self.update_playback();

        crate::ui::draw(self, ui);

        self.commit_history(&ctx);
        let title = match self.screen {
            Screen::Home => "Klip".to_string(),
            Screen::Editor => format!("{} — Klip", self.project.name),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        crate::dev::tick(self, &ctx);
        self.autosave(&ctx);
        if self.importing > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self) {
        self.stop();
        self.save_now();
        if let Some(id) = &self.project_id {
            store::write_thumb(&self.project, &self.store.thumb_path(id));
        }
        if let Some(j) = &self.export.job {
            if j.state().1 == crate::export::ExportStatus::Running {
                j.cancel();
            }
        }
    }
}

pub fn to_image(f: &Frame) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba)
}

pub fn aspect_of(a: Aspect) -> f32 {
    let (w, h) = a.canvas();
    w as f32 / h as f32
}

/// Default look for imported/auto captions.
pub fn caption_template(id: Id) -> TextClip {
    TextClip {
        id,
        text: String::new(),
        start: 0.0,
        dur: 2.0,
        x: 0.5,
        y: 0.72,
        size: 66.0,
        color: [255, 255, 255],
        style: TextStyle::Outline,
        is_caption: true,
    }
}
