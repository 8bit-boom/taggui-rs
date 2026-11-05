//! The per-image data model, ported from the Python `Image` dataclass
//! (`taggui/utils/image.py`) and the directory loading in
//! `taggui/models/image_list_model.py`.

use std::fmt;
use std::path::{Path, PathBuf};

/// One image in the dataset. `tags` is the ordered tag list persisted in
/// the sidecar `.txt` file.
#[derive(Debug, Clone)]
pub struct ImageEntry {
    pub path: PathBuf,
    /// Pixel dimensions `(width, height)`, `None` if they could not be read.
    pub dimensions: Option<(u32, u32)>,
    pub tags: Vec<String>,
    /// File size in bytes, `None` if it could not be read.
    pub file_size: Option<u64>,
    /// Whether a sidecar `.txt` file existed when the folder was loaded
    /// (the Python app tracks this separately from `tags` being empty).
    pub has_caption_file: bool,
    /// Generated thumbnail (small RGBA preview), populated lazily by the GUI.
    pub thumbnail: Option<image::RgbaImage>,
}

impl ImageEntry {
    /// The sidecar `.txt` path for this image: same stem, `.txt` suffix
    /// (`image.path.with_suffix('.txt')` in Python).
    pub fn caption_file_path(&self) -> PathBuf {
        with_txt_suffix(&self.path)
    }
}

/// Python's `Path.with_suffix('.txt')`: replaces an existing suffix or
/// appends one when the file name has no suffix.
pub fn with_txt_suffix(path: &Path) -> PathBuf {
    let mut path_buf = path.to_path_buf();
    path_buf.set_extension("txt");
    path_buf
}

impl fmt::Display for ImageEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_file_path_replaces_suffix() {
        let entry_path = Path::new("/ds/photo.jpeg");
        assert_eq!(with_txt_suffix(entry_path), PathBuf::from("/ds/photo.txt"));
    }

    #[test]
    fn caption_file_path_appends_when_no_suffix() {
        let entry_path = Path::new("/ds/photo");
        assert_eq!(with_txt_suffix(entry_path), PathBuf::from("/ds/photo.txt"));
    }

    #[test]
    fn caption_file_path_handles_dotfiles() {
        // pathlib treats ".hidden" as a stem with no suffix, so
        // with_suffix('.txt') appends; Rust's set_extension does the same.
        let entry_path = Path::new("/ds/.hidden");
        assert_eq!(
            with_txt_suffix(entry_path),
            PathBuf::from("/ds/.hidden.txt")
        );
    }
}
