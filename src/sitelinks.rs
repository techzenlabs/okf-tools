//! Link rewriting, done once at assembly rather than by a render hook.
//!
//! Doing it here rather than in a template keeps the byte-identical raw
//! markdown honest: the bytes served at `/x/index.md` are the bytes in
//! `content/x.md`, and `cmp` over every pair is a build gate. A render hook
//! would rewrite the HTML and leave the markdown pointing somewhere else, so
//! the two surfaces would disagree about where a link goes.
//!
//! Three rewrites, and each one has a measured reason.
//!
//! **Site-absolute links.** OKF §6 recommends bundle-relative links with a
//! leading `/`. Under a mount they would resolve at the site root instead, so
//! `](/x)` becomes `](/<id>/x)`. A mount holding a vendored upstream sets
//! `site_absolute_base` instead, and its `](/x)` resolves to the upstream's
//! own live page, which is what a reader of a mirrored document wants.
//!
//! **Relative markdown links.** Hugo publishes every page as a directory, so
//! `](sibling.md)` from `/b/page/` would resolve to `/b/page/sibling/` rather
//! than to `/b/sibling/`. Relative links are therefore resolved against the
//! document's own directory and written as site paths. Authored content in
//! this estate uses thousands of them and zero of the leading-slash form, so
//! this is the rewrite that decides whether the site navigates at all.
//!
//! **Relative asset links.** Same arithmetic, same fix, and only for a file
//! that is actually in the assembled tree — a link to something the allowlist
//! did not copy is left exactly as written, because §11 says a broken link is
//! a link rather than a build failure.

use std::path::{Path, PathBuf};

use crate::assemble::AssembleError;

/// Rewrite every markdown file under one mount. Returns the number changed.
///
/// `mounted` names the mounted-references tree for this bundle — its subdir in
/// the source repository, and where [`crate::refassets`] copied the escaping
/// targets — or `None` for a bundle mounted at its repository root, which has
/// nothing outside itself.
///
/// # Errors
///
/// Fails when a file under `mount` cannot be read or written.
pub fn rewrite_tree(
    mount: &Path,
    id: &str,
    site_absolute_base: &str,
    mounted: Option<(&str, &Path)>,
) -> Result<usize, AssembleError> {
    let mut changed = 0usize;
    let mut stack = vec![mount.to_path_buf()];
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| AssembleError::Io {
            context: format!("reading {}", dir.display()),
            source,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(crate::walk::is_markdown)
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|source| AssembleError::Io {
            context: format!("reading {}", file.display()),
            source,
        })?;
        let dir = file.parent().unwrap_or(mount);
        let mut context = Context::new(mount, dir, id, site_absolute_base);
        if let Some((subdir, files_dir)) = mounted {
            context = context.with_files(subdir, files_dir);
        }
        let rewritten = rewrite_text(&text, &context);
        if rewritten != text {
            std::fs::write(file, rewritten).map_err(|source| AssembleError::Io {
                context: format!("writing {}", file.display()),
                source,
            })?;
            changed = changed.saturating_add(1);
        }
    }
    Ok(changed)
}

/// Everything a rewrite needs to know about where the document sits.
pub struct Context {
    /// Site path of the document's own directory, such as `/lane-cast/adr`.
    here: String,
    /// The mount's site path, such as `/lane-cast`.
    mount_path: String,
    /// The mount's directory on disk, for existence tests.
    mount_dir: PathBuf,
    /// Upstream base for a mirrored mount, or empty.
    site_absolute_base: String,
    /// The mounted-references tree, for links that leave the subdir.
    files: Option<FilesMount>,
}

/// Where this bundle's out-of-subdir references were mounted.
struct FilesMount {
    /// The bundle's subdir in its source repository, split into segments.
    subdir: Vec<String>,
    /// The copied tree on disk, `static/_files/<id>`, for existence tests.
    dir: PathBuf,
}

