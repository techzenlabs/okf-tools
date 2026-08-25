//! The regression suite the Python these tools port never had.
//!
//! Every fixture bundle under `fixtures/` is synthetic. None is derived from a
//! real corpus, imported from private history, or named after a client, which
//! is a rule this repository's own CI enforces rather than a habit.
//!
//! The bundles cover the six behaviours the port had to carry across
//! unchanged — duplicate-key detection, tab indentation, marker-block
//! rewriting, month-grouped listings, drop-folder suppression and
//! deepest-first ordering — plus the three diagnostics the port adds.

use std::path::{Path, PathBuf};

use okf_tools::{check, config::Config, index};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn load(name: &str) -> (PathBuf, Config) {
    let root = fixture(name);
    let config = Config::load(&root).unwrap_or_default();
    (root, config)
}

/// Copy a fixture somewhere writable, because the index generator writes.
fn scratch_copy(name: &str, label: &str) -> PathBuf {
    let dest = std::env::temp_dir().join(format!("okf-tools-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&fixture(name), &dest);
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    let Ok(()) = std::fs::create_dir_all(to) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            let _ = std::fs::copy(&source, &target);
        }
    }
}

#[test]
fn conformance_fixture_reports_exactly_the_expected_diagnostics() {
    let (root, config) = load("conformance");
    let report = check::check_bundle(&root, &config).unwrap_or_default();

    assert_eq!(
        report.warnings,
        [
            "concepts/bad-actor.md: generated.by `NotAnActor` is not <producer>/<version>, human:<id>, or process:<id>",
            "concepts/bad-at.md: verified.at `last-tuesday` is not an ISO 8601 date/datetime",
            "concepts/bad-stale-after.md: stale_after `soon` is not YYYY-MM-DD",
            "concepts/bad-status.md: status `wip` is not draft/stable/deprecated",
            "concepts/no-description.md: no `description` (recommended, §4.2)",
            "concepts/no-title.md: no `title` (recommended, §4.2)",
            "concepts/unknown-type.md: type `Not A Real Type` is not in the bundle vocabulary",
        ]
    );
    assert_eq!(
        report.errors,
        [
            "bad-index-dash/index.md: index.md entries must be `* [Title](path) - description` (§8)",
            "bad-index-dash/index.md: index.md entries use `*`, not `-` (§8)",
            "bad-index-frontmatter/index.md: index.md may only carry frontmatter at the bundle root (§8)",
            "bad-index-noheading/index.md: index.md has no section heading (§8)",
            "bad-index-plain/index.md: index.md entries must be `* [Title](path) - description` (§8)",
            "concepts/duplicate-key.md: line 4: duplicate frontmatter key `type`",
            "concepts/empty-type.md: frontmatter has no non-empty `type` (§11.2)",
            "concepts/no-frontmatter.md: no YAML frontmatter (§11.1)",
            "concepts/no-type.md: frontmatter has no non-empty `type` (§11.2)",
            "concepts/orphan-list.md: line 2: list item before any key",
            "concepts/tab-indent.md: line 3: tab indentation is not valid YAML",
        ]
    );
    assert_eq!(report.checked, 22);
}

/// A clean document produces nothing, and an unterminated fence is tolerated.
///
/// The second half is a quirk rather than a design choice, and it is asserted
/// here so that changing it is a deliberate act with a failing test attached.
#[test]
fn a_clean_document_and_an_unterminated_fence_both_report_nothing() {
    let (root, config) = load("conformance");
    let report = check::check_bundle(&root, &config).unwrap_or_default();
    for finding in report.errors.iter().chain(report.warnings.iter()) {
        assert!(!finding.contains("clean.md"), "{finding}");
        assert!(!finding.contains("unterminated-fence.md"), "{finding}");
    }
}

#[test]
fn the_three_added_diagnostics_fire_and_only_where_they_should() {
    let (root, config) = load("guards");
    let report = check::check_bundle(&root, &config).unwrap_or_default();

    assert_eq!(
        report.errors,
        [
            "v01-citations.md: body `# Citations` was replaced by front-matter `sources` in v0.2 (§13)",
            "v01-timestamp.md: `timestamp` was replaced by `generated: {by, at}` in v0.2 (§13)",
        ]
    );
    assert_eq!(
        report.warnings,
        [
            "no-runtime.md: type `Attested Computation` has no `runtime` (required for this type, §10.2)"
        ]
    );
}

