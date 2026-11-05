//! Image list filter grammar and evaluation, ported from
//! `taggui/widgets/image_list.py` (the pyparsing grammar) and
//! `taggui/models/proxy_image_list_model.py` (the evaluation).
//!
//! Grammar (case-insensitive keys/keywords, precedence NOT > AND > OR,
//! parenthesized groups allowed, operators must be explicit - the Python
//! pyparsing `infix_notation` inserts no implicit ANDs):
//!
//! ```text
//! filter      := or_expr
//! or_expr     := and_expr (OR and_expr)*
//! and_expr    := not_expr (AND not_expr)*
//! not_expr    := NOT not_expr | primary
//! primary     := '(' or_expr ')
//!              | string_filter ('tag|caption|name|path|ext' ':' value)
//!              | number_filter ('tags|chars|tokens|width|height|area|mp|
//!                                ratio|size' op number)
//!              | bare string
//! value       := quoted ('"' or "'" with backslash escapes) | bare string
//! number      := digits [ '.' digits ] [ whitespace ] [kb|mb|gb]
//! ```
//!
//! A bare string matches if the fnmatch pattern `*string*` matches the
//! caption (tags joined with the separator) or the image path.

const STRING_FILTER_KEYS: [&str; 5] = ["tag", "caption", "name", "path", "ext"];
const NUMBER_FILTER_KEYS: [&str; 9] = [
    "tags", "chars", "tokens", "width", "height", "area", "mp", "ratio", "size",
];

