//! Projects and media kept in the app's private storage: `projects/<id>.fcproj` (upstream's
//! project format), a poster frame per project, and `media/` holding the files imported into them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub modified: u64,
    /// Length of the main sequence in seconds (shown on the home screen).
    pub seconds: f64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub projects: Vec<ProjectEntry>,
}

pub fn root() -> PathBuf {
    craft_mobile_kit::platform::files_dir()
}

fn projects_dir() -> PathBuf {
    let d = root().join("projects");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn media_dir() -> PathBuf {
    let d = root().join("media");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Store {
    pub fn load() -> Store {
        let mut s: Store = std::fs::read(projects_dir().join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        s.projects.retain(|p| project_path(&p.id).exists());
        s.projects.sort_by(|a, b| b.modified.cmp(&a.modified));
        s
    }

    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(projects_dir().join("index.json"), b);
        }
    }

    pub fn upsert(&mut self, e: ProjectEntry) {
        self.projects.retain(|p| p.id != e.id);
        self.projects.insert(0, e);
        self.save();
    }

    pub fn remove(&mut self, id: &str) {
        self.projects.retain(|p| p.id != id);
        let _ = std::fs::remove_file(project_path(id));
        let _ = std::fs::remove_file(poster_path(id));
        self.save();
    }

    pub fn new_id(&self) -> String {
        let base = now();
        let mut n = 0;
        loop {
            let id = format!("p{base}{n:02}");
            if !self.projects.iter().any(|p| p.id == id) && !project_path(&id).exists() {
                return id;
            }
            n += 1;
        }
    }
}

pub fn project_path(id: &str) -> PathBuf {
    projects_dir().join(format!("{id}.fcproj"))
}

pub fn poster_path(id: &str) -> PathBuf {
    projects_dir().join(format!("{id}.poster.rgba"))
}

/// Move a picked file (copied into the cache by the picker) into the media folder, keeping its
/// name (numbered when taken). Files outside the cache (desktop testing) are used where they are.
pub fn adopt_media(picked: &Path, name: &str) -> std::io::Result<PathBuf> {
    if !picked.starts_with(craft_mobile_kit::platform::cache_dir()) {
        return Ok(picked.to_path_buf());
    }
    let clean: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let stem = Path::new(&clean).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "media".into());
    let ext = Path::new(&clean).extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let mut dest = media_dir().join(&clean);
    let mut n = 2;
    while dest.exists() {
        dest = media_dir().join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    if std::fs::rename(picked, &dest).is_err() {
        std::fs::copy(picked, &dest)?;
        let _ = std::fs::remove_file(picked);
    }
    Ok(dest)
}

/// A small RGBA poster: width, height, pixels.
pub fn save_poster(id: &str, w: usize, h: usize, rgba: &[u8]) {
    let mut b = Vec::with_capacity(8 + rgba.len());
    b.extend_from_slice(&(w as u32).to_le_bytes());
    b.extend_from_slice(&(h as u32).to_le_bytes());
    b.extend_from_slice(rgba);
    let _ = std::fs::write(poster_path(id), b);
}

pub fn load_poster(id: &str) -> Option<egui::ColorImage> {
    let b = std::fs::read(poster_path(id)).ok()?;
    let w = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    let h = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
    let px = b.get(8..8 + w * h * 4)?;
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], px))
}
