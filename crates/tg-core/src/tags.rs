//! Tag list operations, ported from the Python app:
//! - `add_caption_to_tags` (`taggui/auto_captioning/captioning_thread.py`)
//! - find & replace (`image_list_model.py:277`): the replacement runs on
//!   the whole caption (tags joined with the separator), so it can cross
//!   tag boundaries; the result is re-split verbatim
//! - rename/delete tags (`image_list_model.py:509`, `:537`): whole-tag
//!   matching (exact equality, or `re.fullmatch` with regex)
//! - sort/reorder helpers (`taggui/dialogs/batch_reorder_tags_dialog.py`)

/// Where a generated caption is merged into the image's tags.
/// Mirrors `utils/enums.py::CaptionPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionPosition {
    BeforeFirstTag,
    AfterLastTag,
    OverwriteFirstTag,
    OverwriteAllTags,
    DoNotAdd,
}

impl CaptionPosition {
    /// The exact combo-box strings from the Python app, so user habits
    /// transfer (`list(CaptionPosition)` in the GUI).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BeforeFirstTag => "Insert before first tag",
            Self::AfterLastTag => "Insert after last tag",
            Self::OverwriteFirstTag => "Overwrite first tag",
            Self::OverwriteAllTags => "Overwrite all tags",
            Self::DoNotAdd => "Do not add to tags",
        }
    }
}

impl std::str::FromStr for CaptionPosition {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Insert before first tag" => Ok(Self::BeforeFirstTag),
            "Insert after last tag" => Ok(Self::AfterLastTag),
            "Overwrite first tag" => Ok(Self::OverwriteFirstTag),
            "Overwrite all tags" => Ok(Self::OverwriteAllTags),
            "Do not add to tags" => Ok(Self::DoNotAdd),
            _ => Err(()),
        }
    }
}

/// Merge caption tags into existing tags, ported 1:1 from
/// `add_caption_to_tags`. `caption` is split on the tag separator first,
/// exactly like Python (`caption.split(tag_separator)`).
pub fn add_caption_to_tags(
    tags: &[String],
    caption: &str,
    caption_position: CaptionPosition,
    tag_separator: &str,
) -> Vec<String> {
    if caption_position == CaptionPosition::DoNotAdd || caption.is_empty() {
        return tags.to_vec();
    }
    let caption_tags: Vec<String> = caption.split(tag_separator).map(str::to_owned).collect();
    let mut tags = tags.to_vec();
    match caption_position {
        CaptionPosition::BeforeFirstTag => {
            tags.splice(0..0, caption_tags);
        }
        CaptionPosition::AfterLastTag => {
            tags.extend(caption_tags);
        }
        CaptionPosition::OverwriteFirstTag => {
            if tags.is_empty() {
                tags = caption_tags;
            } else {
                tags.splice(0..1, caption_tags);
            }
        }
        CaptionPosition::OverwriteAllTags => {
            tags = caption_tags;
        }
        CaptionPosition::DoNotAdd => unreachable!(),
    }
    tags
}

/// Translate a Python `re.sub` replacement template to the Rust `regex`
/// crate's syntax, so filter templates written for the Python app keep
/// working: `\1`..`\9` become `$1`..`$9`, `\g<name>` becomes
/// `${name}`, `\\` becomes a literal backslash, and a literal `$`
/// (special in Rust, not in Python) is escaped as `$$`.
fn translate_python_replacement(replacement: &str) -> String {
    let mut result = String::with_capacity(replacement.len());
    let mut chars = replacement.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '\\' {
            if current == '$' {
                result.push_str("$$");
            } else {
                result.push(current);
            }
            continue;
        }
        match chars.next() {
            Some('\\') => result.push('\\'),
            Some(digit @ '0'..='9') => {
                result.push('$');
                result.push(digit);
            }
            Some('g') => {
                if chars.peek() == Some(&'<') {
                    chars.next();
                    let mut name = String::new();
                    for inner in chars.by_ref() {
                        if inner == '>' {
                            break;
                        }
                        name.push(inner);
                    }
                    result.push_str("${");
                    result.push_str(&name);
                    result.push('}');
                } else {
                    result.push_str("\\g");
                }
            }
            Some(other) => {
                // An escaped character in a Python template is a literal
                // (e.g. `\n` is a backreference-free literal only for
                // non-digits; Python actually turns `\n` into `n` for
                // unknown escapes - mirror that by dropping the backslash).
                if other == '$' {
                    result.push_str("$$");
                } else {
                    result.push(other);
                }
            }
            None => {} // a lone trailing backslash disappears (Python: error; be lenient)
        }
    }
    result
}

