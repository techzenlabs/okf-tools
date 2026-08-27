//! `okf-assemble`: a tenant manifest in, a Hugo content tree out.
//!
//! The assembly step exists because Hugo cannot read an OKF bundle as it
//! stands. OKF §8 reserves `index.md` for a directory listing, and Hugo reads
//! a directory containing one as a *leaf bundle*, which makes every sibling a
//! page resource rather than a page. Measured on a four-page fixture, Hugo saw
//! 2 pages instead of 5, and the rename to `_index.md` recovered all 5. On a
//! real bundle that is hundreds of pages against tens, it fails silently, and
//! it is why [`rename_indexes`] has a fixture of its own.
//!
//! Having an assembly step then pays for itself three more times: it is where
//! links are rewritten so bundle-relative paths resolve under a mount, where
//! the confidentiality scan has a tree to run over, and where the shared
//! layouts and the generated `hugo.toml` land so no tenant holds a copy.
//!
//! Nothing here has a network credential. The CI job fetches, drops its
//! tokens, and only then runs this, which is why a template bug has nothing to
//! exfiltrate. The one exception is a direct run on a laptop, where the fetch
//! below uses whatever the developer's git is already configured with.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::collision::Collision;
use crate::manifest::{Bundle, Manifest};

#[derive(Debug, thiserror::Error)]
pub enum AssembleError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} is not on PATH, and {purpose}")]
    MissingProgram {
        program: &'static str,
        purpose: &'static str,
    },
    #[error("{command} failed ({status}): {stderr}")]
    Command {
        command: String,
        status: String,
        stderr: String,
    },
    #[error(
        "bundle `{id}`: {path} is not a directory; \
         `--local {id}=<path>` has to name a bundle root that exists"
    )]
    LocalMissing { id: String, path: String },
    #[error(
        "bundle `{id}`: {path} is not a directory; \
         `--pinned {id}=<path>@<rev>` has to name a bundle root that exists"
    )]
    PinnedMissing { id: String, path: String },
    #[error(
        "bundle `{id}`: the source handed to --pinned was fetched at {fetched}, \
         but site.toml pins {pinned}. The fetch specification has drifted from \
         the manifest; run `okf-assemble --bundles` and commit the diff"
    )]
    PinnedRevMismatch {
        id: String,
        fetched: String,
        pinned: String,
    },
    #[error(
        "bundle `{id}` is named by both --pinned and --local; a source is \
         either the verified pin or a working-tree override, never both"
    )]
    PinnedAndLocal { id: String },
    #[error("bundle `{id}`: subdir `{subdir}` is not in the fetched tree at {rev}")]
    SubdirMissing {
        id: String,
        subdir: String,
        rev: String,
    },
    #[error(
        "mermaid.min.js was not found. Set OKF_MERMAID_JS or pass --mermaid \
         <path>; the nix package wires it to nixpkgs#mermaid-cli, and it is \
         copied rather than executed"
    )]
    NoMermaid,
    #[error("--local names bundle `{id}`, which is not in the manifest")]
    UnknownLocal { id: String },
    #[error("--pinned names bundle `{id}`, which is not in the manifest")]
    UnknownPinned { id: String },
    #[error("{}", collision_report(.0))]
    Collisions(Vec<(String, Collision)>),
    #[error(
        "bundle `{id}`: {dir} carries both index.md and _index.md, and the \
         rename that makes the listing a section would overwrite one with the \
         other. Delete the _index.md: okf-assemble writes it"
    )]
    ListingWouldBeOverwritten { id: String, dir: String },
}

/// Every collision this run found, one per line, naming both source files and
/// the URL they contend for.
///
/// All of them rather than the first, because a bundle that grew the shape
/// once usually grew it from a scaffold and has it more than once, and a fix
/// per run is a fix per hour.
fn collision_report(found: &[(String, Collision)]) -> String {
    let lines: Vec<String> = found
        .iter()
        .map(|(id, collision)| {
            let Collision { page, listing, url } = collision;
            format!("  {id}/{page} and {id}/{listing} both publish at /{id}/{url}/")
        })
        .collect();
    format!(
        "a page and a directory listing cannot both publish at one URL, and \
         Hugo keeps one of them without saying which:\n{}\n\nFold the page into \
         the listing, or rename it to something that is not its sibling \
         directory's name. Fix it in the bundle repository: okf-check \
         reports it there, and every tenant mounting the bundle hits it here.",
        lines.join("\n")
    )
}

