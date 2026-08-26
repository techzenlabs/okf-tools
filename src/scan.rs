//! `okf-scan`: the confidentiality gate, which fails closed.
//!
//! It runs over an assembled tree, which came from a `git fetch` and therefore
//! holds only tracked files. That is worth noticing rather than assuming: a
//! scanner walking a working tree instead would flag ignored build artefacts
//! nobody publishes and miss nothing, which trains people to ignore it.
//!
//! **Failing closed means three things, not one.** A finding fails. A file it
//! cannot read fails, because "unreadable" and "clean" are not the same
//! answer. And a run that scanned *nothing* fails, because a scanner pointed
//! at the wrong directory otherwise reports success — which is the failure
//! mode a confidentiality gate can least afford.
//!
//! Matches are never printed. A finding names the file, the line and the rule,
//! and shows at most a masked prefix, because the log a public CI writes is
//! itself a publication.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fancy_regex::Regex;

/// How much of a file is inspected before it is called binary.
const SNIFF_BYTES: usize = 8192;

/// A file larger than this is reported as unscannable rather than read.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// Directories never descended into.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".direnv",
    "node_modules",
    "target",
    "result",
    ".venv",
    "__pycache__",
];

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("scan rule `{name}` does not compile: {source}")]
    Rule {
        name: &'static str,
        #[source]
        source: Box<fancy_regex::Error>,
    },
    #[error("{path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// One thing found, described without reproducing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub rule: &'static str,
    /// A masked prefix, enough to find the line and not enough to leak it.
    pub masked: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {} ({})",
            self.path, self.line, self.rule, self.masked
        )
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Files actually read and inspected.
    pub scanned: usize,
    /// Files skipped because they are not text.
    pub binary: usize,
    /// Files that could not be inspected. Each one fails the run.
    pub unreadable: Vec<String>,
}

impl Report {
    /// A clean scan is a scan that read something and found nothing.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.unreadable.is_empty() && self.scanned > 0
    }

    /// Why the run failed, for a caller that has to say so.
    #[must_use]
    pub fn failure_reason(&self) -> Option<String> {
        if !self.findings.is_empty() {
            return Some(format!("{} finding(s)", self.findings.len()));
        }
        if !self.unreadable.is_empty() {
            return Some(format!(
                "{} file(s) could not be inspected, which is not the same answer as clean",
                self.unreadable.len()
            ));
        }
        if self.scanned == 0 {
            return Some(
                "no files were inspected; a scan that read nothing is not a clean scan".to_owned(),
            );
        }
        None
    }
}

/// What a scan looks for, and where.
#[derive(Debug, Default)]
pub struct Options {
    /// Also flag an unformatted nine-digit run.
    ///
    /// Off by default and deliberately so. The pattern is the most aggressive
    /// of the three and it fires on any nine adjacent digits, which in a
    /// repository full of commit hashes and pinned revisions is mostly noise —
    /// and a gate people learn to ignore protects nothing.
    pub bare_nine_digit: bool,
    /// Literal strings that must not appear, case-insensitively.
    ///
    /// Supplied by the caller and never shipped. A list of the names that must
    /// not appear is itself a disclosure of who they are, so it lives outside
    /// any repository this tool is checked into.
    pub deny: Vec<String>,
    /// Path prefixes not scanned, named on the command line so an exemption is
    /// always visible in the run that used it.
    pub exclude: Vec<String>,
}

struct Rule {
    name: &'static str,
    pattern: Regex,
}

