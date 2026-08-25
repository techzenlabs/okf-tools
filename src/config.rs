//! `okf.toml`, the per-bundle configuration.
//!
//! Every value here was a hard-coded constant in the Python tools this crate
//! ports. The defaults reproduce those constants exactly, so a bundle with no
//! `okf.toml` at all behaves as the originals did.
//!
//! TOML rather than YAML because a configuration file is not a document, and
//! `config_version` so the tool can refuse a file it does not understand
//! rather than misread it.

use std::collections::BTreeSet;
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
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            skip_names: ["node_modules", "__pycache__", "result"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
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
    #[serde(rename = "mirror")]
    pub mirrors: Vec<Mirror>,
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
            mirrors: Vec::new(),
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
