//! Everything that talks to ffmpeg/ffprobe for *reading* media:
//! probing, single-frame grabs, streaming decode for playback, filmstrip
//! thumbnails and audio waveforms.
//!
//! We drive the ffmpeg CLI instead of linking libav*: no dev headers needed,
//! no ABI churn, and the exact same binary does the export, so the preview
//! and the final render decode identically.

use crate::model::{FPS, MediaInfo, MediaKind};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};

pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "heic", "avif"];
const AUDIO_EXT: &[&str] = &["mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma"];

pub fn ffmpeg() -> Command {
    let mut c = Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c.stdin(Stdio::null()).stderr(Stdio::null());
    c
}

/// Probe a file with ffprobe. `id` is assigned by the caller later.
pub fn probe(path: &Path) -> Result<MediaInfo, String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe not found: {e}"))?;
    if !out.status.success() {
        return Err(format!("Unsupported file: {}", path.display()));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe output: {e}"))?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let video = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    });
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let duration = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let kind = if IMAGE_EXT.contains(&ext.as_str()) {
        MediaKind::Image
    } else if video.is_none() || AUDIO_EXT.contains(&ext.as_str()) {
        if !has_audio {
            return Err(format!("No audio or video in {}", path.display()));
        }
        MediaKind::Audio
    } else {
        MediaKind::Video
    };

    let (mut width, mut height) = video
        .map(|s| {
            (
                s["width"].as_u64().unwrap_or(0) as u32,
                s["height"].as_u64().unwrap_or(0) as u32,
            )
        })
        .unwrap_or((0, 0));
    // Phone footage is usually stored landscape with a rotation flag.
    let rotation = video
        .and_then(|s| {
            s["side_data_list"]
                .as_array()
                .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
                .or_else(|| s["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
        })
        .unwrap_or(0.0);
    if (rotation.abs() - 90.0).abs() < 1.0 || (rotation.abs() - 270.0).abs() < 1.0 {
        std::mem::swap(&mut width, &mut height);
    }
    if kind != MediaKind::Audio && (width == 0 || height == 0) {
        return Err(format!("Could not read video size of {}", path.display()));
    }
    if kind == MediaKind::Video && duration <= 0.0 {
        return Err(format!("Could not read duration of {}", path.display()));
    }

    Ok(MediaInfo {
        id: 0,
        path: path.to_path_buf(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        kind,
        duration: if kind == MediaKind::Image { 0.0 } else { duration },
        width,
        height,
        has_audio: has_audio && kind != MediaKind::Image,
    })
}

/// Size that fits `info` inside a `max_w`×`max_h` box, keeping aspect, even dims.
pub fn fit_size(info: &MediaInfo, max_w: u32, max_h: u32) -> (usize, usize) {
    let (w, h) = (info.width.max(2) as f64, info.height.max(2) as f64);
    let s = (max_w as f64 / w).min(max_h as f64 / h).min(1.0);
    let even = |v: f64| ((v / 2.0).round() as usize * 2).max(2);
    (even(w * s), even(h * s))
}

/// Decode exactly one frame at `t` seconds.
pub fn grab_frame(path: &Path, t: f64, w: usize, h: usize) -> Option<Frame> {
    let mut cmd = ffmpeg();
    if t > 0.0 {
        cmd.args(["-ss", &format!("{t:.3}")]);
    }
    let out = cmd
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf", &format!("scale={w}:{h}:flags=bilinear")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stdout(Stdio::piped())
        .output()
        .ok()?;
    if out.stdout.len() < w * h * 4 {
        // Seeking past the last frame yields nothing; retry slightly earlier.
        if t > 0.2 {
            return grab_frame(path, t - 0.2, w, h);
        }
        return None;
    }
    let mut rgba = out.stdout;
    rgba.truncate(w * h * 4);
    Some(Frame { w, h, rgba })
}

/// A handful of evenly spaced thumbnails for the timeline filmstrip.
pub fn filmstrip(info: &MediaInfo, count: usize, thumb_h: u32) -> Vec<(f64, Frame)> {
    let (w, h) = fit_size(info, thumb_h * 4, thumb_h);
    if info.kind == MediaKind::Image {
        return grab_frame(&info.path, 0.0, w, h)
            .map(|f| vec![(0.0, f)])
            .unwrap_or_default();
    }
    (0..count)
        .filter_map(|i| {
            let t = info.duration * (i as f64 + 0.5) / count as f64;
            grab_frame(&info.path, t, w, h).map(|f| (t, f))
        })
        .collect()
}

/// Peak amplitude per 1/`rate` second, normalised 0..1.
pub const WAVE_RATE: f64 = 25.0;
pub fn waveform(path: &Path) -> Vec<f32> {
    let Ok(mut child) = ffmpeg()
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "4000", "-f", "s16le", "pipe:1"])
        .stdout(Stdio::piped())
        .spawn()
    else {
        return vec![];
    };
    let mut raw = vec![];
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut raw);
    }
    let _ = child.wait();
    let per = (4000.0 / WAVE_RATE) as usize;
    let peaks: Vec<f32> = raw
        .chunks(per * 2)
        .map(|chunk| {
            chunk
                .chunks_exact(2)
                .map(|b| (i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).abs())
                .fold(0.0f32, f32::max)
        })
        .collect();
    let max = peaks.iter().cloned().fold(0.01, f32::max);
    peaks.into_iter().map(|p| (p / max).sqrt()).collect()
}