fn io<E: Into<std::io::Error>>(context: impl Into<String>) -> impl FnOnce(E) -> AssembleError {
    move |source| AssembleError::Io {
        context: context.into(),
        source: source.into(),
    }
}

/// A source directory already at a known commit, offered in place of a fetch.
#[derive(Debug, Clone)]
pub struct Pinned {
    /// The bundle repository root, typically a nix store path.
    pub path: PathBuf,
    /// The commit the caller fetched that path at. Compared against the
    /// manifest's `rev`; a mismatch refuses the whole assembly.
    pub rev: String,
}

/// Everything an assembly run needs beyond the manifest itself.
pub struct Options {
    /// The site repository root. Every other path is relative to it.
    pub root: PathBuf,
    /// Working trees standing in for a fetch, for this invocation only.
    ///
    /// A local override is an argument and is never written to the manifest:
    /// when a working tree and the pinned `rev` disagree, the manifest wins.
    pub locals: BTreeMap<String, PathBuf>,
    /// Sources already fetched at a claimed commit, standing in for the fetch.
    ///
    /// The claim is verified, not trusted: assembly refuses a source whose
    /// rev is not the manifest's own pin. This is how a nix sandbox hands
    /// over store paths it fetched at evaluation — the caller says what it
    /// fetched, the manifest says what it pinned, and the build fails the
    /// moment the two disagree. A verified pin is the pinned corpus, so it
    /// is never stamped as a local override.
    pub pinned: BTreeMap<String, Pinned>,
    /// Where `mermaid.min.js` is copied from.
    pub mermaid: Option<PathBuf>,
    /// Also write the assembled tree as one archive.
    pub tarball: bool,
}

impl Options {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            locals: BTreeMap::new(),
            pinned: BTreeMap::new(),
            mermaid: None,
            tarball: false,
        }
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn content(&self) -> PathBuf {
        self.root.join("content")
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub bundles: usize,
    pub files: usize,
    pub renamed: usize,
    pub rewritten: usize,
    pub local: Vec<String>,
    /// Bundles assembled from a `--pinned` source whose rev matched the
    /// manifest. Verified pins, never stamped.
    pub pinned: Vec<String>,
}

/// Assemble the whole content tree from the manifest.
///
/// The content tree is removed and rebuilt from empty on every run, so the
/// output is a function of the manifest rather than of whatever the last run
/// left behind. That is half of what makes the build reproducible; the pinned
/// `rev` is the other half.
///
/// # Errors
///
/// Fails when a bundle cannot be fetched or copied, when `--local` names a
/// bundle the manifest does not hold, or when any write fails.
pub fn assemble(manifest: &Manifest, options: &Options) -> Result<Outcome, AssembleError> {
    for id in options.locals.keys() {
        if !manifest.bundles.iter().any(|b| b.id == *id) {
            return Err(AssembleError::UnknownLocal { id: id.clone() });
        }
    }
    for id in options.pinned.keys() {
        if !manifest.bundles.iter().any(|b| b.id == *id) {
            return Err(AssembleError::UnknownPinned { id: id.clone() });
        }
        if options.locals.contains_key(id) {
            return Err(AssembleError::PinnedAndLocal { id: id.clone() });
        }
    }

    let content = options.content();
    reset_dir(&content)?;

    let extensions = manifest.asset_extensions();
    let mut outcome = Outcome::default();
    let mut collisions: Vec<(String, Collision)> = Vec::new();
    for bundle in &manifest.bundles {
        let source = source_tree(bundle, options, &mut outcome)?;
        let dest = content.join(&bundle.id);
        outcome.files = outcome
            .files
            .saturating_add(copy_bundle(&source, &dest, &extensions)?);
        // Before the rename, so the paths reported are the ones an author will
        // find in the bundle repository rather than the `_index.md` this step
        // is about to write.
        collisions.extend(
            crate::collision::find(&dest, &[])
                .into_iter()
                .map(|found| (bundle.id.clone(), found)),
        );
        outcome.renamed = outcome
            .renamed
            .saturating_add(rename_indexes(&dest, &bundle.id)?);
        outcome.rewritten = outcome
            .rewritten
            .saturating_add(crate::sitelinks::rewrite_tree(
                &dest,
                &bundle.id,
                &bundle.site_absolute_base,
            )?);
        outcome.bundles = outcome.bundles.saturating_add(1);
    }
    // After every bundle, so one run names every collision the tenant has
    // rather than the first one it walked into.
    if !collisions.is_empty() {
        return Err(AssembleError::Collisions(collisions));
    }

    write_root_index(manifest, &content)?;
    crate::sitegen::write_shared(&options.root)?;
    crate::sitegen::write_hugo_config(manifest, &options.root, &outcome.local)?;
    // The nix fetch specification lands in the tree on every run, the way the
    // justfile does, so a rolled pin reaches `nix/bundles.nix` in the same
    // build that assembles it.
    crate::sitegen::write_bundles_nix(manifest, &options.root)?;
    copy_mermaid(options)?;
    crate::sitegen::write_build_lock(manifest, &options.root)?;
    if options.tarball {
        write_tarball(&options.root, &content)?;
    }
    Ok(outcome)
}

