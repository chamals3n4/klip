//! Project store: every project lives in its own folder that Klip manages.
//!
//! ```text
//! ~/.local/share/klip/projects/
//!   <id>/project.klip   the full project (JSON)
//!   <id>/meta.json      name, dates, duration… (all the home screen reads)
//!   <id>/thumb.jpg      cover image for the project grid
//!   .trash/<id>/        deleted projects, purged after 30 days
//! ```
//!
//! Media is referenced where it lives on disk, never copied.

use crate::model::{Aspect, Project};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current `.klip` format version.
pub const FORMAT_VERSION: u32 = 2;
const PROJECT_FILE: &str = "project.klip";
const META_FILE: &str = "meta.json";
const THUMB_FILE: &str = "thumb.jpg";
const TRASH_DIR: &str = ".trash";
/// How long deleted projects stay recoverable.
pub const TRASH_TTL: u64 = 30 * 24 * 60 * 60;

/// What the home screen needs to show a project, without loading it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub aspect: Aspect,
    /// Unix seconds.
    pub created: u64,
    pub edited: u64,
    /// Seconds of video on the main track.
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub clips: usize,
    /// Set while the project sits in the trash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<u64>,
}

impl Meta {
    fn refresh(&mut self, p: &Project) {
        self.name = p.name.clone();
        self.aspect = p.aspect;
        self.duration = p.duration();
        self.clips = p.video.len() + p.texts.len() + p.audio.len();
        self.edited = now();
    }
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn thumb_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(THUMB_FILE)
    }

    /// Where a listed project's cover lives (it moves with the project to the trash).
    pub fn cover_path(&self, m: &Meta) -> PathBuf {
        if m.deleted.is_some() { self.trash().join(&m.id).join(THUMB_FILE) } else { self.thumb_path(&m.id) }
    }

    fn trash(&self) -> PathBuf {
        self.root.join(TRASH_DIR)
    }

    /// All projects, most recently edited first. Broken folders are skipped.
    pub fn list(&self) -> Vec<Meta> {
        let mut out = scan(&self.root);
        out.sort_by(|a, b| b.edited.cmp(&a.edited).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Projects in the trash, most recently deleted first.
    pub fn list_trash(&self) -> Vec<Meta> {
        let mut out = scan(&self.trash());
        out.sort_by(|a, b| b.deleted.cmp(&a.deleted));
        out
    }

    /// A fresh, empty project.
    pub fn create(&self, name: &str, aspect: Aspect) -> io::Result<Meta> {
        let mut p = Project { aspect, ..Default::default() };
        p.name = self.unique_name(name);
        self.insert(&p)
    }

    /// Add an existing project (e.g. an imported `.klip`) as a new entry.
    pub fn insert(&self, p: &Project) -> io::Result<Meta> {
        let id = self.new_id();
        std::fs::create_dir_all(self.dir(&id))?;
        let t = now();
        let mut meta = Meta {
            id: id.clone(),
            name: p.name.clone(),
            aspect: p.aspect,
            created: t,
            edited: t,
            duration: 0.0,
            clips: 0,
            deleted: None,
        };
        meta.refresh(p);
        write_project(&self.dir(&id).join(PROJECT_FILE), p, None)?;
        write_json(&self.dir(&id).join(META_FILE), &meta)?;
        Ok(meta)
    }

    pub fn meta(&self, id: &str) -> io::Result<Meta> {
        read_meta(&self.dir(id))
    }

    pub fn load(&self, id: &str) -> Result<Project, String> {
        read_project(&self.dir(id).join(PROJECT_FILE)).map(|(p, _)| p)
    }

    /// Write the project and refresh its metadata.
    pub fn save(&self, id: &str, p: &Project) -> io::Result<Meta> {
        let dir = self.dir(id);
        let mut meta = read_meta(&dir)?;
        meta.refresh(p);
        write_project(&dir.join(PROJECT_FILE), p, None)?;
        write_json(&dir.join(META_FILE), &meta)?;
        Ok(meta)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Meta, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name can't be empty".into());
        }
        let mut p = self.load(id)?;
        p.name = name.to_string();
        self.save(id, &p).map_err(|e| e.to_string())
    }

    /// Copy a project under a new name ("Name copy", "Name copy 2", …).
    pub fn duplicate(&self, id: &str) -> Result<Meta, String> {
        let mut p = self.load(id)?;
        p.name = self.unique_name(&format!("{} copy", p.name));
        let meta = self.insert(&p).map_err(|e| e.to_string())?;
        let thumb = self.thumb_path(id);
        if thumb.exists() {
            let _ = std::fs::copy(thumb, self.thumb_path(&meta.id));
        }
        Ok(meta)
    }

    /// Move a project to the trash.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        let mut meta = self.meta(id)?;
        meta.deleted = Some(now());
        write_json(&self.dir(id).join(META_FILE), &meta)?;
        std::fs::create_dir_all(self.trash())?;
        std::fs::rename(self.dir(id), self.trash().join(id))
    }

    /// Bring a project back from the trash.
    pub fn restore(&self, id: &str) -> io::Result<Meta> {
        let from = self.trash().join(id);
        let mut meta = read_meta(&from)?;
        meta.deleted = None;
        write_json(&from.join(META_FILE), &meta)?;
        std::fs::rename(from, self.dir(id))?;
        Ok(meta)
    }

    /// Permanently remove one project from the trash.
    pub fn delete_forever(&self, id: &str) -> io::Result<()> {
        std::fs::remove_dir_all(self.trash().join(id))
    }

    /// Permanently remove trashed projects older than `ttl` seconds.
    pub fn purge_trash(&self, ttl: u64) -> usize {
        let cutoff = now().saturating_sub(ttl);
        let mut n = 0;
        for m in scan(&self.trash()) {
            if m.deleted.unwrap_or(0) <= cutoff && std::fs::remove_dir_all(self.trash().join(&m.id)).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Import a `.klip` file from anywhere as a new project.
    pub fn import_file(&self, path: &Path) -> Result<Meta, String> {
        let (mut p, _) = read_project(path)?;
        if p.name.trim().is_empty() {
            p.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        }
        self.import_project(p).map_err(|e| e.to_string())
    }

    /// Add a project under a name that isn't taken yet.
    pub fn import_project(&self, mut p: Project) -> io::Result<Meta> {
        p.name = self.unique_name(&p.name);
        self.insert(&p)
    }

    /// `base`, or `base 2`, `base 3`… whichever isn't taken yet.
    pub fn unique_name(&self, base: &str) -> String {
        let base = match base.trim() {
            "" => "Untitled reel",
            b => b,
        };
        let taken: Vec<String> = self.list().into_iter().map(|m| m.name.to_lowercase()).collect();
        if !taken.contains(&base.to_lowercase()) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|c| !taken.contains(&c.to_lowercase()))
            .unwrap()
    }

    fn new_id(&self) -> String {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        loop {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let id = format!("{:012x}{:04x}", nanos as u64 & 0xffff_ffff_ffff, seq & 0xffff);
            if !self.dir(&id).exists() && !self.trash().join(&id).exists() {
                return id;
            }
        }
    }
}

