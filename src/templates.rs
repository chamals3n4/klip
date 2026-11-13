//! Templates: one-click looks for the clips already on the timeline.
//!
//! A template restyles the main-track clips, replaces the project's titles
//! with its own placeholder text, and sets up the music for a clean start
//! and finish. Captions are left alone, and the whole change is one undo step.

use crate::fx::{Adjust, FilterPreset, Mask, MaskShape};
use crate::model::{Aspect, FitMode, Project, TextClip, TextStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Template {
    /// The monochrome motivational edit: black canvas, wide clips in rounded
    /// square cards, tall clips full-screen, a rose-tinted black & white look
    /// and one or two words of text at a time.
    Motivation,
}

/// Square card size as a share of the canvas width.
pub const CARD: f32 = 0.91;
/// Card corner rounding (share of the card's half-width).
pub const CARD_ROUND: f32 = 0.1;

/// Placeholder script, shown a word or two at a time.
const SCRIPT: [&str; 14] = [
    "this is",
    "your story",
    "the frustration",
    "is real",
    "but",
    "you keep going",
    "MAYBE",
    "this is",
    "how it starts",
    "nobody",
    "sees the work",
    "keep",
    "showing up",
    "every day",
];

impl Template {
    pub const ALL: [Template; 1] = [Template::Motivation];

    pub fn name(self) -> &'static str {
        match self {
            Template::Motivation => "Motivation",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Template::Motivation => "Rounded clip cards on black, a rose-tinted mono look and a word or two at a time. Made for a voiceover or music.",
        }
    }

    /// The first few words, for previews.
    pub fn sample_text(self) -> &'static str {
        SCRIPT[0]
    }

    /// Restyle `p` with this template. Needs at least one clip.
    pub fn apply(self, p: &mut Project) -> Result<(), &'static str> {
        if p.video.is_empty() {
            return Err("Add your clips to the timeline first");
        }
        p.aspect = Aspect::Reel;
        let (cw, ch) = p.aspect.canvas();
        for i in 0..p.video.len() {
            let (w, h) = p.media(p.video[i].media).map(|m| (m.width as f32, m.height as f32)).unwrap_or((cw as f32, ch as f32));
            let c = &mut p.video[i];
            c.fit = FitMode::Fill;
            c.x = 0.0;
            c.y = 0.0;
            c.rotation = 0.0;
            c.filter = FilterPreset::Rose;
            c.filter_amount = 1.0;
            c.adjust = Adjust::default();
            c.fade_in = 0.0;
            c.fade_out = 0.0;
            c.transition = None;
            if w / h.max(1.0) >= 0.9 {
                // Wide or square footage: a rounded square card on black.
                let (bw, bh) = crate::model::base_size(FitMode::Fill, w, h, cw as f32, ch as f32);
                c.scale = (cw as f32 * CARD / bw.min(bh)).clamp(0.05, 4.0);
                c.mask = Mask { shape: MaskShape::Rect, size: 1.0, feather: 0.0, round: CARD_ROUND, invert: false, ratio: 1.0 };
            } else {
                // Tall footage fills the screen.
                c.scale = 1.0;
                c.mask = Mask::default();
            }
        }
        // Hard cuts, with a soft open and close.
        if let Some(c) = p.video.first_mut() {
            c.fade_in = 0.3_f32.min((c.dur() / 2.0) as f32);
        }
        if let Some(c) = p.video.last_mut() {
            c.fade_out = 0.6_f32.min((c.dur() / 2.0) as f32);
        }

        // Text: replace the old titles (keep captions) with a word or two at
        // a time, centred on the clip. Long clips get a few changes.
        p.texts.retain(|t| t.is_caption);
        let starts = p.video_starts();
        let mut line = 0;
        for i in 0..p.video.len() {
            let dur = p.video[i].dur();
            let parts = (dur / 1.2).ceil().clamp(1.0, 3.0) as usize;
            let each = dur / parts as f64;
            for k in 0..parts {
                let id = p.new_id();
                p.texts.push(TextClip {
                    id,
                    text: SCRIPT[line % SCRIPT.len()].to_string(),
                    start: starts[i] + each * k as f64,
                    dur: each,
                    x: 0.5,
                    y: 0.5,
                    size: 64.0,
                    color: [255, 255, 255],
                    style: TextStyle::Shadow,
                    is_caption: false,
                });
                line += 1;
            }
        }

        // Music: settle the volume and fade it in and out.
        for a in p.audio.iter_mut() {
            a.volume = 0.8;
            let half = (a.dur / 2.0) as f32;
            a.fade_in = 1.0_f32.min(half);
            a.fade_out = 2.0_f32.min(half);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioClip, MediaInfo, MediaKind, VideoClip};

    fn project() -> Project {
        let mut p = Project { aspect: Aspect::Landscape, ..Default::default() };
        for (i, (w, h)) in [(1920, 1080), (1080, 1920)].into_iter().enumerate() {
            let mid = p.new_id();
            p.media.push(MediaInfo { id: mid, path: format!("/v{i}.mp4").into(), name: String::new(), kind: MediaKind::Video, duration: 10.0, width: w, height: h, has_audio: false });
            let id = p.new_id();
            p.video.push(VideoClip::new(id, mid, 0.0, 3.0, FitMode::Fit));
        }
        let id = p.new_id();
        p.audio.push(AudioClip::new(id, 99, 0.0, 0.0, 12.0, 1.0));
        let id = p.new_id();
        p.texts.push(TextClip { id, text: "old title".into(), start: 0.0, dur: 1.0, x: 0.5, y: 0.5, size: 40.0, color: [255; 3], style: TextStyle::Plain, is_caption: false });
        let id = p.new_id();
        p.texts.push(TextClip { id, text: "a caption".into(), start: 0.0, dur: 1.0, x: 0.5, y: 0.8, size: 40.0, color: [255; 3], style: TextStyle::Plain, is_caption: true });
        p
    }

    #[test]
    fn motivation_layout() {
        let mut p = project();
        Template::Motivation.apply(&mut p).unwrap();
        assert_eq!(p.aspect, Aspect::Reel);

        // Wide clip: a square card 91% of the canvas wide.
        let wide = &p.video[0];
        assert_eq!(wide.mask.shape, MaskShape::Rect);
        assert_eq!(wide.mask.ratio, 1.0);
        let (bw, bh) = crate::model::base_size(FitMode::Fill, 1920.0, 1080.0, 1080.0, 1920.0);
        assert!((bw.min(bh) * wide.scale - 1080.0 * CARD).abs() < 1.0);
        // Tall clip: full screen, no mask.
        assert!(p.video[1].mask.is_none());
        assert_eq!(p.video[1].scale, 1.0);
        assert!(p.video.iter().all(|c| c.filter == FilterPreset::Rose));

        // Text: old titles gone, captions kept, words cover the whole video.
        assert!(!p.texts.iter().any(|t| t.text == "old title"));
        assert!(p.texts.iter().any(|t| t.is_caption));
        let words: Vec<_> = p.texts.iter().filter(|t| !t.is_caption).collect();
        assert_eq!(words.len(), 6, "3 s clips get three changes each");
        let covered: f64 = words.iter().map(|t| t.dur).sum();
        assert!((covered - p.duration()).abs() < 1e-6);
        assert!(p.audio[0].fade_out > 0.0);

        assert!(Template::Motivation.apply(&mut Project::default()).is_err());
    }
}
