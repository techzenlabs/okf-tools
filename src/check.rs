//! OKF v0.2 conformance checking.
//!
//! Conformance (§11) is three rules: parseable frontmatter on every
//! non-reserved `.md`, a non-empty `type` in it, and the reserved `index.md`
//! and `log.md` following §8 and §9.
//!
//! On top of the spec this reports the bundle's own conventions — the type
//! vocabulary, the actor form, ISO date shapes — as **warnings**, because a
//! free-for-all `type` makes a bundle unsearchable while staying perfectly
//! conformant. A foreign consumer is unaffected by any of them.
//!
//! Message text here is load-bearing. It reproduces the Python tool this
//! module replaces byte-for-byte, which is what lets the two be diffed against
//! a real corpus to prove the port changed nothing.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::config::Config;
use crate::frontmatter::{Frontmatter, ParseError, parse_strict, unquote};
use crate::walk;

#[expect(
    clippy::expect_used,
    reason = "static pattern literals, all forced by tests::every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// §7's actor form: `human:<id>`, `process:<id>` or `<producer>/<version>`.
static ACTOR: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"^(human:[\w.\-]+|process:[\w.\-]+|[\w.\-]+/[\w.:\-]+)$"));
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| compiled(r"^\d{4}-\d{2}-\d{2}$"));
static ISO_DATETIME: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"^\d{4}-\d{2}-\d{2}([T ][\d:.]+(Z|[+-]\d{2}:?\d{2})?)?$"));

/// `by:` and `at:` inside a trust block, read out of the raw text.
///
/// The block is a YAML flow mapping in every document that has one, and
/// reading it textually is what the Python does. A structural read would
/// change which documents report a diagnostic, so it is a later change with
/// its own diff rather than a quiet improvement made while porting.
static TRUST_BY: LazyLock<Regex> = LazyLock::new(|| compiled(r"\bby:\s*([^,}\s]+)"));
static TRUST_AT: LazyLock<Regex> = LazyLock::new(|| compiled(r"\bat:\s*([^,}\s]+)"));

static SECTION_HEADING: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^#\s+\S"));
static INDEX_ENTRY: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^\*\s+\[[^\]]*\]\([^)]*\)"));
static INDEX_BULLET: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^[*\-]\s+"));
static LOG_HEADING: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^##\s+(.+?)\s*$"));
static LEADING_FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?s)^---\r?\n.*?\r?\n---\r?\n"));

/// A v0.1 document reintroduced after the fact.
///
/// §13 replaced both of these in v0.2. Nothing in the estate carries either,
/// so these are a guard against reintroduction rather than migration work.
static V01_CITATIONS_HEADING: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?m)^#\s+Citations\s*$"));

/// What a run found.
#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub checked: usize,
}

impl Report {
    fn err(&mut self, path: &str, message: &str) {
        self.errors.push(format!("{path}: {message}"));
    }

    fn warn(&mut self, path: &str, message: &str) {
        self.warnings.push(format!("{path}: {message}"));
    }

    /// The exit status: non-zero when the bundle is non-conformant or has
    /// spent more than its warning budget.
    #[must_use]
    pub fn is_failure(&self, budget: usize) -> bool {
        !self.errors.is_empty() || self.warnings.len() > budget
    }
}

/// Check the bundle rooted at `root`.
///
/// # Errors
///
/// Fails only when the configured vocabulary cannot be resolved. A file that
/// cannot be read is reported as a finding, not as an error of this function.
pub fn check_bundle(root: &Path, config: &Config) -> Result<Report, crate::config::ConfigError> {
    let types = config.types()?;
    let mut report = Report::default();

    if !root.join("index.md").exists() {
        report.err(
            "index.md",
            "bundle root has no index.md declaring okf_version",
        );
    }

    for path in walk::markdown_files(root, &config.paths.skip_names) {
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let name = crate::walk::to_posix(rel);
        let text = walk::read_lossy(&path);
        report.checked = report.checked.saturating_add(1);

        match path.file_name().and_then(|n| n.to_str()) {
            Some("index.md") => {
                let is_root = path.parent() == Some(root);
                check_index(&mut report, &name, &text, is_root, config);
            }
            Some("log.md") => check_log(&mut report, &name, &text),
            _ => check_concept(&mut report, &name, &text, &types),
        }
    }
    Ok(report)
}

