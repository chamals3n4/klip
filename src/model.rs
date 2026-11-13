//! Project data model. Everything here is plain data (serde-serialisable) so
//! undo/redo is just cloning the project and saving is just writing JSON.

use crate::fx::{Adjust, FilterPreset, Mask, Transition};
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
    /// Base size on the canvas (cover / contain) and background.
    pub fit: FitMode,
    pub volume: f32,
    /// Free transform on top of `fit`: uniform scale…
    #[serde(default = "one")]
    pub scale: f32,
    /// …centre offset as a fraction of canvas width / height…
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// …and rotation in degrees (clockwise).
    #[serde(default)]
    pub rotation: f32,
    /// Fade from / to black, seconds.
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default)]
    pub fade_out: f32,
    #[serde(default)]
    pub filter: FilterPreset,
    #[serde(default = "one")]
    pub filter_amount: f32,
    #[serde(default)]
    pub adjust: Adjust,
    /// Transition into the *next* clip on the main track.
    #[serde(default)]
    pub transition: Option<Transition>,
    /// Fades on the clip's own sound only (the picture keeps playing).
    #[serde(default)]
    pub audio_fade_in: f32,
    #[serde(default)]
    pub audio_fade_out: f32,
    /// Playback speed (0.25–4×). The clip lasts `source length / speed`
    /// on the timeline.
    #[serde(default = "one")]
    pub speed: f32,
    /// Cut-out shape; moves, scales and rotates with the clip.
    #[serde(default)]
    pub mask: Mask,
}

fn one() -> f32 {
    1.0
}

impl VideoClip {
    pub fn new(id: Id, media: Id, src_in: f64, src_out: f64, fit: FitMode) -> Self {
        Self {
            id,
            media,
            src_in,
            src_out,
            fit,
            volume: 1.0,
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            filter: FilterPreset::None,
            filter_amount: 1.0,
            adjust: Adjust::default(),
            transition: None,
            audio_fade_in: 0.0,
            audio_fade_out: 0.0,
            speed: 1.0,
            mask: Mask::default(),
        }
    }

    pub const SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.25..=4.0;

    /// Effective speed, kept in range.
    pub fn speed(&self) -> f64 {
        self.speed.clamp(*Self::SPEED_RANGE.start(), *Self::SPEED_RANGE.end()) as f64
    }

    /// Seconds of source media the clip uses.
    pub fn src_len(&self) -> f64 {
        (self.src_out - self.src_in).max(0.0)
    }

    /// Seconds the clip lasts on the timeline.
    pub fn dur(&self) -> f64 {
        self.src_len() / self.speed()
    }

    /// Source time shown `local` timeline seconds into the clip.
    pub fn src_at(&self, local: f64) -> f64 {
        self.src_in + local * self.speed()
    }

    /// Change speed, keeping fades within the new length.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(*Self::SPEED_RANGE.start(), *Self::SPEED_RANGE.end());
        let half = (self.dur() / 2.0) as f32;
        self.fade_in = self.fade_in.min(half);
        self.fade_out = self.fade_out.min(half);
        self.audio_fade_in = self.audio_fade_in.min(half);
        self.audio_fade_out = self.audio_fade_out.min(half);
    }

    /// Sound fades for this clip, on the timeline.
    pub fn audio_fades(&self) -> AudioFades {
        AudioFades { fade_in: self.audio_fade_in, fade_out: self.audio_fade_out, dur: self.dur() }
    }

    pub fn grade(&self) -> Adjust {
        crate::fx::grade(self.filter, self.filter_amount, &self.adjust)
    }

    pub fn has_transform(&self) -> bool {
        (self.scale - 1.0).abs() > 1e-4 || self.x.abs() > 1e-4 || self.y.abs() > 1e-4 || self.rotation.abs() > 1e-3
    }

    /// Opacity from the clip's own fades at `local` seconds into it.
    pub fn fade_alpha(&self, local: f64) -> f32 {
        let mut a = 1.0f32;
        if self.fade_in > 0.0 {
            a = a.min((local as f32 / self.fade_in).clamp(0.0, 1.0));
        }
        if self.fade_out > 0.0 {
            a = a.min(((self.dur() - local) as f32 / self.fade_out).clamp(0.0, 1.0));
        }
        a
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
    /// Fade from / to silence, seconds.
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default)]
    pub fade_out: f32,
}

