//! Writing frontmatter into a bundle that has none.
//!
//! The rule this module is built around is that it does only what is
//! *derivable*, and refuses the rest. `title` comes from the document's own
//! H1, `description` from its own lead paragraph, `type` from a path rule
//! somebody wrote down. Everything else — `tags`, `status`, `stale_after`,
//! `sources`, `verified` — is judgement, and a tool that guessed at it would
//! be asserting things about the knowledge that nobody checked.
//!
//! Three refusals are load-bearing:
//!
//! * **No default type.** A file no `[[type_rules]]` entry matches is
//!   reported and left alone. Guessing `type`, the one field §11 requires,
//!   would be the same failure as fabricating `verified`.
//! * **Never overwrite.** An existing key is left exactly as written, which
//!   is what makes a second run a no-op and a revert a single `git checkout`.
//! * **Do not edit likely generated files.** A generated marker near the start
//!   or a `generated` frontmatter key leaves the file untouched until its path
//!   is listed under `[paths] generated`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use crate::config::Config;
use crate::frontmatter::{Frontmatter, ParseError, parse_strict};
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
    /// The file looks generated but `[paths] generated` does not classify it.
    LikelyGenerated(GeneratedSignal),
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
            Self::LikelyGenerated(signal) => format!("likely-generated: {}", signal.label()),
            Self::NoTypeRule => "no-type-rule".to_owned(),
            Self::Unparseable(why) => format!("unparseable: {why}"),
            Self::AlreadyMigrated => "already-migrated".to_owned(),
        }
    }
}

/// The content signal that made an unclassified file look generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedSignal {
    /// A generated or do-not-edit marker appeared near the start.
    Marker,
    /// The existing frontmatter contains a top-level `generated` key.
    FrontmatterKey,
}

impl GeneratedSignal {
    fn label(self) -> &'static str {
        match self {
            Self::Marker => "marker near start",
            Self::FrontmatterKey => "`generated` frontmatter key",
        }
    }
}

/// The content signal that suggests a generator owns this document.
///
/// The detector is shared with branch adoption surveys so the warning before
/// adoption and the migration refusal after adoption cannot drift apart.
#[must_use]
pub fn generated_signal(text: &str) -> Option<GeneratedSignal> {
    if has_generated_marker(text) {
        return Some(GeneratedSignal::Marker);
    }
    parse_strict(text)
        .ok()
        .filter(|frontmatter| frontmatter.keys().any(|key| key == "generated"))
        .map(|_| GeneratedSignal::FrontmatterKey)
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

    /// Files whose contents suggest a generator owns them, but whose paths
    /// are not classified under `[paths] generated`.
    pub fn likely_generated(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.skip, Some(Skip::LikelyGenerated(_))))
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

/// A tracked non-Markdown file that may bind one document's exact bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PinnedReference {
    /// Bundle-relative path of the document that would change.
    pub document: String,
    /// Repository-relative path of the file that mentions it.
    pub file: String,
    /// One-based line where the document path appears.
    pub path_line: usize,
    /// Nearby key that suggests a byte-level binding.
    pub key: String,
    /// One-based line where the key appears.
    pub key_line: usize,
}

/// How far on either side of a path mention a pin-shaped key may appear.
///
/// Manifests usually keep one record within a handful of lines. Twelve is a
/// conservative textual preflight rather than a claim to parse every manifest
/// format: a false positive costs one review, while a missed binding lets the
/// migration invalidate evidence.
const PIN_CONTEXT_LINES: usize = 12;

static PIN_KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static pattern literal, forced by tests in this module"
    )]
    regex::Regex::new(r#"(?i)(?:^|[\s{,])["']?(sha256|digest|byte_length|size|length)["']?\s*[:=]"#)
        .expect("static regex literal must compile")
});

