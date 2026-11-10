//! Export: turn the project into one ffmpeg command with a filter graph.
//!
//! Each main-track clip is graded, placed on its background with the user's
//! transform, faded, then joined to its neighbour with xfade (transition) or
//! concat (hard cut). Text is drawn on top and music mixed in.

use crate::media::ffmpeg;
use crate::model::{FitMode, MediaKind, Project, TextStyle, FPS, base_size};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    High,
    Standard,
}

#[derive(Clone)]
pub struct ExportSettings {
    pub out: PathBuf,
    /// 1.0 = 1080p, 0.6667 = 720p.
    pub scale: f32,
    pub quality: Quality,
    pub font: Option<PathBuf>,
}

#[derive(Clone, PartialEq)]
pub enum ExportStatus {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

pub struct ExportJob {
    pub progress: Arc<Mutex<(f32, ExportStatus)>>,
    child: Arc<Mutex<Option<Child>>>,
    pub out: PathBuf,
}

impl ExportJob {
    pub fn cancel(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        self.progress.lock().unwrap().1 = ExportStatus::Cancelled;
    }
    pub fn state(&self) -> (f32, ExportStatus) {
        self.progress.lock().unwrap().clone()
    }
}

fn even(v: f32) -> u32 {
    ((v / 2.0).round() as u32 * 2).max(2)
}

fn hex(c: [u8; 3]) -> String {
    format!("0x{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Quote a value for use inside an ffmpeg filter graph.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "'\\''"))
}

/// Build the full ffmpeg argument list. `tmp` receives text files for drawtext.
pub fn build_args(p: &Project, s: &ExportSettings, tmp: &Path) -> Result<Vec<String>, String> {
    if p.video.is_empty() {
        return Err("Add at least one clip to the main track before exporting.".into());
    }
    let (cw, ch) = p.aspect.canvas();
    let (w, h) = (even(cw as f32 * s.scale), even(ch as f32 * s.scale));
    let total = p.duration();
    let k = s.scale; // canvas px -> output px

    let mut args: Vec<String> = vec!["-y".into()];
    let mut fg = String::new();
    let mut input = 0usize;

    // --- main track -------------------------------------------------------
    // Each clip: [optional grade] → background (black or blurred copy) with
    // the frame scaled/rotated/offset on top → fades. Clips are then chained
    // with xfade/acrossfade at transitions and concat at hard cuts.
    for (i, c) in p.video.iter().enumerate() {
        let m = p.media(c.media).ok_or("Missing media for a clip")?;
        let dur = c.dur();
        if m.kind == MediaKind::Image {
            args.extend(["-loop", "1", "-framerate", "30"].map(String::from));
            args.extend(["-t".into(), format!("{dur:.3}")]);
        } else {
            args.extend(["-ss".into(), format!("{:.3}", c.src_in)]);
            args.extend(["-t".into(), format!("{:.3}", c.src_len())]);
        }
        args.extend(["-i".into(), m.path.to_string_lossy().into_owned()]);
        let n = input;
        input += 1;

        let grade = c.grade().ffmpeg();
        let grade = if grade.is_empty() { String::new() } else { format!(",{grade}") };
        let speed = if m.kind == MediaKind::Image { 1.0 } else { c.speed() };
        let _ = write!(fg, "[{n}:v]setpts=(PTS-STARTPTS)/{speed:.4},fps={FPS},setsar=1{grade},split[b{i}][f{i}];");

        // Background.
        if c.fit == FitMode::Blur {
            let (sw, sh) = (even(w as f32 / 4.0), even(h as f32 / 4.0));
            let br = ((w.min(h) / 24).max(4)) / 4;
            let _ = write!(
                fg,
                "[b{i}]scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},boxblur={br}:2,scale={w}:{h},eq=brightness=-0.08,format=yuv420p[bg{i}];"
            );
        } else {
            let _ = write!(fg, "[b{i}]nullsink;color=c=black:s={w}x{h}:r={FPS}:d={dur:.3},format=yuv420p[bg{i}];");
        }

        // Foreground: base size from `fit`, then the user's transform.
        let (bw, bh) = base_size(c.fit, m.width as f32, m.height as f32, w as f32, h as f32);
        let scale = c.scale.clamp(0.05, 4.0);
        let (fw, fh) = (even(bw * scale), even(bh * scale));
        let rot = if c.rotation.abs() > 1e-3 {
            let r = c.rotation.to_radians();
            let ow = even(fw as f32 * r.cos().abs() + fh as f32 * r.sin().abs());
            let oh = even(fw as f32 * r.sin().abs() + fh as f32 * r.cos().abs());
            format!(",format=rgba,rotate=a={r:.6}:ow={ow}:oh={oh}:c=none")
        } else {
            String::new()
        };
        let (dx, dy) = ((c.x * w as f32).round() as i32, (c.y * h as f32).round() as i32);
        let _ = write!(
            fg,
            "[f{i}]scale={fw}:{fh}:flags=bicubic{rot}[fg{i}];[bg{i}][fg{i}]overlay=x=(W-w)/2+({dx}):y=(H-h)/2+({dy}):eof_action=pass,format=yuv420p,setsar=1"
        );
        if c.fade_in > 0.0 {
            let _ = write!(fg, ",fade=t=in:st=0:d={:.3}", c.fade_in);
        }
        if c.fade_out > 0.0 {
            let _ = write!(fg, ",fade=t=out:st={:.3}:d={:.3}", (dur - c.fade_out as f64).max(0.0), c.fade_out);
        }
        let _ = write!(fg, ",trim=duration={dur:.3},settb=AVTB[v{i}];");

        // Audio (silence for clips without sound).
        if m.has_audio {
            let _ = write!(
                fg,
                "[{n}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,{}volume={:.3},apad,atrim=0:{dur:.3},asetpts=PTS-STARTPTS",
                match crate::fx::atempo(speed) {
                    t if t.is_empty() => t,
                    t => format!("{t},"),
                },
                c.volume
            );
        } else {
            let _ = write!(fg, "anullsrc=r=48000:cl=stereo,atrim=0:{dur:.3},aformat=sample_fmts=fltp");
        }
        if c.fade_in > 0.0 {
            let _ = write!(fg, ",afade=t=in:st=0:d={:.3}", c.fade_in);
        }
        if c.fade_out > 0.0 {
            let _ = write!(fg, ",afade=t=out:st={:.3}:d={:.3}", (dur - c.fade_out as f64).max(0.0), c.fade_out);
        }
        let _ = write!(fg, "[a{i}];");
    }

    // Join clips: xfade/acrossfade where there's a transition, concat otherwise.
    let (mut av, mut aa) = ("v0".to_string(), "a0".to_string());
    let mut acc = p.video[0].dur();
    for k in 1..p.video.len() {
        let d = p.trans_dur(k - 1);
        let (nv, na) = (format!("jv{k}"), format!("ja{k}"));
        if d > 0.0 {
            let kind = p.video[k - 1].transition.map(|t| t.kind.xfade()).unwrap_or("fade");
            let _ = write!(
                fg,
                "[{av}][v{k}]xfade=transition={kind}:duration={d:.3}:offset={:.3}[{nv}];[{aa}][a{k}]acrossfade=d={d:.3}[{na}];",
                (acc - d).max(0.0)
            );
            acc += p.video[k].dur() - d;
        } else {
            let _ = write!(fg, "[{av}][{aa}][v{k}][a{k}]concat=n=2:v=1:a=1[{nv}][{na}];");
            acc += p.video[k].dur();
        }
        av = nv;
        aa = na;
    }
    let _ = write!(fg, "[{av}]null[vcat];[{aa}]anull[acat];");

    // --- text overlays ----------------------------------------------------
    // Each line is drawn and centred on its own (drawtext's `text_align` only
    // exists in FFmpeg 6.1+, and Ubuntu 22.04 / Debian 12 ship older builds).
    // Lines sit on a fixed pitch; the font's glyph extents keep baselines even.
    let mut vlabel = "vcat".to_string();
    let texts: Vec<_> = p.texts.iter().filter(|t| t.start < total && !t.text.trim().is_empty()).collect();
    let mut n_draw = 0usize;
    for (i, t) in texts.iter().enumerate() {
        let size = (t.size * k).max(4.0);
        let lines: Vec<&str> = t.text.lines().collect();
        let pitch = size * 1.25;
        let block = pitch * lines.len() as f32;
        for (li, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue; // blank line: keeps its space, nothing to draw
            }
            let file = tmp.join(format!("text{i}_{li}.txt"));
            std::fs::write(&file, line).map_err(|e| e.to_string())?;
            let top = li as f32 * pitch - block / 2.0;
            let mut opts = format!(
                "textfile={}:fontsize={size:.1}:fontcolor={}:x='(w*{:.4})-text_w/2':y='(h*{:.4})+({top:.2})+({pitch:.2}-(max_glyph_a-max_glyph_d))/2':enable='between(t,{:.3},{:.3})'",
                q(&file.to_string_lossy()),
                hex(t.color),
                t.x,
                t.y,
                t.start,
                t.end()
            );
            if let Some(f) = &s.font {
                let _ = write!(opts, ":fontfile={}", q(&f.to_string_lossy()));
            }
            match t.style {
                TextStyle::Plain => {}
                TextStyle::Outline => {
                    let _ = write!(opts, ":borderw={:.1}:bordercolor=black", (size * 0.07).max(1.0));
                }
                TextStyle::Boxed => {
                    let _ = write!(opts, ":box=1:boxcolor=black@0.62:boxborderw={:.0}", size * 0.28);
                }
                TextStyle::Shadow => {
                    let d = (size * 0.06).max(1.0);
                    let _ = write!(opts, ":shadowx={d:.0}:shadowy={d:.0}:shadowcolor=black@0.7");
                }
            }
            let out = format!("t{n_draw}");
            n_draw += 1;
            let _ = write!(fg, "[{vlabel}]drawtext={opts}[{out}];");
            vlabel = out;
        }
    }

    // --- music / sfx --------------------------------------------------------
    let mut music = vec![];
    for (j, a) in p.audio.iter().filter(|a| a.start < total).enumerate() {
        let m = p.media(a.media).ok_or("Missing media for an audio clip")?;
        let dur = a.dur.min(total - a.start);
        args.extend(["-ss".into(), format!("{:.3}", a.src_in), "-t".into(), format!("{dur:.3}")]);
        args.extend(["-i".into(), m.path.to_string_lossy().into_owned()]);
        let n = input;
        input += 1;
        let ms = (a.start * 1000.0).round() as i64;
        let _ = write!(
            fg,
            "[{n}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.3},adelay={ms}:all=1[m{j}];",
            a.volume
        );
        music.push(format!("[m{j}]"));
    }
    let alabel = if music.is_empty() {
        "acat".to_string()
    } else {
        let _ = write!(
            fg,
            "[acat]{}amix=inputs={}:duration=first:dropout_transition=0:normalize=0[amixed];",
            music.join(""),
            music.len() + 1
        );
        "amixed".to_string()
    };
    let _ = write!(fg, "[{vlabel}]null[vout];[{alabel}]alimiter=limit=0.97[aout]");

    let crf = match s.quality {
        Quality::High => "18",
        Quality::Standard => "23",
    };
    args.extend(["-filter_complex".into(), fg]);
    args.extend(
        [
            "-map", "[vout]", "-map", "[aout]", "-c:v", "libx264", "-preset", "medium", "-crf", crf,
            "-profile:v", "high", "-pix_fmt", "yuv420p", "-r", "30", "-c:a", "aac", "-b:a", "192k",
            "-ar", "48000", "-movflags", "+faststart",
        ]
        .map(String::from),
    );
    args.extend(["-t".into(), format!("{total:.3}")]);
    args.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    args.push(s.out.to_string_lossy().into_owned());
    Ok(args)
}

pub fn start(p: &Project, s: ExportSettings, ctx: egui::Context) -> Result<ExportJob, String> {
    let tmp = std::env::temp_dir().join(format!("klip-export-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let args = build_args(p, &s, &tmp)?;
    let total = p.duration().max(0.001);

    let mut cmd = ffmpeg();
    // Keep stderr for error reporting (ffmpeg() silences it by default).
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Could not start ffmpeg: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let progress = Arc::new(Mutex::new((0.0f32, ExportStatus::Running)));
    let child = Arc::new(Mutex::new(Some(child)));
    let (pg, ch) = (progress.clone(), child.clone());

    let err_buf = Arc::new(Mutex::new(String::new()));
    let eb = err_buf.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut b = eb.lock().unwrap();
            b.push_str(&line);
            b.push('\n');
        }
    });

    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = v.trim().parse::<f64>() {
                    let mut g = pg.lock().unwrap();
                    g.0 = ((us / 1e6) / total).clamp(0.0, 1.0) as f32;
                    ctx.request_repaint();
                }
            }
        }
        let status = ch.lock().unwrap().take().and_then(|mut c| c.wait().ok());
        let _ = std::fs::remove_dir_all(&tmp);
        let mut g = pg.lock().unwrap();
        if g.1 == ExportStatus::Cancelled {
            // keep
        } else if status.is_some_and(|s| s.success()) {
            *g = (1.0, ExportStatus::Done);
        } else {
            let err = err_buf.lock().unwrap();
            let tail: Vec<&str> = err.lines().rev().take(6).collect();
            let msg = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            g.1 = ExportStatus::Failed(if msg.is_empty() { "ffmpeg failed".into() } else { msg });
        }
        ctx.request_repaint();
    });

    Ok(ExportJob { progress, child, out: s.out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    /// Synthesise small test clips with ffmpeg (cached between runs).
    fn gen_media(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let jobs: [(&str, &[&str]); 4] = [
            ("landscape.mp4", &["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "6", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
            ("portrait.mp4", &["-f", "lavfi", "-i", "testsrc2=size=720x1280:rate=30", "-f", "lavfi", "-i", "sine=frequency=660", "-t", "5", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
            ("photo.png", &["-f", "lavfi", "-i", "smptebars=size=1200x800", "-frames:v", "1"]),
            ("music.mp3", &["-f", "lavfi", "-i", "sine=frequency=220:duration=12", "-c:a", "libmp3lame"]),
        ];
        for (name, a) in jobs {
            let out = dir.join(name);
            if !out.exists() {
                let ok = std::process::Command::new("ffmpeg").args(["-v", "error", "-y"]).args(a).arg(&out).status().unwrap();
                assert!(ok.success(), "could not generate {name}");
            }
        }
    }

    /// End-to-end render (slow, ~5 s): `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn renders_full_project() {
        let dir = std::env::temp_dir().join("klip_test_media");
        gen_media(&dir);
        let mut p = Project::default();
        let add = |p: &mut Project, f: &str| {
            let mut m = crate::media::probe(&dir.join(f)).unwrap();
            m.id = p.new_id();
            p.media.push(m.clone());
            m
        };
        let land = add(&mut p, "landscape.mp4");
        let port = add(&mut p, "portrait.mp4");
        let photo = add(&mut p, "photo.png");
        let music = add(&mut p, "music.mp3");
        assert_eq!(photo.kind, MediaKind::Image);
        assert_eq!(music.kind, MediaKind::Audio);
        for (m, fit, a, b) in [(&land, FitMode::Blur, 1.0, 3.0), (&port, FitMode::Fill, 0.0, 2.0), (&photo, FitMode::Fit, 0.0, 1.5), (&land, FitMode::Fit, 4.0, 5.0)] {
            let id = p.new_id();
            p.video.push(VideoClip::new(id, m.id, a, b, fit));
        }
        // New features: transform, rotation, grade, fades, transitions.
        p.video[0].scale = 1.3;
        p.video[0].x = 0.1;
        p.video[0].rotation = 8.0;
        p.video[0].filter = crate::fx::FilterPreset::Film;
        p.video[0].fade_in = 0.5;
        p.video[0].transition = Some(crate::fx::Transition { kind: crate::fx::TransitionKind::SlideLeft, dur: 0.5 });
        p.video[1].adjust.saturation = 1.4;
        p.video[1].transition = Some(crate::fx::Transition { kind: crate::fx::TransitionKind::Mix, dur: 0.4 });
        p.video[3].fade_out = 0.4;
        // Speed: 2× (2 s → 1 s) and 0.5× (1 s → 2 s).
        p.video[1].set_speed(2.0);
        p.video[3].set_speed(0.5);
        for (i, st) in TextStyle::ALL.iter().enumerate() {
            let id = p.new_id();
            p.texts.push(TextClip { id, text: format!("Style {} it's: \"quoted\"\nline two", st.label()), start: i as f64, dur: 2.0, x: 0.5, y: 0.2 + i as f32 * 0.2, size: 70.0, color: [255, 214, 10], style: *st, is_caption: false });
        }
        let id = p.new_id();
        p.audio.push(AudioClip { id, media: music.id, start: 0.5, src_in: 1.0, dur: 4.0, volume: 0.5 });

        let out = std::env::temp_dir().join("klip_test_out.mp4");
        let tmp = std::env::temp_dir().join("klip_test_tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let s = ExportSettings { out: out.clone(), scale: 0.6667, quality: Quality::Standard, font: crate::theme::find_text_font() };
        let args = build_args(&p, &s, &tmp).unwrap();
        let o = std::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-nostdin"]).args(&args).output().unwrap();
        assert!(o.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr));
        let info = crate::media::probe(&out).unwrap();
        assert_eq!((info.width, info.height), (720, 1280));
        assert!((info.duration - p.duration()).abs() < 0.15, "duration {} vs {}", info.duration, p.duration());
        assert!(info.has_audio);
    }
}
