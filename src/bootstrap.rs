//! `okf-assemble --bootstrap`: the one moment a local directory layout is
//! allowed to touch a manifest.
//!
//! It runs the scope predicates over a directory of checkouts, reads each
//! `origin`, normalises an SSH remote to the HTTPS one naming the same
//! repository, resolves `HEAD` to a commit, and writes a *draft*. Michael
//! edits the draft; nothing here decides membership.
//!
//! Two refusals matter more than the discovery does. It will not run against a
//! manifest that already has bundles, because a discovery pass that can
//! overwrite a reviewed manifest is a discovery pass that eventually does. And
//! a repository with no remote goes to `deferred.toml` rather than into the
//! manifest with a placeholder, because CI cannot fetch what has no remote and
//! a manifest entry that always fails is worse than an absent one.

use std::path::{Path, PathBuf};

use crate::assemble::AssembleError;
use crate::manifest::{Bundle, Manifest, is_mount_name, normalise_remote};

/// A checkout the predicates matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub path: PathBuf,
    /// `"."` for a whole-repository bundle, `"docs"` for a code repository.
    pub subdir: String,
    /// Empty when the checkout has no `origin`.
    pub repo: String,
    pub rev: String,
    pub git_ref: String,
}

#[derive(Debug, Default)]
pub struct Draft {
    pub bundles: Vec<Bundle>,
    /// Checkouts with no remote, which CI cannot fetch.
    pub deferred: Vec<Candidate>,
}

/// Walk `scan_root` two levels deep and classify what is there.
///
/// Two levels, because this estate keeps repositories one level inside a
/// rollup directory and the rollup directories themselves are not repositories.
#[must_use]
pub fn discover(scan_root: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    for first in sorted_dirs(scan_root) {
        if let Some(candidate) = classify(&first) {
            found.push(candidate);
            continue;
        }
        for second in sorted_dirs(&first) {
            if let Some(candidate) = classify(&second) {
                found.push(candidate);
            }
        }
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.dedup_by(|a, b| a.id == b.id);
    found
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.starts_with('.'))
        })
        .collect();
    kept.sort();
    kept
}

/// The scope predicates, applied to one directory.
///
/// A whole-repository bundle carries `meetings/` and `org/` at its root. A code
/// repository is one whose `.git` is a *directory* — a worktree's `.git` is a
/// file, and excluding worktrees is what stops the same repository being
/// counted three times under three names.
fn classify(dir: &Path) -> Option<Candidate> {
    let git = dir.join(".git");
    if !git.exists() || !dir.join("flake.nix").is_file() {
        return None;
    }
    let subdir = if dir.join("meetings").is_dir() && dir.join("org").is_dir() {
        ".".to_owned()
    } else if git.is_dir() && has_markdown_under(&dir.join("docs")) {
        "docs".to_owned()
    } else {
        return None;
    };
    let id = mount_name(dir)?;
    let remote = crate::assemble::capture(
        "git",
        &[
            "-C",
            &dir.display().to_string(),
            "remote",
            "get-url",
            "origin",
        ],
    )
    .map(|url| normalise_remote(&url))
    .unwrap_or_default();
    let rev = crate::assemble::capture(
        "git",
        &["-C", &dir.display().to_string(), "rev-parse", "HEAD"],
    )
    .unwrap_or_default();
    let git_ref = crate::assemble::capture(
        "git",
        &[
            "-C",
            &dir.display().to_string(),
            "symbolic-ref",
            "--quiet",
            "HEAD",
        ],
    )
    .unwrap_or_else(|| "refs/heads/main".to_owned());
    Some(Candidate {
        id,
        path: dir.to_path_buf(),
        subdir,
        repo: remote,
        rev,
        git_ref,
    })
}

fn has_markdown_under(dir: &Path) -> bool {
    dir.is_dir() && crate::walk::has_markdown(dir)
}

/// A directory name turned into a mount name, or `None` if it cannot be.
#[must_use]
pub fn mount_name(dir: &Path) -> Option<String> {
    let raw = dir.file_name().and_then(|n| n.to_str())?;
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    is_mount_name(&out).then_some(out)
}

