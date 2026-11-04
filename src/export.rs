//! Export: turn the project into one ffmpeg command with a filter graph.
//!
//!   main track clips ──scale/crop/blur──┐
//!                                      concat ── drawtext… ──► video
//!   clip audio / silence ───────────────┘   └── amix(music…) ──► audio

use crate::media::ffmpeg;
use crate::model::{FitMode, MediaKind, Project, TextStyle, FPS};
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
    for (i, c) in p.video.iter().enumerate() {
        let m = p.media(c.media).ok_or("Missing media for a clip")?;
        if m.kind == MediaKind::Image {
            args.extend(["-loop", "1", "-framerate", "30"].map(String::from));
            args.extend(["-t".into(), format!("{:.3}", c.dur())]);
        } else {
            args.extend(["-ss".into(), format!("{:.3}", c.src_in)]);
            args.extend(["-t".into(), format!("{:.3}", c.dur())]);
        }
        args.extend(["-i".into(), m.path.to_string_lossy().into_owned()]);
        let n = input;
        input += 1;

        let pre = format!("[{n}:v]fps={FPS},setpts=PTS-STARTPTS");
        let cover = format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
        let contain = format!("scale={w}:{h}:force_original_aspect_ratio=decrease");
        match c.fit {
            FitMode::Fill => {
                let _ = write!(fg, "{pre},{cover},setsar=1,format=yuv420p[v{i}];");
            }
            FitMode::Fit => {
                let _ = write!(
                    fg,
                    "{pre},{contain},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format=yuv420p[v{i}];"
                );
            }
            FitMode::Blur => {
                let r = (w.min(h) / 24).max(4);
                let _ = write!(
                    fg,
                    "{pre},split[b{i}][f{i}];[b{i}]scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},boxblur={br}:2,scale={w}:{h},eq=brightness=-0.08[bb{i}];\
                     [f{i}]{contain}[ff{i}];[bb{i}][ff{i}]overlay=(W-w)/2:(H-h)/2,setsar=1,format=yuv420p[v{i}];",
                    sw = even(w as f32 / 4.0),
                    sh = even(h as f32 / 4.0),
                    br = r / 4,
                );
            }
        }
        let dur = c.dur();
        if m.has_audio {
            let _ = write!(
                fg,
                "[{n}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.3},apad,atrim=0:{dur:.3},asetpts=PTS-STARTPTS[a{i}];",
                c.volume
            );
        } else {
            let _ = write!(fg, "anullsrc=r=48000:cl=stereo,atrim=0:{dur:.3},aformat=sample_fmts=fltp[a{i}];");
        }
    }
    for i in 0..p.video.len() {
        let _ = write!(fg, "[v{i}][a{i}]");
    }
    let _ = write!(fg, "concat=n={}:v=1:a=1[vcat][acat];", p.video.len());

    // --- text overlays ----------------------------------------------------
    let mut vlabel = "vcat".to_string();
    let texts: Vec<_> = p.texts.iter().filter(|t| t.start < total && !t.text.trim().is_empty()).collect();
    for (i, t) in texts.iter().enumerate() {
        let file = tmp.join(format!("text{i}.txt"));
        std::fs::write(&file, &t.text).map_err(|e| e.to_string())?;
        let size = (t.size * k).max(4.0);
        let mut opts = format!(
            "textfile={}:fontsize={size:.1}:fontcolor={}:text_align=C:line_spacing={:.0}:x=(w*{:.4})-text_w/2:y=(h*{:.4})-text_h/2:enable='between(t,{:.3},{:.3})'",
            q(&file.to_string_lossy()),
            hex(t.color),
            size * 0.12,
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
        let out = format!("t{i}");
        let _ = write!(fg, "[{vlabel}]drawtext={opts}[{out}];");
        vlabel = out;
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

    /// End-to-end render. Run with:
    /// `KLIP_TEST_MEDIA=/dir/with/landscape.mp4,portrait.mp4,photo.png,music.mp3 cargo test -- --ignored`
    #[test]
    #[ignore]
    fn renders_full_project() {
        let dir = PathBuf::from(std::env::var("KLIP_TEST_MEDIA").expect("KLIP_TEST_MEDIA"));
        let mut p = Project::default();
        let mut add = |p: &mut Project, f: &str| {
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
            p.video.push(VideoClip { id, media: m.id, src_in: a, src_out: b, fit, volume: 1.0 });
        }
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