// ------------------------------------------------------------- file format

/// Read a `.klip` file: (project, the path it was saved from, if recorded).
pub fn read_project(path: &Path) -> Result<(Project, Option<PathBuf>), String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    if v["app"] != "klip" {
        return Err("Not a Klip project".into());
    }
    let p: Project = serde_json::from_value(v["project"].clone()).map_err(|e| e.to_string())?;
    let origin = v["path"].as_str().map(PathBuf::from);
    Ok((p, origin))
}

/// Write a `.klip` file safely: a crash mid-write never leaves a broken file.
pub fn write_project(path: &Path, p: &Project, origin: Option<&Path>) -> io::Result<()> {
    let mut json = serde_json::json!({ "app": "klip", "version": FORMAT_VERSION, "project": p });
    if let Some(o) = origin {
        json["path"] = serde_json::json!(o);
    }
    write_atomic(path, serde_json::to_string_pretty(&json)?.as_bytes())
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> io::Result<()> {
    write_atomic(path, serde_json::to_string_pretty(v)?.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn read_meta(dir: &Path) -> io::Result<Meta> {
    let s = std::fs::read_to_string(dir.join(META_FILE))?;
    serde_json::from_str(&s).map_err(io::Error::other)
}

/// Every readable project folder directly under `dir`.
fn scan(dir: &Path) -> Vec<Meta> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| !n.starts_with('.')))
        .filter_map(|e| read_meta(&e.path()).ok())
        .collect()
}

