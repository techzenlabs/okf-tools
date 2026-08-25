//! Write derivable frontmatter into a bundle that has none.
//!
//! Usage: `okf-migrate [--report | --dry-run | --apply] [<path>…]`, run from
//! the repository root. With no mode, `--report` is assumed, because a tool
//! that writes by default is a tool somebody runs by accident.
//!
//! Paths, when given, are bundle-relative prefixes, so a migration is done one
//! directory at a time. That is what makes a batch a unit a person can review
//! and `git checkout` can address.

use std::process::ExitCode;

use okf_tools::migrate::{self, Skip};

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let prefixes: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();

    let cwd = std::env::current_dir()?;
    let (root, config) = okf_tools::open_bundle(&cwd)?;
    let mut plan = migrate::plan(&root, &config);
    if !prefixes.is_empty() {
        plan.entries
            .retain(|e| prefixes.iter().any(|p| e.path.starts_with(p)));
    }

    let unmatched: Vec<_> = plan.unmatched().collect();
    let undescribed: Vec<_> = plan.undescribed().collect();
    let changes = plan.changes().count();

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

    if apply {
        let written = migrate::apply(&root, &plan)?;
        println!("Migrated {written} file(s).");
        // Unmatched files are the whole point of reporting rather than
        // guessing, so they make the run non-zero: the batch is not finished
        // until somebody has typed them.
        if !unmatched.is_empty() {
            return Ok(ExitCode::FAILURE);
        }
    } else {
        println!();
        println!("{changes} file(s) would change. Nothing written.");
    }
    Ok(ExitCode::SUCCESS)
}
