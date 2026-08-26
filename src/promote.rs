//! Promotion: copying a page from a private bundle into a client-facing one.
//!
//! Nothing here moves and nothing here deletes. The source page stays exactly
//! where it was, keeps its meeting links and its people links, and gains one
//! key pointing outward. The promoted page gains the reciprocal, recording the
//! repository, path and commit it came from. References run private to public
//! and never the other way.
//!
//! The whole mechanism is a refusal. `propose` drafts a page and lists every
//! link whose target the destination bundle does not hold, and it writes
//! nothing until that list is empty. A tool that installed a page carrying an
//! unresolved pointer into a profile directory would be worse than no tool,
//! because it would make the disclosure look reviewed.
//!
//! Two classes of unresolved link get different advice, because they need
//! different resolutions:
//!
//! * A **profile link** resolves into a plain name, or into the page's `owner`
//!   record. Contact identity publishes; an assessment of a person does not,
//!   and the link is the pointer into the assessment.
//! * An **evidence link** resolves by restating the claim as a dated statement
//!   carrying its own Confirmed / Assumed / Needs-confirmation label, with the
//!   citation dropped rather than kept and marked unreachable. The path itself
//!   discloses: a directory name carrying a date and a subject is a disclosure
//!   even where the rendered page suppresses the link, because the
//!   raw-markdown route emits the string verbatim.
//!
//! What this module cannot do is recognise characterisation. A sentence
//! carrying a read on somebody is forbidden by the same rule that forbids the
//! profile, and no checker sees it. The gate catches pointers and the reviewer
//! catches characterisation; this is the half that cannot be forgotten.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::config::{Config, PrivateClass};
use crate::frontmatter::{self, ParseError, parse_strict};
use crate::links::{self, Target};

#[expect(
    clippy::expect_used,
    reason = "static pattern literal, forced by tests::every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// The opening frontmatter fence, capturing the block and its trailing newline.
static LEADING_FENCE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?s)^---\r?\n(.*?\r?\n)---\r?\n"));

/// A top-level key line inside a frontmatter block.
static TOP_KEY: LazyLock<Regex> = LazyLock::new(|| compiled(r"^([A-Za-z0-9_][\w.\-]*):"));

#[derive(Debug, thiserror::Error)]
pub enum PromoteError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git {command} in {root}: {message}")]
    Git {
        command: String,
        root: String,
        message: String,
    },
    #[error(
        "{path} matches no [[promote.route]]; the routing rule is data, and a \
         page nobody has routed is a page nobody has decided about"
    )]
    NoRoute { path: String },
    #[error("{path} routes to `{routed}`, not `{asked}`")]
    WrongDestination {
        path: String,
        routed: String,
        asked: String,
    },
    #[error("no [[promote.destination]] named `{name}`")]
    UnknownDestination { name: String },
    #[error(
        "destination bundle `{bundle}` does not set [confidentiality] {missing}; \
         a promoted page may only be installed where the gate that protects it runs"
    )]
    GateOff { bundle: String, missing: String },
    #[error("{path}: no YAML frontmatter (§11.1)")]
    NoFrontmatter { path: String },
    #[error("{path}: {why}")]
    Unparseable { path: String, why: String },
    #[error("{path} has no commit in {root}; promote from a committed source")]
    NoCommit { path: String, root: String },
    #[error("{page}: no `promoted_from` block")]
    MissingProvenance { page: String },
    #[error("{path}: {count} unresolved item(s) stand; nothing was written")]
    Blocked { path: String, count: usize },
    #[error("no [[promote.source]] names repository `{repo}`, recorded by {page}")]
    UnknownSourceRepo { repo: String, page: String },
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
}

/// Where a commit comes from.
///
/// An injected port rather than a `Command` call buried in the middle of the
/// drafting logic, so the fixtures can promote from a tree that is not a
/// repository and still assert on the recorded provenance.
pub trait Revisions {
    /// The `owner/name` of the repository checked out at `root`.
    ///
    /// # Errors
    ///
    /// Fails when the repository has no `origin` remote to name it by.
    fn repo_name(&self, root: &Path) -> Result<String, PromoteError>;

