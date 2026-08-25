//! Writing frontmatter into a bundle that has none.
//!
//! The rule this module is built around is that it does only what is
//! *derivable*, and refuses the rest. `title` comes from the document's own
//! H1, `description` from its own lead paragraph, `type` from a path rule
//! somebody wrote down. Everything else — `tags`, `status`, `stale_after`,
//! `sources`, `verified` — is judgement, and a tool that guessed at it would
//! be asserting things about the knowledge that nobody checked.
//!
//! Two refusals are load-bearing:
//!
//! * **No default type.** A file no `[[type_rules]]` entry matches is
//!   reported and left alone. Guessing `type`, the one field §11 requires,
//!   would be the same failure as fabricating `verified`.
//! * **Never overwrite.** An existing key is left exactly as written, which
//!   is what makes a second run a no-op and a revert a single `git checkout`.

use std::path::Path;
use std::sync::LazyLock;

use crate::config::Config;
use crate::frontmatter::{ParseError, parse_strict};
use crate::index::{MD_LINK, PARA_SPLIT, starts_with_any, truncate_words};
use crate::walk;

/// Where a derived value came from, for the review report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Already present in the document; not written.
    Existing,
    /// Taken from the document's first `# ` heading.
    Heading,
    /// Taken from the document's first prose paragraph.
    Paragraph,
    /// Derived from the filename, because the document had no heading.
    Filename,
    /// Nothing derivable. Reported for a human to write.
    None,
}

impl Source {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::Heading => "heading",
            Self::Paragraph => "paragraph",
            Self::Filename => "filename",
            Self::None => "none",
        }
    }
}

/// Why a file was left untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// `index.md` and `log.md` are generated and hand-maintained; §8 and §9
    /// own their shape, not this tool.
    Reserved,
    /// A generator owns the file. Writing here would be truncated on the next
    /// regeneration, and the freshness gate would then fail closed.
    Generated,
    /// No `[[type_rules]]` entry matched, so `type` is a human's call.
    NoTypeRule,
    /// The frontmatter is malformed. Migrating into a broken block would
    /// compound the problem rather than fix it.
    Unparseable(String),
    /// Every key this tool writes is already present.
    AlreadyMigrated,
}

impl Skip {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Reserved => "reserved".to_owned(),
            Self::Generated => "generated".to_owned(),
            Self::NoTypeRule => "no-type-rule".to_owned(),
            Self::Unparseable(why) => format!("unparseable: {why}"),
            Self::AlreadyMigrated => "already-migrated".to_owned(),
        }
    }
}

/// What migration would do to one file.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub concept_type: Option<String>,
    pub rule: Option<String>,
    pub title: Option<String>,
    pub title_source: Source,
    pub description: Option<String>,
    pub description_source: Source,
    pub skip: Option<Skip>,
    /// The file's full new text, when there is a change to make.
    pub rewritten: Option<String>,
}

impl Entry {
    /// One TSV row: path, type, rule, title source, description source, skip.
    #[must_use]
    pub fn tsv(&self) -> String {
        let dash = "-".to_owned();
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.path,
            self.concept_type.clone().unwrap_or_else(|| dash.clone()),
            self.rule.clone().unwrap_or_else(|| dash.clone()),
            self.title_source.label(),
            self.description_source.label(),
            self.skip.as_ref().map_or(dash, Skip::label),
        )
    }
}

/// The whole plan for a bundle.
#[derive(Debug, Default)]
pub struct Plan {
    pub entries: Vec<Entry>,
}

impl Plan {
    /// Files that need a human before they can be migrated.
    pub fn unmatched(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.skip, Some(Skip::NoTypeRule)))
    }

    /// Files that would be migrated but have no description anyone can derive.
    pub fn undescribed(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.skip.is_none() && e.description_source == Source::None)
    }

    /// Files this run would rewrite.
    pub fn changes(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.rewritten.is_some())
    }
}

