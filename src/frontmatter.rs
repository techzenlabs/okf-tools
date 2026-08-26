//! Frontmatter parsing.
//!
//! Two parsers live here on purpose, because the Python tools this crate ports
//! carry two and they disagree in ways the generated output depends on:
//!
//! * [`parse_strict`] is `okf-check`'s validating parser. It reports the
//!   malformed blocks a conformance check cares about (duplicate keys, tab
//!   indentation, a list item before any key) and refuses the file.
//! * [`parse_lenient`] is `okf-index`'s extractor. It reports nothing, takes
//!   the last value for a repeated key, and ignores a key whose value is
//!   empty.
//!
//! Collapsing them would change generated output on a real corpus, so they
//! stay separate. Both are a strict YAML *subset*: a key is only a key at
//! column zero, which is what keeps nested mappings from being read as
//! top-level ones.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

/// Compile a pattern that is a literal in this crate's source.
///
/// Threading a `Result` out of every `LazyLock` buys nothing: a malformed
/// literal is an authoring bug, and [`tests::every_pattern_compiles`] forces
/// each one so it surfaces as a failing test rather than at a user's first run.
#[expect(
    clippy::expect_used,
    reason = "static pattern literals, all forced by every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// The frontmatter fence, as `okf-check` matches it: anchored at the start of
/// the file, with the trailing newline optional.
static FENCE_OPTIONAL_NL: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n?"));

/// The same fence as `okf-index` matches it. The trailing newline is
/// *required* here, which is the one difference between the two, and it
/// decides whether a file with no body is treated as having frontmatter.
static FENCE_REQUIRED_NL: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n"));

/// A top-level key. Anchored at column zero, so an indented key is a
/// continuation line rather than a key of its own.
static KEY_LINE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"^([A-Za-z0-9_][\w.\-]*):[ \t]*(.*)$"));

/// A block-sequence item.
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| compiled(r"^[ \t]*-[ \t]+"));

/// One entry of a parsed block.
///
/// A sequence is stored under `"<key>[]"` rather than under `<key>`, which is
/// what the Python does and what keeps a sequence from colliding with a scalar
/// of the same name during duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Scalar(String),
    List(Vec<String>),
}

/// A parsed frontmatter block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    entries: BTreeMap<String, Entry>,
}

impl Frontmatter {
    /// The raw text of a scalar key, exactly as it appeared after the colon.
    ///
    /// Returns `None` for a sequence, matching the Python, where a sequence
    /// lives under a different key entirely.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        match self.entries.get(key) {
            Some(Entry::Scalar(v)) => Some(v.as_str()),
            _ => None,
        }
    }

    /// The scalar at `key`, unquoted, or the empty string when absent.
    ///
    /// This is the shape almost every caller wants, and it collapses the
    /// Python's `unquote(fm.get(k, ""))` into one call.
    #[must_use]
    pub fn get_unquoted(&self, key: &str) -> String {
        unquote(self.get(key).unwrap_or_default())
    }

    /// Every key present, including the `"<key>[]"` form for sequences.
    ///
    /// Sorted, because the only caller reports unexpected keys and the report
    /// has to be stable.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    fn insert_scalar(&mut self, key: &str, value: &str) {
        self.entries
            .insert(key.to_owned(), Entry::Scalar(value.to_owned()));
    }

    fn contains_scalar(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(Entry::Scalar(_)))
    }

    fn push_list_item(&mut self, key: &str, item: &str) {
        let slot = format!("{key}[]");
        match self
            .entries
            .entry(slot)
            .or_insert_with(|| Entry::List(Vec::new()))
        {
            Entry::List(items) => items.push(item.to_owned()),
            Entry::Scalar(_) => {}
        }
    }
}

/// Why a block was refused by [`parse_strict`].
///
/// Each variant carries the 1-based line number the Python reports, which
/// counts the opening fence as line 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The file does not open with a frontmatter fence.
    NoFence,
    TabIndentation {
        line: usize,
    },
    DuplicateKey {
        line: usize,
        key: String,
    },
    ListItemBeforeKey {
        line: usize,
    },
    NotAKey {
        line: usize,
    },
}

impl ParseError {
    /// The diagnostic text, byte-for-byte as the Python emits it.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoFence => "no YAML frontmatter (§11.1)".to_owned(),
            Self::TabIndentation { line } => {
                format!("line {line}: tab indentation is not valid YAML")
            }
            Self::DuplicateKey { line, key } => {
                format!("line {line}: duplicate frontmatter key `{key}`")
            }
            Self::ListItemBeforeKey { line } => format!("line {line}: list item before any key"),
            Self::NotAKey { line } => {
                format!("line {line}: not a key, list item, or continuation")
            }
        }
    }
}

