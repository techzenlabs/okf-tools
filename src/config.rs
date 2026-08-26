//! `okf.toml`, the per-bundle configuration.
//!
//! Every value here was a hard-coded constant in the Python tools this crate
//! ports. The defaults reproduce those constants exactly, so a bundle with no
//! `okf.toml` at all behaves as the originals did.
//!
//! TOML rather than YAML because a configuration file is not a document, and
//! `config_version` so the tool can refuse a file it does not understand
//! rather than misread it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

/// The only `config_version` this build understands.
pub const SUPPORTED_CONFIG_VERSION: u32 = 1;

/// The bundled type vocabularies, composed by `[vocabulary] extends`.
///
/// They ship inside the binary rather than beside it so a consumer that
/// installs one static file still gets them.
const PRESET_CORE: &str = include_str!("../presets/core.toml");
const PRESET_CAPTURE: &str = include_str!("../presets/capture.toml");
const PRESET_KNOWLEDGE: &str = include_str!("../presets/knowledge.toml");
const PRESET_ENGINEERING: &str = include_str!("../presets/engineering.toml");

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "{path}: config_version {found} is newer than this build understands \
         ({SUPPORTED_CONFIG_VERSION}); upgrade okf-tools"
    )]
    Version { path: String, found: u32 },
    #[error(
        "unknown vocabulary preset `{name}`; known presets are core, capture, knowledge, engineering"
    )]
    UnknownPreset { name: String },
    #[error("invalid title_strip pattern `{pattern}`: {source}")]
    MirrorPattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
    #[error("[[retype]] from = \"{from}\": {why}")]
    RetypeRule { from: String, why: String },
}

/// A path shape and the `type` every document matching it carries.
///
/// Rules are tried in file order and the first match wins, so a more specific
/// shape is written above a more general one. `okf-migrate --report` names the
/// rule that matched each file, which is what makes an ordering mistake
/// visible rather than silent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeRule {
    /// A bundle-relative glob. See [`crate::glob`].
    pub path: String,
    #[serde(rename = "type")]
    pub concept_type: String,
}

/// A vendored upstream mirror whose entry titles carry a site tail.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mirror {
    /// Bundle-relative directory prefixes the rule applies under.
    pub paths: Vec<String>,
    /// A pattern removed from each entry title before it is listed.
    pub title_strip: String,
}

/// One row of the `okf-migrate --retype` rename table.
///
/// A row either renames — `to` names the vocabulary name the old value
/// becomes — or refers every file carrying the old name to a person, and then
/// `review` says why. Exactly one of the two, because a row that says neither
/// is a name somebody forgot to finish and a row that says both is two rules.
///
/// There is no third form that deletes `type`. §11 requires the field, so a
/// name being retired from a vocabulary is a `review` row: the files are
/// listed, and a person retypes or deletes each one. See
/// [`crate::retype`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetypeRule {
    /// The `type` value the documents carry today.
    pub from: String,
    /// The vocabulary name it becomes.
    pub to: Option<String>,
    /// Why a person decides this one, printed beside every file that has it.
    pub review: Option<String>,
}

/// What the rename table says about one old `type` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retype {
    /// Rewrite the value to this name.
    To(String),
    /// Leave every file carrying it exactly as written, and list them.
    Review(String),
}

