//! Developer hooks, all driven by environment variables (no effect otherwise):
//!
//! - `KLIP_DEMO=1`              imported media is auto-added to the timeline
//! - `KLIP_DEMO_SEEK=1.5`       move the playhead once imports finish
//! - `KLIP_DEMO_SELECT=text`    select the first video|text|audio clip
//! - `KLIP_DEMO_TAB=text`       open a library tab (media|audio|text|captions)
//! - `KLIP_DEMO_PLAY=1`        start playback once imports finish
//! - `KLIP_DEMO_EXPORT=open|run` open the export dialog (and start rendering)
//! - `KLIP_DEMO_FULLSCREEN=1` open the full-screen preview
//! - `KLIP_MUTE=1`             no preview audio
//! - `KLIP_SCREENSHOT=out.png`  save a screenshot after `KLIP_SCREENSHOT_DELAY` s, then quit

use crate::app::{KlipApp, LeftTab, Selection};
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
}

impl Dev {
    pub fn from_env() -> Self {
        Self {
            demo: std::env::var_os("KLIP_DEMO").is_some(),
            shot: std::env::var_os("KLIP_SCREENSHOT").map(PathBuf::from),
            delay: std::env::var("KLIP_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0),
            requested: false,
            applied: false,
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
        match std::env::var("KLIP_DEMO_EXPORT").as_deref() {
            Ok("open") => app.export.open = true,
            Ok("run") => {
                app.export.open = true;
                app.export.out = std::env::temp_dir().join("klip_demo_export.mp4");
                crate::ui::export_dialog::start(app, ctx);
            }
            _ => {}
        }
        if std::env::var_os("KLIP_DEMO_FULLSCREEN").is_some() {
            app.set_fullscreen(true);
        }
        if std::env::var_os("KLIP_DEMO_PLAY").is_some() {
            app.toggle_play();
        }
        app.left_tab = match std::env::var("KLIP_DEMO_TAB").as_deref() {
            Ok("audio") => LeftTab::Audio,
            Ok("text") => LeftTab::Text,
            Ok("captions") => LeftTab::Captions,
            _ => LeftTab::Media,
        };
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