    /// The commit that last touched `relative`.
    ///
    /// # Errors
    ///
    /// Fails when the path has never been committed, which is what makes
    /// `promoted_from.rev` a real commit rather than a plausible one.
    fn last_rev(&self, root: &Path, relative: &str) -> Result<String, PromoteError>;

    /// The content of `relative` as of `rev`.
    ///
    /// # Errors
    ///
    /// Fails when the commit or the path is not in the repository.
    fn blob_at(&self, root: &Path, rev: &str, relative: &str) -> Result<String, PromoteError>;
}

/// `git`, as installed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Git;

impl Git {
    fn run(root: &Path, args: &[&str]) -> Result<String, PromoteError> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map_err(|source| PromoteError::Io {
                path: format!("git {}", args.join(" ")),
                source,
            })?;
        if !output.status.success() {
            return Err(PromoteError::Git {
                command: args.join(" "),
                root: root.display().to_string(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Revisions for Git {
    fn repo_name(&self, root: &Path) -> Result<String, PromoteError> {
        let url = Self::run(root, &["remote", "get-url", "origin"])?;
        let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
        let tail: Vec<&str> = url.rsplit(['/', ':']).take(2).collect();
        Ok(tail.into_iter().rev().collect::<Vec<_>>().join("/"))
    }

    fn last_rev(&self, root: &Path, relative: &str) -> Result<String, PromoteError> {
        let rev = Self::run(root, &["log", "-1", "--format=%H", "--", relative])?;
        let rev = rev.trim().to_owned();
        if rev.is_empty() {
            return Err(PromoteError::NoCommit {
                path: relative.to_owned(),
                root: root.display().to_string(),
            });
        }
        Ok(rev)
    }

    fn blob_at(&self, root: &Path, rev: &str, relative: &str) -> Result<String, PromoteError> {
        // `<rev>:<path>` is resolved against the *repository* root, while
        // `log -- <path>` is resolved against the working directory. Where the
        // bundle root is a subdirectory those two disagree, and a lookup that
        // is right in production and wrong in a fixture is worse than one that
        // is wrong in both. `./` makes the pathspec working-directory relative,
        // so both agree everywhere.
        Self::run(root, &["show", &format!("{rev}:./{relative}")])
    }
}

/// Does an item block the write, or only inform the reviewer?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Nothing is written while one of these stands.
    Unresolved,
    /// Worth saying, and not a reason to refuse.
    Note,
}

/// What kind of thing the reviewer has to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A pointer into the interpretive layer.
    ProfileLink,
    /// A citation of raw evidence the reader cannot open.
    EvidenceLink,
    /// A target the destination bundle does not hold.
    OutsideLink,
    /// An absolute URL outside what the bundle may carry.
    ForeignUrl,
    /// A `type` the destination's vocabulary does not have.
    ForeignType,
    /// No `owner` record on the promoted page.
    MissingOwner,
    /// An `owner` record the destination's schema does not have room for.
    OwnerRecord,
}

impl Kind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ProfileLink => "profile-link",
            Self::EvidenceLink => "evidence-link",
            Self::OutsideLink => "outside-link",
            Self::ForeignUrl => "foreign-url",
            Self::ForeignType => "foreign-type",
            Self::MissingOwner => "missing-owner",
            Self::OwnerRecord => "owner-record",
        }
    }

    fn severity(self) -> Severity {
        match self {
            Self::MissingOwner => Severity::Note,
            _ => Severity::Unresolved,
        }
    }
}

/// One line of the resolution report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    /// 1-based line in the draft, or 0 when the item is about the page itself.
    pub line: usize,
    /// The link target, the type name, or whatever the item is about.
    pub subject: String,
    /// The sentence the subject appears in.
    pub sentence: String,
    /// What to put there instead.
    pub replacement: String,
}