fn check_concept(report: &mut Report, path: &str, text: &str, types: &BTreeSet<String>) {
    let fm = match parse_strict(text) {
        Ok(fm) => fm,
        Err(ParseError::NoFence) => {
            // A file that opens with `---` but never closes the fence is
            // silently tolerated, which is the original's behaviour.
            if !text.starts_with("---") {
                report.err(path, &ParseError::NoFence.message());
            }
            return;
        }
        Err(other) => {
            report.err(path, &other.message());
            return;
        }
    };

    let ctype = fm.get_unquoted("type");
    if ctype.is_empty() {
        report.err(path, "frontmatter has no non-empty `type` (§11.2)");
    } else if !types.is_empty() && !types.contains(&ctype) {
        report.warn(
            path,
            &format!("type `{ctype}` is not in the bundle vocabulary"),
        );
    }
    if fm.get_unquoted("title").is_empty() {
        report.warn(path, "no `title` (recommended, §4.2)");
    }
    if fm.get_unquoted("description").is_empty() {
        report.warn(path, "no `description` (recommended, §4.2)");
    }
    check_v01_guards(report, path, text, &fm);
    if ctype == "Attested Computation" && fm.get_unquoted("runtime").is_empty() {
        report.warn(
            path,
            "type `Attested Computation` has no `runtime` (required for this type, §10.2)",
        );
    }
    check_trust(report, path, &fm);
}

/// §13's two breaking changes, as a guard against reintroduction.
///
/// Both carry the fix in the message, because the reader of this diagnostic is
/// someone who copied a v0.1 document from somewhere else.
fn check_v01_guards(report: &mut Report, path: &str, text: &str, fm: &Frontmatter) {
    if fm.get("timestamp").is_some() {
        report.err(
            path,
            "`timestamp` was replaced by `generated: {by, at}` in v0.2 (§13)",
        );
    }
    if V01_CITATIONS_HEADING.is_match(text) {
        report.err(
            path,
            "body `# Citations` was replaced by front-matter `sources` in v0.2 (§13)",
        );
    }
}

/// `generated` / `verified` actor and timestamp form (§5, §6).
fn check_trust(report: &mut Report, path: &str, fm: &Frontmatter) {
    for field in ["generated", "verified"] {
        let raw = fm.get(field).unwrap_or_default();
        if raw.is_empty() {
            continue;
        }
        for by in TRUST_BY.captures_iter(raw).filter_map(|c| c.get(1)) {
            let by = by.as_str();
            if !ACTOR.is_match(by) {
                report.warn(
                    path,
                    &format!(
                        "{field}.by `{by}` is not <producer>/<version>, \
                         human:<id>, or process:<id>"
                    ),
                );
            }
        }
        for at in TRUST_AT.captures_iter(raw).filter_map(|c| c.get(1)) {
            let at = at.as_str();
            if !ISO_DATETIME.is_match(at.trim_matches(['"', '\''])) {
                report.warn(
                    path,
                    &format!("{field}.at `{at}` is not an ISO 8601 date/datetime"),
                );
            }
        }
    }
    if let Some(raw) = fm.get("stale_after")
        && !ISO_DATE.is_match(&unquote(raw))
    {
        report.warn(path, &format!("stale_after `{raw}` is not YYYY-MM-DD"));
    }
    if let Some(raw) = fm.get("status") {
        let value = unquote(raw);
        if !matches!(value.as_str(), "draft" | "stable" | "deprecated") {
            report.warn(
                path,
                &format!("status `{raw}` is not draft/stable/deprecated"),
            );
        }
    }
}

/// §8 — a listing. Frontmatter is permitted only at the bundle root.
fn check_index(report: &mut Report, path: &str, text: &str, is_root: bool, config: &Config) {
    let has_fm = text.starts_with("---");
    if has_fm && !is_root {
        report.err(
            path,
            "index.md may only carry frontmatter at the bundle root (§8)",
        );
        return;
    }

    let mut body = text;
    let stripped;
    if has_fm {
        let fm = match parse_strict(text) {
            Ok(fm) => fm,
            // An unterminated fence is passed over here, as it is for a
            // concept: the file opened with `---`, so there is no missing
            // frontmatter to report.
            Err(ParseError::NoFence) => return,
            Err(other) => {
                report.err(path, &other.message());
                return;
            }
        };
        let declared = fm.get_unquoted("okf_version");
        if declared != config.okf_version {
            report.err(
                path,
                &format!(
                    "bundle-root index.md must declare okf_version: \"{}\" (§8)",
                    config.okf_version
                ),
            );
        }
        let allowed: BTreeSet<&str> = config.index.root_keys.iter().map(String::as_str).collect();
        let unknown: Vec<&str> = fm.keys().filter(|k| !allowed.contains(k)).collect();
        if !unknown.is_empty() {
            report.warn(
                path,
                &format!("unexpected root index keys: {}", unknown.join(", ")),
            );
        }
        stripped = LEADING_FRONTMATTER.replacen(text, 1, "").into_owned();
        body = &stripped;
    }

    if !SECTION_HEADING.is_match(body) {
        report.err(path, "index.md has no section heading (§8)");
    }
    let has_entries = INDEX_ENTRY.is_match(body);
    if INDEX_BULLET.is_match(body) && !has_entries {
        report.err(
            path,
            "index.md entries must be `* [Title](path) - description` (§8)",
        );
    }
    if body.lines().any(|line| line.starts_with("- ")) {
        report.err(path, "index.md entries use `*`, not `-` (§8)");
    }
}