/// §11 is three rules and none of them mentions §10.2, so a missing `runtime`
/// must never make a bundle non-conformant.
#[test]
fn a_missing_runtime_warns_without_failing_conformance() {
    let (root, config) = load("guards");
    let report = check::check_bundle(&root, &config).unwrap_or_default();
    assert!(
        !report
            .errors
            .iter()
            .any(|e| e.contains("runtime") || e.contains("no-runtime.md"))
    );
}

#[test]
fn listings_group_by_month_suppress_drop_folders_and_run_deepest_first() {
    let root = scratch_copy("listing", "listing");
    let config = Config::load(&root).unwrap_or_default();
    let outcome = index::run(&root, &config, false).unwrap_or_default();
    assert_eq!(outcome.directories, 5);

    let meetings = std::fs::read_to_string(root.join("meetings/index.md")).unwrap_or_default();
    // Month grouping, with the folders inside one month in name order.
    assert!(meetings.contains("## 2026-01\n"), "{meetings}");
    assert!(meetings.contains("## 2026-02\n"), "{meetings}");
    // A folder whose name carries no date lands in its own group.
    assert!(meetings.contains("## undated\n"), "{meetings}");
    // Two summaries in one folder are both listed, `summary-flows` first
    // because `-` sorts before `.`.
    let flows = meetings.find("summary-flows.md");
    let plain = meetings.find("2026-01-20-beta/summary.md");
    assert!(flows < plain && flows.is_some(), "{meetings}");
    // A folder holding markdown but no summary falls back to a folder link.
    assert!(
        meetings.contains("* [2026-02-03-gamma](2026-02-03-gamma/)"),
        "{meetings}"
    );

    // A drop folder lists nothing, and its block collapses rather than
    // leaving a run of blank lines behind.
    let inbox = std::fs::read_to_string(root.join("inbox/index.md")).unwrap_or_default();
    assert!(
        inbox.contains(&format!("{}\n{}", index::BEGIN, index::END)),
        "{inbox}"
    );
    assert!(!inbox.contains("dropped.md"), "{inbox}");

    // Deepest-first: the child index was rewritten before the parent read its
    // description, so the parent shows the promoted lead rather than the
    // stale list item that sat above it.
    let deep = std::fs::read_to_string(root.join("deep/index.md")).unwrap_or_default();
    assert!(
        deep.contains("* [nested](nested/) - The real lead paragraph, which only a deepest-first run promotes."),
        "{deep}"
    );

    // The stale entry planted in the root block is gone, which is the whole
    // point of regenerating between markers.
    let index_md = std::fs::read_to_string(root.join("index.md")).unwrap_or_default();
    assert!(!index_md.contains("STALE ENTRY"), "{index_md}");
    // The configured title reached the root frontmatter, where the Python had
    // a client's name compiled in.
    assert!(
        index_md.contains("title: \"Listing fixture\""),
        "{index_md}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Generation is a pure function of the tree, so a second run changes nothing.
///
/// This is the property every migration batch's exit criterion leans on, and
/// asserting it here means a regression shows up in this crate rather than in
/// somebody's repository.
#[test]
fn a_second_index_run_writes_nothing() {
    let root = scratch_copy("listing", "idempotent");
    let config = Config::load(&root).unwrap_or_default();
    let _ = index::run(&root, &config, false);

    let second = index::run(&root, &config, false).unwrap_or_default();
    assert_eq!(second.written, 0);

    let checked = index::run(&root, &config, true).unwrap_or_default();
    assert!(checked.stale.is_empty(), "{:?}", checked.stale);

    let _ = std::fs::remove_dir_all(&root);
}

/// `--check` has to notice a hand-edited listing, or the gate is decorative.
#[test]
fn check_mode_catches_a_hand_edited_listing() {
    let root = scratch_copy("listing", "stale");
    let config = Config::load(&root).unwrap_or_default();
    let _ = index::run(&root, &config, false);

    let path = root.join("deep/index.md");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let tampered = text.replace("[nested](nested/)", "[renamed by hand](nested/)");
    let _ = std::fs::write(&path, tampered);

    let outcome = index::run(&root, &config, true).unwrap_or_default();
    assert_eq!(outcome.stale, ["deep/index.md"]);

    let _ = std::fs::remove_dir_all(&root);
}
