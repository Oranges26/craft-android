//! Recent photos: a copy of each opened original, its edits and a thumbnail, kept in the app's
//! private storage so reopening a photo brings its edits back.

use std::path::PathBuf;

use lightcraft_develop::DevelopSettings;
use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 24;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub name: String,
    /// The library's copy of the original.
    pub file: PathBuf,
    pub opened: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Library {
    pub entries: Vec<Entry>,
}

fn dir() -> PathBuf {
    let d = craft_mobile_kit::platform::files_dir().join("library");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Library {
    pub fn load() -> Library {
        let mut lib: Library = std::fs::read(dir().join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        lib.entries.retain(|e| e.file.exists());
        lib
    }

    fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(dir().join("index.json"), b);
        }
    }

    /// Take `picked` (a file the picker copied into the cache) into the library. Returns the
    /// library copy's path and the photo's key.
    pub fn add(&mut self, picked: &std::path::Path, name: &str) -> Result<(PathBuf, String), String> {
        let bytes = std::fs::read(picked).map_err(|e| format!("Couldn't read {name}: {e}"))?;
        let key = crate::doc::key_of(&bytes);
        let ext = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "img".into());
        let file = dir().join(format!("{key}.{ext}"));
        if !file.exists() {
            std::fs::write(&file, &bytes).map_err(|e| format!("Couldn't store {name}: {e}"))?;
        }
        if picked.starts_with(craft_mobile_kit::platform::cache_dir()) {
            let _ = std::fs::remove_file(picked);
        }
        self.entries.retain(|e| e.key != key);
        self.entries.insert(0, Entry { key: key.clone(), name: name.to_string(), file: file.clone(), opened: now() });
        while self.entries.len() > MAX_RECENT {
            if let Some(old) = self.entries.pop() {
                self.forget_files(&old);
            }
        }
        self.save();
        Ok((file, key))
    }

    pub fn touch(&mut self, key: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            let mut e = self.entries.remove(i);
            e.opened = now();
            self.entries.insert(0, e);
            self.save();
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            let e = self.entries.remove(i);
            self.forget_files(&e);
            self.save();
        }
    }

    fn forget_files(&self, e: &Entry) {
        let _ = std::fs::remove_file(&e.file);
        let _ = std::fs::remove_file(edits_path(&e.key));
        let _ = std::fs::remove_file(thumb_path(&e.key));
    }
}

pub fn edits_path(key: &str) -> PathBuf {
    dir().join(format!("{key}.edits.json"))
}

pub fn thumb_path(key: &str) -> PathBuf {
    dir().join(format!("{key}.thumb.png"))
}

pub fn load_edits(key: &str) -> Option<DevelopSettings> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(edits_path(key)).ok()?).ok()?;
    DevelopSettings::from_untrusted_json(&v).ok()
}

pub fn save_edits(key: &str, s: &DevelopSettings) {
    if let Ok(b) = serde_json::to_vec(&s.to_json()) {
        let _ = std::fs::write(edits_path(key), b);
    }
}

pub fn save_thumb(key: &str, img: &lightcraft_raster::Rgba8) {
    let t = crate::render::shrink(img, 360);
    if let Ok(b) = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&t), &Default::default()) {
        let _ = std::fs::write(thumb_path(key), b);
    }
}

pub fn load_thumb(key: &str) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(thumb_path(key)).ok()?;
    let d = lightcraft_codecs::decode_thumbnail(&bytes, 360).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([d.image.width, d.image.height], bytemuck_rgba(&d.image)))
}

fn bytemuck_rgba(img: &lightcraft_raster::Rgba8) -> &[u8] {
    unsafe { std::slice::from_raw_parts(img.data.as_ptr().cast::<u8>(), img.data.len() * 4) }
}
