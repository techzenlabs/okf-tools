//! What Hugo does to a path segment on the way to a URL.
//!
//! Two callers need the same answer and used to derive it separately: the
//! raw-markdown verifier, which has to know where a source file was published,
//! and the link rewriter, which has to write a URL a reader can follow. Each
//! assumed the segment survived unchanged. The site shipped links to pages
//! that were not there, and the verifier reported the same page missing and
//! unexplained at once. One function now, measured once.
//!
//! Measured against Hugo 0.165 by building a site whose content tree carried
//! one file per shape and reading the published paths back off disk. The
//! answer is not "lowercase": Hugo drops most punctuation, turns each
//! whitespace character into a hyphen, keeps a percent escape whole, and
//! lowercases with Go's simple case mapping, which is not Rust's.
//!
//! What Hugo does *not* transform is the filename of a non-markdown page
//! resource. `Docs Dir/Diagram Alpha.png` publishes at
//! `docs-dir/Diagram Alpha.png`: the directory segments are sanitised and the
//! leaf is left exactly as written. That was measured too, because a
//! sanitised asset name would be a broken image on every page carrying one.

use std::sync::LazyLock;

use regex::Regex;

/// One path segment, written the way Hugo publishes it.
///
/// Not "lowercased", which is the assumption that produced the false failure
/// this function replaces. Hugo drops most punctuation and turns whitespace
/// into hyphens as well, and every clause below is a measurement against
/// Hugo 0.165 rather than a reading of its source:
///
/// | source segment | published segment |
/// | --- | --- |
/// | `README` | `readme` |
/// | `Release Notes (Draft)` | `release-notes-draft` |
/// | `My  Two Spaces` | `my--two-spaces` |
/// | ` Leading And Trailing ` | `-leading-and-trailing-` |
/// | `Pct%2F` | `pct%2f` |
/// | `Pct%zz` | `pctzz` |
/// | `Ünï-Çø` | `ünï-çø` |
/// | `İstanbul` | `istanbul` |
/// | `ΟΔΟΣ` | `οδοσ` |
/// | `Ⅷroman` | `roman` |
#[must_use]
pub fn url_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (offset, ch) in name.char_indices() {
        let rest = name.get(offset.saturating_add(1)..).unwrap_or_default();
        if ch == '%' && starts_with_hex_pair(rest) {
            // A percent escape already in the name survives whole, so a
            // segment written `Pct%2F` publishes at `pct%2f` and not `pct2f`.
            out.push('%');
        } else if is_letter_or_digit(ch) || KEPT.contains(&ch) {
            push_lowercase(&mut out, ch);
        } else if ch.is_whitespace() {
            // One hyphen per space, with no collapsing and no trimming: two
            // spaces publish as two hyphens, and a leading space as a leading
            // hyphen. Both measured.
            out.push('-');
        }
    }
    out
}

/// What Hugo keeps in a path segment besides a letter or a digit.
///
/// Measured one character at a time. `&`, `,`, `(`, `)`, `!`, `'`, `;`, `=`,
/// `$`, `[`, `]`, `{`, `}`, `?`, `:`, `<`, `>`, `|`, `^`, `"`, `` ` ``, `*`
/// and an emoji are each dropped instead.
const KEPT: &[char] = &['_', '-', '.', '#', '+', '~', '@', '\\'];

fn starts_with_hex_pair(rest: &str) -> bool {
    let mut chars = rest.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(first), Some(second))
            if first.is_ascii_hexdigit() && second.is_ascii_hexdigit()
    )
}

#[expect(
    clippy::expect_used,
    reason = "a static pattern literal, forced by tests::the_category_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// Unicode category `L` or `Nd`, which is what Go's `unicode.IsLetter` and
/// `unicode.IsDigit` accept and therefore what Hugo keeps.
///
/// The nearest `char` predicates are both wider and both wrong here.
/// `char::is_alphabetic` is the Alphabetic property, which holds for `Ⅷ`
/// (category `Nl`); `char::is_numeric` holds for `½` (category `No`). Hugo
/// drops both, measured. `regex` already carries the category tables, so the
/// test is exact rather than close.
static LETTER_OR_DIGIT: LazyLock<Regex> = LazyLock::new(|| compiled(r"^[\p{L}\p{Nd}]$"));

fn is_letter_or_digit(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_alphanumeric();
    }
    let mut buffer = [0u8; 4];
    LETTER_OR_DIGIT.is_match(ch.encode_utf8(&mut buffer))
}

/// Lowercase one character the way Go does, which is not the way Rust does.
///
/// Go uses the simple case mapping and `char::to_lowercase` the full one, and
/// they part on exactly one character: `İ` (U+0130), which Rust expands to `i`
/// plus a combining dot above. Measured: Hugo publishes `İstanbul.md` at
/// `/istanbul/`.
///
/// The other place the two mappings part is final sigma, and mapping one
/// character at a time is what avoids it. `ΟΔΟΣ` publishes at `οδοσ`, which is
/// what this produces and what `str::to_lowercase` would not: that one applies
/// the contextual rule and would give `οδος`.
fn push_lowercase(out: &mut String, ch: char) {
    if ch == '\u{130}' {
        out.push('i');
        return;
    }
    out.extend(ch.to_lowercase());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_category_pattern_compiles() {
        assert!(is_letter_or_digit('a'));
        assert!(LETTER_OR_DIGIT.is_match("é"));
    }

    /// Every row was read off a Hugo build, one file per row, and not off
    /// Hugo's source. The two disagree: the source suggests spaces collapse
    /// and the build shows they do not.
    #[test]
    fn a_segment_is_transformed_the_way_hugo_was_measured_to_transform_it() {
        for (source, published) in [
            ("README", "readme"),
            ("Documentation", "documentation"),
            ("already-lower", "already-lower"),
            ("Release Notes (Draft)", "release-notes-draft"),
            ("My  Two Spaces", "my--two-spaces"),
            (" Leading And Trailing ", "-leading-and-trailing-"),
            ("A - B", "a---b"),
            ("Trailing--Dashes", "trailing--dashes"),
            ("Foo_Bar", "foo_bar"),
            ("a.b.c", "a.b.c"),
            ("at@sign", "at@sign"),
            ("hash#tag", "hash#tag"),
            ("plus+one", "plus+one"),
            ("Tilde~x", "tilde~x"),
            ("A&B", "ab"),
            ("comma,x", "commax"),
            ("100% Sure", "100-sure"),
            ("Pct%2F", "pct%2f"),
            ("Pct%zz", "pctzz"),
            ("emoji\u{1f600}x", "emojix"),
            ("Caf\u{e9}", "caf\u{e9}"),
            ("\u{c4}nderung", "\u{e4}nderung"),
            // Go lowercases with the simple mapping and Rust with the full
            // one. Both rows are a Hugo build's answer, not Rust's.
            ("\u{130}stanbul", "istanbul"),
            (
                "\u{39f}\u{394}\u{39f}\u{3a3}",
                "\u{3bf}\u{3b4}\u{3bf}\u{3c3}",
            ),
            // Category Nl and category No are neither letters nor digits.
            ("\u{2167}roman", "roman"),
            ("\u{bd}half", "half"),
        ] {
            assert_eq!(url_segment(source), published, "{source}");
        }
    }
}