/// `$XDG_DATA_HOME/klip`, usually `~/.local/share/klip`.
/// `KLIP_DATA_DIR` overrides it (used by tests and automation).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KLIP_DATA_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("klip")
}

// -------------------------------------------------------------- thumbnails

/// Pixel size of a project's cover image (cropped to the canvas ratio).
pub fn thumb_size(aspect: Aspect) -> (usize, usize) {
    let w = 288.0;
    let h = (w / crate::app::aspect_of(aspect) / 2.0).round() * 2.0;
    (w as usize, h as usize)
}

/// Render the cover image from the first clip of the main track. Removes a
/// stale cover when the project has no video.
pub fn write_thumb(p: &Project, out: &Path) -> bool {
    let Some(clip) = p.video.first() else {
        let _ = std::fs::remove_file(out);
        return false;
    };
    let Some(m) = p.media.iter().find(|m| m.id == clip.media) else { return false };
    if !m.path.exists() {
        return false;
    }
    let (w, h) = thumb_size(p.aspect);
    let mut cmd = crate::media::ffmpeg();
    if m.kind == crate::model::MediaKind::Video {
        // A moment in, so fades from black don't make an empty cover.
        let t = clip.src_in + (clip.src_out - clip.src_in).clamp(0.0, 3.0) * 0.3;
        cmd.args(["-ss", &format!("{t:.3}")]);
    }
    let mut vf = format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
    let grade = crate::fx::grade(clip.filter, clip.filter_amount, &clip.adjust).ffmpeg();
    if !grade.is_empty() {
        vf = format!("{vf},{grade}");
    }
    let tmp = out.with_extension("tmp.jpg");
    let ok = cmd
        .arg("-i")
        .arg(&m.path)
        .args(["-frames:v", "1", "-vf", &vf, "-q:v", "4", "-y"])
        .arg(&tmp)
        .status()
        .is_ok_and(|s| s.success());
    ok && std::fs::rename(&tmp, out).is_ok()
}

