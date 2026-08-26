//! `site.toml`, the tenant manifest.
//!
//! One tracked file in a tenant's own site repository is the durable
//! definition of that tenant. It names every bundle by repository identity and
//! pinned commit, stores no path from anybody's machine, names no other
//! tenant, and has no mode that discovers a repository nobody listed.
//!
//! Two properties here are enforced rather than documented. **A ref that moves
//! is not reproducible**, so the fetch asks for `rev` and nothing else; `ref`
//! exists only so `--update <id>` knows where to look for a newer commit,
//! which makes a roll-forward a reviewed diff rather than silent drift. And
//! **every URL is stored as HTTPS**, because an SSH URL behaves differently on
//! a runner than on a laptop once a machine-local `insteadOf` rewrite is in
//! play, and that has blocked unattended work before.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The only `schema_version` this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Asset extensions copied out of a bundle alongside its markdown.
///
/// An allowlist rather than a denylist, and deliberately images only. A signed
/// agreement lives beside its markdown as `.pdf` or `.typ` in at least one
/// bundle in this estate and must never reach a published surface, so a
/// document format is not on this list and adding one is a reviewed change.
pub const DEFAULT_ASSET_EXTENSIONS: &[&str] =
    &["png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "ico"];

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
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
    #[error("{path}: could not be written back: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: could not be serialised: {source}")]
    Serialise {
        path: String,
        #[source]
        source: toml::ser::Error,
    },
    #[error(
        "{path}: schema_version {found} is newer than this build understands \
         ({SUPPORTED_SCHEMA_VERSION}); upgrade okf-tools"
    )]
    Version { path: String, found: u32 },
    #[error(
        "bundle id `{id}` is not a mount name: use lowercase letters, digits \
         and hyphens, starting with a letter or digit"
    )]
    BadId { id: String },
    #[error("bundle id `{id}` appears twice; an id is the mount path and has to be unique")]
    DuplicateId { id: String },
    #[error(
        "bundle `{id}`: rev `{rev}` is not a 40-character commit; \
         `okf-assemble --update {id}` resolves the ref and writes one"
    )]
    BadRev { id: String, rev: String },
    #[error(
        "bundle `{id}`: repo `{repo}` is not an https:// or file:// URL; \
         the manifest normalises SSH remotes on write, and a machine-local \
         `insteadOf` rewrite has no business in a tracked file"
    )]
    BadRepo { id: String, repo: String },
    #[error(
        "bundle `{id}`: subdir `{subdir}` escapes the bundle; use \".\" or a \
         path inside it"
    )]
    BadSubdir { id: String, subdir: String },
    #[error(
        "bundle `{id}` names credential `{credential}`, which is not in \
         credentials.allow; a manifest reaching for another tenant's secret \
         fails here rather than at fetch time"
    )]
    CredentialNotAllowed { id: String, credential: String },
    #[error("no bundles in {path}; there is nothing to assemble")]
    NoBundles { path: String },
}

/// One source repository, mounted at `content/<id>/`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// The stable identity, and the mount path.
    pub id: String,
    /// An HTTPS clone URL. Normalised on write; never an SSH one.
    pub repo: String,
    /// What this bundle tracks, for roll-forward. Never what a build fetches.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// The 40-character commit this build fetches. Written by `--update`.
    pub rev: String,
    /// The bundle root inside the repository: `"."`, or `"docs"` for a code
    /// repository whose bundle is not the whole tree.
    #[serde(default = "dot")]
    pub subdir: String,
    /// The *name* of a secret, and never a secret.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credential: String,
    /// A site-absolute base for a mount holding a vendored upstream, so
    /// `](/x)` in a mirrored page resolves to the upstream's live page rather
    /// than to this site's root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site_absolute_base: String,
}

fn dot() -> String {
    ".".to_owned()
}

