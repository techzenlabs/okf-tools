//! Every link a page carries, and where its target lands.
//!
//! Two gates read this module and they read it for the same reason. A link out
//! of a bundle is the one disclosure a checker can actually see: a name in
//! prose is contact identity, but `[Dana Quill](../people/dana-quill.md)` is a
//! pointer into an interpretive layer, and the path itself discloses even
//! where a rendered page suppresses the link.
//!
//! So containment is decided textually, on the path as written, and never by
//! asking the filesystem to canonicalise it. A symlink out of the bundle would
//! resolve to a target inside it, and the string a raw-markdown publishing
//! route emits is the one written here.

use std::sync::LazyLock;

use regex::Regex;

#[expect(
    clippy::expect_used,
    reason = "static pattern literals, all forced by tests::every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// `[text](target)` and `![alt](target)`, with an optional `"title"` tail and
/// an optional `<…>` wrapper around the target.
static INLINE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"\[[^\]]*\]\(\s*<?([^)<>\s]*)>?[^)]*\)"));

/// `[label]: target`, a reference definition at column zero.
static REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?m)^\[[^\]]+\]:[ \t]*<?([^>\s]+)"));

/// `<https://example.test/x>`, an autolink.
static AUTOLINK: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"<([A-Za-z][A-Za-z0-9+.\-]*:[^>\s]+)>"));

/// A sentence boundary: `.`, `!` or `?` followed by space.
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| compiled(r"[.!?]\s"));

/// One link, with enough context for a person to decide what to do about it.
///
/// The sentence is carried rather than the line because the reader of a
/// resolution report is deciding what a claim needs, not where a string is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The target exactly as written, fragment and query included.
    pub target: String,
    /// 1-based line number in the file.
    pub line: usize,
    /// The sentence the link sits in, whitespace collapsed.
    pub sentence: String,
}

/// Every link in `text`, in the order they appear.
///
/// Duplicates are kept: the same target cited twice is two decisions, because
/// the two sentences may need different resolutions.
#[must_use]
pub fn links(text: &str) -> Vec<Link> {
    let mut found: Vec<(usize, Link)> = Vec::new();
    for pattern in [&*INLINE, &*REFERENCE, &*AUTOLINK] {
        for caps in pattern.captures_iter(text) {
            let Some(target) = caps.get(1) else { continue };
            if target.as_str().is_empty() {
                continue;
            }
            found.push((
                target.start(),
                Link {
                    target: target.as_str().to_owned(),
                    line: line_of(text, target.start()),
                    sentence: sentence_at(text, target.start()),
                },
            ));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    found.dedup_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, link)| link).collect()
}

/// The 1-based line holding byte offset `at`.
#[must_use]
pub fn line_of(text: &str, at: usize) -> usize {
    text.get(..at)
        .unwrap_or_default()
        .matches('\n')
        .count()
        .saturating_add(1)
}

/// The sentence around byte offset `at`, whitespace collapsed.
///
/// Bounded by the line, because a markdown paragraph is reflowed and a bullet
/// is a sentence whether or not it ends in a full stop.
fn sentence_at(text: &str, at: usize) -> String {
    let start = text
        .get(..at)
        .unwrap_or_default()
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let rest = text.get(start..).unwrap_or_default();
    let end = rest.find('\n').unwrap_or(rest.len());
    let line = rest.get(..end).unwrap_or_default();
    let within = at.saturating_sub(start);

    let mut from = 0;
    let mut to = line.len();
    for boundary in SENTENCE_END.find_iter(line) {
        if boundary.end() <= within {
            from = boundary.end();
        } else {
            to = boundary.end();
            break;
        }
    }
    line.get(from..to)
        .unwrap_or(line)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a link target lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `#anchor`: the page itself.
    Fragment,
    /// An absolute URL, carrying its scheme lowercased.
    Url { scheme: String },
    /// A path inside the bundle, normalised and bundle-relative.
    Inside { path: String },
    /// A path that climbs above the bundle root.
    Escapes,
}

impl Target {
    /// Is this target one the bundle's own reader can follow?
    ///
    /// `site_urls` empty means any `http`/`https` URL passes, which is what
    /// lets a promoted page cite a vendor document. Non-empty narrows it.
    #[must_use]
    pub fn url_is_reachable(&self, site_urls: &[String], raw: &str) -> bool {
        match self {
            Self::Url { scheme } if scheme == "http" || scheme == "https" => {
                site_urls.is_empty() || site_urls.iter().any(|prefix| raw.starts_with(prefix))
            }
            // A `mailto:` is contact identity, which publishes.
            Self::Url { scheme } => scheme == "mailto",
            _ => true,
        }
    }
}

