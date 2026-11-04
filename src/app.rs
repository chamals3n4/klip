//! App state and the non-UI logic: import pipeline, playback engine,
//! undo/redo, project I/O and keyboard shortcuts.

use crate::audio::AudioPreview;
use crate::export::{ExportJob, Quality};
use crate::media::{self, Frame, GrabRequest, Grabber, StreamDecoder};
use crate::model::*;
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Media,
    Audio,
    Text,
    Captions,
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
    pub strip: Vec<(f64, TextureHandle)>,
    pub wave: Option<Arc<Vec<f32>>>,
}

pub enum Job {
    Probed(Result<MediaInfo, String>),
    Thumb(Id, Frame),
    Strip(Id, Vec<(f64, Frame)>),
    Wave(Id, Vec<f32>),
}

pub enum DialogResult {
    Import(Vec<PathBuf>),
    ImportSrt(PathBuf),
    Open(PathBuf),
    SaveAs(PathBuf),
    ExportTo(PathBuf),
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

pub struct KlipApp {
    pub project: Project,
    pub project_path: Option<PathBuf>,
    committed: Project,
    saved: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,

    pub selection: Selection,
    pub lib_selected: Option<Id>,
    pub left_tab: LeftTab,
    pub playhead: f64,
    pub playing: bool,
    play_anchor: (Instant, f64),

    pub assets: HashMap<Id, Assets>,
    jobs_tx: Sender<Job>,
    jobs_rx: Receiver<Job>,
    pub importing: usize,
    dialog_tx: Sender<DialogResult>,
    dialog_rx: Receiver<DialogResult>,

    grabber: Grabber,
    requested: Option<(Id, i64)>,
    decoder: Option<StreamDecoder>,
    next_decoder: Option<StreamDecoder>,
    voice: Option<String>,
    audio: AudioPreview,
    pub preview_tex: Option<TextureHandle>,
    pub preview_blur: Option<TextureHandle>,
    /// Which media the preview texture currently shows (None = nothing).
    pub preview_media: Option<Id>,

    pub tl: TimelineView,
    /// Preview fills the whole screen (F / Esc).
    pub fullscreen: bool,
    pub export: ExportUi,
    pub toasts: Vec<Toast>,
    pub text_font: Option<PathBuf>,
    pub ctx: egui::Context,
    pub dev: crate::dev::Dev,
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
        let mut app = Self {
            committed: project.clone(),
            saved: project.clone(),
            project,
            project_path: None,
            undo: vec![],
            redo: vec![],
            selection: Selection::None,
            lib_selected: None,
            left_tab: LeftTab::Media,
            playhead: 0.0,
            playing: false,
            play_anchor: (Instant::now(), 0.0),
            assets: HashMap::new(),
            jobs_tx,
            jobs_rx,
            importing: 0,
            dialog_tx,
            dialog_rx,
            grabber: Grabber::new(ctx.clone()),
            requested: None,
            decoder: None,
            next_decoder: None,
            voice: None,
            audio: AudioPreview::default(),
            preview_tex: None,
            preview_blur: None,
            preview_media: None,
            fullscreen: false,
            tl: TimelineView { pps: 60.0, scroll: 0.0, drag: TlDrag::None, snap_line: None },
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
            dev: crate::dev::Dev::from_env(),
            last_title: String::new(),
        };
        // `klip file.mp4 project.klip …` on the command line.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if let Some(p) = args.iter().find(|p| p.extension().is_some_and(|e| e == "klip")) {
            app.open_project(p);
        }
        let media: Vec<PathBuf> = args.into_iter().filter(|p| p.extension().is_none_or(|e| e != "klip")).collect();
        if !media.is_empty() {
            app.import(media);
        }
        app
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