/// Find and replace arbitrary text in captions, within and across tag
/// boundaries. Port of `ImageListModel.find_and_replace`
/// (`image_list_model.py:277`): the tags are joined into a caption, the
/// replacement runs on the whole caption string, and the caption is
/// re-split on the separator (verbatim, no trimming - the Python app has
/// no case-insensitivity option either).
///
/// Returns whether anything changed.
pub fn find_and_replace_in_caption_mut(
    tags: &mut Vec<String>,
    find: &str,
    replace_with: &str,
    use_regex: bool,
    tag_separator: &str,
) -> bool {
    if find.is_empty() {
        return false;
    }
    let caption = tags.join(tag_separator);
    let new_caption = if use_regex {
        let Ok(regex) = regex::Regex::new(find) else {
            return false;
        };
        if !regex.is_match(&caption) {
            return false;
        }
        let replacement = translate_python_replacement(replace_with);
        regex
            .replace_all(&caption, replacement.as_str())
            .into_owned()
    } else {
        if !caption.contains(find) {
            return false;
        }
        caption.replace(find, replace_with)
    };
    let new_tags: Vec<String> = new_caption
        .split(tag_separator)
        .map(str::to_owned)
        .collect();
    if &new_tags == tags {
        return false;
    }
    *tags = new_tags;
    true
}

/// Rename whole tags. Port of `rename_tags` (`image_list_model.py:509`):
/// with regex, each tag that `fullmatch`es the pattern becomes `new_tag`;
/// without regex, each tag exactly equal to one of `old_tags` does.
/// Returns whether the image changed.
pub fn rename_tags_in_image(
    tags: &mut [String],
    old_tags: &[String],
    new_tag: &str,
    use_regex: bool,
) -> bool {
    let matches = |tag: &str| -> bool {
        if use_regex {
            regex::Regex::new(&old_tags[0])
                .map(|regex| regex.is_match_full(tag))
                .unwrap_or(false)
        } else {
            old_tags.iter().any(|old| old == tag)
        }
    };
    if !tags.iter().any(|tag| matches(tag)) {
        return false;
    }
    for tag in tags.iter_mut() {
        if matches(tag) {
            *tag = new_tag.to_owned();
        }
    }
    true
}

/// Delete whole tags. Port of `delete_tags` (`image_list_model.py:537`).
/// Returns whether the image changed.
pub fn delete_tags_in_image(tags: &mut Vec<String>, old_tags: &[String], use_regex: bool) -> bool {
    let matches = |tag: &str| -> bool {
        if use_regex {
            regex::Regex::new(&old_tags[0])
                .map(|regex| regex.is_match_full(tag))
                .unwrap_or(false)
        } else {
            old_tags.iter().any(|old| old == tag)
        }
    };
    if !tags.iter().any(|tag| matches(tag)) {
        return false;
    }
    tags.retain(|tag| !matches(tag));
    true
}

/// `re.fullmatch` equivalent: the pattern must match the entire string.
trait FullMatch {
    fn is_match_full(&self, text: &str) -> bool;
}

impl FullMatch for regex::Regex {
    fn is_match_full(&self, text: &str) -> bool {
        self.is_match(text) && self.find(text).map(|m| m.as_str() == text).unwrap_or(false)
    }
}