/// The tenant's own presentation, which `okf-assemble` derives `hugo.toml`
/// from so the generator configuration cannot fork per tenant.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct SiteSettings {
    pub title: String,
    pub base_url: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Manifest {
    pub schema_version: u32,
    pub tenant: String,
    pub site: SiteSettings,
    /// Extensions copied beside the markdown. Empty means the default list.
    pub asset_extensions: Vec<String>,
    #[serde(rename = "bundle")]
    pub bundles: Vec<Bundle>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: SUPPORTED_SCHEMA_VERSION,
            tenant: String::new(),
            site: SiteSettings::default(),
            asset_extensions: Vec::new(),
            bundles: Vec::new(),
        }
    }
}

impl Manifest {
    /// Read and validate `site.toml`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed, declares a newer
    /// `schema_version` than this build understands, or holds a bundle whose
    /// id, rev, repo or subdir would not survive being used as written.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: shown.clone(),
            source,
        })?;
        let manifest: Self = toml::from_str(&text).map_err(|source| ManifestError::Parse {
            path: shown.clone(),
            source,
        })?;
        if manifest.schema_version > SUPPORTED_SCHEMA_VERSION {
            return Err(ManifestError::Version {
                path: shown,
                found: manifest.schema_version,
            });
        }
        manifest.validate(&shown)?;
        Ok(manifest)
    }

    /// Write the manifest back, preserving nothing but its values.
    ///
    /// Only `--bootstrap` and `--update` call this, and `--local` never does:
    /// a local override is an argument for one invocation, and when a working
    /// tree and the pinned `rev` disagree the manifest wins.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be serialised or written.
    pub fn save(&self, path: &Path) -> Result<(), ManifestError> {
        let shown = path.display().to_string();
        let text = toml::to_string_pretty(self).map_err(|source| ManifestError::Serialise {
            path: shown.clone(),
            source,
        })?;
        std::fs::write(path, text).map_err(|source| ManifestError::Write {
            path: shown,
            source,
        })
    }

    fn validate(&self, shown: &str) -> Result<(), ManifestError> {
        if self.bundles.is_empty() {
            return Err(ManifestError::NoBundles {
                path: shown.to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for bundle in &self.bundles {
            if !is_mount_name(&bundle.id) {
                return Err(ManifestError::BadId {
                    id: bundle.id.clone(),
                });
            }
            if !seen.insert(bundle.id.clone()) {
                return Err(ManifestError::DuplicateId {
                    id: bundle.id.clone(),
                });
            }
            if !is_commit(&bundle.rev) {
                return Err(ManifestError::BadRev {
                    id: bundle.id.clone(),
                    rev: bundle.rev.clone(),
                });
            }
            if !(bundle.repo.starts_with("https://") || bundle.repo.starts_with("file://")) {
                return Err(ManifestError::BadRepo {
                    id: bundle.id.clone(),
                    repo: bundle.repo.clone(),
                });
            }
            if !is_inside(&bundle.subdir) {
                return Err(ManifestError::BadSubdir {
                    id: bundle.id.clone(),
                    subdir: bundle.subdir.clone(),
                });
            }
        }
        Ok(())
    }

    /// Assert every credential this manifest names is allowed here.
    ///
    /// The credential is what enforces tenancy, not the manifest: a
    /// fine-grained token's repository list is exactly one tenant's bundles,
    /// so a manifest reaching for another tenant's secret produces a broken
    /// build rather than a cross-client read. This check moves that failure
    /// from the fetch to the first line of the job, where it names the bundle.
    ///
    /// An absent `credentials.allow` means the tenant has not opted in, and
    /// nothing is checked; an empty one means no credential is allowed.
    ///
    /// # Errors
    ///
    /// Fails when a bundle names a credential the file does not list.
    pub fn check_credentials(&self, allow: Option<&BTreeSet<String>>) -> Result<(), ManifestError> {
        let Some(allow) = allow else {
            return Ok(());
        };
        for bundle in &self.bundles {
            if !bundle.credential.is_empty() && !allow.contains(&bundle.credential) {
                return Err(ManifestError::CredentialNotAllowed {
                    id: bundle.id.clone(),
                    credential: bundle.credential.clone(),
                });
            }
        }
        Ok(())
    }

    /// The asset extensions this tenant copies, lowercased.
    #[must_use]
    pub fn asset_extensions(&self) -> Vec<String> {
        if self.asset_extensions.is_empty() {
            return DEFAULT_ASSET_EXTENSIONS
                .iter()
                .map(|e| (*e).to_owned())
                .collect();
        }
        self.asset_extensions
            .iter()
            .map(|e| e.trim_start_matches('.').to_lowercase())
            .collect()
    }
}

/// Read `credentials.allow`: one credential name per line, `#` comments.
///
/// # Errors
///
/// Fails when the file exists but cannot be read. An absent file is `None`,
/// which means the tenant has not declared an allowlist.
pub fn read_credentials_allow(path: &Path) -> Result<Option<BTreeSet<String>>, ManifestError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.display().to_string(),
        source,
    })?;
    Ok(Some(
        text.lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
    ))
}