/// Parse a frontmatter block, reporting the malformed shapes a conformance
/// check cares about.
///
/// A block that opens correctly but breaks a rule yields the error *and* stops
/// parsing, so at most one structural error is reported per file.
///
/// # Errors
///
/// Returns [`ParseError::NoFence`] when the file does not open with `---`, and
/// the corresponding variant for the first structural violation found.
pub fn parse_strict(text: &str) -> Result<Frontmatter, ParseError> {
    let caps = FENCE_OPTIONAL_NL
        .captures(text)
        .ok_or(ParseError::NoFence)?;
    let Some(body) = caps.get(1) else {
        return Err(ParseError::NoFence);
    };

    let mut fm = Frontmatter::default();
    let mut current_key: Option<String> = None;

    // The opening fence is line 1, so the first body line is line 2.
    for (offset, line) in body.as_str().split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let n = offset.saturating_add(2);

        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with('\t') {
            return Err(ParseError::TabIndentation { line: n });
        }
        if let Some(km) = KEY_LINE.captures(line) {
            let (Some(key), Some(value)) = (km.get(1), km.get(2)) else {
                return Err(ParseError::NotAKey { line: n });
            };
            let key = key.as_str();
            if fm.contains_scalar(key) {
                return Err(ParseError::DuplicateKey {
                    line: n,
                    key: key.to_owned(),
                });
            }
            fm.insert_scalar(key, value.as_str().trim());
            current_key = Some(key.to_owned());
        } else if LIST_ITEM.is_match(line) {
            let Some(key) = current_key.as_deref() else {
                return Err(ParseError::ListItemBeforeKey { line: n });
            };
            // The Python splits on the first `-` anywhere in the line, not on
            // the list dash. For every list item that has ever been written
            // these coincide, and reproducing it keeps the port honest.
            let item = line.split_once('-').map_or("", |(_, rest)| rest).trim();
            fm.push_list_item(key, item);
        } else if line.starts_with(' ') || line.starts_with('-') {
            // A continuation line, or a nested item under an indented key.
            // Neither is this parser's business: only column zero is a key.
        } else {
            return Err(ParseError::NotAKey { line: n });
        }
    }
    Ok(fm)
}

/// Parse a frontmatter block for value extraction, reporting nothing.
///
/// Returns the block and the body that follows it. A file with no frontmatter
/// yields an empty block and the whole text as the body.
///
/// Two deliberate differences from [`parse_strict`], both load-bearing for
/// generated output: a repeated key takes its *last* value, and a key whose
/// raw value is empty is dropped entirely rather than stored as `""`.
#[must_use]
pub fn parse_lenient(text: &str) -> (Frontmatter, &str) {
    let Some(caps) = FENCE_REQUIRED_NL.captures(text) else {
        return (Frontmatter::default(), text);
    };
    let (Some(body), Some(whole)) = (caps.get(1), caps.get(0)) else {
        return (Frontmatter::default(), text);
    };

    let mut fm = Frontmatter::default();
    for line in body.as_str().split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(km) = KEY_LINE.captures(line) else {
            continue;
        };
        let (Some(key), Some(raw)) = (km.get(1), km.get(2)) else {
            continue;
        };
        // An empty raw value is skipped, so `verified:` opening a block
        // sequence leaves no scalar behind.
        if raw.as_str().is_empty() {
            continue;
        }
        fm.insert_scalar(key.as_str(), &unquote_lenient(raw.as_str().trim()));
    }
    (fm, &text[whole.end()..])
}

/// Strip one layer of matching quotes and undo the two escapes.
///
/// This is `okf-check`'s rule, which requires at least two characters, so a
/// lone quote character is left alone.
#[must_use]
pub fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        return strip_quotes(value);
    }
    value.to_owned()
}

/// The same, under `okf-index`'s rule, which has no length guard.
///
/// The two differ on exactly one input: a value that is a single quote
/// character is its own opener and closer here and unquotes to the empty
/// string. Keeping the difference costs one function and is the sort of thing
/// a "tidy up while porting" pass silently changes.
#[must_use]
pub fn unquote_lenient(value: &str) -> String {
    strip_quotes(value)
}

