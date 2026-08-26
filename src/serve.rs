//! `okf-serve`: the process that refuses, which is the boundary.
//!
//! **Behind a proxy is not a posture. Refusing to serve without proxy-injected
//! identity is.** A site that merely sits behind Entra's Easy Auth is
//! bypassable the moment its origin is reachable, and an origin is reachable
//! more often than anyone plans: a private endpoint misconfigured, a slot
//! swapped, a `$web` container switched on by somebody adding a storage
//! account. `ria-one` does not rely on the proxy being in front — a release
//! build of it refuses to start in any auth mode but `proxy` — and this copies
//! that stance rather than its resources.
//!
//! So `--require-header X-MS-CLIENT-PRINCIPAL` makes an unauthenticated
//! request fail because *this process* refuses it. The access-matrix probe
//! then tests a property of the build rather than a property of a setting.
//!
//! The three tailnet tenants run the same binary with no flag, because there
//! the bind address is the boundary and there is no proxy to inject anything.
//! One binary, two deployments, and the difference is a flag somebody can read
//! in a unit file.
//!
//! # What this is not
//!
//! It is not an internet-facing web server. It serves a directory of files
//! that a build produced, on a tailnet or behind an authenticating proxy, and
//! its whole security argument is the refusal above plus the two below:
//!
//! * **The header check runs before anything touches the filesystem.** An
//!   unauthenticated request never resolves a path, so a traversal bug could
//!   not be reached without an identity anyway.
//! * **A directory is never a listing.** A directory resolves to its
//!   `index.html` or to 404. `okf-assemble` writes `_index.md` for every
//!   section, so a directory with no `index.html` is a build that went wrong,
//!   and showing a reader the file names is not the way to say so.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Files served with a type this table names; everything else is a download.
///
/// Kept to what an OKF site actually publishes, which is Hugo's output plus
/// Pagefind's index and the mermaid bundle. An unlisted extension still
/// serves — as `application/octet-stream`, which a browser downloads rather
/// than renders, and rendering an unknown type is the mistake worth avoiding.
const CONTENT_TYPES: &[(&str, &str)] = &[
    ("css", "text/css; charset=utf-8"),
    ("gif", "image/gif"),
    ("htm", "text/html; charset=utf-8"),
    ("html", "text/html; charset=utf-8"),
    ("ico", "image/vnd.microsoft.icon"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript; charset=utf-8"),
    ("json", "application/json; charset=utf-8"),
    ("map", "application/json; charset=utf-8"),
    ("md", "text/markdown; charset=utf-8"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
    ("txt", "text/plain; charset=utf-8"),
    ("wasm", "application/wasm"),
    ("webp", "image/webp"),
    ("woff2", "font/woff2"),
    ("xml", "application/xml; charset=utf-8"),
    ("zst", "application/zstd"),
];

/// What a request path resolves to under the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A readable file, inside the root.
    File(PathBuf),
    /// Nothing is there. The caller serves `404.html` if the root has one.
    Missing,
    /// The path tried to leave the root, or carried something a path may not.
    Refused,
}

/// The MIME type for a path's extension.
#[must_use]
pub fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    CONTENT_TYPES
        .iter()
        .find(|(name, _)| *name == extension)
        .map_or("application/octet-stream", |(_, kind)| *kind)
}

/// Percent-decoding, refusing the two encodings that are never legitimate.
///
/// A `%00` truncates a path in anything written in C further down, and a
/// `%2f` is a slash somebody did not want the path splitter to see. Neither
/// appears in a URL a Hugo site emits, so refusing beats decoding.
fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = raw.get(index + 1..index + 3)?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == 0 || byte == b'/' || byte == b'\\' {
                return None;
            }
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Where a request path lands under `root`.
///
/// Containment is asserted twice and the second one is the load-bearing half.
/// Rejecting `..` textually stops the obvious attempt; canonicalising and
/// re-checking the prefix stops a symlink inside the tree pointing out of it,
/// which a build that copied a store path can produce without anybody meaning
/// it.
#[must_use]
pub fn resolve(root: &Path, request_path: &str) -> Resolution {
    let path = request_path.split(['?', '#']).next().unwrap_or_default();
    let Some(decoded) = decode(path) else {
        return Resolution::Refused;
    };
    if !decoded.starts_with('/') {
        return Resolution::Refused;
    }

    let mut relative = PathBuf::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Resolution::Refused,
            other => {
                if other.contains('\0') || other.contains('\\') {
                    return Resolution::Refused;
                }
                relative.push(other);
            }
        }
    }
    // A component the splitter did not produce is a component nobody asked
    // for. Belt and braces against a platform reading `C:` or `\\?\` as one.
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Resolution::Refused;
    }

    let candidate = root.join(&relative);
    let target = if candidate.is_dir() {
        candidate.join("index.html")
    } else {
        candidate
    };
    let (Ok(real_root), Ok(real_target)) = (root.canonicalize(), target.canonicalize()) else {
        return Resolution::Missing;
    };
    if !real_target.starts_with(&real_root) {
        return Resolution::Refused;
    }
    if real_target.is_file() {
        Resolution::File(real_target)
    } else {
        Resolution::Missing
    }
}

