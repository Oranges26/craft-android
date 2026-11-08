//! The open photo: its decoded preview source, its develop settings and their history.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_develop::DevelopSettings;
use lightcraft_pipeline::SourceInfo;
use lightcraft_raster::Rgb32f;

/// Long edge of the decoded source the editor works on. Phone screens are at most ~3200 px on the
/// long side; zooming past that shows the preview's own pixels (exports always use the original).
pub const PREVIEW_EDGE: usize = 2560;

/// A decoded photo, ready to edit.
pub struct Loaded {
    pub key: String,
    pub name: String,
    pub path: PathBuf,
    pub src: Arc<Rgb32f>,
    pub info: SourceInfo,
}

/// Decode `path` for editing (runs on a worker thread).
pub fn load(path: &Path, name: &str) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    let key = key_of(&bytes);
    let (src, info) = lightcraft_engine::files::load_bytes(&bytes, PREVIEW_EDGE).map_err(|e| format!("Couldn't open {name}: {e}"))?;
    Ok(Loaded { key, name: name.to_string(), path: path.to_path_buf(), src: Arc::new(src), info })
}

/// A stable identity for a file's contents (its edits are stored under it).
pub fn key_of(bytes: &[u8]) -> String {
    let mut h = crc32fast::Hasher::new();
    h.update(&(bytes.len() as u64).to_le_bytes());
    let head = &bytes[..bytes.len().min(4 << 20)];
    h.update(head);
    if bytes.len() > head.len() {
        h.update(&bytes[bytes.len().saturating_sub(1 << 20)..]);
    }
    format!("{:08x}{:012x}", h.finalize(), bytes.len())
}

/// The settings a photo starts with.
pub fn defaults(info: &SourceInfo) -> DevelopSettings {
    if info.raw { DevelopSettings::for_raw(info.as_shot_temp, info.as_shot_tint) } else { DevelopSettings::default() }
}

/// An open photo being edited.
pub struct Doc {
    pub photo: Loaded,
    pub settings: DevelopSettings,
    pub original: DevelopSettings,
    undo: Vec<DevelopSettings>,
    redo: Vec<DevelopSettings>,
    /// Settings when the current gesture began (a slider drag is one undo step).
    gesture: Option<DevelopSettings>,
    /// Bumped on every change: the renderer and the autosave follow it.
    pub generation: u64,
    pub saved_generation: u64,
}

impl Doc {
    pub fn new(photo: Loaded, saved: Option<DevelopSettings>) -> Doc {
        let original = defaults(&photo.info);
        let settings = saved.unwrap_or_else(|| original.clone());
        Doc { photo, settings, original, undo: Vec::new(), redo: Vec::new(), gesture: None, generation: 1, saved_generation: 1 }
    }

    /// Start a gesture (slider drag): its changes become one undo step.
    pub fn begin(&mut self) {
        if self.gesture.is_none() {
            self.gesture = Some(self.settings.clone());
        }
    }

    /// End the gesture: one undo step if anything changed.
    pub fn end(&mut self) {
        if let Some(before) = self.gesture.take()
            && before != self.settings
        {
            self.push_undo(before);
        }
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Change the settings live (inside a gesture, or as its own step outside one).
    pub fn edit(&mut self, f: impl FnOnce(&mut DevelopSettings)) {
        let before = self.settings.clone();
        f(&mut self.settings);
        if before == self.settings {
            return;
        }
        self.generation += 1;
        if self.gesture.is_none() {
            self.push_undo(before);
        }
    }

    /// Replace the settings as one step.
    pub fn set(&mut self, s: DevelopSettings) {
        self.edit(|cur| *cur = s);
    }

    fn push_undo(&mut self, before: DevelopSettings) {
        self.undo.push(before);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.gesture = None;
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.settings, s));
            self.generation += 1;
        }
    }

    pub fn redo(&mut self) {
        self.gesture = None;
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.settings, s));
            self.generation += 1;
        }
    }

    /// Size of the oriented source (before crop), in preview pixels.
    pub fn oriented_size(&self) -> (f64, f64) {
        let (w, h) = (self.photo.src.width as f64, self.photo.src.height as f64);
        if self.settings.orientation.swaps_axes() { (h, w) } else { (w, h) }
    }
}