/// The detectors, in the order a finding is attributed.
fn rule_sources(bare_nine_digit: bool) -> Vec<(&'static str, &'static str)> {
    let mut rules = vec![
        (
            "private-key",
            r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----",
        ),
        ("putty-private-key", r"PuTTY-User-Key-File-\d"),
        ("github-token", r"\bgh[pousr]_[A-Za-z0-9]{36,255}\b"),
        (
            "github-fine-grained-token",
            r"\bgithub_pat_[A-Za-z0-9_]{22,255}\b",
        ),
        ("slack-token", r"\bxox[abeoprsu]-[A-Za-z0-9-]{10,}"),
        ("aws-access-key-id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        ("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
        ("npm-token", r"\bnpm_[A-Za-z0-9]{36}\b"),
        (
            "azure-storage-account-key",
            r"(?i)AccountKey\s*=\s*[A-Za-z0-9+/]{40,}={0,2}",
        ),
        (
            "json-web-token",
            r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
        ),
        // Three passes at the same identifier, because the formatting varies
        // and the labelled form catches a value the digits alone would not.
        (
            "national-identifier-labelled",
            r"(?i)\b(?:ssn|ssns|s\.s\.?n\.?|social\s*security(?:\s*(?:no|number|num|#))?)\b\s*[:#=\-]?\s*\d{3}[-.\s]?\d{2}[-.\s]?\d{4}\b",
        ),
        (
            "national-identifier-formatted",
            r"\b\d{3}[-.\s]\d{2}[-.\s]\d{4}\b",
        ),
    ];
    if bare_nine_digit {
        // Hex-aware boundaries rather than digit-aware ones. A nine-digit run
        // inside a commit hash is bounded by letters, so a digit-only
        // lookaround matches it; this one does not.
        rules.push((
            "national-identifier-bare",
            r"(?<![0-9A-Fa-f])\d{9}(?![0-9A-Fa-f])",
        ));
    }
    rules
}

fn compile(options: &Options) -> Result<Vec<Rule>, ScanError> {
    rule_sources(options.bare_nine_digit)
        .into_iter()
        .map(|(name, pattern)| {
            Regex::new(pattern)
                .map(|pattern| Rule { name, pattern })
                .map_err(|source| ScanError::Rule {
                    name,
                    source: Box::new(source),
                })
        })
        .collect()
}

/// Scan `root`, which may be a file or a directory.
///
/// # Errors
///
/// Fails only when a detector does not compile, which is a defect in this
/// crate rather than in the tree being scanned. Everything about the tree is
/// reported in the [`Report`].
pub fn scan(root: &Path, options: &Options) -> Result<Report, ScanError> {
    let rules = compile(options)?;
    let deny: Vec<String> = options.deny.iter().map(|d| d.to_lowercase()).collect();
    let mut report = Report::default();
    let mut queue = vec![root.to_path_buf()];
    let excluded: BTreeSet<String> = options.exclude.iter().cloned().collect();

    while let Some(path) = queue.pop() {
        let shown = display_path(root, &path);
        if excluded
            .iter()
            .any(|prefix| shown.starts_with(prefix.as_str()))
        {
            continue;
        }
        if path.is_dir() {
            push_children(&path, &mut queue, &mut report);
            continue;
        }
        scan_file(&path, &shown, &rules, &deny, &mut report);
    }
    report
        .findings
        .sort_by(|a, b| (a.path.as_str(), a.line, a.rule).cmp(&(b.path.as_str(), b.line, b.rule)));
    report.unreadable.sort();
    Ok(report)
}

fn push_children(dir: &Path, queue: &mut Vec<PathBuf>, report: &mut Report) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        report.unreadable.push(dir.display().to_string());
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        let name = child
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if child.is_dir() && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        queue.push(child);
    }
}

fn scan_file(path: &Path, shown: &str, rules: &[Rule], deny: &[String], report: &mut Report) {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE_BYTES => {
            report.unreadable.push(shown.to_owned());
            return;
        }
        Ok(_) => {}
        Err(_) => {
            report.unreadable.push(shown.to_owned());
            return;
        }
    }
    let Ok(bytes) = std::fs::read(path) else {
        report.unreadable.push(shown.to_owned());
        return;
    };
    if bytes.iter().take(SNIFF_BYTES).any(|b| *b == 0) {
        report.binary = report.binary.saturating_add(1);
        return;
    }
    report.scanned = report.scanned.saturating_add(1);
    let text = String::from_utf8_lossy(&bytes);
    for (number, line) in text.lines().enumerate() {
        inspect_line(shown, number.saturating_add(1), line, rules, deny, report);
    }
}

fn inspect_line(
    shown: &str,
    line_number: usize,
    line: &str,
    rules: &[Rule],
    deny: &[String],
    report: &mut Report,
) {
    for rule in rules {
        if let Ok(Some(found)) = rule.pattern.find(line) {
            report.findings.push(Finding {
                path: shown.to_owned(),
                line: line_number,
                rule: rule.name,
                masked: mask(found.as_str()),
            });
        }
    }
    if deny.is_empty() {
        return;
    }
    let lowered = line.to_lowercase();
    if deny.iter().any(|word| lowered.contains(word.as_str())) {
        report.findings.push(Finding {
            path: shown.to_owned(),
            line: line_number,
            rule: "denied-term",
            masked: "…".to_owned(),
        });
    }
}