/// Is the identity header this deployment requires present and non-empty?
///
/// Case-insensitive, because HTTP header names are, and a proxy that sends
/// `X-Ms-Client-Principal` is sending the same header. An empty value is
/// absent: a proxy that strips the value but leaves the name behind has not
/// authenticated anybody.
#[must_use]
pub fn identified(headers: &BTreeMap<String, String>, required: Option<&str>) -> bool {
    let Some(required) = required else {
        return true;
    };
    let wanted = required.to_ascii_lowercase();
    headers
        .iter()
        .any(|(name, value)| name.to_ascii_lowercase() == wanted && !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("okf-serve-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("section")).ok();
        std::fs::write(root.join("index.html"), "home").ok();
        std::fs::write(root.join("404.html"), "gone").ok();
        std::fs::write(root.join("section/index.html"), "section").ok();
        std::fs::write(root.join("a file.txt"), "spaces").ok();
        root
    }

    #[test]
    fn a_directory_resolves_to_its_index_and_never_to_a_listing() {
        let root = scratch("dir");
        assert_eq!(
            resolve(&root, "/section/"),
            Resolution::File(
                root.join("section/index.html")
                    .canonicalize()
                    .unwrap_or_default()
            )
        );
        std::fs::create_dir_all(root.join("bare")).ok();
        std::fs::write(root.join("bare/page.html"), "x").ok();
        assert_eq!(resolve(&root, "/bare/"), Resolution::Missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_root_path_is_the_root_index() {
        let root = scratch("root");
        assert!(matches!(resolve(&root, "/"), Resolution::File(_)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every shape of "leave the root" this server will ever be shown, and one
    /// it would only be shown by a build that went wrong.
    #[test]
    fn nothing_reaches_outside_the_root() {
        let root = scratch("escape");
        for path in [
            "/../etc/passwd",
            "/section/../../etc/passwd",
            "/%2e%2e/etc/passwd",
            "/%2fetc/passwd",
            "/a%00.html",
            "relative",
            "/section\\..\\..",
        ] {
            assert_eq!(resolve(&root, path), Resolution::Refused, "{path}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A symlink inside the tree pointing out of it. Textual `..` rejection
    /// does not see this one, which is why containment is asserted after
    /// canonicalising as well as before.
    #[test]
    fn a_symlink_out_of_the_tree_is_refused() {
        let root = scratch("symlink");
        let outside = std::env::temp_dir().join("okf-serve-outside.txt");
        std::fs::write(&outside, "not yours").ok();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("leak.txt")).ok();
        #[cfg(unix)]
        assert_eq!(resolve(&root, "/leak.txt"), Resolution::Refused);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn a_percent_encoded_space_resolves_and_a_query_string_is_not_part_of_the_path() {
        let root = scratch("query");
        assert!(matches!(
            resolve(&root, "/a%20file.txt?v=2"),
            Resolution::File(_)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_is_missing_rather_than_refused() {
        let root = scratch("missing");
        assert_eq!(resolve(&root, "/nowhere.html"), Resolution::Missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn content_types_cover_what_a_site_publishes() {
        assert_eq!(
            content_type(Path::new("a/b.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("a/b.HTML")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("pagefind/x.pf_fragment")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type(Path::new("noextension")),
            "application/octet-stream"
        );
    }

    #[test]
    fn the_identity_header_is_case_insensitive_and_an_empty_value_is_absent() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Ms-Client-Principal".to_owned(), "abc".to_owned());
        assert!(identified(&headers, Some("X-MS-CLIENT-PRINCIPAL")));

        headers.insert("X-Ms-Client-Principal".to_owned(), "   ".to_owned());
        assert!(!identified(&headers, Some("X-MS-CLIENT-PRINCIPAL")));

        assert!(!identified(&BTreeMap::new(), Some("X-MS-CLIENT-PRINCIPAL")));
        // No flag, no requirement: this is the tailnet deployment.
        assert!(identified(&BTreeMap::new(), None));
    }
}
