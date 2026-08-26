//! One directory holding a page and a section that publish at the same URL.
//!
//! §8 reserves `index.md` for a directory listing, and a site build acts on
//! that reservation: [`crate::assemble::rename_indexes`] renames every
//! `index.md` to `_index.md`, which is what stops Hugo reading the directory
//! as a *leaf bundle* and demoting every sibling from a page to a page
//! resource. The rename also commits the directory to publishing as a
//! **section** at its own URL, and that is the half this module is about. A
//! sibling `<name>.md` is a page that wants the same URL, and Hugo resolves
//! the contention by publishing one of them and reporting success.
//!
//! Measured on Hugo 0.165, one shape per site, reading the published tree back
//! off disk rather than reasoning about it:
//!
//! | source tree | published at `/plans/` | lost |
//! | --- | --- | --- |
//! | `plans.md` + `plans/_index.md` + `plans/child.md` | the section | `plans.md` |
//! | `Plans.md` + `plans/_index.md` | the section | `Plans.md` |
//! | `plans.md` + `plans/index.md` + `plans/child.md` | the leaf bundle | `plans.md` *and* `plans/child.md` |
//! | `plans.md` + `plans/child.md`, no listing | the page, child still at `/plans/child/` | nothing |
//! | `plans.md`, no `plans/` | the page | nothing |
//!
//! Three things follow, and none of them is what reading the source would have
//! suggested.
//!
//! **Which file survives is not a property of the shape.** The section won in
//! every row above, and on a fourth site where the two names differed only
//! after sanitisation — `Release Notes (Draft).md` beside
//! `Release Notes Draft/` — the page won and the section was dropped. The two
//! bundles in the estate that hit this each lost the other half.
//!
//! **Hugo says nothing.** Not at `--logLevel warn`, not with
//! `--printPathWarnings`, and the build exits 0. The one exception is the
//! sanitisation case, which does warn — so a build log is not evidence either
//! way.
//!
//! **The comparison is on [`url_segment`], not on the name.** `Plans.md`
//! beside `plans/` collides, and so does `Release Notes (Draft).md` beside
//! `Release Notes Draft/`. Comparing names would have missed both.
//!
//! A directory carrying no listing is deliberately not a section here. Row
//! four is the measurement: the page takes the URL, every child still
//! publishes beneath it, and no source file is lost. Only a directory holding
//! a listing claims the URL, which is why this is a rule about §8's reserved
//! name rather than about directories in general.

use std::path::{Path, PathBuf};

use crate::hugopath::url_segment;

/// A page and a listing that cannot both be published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The page, relative to the root that was scanned.
    pub page: String,
    /// The directory listing it contends with, relative to the same root.
    pub listing: String,
    /// The published path both want, without a leading or trailing slash.
    pub url: String,
}

/// The two names a directory listing can carry.
///
/// `index.md` is what §8 reserves and what a source bundle holds. `_index.md`
/// is what the assembly step renames it to, and a bundle that already carries
/// one — hand-written, or copied out of an assembled tree — collides in
/// exactly the same way. Both are listings for this purpose.
pub const LISTING_NAMES: [&str; 2] = ["index.md", "_index.md"];

/// Is this the name of a directory listing rather than of a page?
#[must_use]
pub fn is_listing(name: &str) -> bool {
    LISTING_NAMES.contains(&name)
}

/// Every page under `root` that contends with a sibling directory's listing.
///
/// Sorted by page path, which is the order the checker reports its other
/// findings in.
#[must_use]
pub fn find(root: &Path, skip: &[String]) -> Vec<Collision> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let children = crate::walk::children(&dir, skip);
        let mut sections: Vec<(String, PathBuf)> = Vec::new();
        for child in &children {
            if !child.is_dir() {
                continue;
            }
            stack.push(child.clone());
            let Some(segment) = segment_of(child) else {
                continue;
            };
            if let Some(listing) = LISTING_NAMES
                .into_iter()
                .map(|name| child.join(name))
                .find(|path| path.is_file())
            {
                sections.push((segment, listing));
            }
        }
        if sections.is_empty() {
            continue;
        }
        for child in &children {
            if child.is_dir() {
                continue;
            }
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if is_listing(name) || !crate::walk::is_markdown(name) {
                continue;
            }
            let Some(segment) = segment_of(child) else {
                continue;
            };
            let Some((_, listing)) = sections.iter().find(|(other, _)| *other == segment) else {
                continue;
            };
            let Ok(page) = child.strip_prefix(root) else {
                continue;
            };
            let Ok(shown) = listing.strip_prefix(root) else {
                continue;
            };
            found.push(Collision {
                page: crate::walk::to_posix(page),
                listing: crate::walk::to_posix(shown),
                url: published_path(listing.parent().unwrap_or(root), root),
            });
        }
    }
    found.sort_by(|a, b| a.page.cmp(&b.page));
    found
}

/// The URL segment this path claims, with `.md` taken off a file first.
///
/// `None` when the name sanitises away to nothing. Measured: a page named
/// `Ⅻ.md` publishes at its parent rather than at a segment of its own, so it
/// contends with no sibling directory and reporting it would be a false
/// finding.
fn segment_of(path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let segment = url_segment(stem);
    (!segment.is_empty()).then_some(segment)
}

/// Where a directory publishes, relative to the root that was scanned.
fn published_path(dir: &Path, root: &Path) -> String {
    let Ok(relative) = dir.strip_prefix(root) else {
        return String::new();
    };
    relative
        .components()
        .map(|part| url_segment(&part.as_os_str().to_string_lossy()))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<String>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(label: &str, files: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("okf-collision-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, "x\n");
        }
        root
    }

    #[test]
    fn a_page_beside_its_sibling_listing_is_reported_under_either_listing_name() {
        for listing in LISTING_NAMES {
            let root = tree(
                listing.trim_start_matches('_'),
                &[&format!("plans/{listing}"), "plans.md", "plans/child.md"],
            );
            let found = find(&root, &[]);
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].page, "plans.md");
            assert_eq!(found[0].listing, format!("plans/{listing}"));
            assert_eq!(found[0].url, "plans");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// The measured sanitisation cases, which comparing names would miss.
    #[test]
    fn two_names_that_sanitise_to_one_segment_collide() {
        let root = tree(
            "sanitised",
            &[
                "notes/index.md",
                "Notes.md",
                "Release Notes Draft/index.md",
                "Release Notes (Draft).md",
            ],
        );
        let pages: Vec<String> = find(&root, &[]).into_iter().map(|c| c.page).collect();
        assert_eq!(pages, ["Notes.md", "Release Notes (Draft).md"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The negative controls, both of them measured against Hugo as losing
    /// nothing. A gate that fired on either would be worse than no gate.
    #[test]
    fn a_directory_with_no_listing_and_a_page_with_no_directory_report_nothing() {
        let root = tree(
            "quiet",
            &[
                "index.md",
                "guides.md",
                "guides/deeper.md",
                "standalone.md",
                "notes/index.md",
                "notes/kept.md",
            ],
        );
        assert!(find(&root, &[]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_nested_collision_carries_the_whole_published_path() {
        let root = tree("nested", &["docs/plans/index.md", "docs/plans.md"]);
        let found = find(&root, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].url, "docs/plans");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_skipped_directory_is_outside_the_scan() {
        let root = tree("skipped", &["result/plans/index.md", "result/plans.md"]);
        assert!(find(&root, &["result".to_owned()]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