/// §9 — chronological, ISO date headings, newest first.
fn check_log(report: &mut Report, path: &str, text: &str) {
    if text.starts_with("---") {
        report.err(path, "log.md must not carry frontmatter (§9)");
    }
    let headings: Vec<&str> = LOG_HEADING
        .captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if headings.is_empty() {
        report.err(path, "log.md has no `## YYYY-MM-DD` date headings (§9)");
    }
    for heading in &headings {
        if !ISO_DATE.is_match(heading) {
            report.err(
                path,
                &format!("log.md heading `{heading}` is not YYYY-MM-DD (§9)"),
            );
        }
    }
    let mut newest_first = headings.clone();
    newest_first.sort_unstable();
    newest_first.reverse();
    if headings != newest_first {
        report.warn(path, "log.md date headings are not newest-first (§9)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(ACTOR.is_match("human:alex"));
        assert!(ISO_DATE.is_match("2026-08-24"));
        assert!(ISO_DATETIME.is_match("2026-08-24T15:11:17Z"));
        assert!(TRUST_BY.is_match("{ by: human:alex }"));
        assert!(TRUST_AT.is_match("{ at: 2026-08-24 }"));
        assert!(SECTION_HEADING.is_match("# Title\n"));
        assert!(INDEX_ENTRY.is_match("* [T](p.md) - d\n"));
        assert!(INDEX_BULLET.is_match("* x\n"));
        assert!(LOG_HEADING.is_match("## 2026-08-24\n"));
        assert!(LEADING_FRONTMATTER.is_match("---\na: b\n---\n"));
        assert!(V01_CITATIONS_HEADING.is_match("# Citations\n"));
    }

    fn concept(text: &str) -> Report {
        let mut report = Report::default();
        let types = ["Runbook".to_owned()].into_iter().collect();
        check_concept(&mut report, "p.md", text, &types);
        report
    }

    #[test]
    fn a_missing_type_is_an_error_and_a_stray_one_is_a_warning() {
        let report = concept("---\ntitle: T\ndescription: D\n---\nbody\n");
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("§11.2"));

        let report = concept("---\ntype: Nope\ntitle: T\ndescription: D\n---\nbody\n");
        assert!(report.errors.is_empty());
        assert_eq!(
            report.warnings,
            ["p.md: type `Nope` is not in the bundle vocabulary"]
        );
    }

    #[test]
    fn a_file_with_no_frontmatter_at_all_is_reported() {
        let report = concept("just prose\n");
        assert_eq!(report.errors, ["p.md: no YAML frontmatter (§11.1)"]);
    }

    #[test]
    fn an_unterminated_fence_is_tolerated_as_the_original_tolerates_it() {
        let report = concept("---\ntype: Runbook\nnever closed\n");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }

    #[test]
    fn v01_shapes_are_guarded_against_reintroduction() {
        let report = concept(
            "---\ntype: Runbook\ntitle: T\ndescription: D\ntimestamp: 2026-01-01\n---\nb\n",
        );
        assert!(report.errors.iter().any(|e| e.contains("§13")));

        let report = concept("---\ntype: Runbook\ntitle: T\ndescription: D\n---\n# Citations\n");
        assert!(report.errors.iter().any(|e| e.contains("`sources`")));
    }

    #[test]
    fn attested_computation_without_a_runtime_warns_but_stays_conformant() {
        let mut report = Report::default();
        let types = ["Attested Computation".to_owned()].into_iter().collect();
        check_concept(
            &mut report,
            "p.md",
            "---\ntype: Attested Computation\ntitle: T\ndescription: D\n---\nb\n",
            &types,
        );
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w| w.contains("§10.2")));
    }

    #[test]
    fn an_empty_vocabulary_warns_on_nothing() {
        let mut report = Report::default();
        check_concept(
            &mut report,
            "p.md",
            "---\ntype: Anything At All\ntitle: T\ndescription: D\n---\nb\n",
            &BTreeSet::new(),
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn log_headings_must_be_iso_and_newest_first() {
        let mut report = Report::default();
        check_log(
            &mut report,
            "log.md",
            "# Log\n\n## 2026-01-01\n\n## 2026-02-01\n",
        );
        assert!(report.warnings.iter().any(|w| w.contains("newest-first")));

        let mut report = Report::default();
        check_log(&mut report, "log.md", "# Log\n\n## yesterday\n");
        assert!(report.errors.iter().any(|e| e.contains("not YYYY-MM-DD")));
    }

    #[test]
    fn a_non_root_index_may_not_carry_frontmatter() {
        let mut report = Report::default();
        check_index(
            &mut report,
            "d/index.md",
            "---\na: b\n---\n# D\n",
            false,
            &Config::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("bundle root"));
    }

    #[test]
    fn index_entries_must_use_a_star() {
        let mut report = Report::default();
        check_index(
            &mut report,
            "index.md",
            "# D\n\n- [T](p.md)\n",
            false,
            &Config::default(),
        );
        assert!(report.errors.iter().any(|e| e.contains("`*`, not `-`")));
    }
}
