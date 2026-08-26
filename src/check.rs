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

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::collision::Collision;
use crate::config::{Confidentiality, Config};
use crate::frontmatter::{self, Frontmatter, ParseError, parse_strict, unquote};
use crate::links::{self, Target};
use crate::staleness::Day;
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
/// §9's log heading shape, and only that.
///
/// Deliberately not [`crate::staleness::Day`], which range-checks the month
/// and the day: this pattern is what the Python emitted an error from, the
/// parity run compared those errors, and tightening it would fail a bundle
/// over a heading that has nothing to do with staleness.
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

    // Built once, ahead of the walk, so the diagnostic lands on the page it
    // is about and in the same path order as every other finding.
    let collisions: BTreeMap<String, Collision> =
        crate::collision::find(root, &config.paths.skip_names)
            .into_iter()
            .map(|c| (c.page.clone(), c))
            .collect();
    for path in walk::markdown_files(root, &config.paths.skip_names) {
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let name = crate::walk::to_posix(rel);
        let text = walk::read_lossy(&path);
        report.checked = report.checked.saturating_add(1);

        if let Some(collision) = collisions.get(&name) {
            check_section_collision(&mut report, &name, collision);
        }

        match path.file_name().and_then(|n| n.to_str()) {
            Some("index.md") => {
                let is_root = path.parent() == Some(root);
                check_index(&mut report, &name, &text, is_root, config);
            }
            Some("log.md") => check_log(&mut report, &name, &text),
            _ => check_concept(
                &mut report,
                &name,
                &text,
                &types,
                config.confidentiality.closed_vocabulary,
                config.as_of.as_ref(),
            ),
        }

        if !config.paths.keep_readme {
            check_readme_retired(&mut report, &name);
        }

        // The confidentiality rules run over every file, reserved names
        // included: a hand-written root index carries links like any other
        // page, and the gate exists for the page nobody thought to check.
        if config.confidentiality.links_stay_in_bundle {
            check_links(&mut report, &name, &text, root, &config.confidentiality);
        }
        if config.confidentiality.owner_record {
            check_owner(&mut report, &name, &text);
        }
    }
    Ok(report)
}

/// No link leaves the bundle.
///
/// Containment on its own would not catch the failure this exists for. A page
/// hand-copied out of a private bundle keeps `../people/dana-quill.md`, and
/// from `systems/` that resolves to `people/dana-quill.md`, which is *inside*
/// the new bundle root and simply is not there. So the target has to exist,
/// and a link to a file the bundle does not hold is the same error as a link
/// that climbs out of it.
fn check_links(report: &mut Report, path: &str, text: &str, root: &Path, conf: &Confidentiality) {
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut targets: Vec<(usize, String, &'static str, &str)> = links::links(text)
        .into_iter()
        .map(|link| (link.line, link.target, "link", dir))
        .collect();
    // A `sources` entry naming a private path discloses exactly as a body
    // link does, and it is not a link, so the scanner above never sees it.
    // Free prose in the list is left alone: an entry with a space in it is a
    // citation somebody wrote, not a path. Entries are bundle-relative rather
    // than page-relative: frontmatter is not prose, and a citation is written
    // once wherever the page later moves to.
    targets.extend(
        frontmatter::nested_items(text, "sources")
            .into_iter()
            .filter(|(_, item)| item.split_whitespace().count() == 1)
            .map(|(line, item)| (line, item, "sources entry", "")),
    );
    // File order, so a reader walks their own document rather than this
    // function's two passes over it.
    targets.sort_by_key(|(line, _, _, _)| *line);
    for (line, raw, label, from) in &targets {
        judge_target(report, path, root, conf, *line, raw, label, from);
    }
}

/// Judge one target written on `path`, reporting it as `label`.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a distinct fact about one finding, and a struct               here would be a parameter list with a name"
)]
fn judge_target(
    report: &mut Report,
    path: &str,
    root: &Path,
    conf: &Confidentiality,
    line: usize,
    raw: &str,
    label: &str,
    from: &str,
) {
    let target = links::classify(raw, from);
    match &target {
        Target::Fragment => {}
        Target::Url { .. } => {
            if !target.url_is_reachable(&conf.site_urls, raw) {
                report.err(
                    path,
                    &format!("line {line}: {label} `{raw}` is not a URL this bundle may carry"),
                );
            }
        }
        Target::Escapes => report.err(
            path,
            &format!("line {line}: {label} `{raw}` leaves the bundle"),
        ),
        Target::Inside { path: inside } => {
            if !root.join(inside).exists() {
                report.err(
                    path,
                    &format!("line {line}: {label} `{raw}` has no target in this bundle"),
                );
            }
        }
    }
}