/// Decode a cover image written by [`write_thumb`].
pub fn read_thumb(path: &Path, aspect: Aspect) -> Option<crate::media::Frame> {
    if !path.exists() {
        return None;
    }
    let (w, h) = thumb_size(aspect);
    crate::media::grab_frame(path, 0.0, w, h)
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FitMode, MediaInfo, MediaKind, VideoClip};

    /// A store in its own temp folder, removed when dropped.
    struct TempStore(Store);
    impl TempStore {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("klip_store_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            Self(Store::at(root))
        }
    }
    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.root());
        }
    }
    impl std::ops::Deref for TempStore {
        type Target = Store;
        fn deref(&self) -> &Store {
            &self.0
        }
    }

    fn sample() -> Project {
        let mut p = Project { name: "Trip".into(), aspect: Aspect::Square, ..Default::default() };
        p.media.push(MediaInfo {
            id: 1,
            path: "/videos/a.mp4".into(),
            name: "a.mp4".into(),
            kind: MediaKind::Video,
            duration: 10.0,
            width: 1080,
            height: 1920,
            has_audio: true,
        });
        p.video.push(VideoClip::new(2, 1, 0.0, 4.0, FitMode::Fill));
        p.next_id = 3;
        p
    }

    #[test]
    fn create_list_and_save() {
        let s = TempStore::new("create");
        assert!(s.list().is_empty());
        let a = s.create("Reel", Aspect::Reel).unwrap();
        let b = s.create("Reel", Aspect::Landscape).unwrap();
        assert_eq!(b.name, "Reel 2");
        assert_eq!(s.load(&a.id).unwrap().aspect, Aspect::Reel);

        let mut p = sample();
        p.name = "Edited".into();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let m = s.save(&a.id, &p).unwrap();
        assert_eq!((m.name.as_str(), m.clips, m.duration), ("Edited", 1, 4.0));
        assert_eq!(m.created, a.created);
        assert_eq!(s.load(&a.id).unwrap(), p);
        // Most recently edited first.
        assert_eq!(s.list().iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![a.id, b.id]);
    }

    #[test]
    fn rename_and_duplicate() {
        let s = TempStore::new("dup");
        let a = s.insert(&sample()).unwrap();
        assert!(s.rename(&a.id, "  ").is_err());
        s.rename(&a.id, "Beach day").unwrap();
        assert_eq!(s.load(&a.id).unwrap().name, "Beach day");

        let c1 = s.duplicate(&a.id).unwrap();
        let c2 = s.duplicate(&a.id).unwrap();
        assert_eq!((c1.name.as_str(), c2.name.as_str()), ("Beach day copy", "Beach day copy 2"));
        assert_ne!(c1.id, a.id);
        assert_eq!(s.load(&c1.id).unwrap().video, sample().video);
        assert_eq!(s.list().len(), 3);
    }

    #[test]
    fn trash_restore_and_purge() {
        let s = TempStore::new("trash");
        let a = s.create("One", Aspect::Reel).unwrap();
        let b = s.create("Two", Aspect::Reel).unwrap();
        s.delete(&a.id).unwrap();
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.list_trash()[0].id, a.id);
        assert!(s.list_trash()[0].deleted.is_some());

        s.restore(&a.id).unwrap();
        assert_eq!(s.list().len(), 2);
        assert!(s.meta(&a.id).unwrap().deleted.is_none());

        s.delete(&b.id).unwrap();
        assert_eq!(s.purge_trash(TRASH_TTL), 0, "fresh deletes are kept");
        assert_eq!(s.purge_trash(0), 1);
        assert!(s.list_trash().is_empty());
    }

    #[test]
    fn import_and_format() {
        let s = TempStore::new("import");
        // A v1 file as written by Klip 0.1.
        let file = s.root().with_extension("klip");
        let v1 = serde_json::json!({ "app": "klip", "version": 1, "project": sample() });
        std::fs::write(&file, v1.to_string()).unwrap();
        let a = s.import_file(&file).unwrap();
        let b = s.import_file(&file).unwrap();
        assert_eq!((a.name.as_str(), b.name.as_str()), ("Trip", "Trip 2"));
        assert_eq!(s.load(&a.id).unwrap().media, sample().media);

        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(s.dir(&a.id).join(PROJECT_FILE)).unwrap()).unwrap();
        assert_eq!(saved["version"], FORMAT_VERSION);

        std::fs::write(&file, "{\"hello\":1}").unwrap();
        assert!(s.import_file(&file).is_err());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn broken_folders_are_skipped() {
        let s = TempStore::new("broken");
        s.create("Ok", Aspect::Reel).unwrap();
        std::fs::create_dir_all(s.root().join("junk")).unwrap();
        std::fs::write(s.root().join("junk").join(META_FILE), "not json").unwrap();
        assert_eq!(s.list().len(), 1);
    }
}
