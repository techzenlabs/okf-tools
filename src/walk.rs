//! Filesystem traversal shared by the checker and the indexer.
//!
//! Two rules decide what is inside a bundle, and they are not the same rule:
//!
//! * [`markdown_files`] and [`children`] skip dot-directories and the
//!   configured `skip_names`, because configuration is not knowledge.
//! * [`has_markdown`] does **not**. It answers "is there any markdown down
//!   there at all", and the original answers it over the raw tree.
//!
//! The difference is visible in output — a directory holding only hidden
//! markdown still gets an index, and that index lists nothing — so the two
//! stay distinct rather than being unified into one tidier helper.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Is this a markdown file?
///
/// Case-sensitive on purpose. The original compares a path suffix against
/// `".md"` exactly, so a file named `NOTES.MD` is not in the bundle, and
/// making the comparison case-insensitive here would silently add files to
/// every corpus this tool has already been run against.
#[must_use]
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "parity: the original compares the suffix exactly, and loosening \
              it would silently enlarge every corpus already run against"
)]
pub fn is_markdown(name: &str) -> bool {
    name.ends_with(".md")
}

/// Is this path component outside the bundle?
#[must_use]
pub fn is_skipped(component: &str, skip_names: &[String]) -> bool {
    component.starts_with('.') || skip_names.iter().any(|s| s == component)
}

/// Read a file, replacing invalid UTF-8 rather than failing.
///
/// A stray byte in one document must not take down a whole-bundle run, and
/// replacement is what the original does.
#[must_use]
pub fn read_lossy(path: &Path) -> String {
    std::fs::read(path).map_or_else(
        |_| String::new(),
        |bytes| String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// A path rendered with forward slashes, for display and comparison.
#[must_use]
pub fn to_posix(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every `.md` file in the bundle, sorted by path.
///
/// Sorted by the full relative path as bytes, which is the order the original
/// reports diagnostics in, and therefore the order a diff between the two is
/// taken in.
#[must_use]
pub fn markdown_files(root: &Path, skip_names: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_markdown(root, skip_names, &mut found);
    found.sort_by(|a, b| compare_paths(root, a, b));
    found
}

fn compare_paths(root: &Path, a: &Path, b: &Path) -> Ordering {
    let left = a.strip_prefix(root).map(to_posix).unwrap_or_default();
    let right = b.strip_prefix(root).map(to_posix).unwrap_or_default();
    left.as_bytes().cmp(right.as_bytes())
}

fn collect_markdown(dir: &Path, skip_names: &[String], found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_skipped(name, skip_names) {
            continue;
        }
        if path.is_dir() {
            collect_markdown(&path, skip_names, found);
        } else if is_markdown(name) {
            found.push(path);
        }
    }
}

/// The immediate children of `dir`, sorted by name, with skipped ones removed.
#[must_use]
pub fn children(dir: &Path, skip_names: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut kept: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !is_skipped(n, skip_names))
        })
        .collect();
    kept.sort_by(|a, b| {
        let left = a
            .file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .to_vec();
        let right = b
            .file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .to_vec();
        left.cmp(&right)
    });
    kept
}

/// Is there any markdown anywhere under `dir`?
///
/// Deliberately unfiltered: hidden directories count. See the module note.
#[must_use]
pub fn has_markdown(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if has_markdown(&path) {
                return true;
            }
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_markdown)
        {
            return true;
        }
    }
    false
}

/// Every file directly in `dir` whose name matches `pattern`, sorted.
#[must_use]
pub fn glob_children(dir: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut matched: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| crate::config::glob_match(pattern, n))
        })
        .collect();
    matched.sort();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_components_and_skip_names_are_outside_the_bundle() {
        let skip = vec!["node_modules".to_owned()];
        assert!(is_skipped(".git", &skip));
        assert!(is_skipped("node_modules", &skip));
        assert!(!is_skipped("docs", &skip));
    }

    #[test]
    fn posix_rendering_joins_with_forward_slashes() {
        assert_eq!(to_posix(Path::new("a/b/c.md")), "a/b/c.md");
    }

    /// Byte order, not locale order: `a-b` sorts before `a/b` because `-` is
    /// 0x2D and `/` is 0x2F, and the original's sort has the same property.
    #[test]
    fn paths_sort_by_bytes() {
        let root = Path::new("/r");
        let hyphen = PathBuf::from("/r/a-b/x.md");
        let slash = PathBuf::from("/r/a/b.md");
        assert_eq!(compare_paths(root, &hyphen, &slash), Ordering::Less);
    }
}