/// Find changed documents mentioned near byte-binding keys.
///
/// `documents` are bundle-relative paths. `tracked` contains the current text
/// of Git-tracked non-Markdown files and their repository-relative paths. The
/// scan deliberately understands only proximity, not the host file's format;
/// it reports possible bindings for a person to decide rather than asserting
/// that any digest has a particular meaning.
///
/// # Errors
///
/// Fails when the set of literal document paths cannot be compiled into the
/// bounded multi-pattern matcher.
pub fn pinned_references<'document, 'tracked>(
    documents: impl IntoIterator<Item = &'document str>,
    tracked: impl IntoIterator<Item = (&'tracked str, &'tracked str)>,
) -> Result<Vec<PinnedReference>, regex::Error> {
    let documents: Vec<&str> = documents.into_iter().collect();
    if documents.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = regex::RegexSet::new(documents.iter().map(|path| regex::escape(path)))?;
    let mut references = BTreeSet::new();

    for (file, text) in tracked {
        let lines: Vec<&str> = text.lines().collect();
        let keys: Vec<Vec<String>> = lines
            .iter()
            .map(|line| {
                PIN_KEY
                    .captures_iter(line)
                    .filter_map(|captures| captures.get(1).map(|key| key.as_str().to_owned()))
                    .collect()
            })
            .collect();
        if keys.iter().all(Vec::is_empty) {
            continue;
        }

        for (path_index, line) in lines.iter().enumerate() {
            let path_matches = matcher.matches(line);
            if !path_matches.matched_any() {
                continue;
            }
            let first = path_index.saturating_sub(PIN_CONTEXT_LINES);
            let last = path_index
                .saturating_add(PIN_CONTEXT_LINES)
                .min(lines.len().saturating_sub(1));
            for document_index in &path_matches {
                for (key_index, line_keys) in keys.iter().enumerate().take(last + 1).skip(first) {
                    for key in line_keys {
                        references.insert(PinnedReference {
                            document: documents[document_index].to_owned(),
                            file: file.to_owned(),
                            path_line: path_index.saturating_add(1),
                            key: key.to_owned(),
                            key_line: key_index.saturating_add(1),
                        });
                    }
                }
            }
        }
    }

    Ok(references.into_iter().collect())
}

/// How many documents must share a paragraph before it stops being a
/// description and starts being boilerplate.
///
/// Ten is deliberately well clear of coincidence. Two sibling documents can
/// open the same way by accident; ten cannot, and in practice the number seen
/// is in the hundreds — 553 execution plans in `benefits-platform` and 573 in
/// `radiology-platform` each share their plan template's standing sentence.
const BOILERPLATE_SHARED_BY: usize = 10;

/// How many times to look again after removing a layer of boilerplate.
///
/// Each round can only reveal what the previous one was hiding, and in this
/// estate two rounds settle it. Eight is a backstop against a pathological
/// corpus rather than a number anything depends on.
const BOILERPLATE_ROUNDS: usize = 8;

/// Only the opening lines count as a whole-file ownership marker. A generated
/// block later in a hand-written document does not make its frontmatter unsafe
/// to migrate.
const GENERATED_MARKER_LINES: usize = 8;

/// Work out what migration would do, writing nothing.
///
/// Two passes, because whether a paragraph is a description is not a property
/// of the document it sits in. A sentence that hundreds of documents share
/// verbatim describes the *template* they were copied from, and writing it
/// into all of them puts one string in hundreds of listing entries. The first
/// pass finds those; the second re-derives past them.
///
/// The counting pass reads **every** document, including the ones this run
/// would not write to. See [`description_candidates`]: counting only what a
/// run was newly describing made the rule work on a bundle's first migration
/// and fail on every one after it.
#[must_use]
pub fn plan(root: &Path, config: &Config) -> Plan {
    let documents: Vec<(String, String)> = walk::markdown_files(root, &config.paths.skip_names)
        .into_iter()
        .filter_map(|path| {
            let relative = walk::to_posix(path.strip_prefix(root).ok()?);
            Some((relative, walk::read_lossy(&path)))
        })
        .collect();

    // Iterate rather than sweep once. A round only ever sees each document's
    // *current* first choice, so skipping one shared paragraph can reveal a
    // second one behind it — measured on `benefits-platform`, where 553
    // documents share the plan template's sentence and 31 of those then share
    // the next paragraph too. Rounds are bounded because each one strictly
    // grows the set, and a corpus that needed more than a handful is one where
    // no paragraph is a description.
    let mut boilerplate: HashSet<String> = HashSet::new();
    for _ in 0..BOILERPLATE_ROUNDS {
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (relative, text) in &documents {
            for candidate in description_candidates(relative, text, config, &boilerplate) {
                *seen.entry(candidate).or_default() += 1;
            }
        }
        let found: Vec<String> = seen
            .into_iter()
            .filter(|&(_, count)| count >= BOILERPLATE_SHARED_BY)
            .map(|(description, _)| description)
            .collect();
        if found.is_empty() {
            break;
        }
        boilerplate.extend(found);
    }

    let mut plan = Plan::default();
    for (relative, text) in &documents {
        plan.entries
            .push(plan_one(relative, text, config, &boilerplate));
    }
    plan
}

