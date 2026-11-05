//! Sidecar `.txt` caption files, ported byte-for-byte from
//! `taggui/models/image_list_model.py`:
//!
//! Read  (line 157): `read_text(encoding='utf-8', errors='replace')`,
//!        split on `tag_separator`, strip each tag, drop empties.
//! Write (line 184): `write_text(tag_separator.join(tags),
//!        encoding='utf-8', errors='replace')` - joined with the
//!        separator, **no trailing newline**.

use std::fs;
use std::path::Path;

use crate::image_entry::with_txt_suffix;

/// Errors that can occur while reading or writing a sidecar file.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: Box<Path>,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: Box<Path>,
        source: std::io::Error,
    },
}

/// Parse caption text into tags exactly like the Python app: split on the
/// separator, strip whitespace from each tag, drop empty tags.
///
/// Invalid UTF-8 becomes U+FFFD replacement characters, matching Python's
/// `errors='replace'`.
pub fn parse_tags(caption: &str, tag_separator: &str) -> Vec<String> {
    caption
        .split(tag_separator)
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Read a sidecar `.txt` file and parse it into tags.
pub fn read_tags(image_path: &Path, tag_separator: &str) -> Result<Vec<String>, SidecarError> {
    let caption_path = with_txt_suffix(image_path);
    let bytes = fs::read(&caption_path).map_err(|source| SidecarError::Read {
        path: caption_path.clone().into_boxed_path(),
        source,
    })?;
    // `String::from_utf8_lossy` inserts U+FFFD for invalid bytes, the same
    // replacement marker Python's `errors='replace'` inserts.
    let caption = String::from_utf8_lossy(&bytes);
    Ok(parse_tags(&caption, tag_separator))
}

/// Write tags to the sidecar `.txt` file, joined with the separator and
/// with no trailing newline - byte-identical to the Python app's
/// `write_text(self.tag_separator.join(image.tags), encoding='utf-8')`.
pub fn write_tags(
    image_path: &Path,
    tags: &[String],
    tag_separator: &str,
) -> Result<(), SidecarError> {
    let caption_path = with_txt_suffix(image_path);
    let caption = tags.join(tag_separator);
    fs::write(&caption_path, caption.as_bytes()).map_err(|source| SidecarError::Write {
        path: caption_path.clone().into_boxed_path(),
        source,
    })
}

/// Whether a sidecar `.txt` file exists for the given image path.
pub fn has_caption_file(image_path: &Path) -> bool {
    with_txt_suffix(image_path).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn parses_tags_with_default_separator() {
        let tags = parse_tags("1girl, solo, long hair", ", ");
        assert_eq!(tags, ["1girl", "solo", "long hair"]);
    }

    #[test]
    fn strips_and_drops_empty_tags() {
        // Matches Python: `split` then `strip` then drop falsy. The part
        // 'b  ,' keeps its trailing comma - a comma is not whitespace, so
        // the strip leaves it (Python behaves identically).
        let tags = parse_tags("  a , , b  ,, c ", ", ");
        assert_eq!(tags, ["a", "b  ,", "c"]);
    }

    #[test]
    fn parses_tags_with_custom_separator() {
        let tags = parse_tags("a; b", "; ");
        assert_eq!(tags, ["a", "b"]);
    }

    #[test]
    fn round_trips_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("img.png");
        write_file(&image_path, b"png");
        let tags = vec!["1girl".to_string(), "solo".to_string()];
        write_tags(&image_path, &tags, ", ").unwrap();
        let raw = fs::read(image_path.with_extension("txt")).unwrap();
        // Byte-exact: joined with ', ', no trailing newline.
        assert_eq!(raw, b"1girl, solo");
        assert_eq!(read_tags(&image_path, ", ").unwrap(), tags);
    }

    #[test]
    fn reading_replacement_markers_survive() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("img.png");
        // Invalid UTF-8 byte -> both implementations show U+FFFD.
        write_file(&image_path.with_extension("txt"), b"bad \xff tag, ok");
        let tags = read_tags(&image_path, ", ").unwrap();
        assert_eq!(tags, ["bad \u{fffd} tag", "ok"]);
    }

    #[test]
    fn empty_sidecar_yields_no_tags() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("img.png");
        write_file(&image_path.with_extension("txt"), b"");
        assert_eq!(read_tags(&image_path, ", ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn tags_containing_separator_survive_a_write_read_cycle_only_verbatim() {
        // The Python app has the same behavior: a tag containing the
        // separator is written verbatim and re-read as two tags. Document
        // it as a test so the Rust port never "fixes" it silently.
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("img.png");
        write_file(&image_path, b"png");
        let tags = vec!["a, b".to_string()];
        write_tags(&image_path, &tags, ", ").unwrap();
        assert_eq!(read_tags(&image_path, ", ").unwrap(), ["a", "b"]);
    }
}