/// `owner` carries exactly `name`, `title` and `email`.
///
/// An error rather than a warning, and the only local convention in this tool
/// that is one. The record is *constructed* from a source page's owner bullet
/// cross-checked against a profile, never sliced out of the profile, and the
/// schema is what keeps it from growing back toward one: a subkey that would
/// hold an assessment has nowhere to go.
fn check_owner(report: &mut Report, path: &str, text: &str) {
    for (_, message) in owner_errors(text) {
        report.err(path, &message);
    }
    for record in &frontmatter::nested_records(text, "owner") {
        if record.get("title").unwrap_or_default().is_empty() {
            let line = record.line;
            report.warn(path, &format!("line {line}: owner record has no `title`"));
        }
    }
}

/// Everything wrong with a page's `owner` record, as (line, message).
///
/// Shared with `okf-promote`, which runs it over a hand-restated draft. A
/// reviewer constructing an owner record by hand is exactly when a fourth
/// subkey appears, and catching it at promotion beats catching it in the
/// destination repository's build afterwards.
///
/// Errors only. A missing `title` is a quality matter and stays a warning of
/// the checker's own; a subkey the schema does not have is the boundary.
#[must_use]
pub fn owner_errors(text: &str) -> Vec<(usize, String)> {
    const PERMITTED: [&str; 3] = ["name", "title", "email"];

    let Ok(fm) = parse_strict(text) else {
        // Malformed frontmatter is already an error from the concept check.
        return Vec::new();
    };
    let Some(raw) = fm.get("owner") else {
        return Vec::new();
    };
    if !raw.trim().is_empty() {
        return vec![(
            0,
            "owner must be a sequence of `- name:` records, not a scalar".to_owned(),
        )];
    }
    let records = frontmatter::nested_records(text, "owner");
    if records.is_empty() {
        return vec![(
            0,
            "owner is present but holds no `- name:` record".to_owned(),
        )];
    }

    let mut found = Vec::new();
    for record in &records {
        for field in &record.fields {
            if !PERMITTED.contains(&field.name.as_str()) {
                let (at, key) = (field.line, &field.name);
                found.push((
                    at,
                    format!("line {at}: owner subkey `{key}` is not name, title or email"),
                ));
            }
        }
        if record.get("name").unwrap_or_default().is_empty() {
            let line = record.line;
            found.push((line, format!("line {line}: owner record has no `name`")));
        }
    }
    found.sort_by_key(|(line, _)| *line);
    found
}