/// `index.md` and `log.md` are generated and hand-maintained; §8 and §9 own
/// their shape, not this tool or [`crate::retype`].
pub(crate) fn is_reserved(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    name == "index.md" || name == "log.md"
}

/// A document this tool is willing to migrate: what its frontmatter already
/// says, its body, and the `type` it will carry.
struct Opened<'a> {
    existing: Option<Frontmatter>,
    body: &'a str,
    concept_type: String,
    rule: String,
    /// Whether `type` was already written, in which case this run leaves it.
    type_existed: bool,
}

impl Opened<'_> {
    /// The scalar at `key`, or `None` when it is absent or empty.
    ///
    /// Absent and empty are the same answer here: a `description:` with
    /// nothing after it describes nothing, and both mean this run may write.
    fn value(&self, key: &str) -> Option<String> {
        let value = self.existing.as_ref()?.get_unquoted(key);
        (!value.is_empty()).then_some(value)
    }
}

/// Open a document, or say why this tool leaves it alone.
///
/// Shared by [`plan_one`] and [`description_candidates`], so the two passes
/// over a bundle cannot disagree about which files they are looking at.
fn open<'a>(relative: &str, text: &'a str, config: &Config) -> Result<Opened<'a>, Skip> {
    if is_reserved(relative) {
        return Err(Skip::Reserved);
    }
    if config.is_generated(relative) {
        return Err(Skip::Generated);
    }
    if let Some(signal) = generated_signal(text) {
        return Err(Skip::LikelyGenerated(signal));
    }

    let (existing, body) = match parse_strict(text) {
        Ok(fm) => (Some(fm), strip_frontmatter(text)),
        Err(ParseError::NoFence) if !text.starts_with("---") => (None, text),
        Err(ParseError::NoFence) => {
            return Err(Skip::Unparseable("unterminated fence".to_owned()));
        }
        Err(other) => return Err(Skip::Unparseable(other.message())),
    };
    let written_type = existing
        .as_ref()
        .map(|fm| fm.get_unquoted("type"))
        .filter(|value| !value.is_empty());

    // `type` decides whether this file can be migrated at all.
    let (concept_type, rule, type_existed) = if let Some(concept_type) = written_type {
        (concept_type, "existing".to_owned(), true)
    } else if let Some((concept_type, rule)) = config.type_for(relative) {
        (concept_type.to_owned(), rule.to_owned(), false)
    } else {
        return Err(Skip::NoTypeRule);
    };

    Ok(Opened {
        existing,
        body,
        concept_type,
        rule,
        type_existed,
    })
}

/// Whether the opening lines carry a whole-file generated marker.
///
/// HTML comments are scanned across line boundaries. An H1 ending in
/// `(generated)` is also a common self-declaration. Both checks stop after the
/// opening lines so a generated block inside hand-written prose stays safe.
fn has_generated_marker(text: &str) -> bool {
    let mut in_comment = false;
    for line in text.lines().take(GENERATED_MARKER_LINES) {
        let lower = line.to_ascii_lowercase();
        let trimmed = lower.trim();
        if trimmed
            .strip_prefix("# ")
            .is_some_and(|heading| heading.trim_end().ends_with("(generated)"))
        {
            return true;
        }

        let mut remaining = lower.as_str();
        loop {
            if !in_comment {
                let Some(start) = remaining.find("<!--") else {
                    break;
                };
                remaining = remaining.get(start.saturating_add(4)..).unwrap_or_default();
                in_comment = true;
            }

            let (comment, rest) = remaining
                .split_once("-->")
                .map_or((remaining, None), |(comment, rest)| (comment, Some(rest)));
            let marks_generated_block =
                comment.contains("begin generated") || comment.contains("end generated");
            if !marks_generated_block
                && (comment.contains("generated") || comment.contains("do not edit"))
            {
                return true;
            }
            let Some(rest) = rest else {
                break;
            };
            in_comment = false;
            remaining = rest;
        }
    }
    false
}

