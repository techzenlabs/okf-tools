//! Read-only checks that make an OKF adoption reviewable.

use std::collections::BTreeSet;
use std::path::Path;

use crate::config::Config;

/// Why a document on another branch needs attention before adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// A configured path or a content signal says a generator owns the file.
    Generated,
    /// No type rule covers the path.
    Untypable,
    /// The document is non-conformant as written.
    WouldFail,
}

impl Classification {
    /// Stable label used by the command's tabular output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Untypable => "untypable",
            Self::WouldFail => "would fail",
        }
    }
}

/// One document found only on a remote-tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub classification: Classification,
}

/// Findings for one remote-tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSurvey {
    pub branch: String,
    pub findings: Vec<Finding>,
}

/// A failure to inspect the repository or classify a document.
#[derive(Debug, thiserror::Error)]
pub enum SurveyError {
    #[error("could not run `git {command}`: {source}")]
    GitIo {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`git {command}` failed in {root}: {message}")]
    Git {
        command: String,
        root: String,
        message: String,
    },
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
}

#[derive(Debug, Clone, Copy)]
struct Git<'root> {
    root: &'root Path,
}

impl Git<'_> {
    fn run(&self, args: &[&str]) -> Result<Vec<u8>, SurveyError> {
        let command = args.join(" ");
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(self.root)
            .args(args)
            .output()
            .map_err(|source| SurveyError::GitIo {
                command: command.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(SurveyError::Git {
                command,
                root: self.root.display().to_string(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output.stdout)
    }

    fn remote_refs(&self) -> Result<Vec<String>, SurveyError> {
        let bytes = self.run(&[
            "for-each-ref",
            "--format=%(refname)%09%(symref)%09%(objecttype)",
            "refs/remotes/",
        ])?;
        let mut refs = Vec::new();
        for line in String::from_utf8_lossy(&bytes).lines() {
            let mut fields = line.split('\t');
            let Some(reference) = fields.next() else {
                continue;
            };
            let symbolic = fields.next().unwrap_or_default();
            let object_type = fields.next().unwrap_or_default();
            if symbolic.is_empty() && object_type == "commit" {
                refs.push(reference.to_owned());
            }
        }
        Ok(refs)
    }

    fn markdown_paths(
        &self,
        reference: &str,
        bundle_root: &str,
    ) -> Result<Vec<String>, SurveyError> {
        let bytes = self.run(&[
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            "--full-tree",
            reference,
            "--",
            bundle_root,
        ])?;
        Ok(bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .filter(|path| crate::walk::is_markdown(path))
            .collect())
    }

    fn blob(&self, reference: &str, path: &str) -> Result<String, SurveyError> {
        let object = format!("{reference}:{path}");
        let bytes = self.run(&["show", &object])?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Survey local remote-tracking refs without checking out or changing one.
///
/// Only branches with findings are returned. The caller is responsible for
/// fetching first when it needs the remote-tracking refs refreshed.
///
/// # Errors
///
/// Fails when Git cannot enumerate a ref or read one of its Markdown blobs,
/// or when the adopting configuration cannot resolve its vocabulary.
pub fn survey_branches(
    repo_root: &Path,
    bundle_root: &Path,
    config: &Config,
) -> Result<Vec<BranchSurvey>, SurveyError> {
    let git = Git { root: repo_root };
    let current: BTreeSet<String> =
        crate::walk::markdown_files(bundle_root, &config.paths.skip_names)
            .into_iter()
            .filter_map(|path| {
                path.strip_prefix(bundle_root)
                    .ok()
                    .map(crate::walk::to_posix)
            })
            .collect();
    let bundle_prefix = repository_bundle_prefix(&config.bundle_root);
    let mut surveys = Vec::new();

    for reference in git.remote_refs()? {
        let mut findings = Vec::new();
        for repo_path in git.markdown_paths(&reference, &bundle_prefix)? {
            let Some(relative) = bundle_relative(&repo_path, &bundle_prefix) else {
                continue;
            };
            if current.contains(relative) || is_skipped(relative, &config.paths.skip_names) {
                continue;
            }
            let text = git.blob(&reference, &repo_path)?;
            if let Some(classification) = classify(relative, &text, config)? {
                findings.push(Finding {
                    path: relative.to_owned(),
                    classification,
                });
            }
        }
        if !findings.is_empty() {
            findings.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
            surveys.push(BranchSurvey {
                branch: reference
                    .strip_prefix("refs/remotes/")
                    .unwrap_or(&reference)
                    .to_owned(),
                findings,
            });
        }
    }
    Ok(surveys)
}

fn repository_bundle_prefix(bundle_root: &str) -> String {
    if bundle_root == "." {
        ".".to_owned()
    } else {
        bundle_root.trim_end_matches('/').to_owned()
    }
}

fn bundle_relative<'path>(repo_path: &'path str, bundle_prefix: &str) -> Option<&'path str> {
    if bundle_prefix == "." {
        return Some(repo_path);
    }
    repo_path.strip_prefix(bundle_prefix)?.strip_prefix('/')
}

fn is_skipped(relative: &str, skip_names: &[String]) -> bool {
    relative
        .split('/')
        .any(|component| crate::walk::is_skipped(component, skip_names))
}

fn classify(
    relative: &str,
    text: &str,
    config: &Config,
) -> Result<Option<Classification>, crate::config::ConfigError> {
    if config.is_generated(relative) || crate::migrate::generated_signal(text).is_some() {
        return Ok(Some(Classification::Generated));
    }
    if !crate::migrate::is_reserved(relative) && config.type_for(relative).is_none() {
        return Ok(Some(Classification::Untypable));
    }
    if crate::check::document_would_fail(relative, text, config)? {
        return Ok(Some(Classification::WouldFail));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Paths, TypeRule};

    fn config() -> Config {
        Config {
            paths: Paths {
                generated: vec!["generated/**".to_owned()],
                ..Paths::default()
            },
            type_rules: vec![TypeRule {
                path: "typed/**".to_owned(),
                concept_type: "Reference".to_owned(),
            }],
            ..Config::default()
        }
    }

    #[test]
    fn generated_wins_over_an_absent_type_rule() {
        assert!(matches!(
            classify("generated/inventory.md", "# Inventory\n", &config()),
            Ok(Some(Classification::Generated))
        ));
    }

    #[test]
    fn matching_and_conformance_partition_findings() {
        let config = config();
        assert!(matches!(
            classify("other/page.md", "# Page\n", &config),
            Ok(Some(Classification::Untypable))
        ));
        assert!(matches!(
            classify("typed/page.md", "# Page\n", &config),
            Ok(Some(Classification::WouldFail))
        ));
        assert!(matches!(
            classify(
                "typed/clean.md",
                "---\ntype: Reference\n---\n\n# Clean\n",
                &config,
            ),
            Ok(None)
        ));
    }

    #[test]
    fn both_shared_content_signals_are_generated() {
        let config = config();
        assert!(matches!(
            classify(
                "typed/marker.md",
                "<!-- DO NOT EDIT: generated -->\n# Marker\n",
                &config,
            ),
            Ok(Some(Classification::Generated))
        ));
        assert!(matches!(
            classify(
                "typed/frontmatter.md",
                "---\ngenerated: { by: fixture/1, at: 2026-08-29 }\n---\n# Frontmatter\n",
                &config,
            ),
            Ok(Some(Classification::Generated))
        ));
    }
}
