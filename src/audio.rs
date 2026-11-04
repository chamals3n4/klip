//! Audio preview. Each sounding clip gets its own short-lived ffmpeg process
//! that plays straight to PulseAudio/PipeWire. Crude, but dependency-free and
//! good enough for an MVP; a proper mixer (cpal) can replace it later.

use crate::media::ffmpeg;
use std::collections::HashMap;
use std::path::Path;
use std::process::Child;

pub struct AudioPreview {
    voices: HashMap<String, Child>,
    /// `KLIP_MUTE=1` silences preview audio (handy for automated runs).
    muted: bool,
}

impl Default for AudioPreview {
    fn default() -> Self {
        Self { voices: HashMap::new(), muted: std::env::var_os("KLIP_MUTE").is_some() }
    }
}

impl AudioPreview {
    pub fn is_playing(&self, key: &str) -> bool {
        self.voices.contains_key(key)
    }

    pub fn play(&mut self, key: String, path: &Path, src_t: f64, dur: f64, volume: f32) {
        self.stop(&key);
        if dur <= 0.02 || volume <= 0.001 || self.muted {
            // Mark as "playing" anyway so we don't retry every frame.
            if let Ok(c) = std::process::Command::new("true").spawn() {
                self.voices.insert(key, c);
            }
            return;
        }
        let child = ffmpeg()
            .args(["-ss", &format!("{:.3}", src_t.max(0.0))])
            .args(["-t", &format!("{dur:.3}")])
            .arg("-i")
            .arg(path)
            .args(["-vn", "-af", &format!("volume={volume:.3}"), "-ac", "2"])
            .args(["-f", "pulse", "-buffer_duration", "60", "Klip preview"])
            .spawn();
        if let Ok(c) = child {
            self.voices.insert(key, c);
        }
    }

    pub fn stop(&mut self, key: &str) {
        if let Some(mut c) = self.voices.remove(key) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, mut c) in self.voices.drain() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for AudioPreview {
    fn drop(&mut self) {
        self.stop_all();
    }
}
