//! The shared Hugo layout set, and the gate that stops a tenant forking it.
//!
//! Four site repositories are four chances to copy a theme, and this estate
//! already paid for that once: two knowledge repositories carry byte-identical
//! frozen copies of a script whose upstream fix never reached them, because
//! the fix had nowhere to go. A forked layout would be worse, since a layout
//! bug is invisible until somebody reads a page.
//!
//! So the files ship inside the binary and `okf-assemble` writes them out on
//! every run. A tenant's `layouts/` therefore holds only what that tenant
//! added, which makes the gate below a simple question: does the repository
//! *track* a path this set owns?
//!
//! Overriding still works, and goes through Hugo's own lookup rather than
//! through copying. Adding `layouts/<mount>/single.html` or
//! `layouts/partials/brand.html` composes and is legitimate. Replacing
//! `baseof.html` or a render hook is not, because those carry behaviour that
//! was measured rather than chosen: the real `<html><body>` Pagefind needs to
//! index anything at all, the `data-pagefind-body` region, the flag read after
//! the content block rather than in `<head>`, and the unescaped mermaid arrow.

/// A file `okf-assemble` writes into the site tree on every run.
pub struct SharedFile {
    /// Path relative to the site root, always with forward slashes.
    pub path: &'static str,
    pub contents: &'static str,
}

/// The eleven layout files every tenant renders through.
pub const LAYOUTS: &[SharedFile] = &[
    SharedFile {
        path: "layouts/_default/baseof.html",
        contents: include_str!("../site/layouts/_default/baseof.html"),
    },
    SharedFile {
        path: "layouts/404.html",
        contents: include_str!("../site/layouts/404.html"),
    },
    SharedFile {
        path: "layouts/_default/single.html",
        contents: include_str!("../site/layouts/_default/single.html"),
    },
    SharedFile {
        path: "layouts/_default/list.html",
        contents: include_str!("../site/layouts/_default/list.html"),
    },
    SharedFile {
        path: "layouts/index.html",
        contents: include_str!("../site/layouts/index.html"),
    },
    SharedFile {
        path: "layouts/partials/okf-meta.html",
        contents: include_str!("../site/layouts/partials/okf-meta.html"),
    },
    SharedFile {
        path: "layouts/_default/single.okfmarkdown.md",
        contents: include_str!("../site/layouts/_default/single.okfmarkdown.md"),
    },
    SharedFile {
        path: "layouts/_default/list.okfmarkdown.md",
        contents: include_str!("../site/layouts/_default/list.okfmarkdown.md"),
    },
    SharedFile {
        path: "layouts/index.okfindexjson.json",
        contents: include_str!("../site/layouts/index.okfindexjson.json"),
    },
    SharedFile {
        path: "layouts/_default/list.llmstxt.txt",
        contents: include_str!("../site/layouts/_default/list.llmstxt.txt"),
    },
    SharedFile {
        path: "layouts/_default/_markup/render-codeblock-mermaid.html",
        contents: include_str!("../site/layouts/_default/_markup/render-codeblock-mermaid.html"),
    },
];

/// Everything else identical across tenants: the stylesheet and the recipes.
///
/// The `justfile` is here for the same reason the layouts are. It is the same
/// four recipes in all four tenants, so it has one home and reaches a tenant
/// by rolling a flake input forward rather than by four commits.
pub const SHARED: &[SharedFile] = &[
    SharedFile {
        path: "static/css/okf.css",
        contents: include_str!("../site/static/css/okf.css"),
    },
    SharedFile {
        path: "justfile",
        contents: include_str!("../site/justfile"),
    },
];

/// Every path this set owns.
#[must_use]
pub fn owned_paths() -> Vec<&'static str> {
    LAYOUTS
        .iter()
        .chain(SHARED.iter())
        .map(|file| file.path)
        .collect()
}

/// Does this repository track a file whose path `okf-tools` owns?
///
/// `tracked` is what the repository tracks, which is the right question and
/// not the same as what is on disk: `okf-assemble` writes the shared set into
/// the working tree on every run, so a filesystem walk would report every
/// tenant as a fork the moment it built.
#[must_use]
pub fn forked<'a>(tracked: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let owned = owned_paths();
    let mut found: Vec<String> = tracked
        .into_iter()
        .map(|path| path.trim_start_matches("./").replace('\\', "/"))
        .filter(|path| owned.iter().any(|owned| *owned == path))
        .collect();
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eleven layouts, and the count is asserted because the set is a
    /// contract: a file added here has to be added to the gate's list in the
    /// same edit, or a tenant could fork the new one and nothing would say so.
    #[test]
    fn the_shared_set_is_eleven_layouts_and_two_siblings() {
        assert_eq!(LAYOUTS.len(), 11);
        assert_eq!(SHARED.len(), 2);
        assert_eq!(owned_paths().len(), 13);
    }

    #[test]
    fn no_shared_file_ships_empty() {
        for file in LAYOUTS.iter().chain(SHARED.iter()) {
            assert!(!file.contents.trim().is_empty(), "{} is empty", file.path);
        }
    }

    #[test]
    fn a_tenant_overlay_composes_and_a_replacement_is_reported() {
        let tracked = [
            "layouts/partials/brand.html",
            "layouts/lane-cast/single.html",
            "hugo.toml",
        ];
        assert!(forked(tracked).is_empty());

        let replaced = ["layouts/_default/baseof.html", "site.toml"];
        assert_eq!(forked(replaced), ["layouts/_default/baseof.html"]);
    }
}