/// The rename table, keyed by the old `type` value.
pub type RetypeTable = BTreeMap<String, Retype>;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Vocabulary {
    /// Presets composed into this bundle's vocabulary.
    pub extends: Vec<String>,
    /// Names local to this bundle, unioned with the presets.
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Paths {
    /// Directory names never descended into.
    pub skip_names: Vec<String>,
    /// Globs whose files a generator owns.
    ///
    /// `okf-migrate` never writes frontmatter into one: the next regeneration
    /// would truncate it, and a freshness gate would then fail closed. The
    /// generator emits the block or nobody does.
    pub generated: Vec<String>,
    /// Whether `README.md` survives in this bundle.
    ///
    /// True for a code repository, where a docs gate or GitHub itself depends
    /// on the name; false for a knowledge bundle, where §8's generated
    /// `index.md` takes over the listing role.
    ///
    /// Read by `okf-check`, which warns on every `README.md` still present in
    /// a bundle that set this to `false`. See
    /// `check::check_readme_retired`. Nothing deletes a file over it:
    /// the README-to-`index.md` move is the one genuinely destructive step in
    /// this migration and it stays a reviewed commit somebody can revert.
    pub keep_readme: bool,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            skip_names: ["node_modules", "__pycache__", "result"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            generated: Vec::new(),
            keep_readme: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IndexConfig {
    /// Keys permitted in the bundle-root `index.md` frontmatter.
    pub root_keys: Vec<String>,
    /// Directories whose listing is always empty, because their contents are
    /// transient by design.
    pub suppress: Vec<String>,
    /// Directories whose subdirectories are grouped under `## YYYY-MM`.
    pub group_by_month: Vec<String>,
    /// Directories whose immediate children get no index of their own.
    pub no_index_under: Vec<String>,
    /// Within a month-grouped directory, the files listed for each child.
    pub month_entry_glob: String,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            root_keys: ["okf_version", "title", "description"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            suppress: vec!["inbox".to_owned()],
            group_by_month: vec!["meetings".to_owned()],
            no_index_under: vec!["meetings".to_owned()],
            month_entry_glob: "summary*.md".to_owned(),
        }
    }
}

/// The rules a bundle turns on because its reader is outside the estate.
///
/// Every one of these makes an error out of something OKF §11 leaves to
/// convention, so every one of them is **off by default**. §11 forbids a
/// consumer rejecting a bundle over an unknown `type` or an unknown key, and a
/// checker that errored on those unasked would be non-conformant itself. A
/// bundle opts in because the convention it is buying is a confidentiality
/// boundary rather than a style preference: the reader on the other side is a
/// client, and the failure mode is disclosure rather than untidiness.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Confidentiality {
    /// An unknown `type` is an error rather than a warning.
    ///
    /// The vocabulary is closed **by intent**: `knowledge` has no `Person`, so
    /// a person-shaped page fails the type check outright rather than being
    /// listed among the warnings somebody meant to get to.
    pub closed_vocabulary: bool,
    /// A link whose target is not in this bundle is an error.
    ///
    /// The backstop for the case `okf-promote` cannot see, where a page is
    /// hand-copied and a link is missed. Containment is not enough on its own:
    /// a copied `../people/dana-quill.md` resolves *inside* the new bundle
    /// root and simply is not there, so the target has to exist.
    pub links_stay_in_bundle: bool,
    /// `owner` must be a sequence of `{name, title, email}` records, and any
    /// other subkey is an error.
    ///
    /// The schema is the enforcement. A prose convention saying "do not add an
    /// assessment here" is a convention; a record with nowhere to grow is a
    /// boundary.
    pub owner_record: bool,
    /// Absolute URL prefixes this bundle may link to.
    ///
    /// Empty means any `http`/`https` URL is acceptable, which is the reading
    /// that lets a promoted page cite a vendor document. Naming prefixes
    /// narrows it to the tenant's own site and nothing else.
    pub site_urls: Vec<String>,
}

/// A bundle that promoted pages are copied into.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    /// The name `--to` is given.
    pub name: String,
    /// Where the bundle is checked out, relative to this repository's root.
    pub path: String,
    /// The published base URL, joined with the page's bundle-relative path to
    /// give the `promoted_to` value written back into the source.
    pub url: String,
}

/// Where a source page goes, decided before anything is drafted.
///
/// The routing rule is data rather than derivation: a page about a system that
/// has a repository belongs beside that repository's code, and no tool can
/// work out which systems those are. `--to` naming a different bundle than the
/// route is a refusal, not a preference.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// A bundle-relative glob over source pages. See [`crate::glob`].
    pub from: String,
    /// The [`Destination::name`] this shape promotes to.
    pub to: String,
    /// The destination directory the page lands in.
    pub into: String,
}

/// A source repository, as seen from a destination bundle.
///
/// `--refresh` and `--drift` run where both repositories are checked out,
/// which is one person's machine and never a client's runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRepo {
    /// Matched against the `repo` recorded in a page's `promoted_from`.
    pub repo: String,
    /// Where that repository is checked out, relative to this one's root.
    pub path: String,
}

/// `[promote]`, the routing table and the two private classes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PromoteConfig {
    /// Source prefixes holding the interpretive layer. A link into one is
    /// resolved into a plain name or into the `owner` record, never carried.
    pub profile_prefixes: Vec<String>,
    /// Source prefixes holding raw evidence. A claim resting on one is
    /// restated as a dated labelled statement and the citation is dropped.
    pub evidence_prefixes: Vec<String>,
    #[serde(rename = "destination")]
    pub destinations: Vec<Destination>,
    #[serde(rename = "route")]
    pub routes: Vec<Route>,
    #[serde(rename = "source")]
    pub sources: Vec<SourceRepo>,
}