fn strip_quotes(value: &str) -> String {
    let bytes = value.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return value.to_owned();
    };
    if first == last && (first == b'"' || first == b'\'') {
        let end = value.len().saturating_sub(1);
        let inner = value.get(1..end).unwrap_or_default();
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    value.to_owned()
}

/// The raw frontmatter block of `text`, fence excluded.
///
/// `None` when the file does not open with one.
#[must_use]
pub fn block(text: &str) -> Option<&str> {
    FENCE_OPTIONAL_NL
        .captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())
}

/// The byte range of the raw value of top-level scalar `key`, within `text`.
///
/// This is what lets a caller rewrite one value and leave every other byte of
/// the file exactly as written — no requoting, no reordering, no reflowing of
/// a block the caller never looked at. `okf-migrate --retype` changes 672
/// documents in one pass and must change nothing but their `type`.
///
/// The range covers the value with trailing whitespace excluded, so a line
/// written `type: Runbook   ` keeps its trailing spaces. It is empty for a key
/// written with no value at all.
///
/// `None` when the file has no frontmatter block or the block has no such key
/// at column zero. Both refusals matter: a `type:` line inside a fenced code
/// block further down the file is an exemplar in somebody's prompt template,
/// not the document's own type, and this never sees it.
#[must_use]
pub fn scalar_span(text: &str, key: &str) -> Option<std::ops::Range<usize>> {
    let body = FENCE_OPTIONAL_NL.captures(text)?.get(1)?;
    let mut at = body.start();
    for raw in body.as_str().split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(caps) = KEY_LINE.captures(line)
            && caps.get(1).is_some_and(|name| name.as_str() == key)
            && let Some(value) = caps.get(2)
        {
            let start = at.saturating_add(value.start());
            let len = value.as_str().trim_end().len();
            return Some(start..start.saturating_add(len));
        }
        // `split('\n')` consumed the separator, so step over it too.
        at = at.saturating_add(raw.len()).saturating_add(1);
    }
    None
}

/// The lines indented under top-level `key`, with their 1-based file line
/// numbers.
///
/// [`parse_strict`] is a YAML subset in which only column zero is a key, which
/// is what keeps a nested mapping from being read as a top-level one. That is
/// the right rule for conformance and the wrong one for two keys the
/// confidentiality gates have to see inside: `owner` and `promoted_from`. This
/// reads one level down, for those two and nothing else.
#[must_use]
pub fn nested_lines<'a>(text: &'a str, key: &str) -> Vec<(usize, &'a str)> {
    let Some(body) = block(text) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mut inside = false;
    for (offset, raw) in body.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        // The opening fence is line 1, so the first body line is line 2.
        let n = offset.saturating_add(2);
        if let Some(name) = KEY_LINE.captures(line).and_then(|c| c.get(1)) {
            inside = name.as_str() == key;
            continue;
        }
        if inside {
            if line.trim().is_empty() {
                continue;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') {
                inside = false;
                continue;
            }
            lines.push((n, line));
        }
    }
    lines
}

/// The flat `subkey: value` pairs one level under `key`.
///
/// Values are unquoted. A repeated subkey takes its last value, which matters
/// nowhere the two callers use it and is stated so nobody has to guess.
#[must_use]
pub fn nested_map(text: &str, key: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (_, line) in nested_lines(text, key) {
        let trimmed = line.trim_start().trim_start_matches("- ").trim_start();
        if let Some(caps) = KEY_LINE.captures(trimmed)
            && let (Some(name), Some(value)) = (caps.get(1), caps.get(2))
        {
            map.insert(
                name.as_str().to_owned(),
                unquote(value.as_str().trim()).trim().to_owned(),
            );
        }
    }
    map
}

/// One subkey of a record, with the line it was written on.
///
/// The line is carried because the diagnostic that reads this points at a
/// subkey somebody added, and pointing at the record instead would send them
/// to the wrong line of their own file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub line: usize,
    pub name: String,
    pub value: String,
}

/// One record of a block sequence of mappings under `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The file line the record opens on.
    pub line: usize,
    /// Its subkeys, in the order written, with values unquoted.
    pub fields: Vec<Field>,
}

impl Record {
    /// The value of `name`, or `None`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.value.as_str())
    }
}

