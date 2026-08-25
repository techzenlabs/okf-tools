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
//!
//! `promotion/` and `confidentiality/` cover the gates that protect a
//! confidentiality boundary. Those are the ones that have to be watched
//! failing rather than assumed: a gate nobody has seen fail is a gate nobody
//! knows works, and the cost of finding out late is a client reading
//! somebody's read on them.

#![expect(
    clippy::unwrap_used,
    reason = "a panicking assertion is the point of a test"
)]

use std::path::{Path, PathBuf};

use okf_tools::promote::{self, Bundle, Kind, PromoteError, Revisions, Severity};
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

// ---------------------------------------------------------------------------
// The confidentiality gates.
// ---------------------------------------------------------------------------

/// A commit source that answers without a repository.
///
/// The fixtures are a tree rather than a checkout, and shelling out to `git`
/// from the middle of the drafting logic would have made them untestable — so
/// the revision lookup is a port, and this is the other implementation.
struct Fixed {
    rev: &'static str,
    /// What the source looked like at the recorded commit.
    then: Option<String>,
}

impl Fixed {
    fn at(rev: &'static str) -> Self {
        Self { rev, then: None }
    }
}

impl Revisions for Fixed {
    fn repo_name(&self, _root: &Path) -> Result<String, PromoteError> {
        Ok("example/promotion-source-fixture".to_owned())
    }

    fn last_rev(&self, _root: &Path, _relative: &str) -> Result<String, PromoteError> {
        Ok(self.rev.to_owned())
    }

    fn blob_at(&self, root: &Path, rev: &str, relative: &str) -> Result<String, PromoteError> {
        self.then.clone().ok_or_else(|| PromoteError::NoCommit {
            path: format!("{relative}@{rev}"),
            root: root.display().to_string(),
        })
    }
}

/// Both halves of a promotion, copied somewhere writable and side by side, so
/// the destination's `../source` still resolves.
fn promotion_scratch(label: &str) -> (Bundle, Bundle) {
    let root = scratch_copy("promotion", label);
    let source = Bundle::open(&root.join("source"), "source").unwrap();
    let destination = Bundle::open(&root.join("destination"), "fixture-knowledge").unwrap();
    (source, destination)
}

fn kinds(items: &[promote::Item]) -> Vec<&'static str> {
    items.iter().map(|item| item.kind.label()).collect()
}

