//! Assembly, in the crate's own test suite.
//!
//! The gates that need Hugo and Pagefind live in `nix/checks/`, because the
//! failures they catch are properties of rendered output. What is testable
//! here is everything upstream of the generator: the rename, the link
//! rewriting, the shared files landing, and the refusals that stop a manifest
//! doing something it should not.
//!
//! Every fixture is synthetic. None is derived from a real corpus, imported
//! from private history, or named after anybody, which is a rule this
//! repository's own CI enforces rather than a habit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use okf_tools::assemble::{self, Options};
use okf_tools::layouts;
use okf_tools::manifest::{self, Manifest};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("okf-site-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Assemble the fixture tenant into a scratch directory.
fn assemble_fixture(label: &str) -> (PathBuf, assemble::Outcome) {
    let root = scratch(label);
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let mut locals = BTreeMap::new();
    locals.insert("alpha".to_owned(), fixture("site/alpha"));
    locals.insert("beta".to_owned(), fixture("site/beta"));
    let mut options = Options::new(root.clone());
    options.locals = locals;
    options.mermaid = Some(fixture("site/tenant/site.toml"));
    let outcome = assemble::assemble(&manifest, &options).unwrap_or_default();
    (root, outcome)
}

#[test]
fn assembly_renames_every_index_and_lands_the_shared_files() {
    let (root, outcome) = assemble_fixture("assemble");
    assert_eq!(outcome.bundles, 2);
    assert_eq!(outcome.files, 19);
    // Eight `index.md` across the two bundles, and not one survives under that
    // name: Hugo would read each of their directories as a leaf bundle. Still
    // eight with `alpha/loose/` in the fixture: that directory deliberately
    // has none, which is what makes `list.html` fall back to listing the
    // pages Hugo found.
    assert_eq!(outcome.renamed, 8);
    assert_eq!(outcome.local, ["alpha", "beta"]);

    for path in [
        "content/_index.md",
        "content/alpha/_index.md",
        "content/alpha/runbooks/_index.md",
        "content/alpha/Documentation/_index.md",
        "content/beta/notes/_index.md",
        // Two levels of titleless section, which is what the breadcrumb has
        // to name from directory names alone.
        "content/alpha/archive/_index.md",
        "content/alpha/archive/2026/_index.md",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    assert!(!root.join("content/alpha/index.md").exists());

    for file in layouts::LAYOUTS.iter().chain(layouts::SHARED.iter()) {
        assert!(root.join(file.path).is_file(), "missing {}", file.path);
    }
    assert!(root.join("hugo.toml").is_file());
    assert!(root.join("static/build-lock.json").is_file());

    let _ = std::fs::remove_dir_all(&root);
}

/// Every shape of link the corpus actually contains, after assembly.
#[test]
fn assembly_resolves_links_under_the_mount() {
    let (root, _) = assemble_fixture("links");
    let listing = std::fs::read_to_string(root.join("content/alpha/_index.md")).unwrap_or_default();
    assert!(listing.contains("](/alpha/runbooks/)"), "{listing}");

    let page = std::fs::read_to_string(root.join("content/alpha/decisions/0001-pick-a-thing.md"))
        .unwrap_or_default();
    // OKF §6's leading-slash form, which would otherwise resolve at the site
    // root rather than under this bundle's mount.
    assert!(page.contains("](/alpha/runbooks/relay-restart/)"), "{page}");

    let runbook = std::fs::read_to_string(root.join("content/alpha/runbooks/relay-restart.md"))
        .unwrap_or_default();
    // A document-relative link, which Hugo's page-per-directory URLs would
    // otherwise resolve one level too deep.
    assert!(
        runbook.contains("](/alpha/decisions/0001-pick-a-thing/)"),
        "{runbook}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A second run over the same directory produces the same tree.
///
/// The content tree is removed and rebuilt from empty every time, so the
/// output is a function of the manifest rather than of whatever the last run
/// left behind. That is half of what makes the build reproducible.
#[test]
fn a_second_assembly_produces_the_same_tree() {
    let (root, first) = assemble_fixture("idempotent");
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let mut locals = BTreeMap::new();
    locals.insert("alpha".to_owned(), fixture("site/alpha"));
    locals.insert("beta".to_owned(), fixture("site/beta"));
    let mut options = Options::new(root.clone());
    options.locals = locals;
    options.mermaid = Some(fixture("site/tenant/site.toml"));
    let second = assemble::assemble(&manifest, &options).unwrap_or_default();

    assert_eq!(first.files, second.files);
    assert_eq!(first.renamed, second.renamed);
    assert_eq!(first.rewritten, second.rewritten);

    let _ = std::fs::remove_dir_all(&root);
}

/// A local override is an argument, never a manifest edit.
#[test]
fn a_local_override_does_not_touch_the_manifest() {
    let path = fixture("site/tenant/site.toml");
    let before = std::fs::read(&path).unwrap_or_default();
    let (root, outcome) = assemble_fixture("override");
    let after = std::fs::read(&path).unwrap_or_default();
    assert_eq!(before, after);
    assert_eq!(outcome.local.len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

/// A manifest naming a credential the tenant does not hold fails on the first
/// line of the job, not at fetch time.
#[test]
fn a_credential_outside_the_allowlist_is_refused() {
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let allow = manifest::read_credentials_allow(&fixture("site/tenant/credentials.allow"))
        .unwrap_or_default();
    assert!(manifest.check_credentials(allow.as_ref()).is_ok());

    let empty = std::collections::BTreeSet::new();
    assert!(manifest.check_credentials(Some(&empty)).is_err());
    // No allowlist at all means the tenant has not opted in, and nothing is
    // checked. An empty one means nothing is allowed. They are not the same.
    assert!(manifest.check_credentials(None).is_ok());
}

/// `--local` naming a bundle nobody declared is a typo, not a new bundle.
#[test]
fn a_local_override_for_an_unknown_bundle_is_refused() {
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let root = scratch("unknown-local");
    let mut options = Options::new(root.clone());
    options
        .locals
        .insert("nosuchbundle".to_owned(), fixture("site/alpha"));
    assert!(assemble::assemble(&manifest, &options).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// The asset allowlist copies images and leaves everything else behind.
#[test]
fn only_markdown_and_allowlisted_assets_reach_the_content_tree() {
    let (root, _) = assemble_fixture("allowlist");
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![root.join("content")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                found.push(name.to_owned());
            }
        }
    }
    assert!(!found.is_empty());
    assert!(
        found.iter().all(|name| okf_tools::walk::is_markdown(name)),
        "the fixture bundles hold only markdown, and only markdown arrived: {found:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