/// Show enough of a match to find the line, and not enough to leak it.
fn mask(found: &str) -> String {
    let head: String = found.chars().take(4).collect();
    format!("{head}… {} chars", found.chars().count())
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| path.display().to_string(), crate::walk::to_posix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(text: &str, options: &Options) -> Vec<&'static str> {
        let rules = compile(options).unwrap_or_default();
        let mut report = Report::default();
        for (number, line) in text.lines().enumerate() {
            inspect_line("x", number, line, &rules, &[], &mut report);
        }
        report.findings.iter().map(|f| f.rule).collect()
    }

    /// Test vectors are assembled from fragments rather than written out.
    ///
    /// This repository scans itself in CI, and a detector whose own test
    /// corpus trips it is a detector people start excluding files from. The
    /// concatenation is compile-time, so the string under test is exactly the
    /// string that matters — it just is not a sequence of bytes in this file.
    const KEY_HEADER: &str = concat!("-----BEGIN RSA ", "PRIVATE KEY", "-----");
    const KEY_OPENSSH: &str = concat!("-----BEGIN OPENSSH ", "PRIVATE KEY", "-----");
    const KEY_BARE: &str = concat!("-----BEGIN ", "PRIVATE KEY", "-----");
    const ID_FORMATTED: &str = concat!("id 123", "-45-", "6789 here");
    const ID_LABELLED: &str = concat!("SS", "N: 123", "456789");
    const ID_BARE: &str = concat!("bare 1234", "56789 run");
    const HASH_LINE: &str = concat!("rev = \"ab1234", "56789cdef0000000000000000000000000\"");

    #[test]
    fn a_planted_private_key_header_is_found() {
        for planted in [KEY_HEADER, KEY_OPENSSH, KEY_BARE] {
            assert!(
                findings(planted, &Options::default()).contains(&"private-key"),
                "{planted}"
            );
        }
    }

    /// A certificate is public by definition and must not fail a build.
    #[test]
    fn a_certificate_is_not_a_private_key() {
        let certificate = concat!("-----BEGIN ", "CERTIFICATE", "-----");
        assert!(findings(certificate, &Options::default()).is_empty());
    }

    #[test]
    fn the_three_identifier_shapes_are_found_and_the_bare_one_is_opt_in() {
        let default = Options::default();
        assert!(findings(ID_FORMATTED, &default).contains(&"national-identifier-formatted"));
        assert!(findings(ID_LABELLED, &default).contains(&"national-identifier-labelled"));
        assert!(findings(ID_BARE, &default).is_empty());

        let aggressive = Options {
            bare_nine_digit: true,
            ..Options::default()
        };
        assert!(findings(ID_BARE, &aggressive).contains(&"national-identifier-bare"));
    }

    /// The reason the bare pattern is opt-in, asserted rather than argued: a
    /// commit hash carries nine adjacent digits and a digit-only lookaround
    /// matches inside it.
    #[test]
    fn a_commit_hash_is_not_an_identifier_even_under_the_aggressive_rule() {
        let aggressive = Options {
            bare_nine_digit: true,
            ..Options::default()
        };
        assert!(findings(HASH_LINE, &aggressive).is_empty());
    }

    /// The token shapes, with values chosen so no forge's own push protection
    /// reads this file as a leaked credential.
    #[test]
    fn the_forge_token_shapes_are_found() {
        let default = Options::default();
        let github = format!("token: gh{}_{}", "p", "A".repeat(36));
        let fine_grained = format!("token: github{}_{}", "_pat", "B".repeat(24));
        let aws = format!("id: AK{}{}", "IA", "C".repeat(16));
        assert!(findings(&github, &default).contains(&"github-token"));
        assert!(findings(&fine_grained, &default).contains(&"github-fine-grained-token"));
        assert!(findings(&aws, &default).contains(&"aws-access-key-id"));
    }

    #[test]
    fn a_match_is_reported_masked_rather_than_reproduced() {
        let masked = mask(KEY_HEADER);
        assert!(masked.starts_with("----"));
        assert!(!masked.contains("PRIVATE"));
    }

    /// The three ways a run fails, each asserted, because "clean" here is a
    /// claim about the scan as well as about the tree.
    #[test]
    fn an_empty_run_and_an_unreadable_file_are_both_failures() {
        let empty = Report::default();
        assert!(!empty.is_clean());
        assert!(empty.failure_reason().is_some());

        let unreadable = Report {
            scanned: 3,
            unreadable: vec!["a".to_owned()],
            ..Report::default()
        };
        assert!(!unreadable.is_clean());

        let clean = Report {
            scanned: 3,
            ..Report::default()
        };
        assert!(clean.is_clean());
        assert!(clean.failure_reason().is_none());
    }
}
