//! Project data model. Everything here is plain data (serde-serialisable) so
//! undo/redo is just cloning the project and saving is just writing JSON.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type Id = u64;

/// Preview/export frame rate for the whole project.
pub const FPS: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub id: Id,
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    /// Seconds. Images report 0.
    pub duration: f64,
    /// Display size (rotation already applied).
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

impl MediaInfo {
    pub fn aspect(&self) -> f32 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitMode {
    /// Crop to fill the whole canvas.
    Fill,
    /// Letterbox on black.
    Fit,
    /// Fit on top of a blurred, zoomed copy of itself (the classic Reels look).
    Blur,
}

impl FitMode {
    pub const ALL: [FitMode; 3] = [FitMode::Fill, FitMode::Fit, FitMode::Blur];
    pub fn label(self) -> &'static str {
        match self {
            FitMode::Fill => "Fill",
            FitMode::Fit => "Fit",
            FitMode::Blur => "Blur bg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoClip {
    pub id: Id,
    pub media: Id,
    pub src_in: f64,
    pub src_out: f64,
    pub fit: FitMode,
    pub volume: f32,
}

impl VideoClip {
    pub fn dur(&self) -> f64 {
        (self.src_out - self.src_in).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextStyle {
    Plain,
    Outline,
    Boxed,
    Shadow,
}

impl TextStyle {
    pub const ALL: [TextStyle; 4] = [
        TextStyle::Plain,
        TextStyle::Outline,
        TextStyle::Boxed,
        TextStyle::Shadow,
    ];
    pub fn label(self) -> &'static str {
        match self {
            TextStyle::Plain => "Plain",
            TextStyle::Outline => "Outline",
            TextStyle::Boxed => "Box",
            TextStyle::Shadow => "Shadow",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextClip {
    pub id: Id,
    pub text: String,
    pub start: f64,
    pub dur: f64,
    /// Centre position, normalised 0..1 over the canvas.
    pub x: f32,
    pub y: f32,
    /// Font size in canvas pixels (canvas is always 1080 wide).
    pub size: f32,
    pub color: [u8; 3],
    pub style: TextStyle,
    pub is_caption: bool,
}

impl TextClip {
    pub fn end(&self) -> f64 {
        self.start + self.dur
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: Id,
    pub media: Id,
    pub start: f64,
    pub src_in: f64,
    pub dur: f64,
    pub volume: f32,
}

impl AudioClip {
    pub fn end(&self) -> f64 {
        self.start + self.dur
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    Reel,
    Portrait,
    Square,
    Landscape,
}

impl Aspect {
    pub const ALL: [Aspect; 4] = [Aspect::Reel, Aspect::Portrait, Aspect::Square, Aspect::Landscape];
    pub fn label(self) -> &'static str {
        match self {
            Aspect::Reel => "9:16",
            Aspect::Portrait => "4:5",
            Aspect::Square => "1:1",
            Aspect::Landscape => "16:9",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Aspect::Reel => "Reels · Shorts · TikTok",
            Aspect::Portrait => "Feed post",
            Aspect::Square => "Square post",
            Aspect::Landscape => "YouTube",
        }
    }
    /// Canvas size in pixels at full (1080p) quality.
    pub fn canvas(self) -> (u32, u32) {
        match self {
            Aspect::Reel => (1080, 1920),
            Aspect::Portrait => (1080, 1350),
            Aspect::Square => (1080, 1080),
            Aspect::Landscape => (1920, 1080),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub aspect: Aspect,
    pub media: Vec<MediaInfo>,
    /// Main track: magnetic, clips play back to back.
    pub video: Vec<VideoClip>,
    pub texts: Vec<TextClip>,
    pub audio: Vec<AudioClip>,
    pub next_id: Id,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "Untitled reel".into(),
            aspect: Aspect::Reel,
            media: vec![],
            video: vec![],
            texts: vec![],
            audio: vec![],
            next_id: 1,
        }
    }
}

impl Project {
    pub fn new_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: Id) -> Option<&MediaInfo> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Length of the main track — this is the length of the exported video.
    pub fn duration(&self) -> f64 {
        self.video.iter().map(|c| c.dur()).sum()
    }

    /// Timeline start time of every main-track clip.
    pub fn video_starts(&self) -> Vec<f64> {
        let mut t = 0.0;
        self.video
            .iter()
            .map(|c| {
                let s = t;
                t += c.dur();
                s
            })
            .collect()
    }

    /// Index and timeline start of the main-track clip under `t`.
    pub fn video_at(&self, t: f64) -> Option<(usize, f64)> {
        let mut start = 0.0;
        for (i, c) in self.video.iter().enumerate() {
            let end = start + c.dur();
            if t >= start && t < end {
                return Some((i, start));
            }
            start = end;
        }
        // Parked exactly on the end: show the last frame.
        if !self.video.is_empty() && t >= start - 1e-6 {
            let last = self.video.len() - 1;
            return Some((last, start - self.video[last].dur()));
        }
        None
    }

    pub fn video_index(&self, id: Id) -> Option<usize> {
        self.video.iter().position(|c| c.id == id)
    }

    /// Split whichever clip lies under `t` on every track (CapCut-style split
    /// only touches the selected clip, so callers pass a selection filter).
    pub fn split_video(&mut self, t: f64) -> Option<Id> {
        let (i, start) = self.video_at(t)?;
        let local = t - start;
        let c = &self.video[i];
        if local < 0.05 || c.dur() - local < 0.05 {
            return None;
        }
        let mut right = c.clone();
        right.id = self.new_id();
        let cut = self.video[i].src_in + local;
        self.video[i].src_out = cut;
        right.src_in = cut;
        let id = right.id;
        self.video.insert(i + 1, right);
        Some(id)
    }

    pub fn split_text(&mut self, id: Id, t: f64) -> Option<Id> {
        let i = self.texts.iter().position(|c| c.id == id)?;
        let c = &self.texts[i];
        if t <= c.start + 0.05 || t >= c.end() - 0.05 {
            return None;
        }
        let mut right = c.clone();
        right.start = t;
        right.dur = c.end() - t;
        right.id = self.new_id();
        self.texts[i].dur = t - self.texts[i].start;
        let nid = right.id;
        self.texts.push(right);
        Some(nid)
    }

    pub fn split_audio(&mut self, id: Id, t: f64) -> Option<Id> {
        let i = self.audio.iter().position(|c| c.id == id)?;
        let c = &self.audio[i];
        if t <= c.start + 0.05 || t >= c.end() - 0.05 {
            return None;
        }
        let mut right = c.clone();
        let local = t - c.start;
        right.start = t;
        right.src_in = c.src_in + local;
        right.dur = c.dur - local;
        right.id = self.new_id();
        self.audio[i].dur = local;
        let nid = right.id;
        self.audio.push(right);
        Some(nid)
    }
}

/// `mm:ss.ff` (frames) timecode for the UI.
pub fn timecode(t: f64) -> String {
    let t = t.max(0.0);
    let total_frames = (t * FPS).round() as i64;
    let frames = total_frames % FPS as i64;
    let secs = total_frames / FPS as i64;
    format!("{:02}:{:02}.{:02}", secs / 60, secs % 60, frames)
}

/// Parse an `.srt` file into (start, end, text) cues.
pub fn parse_srt(src: &str) -> Vec<(f64, f64, String)> {
    fn ts(s: &str) -> Option<f64> {
        let s = s.trim().replace(',', ".");
        let mut parts = s.split(':');
        let h: f64 = parts.next()?.parse().ok()?;
        let m: f64 = parts.next()?.parse().ok()?;
        let sec: f64 = parts.next()?.parse().ok()?;
        Some(h * 3600.0 + m * 60.0 + sec)
    }
    let src = src.replace("\r\n", "\n");
    let mut out = vec![];
    for block in src.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let mut it = lines[ti].split("-->");
        let (Some(a), Some(b)) = (it.next().and_then(ts), it.next().and_then(ts)) else {
            continue;
        };
        let text = lines[ti + 1..]
            .iter()
            .map(|l| strip_tags(l))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() && b > a {
            out.push((a, b, text));
        }
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '<' | '{' => depth += 1,
            '>' | '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_parses() {
        let s = "1\n00:00:01,000 --> 00:00:02,500\n<i>Hello</i>\nworld\n\n2\n00:00:03,000 --> 00:00:04,000\nBye\n";
        let cues = parse_srt(s);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].2, "Hello\nworld");
        assert!((cues[0].1 - 2.5).abs() < 1e-9);
    }

    #[test]
    fn split_video_clip() {
        let mut p = Project::default();
        p.video.push(VideoClip { id: 99, media: 1, src_in: 1.0, src_out: 5.0, fit: FitMode::Fill, volume: 1.0 });
        p.split_video(2.0).unwrap();
        assert_eq!(p.video.len(), 2);
        assert_eq!(p.video[0].src_out, 3.0);
        assert_eq!(p.video[1].src_in, 3.0);
        assert!((p.duration() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn timecode_format() {
        assert_eq!(timecode(61.5), "01:01.15");
    }
}