impl Default for PromoteConfig {
    fn default() -> Self {
        Self {
            profile_prefixes: vec!["org/people".to_owned()],
            evidence_prefixes: ["meetings", "emails", "chats", "inbox"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            destinations: Vec::new(),
            routes: Vec::new(),
            sources: Vec::new(),
        }
    }
}

impl PromoteConfig {
    /// The route that claims `relative`, or `None` when nothing does.
    ///
    /// First match wins, as `[[type_rules]]` does, so a page named explicitly
    /// is written above the shape that would otherwise sweep it up.
    #[must_use]
    pub fn route_for(&self, relative: &str) -> Option<&Route> {
        self.routes
            .iter()
            .find(|route| crate::glob::matches(&route.from, relative))
    }

    /// The destination named `name`.
    #[must_use]
    pub fn destination(&self, name: &str) -> Option<&Destination> {
        self.destinations.iter().find(|d| d.name == name)
    }

    /// The checkout recorded for the source repository named `repo`.
    #[must_use]
    pub fn source(&self, repo: &str) -> Option<&SourceRepo> {
        self.sources.iter().find(|s| s.repo == repo)
    }

    /// Which private class, if any, a source-relative path belongs to.
    #[must_use]
    pub fn class_of(&self, source_relative: &str) -> Option<PrivateClass> {
        if under_any(&self.profile_prefixes, source_relative) {
            return Some(PrivateClass::Profile);
        }
        if under_any(&self.evidence_prefixes, source_relative) {
            return Some(PrivateClass::Evidence);
        }
        None
    }
}

/// The two kinds of page a promoted one must never point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateClass {
    /// A profile. The interpretive layer, which never travels.
    Profile,
    /// Raw evidence. The path itself discloses, so the citation is dropped
    /// rather than kept and marked unresolvable.
    Evidence,
}

/// Is `path` at or under one of `prefixes`?
///
/// Segment-aware, so `meetings-policy.md` is not under `meetings`.
fn under_any(prefixes: &[String], path: &str) -> bool {
    prefixes.iter().any(|prefix| {
        let prefix = prefix.trim_end_matches('/');
        path == prefix || path.starts_with(&format!("{prefix}/"))
    })
}

/// The parsed `okf.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub config_version: u32,
    pub okf_version: String,
    /// The bundle root, relative to the repository root: `"."` for a
    /// whole-repo bundle, `"docs"` for a code repository.
    pub bundle_root: String,
    /// Title and description written into the generated root `index.md`.
    pub title: String,
    pub description: String,
    /// The warning budget. A run reporting more than this fails; the count may
    /// fall and may not rise.
    pub max_warnings: usize,
    pub vocabulary: Vocabulary,
    pub paths: Paths,
    pub index: IndexConfig,
    /// The rules a client-facing bundle opts into. All off by default.
    pub confidentiality: Confidentiality,
    pub promote: PromoteConfig,
    #[serde(rename = "mirror")]
    pub mirrors: Vec<Mirror>,
    /// The `okf-migrate --retype` rename table. Empty in every bundle that was
    /// typed rather than retyped, which is all but one of them.
    #[serde(rename = "retype")]
    pub retype: Vec<RetypeRule>,
    #[serde(rename = "type_rules")]
    pub type_rules: Vec<TypeRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: SUPPORTED_CONFIG_VERSION,
            okf_version: "0.2".to_owned(),
            bundle_root: ".".to_owned(),
            title: String::new(),
            description: String::new(),
            max_warnings: usize::MAX,
            vocabulary: Vocabulary::default(),
            paths: Paths::default(),
            index: IndexConfig::default(),
            confidentiality: Confidentiality::default(),
            promote: PromoteConfig::default(),
            mirrors: Vec::new(),
            retype: Vec::new(),
            type_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Preset {
    types: Vec<String>,
}