// ---------------------------------------------------------------------------
// Scrub grabber: one background worker that always serves the *latest*
// request, so dragging the playhead never queues up stale decodes.
// ---------------------------------------------------------------------------

pub struct GrabRequest {
    pub key: (u64, i64),
    pub path: PathBuf,
    pub t: f64,
    pub w: usize,
    pub h: usize,
}

pub struct Grabber {
    slot: Arc<(Mutex<Option<GrabRequest>>, Condvar)>,
    pub rx: Receiver<((u64, i64), Frame)>,
}

impl Grabber {
    pub fn new(ctx: egui::Context) -> Self {
        let slot: Arc<(Mutex<Option<GrabRequest>>, Condvar)> = Arc::default();
        let (tx, rx) = bounded(4);
        let s = slot.clone();
        std::thread::Builder::new()
            .name("klip-grab".into())
            .spawn(move || {
                loop {
                    let req = {
                        let (lock, cv) = &*s;
                        let mut g = lock.lock().unwrap();
                        while g.is_none() {
                            g = cv.wait(g).unwrap();
                        }
                        g.take().unwrap()
                    };
                    if let Some(f) = grab_frame(&req.path, req.t, req.w, req.h) {
                        if tx.send((req.key, f)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                }
            })
            .expect("spawn grabber");
        Self { slot, rx }
    }

    pub fn request(&self, req: GrabRequest) {
        let (lock, cv) = &*self.slot;
        *lock.lock().unwrap() = Some(req);
        cv.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder for playback.
// ---------------------------------------------------------------------------

pub struct StreamDecoder {
    pub clip_id: u64,
    rx: Receiver<(f64, Frame)>,
    pending: Option<(f64, Frame)>,
    child: Arc<Mutex<Option<Child>>>,
}

impl StreamDecoder {
    /// Decode `path` from `src_t` for `dur` seconds; frames are stamped with
    /// timeline time starting at `timeline_t`.
    pub fn new(
        ctx: egui::Context,
        clip_id: u64,
        path: &Path,
        src_t: f64,
        dur: f64,
        timeline_t: f64,
        w: usize,
        h: usize,
    ) -> Option<Self> {
        let mut child = ffmpeg()
            .args(["-ss", &format!("{:.3}", src_t.max(0.0))])
            .arg("-i")
            .arg(path)
            .args(["-t", &format!("{:.3}", dur.max(0.05))])
            .args(["-an", "-vf", &format!("fps={FPS},scale={w}:{h}:flags=bilinear")])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx): (Sender<(f64, Frame)>, _) = bounded(8);
        std::thread::Builder::new()
            .name("klip-decode".into())
            .spawn(move || {
                let size = w * h * 4;
                let mut n = 0u64;
                loop {
                    let mut buf = vec![0u8; size];
                    if stdout.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let t = timeline_t + n as f64 / FPS;
                    n += 1;
                    if tx.send((t, Frame { w, h, rgba: buf })).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { clip_id, rx, pending: None, child })
    }

    /// Latest frame whose timestamp is `<= t`, dropping late ones.
    pub fn frame_for(&mut self, t: f64) -> Option<Frame> {
        let mut best = None;
        loop {
            let next = self.pending.take().or_else(|| self.rx.try_recv().ok());
            match next {
                Some((ft, f)) if ft <= t + 0.5 / FPS => best = Some(f),
                Some(other) => {
                    self.pending = Some(other);
                    break;
                }
                None => break,
            }
        }
        best
    }
}

impl Drop for StreamDecoder {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

/// Tiny box-filtered copy of a frame. Rendered with linear filtering and
/// stretched, it reads as a soft blur — a cheap stand-in for a real blur.
pub fn tiny_blur(f: &Frame, tw: usize) -> Frame {
    let th = ((f.h * tw) / f.w.max(1)).max(1);
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        for tx in 0..tw {
            let (x0, x1) = (tx * f.w / tw, ((tx + 1) * f.w / tw).max(tx * f.w / tw + 1));
            let (y0, y1) = (ty * f.h / th, ((ty + 1) * f.h / th).max(ty * f.h / th + 1));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            // Sample a sparse grid — plenty for a blur and keeps it cheap.
            let step = ((x1 - x0) / 6).max(1);
            for y in (y0..y1.min(f.h)).step_by(step) {
                for x in (x0..x1.min(f.w)).step_by(step) {
                    let i = (y * f.w + x) * 4;
                    for c in 0..4 {
                        acc[c] += f.rgba[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = (ty * tw + tx) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
            out[o + 3] = 255;
        }
    }
    // A few 3×3 box passes smooth the blockiness once it's stretched up.
    for _ in 0..3 {
        let src = out.clone();
        for y in 0..th {
            for x in 0..tw {
                let mut acc = [0u32; 3];
                let mut n = 0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < tw && (sy as usize) < th {
                            let i = (sy as usize * tw + sx as usize) * 4;
                            for c in 0..3 {
                                acc[c] += src[i + c] as u32;
                            }
                            n += 1;
                        }
                    }
                }
                let o = (y * tw + x) * 4;
                for c in 0..3 {
                    out[o + c] = (acc[c] / n) as u8;
                }
            }
        }
    }
    Frame { w: tw, h: th, rgba: out }
}