impl Item {
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }

    /// The identity two reports are compared on, so `--refresh` can say which
    /// items are new since the recorded commit rather than listing all of them
    /// again.
    fn identity(&self) -> (Kind, &str, &str) {
        (self.kind, self.subject.as_str(), self.sentence.as_str())
    }
}

/// A drafted page and everything standing between it and the bundle.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// Bundle-relative path in the source.
    pub source_path: String,
    /// The destination bundle's name.
    pub destination: String,
    /// Bundle-relative path in the destination.
    pub destination_path: String,
    /// The published URL, written back into the source as `promoted_to`.
    pub url: String,
    /// The full text of the page that would be installed.
    pub draft: String,
    pub items: Vec<Item>,
}

impl Proposal {
    /// Items that stop the write.
    pub fn unresolved(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .filter(|i| i.severity() == Severity::Unresolved)
    }

    /// Is anything unresolved?
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.unresolved().next().is_some()
    }
}

/// The provenance written into a promoted page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub repo: String,
    pub path: String,
    pub rev: String,
}

/// Draft a promotion and review it, writing nothing.
///
/// `draft_body` replaces the source's text when the reviewer has already
/// restated it; the frontmatter's pointer keys are rewritten either way, so a
/// hand-edited draft cannot carry a stale `promoted_from`.
///
/// # Errors
///
/// Fails when the page is unrouted, routed somewhere other than `to`, has no
/// frontmatter, has never been committed, or when the destination bundle has
/// not turned the confidentiality gates on.
pub fn propose(
    source: &Bundle,
    source_relative: &str,
    to: &str,
    destination: &Bundle,
    draft_body: Option<&str>,
    revisions: &dyn Revisions,
) -> Result<Proposal, PromoteError> {
    let route = source
        .config
        .promote
        .route_for(source_relative)
        .ok_or_else(|| PromoteError::NoRoute {
            path: source_relative.to_owned(),
        })?;
    if route.to != to {
        return Err(PromoteError::WrongDestination {
            path: source_relative.to_owned(),
            routed: route.to.clone(),
            asked: to.to_owned(),
        });
    }
    let target =
        source
            .config
            .promote
            .destination(to)
            .ok_or_else(|| PromoteError::UnknownDestination {
                name: to.to_owned(),
            })?;
    require_gates(destination, to)?;

    let name = source_relative
        .rsplit('/')
        .next()
        .unwrap_or(source_relative);
    let into = route.into.trim_matches('/');
    let destination_path = if into.is_empty() {
        name.to_owned()
    } else {
        format!("{into}/{name}")
    };

    let source_text = match draft_body {
        Some(text) => text.to_owned(),
        None => read(&source.root.join(source_relative))?,
    };
    if let Err(err) = parse_strict(&source_text) {
        return Err(match err {
            ParseError::NoFence => PromoteError::NoFrontmatter {
                path: source_relative.to_owned(),
            },
            other => PromoteError::Unparseable {
                path: source_relative.to_owned(),
                why: other.message(),
            },
        });
    }

    let provenance = Provenance {
        repo: revisions.repo_name(&source.root)?,
        path: source_relative.to_owned(),
        rev: revisions.last_rev(&source.root, source_relative)?,
    };
    let draft = rewrite_frontmatter(&source_text, &provenance, source_relative)?;
    let items = review(
        &draft,
        &destination_path,
        destination,
        source,
        source_relative,
    )?;

    Ok(Proposal {
        source_path: source_relative.to_owned(),
        destination: to.to_owned(),
        url: published_url(&target.url, &destination_path),
        destination_path,
        draft,
        items,
    })
}

/// A bundle root paired with the configuration that describes it.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub root: PathBuf,
    pub config: Config,
    /// The name used in diagnostics.
    pub label: String,
}

impl Bundle {
    /// Open the bundle whose repository root is `repo_root`.
    ///
    /// # Errors
    ///
    /// Fails when `okf.toml` cannot be read or understood.
    pub fn open(repo_root: &Path, label: &str) -> Result<Self, PromoteError> {
        let (root, config) = crate::open_bundle(repo_root)?;
        Ok(Self {
            root,
            config,
            label: label.to_owned(),
        })
    }
}

