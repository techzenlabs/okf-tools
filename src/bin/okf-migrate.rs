//! Write derivable frontmatter into a bundle that has none.
//!
//! Usage: `okf-migrate [--retype] [--report | --dry-run | --apply] [<path>…]`,
//! run from the repository root. With no mode, `--report` is assumed, because
//! a tool that writes by default is a tool somebody runs by accident.
//!
//! Paths, when given, are bundle-relative prefixes, so a migration is done one
//! directory at a time. That is what makes a batch a unit a person can review
//! and `git checkout` can address.
//!
//! `--retype` is the other pass, for the one bundle whose documents are
//! already typed and whose names are being reduced to the ratified vocabulary.
//! It reads the `[[retype]]` table from `okf.toml`, changes nothing but
//! `type`, and lists the files a person has to decide. See
//! [`okf_tools::retype`].

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use okf_tools::config::Config;
use okf_tools::migrate::{self, Skip};
use okf_tools::retype::{self, Action};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-migrate: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    // Positional arguments are meaningful here (bundle-relative path
    // prefixes), so only flags are validated. An unrecognised flag still has
    // to be refused: `--aply` silently meaning "report" is a surprise, and the
    // same slip on a command that wrote would be a destructive one.
    const FLAGS: [&str; 4] = ["--apply", "--dry-run", "--report", "--retype"];
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(bad) = args
        .iter()
        .find(|a| a.starts_with("--") && !FLAGS.contains(&a.as_str()))
    {
        eprintln!(
            "okf-migrate: unrecognised flag `{bad}`\n\n\
             usage: okf-migrate [--retype] [--report | --dry-run | --apply] [<path>…]\n\n\
             With no flag, reports and writes nothing. Paths are\n\
             bundle-relative prefixes, so a batch is one directory.\n\
             --retype applies okf.toml's [[retype]] table to documents that\n\
             already carry a type, and changes nothing but that field."
        );
        return Ok(ExitCode::FAILURE);
    }
    let apply = args.iter().any(|a| a == "--apply");
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let prefixes: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();

    let cwd = std::env::current_dir()?;
    let (root, config) = okf_tools::open_bundle(&cwd)?;
    let repo_root = git_toplevel(&cwd)?;
    if args.iter().any(|a| a == "--retype") {
        return retype_run(&repo_root, &root, &config, &prefixes, apply, dry_run);
    }
    migrate_run(&repo_root, &root, &config, &prefixes, apply, dry_run)
}

/// The migration pass: write the frontmatter a bundle has not got.
fn migrate_run(
    repo_root: &Path,
    root: &Path,
    config: &Config,
    prefixes: &[&str],
    apply: bool,
    dry_run: bool,
) -> anyhow::Result<ExitCode> {
    let mut plan = migrate::plan(root, config);
    if !prefixes.is_empty() {
        plan.entries
            .retain(|e| prefixes.iter().any(|p| e.path.starts_with(p)));
    }

    let unmatched: Vec<_> = plan.unmatched().collect();
    let likely_generated: Vec<_> = plan.likely_generated().collect();
    let undescribed: Vec<_> = plan.undescribed().collect();
    let changes = plan.changes().count();
    let changed_paths: Vec<&str> = plan.changes().map(|entry| entry.path.as_str()).collect();
    let pinned = pinned_preflight(repo_root, &changed_paths)?;

    if dry_run {
        for entry in plan.changes() {
            println!("--- {} ---", entry.path);
            if let Some(text) = &entry.rewritten {
                for line in text.lines().take(6) {
                    println!("  {line}");
                }
            }
        }
    } else if !apply {
        println!("path\ttype\trule\ttitle_source\tdescription_source\tskip");
        for entry in &plan.entries {
            println!("{}", entry.tsv());
        }
    }

    if !unmatched.is_empty() {
        println!();
        println!(
            "{} file(s) matched no [[type_rules]] entry and were not migrated:",
            unmatched.len()
        );
        for entry in &unmatched {
            println!("  {}", entry.path);
        }
    }
    if !likely_generated.is_empty() {
        println!();
        println!(
            "{} file(s) look generated and were not migrated:",
            likely_generated.len()
        );
        for entry in &likely_generated {
            println!(
                "  {}: {}",
                entry.path,
                entry.skip.as_ref().map_or(String::new(), Skip::label)
            );
        }
        println!(
            "Add generator-owned paths to `[paths] generated`, or remove a misleading signal."
        );
    }
    if !undescribed.is_empty() {
        println!();
        println!(
            "{} file(s) have no derivable description; someone has to write one:",
            undescribed.len()
        );
        for entry in &undescribed {
            println!("  {}", entry.path);
        }
    }
    let broken: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| matches!(e.skip, Some(Skip::Unparseable(_))))
        .collect();
    if !broken.is_empty() {
        println!();
        println!(
            "{} file(s) have frontmatter that does not parse:",
            broken.len()
        );
        for entry in &broken {
            println!(
                "  {}: {}",
                entry.path,
                entry.skip.as_ref().map_or(String::new(), Skip::label)
            );
        }
    }
    report_pinned(&pinned);

    if apply {
        if !pinned.is_empty() {
            println!("Migration refused before any file was written.");
            return Ok(ExitCode::FAILURE);
        }
        let written = migrate::apply(root, &plan)?;
        println!("Migrated {written} file(s).");
        // Unmatched files are the whole point of reporting rather than
        // guessing, so they make the run non-zero: the batch is not finished
        // until somebody has typed them.
        if !unmatched.is_empty() || !likely_generated.is_empty() {
            return Ok(ExitCode::FAILURE);
        }
    } else {
        println!();
        println!("{changes} file(s) would change. Nothing written.");
    }
    Ok(ExitCode::SUCCESS)
}

