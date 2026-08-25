//! Regenerate every OKF `index.md` in a bundle from concept frontmatter.
//!
//! Usage: `okf-index [--check]`, run from the repository root.
//!
//! `--check` reports out-of-date indexes and exits non-zero instead of
//! writing, which is what a build gate runs.

use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-index: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let check = std::env::args().any(|a| a == "--check");
    let cwd = std::env::current_dir()?;
    let (root, config) = okf_tools::open_bundle(&cwd)?;
    let outcome = okf_tools::index::run(&root, &config, check)?;

    if check {
        if outcome.stale.is_empty() {
            println!("All index.md files are current.");
            return Ok(ExitCode::SUCCESS);
        }
        println!(
            "{} index file(s) out of date — run okf-index:",
            outcome.stale.len()
        );
        for stale in &outcome.stale {
            println!("  {stale}");
        }
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "Wrote {} index file(s) across {} directories.",
        outcome.written, outcome.directories
    );
    Ok(ExitCode::SUCCESS)
}