/// The first of the three must-fail fixtures.
///
/// A link into a profile directory is the one disclosure a checker can see. It
/// must stop the write, and the advice attached to it must say what to put
/// there instead, because "resolve this" is not a resolution.
#[test]
fn a_page_pointing_into_a_profile_cannot_be_promoted() {
    let (source, destination) = promotion_scratch("promote-profile");
    let proposal = promote::propose(
        &source,
        "org/systems/widget-press.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("1111111111111111111111111111111111111111"),
    )
    .unwrap();

    assert!(proposal.blocked());
    assert_eq!(
        kinds(&proposal.items),
        [
            "evidence-link",
            "profile-link",
            "profile-link",
            "missing-owner"
        ]
    );
    let first = proposal
        .items
        .iter()
        .find(|item| item.kind == Kind::ProfileLink)
        .unwrap();
    assert_eq!(first.subject, "../people/dana-quill.md");
    assert!(first.sentence.contains("**Owner(s):**"), "{first:?}");
    assert!(first.replacement.contains("plain name"), "{first:?}");
    assert!(first.replacement.contains("`owner` record"), "{first:?}");

    // The refusal is the mechanism: nothing reached the bundle.
    assert!(!destination.root.join("systems/widget-press.md").exists());
    let err = promote::install(&destination, &proposal).unwrap_err();
    assert!(
        matches!(err, PromoteError::Blocked { count: 3, .. }),
        "{err}"
    );

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// The second must-fail fixture: a claim resting on a meeting summary.
///
/// The citation is not kept and marked unreachable. The path names a meeting,
/// its date and its subject, and the raw-markdown route emits that string
/// whatever a rendered page does with the link.
#[test]
fn a_page_citing_a_meeting_cannot_be_promoted() {
    let (source, destination) = promotion_scratch("promote-meeting");
    let proposal = promote::propose(
        &source,
        "org/systems/press-relay.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("2222222222222222222222222222222222222222"),
    )
    .unwrap();

    assert!(proposal.blocked());
    let cited = proposal
        .items
        .iter()
        .find(|item| item.kind == Kind::EvidenceLink)
        .unwrap();
    assert_eq!(
        cited.subject,
        "../../meetings/2026-03-04-relay-review/summary.md"
    );
    assert!(
        cited.replacement.contains("Needs-confirmation"),
        "{cited:?}"
    );
    assert!(cited.replacement.contains("citation dropped"), "{cited:?}");
    assert!(!destination.root.join("systems/press-relay.md").exists());

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// The positive control, without which the two refusals prove nothing.
#[test]
fn a_page_with_nothing_left_to_resolve_is_installed_with_its_provenance() {
    let (source, destination) = promotion_scratch("promote-clean");
    let rev = "3333333333333333333333333333333333333333";
    let proposal = promote::propose(
        &source,
        "org/systems/quiet-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at(rev),
    )
    .unwrap();

    assert!(!proposal.blocked(), "{:?}", kinds(&proposal.items));
    assert!(proposal.items.is_empty(), "{:?}", proposal.items);
    assert_eq!(proposal.destination_path, "systems/quiet-mill.md");
    assert_eq!(
        proposal.url,
        "https://docs.example.test/knowledge/systems/quiet-mill"
    );

    let written = promote::install(&destination, &proposal).unwrap();
    let page = std::fs::read_to_string(&written).unwrap_or_default();
    assert!(page.contains("promoted_from:"), "{page}");
    assert!(page.contains(&format!("  rev: \"{rev}\"")), "{page}");
    assert!(
        page.contains("  repo: \"example/promotion-source-fixture\""),
        "{page}"
    );
    assert!(
        page.contains("  path: \"org/systems/quiet-mill.md\""),
        "{page}"
    );

    // The source keeps everything and gains one key pointing outward.
    assert!(
        promote::write_source_pointer(&source, "org/systems/quiet-mill.md", &proposal.url).unwrap()
    );
    let note =
        std::fs::read_to_string(source.root.join("org/systems/quiet-mill.md")).unwrap_or_default();
    assert!(
        note.contains(&format!("promoted_to: \"{}\"", proposal.url)),
        "{note}"
    );
    assert!(note.contains("midday catch-up"), "{note}");
    // A second run is a no-op, so a revert is one `git checkout`.
    assert!(
        !promote::write_source_pointer(&source, "org/systems/quiet-mill.md", &proposal.url)
            .unwrap()
    );

    // And the bundle it landed in still passes its own gates.
    let config = Config::load(&destination.root).unwrap_or_default();
    let report = check::check_bundle(&destination.root, &config).unwrap_or_default();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// The routing rule decides, and `--to` disagreeing with it is a refusal.
///
/// A page that could become wrong because somebody changed code belongs beside
/// the code. Promoting it into the knowledge bundle would put the doc change
/// and the code change in two repositories and two reviews.
#[test]
fn a_destination_the_route_does_not_name_is_refused() {
    let (source, destination) = promotion_scratch("promote-route");
    let err = promote::propose(
        &source,
        "org/systems/repo-owned-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("4444444444444444444444444444444444444444"),
    )
    .unwrap_err();
    assert!(
        matches!(&err, PromoteError::WrongDestination { routed, .. } if routed == "fixture-code-repo"),
        "{err}"
    );

    let err = promote::propose(
        &source,
        "meetings/2026-03-04-relay-review/summary.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("4444444444444444444444444444444444444444"),
    )
    .unwrap_err();
    assert!(matches!(err, PromoteError::NoRoute { .. }), "{err}");

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// A promoted page may only be installed where the gate protecting it runs.
///
/// The three rules are off by default, which is right — §11 forbids a consumer
/// rejecting a bundle over an unknown key — and this is what keeps "off by
/// default" from meaning "forgotten".
#[test]
fn a_destination_that_has_not_turned_the_gates_on_refuses_the_page() {
    let (source, destination) = promotion_scratch("promote-gate-off");
    let config_path = destination.root.join("okf.toml");
    let text = std::fs::read_to_string(&config_path).unwrap_or_default();
    let without = text.replace("owner_record = true", "owner_record = false");
    let _ = std::fs::write(&config_path, without);
    let destination = Bundle::open(&destination.root, "fixture-knowledge").unwrap();

    let err = promote::propose(
        &source,
        "org/systems/quiet-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("5555555555555555555555555555555555555555"),
    )
    .unwrap_err();
    assert!(
        matches!(&err, PromoteError::GateOff { missing, .. } if missing == "owner_record"),
        "{err}"
    );

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// Refreshing reports what the source grew, and does not overwrite the page.
///
/// A promoted page is not a mechanical copy: its meeting-backed claims were
/// restated by hand. Redrawing over it would delete exactly the work the gate
/// exists to require, so the redraw is offered for reading and the only thing
/// written is the commit.
#[test]
fn refresh_reports_only_what_the_source_has_grown_since() {
    let (source, destination) = promotion_scratch("promote-refresh");
    let before = std::fs::read_to_string(source.root.join("org/systems/quiet-mill.md")).unwrap();
    let proposal = promote::propose(
        &source,
        "org/systems/quiet-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("6666666666666666666666666666666666666666"),
    )
    .unwrap();
    let installed = promote::install(&destination, &proposal).unwrap();
    let promoted = std::fs::read_to_string(&installed).unwrap_or_default();

    // The source grows a pointer into a profile after promotion.
    let grown =
        format!("{before}\nSigned off by [Dana Quill](../people/dana-quill.md) each week.\n");
    let _ = std::fs::write(source.root.join("org/systems/quiet-mill.md"), &grown);

    let found = promote::refresh(
        &destination,
        "systems/quiet-mill.md",
        &Fixed {
            rev: "7777777777777777777777777777777777777777",
            then: Some(before.clone()),
        },
    )
    .unwrap();

    assert!(found.moved());
    assert!(found.blocked());
    assert_eq!(kinds(&found.new_items), ["profile-link"]);
    assert_eq!(found.new_items[0].subject, "../people/dana-quill.md");
    // The redraw is available to read; the page itself is untouched.
    assert!(found.redraft.contains("dana-quill.md"));
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap_or_default(),
        promoted
    );

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// A source that moved without growing anything gets its commit recorded.
#[test]
fn refresh_records_the_new_commit_when_nothing_new_is_unresolved() {
    let (source, destination) = promotion_scratch("promote-refresh-clean");
    let before = std::fs::read_to_string(source.root.join("org/systems/quiet-mill.md")).unwrap();
    let proposal = promote::propose(
        &source,
        "org/systems/quiet-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at("8888888888888888888888888888888888888888"),
    )
    .unwrap();
    let installed = promote::install(&destination, &proposal).unwrap();

    let edited = format!("{before}\nAssumed, 2026-03-05: the catch-up window is thirty minutes.\n");
    let _ = std::fs::write(source.root.join("org/systems/quiet-mill.md"), &edited);

    let next = "9999999999999999999999999999999999999999";
    let found = promote::refresh(
        &destination,
        "systems/quiet-mill.md",
        &Fixed {
            rev: next,
            then: Some(before),
        },
    )
    .unwrap();
    assert!(found.moved());
    assert!(!found.blocked(), "{:?}", kinds(&found.new_items));

    promote::bump_rev(&destination, "systems/quiet-mill.md", next).unwrap();
    let page = std::fs::read_to_string(&installed).unwrap_or_default();
    assert!(page.contains(&format!("  rev: \"{next}\"")), "{page}");
    assert!(!page.contains("8888888888"), "{page}");

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// Drift between a private note and its promoted copy is the failure mode, so
/// it is a listed divergence rather than a silent one.
#[test]
fn drift_lists_the_pages_whose_source_has_moved() {
    let (source, destination) = promotion_scratch("promote-drift");
    let recorded = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let proposal = promote::propose(
        &source,
        "org/systems/quiet-mill.md",
        "fixture-knowledge",
        &destination,
        None,
        &Fixed::at(recorded),
    )
    .unwrap();
    let _ = promote::install(&destination, &proposal).unwrap();

    let still = promote::drift(&destination, &Fixed::at(recorded)).unwrap();
    assert!(still.is_empty(), "{still:?}");

    let moved = promote::drift(
        &destination,
        &Fixed::at("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    )
    .unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].page, "systems/quiet-mill.md");
    assert_eq!(moved[0].provenance.rev, recorded);
    assert_eq!(moved[0].provenance.path, "org/systems/quiet-mill.md");

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// The third must-fail fixture, on the other surface.
///
/// This is the backstop for the case the promotion tool cannot see, where a
/// page is hand-copied and a link is missed. It has to fail in the tenant's own
/// repository rather than at review time, so it is `okf-check` rather than
/// `okf-promote`, and every one of these is an error rather than a warning.
#[test]
fn a_bundle_that_closes_its_boundary_fails_on_every_way_out_of_it() {
    let (root, config) = load("confidentiality");
    let report = check::check_bundle(&root, &config).unwrap_or_default();

    assert_eq!(
        report.errors,
        [
            "systems/meeting-citation.md: line 6: sources entry `meetings/2026-03-04-relay-review/summary.md` has no target in this bundle",
            "systems/meeting-citation.md: line 16: link `../meetings/2026-03-04-relay-review/summary.md` has no target in this bundle",
            "systems/owner-notes.md: line 8: owner subkey `notes` is not name, title or email",
            "systems/people-link.md: line 12: link `../people/dana-quill.md` has no target in this bundle",
            "systems/people-link.md: line 15: link `../../outside/thing.md` leaves the bundle",
            "systems/person-page.md: type `Person` is not in the bundle vocabulary (closed by this bundle)",
            "systems/vendor-url.md: line 13: link `https://vendor.example.test/quiet-mill-datasheet` is not a URL this bundle may carry",
        ]
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

/// Containment on its own would let the copied page through.
///
/// `../people/dana-quill.md` written on `systems/press-relay.md` resolves to
/// `people/dana-quill.md`, which is *inside* the destination bundle root and
/// simply is not there. Requiring the target to exist is what catches it, and
/// this test exists because the containment-only version of the rule passed.
#[test]
fn a_copied_link_that_stays_inside_the_root_still_fails() {
    let (root, config) = load("confidentiality");
    let report = check::check_bundle(&root, &config).unwrap_or_default();
    let inside = report
        .errors
        .iter()
        .find(|e| e.contains("people-link.md: line 12"))
        .unwrap();
    assert!(inside.contains("has no target in this bundle"), "{inside}");
    assert!(!inside.contains("leaves the bundle"), "{inside}");
}

/// Every rule is off unless a bundle asks for it, and the reason is §11: a
/// consumer that rejected a bundle over an unknown key would be the
/// non-conformant one.
#[test]
fn none_of_the_three_rules_fires_in_a_bundle_that_has_not_asked() {
    let root = scratch_copy("confidentiality", "conf-default");
    let config_path = root.join("okf.toml");
    let text = std::fs::read_to_string(&config_path).unwrap_or_default();
    let (head, _) = text.split_once("[confidentiality]").unwrap_or((&text, ""));
    let _ = std::fs::write(&config_path, head);

    let config = Config::load(&root).unwrap_or_default();
    let report = check::check_bundle(&root, &config).unwrap_or_default();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        report.warnings,
        ["systems/person-page.md: type `Person` is not in the bundle vocabulary"]
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A note does not block the write, and an unresolved item does.
#[test]
fn a_missing_owner_is_a_note_rather_than_a_refusal() {
    let (source, destination) = promotion_scratch("promote-owner-note");
    let draft = "---\ntype: \"System\"\ntitle: \"Press Relay\"\n\
                 description: \"Restated, with nothing left pointing out.\"\n---\n\n\
                 # Press Relay\n\n\
                 Confirmed, 2026-03-04: the relay feeds the mill on a nightly cycle.\n";
    let proposal = promote::propose(
        &source,
        "org/systems/press-relay.md",
        "fixture-knowledge",
        &destination,
        Some(draft),
        &Fixed::at("cccccccccccccccccccccccccccccccccccccccc"),
    )
    .unwrap();

    assert_eq!(kinds(&proposal.items), ["missing-owner"]);
    assert_eq!(proposal.items[0].severity(), Severity::Note);
    assert!(!proposal.blocked());
    let written = promote::install(&destination, &proposal).unwrap();
    assert!(written.exists());

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}

/// The owner schema runs at the promotion boundary as well as in the bundle.
///
/// `--draft` is the path a reviewer takes, and a reviewer constructing an owner
/// record by hand is exactly when a fourth subkey appears. Catching it here
/// beats catching it in the destination repository's build afterwards.
#[test]
fn a_hand_restated_draft_that_grows_an_owner_subkey_is_refused() {
    let (source, destination) = promotion_scratch("promote-owner-subkey");
    let draft = "---\ntype: \"System\"\ntitle: \"Press Relay\"\n\
                 description: \"Restated, with an owner record that grew.\"\n\
                 owner:\n  - name: \"Dana Quill\"\n    title: \"Director of Operations\"\n\
                 \x20   notes: \"Questions the return on the platform.\"\n---\n\n\
                 # Press Relay\n\n\
                 Confirmed, 2026-03-04: the relay feeds the mill on a nightly cycle.\n";
    let proposal = promote::propose(
        &source,
        "org/systems/press-relay.md",
        "fixture-knowledge",
        &destination,
        Some(draft),
        &Fixed::at("dddddddddddddddddddddddddddddddddddddddd"),
    )
    .unwrap();

    assert_eq!(kinds(&proposal.items), ["owner-record"]);
    assert!(proposal.blocked());
    assert!(
        proposal.items[0].subject.contains("owner subkey `notes`"),
        "{:?}",
        proposal.items[0]
    );
    assert!(!destination.root.join("systems/press-relay.md").exists());

    let _ = std::fs::remove_dir_all(destination.root.parent().unwrap_or(&destination.root));
}