/// Classify `target`, written on a page whose directory is `page_dir`.
///
/// `page_dir` is bundle-relative and slash-separated, empty at the root.
#[must_use]
pub fn classify(target: &str, page_dir: &str) -> Target {
    let trimmed = target.trim();
    if trimmed.starts_with('#') {
        return Target::Fragment;
    }
    if let Some(scheme) = scheme_of(trimmed) {
        return Target::Url { scheme };
    }
    // A fragment or query on a path names a place in the target, not another
    // target, so neither takes part in resolution.
    let bare = trimmed
        .split(['#', '?'])
        .next()
        .unwrap_or_default()
        .trim_end();
    if bare.is_empty() {
        return Target::Fragment;
    }

    let mut segments: Vec<&str> = if bare.starts_with('/') {
        Vec::new()
    } else {
        page_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for part in bare.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Target::Escapes;
                }
            }
            other => segments.push(other),
        }
    }
    Target::Inside {
        path: segments.join("/"),
    }
}

/// The URL scheme of `target`, lowercased, when it has one.
///
/// A Windows drive letter is not a scheme, so a single leading character is
/// refused. Nothing else here needs to know about URLs.
fn scheme_of(target: &str) -> Option<String> {
    let (head, rest) = target.split_once(':')?;
    if head.len() < 2 || !head.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    if !head
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    {
        return None;
    }
    // `path:with:colons.md` is a path; a scheme is followed by `//` or by a
    // non-space opaque part, and a bare relative path has neither.
    if rest.starts_with("//") || head.eq_ignore_ascii_case("mailto") {
        return Some(head.to_ascii_lowercase());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(INLINE.is_match("[a](b.md)"));
        assert!(REFERENCE.is_match("[a]: b.md\n"));
        assert!(AUTOLINK.is_match("<https://example.test/>"));
        assert!(SENTENCE_END.is_match("one. two"));
    }

    #[test]
    fn all_three_link_shapes_are_found_once_each() {
        let text = "Inline [a](one.md) and ![img](two.png) and <https://example.test/>.\n\
                    \n[ref]: three.md\n";
        let found = links(text);
        let targets: Vec<&str> = found.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            targets,
            ["one.md", "two.png", "https://example.test/", "three.md"]
        );
    }

    #[test]
    fn a_link_carries_its_line_and_its_sentence() {
        let text = "# H\n\nFirst one. The queue is run by [Dana](../people/dana.md), who signs off. Third.\n";
        let found = links(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 3);
        assert_eq!(
            found[0].sentence,
            "The queue is run by [Dana](../people/dana.md), who signs off."
        );
    }

    #[test]
    fn a_bullet_is_its_own_sentence() {
        let text = "* Owner: [Dana](../people/dana.md)\n";
        assert_eq!(
            links(text)[0].sentence,
            "* Owner: [Dana](../people/dana.md)"
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_page_directory() {
        assert_eq!(
            classify("../people/dana.md", "org/systems"),
            Target::Inside {
                path: "org/people/dana.md".to_owned()
            }
        );
        assert_eq!(
            classify("./sibling.md", "systems"),
            Target::Inside {
                path: "systems/sibling.md".to_owned()
            }
        );
        assert_eq!(
            classify("/at/root.md", "deep/nested"),
            Target::Inside {
                path: "at/root.md".to_owned()
            }
        );
    }

    #[test]
    fn climbing_above_the_root_is_an_escape() {
        assert_eq!(classify("../../elsewhere.md", "systems"), Target::Escapes);
        assert_eq!(classify("../x.md", ""), Target::Escapes);
    }

    #[test]
    fn a_fragment_or_a_query_names_a_place_in_the_target() {
        assert_eq!(classify("#section", "a"), Target::Fragment);
        assert_eq!(
            classify("other.md#section", "a"),
            Target::Inside {
                path: "a/other.md".to_owned()
            }
        );
    }

    #[test]
    fn schemes_are_recognised_and_paths_holding_a_colon_are_not() {
        assert_eq!(
            classify("https://example.test/x", ""),
            Target::Url {
                scheme: "https".to_owned()
            }
        );
        assert_eq!(
            classify("MAILTO:dana@example.test", ""),
            Target::Url {
                scheme: "mailto".to_owned()
            }
        );
        assert_eq!(
            classify("notes/2026-01-01: thing.md", ""),
            Target::Inside {
                path: "notes/2026-01-01: thing.md".to_owned()
            }
        );
    }

    #[test]
    fn site_urls_narrow_what_an_absolute_url_may_be() {
        let vendor = classify("https://vendor.test/doc", "");
        assert!(vendor.url_is_reachable(&[], "https://vendor.test/doc"));
        let tenant = ["https://docs.example.test/".to_owned()];
        assert!(!vendor.url_is_reachable(&tenant, "https://vendor.test/doc"));
        assert!(
            classify("https://docs.example.test/a", "")
                .url_is_reachable(&tenant, "https://docs.example.test/a")
        );
    }

    #[test]
    fn a_scheme_that_is_neither_web_nor_mail_is_never_reachable() {
        let target = classify("file://etc/passwd", "");
        assert!(!target.url_is_reachable(&[], "file://etc/passwd"));
    }
}
