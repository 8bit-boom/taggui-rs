//! tg-core: the data layer for taggui-rs, ported from the Python TagGUI
//! (`jhc13/taggui` fork `8bit-boom/taggui`). Everything in this crate is
//! byte-parity-tested against the Python app's behavior via golden
//! fixtures - see PORT_GUIDE.md at the repository root.

pub mod filter;
pub mod image_entry;
pub mod settings;
pub mod sidecar;
pub mod stats;
pub mod tags;

/// Scan a directory for images and load their sidecar tags, ported from
/// `ImageListModel.load_directory` (`image_list_model.py`): images sorted
/// by path, tags parsed from the sidecar file, dimensions and file sizes
/// read where possible.
pub fn scan_directory(
    directory_path: &std::path::Path,
    file_formats: &str,
    tag_separator: &str,
) -> Result<Vec<image_entry::ImageEntry>, std::io::Error> {
    let suffixes: Vec<String> = file_formats
        .split(',')
        .map(|format| format.trim().to_lowercase())
        .filter(|format| !format.is_empty())
        .collect();
    let mut entries = Vec::new();
    for dir_entry in std::fs::read_dir(directory_path)? {
        let dir_entry = dir_entry?;
        let path = dir_entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase());
        let Some(extension) = extension else {
            continue;
        };
        if !suffixes.contains(&extension) {
            continue;
        }
        // Read the image header for dimensions (this is cheap with the
        // `image` crate - it only decodes what the format needs... for
        // most formats it decodes everything; the GUI does this in a
        // background thread pool).
        let dimensions = image::image_dimensions(&path).ok();
        let file_size = std::fs::metadata(&path).ok().map(|meta| meta.len());
        let has_caption_file = sidecar::has_caption_file(&path);
        let tags = if has_caption_file {
            sidecar::read_tags(&path, tag_separator).unwrap_or_default()
        } else {
            Vec::new()
        };
        entries.push(image_entry::ImageEntry {
            path,
            dimensions,
            tags,
            file_size,
            has_caption_file,
            thumbnail: None,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_reads_sidecars_and_sorts_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        let c = dir.path().join("c.txt"); // not an image
        let d = dir.path().join("d.webp");
        fs::write(&a, []).unwrap();
        fs::write(&b, []).unwrap();
        fs::write(&c, "ignored").unwrap();
        fs::write(&d, []).unwrap();
        fs::write(a.with_extension("txt"), "1girl, solo").unwrap();
        let entries = scan_directory(dir.path(), "png, webp", ", ").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path.file_name().unwrap(), "a.png");
        assert_eq!(entries[0].tags, ["1girl", "solo"]);
        assert!(entries[0].has_caption_file);
        assert!(!entries[1].has_caption_file);
        assert!(entries[1].tags.is_empty());
        assert_eq!(entries[2].path.file_name().unwrap(), "d.webp");
    }

    #[test]
    fn scan_case_insensitive_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.JPG"), []).unwrap();
        let entries = scan_directory(dir.path(), "jpg", ", ").unwrap();
        assert_eq!(entries.len(), 1);
    }
}
