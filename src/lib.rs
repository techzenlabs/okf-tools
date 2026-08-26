//! Open Knowledge Format v0.2 tooling.
//!
//! This crate is a port of the two Python scripts that grew inside the
//! reference bundle, not a rewrite of them. The asset being carried across is
//! the behaviour those scripts encode — the frontmatter parser that catches
//! duplicate keys and tab indentation, the marker-block index rewrite that
//! cannot drift from the documents, the month-grouped listing, the drop-folder
//! suppression, the deepest-first ordering that lets a parent index read a
//! child's description.
//!
//! Every constant those scripts hard-coded is now a key in [`config`], and the
//! defaults reproduce the constants exactly, so a bundle with no `okf.toml`
//! behaves as the originals did.
//!
//! The port is proved by parity rather than by inspection: `just parity` runs
//! both implementations over a real bundle and requires byte-identical
//! generated indexes *and* identical diagnostics. A checker that agrees on
//! output but disagrees on what it complains about has not been ported.

pub mod assemble;
pub mod bootstrap;
pub mod check;
pub mod collision;
pub mod config;
pub mod frontmatter;
pub mod glob;
pub mod hugopath;
pub mod index;
pub mod layouts;
pub mod links;
pub mod manifest;
pub mod migrate;
pub mod promote;
pub mod retype;
pub mod scan;
pub mod sitegen;
pub mod sitelinks;
pub mod staleness;
pub mod walk;

/// Locate the bundle root and its configuration.
///
/// `okf.toml` is read from the repository root, and `bundle_root` inside it
/// says where the bundle itself begins: `"."` for a knowledge repository whose
/// bundle is the whole repo, `"docs"` for a code repository where it is not.
///
/// # Errors
///
/// Fails when `okf.toml` exists but cannot be read, parsed, or is a newer
/// `config_version` than this build understands.
pub fn open_bundle(
    repo_root: &std::path::Path,
) -> Result<(std::path::PathBuf, config::Config), config::ConfigError> {
    let config = config::Config::load(repo_root)?;
    let root = repo_root.join(&config.bundle_root);
    Ok((root, config))
}