impl Config {
    /// Load `okf.toml` from `dir`, falling back to the defaults when absent.
    ///
    /// # Errors
    ///
    /// Fails when the file exists but cannot be read or parsed, or declares a
    /// `config_version` this build does not understand.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        let path = dir.join("okf.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: shown.clone(),
            source,
        })?;
        let config: Self = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: shown.clone(),
            source,
        })?;
        if config.config_version > SUPPORTED_CONFIG_VERSION {
            return Err(ConfigError::Version {
                path: shown,
                found: config.config_version,
            });
        }
        Ok(config)
    }

    /// The bundle's type vocabulary: every preset named by `extends`, unioned
    /// with the bundle's own `types`.
    ///
    /// An empty result means no vocabulary was declared, and the caller treats
    /// every type as acceptable rather than warning on all of them.
    ///
    /// # Errors
    ///
    /// Fails when `extends` names a preset that does not ship in this build.
    pub fn types(&self) -> Result<BTreeSet<String>, ConfigError> {
        let mut names = BTreeSet::new();
        for preset in &self.vocabulary.extends {
            let text = match preset.as_str() {
                "core" => PRESET_CORE,
                "capture" => PRESET_CAPTURE,
                "knowledge" => PRESET_KNOWLEDGE,
                "engineering" => PRESET_ENGINEERING,
                other => {
                    return Err(ConfigError::UnknownPreset {
                        name: other.to_owned(),
                    });
                }
            };
            let parsed: Preset = toml::from_str(text).map_err(|source| ConfigError::Parse {
                path: format!("preset:{preset}"),
                source,
            })?;
            names.extend(parsed.types);
        }
        names.extend(self.vocabulary.types.iter().cloned());
        Ok(names)
    }

    /// The `type` for a bundle-relative path, with the rule that decided it.
    ///
    /// `None` means no rule matched. That file is *reported*, never assigned a
    /// default: guessing `type`, the one field §11 requires, would be the same
    /// failure as fabricating `verified`.
    #[must_use]
    pub fn type_for(&self, relative: &str) -> Option<(&str, &str)> {
        self.type_rules
            .iter()
            .find(|rule| crate::glob::matches(&rule.path, relative))
            .map(|rule| (rule.concept_type.as_str(), rule.path.as_str()))
    }

    /// Does a generator own this file?
    #[must_use]
    pub fn is_generated(&self, relative: &str) -> bool {
        self.paths
            .generated
            .iter()
            .any(|pattern| crate::glob::matches(pattern, relative))
    }

    /// The validated `--retype` rename table.
    ///
    /// Four things are refused rather than carried, because a rename table
    /// that lies rewrites a corpus wrongly and nobody reads 672 diffs:
    ///
    /// * a row that names neither `to` nor `review`, which is a name somebody
    ///   started and did not finish;
    /// * a row that names both, which is two rules in one place;
    /// * two rows for one `from`, where which one wins is invisible;
    /// * a `to` this bundle's vocabulary does not hold. The vocabulary is
    ///   closed by intent, and a table is exactly where a name gets invented.
    ///
    /// A row renaming a name to itself is refused too: the thirteen names that
    /// survive a rename table unchanged belong absent from it, not restated in
    /// it, and a `from == to` row reads as a rename that does nothing.
    ///
    /// # Errors
    ///
    /// Fails on any of those, and on an `extends` naming a preset this build
    /// does not ship.
    pub fn retype_table(&self) -> Result<RetypeTable, ConfigError> {
        let vocabulary = self.types()?;
        let mut table = RetypeTable::new();
        for rule in &self.retype {
            let refuse = |why: &str| ConfigError::RetypeRule {
                from: rule.from.clone(),
                why: why.to_owned(),
            };
            let action = match (rule.to.as_deref(), rule.review.as_deref()) {
                (Some(_), Some(_)) => {
                    return Err(refuse(
                        "names both `to` and `review`; a row does one or the other",
                    ));
                }
                (None, None) => {
                    return Err(refuse(
                        "names neither `to` nor `review`; say what it becomes, or why a person decides",
                    ));
                }
                (Some(to), None) if to == rule.from => {
                    return Err(refuse(
                        "renames the name to itself; leave it out of the table instead",
                    ));
                }
                (Some(to), None) if !vocabulary.is_empty() && !vocabulary.contains(to) => {
                    return Err(refuse(&format!(
                        "renames to `{to}`, which this bundle's vocabulary does not hold"
                    )));
                }
                (Some(to), None) => Retype::To(to.to_owned()),
                (None, Some(review)) => Retype::Review(review.to_owned()),
            };
            if table.insert(rule.from.clone(), action).is_some() {
                return Err(refuse("appears twice; one old name has one rule"));
            }
        }
        Ok(table)
    }

    /// Compiled mirror rules, paired as (directory prefix, title pattern).
    ///
    /// # Errors
    ///
    /// Fails when a `title_strip` pattern is not a valid regular expression.
    pub fn mirror_rules(&self) -> Result<Vec<(String, regex::Regex)>, ConfigError> {
        let mut rules = Vec::new();
        for mirror in &self.mirrors {
            let pattern = regex::Regex::new(&mirror.title_strip).map_err(|source| {
                ConfigError::MirrorPattern {
                    pattern: mirror.title_strip.clone(),
                    source,
                }
            })?;
            for dir in &mirror.paths {
                rules.push((dir.clone(), pattern.clone()));
            }
        }
        Ok(rules)
    }
}

