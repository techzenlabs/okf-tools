//! Generated `index.md` listings.
//!
//! OKF §8 gives `index.md` the directory-listing role, one per directory,
//! built from the `title` and `description` of the concepts it lists. Each
//! index keeps a hand-written lead above the generated block; everything
//! between the markers is rewritten from the documents themselves, so a
//! listing can never drift from what it lists.
//!
//! The markers are HTML comments, which every renderer ignores and every
//! parser tolerates, so the mechanism costs the format nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use regex::Regex;

use crate::config::Config;
use crate::frontmatter::parse_lenient;
use crate::walk::{self, is_markdown};

pub const BEGIN: &str = "<!-- BEGIN OKF INDEX (tools/okf-index) -->";
pub const END: &str = "<!-- END OKF INDEX -->";

#[expect(
    clippy::expect_used,
    reason = "static pattern literals, all forced by tests::every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

static H1: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^#\s+(.+?)\s*$"));
static H1_LINE: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^#\s+.+?$"));
static PARA_SPLIT: LazyLock<Regex> = LazyLock::new(|| compiled(r"\n\s*\n"));
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| compiled(r"\[([^\]]+)\]\([^)]*\)"));
static ROOT_FM_GREEDY_NL: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?s)^---\n.*?\n---\n+"));
static ROOT_FM_ONE_NL: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?s)^---\n.*?\n---\n"));
static MONTH_PREFIX: LazyLock<Regex> = LazyLock::new(|| compiled(r"^\d{4}-\d{2}"));

/// A bare address in a quoted description lints as an error in the listing;
/// the angle-bracket form is the markdown spelling of the same text.
///
/// This needs real lookaround — the point is to leave an address that is
/// *already* bracketed alone — which is why it is the one pattern in the crate
/// not compiled by the `regex` crate.
#[expect(
    clippy::expect_used,
    reason = "static pattern literal, forced by tests::every_pattern_compiles"
)]
fn bare_email() -> FancyRegex {
    FancyRegex::new(r"(?<![<\w.\-])([\w.+\-]+@[\w\-]+\.[\w.\-]+)(?![>\w])")
        .expect("static regex literal must compile")
}

static BARE_EMAIL: LazyLock<FancyRegex> = LazyLock::new(bare_email);

/// What a run did, or would have done.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Indexes whose content on disk differs from what would be generated.
    pub stale: Vec<String>,
    pub written: usize,
    pub directories: usize,
}

/// Regenerate (or, with `check_only`, compare) every index in the bundle.
///
/// Directories are visited deepest first, because a parent's listing reads its
/// children's descriptions and must see them already current.
///
/// # Errors
///
/// Fails when a mirror `title_strip` pattern is invalid or an index cannot be
/// written.
pub fn run(root: &Path, config: &Config, check_only: bool) -> anyhow::Result<Outcome> {
    let mirrors = config.mirror_rules()?;
    let mut dirs = bundle_directories(root, config);
    // Stable sort on descending depth: within one depth the directories are
    // independent, so their relative order cannot reach the output.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));

    let mut outcome = Outcome {
        directories: dirs.len(),
        ..Outcome::default()
    };

    for dir in &dirs {
        let index = dir.join("index.md");
        let new = render(dir, root, config, &mirrors);
        let old = if index.exists() {
            Some(walk::read_lossy(&index))
        } else {
            None
        };
        if old.as_deref() == Some(new.as_str()) {
            continue;
        }
        if check_only {
            let rel = index
                .strip_prefix(root)
                .map(walk::to_posix)
                .unwrap_or_default();
            outcome.stale.push(rel);
        } else {
            std::fs::write(&index, &new)?;
            outcome.written = outcome.written.saturating_add(1);
        }
    }
    outcome.stale.sort();
    Ok(outcome)
}

/// Every directory that gets an index of its own.
fn bundle_directories(root: &Path, config: &Config) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    collect_directories(root, root, config, &mut dirs);
    dirs
}