/// Every string a document offers as its description: the one already written
/// in its frontmatter, and the first prose paragraph this tool would derive
/// for it.
///
/// **A document that already carries a `description` offers both, and that is
/// the whole of it.** The counting pass used to read only what a run was newly
/// writing, which made the boilerplate rule work exactly once per bundle. On
/// `benefits-platform`'s first migration 553 execution plans shared their
/// template's standing sentence and none of them got it; migrating one
/// newly-merged plan into the same repository an hour later counted that
/// sentence *once*, because all 553 siblings now had descriptions and
/// contributed nothing, and wrote the template's sentence into the new
/// document. Deriving for a described document as well makes the count 554 on
/// the first incremental run.
///
/// The two are deduplicated, so a document whose written description is also
/// its lead paragraph counts once rather than twice.
fn description_candidates(
    relative: &str,
    text: &str,
    config: &Config,
    boilerplate: &HashSet<String>,
) -> Vec<String> {
    let Ok(opened) = open(relative, text, config) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if let Some(written) = opened.value("description") {
        candidates.push(written);
    }
    if let Some(derived) = derive_description(opened.body, boilerplate)
        && !candidates.contains(&derived)
    {
        candidates.push(derived);
    }
    candidates
}

fn plan_one(relative: &str, text: &str, config: &Config, boilerplate: &HashSet<String>) -> Entry {
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

    let opened = match open(relative, text, config) {
        Ok(opened) => opened,
        Err(skip) => {
            entry.skip = Some(skip);
            return entry;
        }
    };
    entry.concept_type = Some(opened.concept_type.clone());
    entry.rule = Some(opened.rule.clone());

    if opened.value("title").is_some() {
        entry.title_source = Source::Existing;
    } else {
        let (title, source) = derive_title(relative, opened.body);
        entry.title = Some(title);
        entry.title_source = source;
    }

    if opened.value("description").is_some() {
        entry.description_source = Source::Existing;
    } else if let Some(description) = derive_description(opened.body, boilerplate) {
        entry.description = Some(description);
        entry.description_source = Source::Paragraph;
    }

    let writes_type = !opened.type_existed;
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
fn derive_description(body: &str, boilerplate: &HashSet<String>) -> Option<String> {
    PARA_SPLIT
        .split(body)
        .map(str::trim)
        .filter(|p| {
            !p.is_empty()
                && !starts_with_any(p, &["#", "*", "-", "|", ">", "<!--", "```", "1."])
                && !opens_a_metadata_block(p)
        })
        .filter_map(render_description)
        .find(|candidate| !boilerplate.contains(candidate))
}

/// One paragraph, collapsed onto a line and truncated the way a listing wants
/// it. Separate from the choosing above so that both passes render a candidate
/// identically — a boilerplate set built from unrendered paragraphs would
/// never match the rendered ones it is meant to exclude.
fn render_description(para: &str) -> Option<String> {
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
    fn tracked_path_near_byte_binding_keys_is_reported() {
        let references = pinned_references(
            ["adr/0001-db.md"],
            [(
                "model.json",
                "{\n  \"path\": \"adr/0001-db.md\",\n  \"byte_length\": 84,\n  \"sha256\": \"fixture\",\n  \"digest\": \"fixture\",\n  \"size\": 84,\n  \"length\": 84\n}\n",
            )],
        )
        .unwrap();

        assert_eq!(references.len(), 5);
        assert_eq!(references[0].document, "adr/0001-db.md");
        assert_eq!(references[0].file, "model.json");
        assert_eq!(references[0].path_line, 2);
        assert_eq!(references[0].key, "byte_length");
        assert_eq!(references[0].key_line, 3);
        let keys: BTreeSet<&str> = references
            .iter()
            .map(|reference| reference.key.as_str())
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["byte_length", "digest", "length", "sha256", "size"])
        );
    }

    #[test]
    fn distant_or_non_key_length_words_do_not_report_a_pin() {
        let mut at_boundary = String::from("length: 84\n");
        at_boundary.push_str(&"context\n".repeat(PIN_CONTEXT_LINES - 1));
        at_boundary.push_str("adr/0001-db.md\n");
        let mut beyond_boundary = String::from("length: 84\n");
        beyond_boundary.push_str(&"context\n".repeat(PIN_CONTEXT_LINES));
        beyond_boundary.push_str("adr/0001-db.md\n");
        let references = pinned_references(
            ["adr/0001-db.md"],
            [
                ("boundary.txt", at_boundary.as_str()),
                ("distant.txt", beyond_boundary.as_str()),
                (
                    "prose.txt",
                    "adr/0001-db.md has enough length for the example.\n",
                ),
            ],
        )
        .unwrap();

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "boundary.txt");
        assert_eq!(references[0].path_line, PIN_CONTEXT_LINES + 1);
        assert_eq!(references[0].key_line, 1);
    }

    #[test]
    fn a_bare_document_gains_type_title_and_description() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let text = "# Use Postgres\n\nWe pick Postgres because it is already run here.\n";
        let entry = plan_one("adr/0001-db.md", text, &config, &HashSet::new());

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
        let entry = plan_one("adr/x.md", text, &config, &HashSet::new());

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
        let entry = plan_one(
            "elsewhere/x.md",
            "# T\n\nProse.\n",
            &config,
            &HashSet::new(),
        );
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
        let entry = plan_one(
            "generated/inventory.md",
            "<!-- GENERATED: do not edit -->\n# I\n\nProse.\n",
            &config,
            &HashSet::new(),
        );
        assert_eq!(entry.skip, Some(Skip::Generated));
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn a_generated_block_in_a_hand_written_file_is_not_a_marker() {
        let config = config_with(&[("**/*.md", "Reference")]);
        let text = "# Catalog\n\n<!-- BEGIN GENERATED CATALOG -->\nGenerated rows.\n<!-- END GENERATED CATALOG -->\n\nThis introduction is hand-written.\n";
        let entry = plan_one("catalog.md", text, &config, &HashSet::new());

        assert!(entry.skip.is_none());
        assert!(entry.rewritten.is_some());
    }

    #[test]
    fn a_generated_marker_near_the_start_is_reported_and_left_alone() {
        let config = config_with(&[("**/*.md", "Reference")]);
        let entry = plan_one(
            "inventory.md",
            "<!--\nThis file is generated. DO NOT EDIT.\n-->\n\n# Inventory\n\nProse.\n",
            &config,
            &HashSet::new(),
        );

        assert_eq!(
            entry.skip.as_ref().map(Skip::label),
            Some("likely-generated: marker near start".to_owned())
        );
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn a_generated_frontmatter_key_is_reported_and_left_alone() {
        let config = config_with(&[("**/*.md", "Reference")]);
        let entry = plan_one(
            "inventory.md",
            "---\ngenerated: { by: InventoryTool, at: 2026-08-29 }\n---\n\n# Inventory\n\nProse.\n",
            &config,
            &HashSet::new(),
        );

        assert_eq!(
            entry.skip.as_ref().map(Skip::label),
            Some("likely-generated: `generated` frontmatter key".to_owned())
        );
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn reserved_filenames_are_left_to_their_own_tools() {
        let config = config_with(&[("**/*.md", "Reference")]);
        assert_eq!(
            plan_one("a/index.md", "# a\n", &config, &HashSet::new()).skip,
            Some(Skip::Reserved)
        );
        assert_eq!(
            plan_one("log.md", "# log\n", &config, &HashSet::new()).skip,
            Some(Skip::Reserved)
        );
    }

    #[test]
    fn a_malformed_block_is_reported_rather_than_migrated_into() {
        let config = config_with(&[("**/*.md", "Reference")]);
        let entry = plan_one(
            "a.md",
            "---\na: 1\na: 2\n---\n\nProse.\n",
            &config,
            &HashSet::new(),
        );
        assert!(matches!(entry.skip, Some(Skip::Unparseable(_))));
        assert!(entry.rewritten.is_none());
    }

    #[test]
    fn a_second_pass_finds_nothing_to_do() {
        let config = config_with(&[("adr/*.md", "Decision Record")]);
        let first = plan_one("adr/x.md", "# T\n\nProse.\n", &config, &HashSet::new())
            .rewritten
            .unwrap();
        let second = plan_one("adr/x.md", &first, &config, &HashSet::new());
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
            &HashSet::new(),
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
            &HashSet::new(),
        );
        assert_eq!(
            entry.description.as_deref(),
            Some("Amended reports reach every original recipient.")
        );
    }

    #[test]
    fn a_sentence_hundreds_of_documents_share_is_not_a_description() {
        // The plan template's standing sentence. 553 execution plans in
        // `benefits-platform` and 573 in `radiology-platform` open on it, so
        // taking it as the description puts one string in hundreds of listing
        // entries. It describes the template, not the document.
        const BOILERPLATE: &str =
            "This execution plan is a living document. Keep the steps ticked.";
        let root = std::env::temp_dir().join(format!("okf-boilerplate-{}", std::process::id()));
        let plans = root.join("plans");
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&plans).unwrap();
        for n in 0..12 {
            std::fs::write(
                plans.join(format!("{n:04}-thing.md")),
                format!("# Plan {n:04}\n\nStatus: active\nOwner: someone\n\n{BOILERPLATE}\n\nWhat plan {n:04} actually does.\n"),
            )
            .unwrap();
        }
        // One document below the threshold shares nothing, so its own lead
        // paragraph stands even though it sits in the same directory.
        std::fs::write(
            plans.join("9999-singular.md"),
            "# Singular\n\nStatus: active\n\nA plan nobody copied from a template.\n",
        )
        .unwrap();

        let config = config_with(&[("plans/*.md", "Execution Plan")]);
        let plan = plan(&root, &config);
        std::fs::remove_dir_all(&root).ok();

        let described: Vec<&str> = plan
            .entries
            .iter()
            .filter_map(|e| e.description.as_deref())
            .collect();
        assert_eq!(described.len(), 13, "{described:?}");
        assert!(
            !described
                .iter()
                .any(|d| d.starts_with("This execution plan")),
            "the shared sentence was written as a description: {described:?}"
        );
        assert!(
            described.contains(&"A plan nobody copied from a template."),
            "a paragraph nobody shares must still be a description: {described:?}"
        );
        assert!(
            described.contains(&"What plan 0000 actually does."),
            "the paragraph after the boilerplate is the description: {described:?}"
        );
    }

    #[test]
    fn boilerplate_is_still_boilerplate_once_its_siblings_are_described() {
        // The incremental run, which is the one that got this wrong. Twelve
        // plans were migrated a week ago and carry descriptions; one new plan
        // lands, copied from the same template. Counting only what this run
        // would newly describe sees the template's sentence once, and writes
        // it into the new document.
        const BOILERPLATE: &str =
            "This execution plan is a living document. Keep the steps ticked.";
        let root = std::env::temp_dir().join(format!("okf-incremental-{}", std::process::id()));
        let plans = root.join("plans");
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&plans).unwrap();
        for n in 0..12 {
            std::fs::write(
                plans.join(format!("{n:04}-thing.md")),
                format!(
                    "---\ntype: \"Execution Plan\"\ntitle: \"Plan {n:04}\"\ndescription: \"What plan {n:04} actually does.\"\n---\n\n# Plan {n:04}\n\n{BOILERPLATE}\n\nWhat plan {n:04} actually does.\n"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            plans.join("0012-new.md"),
            format!("# Plan 0012\n\n{BOILERPLATE}\n\nWhat plan 0012 actually does.\n"),
        )
        .unwrap();

        let config = config_with(&[("plans/*.md", "Execution Plan")]);
        let plan = plan(&root, &config);
        std::fs::remove_dir_all(&root).ok();

        let new = plan
            .entries
            .iter()
            .find(|e| e.path == "plans/0012-new.md")
            .unwrap();
        assert_eq!(
            new.description.as_deref(),
            Some("What plan 0012 actually does."),
            "the template's sentence was written into the one new document"
        );
        // The twelve siblings are finished and this run must not rewrite them.
        assert_eq!(plan.changes().count(), 1);
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
            &HashSet::new(),
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
            &HashSet::new(),
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
            &HashSet::new(),
        );
        assert_eq!(entry.description_source, Source::None);
        assert_eq!(entry.description, None);
    }

    #[test]
    fn a_title_with_a_colon_or_a_quote_survives_quoting() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let text = "# Phase 6: \"Design\" it\n\nProse.\n";
        let out = plan_one("a/x.md", text, &config, &HashSet::new())
            .rewritten
            .unwrap();
        assert!(out.contains(r#"title: "Phase 6: \"Design\" it""#), "{out}");
    }

    #[test]
    fn a_document_with_no_heading_falls_back_to_its_filename() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let entry = plan_one(
            "a/some-long-name.md",
            "Just prose here.\n",
            &config,
            &HashSet::new(),
        );
        assert_eq!(entry.title_source, Source::Filename);
        assert_eq!(entry.title.as_deref(), Some("Some Long Name"));
    }

    #[test]
    fn a_document_with_no_prose_is_reported_as_undescribed() {
        let config = config_with(&[("a/*.md", "Reference")]);
        let entry = plan_one(
            "a/x.md",
            "# T\n\n* only\n* a list\n",
            &config,
            &HashSet::new(),
        );
        assert_eq!(entry.description_source, Source::None);
        assert!(entry.rewritten.is_some(), "it still gains type and title");
    }
}
