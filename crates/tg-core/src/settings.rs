//! App settings, ported from `taggui/utils/settings.py`'s defaults.
//! Stored as `config.toml` next to nothing in particular: the GUI decides
//! where to keep it (platform config dir). This intentionally does NOT
//! reuse the Python app's QSettings store - that's a documented
//! divergence (see PORT_STATUS.md).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Tag separator used for sidecar files (the Python default appends a
    /// space: `insert_space_after_tag_separator`).
    pub tag_separator: String,
    /// Whether to append a space after the tag separator (Python default:
    /// true, making the effective separator ", ").
    pub insert_space_after_tag_separator: bool,
    pub image_list_file_formats: String,
    pub image_list_image_width: u32,
    pub autocomplete_tags: bool,
    pub font_size: u32,
    pub danbooru_tags_csv_path: String,
    /// Saved image-list filters, name -> filter text.
    pub saved_filters: std::collections::BTreeMap<String, String>,
    /// Tag presets: reusable tag lists applied to images.
    pub tag_presets: Vec<TagPreset>,
    /// Directory where downloaded captioning models are stored.
    pub models_directory_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagPreset {
    pub name: String,
    pub tags: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tag_separator: String::from(","),
            insert_space_after_tag_separator: true,
            image_list_file_formats: String::from(
                "avif, bmp, gif, jpg, jpeg, png, tif, tiff, webp",
            ),
            image_list_image_width: 200,
            autocomplete_tags: true,
            font_size: 16,
            danbooru_tags_csv_path: String::new(),
            saved_filters: std::collections::BTreeMap::new(),
            tag_presets: Vec::new(),
            models_directory_path: String::new(),
        }
    }
}

impl Settings {
    /// The separator actually used to join/split tags: the separator plus
    /// an appended space when `insert_space_after_tag_separator` is set
    /// (Python's `get_tag_separator`).
    pub fn effective_tag_separator(&self) -> String {
        let mut separator = self.tag_separator.clone();
        if self.insert_space_after_tag_separator {
            separator.push(' ');
        }
        separator
    }

    pub fn load(path: &Path) -> Result<Self, toml::de::Error> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&text)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).expect("settings serialize as TOML");
        std::fs::write(path, text)
    }
}

/// The default config file path for this platform.
pub fn default_config_path() -> PathBuf {
    if let Some(dir) = dirs::config_dir() {
        return dir.join("taggui-rs").join("config.toml");
    }
    PathBuf::from("taggui-rs-config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_separator_matches_python_default() {
        let settings = Settings::default();
        assert_eq!(settings.effective_tag_separator(), ", ");
        let no_space = Settings {
            insert_space_after_tag_separator: false,
            ..Settings::default()
        };
        assert_eq!(no_space.effective_tag_separator(), ",");
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings::default().save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "font_size = 20\n").unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.font_size, 20);
        assert_eq!(loaded.tag_separator, ",");
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Settings::load(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(loaded, Settings::default());
    }
}