/// Work out what migration would do, writing nothing.
#[must_use]
pub fn plan(root: &Path, config: &Config) -> Plan {
    let mut plan = Plan::default();
    for path in walk::markdown_files(root, &config.paths.skip_names) {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = walk::to_posix(relative);
        let text = walk::read_lossy(&path);
        plan.entries.push(plan_one(&relative, &text, config));
    }
    plan
}

fn plan_one(relative: &str, text: &str, config: &Config) -> Entry {
    let mut entry = Entry {
        path: relative.to_owned(),
        concept_type: None,
        rule: None,
        title: None,
        title_source: Source::None,
        description: None,
        description_source: Source::None,
        skip: None,
        rewritten: None,
    };

    let name = relative.rsplit('/').next().unwrap_or(relative);
    if name == "index.md" || name == "log.md" {
        entry.skip = Some(Skip::Reserved);
        return entry;
    }
    if config.is_generated(relative) {
        entry.skip = Some(Skip::Generated);
        return entry;
    }

    let (existing, body) = match parse_strict(text) {
        Ok(fm) => (Some(fm), strip_frontmatter(text)),
        Err(ParseError::NoFence) if !text.starts_with("---") => (None, text),
        Err(ParseError::NoFence) => {
            entry.skip = Some(Skip::Unparseable("unterminated fence".to_owned()));
            return entry;
        }
        Err(other) => {
            entry.skip = Some(Skip::Unparseable(other.message()));
            return entry;
        }
    };

    let has = |key: &str| {
        existing
            .as_ref()
            .is_some_and(|fm| !fm.get_unquoted(key).is_empty())
    };

    // `type` decides whether this file can be migrated at all.
    if has("type") {
        entry.concept_type = existing.as_ref().map(|fm| fm.get_unquoted("type"));
        entry.rule = Some("existing".to_owned());
    } else if let Some((concept_type, rule)) = config.type_for(relative) {
        entry.concept_type = Some(concept_type.to_owned());
        entry.rule = Some(rule.to_owned());
    } else {
        entry.skip = Some(Skip::NoTypeRule);
        return entry;
    }

    if has("title") {
        entry.title_source = Source::Existing;
    } else {
        let (title, source) = derive_title(relative, body);
        entry.title = Some(title);
        entry.title_source = source;
    }

    if has("description") {
        entry.description_source = Source::Existing;
    } else if let Some(description) = derive_description(body) {
        entry.description = Some(description);
        entry.description_source = Source::Paragraph;
    }

    let writes_type = !has("type");
    if !writes_type && entry.title.is_none() && entry.description.is_none() {
        entry.skip = Some(Skip::AlreadyMigrated);
        return entry;
    }
    entry.rewritten = Some(rewrite(text, &entry, writes_type));
    entry
}

/// The first `# ` heading, or the filename turned back into words.
fn derive_title(relative: &str, body: &str) -> (String, Source) {
    if let Some(heading) = crate::index::H1
        .captures(body)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_owned())
        && !heading.is_empty()
    {
        return (heading, Source::Heading);
    }
    let stem = relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .strip_suffix(".md")
        .unwrap_or(relative);
    (title_case(&stem.replace(['-', '_'], " ")), Source::Filename)
}

/// Title-case a filename slug, leaving a word that is already capitalised or
/// an acronym alone.
fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) if first.is_uppercase() => word.to_owned(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A `Key: value` line, the shape a governance metadata block is written in:
/// `Status: accepted`, `Owner: architecture-owner`, `Last reviewed: 2026-06-12`.
///
/// The key is deliberately narrow — one leading capital, then at most thirty
/// more letters, digits, spaces, slashes, hyphens or underscores before the
/// colon — so that an ordinary sentence containing a colon does not match.
static METADATA_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static pattern literal, forced by tests in this module"
    )]
    regex::Regex::new(r"^[A-Z][A-Za-z0-9 /_-]{0,30}:\s*\S")
        .expect("static regex literal must compile")
});

