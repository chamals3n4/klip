//! Looks and transitions.
//!
//! Colour grading is defined once as simple maths and implemented twice:
//! on the CPU for the live preview and as FFmpeg filters for export, so the
//! two always agree.
//!
//!   1. saturation   — mix each channel towards Rec.601 luma
//!   2. per channel  — contrast around mid-grey, brightness offset,
//!                     temperature gain, then "fade" (lift blacks / dim whites)

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjust {
    /// -0.5 ..= 0.5 (added, in 0..1 units)
    pub brightness: f32,
    /// 0.5 ..= 1.5 (multiplier around mid-grey)
    pub contrast: f32,
    /// 0 ..= 2 (0 = black & white)
    pub saturation: f32,
    /// -1 (cool) ..= 1 (warm)
    pub temperature: f32,
    /// 0 ..= 1 (washed-out film look)
    pub fade: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 1.0, saturation: 1.0, temperature: 0.0, fade: 0.0 }
    }
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Blend from neutral towards `self` by `t` (0..1).
    pub fn scaled(&self, t: f32) -> Self {
        let n = Self::default();
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self {
            brightness: l(n.brightness, self.brightness),
            contrast: l(n.contrast, self.contrast),
            saturation: l(n.saturation, self.saturation),
            temperature: l(n.temperature, self.temperature),
            fade: l(n.fade, self.fade),
        }
    }

    /// Stack a user adjustment on top of a preset look.
    pub fn then(&self, user: &Adjust) -> Self {
        Self {
            brightness: (self.brightness + user.brightness).clamp(-0.6, 0.6),
            contrast: (self.contrast * user.contrast).clamp(0.3, 2.0),
            saturation: (self.saturation * user.saturation).clamp(0.0, 2.5),
            temperature: (self.temperature + user.temperature).clamp(-1.5, 1.5),
            fade: (self.fade + user.fade).clamp(0.0, 1.0),
        }
    }

    fn gains(&self) -> [f32; 3] {
        let t = self.temperature;
        [1.0 + 0.12 * t, 1.0 + 0.02 * t, 1.0 - 0.12 * t]
    }

    fn fade_k(&self) -> f32 {
        self.fade * 0.3
    }

    /// Apply to an RGBA buffer in place (preview path).
    pub fn apply(&self, rgba: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        // Per-channel lookup table for step 2.
        let gains = self.gains();
        let k = self.fade_k();
        let mut lut = [[0u8; 256]; 3];
        for (c, table) in lut.iter_mut().enumerate() {
            for (v, out) in table.iter_mut().enumerate() {
                let x = ((v as f32 / 255.0 - 0.5) * self.contrast + 0.5 + self.brightness) * gains[c];
                let x = x * (1.0 - k) + k * 0.5;
                *out = (x * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        let s = self.saturation;
        let sat = (s - 1.0).abs() > 1e-4;
        for px in rgba.chunks_exact_mut(4) {
            let (mut r, mut g, mut b) = (px[0] as f32, px[1] as f32, px[2] as f32);
            if sat {
                let l = 0.299 * r + 0.587 * g + 0.114 * b;
                r = (l + s * (r - l)).clamp(0.0, 255.0);
                g = (l + s * (g - l)).clamp(0.0, 255.0);
                b = (l + s * (b - l)).clamp(0.0, 255.0);
            }
            px[0] = lut[0][r.round() as usize];
            px[1] = lut[1][g.round() as usize];
            px[2] = lut[2][b.round() as usize];
        }
    }

    /// The same grade as an FFmpeg filter chain (export path). Empty if neutral.
    pub fn ffmpeg(&self) -> String {
        if self.is_identity() {
            return String::new();
        }
        let s = self.saturation;
        let (a, b, c) = (0.299 * (1.0 - s), 0.587 * (1.0 - s), 0.114 * (1.0 - s));
        let mixer = format!(
            "colorchannelmixer=rr={:.5}:rg={b:.5}:rb={c:.5}:gr={a:.5}:gg={:.5}:gb={c:.5}:br={a:.5}:bg={b:.5}:bb={:.5}",
            a + s,
            b + s,
            c + s
        );
        let g = self.gains();
        let k = self.fade_k();
        let expr = |gain: f32| {
            format!(
                "'clip((((val/255-0.5)*{:.5}+0.5+{:.5})*{:.5}*{:.5}+{:.5})*255,0,255)'",
                self.contrast,
                self.brightness,
                gain,
                1.0 - k,
                k * 0.5
            )
        };
        format!("format=rgb24,{mixer},lutrgb=r={}:g={}:b={}", expr(g[0]), expr(g[1]), expr(g[2]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FilterPreset {
    #[default]
    None,
    Vivid,
    Warm,
    Cool,
    Film,
    Faded,
    Mono,
    Noir,
    Dream,
}

impl FilterPreset {
    pub const ALL: [FilterPreset; 9] = [
        FilterPreset::None,
        FilterPreset::Vivid,
        FilterPreset::Warm,
        FilterPreset::Cool,
        FilterPreset::Film,
        FilterPreset::Faded,
        FilterPreset::Mono,
        FilterPreset::Noir,
        FilterPreset::Dream,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterPreset::None => "None",
            FilterPreset::Vivid => "Vivid",
            FilterPreset::Warm => "Warm",
            FilterPreset::Cool => "Cool",
            FilterPreset::Film => "Film",
            FilterPreset::Faded => "Faded",
            FilterPreset::Mono => "Mono",
            FilterPreset::Noir => "Noir",
            FilterPreset::Dream => "Dream",
        }
    }

    pub fn look(self) -> Adjust {
        let a = Adjust::default();
        match self {
            FilterPreset::None => a,
            FilterPreset::Vivid => Adjust { contrast: 1.12, saturation: 1.45, brightness: 0.01, ..a },
            FilterPreset::Warm => Adjust { temperature: 0.8, saturation: 1.1, brightness: 0.02, ..a },
            FilterPreset::Cool => Adjust { temperature: -0.8, contrast: 1.05, ..a },
            FilterPreset::Film => Adjust { contrast: 0.92, saturation: 0.85, temperature: 0.35, fade: 0.45, ..a },
            FilterPreset::Faded => Adjust { contrast: 0.85, saturation: 0.7, fade: 0.7, brightness: 0.03, ..a },
            FilterPreset::Mono => Adjust { saturation: 0.0, contrast: 1.05, ..a },
            FilterPreset::Noir => Adjust { saturation: 0.0, contrast: 1.4, brightness: -0.04, ..a },
            FilterPreset::Dream => Adjust { brightness: 0.06, contrast: 0.88, saturation: 1.15, temperature: 0.2, fade: 0.3 },
        }
    }
}

/// Final grade for a clip: preset at `amount`, then the user's adjustments.
/// FFmpeg audio filter that changes tempo by `speed` without changing pitch.
/// `atempo` only takes 0.5–2, so larger changes are chained. Empty at 1×.
pub fn atempo(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-3 {
        return String::new();
    }
    let mut parts = vec![];
    let mut s = speed;
    while s > 2.0 {
        parts.push("atempo=2".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    parts.push(format!("atempo={s:.4}"));
    parts.join(",")
}

pub fn grade(preset: FilterPreset, amount: f32, user: &Adjust) -> Adjust {
    preset.look().scaled(amount).then(user)
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransitionKind {
    Mix,
    FadeBlack,
    FadeWhite,
    SlideLeft,
    SlideRight,
    SlideUp,
    Wipe,
    ZoomIn,
    Blur,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 9] = [
        TransitionKind::Mix,
        TransitionKind::FadeBlack,
        TransitionKind::FadeWhite,
        TransitionKind::SlideLeft,
        TransitionKind::SlideRight,
        TransitionKind::SlideUp,
        TransitionKind::Wipe,
        TransitionKind::ZoomIn,
        TransitionKind::Blur,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Mix => "Mix",
            TransitionKind::FadeBlack => "Black fade",
            TransitionKind::FadeWhite => "White flash",
            TransitionKind::SlideLeft => "Slide left",
            TransitionKind::SlideRight => "Slide right",
            TransitionKind::SlideUp => "Slide up",
            TransitionKind::Wipe => "Wipe",
            TransitionKind::ZoomIn => "Zoom in",
            TransitionKind::Blur => "Blur",
        }
    }

    /// FFmpeg `xfade` transition name.
    pub fn xfade(self) -> &'static str {
        match self {
            TransitionKind::Mix => "fade",
            TransitionKind::FadeBlack => "fadeblack",
            TransitionKind::FadeWhite => "fadewhite",
            TransitionKind::SlideLeft => "slideleft",
            TransitionKind::SlideRight => "slideright",
            TransitionKind::SlideUp => "slideup",
            TransitionKind::Wipe => "wipeleft",
            TransitionKind::ZoomIn => "zoomin",
            TransitionKind::Blur => "hblur",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub dur: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_noop() {
        let mut px = vec![10, 128, 250, 255];
        Adjust::default().apply(&mut px);
        assert_eq!(px, vec![10, 128, 250, 255]);
        assert!(Adjust::default().ffmpeg().is_empty());
    }

    #[test]
    fn mono_removes_colour() {
        let mut px = vec![200, 50, 50, 255];
        FilterPreset::Mono.look().apply(&mut px);
        assert!((px[0] as i32 - px[1] as i32).abs() <= 2 && (px[1] as i32 - px[2] as i32).abs() <= 2);
    }

    #[test]
    fn atempo_chains_outside_its_range() {
        assert_eq!(super::atempo(1.0), "");
        assert_eq!(super::atempo(1.5), "atempo=1.5000");
        assert_eq!(super::atempo(4.0), "atempo=2,atempo=2.0000");
        assert_eq!(super::atempo(0.25), "atempo=0.5,atempo=0.5000");
    }
}