/// Where this bundle's files come from: a working tree, a verified pinned
/// source, or a pinned fetch.
fn source_tree(
    bundle: &Bundle,
    options: &Options,
    outcome: &mut Outcome,
) -> Result<PathBuf, AssembleError> {
    if let Some(local) = options.locals.get(&bundle.id) {
        if !local.is_dir() {
            return Err(AssembleError::LocalMissing {
                id: bundle.id.clone(),
                path: local.display().to_string(),
            });
        }
        outcome.local.push(bundle.id.clone());
        return subdir_of(bundle, local);
    }
    if let Some(pinned) = options.pinned.get(&bundle.id) {
        if !pinned.path.is_dir() {
            return Err(AssembleError::PinnedMissing {
                id: bundle.id.clone(),
                path: pinned.path.display().to_string(),
            });
        }
        // The whole point of --pinned over --local: the caller's claim is
        // checked against the manifest, and a verified pin is *not* a local
        // override, so nothing downstream stamps the pages. A rev that
        // disagrees is a fetch specification that drifted from site.toml,
        // and it fails the build here rather than shipping the wrong corpus
        // under a lock file that claims the manifest's revs.
        if pinned.rev != bundle.rev {
            return Err(AssembleError::PinnedRevMismatch {
                id: bundle.id.clone(),
                fetched: pinned.rev.clone(),
                pinned: bundle.rev.clone(),
            });
        }
        outcome.pinned.push(bundle.id.clone());
        return subdir_of(bundle, &pinned.path);
    }
    let work = options.work().join(&bundle.id);
    fetch(bundle, &work)?;
    subdir_of(bundle, &work)
}

fn subdir_of(bundle: &Bundle, tree: &Path) -> Result<PathBuf, AssembleError> {
    let root = if bundle.subdir == "." {
        tree.to_path_buf()
    } else {
        tree.join(&bundle.subdir)
    };
    if !root.is_dir() {
        return Err(AssembleError::SubdirMissing {
            id: bundle.id.clone(),
            subdir: bundle.subdir.clone(),
            rev: bundle.rev.clone(),
        });
    }
    Ok(root)
}

/// Fetch exactly the pinned commit, and never the branch.
///
/// `git fetch --depth 1 <repo> <rev>` asks the remote for one commit. Asking
/// for `ref` instead would let a runner pick up whatever landed on the branch
/// after the manifest was reviewed, which is the difference between a
/// reproducible build and a build that agrees with the manifest most of the
/// time.
fn fetch(bundle: &Bundle, work: &Path) -> Result<(), AssembleError> {
    std::fs::create_dir_all(work).map_err(io(format!("creating {}", work.display())))?;
    if !work.join(".git").exists() {
        run(
            "git",
            &["init", "--quiet", &work.display().to_string()],
            None,
        )?;
    }
    let dir = work.display().to_string();
    run(
        "git",
        &[
            "-C",
            &dir,
            // Local transport runs its own upload-pack, which refuses a bare
            // commit unless told otherwise. A file:// remote is how the
            // fixtures exercise this path without a network.
            "-c",
            "uploadpack.allowAnySHA1InWant=true",
            "fetch",
            "--quiet",
            "--depth",
            "1",
            &bundle.repo,
            &bundle.rev,
        ],
        None,
    )?;
    run(
        "git",
        &[
            "-C",
            &dir,
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--force",
            "FETCH_HEAD",
        ],
        None,
    )
}