/// Whether a paragraph opens a governance metadata block rather than prose.
///
/// The test is the paragraph's first line, because that is the only line whose
/// shape is reliable. A block's later lines are not all keyed: a long
/// `Review cadence:` or `Status:` value wraps onto continuation lines that are
/// prose in isolation, and a whole-paragraph test therefore misses the block
/// entirely. Measured on `radiology-platform`, requiring every line to be
/// keyed left 101 of 1,633 blocks undetected; requiring only the first line
/// leaves none.
///
/// The narrow key pattern is what stops this eating real prose. It costs three
/// documents across the three largest bundles in the estate — 3,992 documents
/// — and each of those is *reported* as undescribed rather than given a wrong
/// description, which is the same refusal this module applies to `type`.
fn opens_a_metadata_block(para: &str) -> bool {
    para.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|first| METADATA_LINE.is_match(first))
}

/// The first paragraph that is prose: not a heading, list, table row, quote,
/// code fence, comment, or governance metadata block.
///
/// The metadata exclusion is not a nicety. Four repositories in this estate
/// open almost every document with a `Status:` / `Owner:` / `Last reviewed:`
/// block directly under the H1, and without this rule that block becomes the
/// description of 90% of their documents and propagates into every generated
/// listing. Measured on `radiology-platform`: 1,633 of 1,815 documents, of
/// which 1,631 have real prose in a later paragraph. The two that do not are
/// its `0000-template.md` files, which are correctly reported as undescribed.
fn derive_description(body: &str) -> Option<String> {
    let para = PARA_SPLIT.split(body).map(str::trim).find(|p| {
        !p.is_empty()
            && !starts_with_any(p, &["#", "*", "-", "|", ">", "<!--", "```", "1."])
            && !opens_a_metadata_block(p)
    })?;
    let collapsed = para.split_whitespace().collect::<Vec<_>>().join(" ");
    let flattened = MD_LINK.replace_all(&collapsed, "${1}").into_owned();
    let trimmed = truncate_words(&flattened, 200);
    (!trimmed.is_empty()).then_some(trimmed)
}

static LEADING_FRONTMATTER: LazyLock<regex::Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static pattern literal, forced by tests in this module"
    )]
    regex::Regex::new(r"(?s)^---\r?\n.*?\r?\n---\r?\n").expect("static regex literal must compile")
});

fn strip_frontmatter(text: &str) -> &str {
    LEADING_FRONTMATTER
        .find(text)
        .map_or(text, |m| text.get(m.end()..).unwrap_or_default())
}

/// The file's new text: missing keys added, existing ones untouched.
fn rewrite(text: &str, entry: &Entry, writes_type: bool) -> String {
    let mut added = Vec::new();
    if writes_type && let Some(concept_type) = &entry.concept_type {
        added.push(format!("type: {}", quote(concept_type)));
    }
    if let Some(title) = &entry.title {
        added.push(format!("title: {}", quote(title)));
    }
    if let Some(description) = &entry.description {
        added.push(format!("description: {}", quote(description)));
    }
    if added.is_empty() {
        return text.to_owned();
    }
    let block = added.join("\n");

    // Existing frontmatter: new keys go directly after the opening fence, so
    // the keys this tool owns read together and nothing already written moves.
    // Line endings follow the file rather than this tool's preference.
    if let Some(rest) = text.strip_prefix("---\r\n") {
        return format!("---\r\n{}\r\n{rest}", block.replace('\n', "\r\n"));
    }
    if let Some(rest) = text.strip_prefix("---\n") {
        return format!("---\n{block}\n{rest}");
    }
    // No frontmatter: the document gains a block and keeps its body
    // byte-for-byte, including whatever whitespace it opened with.
    format!("---\n{block}\n---\n\n{text}")
}

/// A double-quoted YAML scalar.
///
/// Always quoted, even when it would parse bare: a title beginning `Phase 6:`
/// or containing a `#` is a mapping or a comment unquoted, and the corpus this
/// writes into is full of both.
fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Apply a plan.
///
/// # Errors
///
/// Fails when a file cannot be written.
pub fn apply(root: &Path, plan: &Plan) -> std::io::Result<usize> {
    let mut written: usize = 0;
    for entry in plan.changes() {
        let Some(text) = &entry.rewritten else {
            continue;
        };
        std::fs::write(root.join(&entry.path), text)?;
        written = written.saturating_add(1);
    }
    Ok(written)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "a panicking assertion is the point of a test"
)]
mod tests {
    use super::*;
    use crate::config::{Paths, TypeRule};