/// A parsed filter. The shape mirrors what the Python evaluation expects
/// from its nested-list AST, but as an enum.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Bare string: `fnmatch(caption, f'*{s}*')` or `fnmatch(path, f'*{s}*')`.
    Contains(String),
    Tag(String),
    Caption(String),
    Name(String),
    Path(String),
    Ext(String),
    Not(Box<Filter>),
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
    Compare {
        key: NumberKey,
        op: CompareOp,
        value: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKey {
    Tags,
    Chars,
    Tokens,
    Width,
    Height,
    Area,
    Mp,
    Ratio,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// The CLIP token counter used by the `tokens:` filter. In Python this is
/// the bundled `clip-vit-base-patch32` tokenizer; the count is the number
/// of tokens minus 2 (for `<|startoftext|>`/`<|endoftext|>`).
pub struct TokenCounter {
    tokenizer: tokenizers::Tokenizer,
}

impl TokenCounter {
    /// Load the bundled CLIP tokenizer embedded in this crate.
    pub fn bundled() -> Result<Self, tokenizers::Error> {
        let tokenizer =
            tokenizers::Tokenizer::from_bytes(include_bytes!("../assets/clip_tokenizer.json"))?;
        Ok(Self { tokenizer })
    }

    pub fn from_file(path: &std::path::Path) -> Result<Self, tokenizers::Error> {
        Ok(Self {
            tokenizer: tokenizers::Tokenizer::from_file(path)?,
        })
    }

    /// The token count used by the `tokens:` filter (total minus 2).
    pub fn token_count(&self, caption: &str) -> usize {
        self.tokenizer
            .encode(caption, false)
            .map(|encoding| encoding.get_ids().len().saturating_sub(2))
            .unwrap_or(0)
    }
}

/// The inputs a filter evaluates against: the relevant fields of an
/// `ImageEntry` plus the tag separator and optional token counter.
pub struct FilterInput<'a> {
    pub path: &'a std::path::Path,
    pub dimensions: Option<(u32, u32)>,
    pub file_size: Option<u64>,
    pub tags: &'a [String],
    pub tag_separator: &'a str,
    pub token_counter: Option<&'a TokenCounter>,
}

impl Filter {
    /// Parse filter text; `None` on invalid input (the Python GUI turns
    /// the line edit red and ignores the filter).
    pub fn parse(text: &str) -> Option<Filter> {
        if text.trim().is_empty() {
            return None;
        }
        let mut parser = Parser::new(text);
        let filter = parser.parse_or();
        // `parse_all=True` in Python: trailing garbage is an error.
        if filter.is_none() || !parser.at_end() {
            return None;
        }
        filter
    }

    /// Evaluate against one image.
    pub fn matches(&self, input: &FilterInput<'_>) -> bool {
        match self {
            Self::Contains(pattern) => {
                let caption = input.tags.join(input.tag_separator);
                // Python wraps bare strings as `*pattern*` for both the
                // caption and the path.
                let caption_pattern = format!("*{pattern}*");
                let path_pattern = format!("*{pattern}*");
                fnmatch_case(&caption_pattern, &caption)
                    || fnmatch_case(&path_pattern, &input.path.to_string_lossy())
            }
            Self::Tag(pattern) => input.tags.iter().any(|tag| fnmatch_case(pattern, tag)),
            Self::Caption(pattern) => {
                let caption = input.tags.join(input.tag_separator);
                fnmatch_case(&format!("*{pattern}*"), &caption)
            }
            Self::Name(pattern) => {
                let name = input
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                fnmatch_case(&format!("*{pattern}*"), &name)
            }
            Self::Path(pattern) => {
                fnmatch_case(&format!("*{pattern}*"), &input.path.to_string_lossy())
            }
            Self::Ext(pattern) => {
                let extension = input
                    .path
                    .extension()
                    .map(|ext| ext.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .to_lowercase();
                fnmatch_case(&pattern.to_lowercase(), &extension)
            }
            Self::Not(inner) => !inner.matches(input),
            Self::And(left, right) => left.matches(input) && right.matches(input),
            Self::Or(left, right) => left.matches(input) || right.matches(input),
            Self::Compare { key, op, value } => {
                let number_to_compare = get_comparison_number(input, *key);
                match number_to_compare {
                    // A missing number (no dimensions, unknown size) never
                    // matches, like Python returning False on None.
                    None => false,
                    Some(number) => match op {
                        CompareOp::Eq => number == *value,
                        CompareOp::Ne => number != *value,
                        CompareOp::Lt => number < *value,
                        CompareOp::Gt => number > *value,
                        CompareOp::Le => number <= *value,
                        CompareOp::Ge => number >= *value,
                    },
                }
            }
        }
    }
}

fn get_comparison_number(input: &FilterInput<'_>, key: NumberKey) -> Option<f64> {
    match key {
        NumberKey::Tags => Some(input.tags.len() as f64),
        NumberKey::Chars => Some(input.tags.join(input.tag_separator).chars().count() as f64),
        NumberKey::Tokens => {
            let caption = input.tags.join(input.tag_separator);
            Some(input.token_counter?.token_count(&caption) as f64)
        }
        NumberKey::Size => input.file_size.map(|size| size as f64),
        NumberKey::Width => input.dimensions.map(|(width, _)| width as f64),
        NumberKey::Height => input.dimensions.map(|(_, height)| height as f64),
        NumberKey::Area => input
            .dimensions
            .map(|(width, height)| (width as f64) * (height as f64)),
        NumberKey::Mp => input
            .dimensions
            .map(|(width, height)| (width as f64) * (height as f64) / 1_000_000.0),
        NumberKey::Ratio => match input.dimensions {
            Some((width, height)) if height != 0 => Some(width as f64 / height as f64),
            _ => None,
        },
    }
}

/// Parse a filter value, stripping an optional `kb`/`mb`/`gb` suffix.
/// Decimal multipliers (1000/1e6/1e9), exactly like Python - NOT 1024.
pub fn parse_filter_number(text: &str) -> Option<f64> {
    let text = text.trim().to_lowercase();
    let multipliers = [
        ("kb", 1_000.0),
        ("mb", 1_000_000.0),
        ("gb", 1_000_000_000.0),
    ];
    for (suffix, multiplier) in multipliers {
        if let Some(stripped) = text.strip_suffix(suffix) {
            return stripped.trim().parse::<f64>().ok().map(|n| n * multiplier);
        }
    }
    let text = text.strip_suffix('b').map(str::trim).unwrap_or(&text);
    text.parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Parser (hand-written recursive descent; precedence NOT > AND > OR,
// left-associative AND/OR, parenthesized groups).
// ---------------------------------------------------------------------------

struct Parser<'a> {
    chars: Vec<char>,
    position: usize,
    #[allow(dead_code)]
    source: &'a str,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            position: 0,
            source,
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_whitespace();
        self.position >= self.chars.len()
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.chars.len() && self.chars[self.position].is_whitespace() {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn starts_with_ignore_case(&self, keyword: &str) -> bool {
        let keyword_chars: Vec<char> = keyword.chars().collect();
        if self.position + keyword_chars.len() > self.chars.len() {
            return false;
        }
        keyword_chars
            .iter()
            .zip(&self.chars[self.position..])
            .all(|(expected, actual)| expected.to_lowercase().eq(actual.to_lowercase()))
    }

    /// A keyword token only matches when not followed by more identifier
    /// characters (`note:` must not parse as `NOT` + `e:`).
    fn consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        if !self.starts_with_ignore_case(keyword) {
            return false;
        }
        let end = self.position + keyword.chars().count();
        if let Some(next) = self.chars.get(end) {
            if next.is_alphanumeric() || *next == '_' {
                return false;
            }
        }
        self.position = end;
        true
    }

    fn parse_or(&mut self) -> Option<Filter> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_whitespace();
            if self.consume_keyword("OR") {
                let right = self.parse_and()?;
                left = Filter::Or(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_and(&mut self) -> Option<Filter> {
        let mut left = self.parse_not()?;
        loop {
            self.skip_whitespace();
            if self.consume_keyword("AND") {
                let right = self.parse_not()?;
                left = Filter::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_not(&mut self) -> Option<Filter> {
        self.skip_whitespace();
        if self.consume_keyword("NOT") {
            let inner = self.parse_not()?;
            return Some(Filter::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<Filter> {
        self.skip_whitespace();
        match self.peek() {
            Some('(') => {
                self.position += 1;
                let inner = self.parse_or()?;
                self.skip_whitespace();
                if self.peek() != Some(')') {
                    return None;
                }
                self.position += 1;
                Some(inner)
            }
            Some(_) => self.parse_atom(),
            None => None,
        }
    }

    /// One of the `key: value` filters or a bare string.
    fn parse_atom(&mut self) -> Option<Filter> {
        // Try to read an identifier followed by ':' to detect a keyed
        // filter; otherwise parse a bare string.
        let save = self.position;
        if let Some((key, key_end)) = self.try_read_key() {
            self.position = key_end;
            self.skip_whitespace();
            if self.peek() == Some(':') {
                self.position += 1;
                self.skip_whitespace();
                if STRING_FILTER_KEYS.contains(&key.as_str()) {
                    let value = self.read_bare_or_quoted_string()?;
                    return match key.as_str() {
                        "tag" => Some(Filter::Tag(value)),
                        "caption" => Some(Filter::Caption(value)),
                        "name" => Some(Filter::Name(value)),
                        "path" => Some(Filter::Path(value)),
                        "ext" => Some(Filter::Ext(value)),
                        _ => None,
                    };
                }
                if NUMBER_FILTER_KEYS.contains(&key.as_str()) {
                    let op = self.read_compare_operator()?;
                    self.skip_whitespace();
                    let number_text_start = self.position;
                    let number_text = self.read_number_literal()?;
                    let _ = number_text_start;
                    let value = parse_filter_number(&number_text)?;
                    let key = match key.as_str() {
                        "tags" => NumberKey::Tags,
                        "chars" => NumberKey::Chars,
                        "tokens" => NumberKey::Tokens,
                        "width" => NumberKey::Width,
                        "height" => NumberKey::Height,
                        "area" => NumberKey::Area,
                        "mp" => NumberKey::Mp,
                        "ratio" => NumberKey::Ratio,
                        "size" => NumberKey::Size,
                        _ => return None,
                    };
                    return Some(Filter::Compare { key, op, value });
                }
            }
            // Not a keyed filter after all: restore and fall through to a
            // bare string starting here.
            self.position = save;
        }
        let text = self.read_bare_or_quoted_string()?;
        Some(Filter::Contains(text))
    }

    fn try_read_key(&self) -> Option<(String, usize)> {
        let mut end = self.position;
        while end < self.chars.len()
            && (self.chars[end].is_alphanumeric() || self.chars[end] == '_')
        {
            end += 1;
        }
        if end == self.position {
            return None;
        }
        let key: String = self.chars[self.position..end]
            .iter()
            .collect::<String>()
            .to_lowercase();
        Some((key, end))
    }

    fn read_compare_operator(&mut self) -> Option<CompareOp> {
        self.skip_whitespace();
        let rest: String = self.chars[self.position..].iter().collect();
        for (text, op) in [
            ("==", CompareOp::Eq),
            ("!=", CompareOp::Ne),
            ("<=", CompareOp::Le),
            (">=", CompareOp::Ge),
            ("=", CompareOp::Eq),
            ("<", CompareOp::Lt),
            (">", CompareOp::Gt),
        ] {
            if rest.starts_with(text) {
                self.position += text.chars().count();
                return Some(op);
            }
        }
        None
    }

    /// A number with optional decimal point and optional whitespace before
    /// an optional kb/mb/gb suffix (`Regex(r'\d+(\.\d+)?\s*(?i:[kmg]?b)?')`
    /// in Python).
    fn read_number_literal(&mut self) -> Option<String> {
        let start = self.position;
        let mut seen_digit = false;
        while let Some(current) = self.peek() {
            if current.is_ascii_digit() {
                seen_digit = true;
                self.position += 1;
            } else if current == '.' {
                self.position += 1;
            } else {
                break;
            }
        }
        if !seen_digit {
            return None;
        }
        // Optional whitespace before the suffix.
        self.skip_whitespace();
        if let Some(current) = self.peek() {
            if matches!(current.to_ascii_lowercase(), 'k' | 'm' | 'g' | 'b') {
                let suffix_start = self.position;
                if self.starts_with_ignore_case("kb")
                    || self.starts_with_ignore_case("mb")
                    || self.starts_with_ignore_case("gb")
                {
                    self.position += 2;
                } else if current.eq_ignore_ascii_case(&'b') {
                    self.position += 1;
                }
                let _ = suffix_start;
            }
        }
        Some(self.chars[start..self.position].iter().collect())
    }

    /// `QuotedString('"' | ''', esc_char='\\') | Word(printables minus
    /// parens)`. A quoted string is one token even with spaces; a bare
    /// string ends at whitespace or an opening/closing parenthesis. An
    /// unterminated quote falls back to a bare-string read starting at the
    /// quote character (Python's QuotedString simply fails and the
    /// bare-string alternative matches instead).
    fn read_bare_or_quoted_string(&mut self) -> Option<String> {
        self.skip_whitespace();
        match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                let quote_start = self.position;
                self.position += 1;
                let mut result = String::new();
                loop {
                    match self.peek() {
                        Some('\\') => {
                            self.position += 1;
                            let escaped = self.peek()?;
                            result.push(escaped);
                            self.position += 1;
                        }
                        Some(current) if current == quote => {
                            self.position += 1;
                            return Some(result);
                        }
                        Some(current) => {
                            result.push(current);
                            self.position += 1;
                        }
                        None => {
                            // Unterminated: retry as a bare string from the
                            // quote character.
                            self.position = quote_start;
                            return self.read_bare_string();
                        }
                    }
                }
            }
            Some(_) => self.read_bare_string(),
            None => None,
        }
    }

    fn read_bare_string(&mut self) -> Option<String> {
        let start = self.position;
        while let Some(current) = self.peek() {
            if current.is_whitespace() || current == '(' || current == ')' {
                break;
            }
            self.position += 1;
        }
        if start == self.position {
            return None;
        }
        Some(self.chars[start..self.position].iter().collect())
    }
}

// ---------------------------------------------------------------------------
// fnmatch (port of `fnmatch.fnmatchcase`).
// ---------------------------------------------------------------------------

/// Translate an fnmatch pattern (`*`, `?`, `[seq]`, `[!seq]`) into a regex.
/// Python's `fnmatch.translate` also treats `\*`/`\?` escapes via the
/// caller's wildcard replacement (`\*` -> `[*]` is done by the caller in
/// Python, not here).
fn fnmatch_pattern_to_regex(pattern: &str) -> String {
    let mut regex = String::with_capacity(pattern.len() * 2);
    regex.push('^');
    let mut chars = pattern.chars().peekable();
    while let Some(current) = chars.next() {
        match current {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '[' => {
                regex.push('[');
                if chars.peek() == Some(&'!') {
                    chars.next();
                    regex.push('^');
                } else if chars.peek() == Some(&']') {
                    // `[]...]`: a literal `]` as the first item.
                    chars.next();
                    regex.push_str("\\]");
                }
                let mut closed = false;
                for inner in chars.by_ref() {
                    regex.push(inner);
                    if inner == ']' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    // Unterminated class: literal bracket, like Python.
                    regex.pop();
                    regex.push_str("\\[");
                }
            }
            other => regex.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex.push('$');
    regex
}

/// `fnmatchcase`: case-sensitive glob matching.
pub fn fnmatch_case(pattern: &str, text: &str) -> bool {
    let regex_pattern = fnmatch_pattern_to_regex(pattern);
    regex::Regex::new(&regex_pattern)
        .map(|regex| regex.is_match(text))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tags: &[&str], dimensions: Option<(u32, u32)>) -> FilterInput<'static> {
        let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
        let path = std::path::PathBuf::from("/ds/img001.png");
        let tag_separator = String::from(", ");
        // Leak the inputs: `FilterInput` borrows everything, and these
        // tests are short-lived one-shot processes anyway.
        FilterInput {
            path: Box::leak(Box::new(path)),
            dimensions,
            file_size: Some(500_000),
            tags: Box::leak(Box::new(tags)),
            tag_separator: Box::leak(Box::new(tag_separator)),
            token_counter: None,
        }
    }

    #[test]
    fn fnmatch_basics() {
        assert!(fnmatch_case("cat", "cat"));
        assert!(fnmatch_case("c*t", "cat"));
        assert!(fnmatch_case("c?t", "cat"));
        assert!(!fnmatch_case("c?t", "cats"));
        assert!(fnmatch_case("[cp]at", "cat"));
        assert!(fnmatch_case("[!c]at", "bat"));
        assert!(!fnmatch_case("[!c]at", "cat"));
        assert!(fnmatch_case("*.png", "img001.png"));
        assert!(!fnmatch_case("*.png", "img001.jpg"));
    }

    #[test]
    fn parse_filter_number_matches_python() {
        assert_eq!(parse_filter_number("500"), Some(500.0));
        assert_eq!(parse_filter_number("1.5"), Some(1.5));
        assert_eq!(parse_filter_number("500kb"), Some(500_000.0));
        assert_eq!(parse_filter_number("  2 MB "), Some(2_000_000.0));
        assert_eq!(parse_filter_number("1gb"), Some(1_000_000_000.0));
        // Decimal multipliers, and a bare 'b' suffix is just stripped.
        assert_eq!(parse_filter_number("500b"), Some(500.0));
        assert_eq!(parse_filter_number("nope"), None);
    }

    #[test]
    fn parses_and_evaluates_number_filters() {
        let filter = Filter::parse("width:>100").unwrap();
        assert_eq!(
            filter,
            Filter::Compare {
                key: NumberKey::Width,
                op: CompareOp::Gt,
                value: 100.0
            }
        );
        let input = entry(&["a"], Some((800, 600)));
        assert!(filter.matches(&input));
        let filter = Filter::parse("size:>=500kb").unwrap();
        assert!(filter.matches(&input)); // 500_000 == 500kb * 1000
        let filter = Filter::parse("size:>500001").unwrap();
        assert!(!filter.matches(&input));
        let filter = Filter::parse("ratio:<2").unwrap();
        assert!(filter.matches(&input)); // 800/600 = 1.33
        let filter = Filter::parse("mp:>=0.4").unwrap();
        assert!(filter.matches(&input)); // 0.48
        let filter = Filter::parse("tags:>1").unwrap();
        assert!(!filter.matches(&input)); // 1 tag
        let filter = Filter::parse("tags:>=1").unwrap();
        assert!(filter.matches(&input));
        // Missing dimensions never match dimension filters.
        let input = entry(&["a"], None);
        let filter = Filter::parse("width:>10").unwrap();
        assert!(!filter.matches(&input));
    }

    #[test]
    fn parses_and_evaluates_string_filters() {
        let input = entry(&["1girl", "blue eyes"], Some((100, 100)));
        // A bare value cannot contain spaces; Python fails to parse the
        // whole filter because of the trailing 'eyes' token.
        assert!(Filter::parse("tag:blue eyes").is_none());
        let filter = Filter::parse("tag:\"blue eyes\"").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("caption:blue").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("name:img*").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("path:/ds/*.png").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("ext:PNG").unwrap();
        assert!(filter.matches(&input)); // ext is case-insensitive
        let filter = Filter::parse("ext:jpg").unwrap();
        assert!(!filter.matches(&input));
    }

    #[test]
    fn parses_boolean_combinations() {
        let input = entry(&["1girl", "blue eyes"], Some((800, 600)));
        let filter = Filter::parse("tag:1girl AND width:>100").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("tag:1boy OR tag:1girl").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("NOT tag:1boy AND tag:1girl").unwrap();
        // NOT binds tighter than AND.
        assert!(filter.matches(&input));
        let filter = Filter::parse("(tag:1boy OR tag:1girl) AND width:>700").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("(tag:1boy OR tag:1girl) AND width:>900").unwrap();
        assert!(!filter.matches(&input));
    }

    #[test]
    fn invalid_filters_are_rejected() {
        assert!(Filter::parse("").is_none());
        assert!(Filter::parse("   ").is_none());
        assert!(Filter::parse("width >").is_none()); // missing value
        assert!(Filter::parse("width > abc").is_none());
        // An unknown key is not a keyed filter; like Python, it parses as
        // one bare-string token (printables minus parens).
        assert_eq!(
            Filter::parse("unknown:foo"),
            Some(Filter::Contains("unknown:foo".into()))
        );
        // Trailing garbage is an error (parse_all=True).
        assert!(Filter::parse("width:>100 zzz").is_none());
        // Juxtaposition without an explicit AND is an error in Python
        // (pyparsing infix_notation has no implicit AND).
        assert!(Filter::parse("tag:a tag:b").is_none());
        // An unterminated quote falls back to a bare-string token that
        // includes the quote character (Python: QuotedString fails, the
        // bare-string alternative matches).
        assert_eq!(
            Filter::parse("tag:unterminated\""),
            Some(Filter::Tag("unterminated\"".into()))
        );
    }

    #[test]
    fn bare_string_matches_caption_or_path_substring() {
        let input = entry(&["1girl", "blue eyes"], Some((100, 100)));
        let filter = Filter::parse("blue").unwrap();
        assert!(filter.matches(&input));
        let filter = Filter::parse("img001").unwrap(); // matches the path
        assert!(filter.matches(&input));
        let filter = Filter::parse("zzz").unwrap();
        assert!(!filter.matches(&input));
    }
}
