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
    #[error("{path}: could not be re-read for a pin edit: {source}")]
    Edit {
        path: String,
        #[source]
        source: toml_edit::TomlError,
    },
    #[error("{path}: has no bundle `{id}` to repin")]
    NoSuchBundle { path: String, id: String },
    #[error(
        "{path}: repinning `{id}` would have changed something other than \
         that bundle's rev, so nothing was written. Report this: the edit is \
         meant to touch one value and a manifest it cannot edit safely is a \
         manifest it must not rewrite"
    )]
    UnsafeEdit { path: String, id: String },
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
    #[error(
        "bundle `{id}`: fetch `{fetch}` is not a scheme this build knows; \
         the only supported value is \"git+ssh\""
    )]
    BadFetch { id: String, fetch: String },
    #[error(
        "bundle `{id}`: fetch = \"git+ssh\" derives its URL from an https:// \
         repo, and `{repo}` is not one"
    )]
    FetchNotDerivable { id: String, repo: String },
    #[error(
        "bundle `{with_fetch}` sets fetch and bundle `{without}` does not; \
         a nix-built site needs every bundle as an input, so the bundles opt \
         in together or not at all"
    )]
    FetchMixed { with_fetch: String, without: String },
    #[error("no bundles in {path}; there is nothing to assemble")]
    NoBundles { path: String },
}

/// One source repository, mounted at `content/<id>/`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
    /// How a nix evaluation fetches this bundle, or empty for a tenant whose
    /// site is built by CI rather than by nix.
    ///
    /// The only supported value is `"git+ssh"`, and the key names *only* the
    /// scheme: the URL is derived from `repo`, so the repository's identity
    /// stays stored once and cannot drift into a second spelling. Setting it
    /// is what makes `okf-assemble` generate `nix/bundles.nix`, and a tenant
    /// that sets it stops needing `credential` — the evaluating user's own
    /// ssh key is the credential, and no token reaches any machine.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fetch: String,
    /// A site-absolute base for a mount holding a vendored upstream, so
    /// `](/x)` in a mirrored page resolves to the upstream's live page rather
    /// than to this site's root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site_absolute_base: String,
}

fn dot() -> String {
    ".".to_owned()
}

impl Bundle {
    /// The URL a nix evaluation fetches this bundle from, when it opts in.
    ///
    /// `https://host/owner/name.git` becomes `ssh://git@host/owner/name.git`
    /// and nothing else changes: the scheme is the whole difference, which is
    /// why `fetch` names a scheme rather than storing a second URL. Measured
    /// on the build machine: `git+ssh` reaches every private bundle through
    /// the evaluating user's key, and `git+https` fails with no credential
    /// helper.
    #[must_use]
    pub fn nix_fetch_url(&self) -> Option<String> {
        if self.fetch != "git+ssh" {
            return None;
        }
        self.repo
            .strip_prefix("https://")
            .map(|rest| format!("ssh://git@{rest}"))
    }
}