    fn config_with(rules: &[(&str, &str)]) -> Config {
        Config {
            type_rules: rules
                .iter()
                .map(|(path, concept_type)| TypeRule {
                    path: (*path).to_owned(),
                    concept_type: (*concept_type).to_owned(),
                })
                .collect(),
            ..Config::default()
        }
    }

    #[test]
    fn a_bare_document_gains_type_title_and_description() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let text = "# Use Postgres\n\nWe pick Postgres because it is already run here.\n";
        let entry = plan_one("adr/0001-db.md", text, &config);

        assert_eq!(entry.concept_type.as_deref(), Some("Decision Record"));
        assert_eq!(entry.title_source, Source::Heading);
        assert_eq!(entry.description_source, Source::Paragraph);
        let out = entry.rewritten.unwrap();
        assert!(out.starts_with(
            "---\ntype: \"Decision Record\"\ntitle: \"Use Postgres\"\ndescription: \"We pick Postgres because it is already run here.\"\n---\n\n# Use Postgres"
        ), "{out}");
    }

    #[test]
    fn an_existing_key_is_never_overwritten() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let text = "---\ntype: \"Runbook\"\ntitle: \"Kept\"\n---\n\n# Ignored heading\n\nProse.\n";
        let entry = plan_one("adr/x.md", text, &config);

        assert_eq!(entry.concept_type.as_deref(), Some("Runbook"));
        assert_eq!(entry.title_source, Source::Existing);
        let out = entry.rewritten.unwrap();
        assert!(out.contains("type: \"Runbook\""), "{out}");
        assert!(out.contains("title: \"Kept\""), "{out}");
        assert!(out.contains("description: \"Prose.\""), "{out}");
        assert_eq!(out.matches("type:").count(), 1, "{out}");
    }

    #[test]
    fn a_file_no_rule_matches_is_reported_and_left_alone() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let entry = plan_one("elsewhere/x.md", "# T\n\nProse.\n", &config);
        assert_eq!(entry.skip, Some(Skip::NoTypeRule));
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn a_generator_owned_file_is_never_written_into() {
        let config = Config {
            paths: Paths {
                generated: vec!["generated/**".to_owned()],
                ..Paths::default()
            },
            ..config_with(&[("**/*.md", "Reference")])
        };
        let entry = plan_one("generated/inventory.md", "# I\n\nProse.\n", &config);
        assert_eq!(entry.skip, Some(Skip::Generated));
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn reserved_filenames_are_left_to_their_own_tools() {
        let config = config_with(&[("**/*.md", "Reference")]);
        assert_eq!(
            plan_one("a/index.md", "# a\n", &config).skip,
            Some(Skip::Reserved)
        );
        assert_eq!(
            plan_one("log.md", "# log\n", &config).skip,
            Some(Skip::Reserved)
        );
    }

    #[test]
    fn a_malformed_block_is_reported_rather_than_migrated_into() {
        let config = config_with(&[("**/*.md", "Reference")]);
        let entry = plan_one("a.md", "---\na: 1\na: 2\n---\n\nProse.\n", &config);
        assert!(matches!(entry.skip, Some(Skip::Unparseable(_))));
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn a_second_pass_finds_nothing_to_do() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let first = plan_one("adr/x.md", "# T\n\nProse.\n", &config)
            .rewritten
            .unwrap();
        let second = plan_one("adr/x.md", &first, &config);
        assert_eq!(second.skip, Some(Skip::AlreadyMigrated));
        assert!(second.rewritten.is_none());
    }

    #[test]
    fn a_governance_metadata_block_is_not_a_description() {
        let text = "# Use Postgres\n\nStatus: accepted\nOwner: architecture-owner\nLast reviewed: 2026-06-12\n\nWe pick Postgres because it is already run here.\n";
        let entry = plan_one(
            "adr/0001-postgres.md",
            text,
            &config_with(&[("adr/*.md", "Decision Record")]),
        );
        assert_eq!(entry.description_source, Source::Paragraph);
        assert_eq!(
            entry.description.as_deref(),
            Some("We pick Postgres because it is already run here.")
        );
    }

    #[test]
    fn a_single_line_status_block_is_skipped_too() {
        // 26 of radiology-platform's 1,815 documents open on exactly this, so
        // a rule requiring two or more lines would leave every one of them
        // describing itself as "Status: accepted".
        let text = "# Amended Report Distribution\n\nStatus: accepted\n\nAmended reports reach every original recipient.\n";
        let entry = plan_one(
            "adr/0070-amended.md",
            text,
            &config_with(&[("adr/*.md", "Decision Record")]),
        );
        assert_eq!(
            entry.description.as_deref(),
            Some("Amended reports reach every original recipient.")
        );
    }

    #[test]
    fn a_metadata_block_with_a_wrapped_value_is_still_a_block() {
        // 101 of radiology-platform's blocks look like this: the last value is
        // long enough to wrap, and the continuation line is prose in
        // isolation. Testing every line would read the whole block as a
        // description; testing the first line reads it as metadata.
        let text = "# Order Intake\n\nStatus: accepted\nOwner: integration-owner\nReview cadence: before changing HL7 order intake, report delivery,\nor billing handoff\n\nHL7 orders arrive over MLLP and are acknowledged synchronously.\n";
        let entry = plan_one(
            "architecture/order-intake.md",
            text,
            &config_with(&[("architecture/*.md", "Architecture Note")]),
        );
        assert_eq!(
            entry.description.as_deref(),
            Some("HL7 orders arrive over MLLP and are acknowledged synchronously.")
        );
    }

    #[test]
    fn a_second_status_paragraph_is_skipped_as_well() {
        // data-lakehouse writes a keyed metadata block, then a separate
        // `Status: **1 of 1 done** (...)` annotation, and only then the real
        // description. Skipping one block and stopping at the next left 92
        // documents describing themselves by their completion count.
        let text = "# Cert Register\n\nOwner: cert3\nReviewed: 2026-06-25\n\nStatus: **1 of 1 done** (`0223` landed the transport\ndescriptor).\n\nA single-issue resolution pass certifying the connector end to end.\n";
        let entry = plan_one(
            "release/cert-register.md",
            text,
            &config_with(&[("release/*.md", "Release Note")]),
        );
        assert_eq!(
            entry.description.as_deref(),
            Some("A single-issue resolution pass certifying the connector end to end.")
        );
    }

    #[test]
    fn a_document_that_is_only_metadata_is_reported_rather_than_described() {
        let text = "# Template\n\nStatus: proposed\nOwner: TBD\n";
        let entry = plan_one(
            "adr/0000-template.md",
            text,
            &config_with(&[("adr/*.md", "Decision Record")]),
        );
        assert_eq!(entry.description_source, Source::None);
        assert_eq!(entry.description, None);
    }

    #[test]
    fn a_title_with_a_colon_or_a_quote_survives_quoting() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let text = "# Phase 6: \"Design\" it\n\nProse.\n";
        let out = plan_one("a/x.md", text, &config).rewritten.unwrap();
        assert!(out.contains(r#"title: "Phase 6: \"Design\" it""#), "{out}");
    }

    #[test]
    fn a_document_with_no_heading_falls_back_to_its_filename() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let entry = plan_one("a/some-long-name.md", "Just prose here.\n", &config);
        assert_eq!(entry.title_source, Source::Filename);
        assert_eq!(entry.title.as_deref(), Some("Some Long Name"));
    }

    #[test]
    fn a_document_with_no_prose_is_reported_as_undescribed() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let entry = plan_one("a/x.md", "# T\n\n* only\n* a list\n", &config);
        assert_eq!(entry.description_source, Source::None);
        assert!(entry.rewritten.is_some(), "it still gains type and title");
    }
}