fn collect_directories(dir: &Path, root: &Path, config: &Config, out: &mut Vec<PathBuf>) {
    for child in walk::children(dir, &config.paths.skip_names) {
        if !child.is_dir() {
            continue;
        }
        if in_bundle(&child, root, config) {
            out.push(child.clone());
        }
        collect_directories(&child, root, config, out);
    }
}

fn in_bundle(dir: &Path, root: &Path, config: &Config) -> bool {
    let Ok(rel) = dir.strip_prefix(root) else {
        return false;
    };
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts
        .iter()
        .any(|p| walk::is_skipped(p, &config.paths.skip_names))
    {
        return false;
    }
    // A directory that is itself an entry in a month-grouped listing gets no
    // index: an index inside each of ~275 meeting folders would be noise.
    if parts.len() == 2
        && parts
            .first()
            .is_some_and(|first| config.index.no_index_under.contains(first))
    {
        return false;
    }
    walk::has_markdown(dir)
}

/// The full text of one directory's `index.md`.
fn render(dir: &Path, root: &Path, config: &Config, mirrors: &[(String, Regex)]) -> String {
    let generated = entries_for(dir, root, config, mirrors);
    let index = dir.join("index.md");
    let is_root = dir == root;

    if index.exists() {
        let text = walk::read_lossy(&index);
        if text.contains(BEGIN) && text.contains(END) {
            let out = replace_block(&text, &block(&generated));
            return if is_root {
                format!(
                    "{}{}",
                    root_frontmatter(config),
                    ROOT_FM_GREEDY_NL.replacen(&out, 1, "")
                )
            } else {
                out
            };
        }
        // First run for this directory: keep whatever prose is there as the
        // lead, and drop the hand-maintained listing below it.
        let body = ROOT_FM_ONE_NL.replacen(&text, 1, "");
        let (heading, lead) = first_run_lead(&body, dir);
        return assemble(&heading, &lead, &generated, is_root, config);
    }

    let heading = format!("# {}", dir_name(dir));
    assemble(&heading, "", &generated, is_root, config)
}

fn first_run_lead(body: &str, dir: &Path) -> (String, String) {
    let Some(head) = H1_LINE.find(body) else {
        return (format!("# {}", dir_name(dir)), String::new());
    };
    let rest = body.get(head.end()..).unwrap_or_default();
    let lead = PARA_SPLIT
        .split(rest)
        .map(str::trim)
        .find(|p| !p.is_empty() && !starts_with_any(p, &["#", "*", "-", "|", "<!--"]))
        .unwrap_or_default();
    (head.as_str().to_owned(), lead.to_owned())
}

fn assemble(heading: &str, lead: &str, generated: &str, is_root: bool, config: &Config) -> String {
    let mut parts = vec![heading.to_owned(), String::new()];
    if !lead.is_empty() {
        parts.push(lead.to_owned());
        parts.push(String::new());
    }
    parts.push(block(generated));
    parts.push(String::new());
    let out = parts.join("\n");
    if is_root {
        format!("{}{out}", root_frontmatter(config))
    } else {
        out
    }
}

fn root_frontmatter(config: &Config) -> String {
    format!(
        "---\nokf_version: \"{}\"\ntitle: \"{}\"\ndescription: \"{}\"\n---\n\n",
        config.okf_version, config.title, config.description
    )
}

/// Swap the marker-delimited region for a freshly generated one.
///
/// Uses a closure rather than a replacement template because entry text
/// carries backslashes and dollar signs that a template would expand.
fn replace_block(text: &str, replacement: &str) -> String {
    MARKED_BLOCK
        .replace_all(text, |_: &regex::Captures| replacement)
        .into_owned()
}

static MARKED_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    compiled(&format!(
        "(?s){}.*?{}",
        regex::escape(BEGIN),
        regex::escape(END)
    ))
});

/// The marker-delimited region, collapsed when there is nothing to list so it
/// does not lint as a run of blank lines.
fn block(generated: &str) -> String {
    let body = generated.trim();
    if body.is_empty() {
        format!("{BEGIN}\n{END}")
    } else {
        format!("{BEGIN}\n\n{body}\n\n{END}")
    }
}