/// The tenant's own presentation, which `okf-assemble` derives `hugo.toml`
/// from so the generator configuration cannot fork per tenant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct SiteSettings {
    pub title: String,
    pub base_url: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
    /// **This drops every comment and every choice of layout in the file, so
    /// it is only ever correct for a manifest this tool is creating.** Only
    /// `--bootstrap` calls it, and only after `is_empty_manifest` has said
    /// there is no reviewed manifest there to lose. A pin bump goes through
    /// [`set_bundle_rev`] instead: `--update` is the one command the standard
    /// tells every tenant to run, and routing it through here deleted 65
    /// lines of a live manifest — including the paragraph saying which
    /// private bundle must never be mounted.
    ///
    /// `--local` never writes at all: a local override is an argument for one
    /// invocation, and when a working tree and the pinned `rev` disagree the
    /// manifest wins.
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
            if !bundle.fetch.is_empty() {
                if bundle.fetch != "git+ssh" {
                    return Err(ManifestError::BadFetch {
                        id: bundle.id.clone(),
                        fetch: bundle.fetch.clone(),
                    });
                }
                if bundle.nix_fetch_url().is_none() {
                    return Err(ManifestError::FetchNotDerivable {
                        id: bundle.id.clone(),
                        repo: bundle.repo.clone(),
                    });
                }
            }
        }
        // All bundles opt into a nix fetch together or not at all: a partial
        // set cannot build a site, and a check that only watched half the
        // pins would be the drift it exists to catch.
        let with_fetch = self.bundles.iter().find(|b| !b.fetch.is_empty());
        let without = self.bundles.iter().find(|b| b.fetch.is_empty());
        if let (Some(with_fetch), Some(without)) = (with_fetch, without) {
            return Err(ManifestError::FetchMixed {
                with_fetch: with_fetch.id.clone(),
                without: without.id.clone(),
            });
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

/// Repin one bundle: rewrite its `rev` in `site.toml` and change nothing else.
///
/// A manifest is a document as much as it is data. The `ria` manifest carries
/// a paragraph saying that `ria/ria-documentation` — a private client
/// engagement record — is deliberately absent and must never be mounted, and
/// that paragraph is the only place that reasoning is written down. Round
/// tripping the file through a serialiser deletes it: the serialiser emits
/// values, and a comment is not a value. So the pin bump edits the document
/// rather than regenerating it.
///
/// `toml_edit` is what makes that a value substitution instead of a text
/// substitution. The alternative — find the `[[bundle]]` block, find the line
/// starting `rev`, splice — is a second, worse TOML parser: it has to be told
/// about literal strings, inline tables, a `rev` inside a comment, and a
/// bundle written `bundle = [{ ... }]`, and every one it gets wrong either
/// corrupts the file or silently repins the wrong bundle. Here the DOM
/// answers all of those, and the value's decor is carried across so a comment
/// trailing the `rev` on its own line survives too.
///
/// The rewrite is then checked before it lands: the rendered text is parsed
/// back as a [`Manifest`] and compared against the manifest that was read,
/// with only this bundle's rev changed. Nothing is written unless they agree.
/// A file that gates a client's confidentiality is one where a wrong edit
/// costs more than a refused one.
///
/// # Errors
///
/// Fails when the file cannot be read, re-parsed, or written; when no bundle
/// carries `id`; and when the rewrite would have changed anything beyond that
/// bundle's `rev`, in which case the file is left untouched.
pub fn set_bundle_rev(path: &Path, id: &str, rev: &str) -> Result<(), ManifestError> {
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: shown.clone(),
        source,
    })?;
    let before: Manifest = toml::from_str(&text).map_err(|source| ManifestError::Parse {
        path: shown.clone(),
        source,
    })?;
    let mut document: toml_edit::DocumentMut =
        text.parse().map_err(|source| ManifestError::Edit {
            path: shown.clone(),
            source,
        })?;

    let edited = document
        .get_mut("bundle")
        .is_some_and(|bundles| repin(bundles, id, rev));
    if !edited {
        return Err(ManifestError::NoSuchBundle {
            path: shown,
            id: id.to_owned(),
        });
    }

    let rendered = document.to_string();
    let mut expected = before;
    for bundle in &mut expected.bundles {
        if bundle.id == id {
            rev.clone_into(&mut bundle.rev);
        }
    }
    let after: Manifest = toml::from_str(&rendered).map_err(|source| ManifestError::Parse {
        path: shown.clone(),
        source,
    })?;
    if after != expected {
        return Err(ManifestError::UnsafeEdit {
            path: shown,
            id: id.to_owned(),
        });
    }

    std::fs::write(path, rendered).map_err(|source| ManifestError::Write {
        path: shown,
        source,
    })
}

/// Set `rev` on the bundle carrying `id`, reporting whether one was found.
///
/// Both spellings of a bundle list are handled: `[[bundle]]` blocks, which is
/// what every manifest in this estate uses and what `--bootstrap` writes, and
/// `bundle = [{ ... }]`, which parses to the same manifest and would
/// otherwise be repinned by nobody while the command still reported success.
fn repin(bundles: &mut toml_edit::Item, id: &str, rev: &str) -> bool {
    if let Some(tables) = bundles.as_array_of_tables_mut() {
        for table in tables.iter_mut() {
            if table.get("id").and_then(toml_edit::Item::as_str) == Some(id) {
                return table
                    .get_mut("rev")
                    .and_then(toml_edit::Item::as_value_mut)
                    .is_some_and(|value| {
                        replace_in_place(value, rev);
                        true
                    });
            }
        }
    }
    if let Some(array) = bundles.as_array_mut() {
        for entry in array.iter_mut() {
            let Some(table) = entry.as_inline_table_mut() else {
                continue;
            };
            if table.get("id").and_then(toml_edit::Value::as_str) == Some(id) {
                return table.get_mut("rev").is_some_and(|value| {
                    replace_in_place(value, rev);
                    true
                });
            }
        }
    }
    false
}

/// Swap a value, keeping the whitespace and comments that surround it.
///
/// The decor is the reason this is not an assignment: a value's suffix holds
/// anything trailing it on the line, so `rev = "…"  # bumped 2026-08-14` keeps
/// its note through a repin.
fn replace_in_place(value: &mut toml_edit::Value, rev: &str) {
    let decor = value.decor().clone();
    *value = toml_edit::Value::from(rev);
    *value.decor_mut() = decor;
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

    /// The scheme is the whole difference between the stored URL and the
    /// fetched one, which is what lets `fetch` be a scheme name rather than a
    /// second copy of the repository's identity.
    #[test]
    fn a_nix_fetch_url_is_the_https_repo_with_only_the_scheme_changed() {
        let mut bundle = Bundle {
            id: "alpha".to_owned(),
            repo: "https://github.com/owner/alpha.git".to_owned(),
            git_ref: "refs/heads/main".to_owned(),
            rev: "1".repeat(40),
            subdir: ".".to_owned(),
            credential: String::new(),
            fetch: "git+ssh".to_owned(),
            site_absolute_base: String::new(),
        };
        assert_eq!(
            bundle.nix_fetch_url().as_deref(),
            Some("ssh://git@github.com/owner/alpha.git")
        );
        bundle.fetch = String::new();
        assert_eq!(bundle.nix_fetch_url(), None);
        bundle.fetch = "git+ssh".to_owned();
        bundle.repo = "file:///somewhere/alpha".to_owned();
        assert_eq!(bundle.nix_fetch_url(), None);
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