impl Context {
    /// # Panics
    ///
    /// Never: a directory under the mount always strips the mount prefix.
    #[must_use]
    pub fn new(mount: &Path, dir: &Path, id: &str, site_absolute_base: &str) -> Self {
        let relative = dir
            .strip_prefix(mount)
            .map(crate::walk::to_posix)
            .unwrap_or_default();
        let mount_path = format!("/{id}");
        let here = if relative.is_empty() {
            mount_path.clone()
        } else {
            format!("{mount_path}/{relative}")
        };
        Self {
            here,
            mount_path,
            mount_dir: mount.to_path_buf(),
            site_absolute_base: site_absolute_base.trim_end_matches('/').to_owned(),
            files: None,
        }
    }

    /// Name the mounted-references tree, enabling the `/_files/` rewrite.
    #[must_use]
    pub fn with_files(mut self, subdir: &str, dir: &Path) -> Self {
        self.files = Some(FilesMount {
            subdir: subdir
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .map(str::to_owned)
                .collect(),
            dir: dir.to_path_buf(),
        });
        self
    }
}

/// The path component of every inline link target in one document, with any
/// fragment and any title stripped — the same walk [`rewrite_text`] makes,
/// shared so [`crate::refassets`] cannot disagree with it about what a link
/// is.
pub(crate) fn link_paths(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let (_, tail) = rest.split_at(at.saturating_add(2));
        let Some(close) = find_target_end(tail) else {
            rest = tail;
            continue;
        };
        let (target, after) = tail.split_at(close);
        let url = target
            .split_once(char::is_whitespace)
            .map_or(target, |(url, _)| url);
        let path = url.split_once('#').map_or(url, |(path, _)| path);
        if !path.is_empty() {
            out.push(path);
        }
        rest = after;
    }
    out
}

/// Rewrite every markdown link target in one document.
#[must_use]
pub fn rewrite_text(text: &str, context: &Context) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let (head, tail) = rest.split_at(at.saturating_add(2));
        out.push_str(head);
        let Some(close) = find_target_end(tail) else {
            rest = tail;
            continue;
        };
        let (target, after) = tail.split_at(close);
        out.push_str(&rewrite_target(target, context));
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Where the link target ends, allowing one level of nested parentheses.
fn find_target_end(tail: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in tail.char_indices() {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' if depth == 0 => return Some(offset),
            ')' => depth = depth.saturating_sub(1),
            '\n' => return None,
            _ => {}
        }
    }
    None
}

fn rewrite_target(target: &str, context: &Context) -> String {
    // A title after the URL — `](path "Title")` — is left alone.
    let (url, title) = match target.find(char::is_whitespace) {
        Some(at) => target.split_at(at),
        None => (target, ""),
    };
    let (path, fragment) = url.split_once('#').map_or((url, ""), |(p, f)| (p, f));
    let rewritten = rewrite_path(path, context);
    let mut out = rewritten;
    if !fragment.is_empty() || url.ends_with('#') {
        out.push('#');
        out.push_str(fragment);
    }
    out.push_str(title);
    out
}

fn rewrite_path(path: &str, context: &Context) -> String {
    if path.is_empty() || path.starts_with("//") || path.contains(':') {
        // Empty, protocol-relative, or carrying a scheme. Not ours.
        return path.to_owned();
    }
    let mount_id = context.mount_path.trim_start_matches('/');
    if let Some(rest) = path.strip_prefix('/') {
        if !context.site_absolute_base.is_empty() {
            // A mirrored page's site-absolute link belongs to the upstream it
            // was copied from, and stays exactly as the upstream wrote it.
            return format!("{}/{rest}", context.site_absolute_base);
        }
        return resolve(vec![mount_id], rest, context)
            .unwrap_or_else(|| format!("{}/{rest}", context.mount_path));
    }
    let here: Vec<&str> = context
        .here
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    resolve(here, path, context)
        .or_else(|| resolve_files(path, context))
        .unwrap_or_else(|| path.to_owned())
}

