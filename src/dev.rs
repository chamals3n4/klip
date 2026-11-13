//! Developer hooks, all driven by environment variables (no effect otherwise):
//!
//! - `KLIP_DEMO=1`              imported media is auto-added to the timeline
//! - `KLIP_DEMO_SEEK=1.5`       move the playhead once imports finish
//! - `KLIP_DEMO_SELECT=text`    select the first video|text|audio clip
//! - `KLIP_DEMO_TAB=text`       open a library tab (media|audio|text|captions)
//! - `KLIP_DEMO_PLAY=1`        start playback once imports finish
//! - `KLIP_DEMO_EXPORT=open|run` open the export dialog (and start rendering)
//! - `KLIP_DEMO_FULLSCREEN=1` open the full-screen preview
//! - `KLIP_DEMO_TRANSITION=slideleft`, `KLIP_DEMO_FILTER=film`,
//!   `KLIP_DEMO_TRANSFORM=scale,x,y,deg`, `KLIP_DEMO_SPEED=2`,
//!   `KLIP_DEMO_AFADE=in,out` pre-apply effects
//! - `KLIP_DEMO_NAME`, `KLIP_DEMO_ASPECT=9:16` name / ratio of the project
//!   created from media on the command line
//! - `KLIP_DEMO_HOME=1`        go back to the home screen once the demo is set up
//! - `KLIP_DEMO_DIALOG=new|rename`, `KLIP_DEMO_MENU=1`, `KLIP_DEMO_TRASH=1`,
//!   `KLIP_DEMO_DELETE=<name>`, `KLIP_DEMO_OPEN=<name>` drive the home screen
//! - `KLIP_DATA_DIR=dir`       where projects are stored (default for
//!   automated runs: a throwaway folder in /tmp)
//! - `KLIP_MUTE=1`             no preview audio
//! - `KLIP_SCREENSHOT=out.png`  save a screenshot after `KLIP_SCREENSHOT_DELAY` s, then quit

use crate::app::{KlipApp, LeftTab, NewProject, Screen, Selection};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Default)]
pub struct Dev {
    pub demo: bool,
    shot: Option<PathBuf>,
    delay: f64,
    requested: bool,
    applied: bool,
    home_applied: bool,
}

impl Dev {
    /// Running under automation (uses a throwaway project store).
    pub fn active(&self) -> bool {
        self.demo || self.shot.is_some()
    }