/// Does `name` match `pattern`, where `*` matches any run of characters?
///
/// Deliberately not a full glob: the only patterns a bundle configures are
/// filename shapes like `summary*.md`, and a `/` never appears in one.
#[must_use]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return pattern == name;
    };
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut last: Option<&str> = None;
    for part in parts {
        if let Some(previous) = last.take() {
            // A middle segment: find it anywhere in what is left.
            match rest.find(previous) {
                Some(at) => rest = rest.get(at.saturating_add(previous.len())..).unwrap_or(""),
                None => return false,
            }
        }
        last = Some(part);
    }
    match last {
        // No `*` in the pattern at all: it was a literal.
        None => rest.is_empty(),
        Some(tail) => rest.len() >= tail.len() && rest.ends_with(tail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_reproduce_the_python_constants() {
        let config = Config::default();
        assert_eq!(
            config.paths.skip_names,
            ["node_modules", "__pycache__", "result"]
        );
        assert_eq!(config.index.suppress, ["inbox"]);
        assert_eq!(config.index.group_by_month, ["meetings"]);
        assert_eq!(config.index.month_entry_glob, "summary*.md");
        assert_eq!(config.okf_version, "0.2");
    }

    #[test]
    fn every_shipped_preset_parses_and_is_non_empty() {
        for preset in ["core", "capture", "knowledge", "engineering"] {
            let config = Config {
                vocabulary: Vocabulary {
                    extends: vec![preset.to_owned()],
                    types: Vec::new(),
                },
                ..Config::default()
            };
            let types = config.types().unwrap_or_default();
            assert!(!types.is_empty(), "preset {preset} is empty");
        }
    }

    #[test]
    fn an_unknown_preset_is_refused() {
        let config = Config {
            vocabulary: Vocabulary {
                extends: vec!["nope".to_owned()],
                types: Vec::new(),
            },
            ..Config::default()
        };
        assert!(config.types().is_err());
    }

    #[test]
    fn the_confidentiality_rules_are_off_unless_a_bundle_asks() {
        let conf = Config::default().confidentiality;
        assert!(!conf.closed_vocabulary);
        assert!(!conf.links_stay_in_bundle);
        assert!(!conf.owner_record);
        assert!(conf.site_urls.is_empty());
    }

    #[test]
    fn the_first_matching_route_wins_so_a_named_page_beats_a_shape() {
        let config: Config = toml::from_str(
            "[[promote.route]]\n\
             from = \"org/systems/mill.md\"\n\
             to = \"code-repo\"\n\
             into = \"docs/systems\"\n\
             \n\
             [[promote.route]]\n\
             from = \"org/systems/*.md\"\n\
             to = \"knowledge\"\n\
             into = \"systems\"\n",
        )
        .unwrap_or_default();
        assert_eq!(
            config
                .promote
                .route_for("org/systems/mill.md")
                .map(|r| r.to.as_str()),
            Some("code-repo")
        );
        assert_eq!(
            config
                .promote
                .route_for("org/systems/press.md")
                .map(|r| r.to.as_str()),
            Some("knowledge")
        );
        assert!(config.promote.route_for("meetings/x/summary.md").is_none());
    }

    /// A prefix match on a bare string would make `meetings-policy.md` private
    /// and `org/people-process.md` a profile. Neither is.
    #[test]
    fn private_classes_match_whole_path_segments() {
        let promote = PromoteConfig::default();
        assert_eq!(
            promote.class_of("org/people/dana-quill.md"),
            Some(PrivateClass::Profile)
        );
        assert_eq!(
            promote.class_of("meetings/2026-03-04-x/summary.md"),
            Some(PrivateClass::Evidence)
        );
        assert_eq!(promote.class_of("org/people-process.md"), None);
        assert_eq!(promote.class_of("meetings-policy.md"), None);
        assert_eq!(promote.class_of("org/systems/press.md"), None);
    }

    #[test]
    fn glob_matches_the_shapes_a_bundle_configures() {
        assert!(glob_match("summary*.md", "summary.md"));
        assert!(glob_match("summary*.md", "summary-flows.md"));
        assert!(!glob_match("summary*.md", "transcript.txt"));
        assert!(!glob_match("summary*.md", "notes-summary.md"));
        assert!(glob_match("*.md", "a.md"));
        assert!(glob_match("exact.md", "exact.md"));
        assert!(!glob_match("exact.md", "other.md"));
    }
}