/// Copy the markdown and the allowlisted assets, and nothing else.
///
/// An allowlist rather than a filter, because a bundle holds more than its
/// documents: mail archives, tokens, signed agreements, build output. What
/// reaches the site is what somebody named.
fn copy_bundle(source: &Path, dest: &Path, extensions: &[String]) -> Result<usize, AssembleError> {
    std::fs::create_dir_all(dest).map_err(io(format!("creating {}", dest.display())))?;
    let mut copied = 0usize;
    let mut stack = vec![(source.to_path_buf(), dest.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        let entries =
            std::fs::read_dir(&from).map_err(io(format!("reading {}", from.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                let into = to.join(&name);
                stack.push((path, into));
            } else if wanted(&name, extensions) {
                std::fs::create_dir_all(&to).map_err(io(format!("creating {}", to.display())))?;
                copy_writable(&path, &to.join(&name))?;
                copied = copied.saturating_add(1);
            }
        }
    }
    Ok(copied)
}

fn wanted(name: &str, extensions: &[String]) -> bool {
    if crate::walk::is_markdown(name) {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        let extension = extension.to_lowercase();
        extensions.contains(&extension)
    })
}

/// Rename every `index.md` to `_index.md`.
///
/// **Mandatory, not cosmetic.** OKF §8 reserves `index.md` for a directory
/// listing; Hugo reads a directory holding one as a leaf bundle and demotes
/// every sibling from a page to a page resource. The listing survives the
/// rename intact — it is the same bytes under a name Hugo reads as a section —
/// and the raw-markdown route serves it unchanged.
///
/// A directory already carrying an `_index.md` is refused rather than
/// renamed over. `std::fs::rename` replaces its destination, so a bundle
/// holding both names in one directory used to lose the `_index.md` here,
/// silently, which is the same failure the rename exists to prevent.
///
/// # Errors
///
/// Fails when a directory cannot be read, when a rename would overwrite a
/// listing that is already there, or when a rename cannot be performed.
pub fn rename_indexes(root: &Path, id: &str) -> Result<usize, AssembleError> {
    let mut renamed = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(io(format!("reading {}", dir.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().is_some_and(|n| n == "index.md") {
                let into = dir.join("_index.md");
                if into.exists() {
                    return Err(AssembleError::ListingWouldBeOverwritten {
                        id: id.to_owned(),
                        dir: crate::walk::to_posix(dir.strip_prefix(root).unwrap_or(&dir)),
                    });
                }
                std::fs::rename(&path, &into)
                    .map_err(io(format!("renaming {}", path.display())))?;
                renamed = renamed.saturating_add(1);
            }
        }
    }
    Ok(renamed)
}

/// The tenant root page, which no bundle owns.
fn write_root_index(manifest: &Manifest, content: &Path) -> Result<(), AssembleError> {
    let title = if manifest.site.title.is_empty() {
        manifest.tenant.clone()
    } else {
        manifest.site.title.clone()
    };
    let body = format!(
        "---\ntitle: {}\ndescription: {}\n---\n\n{}\n",
        yaml_string(&title),
        yaml_string(&manifest.site.description),
        "<!-- Written by okf-assemble. The listing below the fold is Hugo's, \
         built from the mounted bundles. -->"
    );
    std::fs::write(content.join("_index.md"), body).map_err(io(format!(
        "writing {}",
        content.join("_index.md").display()
    )))
}

/// A YAML double-quoted scalar, which is the one quoting style that needs no
/// knowledge of what is inside it.
#[must_use]
pub fn yaml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len().saturating_add(2));
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Copy the mermaid browser bundle out of the store path.
///
/// Copied, never run. The whole trust surface of this build is two pinned
/// binaries plus templates written here, and mermaid executes only in a
/// reader's browser from a file `flake.lock` pins.
fn copy_mermaid(options: &Options) -> Result<(), AssembleError> {
    let source = options
        .mermaid
        .clone()
        .or_else(|| std::env::var_os("OKF_MERMAID_JS").map(PathBuf::from))
        .ok_or(AssembleError::NoMermaid)?;
    if !source.is_file() {
        return Err(AssembleError::NoMermaid);
    }
    let dest = options.root.join("static/js");
    std::fs::create_dir_all(&dest).map_err(io(format!("creating {}", dest.display())))?;
    copy_writable(&source, &dest.join("mermaid.min.js"))
}

/// Copy a file and leave the copy writable.
///
/// Sources here are frequently read-only — a nix store path for the mermaid
/// bundle, and a store-resident fixture in the checks — and `std::fs::copy`
/// carries the mode across. A read-only copy then fails the link rewrite that
/// runs over it moments later, and fails the *next* run's overwrite, both of
/// which look like a permissions problem rather than the design decision they
/// actually are.
fn copy_writable(from: &Path, to: &Path) -> Result<(), AssembleError> {
    use std::os::unix::fs::PermissionsExt as _;

    if to.exists() {
        std::fs::remove_file(to).map_err(io(format!("replacing {}", to.display())))?;
    }
    std::fs::copy(from, to).map_err(io(format!("copying {}", from.display())))?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o644))
        .map_err(io(format!("making {} writable", to.display())))
}