    pub fn dialog_open(&self) {
        self.spawn_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Open project")
                .add_filter("Klip project", &["klip"])
                .pick_file()
                .map(DialogResult::Open)
        });
    }

    pub fn dialog_save_as(&self) {
        let name = format!("{}.klip", safe_file_name(&self.project.name));
        self.spawn_dialog(move || {
            rfd::FileDialog::new()
                .set_title("Save project")
                .set_file_name(name)
                .add_filter("Klip project", &["klip"])
                .save_file()
                .map(DialogResult::SaveAs)
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
                self.open_project(&p);
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
                let count = ((info.duration / 1.5).ceil() as usize).clamp(4, 40);
                let strip = media::filmstrip(&info, count, 96);
                let _ = tx.send(Job::Strip(info.id, strip));
                ctx.request_repaint();
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
                    self.assets.entry(id).or_default().thumb = Some(tex);
                }
                Job::Strip(id, frames) => {
                    let strip = frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (t, f))| {
                            (t, self.ctx.load_texture(format!("strip{id}_{i}"), to_image(&f), TextureOptions::LINEAR))
                        })
                        .collect();
                    self.assets.entry(id).or_default().strip = strip;
                }
                Job::Wave(id, w) => {
                    self.assets.entry(id).or_default().wave = Some(Arc::new(w));
                }
            }
        }
        while let Ok(r) = self.dialog_rx.try_recv() {
            match r {
                DialogResult::Import(p) => self.import(p),
                DialogResult::ImportSrt(p) => self.import_srt(&p),
                DialogResult::Open(p) => self.open_project(&p),
                DialogResult::SaveAs(p) => self.save_to(p),
                DialogResult::ExportTo(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("mp4");
                    }
                    self.export.out = p;
                }
            }
        }
    }

    // ------------------------------------------------------- project file I/O

    pub fn save(&mut self) {
        match self.project_path.clone() {
            Some(p) => self.save_to(p),
            None => self.dialog_save_as(),
        }
    }

    fn save_to(&mut self, mut path: PathBuf) {
        if path.extension().is_none() {
            path.set_extension("klip");
        }
        let json = serde_json::json!({ "app": "klip", "version": 1, "project": self.project });
        match std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()) {
            Ok(()) => {
                self.saved = self.project.clone();
                self.project_path = Some(path);
                self.toast(ToastKind::Success, "Project saved");
            }
            Err(e) => self.toast(ToastKind::Error, format!("Save failed: {e}")),
        }
    }

    pub fn open_project(&mut self, path: &Path) {
        let loaded = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(|e| e.to_string()))
            .and_then(|v| serde_json::from_value::<Project>(v["project"].clone()).map_err(|e| e.to_string()));
        match loaded {
            Ok(p) => {
                self.stop();
                let missing = p.media.iter().filter(|m| !m.path.exists()).count();
                self.project = p.clone();
                self.committed = p.clone();
                self.saved = p;
                self.undo.clear();
                self.redo.clear();
                self.assets.clear();
                self.selection = Selection::None;
                self.playhead = 0.0;
                self.project_path = Some(path.to_path_buf());
                self.export.out = default_export_dir().join(format!("{}.mp4", safe_file_name(&self.project.name)));
                for m in self.project.media.clone() {
                    self.generate_assets(&m);
                }
                self.invalidate_preview();
                if missing > 0 {
                    self.toast(ToastKind::Error, format!("{missing} media file(s) are missing"));
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Could not open project: {e}")),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    // ---------------------------------------------------------------- history

    fn commit_history(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
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
        let clip = VideoClip { id, media: media_id, src_in: 0.0, src_out: dur, fit, volume: 1.0 };
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

    pub fn delete_selection(&mut self) {
        match self.selection {
            Selection::Video(id) => {
                self.project.video.retain(|c| c.id != id);
                self.invalidate_preview();
            }
            Selection::Text(id) => self.project.texts.retain(|c| c.id != id),
            Selection::Audio(id) => self.project.audio.retain(|c| c.id != id),
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
            Selection::None => {}
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
        };
        if !ok {
            self.selection = Selection::None;
        }
    }

    // --------------------------------------------------------------- playback

    pub fn toggle_play(&mut self) {
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
            self.play_anchor = (Instant::now(), self.playhead);
        }
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.decoder = None;
        self.next_decoder = None;
        self.audio.stop_all();
        self.voice = None;
        self.requested = None;
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
            self.voice = None;
            self.play_anchor = (Instant::now(), t);
        }
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    pub fn invalidate_preview(&mut self) {
        self.requested = None;
        if self.playing {
            self.decoder = None;
            self.next_decoder = None;
            self.audio.stop_all();
            self.voice = None;
            self.play_anchor = (Instant::now(), self.playhead);
        }
    }

    fn show_frame(&mut self, media_id: Id, f: &Frame) {
        let img = to_image(f);
        match &mut self.preview_tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.preview_tex = Some(self.ctx.load_texture("preview", img, TextureOptions::LINEAR)),
        }
        let tiny = to_image(&media::tiny_blur(f, 40));
        match &mut self.preview_blur {
            Some(t) => t.set(tiny, TextureOptions::LINEAR),
            None => self.preview_blur = Some(self.ctx.load_texture("preview_blur", tiny, TextureOptions::LINEAR)),
        }
        self.preview_media = Some(media_id);
    }

    fn update_playback(&mut self) {
        let total = self.project.duration();
        if self.playing {
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
            self.preview_media = None;
            self.decoder = None;
            return;
        };
        let clip = self.project.video[i].clone();
        let Some(m) = self.project.media(clip.media).cloned() else { return };
        let local = (t - start).clamp(0.0, clip.dur());
        let (w, h) = media::fit_size(&m, 1280, 1280);

        if self.playing && m.kind == MediaKind::Video {
            if self.decoder.as_ref().is_none_or(|d| d.clip_id != clip.id) {
                self.decoder = match self.next_decoder.take() {
                    Some(d) if d.clip_id == clip.id => Some(d),
                    _ => StreamDecoder::new(
                        self.ctx.clone(),
                        clip.id,
                        &m.path,
                        clip.src_in + local,
                        clip.dur() - local,
                        t,
                        w,
                        h,
                    ),
                };
            }
            if let Some(f) = self.decoder.as_mut().and_then(|d| d.frame_for(t)) {
                self.show_frame(m.id, &f);
            }
            // Pre-roll the next clip so cuts are seamless.
            let end = start + clip.dur();
            if end - t < 1.0 && self.next_decoder.is_none() {
                if let Some(next) = self.project.video.get(i + 1).cloned() {
                    if let Some(nm) = self.project.media(next.media).cloned() {
                        if nm.kind == MediaKind::Video {
                            let (nw, nh) = media::fit_size(&nm, 1280, 1280);
                            self.next_decoder = StreamDecoder::new(
                                self.ctx.clone(), next.id, &nm.path, next.src_in, next.dur(), end, nw, nh,
                            );
                        }
                    }
                }
            }
        } else {
            self.decoder = None;
            let src_t = if m.kind == MediaKind::Image { 0.0 } else { clip.src_in + local };
            let key = (m.id, (src_t * FPS).round() as i64);
            let shown = self.preview_media == Some(m.id) && self.requested == Some(key);
            if !shown && self.requested != Some(key) {
                self.requested = Some(key);
                self.grabber.request(GrabRequest { key, path: m.path.clone(), t: src_t, w, h });
            }
        }
        while let Ok((key, f)) = self.grabber.rx.try_recv() {
            if Some(key) == self.requested || self.preview_media.is_none() {
                self.show_frame(key.0, &f);
            }
        }

        // ---- audio preview
        if self.playing {
            let key = format!("v{}", clip.id);
            if self.voice.as_deref() != Some(&key) {
                if let Some(old) = self.voice.take() {
                    self.audio.stop(&old);
                }
                if m.has_audio {
                    self.audio.play(key.clone(), &m.path, clip.src_in + local, clip.dur() - local, clip.volume);
                }
                self.voice = Some(key);
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
        let typing = ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input();
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
                "save" => self.save(),
                "import" => self.dialog_import(),
                "open" => self.dialog_open(),
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
                "zoomin" => self.tl.pps = (self.tl.pps * 1.25).min(800.0),
                "zoomout" => self.tl.pps = (self.tl.pps / 1.25).max(4.0),
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
        if !dropped.is_empty() {
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
        let title = format!(
            "{}{} — Klip",
            if self.is_dirty() { "● " } else { "" },
            self.project.name
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        crate::dev::tick(self, &ctx);
        if self.importing > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self) {
        self.stop();
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