/// Is this a safe mount name?
///
/// The id becomes a path segment under `content/`, so it is bounded here
/// rather than trusted: a manifest is reviewed, but a reviewer should not have
/// to be the thing that stops `../` from escaping the content tree.
#[must_use]
pub fn is_mount_name(id: &str) -> bool {
    !id.is_empty()
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Is this a 40-character commit rather than a branch name or a short sha?
#[must_use]
pub fn is_commit(rev: &str) -> bool {
    rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit())
}

/// Does this relative path stay inside the tree it is relative to?
#[must_use]
pub fn is_inside(path: &str) -> bool {
    if path == "." {
        return true;
    }
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|part| part != ".." && !part.is_empty())
}

/// Rewrite an SSH remote as the HTTPS one naming the same repository.
///
/// `git@host:owner/repo.git` and `ssh://git@host/owner/repo.git` both become
/// `https://host/owner/repo.git`. Anything already HTTPS is returned as it
/// stands, and anything else is returned unchanged so the caller's validation
/// reports it rather than this function guessing.
#[must_use]
pub fn normalise_remote(remote: &str) -> String {
    let remote = remote.trim();
    if let Some(rest) = remote.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map_or(rest, |(_, tail)| tail);
        return format!("https://{rest}");
    }
    if !remote.contains("://")
        && let Some((host_part, path)) = remote.split_once(':')
    {
        let host = host_part.split_once('@').map_or(host_part, |(_, h)| h);
        return format!("https://{host}/{}", path.trim_start_matches('/'));
    }
    remote.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_remotes_normalise_to_the_https_url_for_the_same_repository() {
        assert_eq!(
            normalise_remote("git@github.com:owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            normalise_remote("ssh://git@gitea.example.test/owner/repo.git"),
            "https://gitea.example.test/owner/repo.git"
        );
        assert_eq!(
            normalise_remote("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
    }

    /// The id is a path segment under `content/`, so the traversal shapes are
    /// refused here rather than caught by whoever reviews the manifest.
    #[test]
    fn a_mount_name_cannot_escape_the_content_tree() {
        assert!(is_mount_name("lane-cast"));
        assert!(is_mount_name("knowledge"));
        assert!(!is_mount_name(".."));
        assert!(!is_mount_name("a/b"));
        assert!(!is_mount_name("-leading"));
        assert!(!is_mount_name("Upper"));
        assert!(!is_mount_name(""));
    }

    #[test]
    fn a_branch_name_or_a_short_sha_is_not_a_rev() {
        assert!(is_commit(&"a".repeat(40)));
        assert!(!is_commit("main"));
        assert!(!is_commit("abc1234"));
        assert!(!is_commit(&"z".repeat(40)));
    }

    #[test]
    fn a_subdir_stays_inside_the_bundle() {
        assert!(is_inside("."));
        assert!(is_inside("docs"));
        assert!(is_inside("docs/site"));
        assert!(!is_inside("../docs"));
        assert!(!is_inside("/docs"));
        assert!(!is_inside("docs/../.."));
    }
}