/// The generated listing for one directory.
fn entries_for(dir: &Path, root: &Path, config: &Config, mirrors: &[(String, Regex)]) -> String {
    let mut concepts = Vec::new();
    let mut subdirs = Vec::new();
    for child in walk::children(dir, &config.paths.skip_names) {
        let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if child.is_dir() {
            if walk::has_markdown(&child) {
                subdirs.push(child.clone());
            }
        } else if is_markdown(name) && name != "index.md" && name != "log.md" {
            concepts.push(child.clone());
        }
    }

    let relative = dir
        .strip_prefix(root)
        .map(walk::to_posix)
        .unwrap_or_default();
    // A drop folder's contents are transient by design: every item leaves on
    // the next processing run, so listing them would churn the index on every
    // scrape and leave dangling links the moment one is filed.
    if config.index.suppress.contains(&relative) {
        return String::new();
    }
    if config.index.group_by_month.contains(&relative) {
        return month_grouped(&subdirs, config, root, mirrors);
    }

    let mut lines: Vec<String> = Vec::new();
    for concept in &concepts {
        let (title, desc) = label(concept, root, mirrors);
        lines.push(entry(&title, &file_name(concept), &desc));
    }
    if !subdirs.is_empty() {
        if !concepts.is_empty() {
            lines.push(String::new());
        }
        lines.push("## Sections\n".to_owned());
        for sub in &subdirs {
            let (title, desc) = dir_label(sub);
            lines.push(entry(&title, &format!("{}/", dir_name(sub)), &desc));
        }
    }
    finish(&lines)
}

/// A chronological stream, grouped by month so the listing stays navigable at
/// several hundred entries.
fn month_grouped(
    subdirs: &[PathBuf],
    config: &Config,
    root: &Path,
    mirrors: &[(String, Regex)],
) -> String {
    let mut by_month: BTreeMap<String, Vec<&PathBuf>> = BTreeMap::new();
    for sub in subdirs {
        let name = dir_name(sub);
        let month = if MONTH_PREFIX.is_match(&name) {
            name.chars().take(7).collect()
        } else {
            "undated".to_owned()
        };
        by_month.entry(month).or_default().push(sub);
    }

    let mut lines: Vec<String> = Vec::new();
    for (month, subs) in &by_month {
        lines.push(format!("## {month}\n"));
        for sub in subs {
            let summaries = walk::glob_children(sub, &config.index.month_entry_glob);
            for summary in &summaries {
                let (title, desc) = label(summary, root, mirrors);
                let rel = format!("{}/{}", dir_name(sub), file_name(summary));
                lines.push(entry(&title, &rel, &desc));
            }
            if summaries.is_empty() {
                let name = dir_name(sub);
                lines.push(entry(&name, &format!("{name}/"), ""));
            }
        }
        lines.push(String::new());
    }
    finish(&lines)
}

fn entry(title: &str, target: &str, desc: &str) -> String {
    if desc.is_empty() {
        format!("* [{title}]({target})")
    } else {
        format!("* [{title}]({target}) - {desc}")
    }
}

fn finish(lines: &[String]) -> String {
    format!("{}\n", lines.join("\n").trim_end())
}

/// Display title and one-line description for a concept document.
fn label(path: &Path, root: &Path, mirrors: &[(String, Regex)]) -> (String, String) {
    let text = walk::read_lossy(path);
    let (fm, body) = parse_lenient(&text);

    let mut title = fm.get("title").unwrap_or_default().to_owned();
    if title.is_empty() {
        title = H1.captures(body).and_then(|c| c.get(1)).map_or_else(
            || file_stem(path).replace('-', " "),
            |m| m.as_str().trim().to_owned(),
        );
    }
    if let Ok(rel) = path.strip_prefix(root) {
        let rel = walk::to_posix(rel);
        for (prefix, pattern) in mirrors {
            if rel.starts_with(&format!("{prefix}/")) {
                title = pattern.replace_all(&title, "").trim().to_owned();
            }
        }
    }
    // A bracket in a title would close the link's own text early.
    let title = title.replace('[', "(").replace(']', ")");

    let desc = fm.get("description").unwrap_or_default().trim().to_owned();
    let desc = BARE_EMAIL.replace_all(&desc, "<${1}>").into_owned();
    (title, desc)
}