    pub fn from_env() -> Self {
        Self {
            demo: std::env::var_os("KLIP_DEMO").is_some(),
            shot: std::env::var_os("KLIP_SCREENSHOT").map(PathBuf::from),
            delay: std::env::var("KLIP_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0),
            requested: false,
            applied: false,
            home_applied: false,
        }
    }
}

pub fn tick(app: &mut KlipApp, ctx: &egui::Context) {
    if app.dev.demo && !app.dev.applied && app.importing == 0 && !app.project.media.is_empty() {
        app.dev.applied = true;
        if let Some(t) = std::env::var("KLIP_DEMO_SEEK").ok().and_then(|v| v.parse::<f64>().ok()) {
            app.seek(t);
        }
        app.selection = match std::env::var("KLIP_DEMO_SELECT").as_deref() {
            Ok("video") => app.project.video.first().map_or(Selection::None, |c| Selection::Video(c.id)),
            Ok("text") => app.project.texts.first().map_or(Selection::None, |c| Selection::Text(c.id)),
            Ok("audio") => app.project.audio.first().map_or(Selection::None, |c| Selection::Audio(c.id)),
            _ => Selection::None,
        };
        // KLIP_DEMO_TRANSITION=slideleft  → on every cut
        if let Ok(name) = std::env::var("KLIP_DEMO_TRANSITION") {
            if let Some(k) = crate::fx::TransitionKind::ALL.into_iter().find(|k| k.xfade() == name || format!("{k:?}").eq_ignore_ascii_case(&name)) {
                app.apply_transition_all(k, 0.8);
            }
        }
        // KLIP_DEMO_FILTER=film  → first clip
        if let Ok(name) = std::env::var("KLIP_DEMO_FILTER") {
            if let Some(f) = crate::fx::FilterPreset::ALL.into_iter().find(|f| f.label().eq_ignore_ascii_case(&name)) {
                if let Some(c) = app.project.video.first_mut() {
                    c.filter = f;
                }
            }
        }
        // KLIP_DEMO_AFADE=in,out  → sound fades on the first music and video clip
        if let Ok(v) = std::env::var("KLIP_DEMO_AFADE") {
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let [fi, fo] = n.as_slice() {
                if let Some(a) = app.project.audio.first_mut() {
                    (a.fade_in, a.fade_out) = (*fi, *fo);
                }
                if let Some(c) = app.project.video.first_mut() {
                    (c.audio_fade_in, c.audio_fade_out) = (*fi, *fo);
                }
            }
        }
        // KLIP_DEMO_SPEED=2  → first clip
        if let Some(v) = std::env::var("KLIP_DEMO_SPEED").ok().and_then(|v| v.parse::<f32>().ok()) {
            if let Some(c) = app.project.video.first_mut() {
                c.set_speed(v);
            }
        }
        // KLIP_DEMO_TRANSFORM=scale,x,y,rotation  → first clip
        if let Ok(v) = std::env::var("KLIP_DEMO_TRANSFORM") {
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let (Some(c), [s, x, y, r]) = (app.project.video.first_mut(), n.as_slice()) {
                c.scale = *s;
                c.x = *x;
                c.y = *y;
                c.rotation = *r;
            }
        }
        if let Some(t) = std::env::var("KLIP_DEMO_SEEK").ok().and_then(|v| v.parse::<f64>().ok()) {
            app.seek(t);
        }
        if std::env::var("KLIP_DEMO_SELECT").as_deref() == Ok("transition") {
            if let Some(c) = app.project.video.first() {
                app.selection = Selection::Transition(c.id);
            }
        }
        if let Some(z) = std::env::var("KLIP_DEMO_VIEWZOOM").ok().and_then(|v| v.parse::<f32>().ok()) {
            app.view_zoom = z;
        }
        // KLIP_DEMO_ZOOM=out → fully zoomed out (clamped to the fit limit)
        if std::env::var("KLIP_DEMO_ZOOM").as_deref() == Ok("out") {
            app.tl.pps = 0.0;
        }
        if std::env::var_os("KLIP_DEMO_FULLSCREEN").is_some() {
            app.set_fullscreen(true);
        }
        match std::env::var("KLIP_DEMO_EXPORT").as_deref() {
            Ok("open") => app.export.open = true,
            Ok("run") => {
                app.export.open = true;
                app.export.out = std::env::temp_dir().join("klip_demo_export.mp4");
                crate::ui::export_dialog::start(app, ctx);
            }
            _ => {}
        }
        match std::env::var("KLIP_DEMO_VTAB").as_deref() {
            Ok("speed") => app.video_tab = crate::app::VideoTab::Speed,
            Ok("audio") => app.video_tab = crate::app::VideoTab::Audio,
            _ => {}
        }
        if std::env::var_os("KLIP_DEMO_PLAY").is_some() {
            app.toggle_play();
        }
        app.left_tab = match std::env::var("KLIP_DEMO_TAB").as_deref() {
            Ok("audio") => LeftTab::Audio,
            Ok("text") => LeftTab::Text,
            Ok("captions") => LeftTab::Captions,
            Ok("transitions") => LeftTab::Transitions,
            Ok("filters") => LeftTab::Filters,
            _ => LeftTab::Media,
        };
    }
    if app.dev.applied && app.screen == Screen::Editor && std::env::var_os("KLIP_DEMO_HOME").is_some() && !app.dev.home_applied {
        app.go_home();
    }
    if app.dev.active() && app.screen == Screen::Home && !app.dev.home_applied {
        app.dev.home_applied = true;
        home_hooks(app, ctx);
    }
    let Some(path) = app.dev.shot.clone() else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    if !app.dev.requested && ctx.input(|i| i.time) > app.dev.delay {
        app.dev.requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    let img = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = img {
        let [w, h] = img.size;
        let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        if let Ok(mut child) = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-i", "-"])
            .arg(&path)
            .stdin(Stdio::piped())
            .spawn()
        {
            let _ = child.stdin.take().unwrap().write_all(&raw);
            let _ = child.wait();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn home_hooks(app: &mut KlipApp, ctx: &egui::Context) {
    let first = app.home.projects.first().cloned();
    if let Ok(name) = std::env::var("KLIP_DEMO_DELETE") {
        if let Some(m) = app.home.projects.iter().find(|m| m.name == name).cloned() {
            if app.store.delete(&m.id).is_ok() {
                app.home.undo_delete = Some((m, ctx.input(|i| i.time) + 60.0));
                app.refresh_home();
            }
        }
    }
    match std::env::var("KLIP_DEMO_DIALOG").as_deref() {
        Ok("new") => {
            app.home.new_project = Some(NewProject { name: app.store.unique_name("Untitled reel"), aspect: crate::model::Aspect::Reel, media: vec![] })
        }
        Ok("rename") => app.home.rename = first.as_ref().map(|m| (m.id.clone(), m.name.clone())),
        _ => {}
    }
    if std::env::var_os("KLIP_DEMO_MENU").is_some() {
        app.home.menu = first.map(|m| m.id);
    }
    if let Ok(name) = std::env::var("KLIP_DEMO_OPEN") {
        if let Some(m) = app.home.projects.iter().find(|m| m.name == name).cloned() {
            app.open_project(&m.id);
        }
    }
    if std::env::var_os("KLIP_DEMO_TRASH").is_some() {
        app.home.show_trash = true;
    }
}
