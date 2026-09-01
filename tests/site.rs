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
    assert_eq!(outcome.files, 105);
    // Nineteen `index.md` across the two bundles, and not one survives under
    // that name: Hugo would read each of their directories as a leaf bundle.
    // Nineteen, not twenty, with `alpha/loose/` in the fixture: that
    // directory deliberately has none, which is what makes `list.html` fall
    // back to listing the pages Hugo found — and `alpha/journal/log.md` is a
    // page, not an index, however listing-shaped its directory is.
    assert_eq!(outcome.renamed, 19);
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

/// A `--pinned` source whose rev matches the manifest is the pinned corpus:
/// assembled without a fetch, verified, and never stamped as a local build.
#[test]
fn a_pinned_source_at_the_manifest_rev_is_verified_and_not_stamped() {
    let root = scratch("pinned-clean");
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let mut options = Options::new(root.clone());
    options.pinned.insert(
        "alpha".to_owned(),
        assemble::Pinned {
            path: fixture("site/alpha"),
            rev: "1".repeat(40),
        },
    );
    options.pinned.insert(
        "beta".to_owned(),
        assemble::Pinned {
            path: fixture("site/beta"),
            rev: "2".repeat(40),
        },
    );
    options.mermaid = Some(fixture("site/tenant/site.toml"));
    let outcome = assemble::assemble(&manifest, &options).unwrap_or_default();
    assert_eq!(outcome.pinned, ["alpha", "beta"]);
    assert!(outcome.local.is_empty());
    // The stamp lives in the generated hugo.toml, and a verified pin must
    // not carry it: the footer span and the pagefind index both follow from
    // this one key.
    let hugo = std::fs::read_to_string(root.join("hugo.toml")).unwrap_or_default();
    assert!(
        !hugo.contains("okf_local_bundles"),
        "a verified pin was stamped as a local build:\n{hugo}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A `--pinned` source at any other rev is a drifted fetch specification and
/// refuses the whole assembly, naming both commits.
#[test]
fn a_pinned_source_at_the_wrong_rev_refuses_the_assembly() {
    let root = scratch("pinned-drift");
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let mut options = Options::new(root.clone());
    options.pinned.insert(
        "alpha".to_owned(),
        assemble::Pinned {
            path: fixture("site/alpha"),
            rev: "f".repeat(40),
        },
    );
    options.pinned.insert(
        "beta".to_owned(),
        assemble::Pinned {
            path: fixture("site/beta"),
            rev: "2".repeat(40),
        },
    );
    options.mermaid = Some(fixture("site/tenant/site.toml"));
    let err = match assemble::assemble(&manifest, &options) {
        Ok(_) => String::new(),
        Err(err) => err.to_string(),
    };
    assert!(err.contains(&"f".repeat(40)), "no fetched rev named: {err}");
    assert!(err.contains(&"1".repeat(40)), "no pinned rev named: {err}");
    assert!(
        err.contains("okf-assemble --bundles"),
        "no fix named: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// One bundle cannot be both the verified pin and a working-tree override.
#[test]
fn a_bundle_named_by_both_pinned_and_local_is_refused() {
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let root = scratch("pinned-and-local");
    let mut options = Options::new(root.clone());
    options
        .locals
        .insert("alpha".to_owned(), fixture("site/alpha"));
    options.pinned.insert(
        "alpha".to_owned(),
        assemble::Pinned {
            path: fixture("site/alpha"),
            rev: "1".repeat(40),
        },
    );
    let err = match assemble::assemble(&manifest, &options) {
        Ok(_) => String::new(),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("both"), "not refused, or wrongly: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

/// `--pinned` naming a bundle nobody declared is a typo, not a new bundle.
#[test]
fn a_pinned_source_for_an_unknown_bundle_is_refused() {
    let manifest = Manifest::load(&fixture("site/tenant/site.toml")).unwrap_or_default();
    let root = scratch("unknown-pinned");
    let mut options = Options::new(root.clone());
    options.pinned.insert(
        "nosuchbundle".to_owned(),
        assemble::Pinned {
            path: fixture("site/alpha"),
            rev: "1".repeat(40),
        },
    );
    let err = match assemble::assemble(&manifest, &options) {
        Ok(_) => String::new(),
        Err(err) => err.to_string(),
    };
    assert!(
        err.contains("nosuchbundle"),
        "not refused, or wrongly: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
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

/// A `fetch` key the build does not know, a bundle it cannot derive an ssh
/// URL for, and a manifest where only some bundles opt in are all refused at
/// load, where the message names the bundle.
#[test]
fn a_fetch_key_that_cannot_be_honoured_is_refused_at_load() {
    let root = scratch("fetch-refusals");
    let write = |body: &str| {
        let path = root.join("site.toml");
        let _ = std::fs::write(&path, body);
        path
    };
    let head = "schema_version = 1\ntenant = \"t\"\n[site]\ntitle = \"T\"\n\
                base_url = \"https://t.invalid/\"\n";
    let bundle = |id: &str, repo: &str, tail: &str| {
        format!(
            "[[bundle]]\nid = \"{id}\"\nrepo = \"{repo}\"\nref = \"refs/heads/main\"\n\
             rev = \"{}\"\n{tail}",
            "1".repeat(40)
        )
    };

    let refusal = |body: &str| match Manifest::load(&write(body)) {
        Ok(_) => String::new(),
        Err(err) => err.to_string(),
    };

    let unknown = format!(
        "{head}{}",
        bundle(
            "a",
            "https://forge.invalid/a.git",
            "fetch = \"git+https\"\n"
        )
    );
    let err = refusal(&unknown);
    assert!(err.contains("git+ssh"), "not refused, or wrongly: {err}");

    let underivable = format!(
        "{head}{}",
        bundle("a", "file:///somewhere/a", "fetch = \"git+ssh\"\n")
    );
    let err = refusal(&underivable);
    assert!(err.contains("https://"), "not refused, or wrongly: {err}");

    let mixed = format!(
        "{head}{}{}",
        bundle("a", "https://forge.invalid/a.git", "fetch = \"git+ssh\"\n"),
        bundle("b", "https://forge.invalid/b.git", "")
    );
    let err = refusal(&mixed);
    assert!(err.contains("opt"), "not refused, or wrongly: {err}");

    let _ = std::fs::remove_dir_all(&root);
}

/// An assembly of a manifest that opts in lands `nix/bundles.nix` in the
/// tree the way it lands the justfile, so the pin a `--update` rolls reaches
/// the nix fetch specification in the same build.
#[test]
fn assembly_writes_the_nix_fetch_specification_for_an_opted_in_tenant() {
    let root = scratch("bundles-nix");
    let manifest_text = format!(
        "schema_version = 1\ntenant = \"t\"\n[site]\ntitle = \"T\"\n\
         base_url = \"https://t.invalid/\"\n[[bundle]]\nid = \"alpha\"\n\
         repo = \"https://forge.invalid/owner/alpha.git\"\n\
         ref = \"refs/heads/main\"\nrev = \"{}\"\nfetch = \"git+ssh\"\n",
        "1".repeat(40)
    );
    let path = root.join("site.toml");
    let _ = std::fs::write(&path, manifest_text);
    let manifest = Manifest::load(&path).unwrap_or_default();
    let mut options = Options::new(root.clone());
    options
        .locals
        .insert("alpha".to_owned(), fixture("site/alpha"));
    options.mermaid = Some(path.clone());
    assert!(assemble::assemble(&manifest, &options).is_ok());
    let generated = std::fs::read_to_string(root.join("nix/bundles.nix")).unwrap_or_default();
    assert!(
        generated.contains("url = \"ssh://git@forge.invalid/owner/alpha.git\";"),
        "{generated}"
    );
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

/// Assemble the subdir-mounted fixture tenant, whose one bundle takes `docs/`
/// from a tree that also holds `scripts/` and `schemas/`.
fn assemble_subdir_fixture(
    label: &str,
    max_asset_bytes: Option<u64>,
) -> (PathBuf, Result<assemble::Outcome, assemble::AssembleError>) {
    let root = scratch(label);
    let mut manifest = Manifest::load(&fixture("site/tenant-subdir/site.toml")).unwrap_or_default();
    if max_asset_bytes.is_some() {
        manifest.max_asset_bytes = max_asset_bytes;
    }
    let mut locals = BTreeMap::new();
    locals.insert("gamma".to_owned(), fixture("site/gamma"));
    let mut options = Options::new(root.clone());
    options.locals = locals;
    options.mermaid = Some(fixture("site/tenant-subdir/site.toml"));
    let outcome = assemble::assemble(&manifest, &options);
    (root, outcome)
}

/// The referenced-files pass, over every link shape the fixture carries: two
/// files leave the subdir and mount under `/_files/`, and every link that must
/// not be touched — a directory, a dead target, an extension off the
/// allowlist, a path leaving the repository — stays exactly as written.
#[test]
fn a_referenced_file_outside_the_subdir_is_mounted_and_its_link_rewritten() {
    let (root, outcome) = assemble_subdir_fixture("refassets", None);
    let outcome = outcome.unwrap_or_default();
    assert_eq!(outcome.referenced, 2);

    assert!(root.join("static/_files/gamma/scripts/deploy.sh").is_file());
    assert!(
        root.join("static/_files/gamma/schemas/thing.json")
            .is_file()
    );
    // Off the allowlist, and outside the repository: neither mounts.
    assert!(!root.join("static/_files/gamma/scripts/secret.pem").exists());
    assert!(!root.join("static/_files/gamma/outside.sh").exists());

    let guide = std::fs::read_to_string(root.join("content/gamma/guide.md")).unwrap_or_default();
    assert!(
        guide.contains("](/_files/gamma/scripts/deploy.sh)"),
        "{guide}"
    );
    assert!(
        guide.contains("](/_files/gamma/schemas/thing.json)"),
        "{guide}"
    );
    // The in-docs asset takes the ordinary route, not the `/_files/` one.
    assert!(guide.contains("](/gamma/data.json)"), "{guide}");
    // §11 makes a broken link a link: each of these stays as written.
    assert!(guide.contains("](../scripts/)"), "{guide}");
    assert!(guide.contains("](../missing.sh)"), "{guide}");
    assert!(guide.contains("](../scripts/secret.pem)"), "{guide}");
    assert!(guide.contains("](../../outside.sh)"), "{guide}");

    // The same file linked from one level deeper: one copy, one URL.
    let nested =
        std::fs::read_to_string(root.join("content/gamma/sub/nested.md")).unwrap_or_default();
    assert!(
        nested.contains("](/_files/gamma/scripts/deploy.sh)"),
        "{nested}"
    );

    // Every non-markdown byte copied for the bundle is on its account.
    let expected: u64 = ["scripts/deploy.sh", "schemas/thing.json", "docs/data.json"]
        .iter()
        .map(|p| {
            std::fs::metadata(fixture("site/gamma").join(p))
                .map(|m| m.len())
                .unwrap_or_default()
        })
        .sum();
    assert!(expected > 0);
    assert_eq!(outcome.asset_bytes, [("gamma".to_owned(), expected)]);

    let _ = std::fs::remove_dir_all(&root);
}

/// The payload cap fails the build the moment it is crossed, naming the
/// spender, rather than measuring after everything landed.
#[test]
fn an_asset_payload_past_the_budget_refuses_the_assembly() {
    let (root, outcome) = assemble_subdir_fixture("budget", Some(1));
    let err = outcome.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("max_asset_bytes"), "{err}");
    assert!(err.contains("gamma:"), "{err}");
    assert!(err.contains("blob storage"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A second run rebuilds `/_files/` from empty, so a link removed upstream
/// takes its copy with it rather than serving stale forever.
#[test]
fn a_second_assembly_rebuilds_the_mounted_references_from_empty() {
    let (root, first) = assemble_subdir_fixture("refassets-again", None);
    let first = first.unwrap_or_default();
    let stale = root.join("static/_files/gamma/left-behind.sh");
    let _ = std::fs::write(&stale, "echo stale");

    let mut manifest = Manifest::load(&fixture("site/tenant-subdir/site.toml")).unwrap_or_default();
    manifest.max_asset_bytes = None;
    let mut locals = BTreeMap::new();
    locals.insert("gamma".to_owned(), fixture("site/gamma"));
    let mut options = Options::new(root.clone());
    options.locals = locals;
    options.mermaid = Some(fixture("site/tenant-subdir/site.toml"));
    let second = assemble::assemble(&manifest, &options).unwrap_or_default();

    assert_eq!(first.referenced, second.referenced);
    assert_eq!(first.asset_bytes, second.asset_bytes);
    assert!(!stale.exists());
    assert!(root.join("static/_files/gamma/scripts/deploy.sh").is_file());

    let _ = std::fs::remove_dir_all(&root);
}

/// A pin bump changes the rev and not one other byte.
///
/// The defect this gates: `--update` wrote the manifest back through the TOML
/// serialiser, which emits values and has no idea a comment exists. Run
/// against a live manifest it deleted 65 of 105 lines, and among them the
/// paragraph recording that one private repository must never be mounted on a
/// published site. That paragraph is the only place the reasoning is written
/// down, and `--update` is the single command the standard tells every tenant
/// to run for a roll-forward, so nothing else had to go wrong for it to be
/// lost — somebody just had to not read the whole diff.
///
/// The fixture carries a comment in each of the four places a serialiser eats
/// one: above the file, above a `[[bundle]]` block, trailing a `rev` on its
/// own line, and standing alone between two bundles. The assertion is byte
/// equality against the original with the one rev substituted, so a preserved
/// comment that moved, or a blank line that closed up, fails it just as a
/// deleted paragraph does.
#[test]
fn a_pin_bump_rewrites_the_rev_and_nothing_else() {
    let root = scratch("repin");
    let original = std::fs::read_to_string(fixture("site/commented/site.toml")).unwrap_or_default();
    assert!(
        original.contains("never be mounted") && original.contains("# rolled by hand once"),
        "the fixture lost the comments it exists to carry"
    );
    let path = root.join("site.toml");
    let _ = std::fs::write(&path, &original);

    let rolled = "3".repeat(40);
    let outcome = manifest::set_bundle_rev(&path, "alpha", &rolled);
    assert!(outcome.is_ok(), "{outcome:?}");

    let after = std::fs::read_to_string(&path).unwrap_or_default();
    let expected = original.replace(&"1".repeat(40), &rolled);
    assert_ne!(after, original, "the repin wrote nothing");
    assert_eq!(
        after, expected,
        "a pin bump changed something other than alpha's rev"
    );

    // And the manifest still means what it did, with the one value moved.
    let reloaded = Manifest::load(&path).unwrap_or_default();
    assert_eq!(reloaded.bundles[0].rev, rolled);
    assert_eq!(reloaded.bundles[1].rev, "2".repeat(40));
    let _ = std::fs::remove_dir_all(&root);
}

/// A bundle nobody declared is refused, and the file is left as it was.
///
/// The edit is the write, so "no such bundle" has to fail before touching the
/// file rather than after truncating it.
#[test]
fn a_repin_of_an_unknown_bundle_refuses_and_leaves_the_file_alone() {
    let root = scratch("repin-unknown");
    let original = std::fs::read_to_string(fixture("site/commented/site.toml")).unwrap_or_default();
    let path = root.join("site.toml");
    let _ = std::fs::write(&path, &original);
    assert!(manifest::set_bundle_rev(&path, "nosuchbundle", &"3".repeat(40)).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap_or_default(),
        original,
        "a refused repin still rewrote the manifest"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The other spelling of a bundle list. `bundle = [{ ... }]` parses to the
/// same manifest as `[[bundle]]`, so a repin that only knew about the block
/// form would report success over a file it never edited.
#[test]
fn a_repin_reaches_a_bundle_written_as_an_inline_table() {
    let root = scratch("repin-inline");
    let path = root.join("site.toml");
    let body = format!(
        "schema_version = 1\ntenant = \"t\"\n\n# still a comment\n\
         bundle = [{{ id = \"alpha\", repo = \"https://forge.invalid/a.git\", \
         ref = \"refs/heads/main\", rev = \"{}\" }}]\n\n\
         [site]\ntitle = \"T\"\nbase_url = \"https://t.invalid/\"\n",
        "1".repeat(40)
    );
    let _ = std::fs::write(&path, &body);
    let rolled = "3".repeat(40);
    let outcome = manifest::set_bundle_rev(&path, "alpha", &rolled);
    assert!(outcome.is_ok(), "{outcome:?}");
    let after = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(after, body.replace(&"1".repeat(40), &rolled));
    assert!(after.contains("# still a comment"));
    let _ = std::fs::remove_dir_all(&root);
}