/// Display title and description for a subdirectory, taken from its own index.
fn dir_label(dir: &Path) -> (String, String) {
    let index = dir.join("index.md");
    if !index.exists() {
        return (dir_name(dir), String::new());
    }
    let text = walk::read_lossy(&index);
    let (fm, body) = parse_lenient(&text);

    let title = match fm.get("title").filter(|t| !t.is_empty()) {
        Some(t) => t.to_owned(),
        None => H1
            .captures(body)
            .and_then(|c| c.get(1))
            .map_or_else(|| dir_name(dir), |m| m.as_str().trim().to_owned()),
    };

    let declared = fm.get("description").unwrap_or_default().trim().to_owned();
    if !declared.is_empty() {
        return (title, declared);
    }
    (title, derived_description(body))
}

/// A description inferred from the lead paragraph above the generated block.
fn derived_description(body: &str) -> String {
    let lead = body.split(BEGIN).next().unwrap_or_default();
    let lead = H1_LINE.replacen(lead, 1, "");
    let para = PARA_SPLIT
        .split(&lead)
        .map(str::trim)
        .find(|p| !p.is_empty() && !starts_with_any(p, &["#", "<!--"]))
        .unwrap_or_default();

    let collapsed = para.split_whitespace().collect::<Vec<_>>().join(" ");
    let flattened = MD_LINK.replace_all(&collapsed, "${1}").into_owned();
    truncate_words(&flattened, 200)
}

/// Cut to at most `limit` characters, then back to the last whole word.
fn truncate_words(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    let head = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{head}…")
}

fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| text.starts_with(p))
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    dir_name(path)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(H1.is_match("# T\n"));
        assert!(H1_LINE.is_match("# T\n"));
        assert!(PARA_SPLIT.is_match("a\n\nb"));
        assert!(MD_LINK.is_match("[t](u)"));
        assert!(ROOT_FM_GREEDY_NL.is_match("---\na: b\n---\n\n"));
        assert!(ROOT_FM_ONE_NL.is_match("---\na: b\n---\n"));
        assert!(MONTH_PREFIX.is_match("2026-08-24-topic"));
        assert!(BARE_EMAIL.is_match("a@b.com").unwrap_or(false));
    }

    #[test]
    fn an_empty_listing_collapses_to_bare_markers() {
        assert_eq!(block("\n"), format!("{BEGIN}\n{END}"));
        assert_eq!(block("* a\n"), format!("{BEGIN}\n\n* a\n\n{END}"));
    }

    #[test]
    fn a_bare_address_is_bracketed_and_a_bracketed_one_is_left_alone() {
        let bracket = |s: &str| BARE_EMAIL.replace_all(s, "<${1}>").into_owned();
        assert_eq!(
            bracket("from noreply@example.com now"),
            "from <noreply@example.com> now"
        );
        assert_eq!(
            bracket("(alerts@example.net)"),
            "(<alerts@example.net>)"
        );
        assert_eq!(
            bracket("<already@bracketed.com>"),
            "<already@bracketed.com>"
        );
    }

    #[test]
    fn entry_text_omits_the_dash_when_there_is_no_description() {
        assert_eq!(entry("T", "p.md", ""), "* [T](p.md)");
        assert_eq!(entry("T", "p.md", "d"), "* [T](p.md) - d");
    }

    #[test]
    fn a_replacement_containing_dollars_is_not_expanded() {
        let text = format!("lead\n\n{BEGIN}\nold\n{END}\n");
        let out = replace_block(&text, "$1 and \\n");
        assert!(out.contains("$1 and \\n"), "{out}");
    }

    #[test]
    fn truncation_falls_back_to_a_whole_word_and_marks_the_cut() {
        let long = "word ".repeat(60);
        let out = truncate_words(&long, 200);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 201);
        assert_eq!(truncate_words("short", 200), "short");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        let text = "é".repeat(250);
        let out = truncate_words(&text, 200);
        // No space to fall back to, so the whole 200-character cut survives.
        assert_eq!(out.chars().count(), 201);
    }
}
