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

const USAGE: &str = "usage: okf-index [--check]\n\n\
     Regenerates every index.md in the bundle from concept frontmatter.\n\
     --check reports out-of-date indexes and writes nothing.";

fn run() -> anyhow::Result<ExitCode> {
    // Reject anything unrecognised rather than ignoring it. This command's
    // default action WRITES, so a mistyped or probing flag that fell through
    // to it would rewrite every index in the tree — which is exactly what
    // `okf-index --help` used to do.
    if let Some(bad) = unknown_argument(&["--check"]) {
        eprintln!("okf-index: unrecognised argument `{bad}`\n\n{USAGE}");
        return Ok(ExitCode::FAILURE);
    }
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

/// The first argument that is not one this command accepts.
///
/// Positional arguments are rejected too: neither command takes one, and a
/// stray path is far more likely to be a mistake than an intention.
fn unknown_argument(accepted: &[&str]) -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|arg| !accepted.contains(&arg.as_str()))
}