/// A block sequence of mappings under `key`, as
/// `owner: [{name, title, email}, …]` is written.
///
/// A scalar `key: value` yields nothing, which is how the caller tells "no
/// records" from "the wrong shape entirely": it checks the scalar itself.
#[must_use]
pub fn nested_records(text: &str, key: &str) -> Vec<Record> {
    let mut records: Vec<Record> = Vec::new();
    for (n, line) in nested_lines(text, key) {
        let trimmed = line.trim_start();
        let (starts, rest) = match trimmed.strip_prefix("- ") {
            Some(rest) => (true, rest.trim_start()),
            None => (false, trimmed),
        };
        if starts {
            records.push(Record {
                line: n,
                fields: Vec::new(),
            });
        }
        let Some(caps) = KEY_LINE.captures(rest) else {
            continue;
        };
        let (Some(name), Some(value)) = (caps.get(1), caps.get(2)) else {
            continue;
        };
        let Some(current) = records.last_mut() else {
            continue;
        };
        current.fields.push(Field {
            line: n,
            name: name.as_str().to_owned(),
            value: unquote(value.as_str().trim()).trim().to_owned(),
        });
    }
    records
}

/// The items of a block sequence of scalars under `key`, with line numbers.
///
/// `sources:` is the one that matters: an entry naming a private path
/// discloses exactly as a body link does, and it is not a link, so the link
/// scanner would never see it.
#[must_use]
pub fn nested_items(text: &str, key: &str) -> Vec<(usize, String)> {
    nested_lines(text, key)
        .into_iter()
        .filter_map(|(n, line)| {
            let item = line.trim_start().strip_prefix("- ")?.trim();
            (!item.is_empty()).then(|| (n, unquote(item).trim().to_owned()))
        })
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "a panicking assertion is the point of a test"
)]
mod tests {
    use super::*;

    /// Force every `LazyLock` in this module so a malformed pattern literal
    /// fails here rather than at a user's first run.
    #[test]
    fn every_pattern_compiles() {
        assert!(FENCE_OPTIONAL_NL.is_match("---\na: b\n---\n"));
        assert!(FENCE_REQUIRED_NL.is_match("---\na: b\n---\n"));
        assert!(KEY_LINE.is_match("a: b"));
        assert!(LIST_ITEM.is_match("  - x"));
    }

    /// Splice a new value into the span, which is the only thing the span is
    /// for and the only way to assert it lands on the right bytes.
    fn spliced(text: &str, key: &str, value: &str) -> String {
        let span = scalar_span(text, key).unwrap();
        let mut out = text.to_owned();
        out.replace_range(span, value);
        out
    }

    #[test]
    fn a_scalar_span_covers_the_value_and_nothing_around_it() {
        let text = "---\ntype: \"Meeting Summary\"\ntitle: \"Kept\"\n---\n\n# Body\n";
        assert_eq!(
            spliced(text, "type", "\"Meeting\""),
            "---\ntype: \"Meeting\"\ntitle: \"Kept\"\n---\n\n# Body\n"
        );
    }

    #[test]
    fn a_scalar_span_leaves_trailing_whitespace_and_crlf_alone() {
        let text = "---\r\ntype: Runbook  \r\ntitle: Kept\r\n---\r\n\r\nBody\r\n";
        assert_eq!(
            spliced(text, "type", "Reference"),
            "---\r\ntype: Reference  \r\ntitle: Kept\r\n---\r\n\r\nBody\r\n"
        );
    }

    /// The estate has documents whose `^type:` line is an exemplar inside a
    /// fenced code block in a prompt template. It is not the document's own
    /// type, a grep counts it, and this must not see it.
    #[test]
    fn a_type_line_in_a_code_fence_is_not_frontmatter() {
        let template = "# Prompt\n\nWrite the block:\n\n```yaml\ntype: Meeting Summary\n```\n";
        assert_eq!(scalar_span(template, "type"), None);

        // Same line, in a document that does have frontmatter: the block's own
        // key wins and the exemplar below it is never reached.
        let both = "---\ntype: Template\n---\n\n```yaml\ntype: Meeting Summary\n```\n";
        assert_eq!(
            spliced(both, "type", "Template"),
            both,
            "the exemplar in the fence must be untouched"
        );
        assert_eq!(
            spliced(both, "type", "Reference"),
            "---\ntype: Reference\n---\n\n```yaml\ntype: Meeting Summary\n```\n"
        );
    }

    #[test]
    fn a_key_absent_or_indented_has_no_span() {
        assert_eq!(scalar_span("---\ntitle: x\n---\n", "type"), None);
        // Only column zero is a key, so a nested `type` is not this document's.
        assert_eq!(scalar_span("---\nowner:\n  type: x\n---\n", "type"), None);
    }