/// The `--retype` pass: apply `okf.toml`'s `[[retype]]` table.
///
/// Separate from the migration pass rather than folded into it. The bundle
/// this exists for is the byte-for-byte parity target for the whole port, and
/// the pass that changes 672 documents is not the pass to also derive titles,
/// write descriptions or normalise quoting.
fn retype_run(
    repo_root: &Path,
    root: &Path,
    config: &Config,
    prefixes: &[&str],
    apply: bool,
    dry_run: bool,
) -> anyhow::Result<ExitCode> {
    let mut plan = retype::plan(root, config)?;
    if !prefixes.is_empty() {
        let under = |path: &str| prefixes.iter().any(|p| path.starts_with(p));
        plan.entries.retain(|entry| under(&entry.path));
        plan.unparseable.retain(|(path, _)| under(path));
    }

    let judgements: Vec<_> = plan.judgements().collect();
    let changes = plan.changes().count();
    let changed_paths: Vec<&str> = plan.changes().map(|entry| entry.path.as_str()).collect();
    let pinned = pinned_preflight(repo_root, &changed_paths)?;

    if dry_run {
        for entry in plan.changes() {
            if let Action::Rename { to, .. } = &entry.action {
                println!("{}: {} -> {}", entry.path, entry.from, to);
            }
        }
    } else if !apply {
        println!("path\tfrom\tto\taction");
        for entry in &plan.entries {
            println!("{}", entry.tsv());
        }
    }

    if !judgements.is_empty() {
        println!();
        println!(
            "{} file(s) carry a type the table leaves to a person:",
            judgements.len()
        );
        for (entry, why) in &judgements {
            println!("  {} [{}]: {why}", entry.path, entry.from);
        }
    }
    let unnamed = plan.unnamed_types();
    if !unnamed.is_empty() {
        println!();
        println!(
            "{} type name(s) the table does not mention, left as written:",
            unnamed.len()
        );
        for (name, count) in &unnamed {
            println!("  {count:>5}  {name}");
        }
    }
    if !plan.unparseable.is_empty() {
        println!();
        println!(
            "{} file(s) have frontmatter that does not parse:",
            plan.unparseable.len()
        );
        for (path, why) in &plan.unparseable {
            println!("  {path}: {why}");
        }
    }
    report_pinned(&pinned);

    if apply {
        if !pinned.is_empty() {
            println!("Retype refused before any file was written.");
            return Ok(ExitCode::FAILURE);
        }
        let written = retype::apply(root, &plan)?;
        println!("Retyped {written} file(s).");
        // Same rule as a migration's unmatched files: the pass is not finished
        // until somebody has decided the ones the table refused to decide.
        if !judgements.is_empty() {
            return Ok(ExitCode::FAILURE);
        }
    } else {
        println!();
        println!("{changes} file(s) would change. Nothing written.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Refuse to buffer an arbitrarily large tracked file for a textual preflight.
const MAX_TRACKED_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRACKED_TEXT_TOTAL_BYTES: usize = 256 * 1024 * 1024;

fn git_toplevel(invocation_root: &Path) -> anyhow::Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(invocation_root)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse --show-toplevel failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let path = std::str::from_utf8(&output.stdout)
        .map_err(|_| anyhow::anyhow!("git reported a repository root that is not UTF-8"))?
        .trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        anyhow::bail!("git reported an empty repository root");
    }
    Ok(PathBuf::from(path))
}

/// Current contents of Git-tracked files that are not Markdown documents.
fn tracked_non_markdown(repo_root: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(repo_root)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut files = Vec::new();
    let mut total_bytes = 0_usize;
    for encoded in output.stdout.split(|byte| *byte == 0) {
        if encoded.is_empty() {
            continue;
        }
        let path = std::str::from_utf8(encoded)
            .map_err(|_| anyhow::anyhow!("git reported a tracked path that is not UTF-8"))?;
        if Path::new(path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            continue;
        }

        let full_path = repo_root.join(path);
        let metadata = std::fs::symlink_metadata(&full_path).map_err(|source| {
            anyhow::anyhow!("could not inspect tracked file `{path}`: {source}")
        })?;
        let text = if metadata.file_type().is_dir() {
            continue;
        } else if metadata.file_type().is_symlink() {
            std::fs::read_link(&full_path)
                .map_err(|source| {
                    anyhow::anyhow!("could not read tracked symlink `{path}`: {source}")
                })?
                .to_string_lossy()
                .into_owned()
        } else if metadata.file_type().is_file() {
            let Some(text) = read_tracked_text(&full_path, path)? else {
                continue;
            };
            text
        } else {
            anyhow::bail!("tracked path `{path}` is not a regular file, symlink, or directory");
        };
        total_bytes = total_bytes
            .checked_add(text.len())
            .ok_or_else(|| anyhow::anyhow!("tracked text size overflowed"))?;
        if total_bytes > MAX_TRACKED_TEXT_TOTAL_BYTES {
            anyhow::bail!(
                "tracked non-Markdown text exceeds the {MAX_TRACKED_TEXT_TOTAL_BYTES}-byte preflight limit"
            );
        }
        files.push((path.to_owned(), text));
    }
    Ok(files)
}

fn read_tracked_text(path: &Path, display: &str) -> anyhow::Result<Option<String>> {
    let file = std::fs::File::open(path)
        .map_err(|source| anyhow::anyhow!("could not read tracked file `{display}`: {source}"))?;
    let limit = u64::try_from(MAX_TRACKED_TEXT_BYTES.saturating_add(1))?;
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| anyhow::anyhow!("could not read tracked file `{display}`: {source}"))?;
    if bytes.contains(&0) {
        return Ok(None);
    }
    if bytes.len() > MAX_TRACKED_TEXT_BYTES {
        anyhow::bail!(
            "tracked file `{display}` exceeds the {MAX_TRACKED_TEXT_BYTES}-byte preflight limit"
        );
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn pinned_preflight(
    repo_root: &Path,
    changed_paths: &[&str],
) -> anyhow::Result<Vec<migrate::PinnedReference>> {
    if changed_paths.is_empty() {
        return Ok(Vec::new());
    }
    let tracked = tracked_non_markdown(repo_root)?;
    migrate::pinned_references(
        changed_paths.iter().copied(),
        tracked
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str())),
    )
    .map_err(Into::into)
}

fn report_pinned(references: &[migrate::PinnedReference]) {
    if references.is_empty() {
        return;
    }
    let file_count = references
        .iter()
        .map(|reference| reference.file.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!();
    println!("{file_count} tracked non-Markdown file(s) may pin document bytes:");
    for reference in references {
        println!(
            "  {} <- {}:{} ({} at line {})",
            reference.document,
            reference.file,
            reference.path_line,
            reference.key,
            reference.key_line
        );
    }
    println!("Decide what each binding should cover before changing the document bytes.");
}