/// Batch sort a tags list (port of `batch_reorder_tags_dialog.py`): the
/// first tag (the class/caption tag) is always kept in front, the rest are
/// sorted or reversed.
pub fn sort_tags_keeping_first(tags: &[String], descending: bool) -> Vec<String> {
    if tags.is_empty() {
        return Vec::new();
    }
    let mut rest = tags[1..].to_vec();
    if descending {
        rest.sort_unstable_by(|a, b| b.cmp(a));
    } else {
        rest.sort_unstable();
    }
    let mut result = Vec::with_capacity(tags.len());
    result.push(tags[0].clone());
    result.extend(rest);
    result
}

pub fn reverse_tags_keeping_first(tags: &[String]) -> Vec<String> {
    if tags.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::with_capacity(tags.len());
    result.push(tags[0].clone());
    result.extend(tags[1..].iter().rev().cloned());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn add_caption_positions_match_python() {
        let tags = s(&["a", "b"]);
        let separator = ", ";
        let caption = "c, d";
        assert_eq!(
            add_caption_to_tags(&tags, caption, CaptionPosition::BeforeFirstTag, separator),
            s(&["c", "d", "a", "b"])
        );
        assert_eq!(
            add_caption_to_tags(&tags, caption, CaptionPosition::AfterLastTag, separator),
            s(&["a", "b", "c", "d"])
        );
        assert_eq!(
            add_caption_to_tags(
                &tags,
                caption,
                CaptionPosition::OverwriteFirstTag,
                separator
            ),
            s(&["c", "d", "b"])
        );
        assert_eq!(
            add_caption_to_tags(&tags, caption, CaptionPosition::OverwriteAllTags, separator),
            s(&["c", "d"])
        );
    }

    #[test]
    fn add_caption_edge_cases_match_python() {
        // Python: the empty caption short-circuits before the split.
        let tags = s(&["a", "b"]);
        assert_eq!(
            add_caption_to_tags(&tags, "", CaptionPosition::BeforeFirstTag, ", "),
            tags
        );
        assert_eq!(
            add_caption_to_tags(&tags, "c", CaptionPosition::DoNotAdd, ", "),
            tags
        );
        // Overwrite-first with no existing tags yields just the caption.
        assert_eq!(
            add_caption_to_tags(&[], "c, d", CaptionPosition::OverwriteFirstTag, ", "),
            s(&["c", "d"])
        );
        // The input list is not mutated.
        let original = tags.clone();
        let _ = add_caption_to_tags(&tags, "c", CaptionPosition::AfterLastTag, ", ");
        assert_eq!(tags, original);
    }

    #[test]
    fn caption_split_is_verbatim_like_python() {
        // Python does not trim caption parts: "a,b" with separator ", "
        // yields one tag "a,b".
        let result = add_caption_to_tags(&[], "a,b", CaptionPosition::AfterLastTag, ", ");
        assert_eq!(result, s(&["a,b"]));
    }

    #[test]
    fn find_and_replace_runs_on_joined_caption() {
        // Python joins the tags, replaces across the boundary, and
        // re-splits: ["big", "hat"] with find "g, h" -> "bi" + "at"
        // (the separator itself is replaceable!).
        let mut tags = s(&["big", "hat"]);
        assert!(find_and_replace_in_caption_mut(
            &mut tags, "g, h", "l", false, ", "
        ));
        assert_eq!(tags, s(&["bilat"]));
    }

    #[test]
    fn find_and_replace_plain_and_regex() {
        let mut tags = s(&["blue eyes", "blue hair"]);
        assert!(find_and_replace_in_caption_mut(
            &mut tags, "blue", "green", false, ", "
        ));
        assert_eq!(tags, s(&["green eyes", "green hair"]));

        // Python-style replacement template (`\1`), translated to the
        // Rust regex crate's `$1` internally. The pattern consumes the
        // underscore (`blue_`), so the result has none - same as Python's
        // re.sub.
        let mut tags = s(&["blue_eyes", "blue_hair"]);
        assert!(find_and_replace_in_caption_mut(
            &mut tags,
            "blue_(eyes|hair)",
            "green\\1",
            true,
            ", "
        ));
        assert_eq!(tags, s(&["greeneyes", "greenhair"]));
        // A literal `$` in a Python-style template stays a literal dollar
        // (the `$` is special in the Rust regex crate, so the translation
        // escapes it as `$$` internally).
        let mut tags = s(&["blue_eyes"]);
        assert!(find_and_replace_in_caption_mut(
            &mut tags,
            "blue_(eyes)",
            "cost: 5$",
            true,
            ", "
        ));
        assert_eq!(tags, s(&["cost: 5$"]));

        // No match -> unchanged, returns false.
        let mut tags = s(&["a", "b"]);
        assert!(!find_and_replace_in_caption_mut(
            &mut tags, "zzz", "q", false, ", "
        ));
        assert_eq!(tags, s(&["a", "b"]));
        // Empty find -> no-op (Python guards `if not find_text: return`).
        assert!(!find_and_replace_in_caption_mut(
            &mut tags, "", "q", false, ", "
        ));
    }

    #[test]
    fn rename_tags_exact_and_regex() {
        // Exact match: only whole equal tags are renamed.
        let mut tags = s(&["hat", "top hat", "hatter"]);
        assert!(rename_tags_in_image(&mut tags, &s(&["hat"]), "cap", false));
        assert_eq!(tags, s(&["cap", "top hat", "hatter"]));

        // Regex: `fullmatch` semantics.
        let mut tags = s(&["hat", "top hat", "hatter"]);
        assert!(rename_tags_in_image(&mut tags, &s(&["h.*r"]), "cap", true));
        assert_eq!(tags, s(&["hat", "top hat", "cap"]));

        // No match -> false.
        let mut tags = s(&["a"]);
        assert!(!rename_tags_in_image(&mut tags, &s(&["zzz"]), "q", false));
    }

    #[test]
    fn delete_tags_exact_and_regex() {
        let mut tags = s(&["hat", "top hat", "hatter"]);
        assert!(delete_tags_in_image(
            &mut tags,
            &s(&["hat", "top hat"]),
            false
        ));
        assert_eq!(tags, s(&["hatter"]));

        let mut tags = s(&["blue_eyes", "blue_hair", "solo"]);
        assert!(delete_tags_in_image(&mut tags, &s(&["blue_.*"]), true));
        assert_eq!(tags, s(&["solo"]));

        let mut tags = s(&["a"]);
        assert!(!delete_tags_in_image(&mut tags, &s(&["zzz"]), false));
    }

    #[test]
    fn full_match_requires_complete_string() {
        // `h.*r` must NOT fullmatch "hatter" here because the pattern is
        // anchored to the whole string; it DOES match "hatter" entirely.
        let regex = regex::Regex::new("h.*r").unwrap();
        assert!(regex.is_match_full("hatter"));
        assert!(!regex.is_match_full("hat")); // matches prefix only
        let regex = regex::Regex::new("top").unwrap();
        assert!(!regex.is_match_full("top hat"));
    }

    #[test]
    fn sort_and_reverse_keep_first() {
        let tags = s(&["1girl", "zebra", "apple", "mango"]);
        assert_eq!(
            sort_tags_keeping_first(&tags, false),
            s(&["1girl", "apple", "mango", "zebra"])
        );
        assert_eq!(
            sort_tags_keeping_first(&tags, true),
            s(&["1girl", "zebra", "mango", "apple"])
        );
        assert_eq!(
            reverse_tags_keeping_first(&tags),
            s(&["1girl", "mango", "apple", "zebra"])
        );
        assert_eq!(sort_tags_keeping_first(&[], false), Vec::<String>::new());
    }
}