    #[test]
    fn duplicate_key_is_refused_with_its_line() {
        let err = parse_strict("---\na: 1\nb: 2\na: 3\n---\n").unwrap_err();
        assert_eq!(
            err,
            ParseError::DuplicateKey {
                line: 4,
                key: "a".to_owned()
            }
        );
        assert_eq!(err.message(), "line 4: duplicate frontmatter key `a`");
    }

    #[test]
    fn tab_indentation_is_refused() {
        let err = parse_strict("---\na: 1\n\tb: 2\n---\n").unwrap_err();
        assert_eq!(err, ParseError::TabIndentation { line: 3 });
    }

    #[test]
    fn list_item_before_any_key_is_refused() {
        let err = parse_strict("---\n- orphan\n---\n").unwrap_err();
        assert_eq!(err, ParseError::ListItemBeforeKey { line: 2 });
    }

    #[test]
    fn indented_keys_are_continuations_not_keys() {
        let fm = parse_strict("---\nowner:\n  name: A\n  name: B\n---\n").unwrap();
        // `name` repeats, but only at depth, so it is not a duplicate key.
        assert_eq!(fm.get("owner"), Some(""));
        assert_eq!(fm.get("name"), None);
    }

    #[test]
    fn lenient_takes_the_last_value_and_drops_empty_ones() {
        let (fm, body) = parse_lenient("---\na: 1\na: 2\nb:\n---\nbody\n");
        assert_eq!(fm.get("a"), Some("2"));
        assert_eq!(fm.get("b"), None);
        assert_eq!(body, "body\n");
    }

    #[test]
    fn lenient_returns_whole_text_when_there_is_no_fence() {
        let (fm, body) = parse_lenient("no frontmatter\n");
        assert_eq!(fm.keys().count(), 0);
        assert_eq!(body, "no frontmatter\n");
    }

    #[test]
    fn unquote_undoes_one_layer_and_two_escapes() {
        assert_eq!(unquote(r#""a \"b\" c""#), r#"a "b" c"#);
        assert_eq!(unquote("'x'"), "x");
        assert_eq!(unquote("bare"), "bare");
    }

    /// The one input the checker's and the indexer's rules disagree on.
    #[test]
    fn a_lone_quote_unquotes_differently_in_each_tool() {
        assert_eq!(unquote("\""), "\"");
        assert_eq!(unquote_lenient("\""), "");
    }

    #[test]
    fn nested_reads_one_level_under_a_key_and_stops_at_the_next() {
        let text = "---\ntype: System\nowner:\n  - name: \"A\"\n    title: \"T\"\n  - name: \"B\"\ntags:\n  - x\n---\n";
        let records = nested_records(text, "owner");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("name"), Some("A"));
        assert_eq!(records[0].get("title"), Some("T"));
        assert_eq!(records[1].get("name"), Some("B"));
        assert_eq!(records[0].line, 4);
        // Each subkey carries its own line, not the record's.
        assert_eq!(records[0].fields[1].line, 5);
        // `tags` belongs to the next key, not to `owner`.
        assert!(records.iter().all(|r| r.get("x").is_none()));
    }

    #[test]
    fn a_nested_mapping_reads_as_a_flat_map() {
        let text =
            "---\npromoted_from:\n  repo: \"e/n\"\n  path: \"a/b.md\"\n  rev: \"abc\"\n---\n";
        let map = nested_map(text, "promoted_from");
        assert_eq!(map.get("repo").map(String::as_str), Some("e/n"));
        assert_eq!(map.get("rev").map(String::as_str), Some("abc"));
    }

    #[test]
    fn a_scalar_key_has_no_records() {
        let text = "---\nowner: someone\n---\n";
        assert!(nested_records(text, "owner").is_empty());
    }

    #[test]
    fn sequence_items_carry_their_line() {
        let text =
            "---\nsources:\n  - meetings/2026-01-01-x/summary.md\n  - https://example.test/\n---\n";
        let items = nested_items(text, "sources");
        assert_eq!(items[0], (3, "meetings/2026-01-01-x/summary.md".to_owned()));
        assert_eq!(items[1].0, 4);
    }

    #[test]
    fn sequences_live_under_a_bracketed_key() {
        let fm = parse_strict("---\ntags:\n  - a\n  - b\n---\n").unwrap();
        assert_eq!(fm.get("tags"), Some(""));
        assert!(fm.keys().any(|k| k == "tags[]"));
    }
}