/// A promoted page may only be installed where the gate protecting it runs.
///
/// The interlock exists because the three rules are off by default, and off by
/// default is right: §11 forbids a consumer rejecting a bundle over an unknown
/// key. What it must not mean is that somebody creates a client-facing bundle,
/// forgets the config, and gets a tool that cheerfully installs into it.
fn require_gates(destination: &Bundle, name: &str) -> Result<(), PromoteError> {
    let conf = &destination.config.confidentiality;
    let mut missing = Vec::new();
    if !conf.closed_vocabulary {
        missing.push("closed_vocabulary");
    }
    if !conf.links_stay_in_bundle {
        missing.push("links_stay_in_bundle");
    }
    if !conf.owner_record {
        missing.push("owner_record");
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(PromoteError::GateOff {
        bundle: name.to_owned(),
        missing: missing.join(", "),
    })
}

fn published_url(base: &str, destination_path: &str) -> String {
    let page = destination_path
        .strip_suffix(".md")
        .unwrap_or(destination_path);
    format!("{}/{page}", base.trim_end_matches('/'))
}

fn read(path: &Path) -> Result<String, PromoteError> {
    std::fs::read_to_string(path).map_err(|source| PromoteError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Every decision the reviewer still owes, in draft order.
///
/// # Errors
///
/// Fails when the destination's vocabulary cannot be resolved.
fn review(
    draft: &str,
    destination_path: &str,
    destination: &Bundle,
    source: &Bundle,
    source_relative: &str,
) -> Result<Vec<Item>, PromoteError> {
    let destination_dir = destination_path.rsplit_once('/').map_or("", |(d, _)| d);
    let source_dir = source_relative.rsplit_once('/').map_or("", |(d, _)| d);
    let mut items = Vec::new();

    for link in links::links(draft) {
        if let Some(item) = judge(
            Written {
                raw: &link.target,
                line: link.line,
                sentence: &link.sentence,
                destination_dir,
                source_dir,
            },
            destination,
            source,
        ) {
            items.push(item);
        }
    }
    // A `sources` entry is bundle-relative rather than page-relative, and it
    // discloses exactly as a body link does. The link scanner never sees it,
    // because it is not a link.
    for (line, entry) in frontmatter::nested_items(draft, "sources") {
        if entry.split_whitespace().count() != 1 {
            continue;
        }
        if let Some(item) = judge(
            Written {
                raw: &entry,
                line,
                sentence: &format!("sources: - {entry}"),
                destination_dir: "",
                source_dir: "",
            },
            destination,
            source,
        ) {
            items.push(item);
        }
    }
    // File order, so the reviewer walks the page rather than this function's
    // two passes over it.
    items.sort_by_key(|item| item.line);

    let types = destination.config.types()?;
    let declared = parse_strict(draft)
        .map(|fm| fm.get_unquoted("type"))
        .unwrap_or_default();
    if !types.is_empty() && !declared.is_empty() && !types.contains(&declared) {
        items.push(Item {
            kind: Kind::ForeignType,
            line: 0,
            subject: declared.clone(),
            sentence: format!("type: {declared}"),
            replacement: format!(
                "a type this bundle holds: {}",
                types.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        });
    }
    for (line, message) in crate::check::owner_errors(draft) {
        items.push(Item {
            kind: Kind::OwnerRecord,
            line,
            subject: message,
            sentence: String::new(),
            replacement: "a record carrying only `name`, `title` and `email`. The record is \
                          constructed from the source's owner bullet cross-checked against a \
                          profile, never sliced out of the profile, and the schema is what \
                          keeps it from growing back into one."
                .to_owned(),
        });
    }
    if frontmatter::nested_records(draft, "owner").is_empty() {
        items.push(Item {
            kind: Kind::MissingOwner,
            line: 0,
            subject: String::new(),
            sentence: String::new(),
            replacement: "an `owner` record constructed from the source's `**Owner(s):**` \
                          bullet, cross-checked against each profile's `**Responsible for:**` \
                          bullet. Neither bullet publishes. Leave `email` out rather than \
                          guessing one, and say in the promotion note that it is a gap."
                .to_owned(),
        });
    }
    Ok(items)
}

/// One target, and the two directories it resolves against.
#[derive(Clone, Copy)]
struct Written<'a> {
    raw: &'a str,
    line: usize,
    sentence: &'a str,
    /// The drafted page's directory in the destination, which decides whether
    /// the target resolves.
    destination_dir: &'a str,
    /// The source page's directory, which decides what class it belongs to and
    /// therefore what advice the reviewer gets.
    source_dir: &'a str,
}

/// Judge one target as the destination bundle would see it.
fn judge(written: Written<'_>, destination: &Bundle, source: &Bundle) -> Option<Item> {
    let Written {
        raw,
        line,
        sentence,
        destination_dir,
        source_dir,
    } = written;
    let target = links::classify(raw, destination_dir);
    match &target {
        Target::Fragment => None,
        Target::Url { .. } => (!target
            .url_is_reachable(&destination.config.confidentiality.site_urls, raw))
        .then(|| Item {
            kind: Kind::ForeignUrl,
            line,
            subject: raw.to_owned(),
            sentence: sentence.to_owned(),
            replacement: "a URL under this bundle's own site, or a restatement with no link"
                .to_owned(),
        }),
        Target::Inside { path } if destination.root.join(path).exists() => None,
        Target::Inside { .. } | Target::Escapes => {
            let in_source = match links::classify(raw, source_dir) {
                Target::Inside { path } => path,
                _ => String::new(),
            };
            let kind = match source.config.promote.class_of(&in_source) {
                Some(PrivateClass::Profile) => Kind::ProfileLink,
                Some(PrivateClass::Evidence) => Kind::EvidenceLink,
                None => Kind::OutsideLink,
            };
            Some(Item {
                kind,
                line,
                subject: raw.to_owned(),
                sentence: sentence.to_owned(),
                replacement: advice(kind).to_owned(),
            })
        }
    }
}

fn advice(kind: Kind) -> &'static str {
    match kind {
        Kind::ProfileLink => {
            "the person's plain name. A name in prose is contact identity and publishes; \
             the link is a pointer into the interpretive layer and does not. If the \
             sentence states who owns this system, move it into the `owner` record and \
             delete the pointer."
        }
        Kind::EvidenceLink => {
            "a dated statement carrying its Confirmed / Assumed / Needs-confirmation \
             label, with the citation dropped. Do not keep the link and mark it \
             unreachable: the path names a meeting, its date and its subject, and the \
             raw-markdown route emits that string verbatim."
        }
        _ => {
            "a resource this bundle holds, or a restatement with no link. The reader \
             cannot open anything outside the bundle."
        }
    }
}

/// The draft's frontmatter: the two pointer keys dropped, `promoted_from`
/// appended.
///
/// `promoted_to` never travels — it points back at the published page, and on
/// the published page it would be a self-reference — and `promoted_from` is
/// recomputed rather than carried, so a hand-edited draft cannot install a
/// stale commit.
fn rewrite_frontmatter(
    text: &str,
    provenance: &Provenance,
    path: &str,
) -> Result<String, PromoteError> {
    let caps = LEADING_FENCE
        .captures(text)
        .ok_or_else(|| PromoteError::NoFrontmatter {
            path: path.to_owned(),
        })?;
    let (Some(block), Some(whole)) = (caps.get(1), caps.get(0)) else {
        return Err(PromoteError::NoFrontmatter {
            path: path.to_owned(),
        });
    };
    let newline = if block.as_str().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let kept = drop_keys(block.as_str(), &["promoted_to", "promoted_from"]);
    let provenance_block = [
        "promoted_from:".to_owned(),
        format!("  repo: {}", quote(&provenance.repo)),
        format!("  path: {}", quote(&provenance.path)),
        format!("  rev: {}", quote(&provenance.rev)),
    ]
    .join(newline);
    let body = text.get(whole.end()..).unwrap_or_default();
    Ok(format!(
        "---{newline}{kept}{provenance_block}{newline}---{newline}{body}"
    ))
}

/// A frontmatter block with the named top-level keys and their indented
/// continuations removed. The trailing newline is kept.
fn drop_keys(block: &str, keys: &[&str]) -> String {
    let mut out = String::new();
    let mut dropping = false;
    for line in block.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if let Some(name) = TOP_KEY.captures(bare).and_then(|c| c.get(1)) {
            dropping = keys.contains(&name.as_str());
        } else if !bare.starts_with([' ', '\t', '-']) && !bare.trim().is_empty() {
            dropping = false;
        }
        if !dropping {
            out.push_str(line);
        }
    }
    out
}

/// A double-quoted YAML scalar, as `okf-migrate` writes them.
fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Install a proposal into the destination bundle.
///
/// # Errors
///
/// Refuses a blocked proposal, and fails on any write error.
pub fn install(destination: &Bundle, proposal: &Proposal) -> Result<PathBuf, PromoteError> {
    if proposal.blocked() {
        return Err(PromoteError::Blocked {
            path: proposal.destination_path.clone(),
            count: proposal.unresolved().count(),
        });
    }
    let path = destination.root.join(&proposal.destination_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PromoteError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(&path, &proposal.draft).map_err(|source| PromoteError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

/// Point the source page at its published copy.
///
/// Returns `false` when the key was already there with this value, so a second
/// run is a no-op and a revert is one `git checkout`.
///
/// # Errors
///
/// Fails when the source cannot be read or written, or has no frontmatter.
pub fn write_source_pointer(
    source: &Bundle,
    source_relative: &str,
    url: &str,
) -> Result<bool, PromoteError> {
    let path = source.root.join(source_relative);
    let text = read(&path)?;
    let caps = LEADING_FENCE
        .captures(&text)
        .ok_or_else(|| PromoteError::NoFrontmatter {
            path: source_relative.to_owned(),
        })?;
    let (Some(block), Some(whole)) = (caps.get(1), caps.get(0)) else {
        return Err(PromoteError::NoFrontmatter {
            path: source_relative.to_owned(),
        });
    };
    if parse_strict(&text)
        .map(|fm| fm.get_unquoted("promoted_to"))
        .unwrap_or_default()
        == url
    {
        return Ok(false);
    }
    let newline = if block.as_str().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let kept = drop_keys(block.as_str(), &["promoted_to"]);
    let body = text.get(whole.end()..).unwrap_or_default();
    let updated = format!(
        "---{newline}{kept}promoted_to: {}{newline}---{newline}{body}",
        quote(url)
    );
    std::fs::write(&path, updated).map_err(|source| PromoteError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(true)
}

/// What a page's `promoted_from` records.
///
/// # Errors
///
/// Fails when the page carries no `promoted_from` block.
pub fn provenance_of(page: &str, text: &str) -> Result<Provenance, PromoteError> {
    let map = frontmatter::nested_map(text, "promoted_from");
    let get = |key: &str| map.get(key).cloned().unwrap_or_default();
    let (repo, path, rev) = (get("repo"), get("path"), get("rev"));
    if repo.is_empty() || path.is_empty() || rev.is_empty() {
        return Err(PromoteError::MissingProvenance {
            page: page.to_owned(),
        });
    }
    Ok(Provenance { repo, path, rev })
}

/// Every page in a bundle that carries `promoted_from`, with what it records.
#[must_use]
pub fn promoted_pages(bundle: &Bundle) -> Vec<(String, Provenance)> {
    let mut found = Vec::new();
    for path in crate::walk::markdown_files(&bundle.root, &bundle.config.paths.skip_names) {
        let Ok(relative) = path.strip_prefix(&bundle.root) else {
            continue;
        };
        let name = crate::walk::to_posix(relative);
        let text = crate::walk::read_lossy(&path);
        if let Ok(provenance) = provenance_of(&name, &text) {
            found.push((name, provenance));
        }
    }
    found
}

/// One page whose source has moved since it was promoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub page: String,
    pub provenance: Provenance,
    /// The commit that touches the source path now.
    pub current: String,
}

/// Pages whose source has moved.
///
/// Runs only where both repositories are checked out, which is one person's
/// machine and never a client's runner: it reads the private repository to
/// answer a question about the public one.
///
/// # Errors
///
/// Fails when a page records a repository no `[[promote.source]]` names, which
/// is a configuration gap rather than a drift finding and must not be reported
/// as "no drift".
pub fn drift(destination: &Bundle, revisions: &dyn Revisions) -> Result<Vec<Drift>, PromoteError> {
    let mut moved = Vec::new();
    for (page, provenance) in promoted_pages(destination) {
        let root = source_root(destination, &page, &provenance.repo)?;
        let current = revisions.last_rev(&root, &provenance.path)?;
        if current != provenance.rev {
            moved.push(Drift {
                page,
                provenance,
                current,
            });
        }
    }
    Ok(moved)
}

fn source_root(destination: &Bundle, page: &str, repo: &str) -> Result<PathBuf, PromoteError> {
    let entry =
        destination
            .config
            .promote
            .source(repo)
            .ok_or_else(|| PromoteError::UnknownSourceRepo {
                repo: repo.to_owned(),
                page: page.to_owned(),
            })?;
    Ok(destination.root.join(&entry.path))
}

/// What `--refresh` found for one promoted page.
#[derive(Debug, Clone)]
pub struct Refreshed {
    pub page: String,
    pub provenance: Provenance,
    /// The commit that touches the source path now.
    pub current: String,
    /// The page as promotion would draft it from the source as it stands.
    pub redraft: String,
    /// Items the source has grown since it was promoted.
    pub new_items: Vec<Item>,
}

impl Refreshed {
    /// Has the source moved at all?
    #[must_use]
    pub fn moved(&self) -> bool {
        self.current != self.provenance.rev
    }

    /// Does anything block bumping the recorded commit?
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.new_items
            .iter()
            .any(|i| i.severity() == Severity::Unresolved)
    }
}

/// Redraw a promoted page from its source and report what is new.
///
/// The redraw is **not** installed over the page. A promoted page is not a
/// mechanical copy — its meeting-backed claims were restated by hand — so
/// overwriting it with a fresh draft would delete the work the gate exists to
/// require. What refreshing is for is the question a person cannot answer by
/// looking: has the source grown a pointer since this was promoted? So the
/// report is a *difference* between the draft at the recorded commit and the
/// draft as the source stands, and the only thing written is the commit.
///
/// # Errors
///
/// Fails when the page has no provenance, the source repository is not
/// configured or checked out, or the recorded commit is not in it.
pub fn refresh(
    destination: &Bundle,
    page: &str,
    revisions: &dyn Revisions,
) -> Result<Refreshed, PromoteError> {
    let text = read(&destination.root.join(page))?;
    let provenance = provenance_of(page, &text)?;
    let root = source_root(destination, page, &provenance.repo)?;
    let source = Bundle::open(&root, &provenance.repo)?;
    let current = revisions.last_rev(&root, &provenance.path)?;

    let then = revisions.blob_at(&root, &provenance.rev, &provenance.path)?;
    let now = read(&source.root.join(&provenance.path))?;

    let destination_path = page.to_owned();
    let before = review_text(&then, &destination_path, destination, &source, &provenance)?;
    let after = review_text(&now, &destination_path, destination, &source, &provenance)?;

    let known: Vec<(Kind, &str, &str)> = before.1.iter().map(Item::identity).collect();
    let new_items = after
        .1
        .iter()
        .filter(|item| !known.contains(&item.identity()))
        .cloned()
        .collect();

    Ok(Refreshed {
        page: page.to_owned(),
        provenance,
        current,
        redraft: after.0,
        new_items,
    })
}

/// Draft `text` as if promoting it, and review the result.
fn review_text(
    text: &str,
    destination_path: &str,
    destination: &Bundle,
    source: &Bundle,
    provenance: &Provenance,
) -> Result<(String, Vec<Item>), PromoteError> {
    let draft = rewrite_frontmatter(text, provenance, &provenance.path)?;
    let items = review(
        &draft,
        destination_path,
        destination,
        source,
        &provenance.path,
    )?;
    Ok((draft, items))
}

/// Record a refreshed commit on the promoted page.
///
/// # Errors
///
/// Fails on any write error, or when the page has no frontmatter.
pub fn bump_rev(destination: &Bundle, page: &str, rev: &str) -> Result<(), PromoteError> {
    let path = destination.root.join(page);
    let text = read(&path)?;
    let provenance = provenance_of(page, &text)?;
    let updated = rewrite_frontmatter(
        &text,
        &Provenance {
            rev: rev.to_owned(),
            ..provenance
        },
        page,
    )?;
    std::fs::write(&path, updated).map_err(|source| PromoteError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "a panicking assertion is the point of a test"
)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(LEADING_FENCE.is_match("---\na: b\n---\n"));
        assert!(TOP_KEY.is_match("a: b"));
    }

    fn provenance() -> Provenance {
        Provenance {
            repo: "example/notes".to_owned(),
            path: "org/systems/press.md".to_owned(),
            rev: "0f1e2d3c".to_owned(),
        }
    }

    #[test]
    fn the_two_pointer_keys_are_replaced_and_everything_else_survives() {
        let text = "---\ntype: System\ntitle: \"Press\"\npromoted_to: \"https://old/x\"\n\
                    promoted_from:\n  repo: \"stale/repo\"\n  rev: \"deadbeef\"\ntags:\n  - a\n\
                    ---\n\nBody stays.\n";
        let out = rewrite_frontmatter(text, &provenance(), "p.md").unwrap_or_default();
        assert!(!out.contains("https://old/x"), "{out}");
        assert!(!out.contains("stale/repo"), "{out}");
        assert!(!out.contains("deadbeef"), "{out}");
        assert!(out.contains("type: System"), "{out}");
        assert!(out.contains("tags:\n  - a\n"), "{out}");
        assert!(out.contains("  rev: \"0f1e2d3c\""), "{out}");
        assert!(out.ends_with("---\n\nBody stays.\n"), "{out}");
    }

    #[test]
    fn a_crlf_source_keeps_its_line_endings() {
        let text = "---\r\ntype: System\r\n---\r\nbody\r\n";
        let out = rewrite_frontmatter(text, &provenance(), "p.md").unwrap_or_default();
        assert!(out.contains("promoted_from:\r\n  repo:"), "{out:?}");
        assert!(!out.contains("promoted_from:\n  repo:"), "{out:?}");
    }

    #[test]
    fn a_page_with_no_frontmatter_cannot_be_drafted() {
        let err = rewrite_frontmatter("just prose\n", &provenance(), "p.md").unwrap_err();
        assert!(matches!(err, PromoteError::NoFrontmatter { .. }));
    }

    #[test]
    fn the_published_url_drops_the_extension_and_one_slash() {
        assert_eq!(
            published_url("https://docs.example.test/knowledge/", "systems/press.md"),
            "https://docs.example.test/knowledge/systems/press"
        );
    }

    #[test]
    fn provenance_needs_all_three_keys() {
        let text = "---\npromoted_from:\n  repo: \"e/n\"\n  path: \"a.md\"\n---\n";
        assert!(provenance_of("p.md", text).is_err());
        let text =
            "---\npromoted_from:\n  repo: \"e/n\"\n  path: \"a.md\"\n  rev: \"c0ffee\"\n---\n";
        assert_eq!(
            provenance_of("p.md", text)
                .unwrap_or_else(|_| provenance())
                .rev,
            "c0ffee"
        );
    }
}