/// Split what was discovered into a draft manifest and a deferred list.
#[must_use]
pub fn draft(tenant: &str, candidates: Vec<Candidate>) -> Draft {
    let mut out = Draft::default();
    for candidate in candidates {
        if candidate.repo.is_empty() || candidate.rev.len() != 40 {
            out.deferred.push(candidate);
            continue;
        }
        out.bundles.push(Bundle {
            id: candidate.id.clone(),
            repo: candidate.repo.clone(),
            git_ref: candidate.git_ref.clone(),
            rev: candidate.rev.clone(),
            subdir: candidate.subdir.clone(),
            credential: format!("forge-{tenant}"),
            site_absolute_base: String::new(),
        });
    }
    out
}

/// Write `deferred.toml`, which records what was found and cannot be fetched.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn write_deferred(path: &Path, deferred: &[Candidate]) -> Result<(), AssembleError> {
    let mut out = String::from(
        "# Written by `okf-assemble --bootstrap`. Every entry here matched the\n\
         # scope predicates and has no remote CI could fetch, so it is not in\n\
         # site.toml. Give one a remote and move it across by hand.\n\n",
    );
    for candidate in deferred {
        let entry = format!(
            "[[deferred]]\nid = {}\nsubdir = {}\nreason = {}\n\n",
            crate::sitegen::toml_string(&candidate.id),
            crate::sitegen::toml_string(&candidate.subdir),
            crate::sitegen::toml_string(if candidate.repo.is_empty() {
                "no git remote"
            } else {
                "HEAD did not resolve to a commit"
            }),
        );
        out.push_str(&entry);
    }
    std::fs::write(path, out).map_err(|source| AssembleError::Io {
        context: format!("writing {}", path.display()),
        source,
    })
}

/// Is this manifest safe to overwrite with a draft?
#[must_use]
pub fn is_empty_manifest(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    toml::from_str::<Manifest>(&text).is_ok_and(|manifest| manifest.bundles.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_name_becomes_a_mount_name_or_nothing() {
        assert_eq!(
            mount_name(Path::new("/x/lane-cast")).as_deref(),
            Some("lane-cast")
        );
        assert_eq!(
            mount_name(Path::new("/x/Data_Warehouse")).as_deref(),
            Some("data-warehouse")
        );
        assert_eq!(mount_name(Path::new("/x/---")), None);
    }

    /// A repository with no remote is recorded rather than guessed at: CI
    /// cannot fetch what has no remote, and a manifest entry that always fails
    /// is worse than an absent one.
    #[test]
    fn a_remoteless_checkout_is_deferred_rather_than_drafted() {
        let candidates = vec![
            Candidate {
                id: "with-remote".to_owned(),
                path: PathBuf::from("/x/a"),
                subdir: "docs".to_owned(),
                repo: "https://example.invalid/a.git".to_owned(),
                rev: "b".repeat(40),
                git_ref: "refs/heads/main".to_owned(),
            },
            Candidate {
                id: "no-remote".to_owned(),
                path: PathBuf::from("/x/b"),
                subdir: ".".to_owned(),
                repo: String::new(),
                rev: "c".repeat(40),
                git_ref: "refs/heads/main".to_owned(),
            },
        ];
        let draft = draft("techzen", candidates);
        assert_eq!(draft.bundles.len(), 1);
        assert_eq!(draft.bundles[0].id, "with-remote");
        assert_eq!(draft.deferred.len(), 1);
        assert_eq!(draft.deferred[0].id, "no-remote");
    }

    /// The refusal that matters: a reviewed manifest is never overwritten.
    #[test]
    fn a_manifest_with_bundles_refuses_a_bootstrap() {
        let dir = std::env::temp_dir().join(format!("okf-boot-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("site.toml");
        assert!(is_empty_manifest(&path));

        let _ = std::fs::write(
            &path,
            "schema_version = 1\ntenant = \"t\"\n[[bundle]]\nid = \"b\"\n\
             repo = \"https://example.invalid/b.git\"\nref = \"refs/heads/main\"\n\
             rev = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
        );
        assert!(!is_empty_manifest(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
