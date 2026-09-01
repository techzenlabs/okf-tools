//! Files a document references from outside its bundle's subdir, mounted so
//! the link works for a reader who cannot open the repository.
//!
//! A code repository mounts `docs/`, and its documents point sideways: a
//! runbook names `../../scripts/support-bundle.sh`, an architecture page names
//! `../../schemas/inventory.schema.json`. On the site those links dead-end,
//! and the readers they dead-end for are exactly the ones the site exists for
//! — people with no licence on the source forge. Measured on the tenant that
//! raised this, 144 of its 174 referenced files live outside the mounted
//! subdir.
//!
//! So assembly resolves each escaping link against the *fetched tree* — the
//! whole repository is already on disk; `subdir` only narrowed what the copy
//! took — and copies the target under `static/_files/<id>/`, preserving its
//! repository-relative path. Hugo byte-copies `static/`, so the file serves at
//! `/_files/<id>/<path>` with no content-pipeline semantics: no listing entry,
//! no URL sanitising, no media-type guesswork. The leading underscore is what
//! makes the prefix collision-proof — [`crate::manifest::is_mount_name`]
//! refuses it as a bundle id, so no mount can ever claim the URL.
//!
//! Three rules bound what mounts, each inherited rather than invented here:
//!
//! * **The tenant's `asset_extensions` allowlist decides, same as inside the
//!   subdir.** One knob, one policy: a tenant that keeps its list to text
//!   types keeps every published asset scannable, and a tenant that adds a
//!   binary type has made that call once, deliberately, for both surfaces.
//! * **Only a file that exists mounts, and only a link to a mounted file is
//!   rewritten** ([`crate::sitelinks`] checks the copied tree, not its own
//!   arithmetic). A directory link, a dead link, a markdown link and a link
//!   escaping the repository itself all stay exactly as written — §11 makes a
//!   broken link a link, never a build failure.
//! * **Every byte is charged against `max_asset_bytes` before it is copied**,
//!   because these files ship in the site image and the day that stops being
//!   fine should arrive as a red build, not as a slow deploy nobody ties to
//!   its cause.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::assemble::{AssembleError, Budget};

/// Mount every allowlisted file the markdown under `mount` references from
/// outside `subdir`, into `dest`. Returns how many files were copied.
///
/// `repo_root` is the fetched tree the subdir was taken from. A bundle whose
/// subdir is the repository root has nothing outside it, and returns
/// immediately.
///
/// # Errors
///
/// Fails when a document or a referenced file cannot be read or copied, or
/// when a copy would take the asset payload past `max_asset_bytes`.
pub(crate) fn mount(
    mount: &Path,
    repo_root: &Path,
    subdir: &str,
    dest: &Path,
    extensions: &[String],
    budget: &mut Budget,
) -> Result<usize, AssembleError> {
    if subdir == "." || subdir.is_empty() {
        return Ok(0);
    }
    let subdir_segments: Vec<String> = subdir
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_owned)
        .collect();

    let mut mounted: BTreeSet<String> = BTreeSet::new();
    let mut copied = 0usize;
    for file in crate::walk::markdown_files(mount, &[]) {
        let text = std::fs::read_to_string(&file).map_err(|source| AssembleError::Io {
            context: format!("reading {}", file.display()),
            source,
        })?;
        let doc_dir: Vec<String> = file
            .parent()
            .and_then(|dir| dir.strip_prefix(mount).ok())
            .map(|relative| {
                relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        for target in crate::sitelinks::link_paths(&text) {
            let Some(repo_rel) = escaped_target(&subdir_segments, &doc_dir, target) else {
                continue;
            };
            let Some(name) = repo_rel.last() else {
                continue;
            };
            if crate::walk::is_markdown(name) || !crate::assemble::has_extension(name, extensions) {
                continue;
            }
            let relative: PathBuf = repo_rel.iter().collect();
            let source = repo_root.join(&relative);
            if !source.is_file() {
                continue;
            }
            // A symlink inside the fetched tree pointing out of it must not
            // become a published copy of whatever it points at.
            let (Ok(real_root), Ok(real_source)) =
                (repo_root.canonicalize(), source.canonicalize())
            else {
                continue;
            };
            if !real_source.starts_with(&real_root) {
                continue;
            }
            if !mounted.insert(repo_rel.join("/")) {
                continue;
            }
            let bytes = std::fs::metadata(&real_source)
                .map_err(|source| AssembleError::Io {
                    context: format!("reading the size of {}", relative.display()),
                    source,
                })?
                .len();
            // Charged before the copy: the cap is spent during the work it
            // bounds, so an oversized payload fails without first landing.
            budget.charge(bytes)?;
            let to = dest.join(&relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|source| AssembleError::Io {
                    context: format!("creating {}", parent.display()),
                    source,
                })?;
            }
            crate::assemble::copy_writable(&real_source, &to)?;
            copied = copied.saturating_add(1);
        }
    }
    Ok(copied)
}

/// Resolve a link target written in `doc_dir` (inside `subdir`) to a
/// repository-relative path, but only when it leaves the subdir and stays in
/// the repository.
///
/// `None` is every other answer: an external or absolute target, one still
/// inside the subdir (the ordinary asset path already covers it), one that
/// climbs out of the repository itself, or one that declares itself a
/// directory with a trailing slash.
pub(crate) fn escaped_target(
    subdir: &[String],
    doc_dir: &[String],
    target: &str,
) -> Option<Vec<String>> {
    if target.is_empty()
        || target.starts_with('/')
        || target.starts_with("//")
        || target.contains(':')
        || target.ends_with('/')
    {
        return None;
    }
    let mut segments: Vec<String> = subdir.iter().chain(doc_dir.iter()).cloned().collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // Climbing above the repository root is nobody's file.
                segments.pop()?;
            }
            other => segments.push(other.to_owned()),
        }
    }
    if segments.len() >= subdir.len() && segments[..subdir.len()] == *subdir {
        return None;
    }
    Some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subdir() -> Vec<String> {
        vec!["docs".to_owned()]
    }

    #[test]
    fn a_target_leaving_the_subdir_resolves_repository_relative() {
        assert_eq!(
            escaped_target(&subdir(), &[], "../scripts/deploy.sh"),
            Some(vec!["scripts".to_owned(), "deploy.sh".to_owned()])
        );
        assert_eq!(
            escaped_target(&subdir(), &["guides".to_owned()], "../../schemas/a.json"),
            Some(vec!["schemas".to_owned(), "a.json".to_owned()])
        );
    }

    #[test]
    fn a_target_inside_the_subdir_is_not_this_modules_business() {
        assert_eq!(escaped_target(&subdir(), &[], "diagram.png"), None);
        assert_eq!(
            escaped_target(&subdir(), &["guides".to_owned()], "../notes/a.json"),
            None
        );
    }

    #[test]
    fn a_target_that_climbs_out_of_the_repository_is_left_alone() {
        assert_eq!(escaped_target(&subdir(), &[], "../../outside.sh"), None);
    }

    #[test]
    fn external_absolute_and_directory_targets_are_left_alone() {
        assert_eq!(
            escaped_target(&subdir(), &[], "https://a.invalid/x.sh"),
            None
        );
        assert_eq!(escaped_target(&subdir(), &[], "/x.sh"), None);
        assert_eq!(escaped_target(&subdir(), &[], "../scripts/"), None);
    }
}