impl AudioClip {
    pub fn new(id: Id, media: Id, start: f64, src_in: f64, dur: f64, volume: f32) -> Self {
        Self { id, media, start, src_in, dur, volume, fade_in: 0.0, fade_out: 0.0 }
    }

    pub fn end(&self) -> f64 {
        self.start + self.dur
    }

    pub fn fades(&self) -> AudioFades {
        AudioFades { fade_in: self.fade_in, fade_out: self.fade_out, dur: self.dur }
    }
}

/// Fade in / out on a stretch of sound `dur` seconds long.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFades {
    pub fade_in: f32,
    pub fade_out: f32,
    pub dur: f64,
}

impl AudioFades {
    pub fn is_none(&self) -> bool {
        self.fade_in <= 0.0 && self.fade_out <= 0.0
    }

    /// Gain (0–1) at `local` seconds in.
    pub fn gain(&self, local: f64) -> f32 {
        let mut g = 1.0f32;
        if self.fade_in > 0.0 {
            g = g.min((local as f32 / self.fade_in).clamp(0.0, 1.0));
        }
        if self.fade_out > 0.0 {
            g = g.min(((self.dur - local) as f32 / self.fade_out).clamp(0.0, 1.0));
        }
        g
    }

    /// FFmpeg `volume` filter applying these fades to a stream that starts
    /// `offset` seconds into the sound. Empty when there are no fades.
    pub fn ffmpeg(&self, offset: f64) -> String {
        if self.is_none() {
            return String::new();
        }
        let mut terms = vec![];
        if self.fade_in > 0.0 {
            terms.push(format!("min(1,(t+{offset:.3})/{:.3})", self.fade_in));
        }
        if self.fade_out > 0.0 {
            terms.push(format!("min(1,max(0,{:.3}-t-{offset:.3})/{:.3})", self.dur, self.fade_out));
        }
        // Small frames so the gain (evaluated per frame) ramps smoothly.
        format!("asetnsamples=n=256:p=0,volume='{}':eval=frame", terms.join("*"))
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

    /// Effective length of the transition after clip `i` (clamped so it
    /// never eats more than half of either neighbour).
    pub fn trans_dur(&self, i: usize) -> f64 {
        match (self.video.get(i).and_then(|c| c.transition), self.video.get(i + 1)) {
            (Some(t), Some(next)) => t.dur.min(self.video[i].dur() * 0.5).min(next.dur() * 0.5).max(0.0),
            _ => 0.0,
        }
    }

    /// Length of the main track — this is the length of the exported video.
    pub fn duration(&self) -> f64 {
        match (self.video_starts().last(), self.video.last()) {
            (Some(s), Some(c)) => s + c.dur(),
            _ => 0.0,
        }
    }

    /// Timeline start time of every main-track clip. Transitions overlap
    /// neighbouring clips, pulling later clips earlier.
    pub fn video_starts(&self) -> Vec<f64> {
        let mut t = 0.0;
        (0..self.video.len())
            .map(|i| {
                let s = t;
                t += self.video[i].dur() - self.trans_dur(i);
                s
            })
            .collect()
    }

    /// Index and timeline start of the main-track clip under `t`. Inside a
    /// transition this is the outgoing clip; see [`Self::transition_at`].
    pub fn video_at(&self, t: f64) -> Option<(usize, f64)> {
        let starts = self.video_starts();
        for (i, c) in self.video.iter().enumerate() {
            if t >= starts[i] && t < starts[i] + c.dur() {
                return Some((i, starts[i]));
            }
        }
        // Parked exactly on the end: show the last frame.
        let last = self.video.len().checked_sub(1)?;
        (t >= starts[last] + self.video[last].dur() - 1e-6).then(|| (last, starts[last]))
    }

    /// If `t` falls inside a transition: (outgoing clip index, progress 0..1).
    pub fn transition_at(&self, t: f64) -> Option<(usize, f32)> {
        let starts = self.video_starts();
        for i in 0..self.video.len().saturating_sub(1) {
            let d = self.trans_dur(i);
            if d <= 0.0 {
                continue;
            }
            let (s, e) = (starts[i + 1], starts[i] + self.video[i].dur());
            if t >= s && t < e {
                return Some((i, ((t - s) / d) as f32));
            }
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
        right.fade_in = 0.0;
        let cut = self.video[i].src_at(local);
        self.video[i].src_out = cut;
        // The left half now ends on a hard cut; the right half keeps the
        // original outgoing transition and fade-out.
        self.video[i].transition = None;
        self.video[i].fade_out = 0.0;
        self.video[i].audio_fade_out = 0.0;
        right.audio_fade_in = 0.0;
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
        right.fade_in = 0.0;
        right.id = self.new_id();
        self.audio[i].dur = local;
        self.audio[i].fade_out = 0.0;
        let nid = right.id;
        self.audio.push(right);
        Some(nid)
    }
}

/// Size (in canvas units) of a source frame before the user's scale: it
/// covers the canvas for [`FitMode::Fill`], fits inside it otherwise.
pub fn base_size(fit: FitMode, src_w: f32, src_h: f32, cw: f32, ch: f32) -> (f32, f32) {
    let sa = src_w.max(1.0) / src_h.max(1.0);
    let ca = cw / ch;
    let cover = fit == FitMode::Fill;
    if (sa > ca) ^ cover { (cw, cw / sa) } else { (ch * sa, ch) }
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
        p.video.push(VideoClip::new(99, 1, 1.0, 5.0, FitMode::Fill));
        p.split_video(2.0).unwrap();
        assert_eq!(p.video.len(), 2);
        assert_eq!(p.video[0].src_out, 3.0);
        assert_eq!(p.video[1].src_in, 3.0);
        assert!((p.duration() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn transitions_overlap_clips() {
        use crate::fx::{Transition, TransitionKind};
        let mut p = Project::default();
        p.video.push(VideoClip::new(1, 1, 0.0, 4.0, FitMode::Fill));
        p.video.push(VideoClip::new(2, 1, 0.0, 4.0, FitMode::Fill));
        p.video[0].transition = Some(Transition { kind: TransitionKind::Mix, dur: 1.0 });
        assert_eq!(p.video_starts(), vec![0.0, 3.0]);
        assert!((p.duration() - 7.0).abs() < 1e-9);
        let (i, prog) = p.transition_at(3.5).unwrap();
        assert_eq!(i, 0);
        assert!((prog - 0.5).abs() < 1e-6);
        // Transition can't exceed half of a neighbour.
        p.video[0].transition = Some(Transition { kind: TransitionKind::Mix, dur: 10.0 });
        assert!((p.trans_dur(0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn timecode_format() {
        assert_eq!(timecode(61.5), "01:01.15");
    }

    #[test]
    fn speed_changes_timing() {
        let mut c = VideoClip::new(1, 1, 2.0, 6.0, FitMode::Fill);
        c.fade_out = 3.0;
        c.set_speed(2.0);
        assert_eq!(c.dur(), 2.0);
        assert_eq!(c.src_at(1.0), 4.0);
        assert_eq!(c.fade_out, 1.0, "fades shrink with the clip");
        c.set_speed(10.0);
        assert_eq!(c.speed, 4.0, "clamped");

        // Splitting a 2× clip at 1 s of timeline cuts the source at 4 s.
        let mut p = Project::default();
        let mut c = VideoClip::new(1, 1, 2.0, 6.0, FitMode::Fill);
        c.set_speed(2.0);
        p.video.push(c);
        p.next_id = 2;
        p.split_video(1.0).unwrap();
        assert_eq!((p.video[0].src_out, p.video[1].src_in), (4.0, 4.0));
        assert_eq!((p.video[0].dur(), p.video[1].dur()), (1.0, 1.0));
        assert_eq!(p.video[1].speed, 2.0);
    }

    #[test]
    fn audio_fades() {
        let f = AudioFades { fade_in: 1.0, fade_out: 2.0, dur: 10.0 };
        assert_eq!(f.gain(0.0), 0.0);
        assert_eq!(f.gain(0.5), 0.5);
        assert_eq!(f.gain(5.0), 1.0);
        assert_eq!(f.gain(9.0), 0.5);
        assert_eq!(f.gain(10.0), 0.0);
        assert_eq!(AudioFades::default().ffmpeg(0.0), "");

        // Splitting keeps the fade-in on the left and the fade-out on the right.
        let mut p = Project::default();
        let mut a = AudioClip::new(1, 1, 0.0, 0.0, 10.0, 1.0);
        a.fade_in = 1.0;
        a.fade_out = 2.0;
        p.audio.push(a);
        p.next_id = 2;
        let right = p.split_audio(1, 4.0).unwrap();
        let (l, r) = (&p.audio[0], p.audio.iter().find(|c| c.id == right).unwrap());
        assert_eq!((l.fade_in, l.fade_out), (1.0, 0.0));
        assert_eq!((r.fade_in, r.fade_out), (0.0, 2.0));
    }
}
