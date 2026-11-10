//! Audio preview. Each sounding clip gets its own short-lived ffmpeg process
//! that plays straight to PulseAudio/PipeWire. A small watchdog restarts a
//! voice once if its process dies right after starting (e.g. the audio server
//! was briefly busy), and logs why.

use crate::media::ffmpeg;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct Voice {
    child: Option<Child>,
    started: Instant,
    path: PathBuf,
    src_t: f64,
    dur: f64,
    speed: f64,
    volume: f32,
    retried: bool,
    log: Arc<Mutex<String>>,
}

pub struct AudioPreview {
    voices: HashMap<String, Voice>,
    /// `KLIP_MUTE=1` silences preview audio (handy for automated runs).
    muted: bool,
    debug: bool,
}

impl Default for AudioPreview {
    fn default() -> Self {
        Self {
            voices: HashMap::new(),
            muted: std::env::var_os("KLIP_MUTE").is_some(),
            debug: std::env::var_os("KLIP_DEBUG_AUDIO").is_some(),
        }
    }
}

/// Play `dur` seconds of output, reading the source from `src_t` at `speed`.
fn spawn(path: &Path, src_t: f64, dur: f64, speed: f64, volume: f32, log: Arc<Mutex<String>>) -> Option<Child> {
    let tempo = crate::fx::atempo(speed);
    let af = if tempo.is_empty() { format!("volume={volume:.3}") } else { format!("{tempo},volume={volume:.3}") };
    let mut child = ffmpeg()
        .args(["-ss", &format!("{:.3}", src_t.max(0.0))])
        .args(["-t", &format!("{:.3}", dur * speed)])
        .arg("-i")
        .arg(path)
        .args(["-vn", "-af", &af, "-ac", "2", "-ar", "48000"])
        .args(["-f", "pulse", "-buffer_duration", "50", "Klip preview"])
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            log.lock().unwrap().push_str(&s);
        });
    }
    Some(child)
}

impl AudioPreview {
    pub fn is_playing(&self, key: &str) -> bool {
        self.voices.contains_key(key)
    }

    /// Play `dur` seconds (timeline time) of `path` from `src_t`, at `speed`.
    pub fn play(&mut self, key: String, path: &Path, src_t: f64, dur: f64, speed: f64, volume: f32) {
        self.stop(&key);
        let silent = dur <= 0.02 || volume <= 0.001 || self.muted;
        let log = Arc::new(Mutex::new(String::new()));
        // Silent voices are still tracked so we don't retry every frame.
        let child = if silent { None } else { spawn(path, src_t, dur, speed, volume, log.clone()) };
        if self.debug {
            eprintln!("[audio] start {key} @{src_t:.2}s for {dur:.2}s vol {volume:.2} → {}", if child.is_some() { "ok" } else { "none" });
        }
        self.voices.insert(
            key,
            Voice { child, started: Instant::now(), path: path.to_path_buf(), src_t, dur, speed, volume, retried: false, log },
        );
    }

    /// Watchdog: call once per frame while playing.
    pub fn reap(&mut self) {
        for (key, v) in self.voices.iter_mut() {
            let Some(child) = v.child.as_mut() else { continue };
            let Ok(Some(status)) = child.try_wait() else { continue };
            v.child = None;
            let age = v.started.elapsed().as_secs_f64();
            let msg = v.log.lock().unwrap().trim().to_string();
            if status.success() {
                continue;
            }
            eprintln!("[audio] {key} exited ({status}) after {age:.2}s: {msg}");
            if !v.retried && age < 2.0 && v.dur - age > 0.1 {
                v.retried = true;
                v.started = Instant::now();
                v.src_t += age * v.speed;
                v.dur -= age;
                v.log = Arc::new(Mutex::new(String::new()));
                v.child = spawn(&v.path, v.src_t, v.dur, v.speed, v.volume, v.log.clone());
            }
        }
    }

    pub fn stop(&mut self, key: &str) {
        if let Some(v) = self.voices.remove(key) {
            kill(v);
        }
    }

    pub fn stop_all(&mut self) {
        for (_, v) in self.voices.drain() {
            kill(v);
        }
    }
}

fn kill(v: Voice) {
    if let Some(mut c) = v.child {
        let _ = c.kill();
        // Reap off the UI thread so stopping never stalls a frame.
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

impl Drop for AudioPreview {
    fn drop(&mut self) {
        self.stop_all();
    }
}