fn check_concept(
    report: &mut Report,
    path: &str,
    text: &str,
    types: &BTreeSet<String>,
    closed_vocabulary: bool,
    as_of: Option<&Day>,
) {
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
        // Everywhere else this is a warning, because §11 forbids a consumer
        // rejecting a bundle over an unknown `type`. A bundle that closes its
        // vocabulary is not a foreign consumer of itself: it has decided that
        // `Person` is not a name it holds, and a person-shaped page arriving
        // in it is a confidentiality failure rather than a vocabulary drift.
        let message = format!("type `{ctype}` is not in the bundle vocabulary");
        if closed_vocabulary {
            report.err(path, &format!("{message} (closed by this bundle)"));
        } else {
            report.warn(path, &message);
        }
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
    check_trust(report, path, &fm, as_of);
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

/// The frontmatter this tool reports on beyond §11: `generated` / `verified`
/// actor and timestamp form (§5, §6), `stale_after`, and `status`.
///
/// One function because the order it emits in is the order a reader sees, and
/// that order is part of what the parity run compared.
fn check_trust(report: &mut Report, path: &str, fm: &Frontmatter, as_of: Option<&Day>) {
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
    if let Some(raw) = fm.get("stale_after") {
        check_stale_after(report, path, raw, as_of);
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

/// Is this document past the day it said it was good until?
///
/// Three outcomes, and the middle one is the point of the change. A value
/// that is not a day is the shape warning this field has always produced. A
/// day with nothing to measure it against is a promise the bundle cannot
/// keep, and saying so on the document is how the author of that promise
/// finds out. A day that has passed is the diagnostic the field was always
/// supposed to produce and never did.
///
/// All three are warnings. §11 forbids a consumer rejecting a bundle over a
/// key it does not like, and a lapsed review date is a quality matter rather
/// than a conformance one. `max_warnings` is what gives them teeth: a bundle
/// records the count it adopted at, so a document going stale spends budget
/// the bundle does not have and the gate goes red until somebody deals with
/// it.
///
/// `as_of` is a parameter rather than a clock read. See [`crate::staleness`].
fn check_stale_after(report: &mut Report, path: &str, raw: &str, as_of: Option<&Day>) {
    let Some(stale_after) = Day::parse(&unquote(raw)) else {
        report.warn(path, &format!("stale_after `{raw}` is not YYYY-MM-DD"));
        return;
    };
    let Some(as_of) = as_of else {
        report.warn(
            path,
            &format!(
                "stale_after `{stale_after}` enforces nothing: this bundle has no \
                 `{file}` naming the day to measure it against",
                file = crate::staleness::AS_OF_FILE
            ),
        );
        return;
    };
    if stale_after.has_passed(as_of) {
        report.warn(
            path,
            &format!("stale_after `{stale_after}` has passed (as of {as_of})"),
        );
    }
}

/// §8 — a page may not claim the URL a sibling directory's listing publishes
/// at.
///
/// An **error**, and the argument for that is §11.3 rather than a fourth gated
/// class. §11 already makes `index.md` following §8 a conformance rule, and
/// this is a rule about that reserved name: it is the `index.md` that commits
/// the directory to publishing as a section, so a page claiming the same URL
/// is a §8 matter and not a new one. A bundle carrying both passes every other
/// rule the standard has and still cannot be mounted without losing a page,
/// which is the standard promising something it does not deliver.
///
/// A warning would be the wrong instrument even setting the class aside.
/// `max_warnings` records what a bundle reported when it adopted, so a bundle
/// adopting with the collision already present banks it in the budget and
/// never fixes it. The ratchet is built to hold a count steady; this is a
/// defect that has to reach zero.
///
/// The fix is one edit. Fold the page into the listing, or rename it to
/// something that is not its sibling directory's name — `overview.md` is what
/// this estate calls that file. See [`crate::collision`] for what was
/// measured.
fn check_section_collision(report: &mut Report, name: &str, collision: &Collision) {
    let listing = &collision.listing;
    let url = &collision.url;
    report.err(
        name,
        &format!(
            "this page and the listing {listing} both publish at `{url}/`, and a \
             site build keeps one of them without saying which. Fold it into the \
             listing, or rename it (§8)"
        ),
    );
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

/// A `README.md` in a bundle that has retired the name.
///
/// `[paths] keep_readme` records a decision every adopting bundle makes: in a
/// code repository the name is load-bearing, because a docs gate or GitHub
/// itself depends on it; in a knowledge bundle §8's generated `index.md` takes
/// the listing role and the README goes. The key used to record that decision
/// and enforce nothing, so one adopting bundle set it to `false`
/// while deleting every `README.md` by hand, and a reader of that config
/// reasonably concluded the tool had done the retirement.
///
/// A warning rather than an error, and deleting nothing, which is the same
/// line every other convention in this checker sits on. §11 conformance is
/// three rules and this is not one of them; what makes it bite is
/// `max_warnings`, which a bundle sets to what it reported when it adopted.
/// A README that comes back raises the count and fails the gate.
fn check_readme_retired(report: &mut Report, path: &str) {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.eq_ignore_ascii_case("readme.md") {
        report.warn(
            path,
            "README.md is still here and [paths] keep_readme is false; \
             §8's generated index.md carries the listing in this bundle",
        );
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
        check_concept(&mut report, "p.md", text, &types, false, None);
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
            false,
            None,
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
            false,
            None,
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn a_closed_vocabulary_turns_the_type_warning_into_an_error() {
        let mut report = Report::default();
        let types = ["System".to_owned()].into_iter().collect();
        check_concept(
            &mut report,
            "people/dana.md",
            "---\ntype: Person\ntitle: T\ndescription: D\n---\nb\n",
            &types,
            true,
            None,
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(
            report.errors,
            [
                "people/dana.md: type `Person` is not in the bundle vocabulary (closed by this bundle)"
            ]
        );
    }

    #[test]
    fn an_owner_subkey_outside_the_three_is_an_error() {
        let mut report = Report::default();
        check_owner(
            &mut report,
            "systems/press.md",
            "---\ntype: System\nowner:\n  - name: \"A\"\n    title: \"T\"\n    notes: \"skeptical of the vendor\"\n---\nb\n",
        );
        assert_eq!(
            report.errors,
            ["systems/press.md: line 6: owner subkey `notes` is not name, title or email"]
        );
    }

    #[test]
    fn an_owner_record_needs_a_name_and_only_warns_without_a_title() {
        let mut report = Report::default();
        check_owner(
            &mut report,
            "p.md",
            "---\nowner:\n  - title: \"T\"\n---\nb\n",
        );
        assert!(report.errors.iter().any(|e| e.contains("has no `name`")));
        assert!(report.warnings.is_empty());

        let mut report = Report::default();
        check_owner(
            &mut report,
            "p.md",
            "---\nowner:\n  - name: \"A\"\n---\nb\n",
        );
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.warnings.iter().any(|w| w.contains("no `title`")));
    }

    #[test]
    fn an_owner_that_is_not_a_sequence_of_records_is_refused() {
        let mut report = Report::default();
        check_owner(&mut report, "p.md", "---\nowner: Dana Quill\n---\nb\n");
        assert!(report.errors.iter().any(|e| e.contains("not a scalar")));

        let mut report = Report::default();
        check_owner(&mut report, "p.md", "---\nowner:\ntype: System\n---\nb\n");
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("no `- name:` record"))
        );
    }

    #[test]
    fn a_page_with_no_owner_key_reports_nothing() {
        let mut report = Report::default();
        check_owner(&mut report, "p.md", "---\ntype: System\n---\nb\n");
        assert!(report.errors.is_empty());
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
