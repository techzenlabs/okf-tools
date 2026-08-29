//! Command-level behavior for `okf-migrate` refusals.

#![expect(
    clippy::expect_used,
    reason = "a panicking assertion is the point of a test"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let sequence = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "okf-migrate-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).expect("fixture should be written");
}

fn run(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_okf-migrate"))
        .arg("--apply")
        .current_dir(root)
        .output()
        .expect("okf-migrate should run")
}

fn track_all(root: &Path) {
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .status()
        .expect("git init should run");
    assert!(initialized.success());
    let added = Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status()
        .expect("git add should run");
    assert!(added.success());
}

#[test]
fn apply_reports_likely_generated_files_and_does_not_write_them() {
    let root = Scratch::new("likely-generated");
    write(
        &root.path().join("okf.toml"),
        "config_version = 1\nbundle_root = \".\"\n\n[[type_rules]]\npath = \"**/*.md\"\ntype = \"Reference\"\n",
    );
    let marker = "<!-- GENERATED: DO NOT EDIT -->\n# Marker\n\nMarker prose.\n";
    let frontmatter = "---\ngenerated: { by: Fixture, at: 2026-08-29 }\n---\n\n# Frontmatter\n\nFrontmatter prose.\n";
    let ordinary = "# Ordinary\n\nOrdinary prose.\n";
    write(&root.path().join("marker.md"), marker);
    write(&root.path().join("frontmatter.md"), frontmatter);
    write(&root.path().join("ordinary.md"), ordinary);
    track_all(root.path());

    let output = run(root.path());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "{stdout}");
    assert!(
        stdout.contains("2 file(s) look generated and were not migrated"),
        "{stdout}"
    );
    assert!(
        stdout.contains("marker.md: likely-generated: marker near start"),
        "{stdout}"
    );
    assert!(
        stdout.contains("frontmatter.md: likely-generated: `generated` frontmatter key"),
        "{stdout}"
    );
    assert!(stdout.contains("Migrated 1 file(s)."), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("marker.md"))
            .expect("marker fixture should be readable"),
        marker
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("frontmatter.md"))
            .expect("frontmatter fixture should be readable"),
        frontmatter
    );
    assert!(
        std::fs::read_to_string(root.path().join("ordinary.md"))
            .expect("ordinary fixture should be readable")
            .starts_with("---\ntype: \"Reference\"")
    );
}

#[test]
fn configured_generated_path_resolves_the_refusal() {
    let root = Scratch::new("configured-generated");
    write(
        &root.path().join("okf.toml"),
        "config_version = 1\nbundle_root = \".\"\n\n[paths]\ngenerated = [\"generated.md\"]\n\n[[type_rules]]\npath = \"**/*.md\"\ntype = \"Reference\"\n",
    );
    let generated = "<!-- GENERATED: DO NOT EDIT -->\n# Generated\n\nGenerated prose.\n";
    write(&root.path().join("generated.md"), generated);
    track_all(root.path());

    let output = run(root.path());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Migrated 0 file(s)."), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("generated.md"))
            .expect("generated fixture should be readable"),
        generated
    );
}

#[test]
fn a_tracked_digest_reference_refuses_the_batch_before_any_write() {
    let root = Scratch::new("pinned-document");
    write(
        &root.path().join("okf.toml"),
        "config_version = 1\nbundle_root = \".\"\n\n[[type_rules]]\npath = \"**/*.md\"\ntype = \"Reference\"\n",
    );
    let pinned = "# Pinned\n\nPinned prose.\n";
    let ordinary = "# Ordinary\n\nOrdinary prose.\n";
    write(&root.path().join("pinned.md"), pinned);
    write(&root.path().join("ordinary.md"), ordinary);
    write(
        &root.path().join("model-manifest.json"),
        "{\n  \"path\": \"pinned.md\",\n  \"byte_length\": 26,\n  \"sha256\": \"fixture\"\n}\n",
    );
    track_all(root.path());

    let output = run(root.path());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "{stdout}");
    assert!(
        stdout.contains("tracked non-Markdown file(s) may pin document bytes"),
        "{stdout}"
    );
    assert!(
        stdout.contains("pinned.md <- model-manifest.json:2 (byte_length at line 3)"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("pinned.md"))
            .expect("pinned fixture should be readable"),
        pinned
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("ordinary.md"))
            .expect("ordinary fixture should be readable"),
        ordinary
    );
}

#[test]
fn a_manifest_above_a_nested_bundle_refuses_the_batch() {
    let root = Scratch::new("nested-pinned-document");
    let bundle = root.path().join("bundle");
    std::fs::create_dir(&bundle).expect("bundle directory should be created");
    write(
        &bundle.join("okf.toml"),
        "config_version = 1\nbundle_root = \".\"\n\n[[type_rules]]\npath = \"**/*.md\"\ntype = \"Reference\"\n",
    );
    let pinned = "# Pinned\n\nPinned prose.\n";
    write(&bundle.join("pinned.md"), pinned);
    write(
        &root.path().join("model-manifest.json"),
        "{\n  \"path\": \"bundle/pinned.md\",\n  \"sha256\": \"fixture\"\n}\n",
    );
    track_all(root.path());

    let output = run(&bundle);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "{stdout}");
    assert!(
        stdout.contains("pinned.md <- model-manifest.json:2 (sha256 at line 3)"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(bundle.join("pinned.md"))
            .expect("pinned fixture should be readable"),
        pinned
    );
}

#[test]
fn markdown_and_untracked_digest_references_do_not_block_migration() {
    let root = Scratch::new("untracked-pin");
    write(
        &root.path().join("okf.toml"),
        "config_version = 1\nbundle_root = \".\"\n\n[paths]\ngenerated = [\"manifest.md\"]\n\n[[type_rules]]\npath = \"**/*.md\"\ntype = \"Reference\"\n",
    );
    write(
        &root.path().join("ordinary.md"),
        "# Ordinary\n\nOrdinary prose.\n",
    );
    write(
        &root.path().join("manifest.md"),
        "{\n  \"path\": \"ordinary.md\",\n  \"sha256\": \"fixture\"\n}\n",
    );
    track_all(root.path());
    write(
        &root.path().join("untracked.json"),
        "{\n  \"path\": \"ordinary.md\",\n  \"sha256\": \"fixture\"\n}\n",
    );

    let output = run(root.path());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Migrated 1 file(s)."), "{stdout}");
}