/// The assembled bundle as one file, for an agent that would rather make one
/// request than several hundred.
fn write_tarball(root: &Path, content: &Path) -> Result<(), AssembleError> {
    let dest = root.join("static");
    std::fs::create_dir_all(&dest).map_err(io(format!("creating {}", dest.display())))?;
    let archive = dest.join("bundle.tar.zst");
    run(
        "tar",
        &[
            "--use-compress-program=zstd",
            "--create",
            // Sorted names and a fixed mtime, or the archive changes on every
            // run and the byte-identical-output claim goes with it.
            "--sort=name",
            "--mtime=@0",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "--file",
            &archive.display().to_string(),
            "--directory",
            &content.display().to_string(),
            ".",
        ],
        Some("`just bundle` needs it to write the assembled bundle"),
    )
}

fn reset_dir(dir: &Path) -> Result<(), AssembleError> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(io(format!("clearing {}", dir.display())))?;
    }
    std::fs::create_dir_all(dir).map_err(io(format!("creating {}", dir.display())))
}

/// Run a program, and fail with what it said rather than with its exit code.
pub(crate) fn run(
    program: &'static str,
    args: &[&str],
    purpose: Option<&'static str>,
) -> Result<(), AssembleError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                AssembleError::MissingProgram {
                    program,
                    purpose: purpose.unwrap_or("the assembly step needs it"),
                }
            } else {
                AssembleError::Io {
                    context: format!("running {program}"),
                    source,
                }
            }
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(AssembleError::Command {
        command: format!("{program} {}", args.join(" ")),
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// Ask a remote what its `ref` points at, without fetching anything.
///
/// `--update` uses this and stops: it rewrites `rev` and does not build. A
/// roll-forward is then a one-line diff somebody reviews, rather than a branch
/// that moved under a build nobody was watching.
#[must_use]
pub fn resolve_ref(repo: &str, git_ref: &str) -> Option<String> {
    let line = capture("git", &["ls-remote", "--quiet", repo, git_ref])?;
    let rev = line.split_whitespace().next()?.to_owned();
    crate::manifest::is_commit(&rev).then_some(rev)
}

/// Capture a program's first line of output, or `None` if it is not there.
pub(crate) fn capture(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_takes_markdown_and_images_and_leaves_documents_alone() {
        let extensions: Vec<String> = crate::manifest::DEFAULT_ASSET_EXTENSIONS
            .iter()
            .map(|e| (*e).to_owned())
            .collect();
        assert!(wanted("a.md", &extensions));
        assert!(wanted("diagram.PNG", &extensions));
        assert!(!wanted("agreement.pdf", &extensions));
        assert!(!wanted("mail.eml", &extensions));
        assert!(!wanted("token.json", &extensions));
        assert!(!wanted("notes.txt", &extensions));
    }

    #[test]
    fn a_title_with_a_quote_survives_being_written_as_yaml() {
        assert_eq!(yaml_string(r#"a "b" \ c"#), r#""a \"b\" \\ c""#);
    }
}