/// Render a link that left its subdir as the `/_files/` path it was mounted
/// at — and only when the file is really there.
///
/// The arithmetic is [`crate::refassets::escaped_target`], the same call the
/// mounting pass made, so the two cannot drift; the existence test is against
/// the tree that pass copied, so anything it declined — a dead link, a
/// directory, an extension the tenant did not allowlist — stays exactly as
/// written. No segment is sanitised: `static/` is byte-copied, and the URL is
/// the path.
fn resolve_files(path: &str, context: &Context) -> Option<String> {
    let files = context.files.as_ref()?;
    let doc_dir: Vec<String> = context
        .here
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .skip(1)
        .map(str::to_owned)
        .collect();
    let segments = crate::refassets::escaped_target(&files.subdir, &doc_dir, path)?;
    let name = segments.last()?;
    if crate::walk::is_markdown(name) {
        return None;
    }
    let on_disk = files.dir.join(segments.iter().collect::<PathBuf>());
    if !on_disk.is_file() {
        return None;
    }
    let mount_id = context.mount_path.trim_start_matches('/');
    Some(format!("/_files/{mount_id}/{}", segments.join("/")))
}

/// Resolve `path` against `base` and render it as a site path.
///
/// `None` means "leave it exactly as it was written", which is the answer for
/// anything that climbed out of its own mount or that names a file the
/// allowlist did not copy. §11 makes a broken link a link.
fn resolve(base: Vec<&str>, path: &str, context: &Context) -> Option<String> {
    let mut segments = base;
    let ends_in_slash = path.ends_with('/');
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    // A target that climbed out of its own mount is not this mount's to
    // rewrite, and rewriting it would silently point at another tenant's page.
    if segments.first() != Some(&context.mount_path.trim_start_matches('/')) {
        return None;
    }
    let name = *segments.last()?;
    // A URL is not the path the author typed. Hugo lowercases each directory
    // segment and drops what it will not carry, so a link written
    // `Documentation/README.md` has to come out as `/lane-cast/documentation/
    // readme/` or it is a link to a page that was never published. See
    // [`crate::hugopath`] for the transform and how it was measured.
    let mut published: Vec<String> = segments
        .iter()
        .map(|segment| crate::hugopath::url_segment(segment))
        .collect();
    if crate::walk::is_markdown(name) {
        let stem = name.strip_suffix(".md")?;
        // `index.md` is the directory's own listing, so it *is* the directory.
        if stem == "index" || stem == "_index" {
            published.pop()?;
        } else {
            *published.last_mut()? = crate::hugopath::url_segment(stem);
        }
        return Some(format!("/{}/", published.join("/")));
    }
    // Everything else has to exist in the assembled tree before it is touched.
    let on_disk = context.mount_dir.join(segments.get(1..)?.join("/"));
    if on_disk.is_dir() {
        return Some(format!("/{}/", published.join("/")));
    }
    if ends_in_slash || !on_disk.is_file() {
        return None;
    }
    // A page resource keeps its own filename. Hugo sanitises the directories
    // above it and copies the leaf across exactly as written, so an asset link
    // is the one place the last segment must not be transformed.
    name.clone_into(published.last_mut()?);
    Some(format!("/{}", published.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(dir: &str) -> Context {
        let mount = PathBuf::from("/nonexistent/content/lane-cast");
        let here = if dir.is_empty() {
            mount.clone()
        } else {
            mount.join(dir)
        };
        Context::new(&mount, &here, "lane-cast", "")
    }

    /// OKF §6's recommended form: bundle-relative with a leading slash. Under
    /// a mount it would otherwise resolve at the site root, and it points at a
    /// document, so it lands on that document's rendered URL.
    #[test]
    fn a_bundle_relative_link_resolves_under_the_mount() {
        assert_eq!(
            rewrite_text("see [x](/adr/0001.md)", &context("")),
            "see [x](/lane-cast/adr/0001/)"
        );
        assert_eq!(
            rewrite_text("see [x](/adr/index.md)", &context("")),
            "see [x](/lane-cast/adr/)"
        );
    }

    #[test]
    fn a_mirror_mount_sends_site_absolute_links_upstream() {
        let mount = PathBuf::from("/nonexistent/content/mirror");
        let context = Context::new(&mount, &mount, "mirror", "https://example.invalid/docs/");
        let out = rewrite_text("[p](/a/b)", &context);
        assert_eq!(out, "[p](https://example.invalid/docs/a/b)");
    }

    /// Hugo publishes every page as a directory, so a sibling link that is
    /// left relative resolves one level too deep. This is the rewrite the
    /// corpus needs thousands of times and the leading-slash form zero.
    #[test]
    fn a_relative_markdown_link_becomes_a_site_path() {
        assert_eq!(
            rewrite_text("[a](sibling.md)", &context("adr")),
            "[a](/lane-cast/adr/sibling/)"
        );
        assert_eq!(
            rewrite_text("[a](../spikes/0002.md)", &context("adr")),
            "[a](/lane-cast/spikes/0002/)"
        );
        assert_eq!(
            rewrite_text("[a](sub/index.md)", &context("adr")),
            "[a](/lane-cast/adr/sub/)"
        );
    }

    #[test]
    fn a_fragment_and_a_link_title_both_survive() {
        assert_eq!(
            rewrite_text(r#"[a](other.md#heading "T")"#, &context("adr")),
            r#"[a](/lane-cast/adr/other/#heading "T")"#
        );
    }

    #[test]
    fn an_external_link_and_a_bare_fragment_are_left_alone() {
        let context = context("adr");
        assert_eq!(
            rewrite_text("[a](https://example.invalid/x)", &context),
            "[a](https://example.invalid/x)"
        );
        assert_eq!(
            rewrite_text("[a](mailto:x@example.invalid)", &context),
            "[a](mailto:x@example.invalid)"
        );
        assert_eq!(rewrite_text("[a](#section)", &context), "[a](#section)");
    }

    /// A link that climbs out of its own mount would otherwise be rewritten to
    /// point at whatever else is mounted beside it.
    #[test]
    fn a_link_escaping_the_mount_is_left_exactly_as_written() {
        assert_eq!(
            rewrite_text("[a](../../elsewhere/x.md)", &context("adr")),
            "[a](../../elsewhere/x.md)"
        );
    }

    /// A relative link to a file the allowlist did not copy stays broken
    /// rather than being invented, because §11 makes a broken link a link.
    #[test]
    fn a_link_to_an_uncopied_asset_is_not_invented() {
        assert_eq!(
            rewrite_text("[a](diagram.png)", &context("adr")),
            "[a](diagram.png)"
        );
    }

    /// Hugo lowercases every directory segment on the way to a URL, so a link
    /// written the way the file is named points at a page that was never
    /// published. These are the two shapes a real bundle carries: a
    /// capitalised directory and a `README.md`.
    #[test]
    fn a_capitalised_page_link_lands_on_the_url_hugo_published() {
        assert_eq!(
            rewrite_text("[a](/Documentation/README.md)", &context("")),
            "[a](/lane-cast/documentation/readme/)"
        );
        assert_eq!(
            rewrite_text("[a](/Documentation/index.md)", &context("")),
            "[a](/lane-cast/documentation/)"
        );
        assert_eq!(
            rewrite_text("[a](README.md)", &context("Documentation")),
            "[a](/lane-cast/documentation/readme/)"
        );
    }

    /// A page resource is the one place the last segment is left alone: Hugo
    /// sanitises the directories above it and copies the filename across
    /// exactly as written, which was measured rather than assumed.
    #[test]
    fn a_directory_is_published_lowercased_and_an_asset_keeps_its_own_name() {
        let mount = std::env::temp_dir().join(format!("okf-links-{}", std::process::id()));
        let _ = std::fs::create_dir_all(mount.join("Documentation"));
        let _ = std::fs::write(mount.join("Documentation/Diagram.PNG"), "x");
        let context = Context::new(&mount, &mount, "lane-cast", "");
        assert_eq!(
            rewrite_text("[a](Documentation/)", &context),
            "[a](/lane-cast/documentation/)"
        );
        assert_eq!(
            rewrite_text("[a](Documentation/Diagram.PNG)", &context),
            "[a](/lane-cast/documentation/Diagram.PNG)"
        );
        let _ = std::fs::remove_dir_all(&mount);
    }

    #[test]
    fn an_image_and_an_unclosed_link_do_not_disturb_the_rest_of_the_document() {
        let context = context("");
        assert_eq!(
            rewrite_text("![alt](/img/x.svg) then [b](/c.md)", &context),
            "![alt](/lane-cast/img/x.svg) then [b](/lane-cast/c/)"
        );
        assert_eq!(
            rewrite_text("a](unclosed\nnext", &context),
            "a](unclosed\nnext"
        );
    }
}
