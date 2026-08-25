//! Validate a bundle against the Open Knowledge Format, v0.2.
//!
//! Usage: `okf-check [--quiet]`, run from the repository root.
//!
//! Spec: <https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md>

use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-check: {err}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage: okf-check [--quiet]\n\
     \x20      okf-check --layouts\n\n\
     Validates the bundle against OKF v0.2. --quiet suppresses warnings.\n\
     --layouts checks a tenant site repository instead: it fails when the\n\
     repository tracks a file whose path okf-tools owns.";

fn run() -> anyhow::Result<ExitCode> {
    if let Some(bad) = unknown_argument(&["--quiet", "--layouts"]) {
        eprintln!("okf-check: unrecognised argument `{bad}`\n\n{USAGE}");
        return Ok(ExitCode::FAILURE);
    }
    if std::env::args().any(|a| a == "--layouts") {
        return Ok(check_layouts());
    }
    let quiet = std::env::args().any(|a| a == "--quiet");
    let cwd = std::env::current_dir()?;
    let (root, config) = okf_tools::open_bundle(&cwd)?;
    let report = okf_tools::check::check_bundle(&root, &config)?;

    if !report.warnings.is_empty() && !quiet {
        println!("{} warning(s):", report.warnings.len());
        for warning in &report.warnings {
            println!("  {warning}");
        }
        println!();
    }
    if !report.errors.is_empty() {
        println!(
            "{} OKF conformance error(s) across {} file(s):",
            report.errors.len(),
            report.checked
        );
        for error in &report.errors {
            println!("  {error}");
        }
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "OKF v0.2 conformant — {} markdown file(s) checked, {} warning(s).",
        report.checked,
        report.warnings.len()
    );
    if report.warnings.len() > config.max_warnings {
        println!(
            "warning budget is {}, and this run spent {}; \
             fix a file rather than raising the budget.",
            config.max_warnings,
            report.warnings.len()
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// Fail when a tenant repository tracks a layout `okf-tools` owns.
///
/// This is "do not fork the theme" turned into a gate. A tenant adding
/// `layouts/<mount>/single.html` or `layouts/partials/brand.html` composes
/// through Hugo's own lookup and passes; replacing `baseof.html` or a render
/// hook does not, because those carry behaviour that was measured rather than
/// chosen and a layout bug is invisible until somebody reads a page.
///
/// A tenant that genuinely needs a different `baseof.html` has found a gap in
/// `okf-tools`, and the fix belongs there.
fn check_layouts() -> ExitCode {
    let Some(tracked) = tracked_files() else {
        eprintln!(
            "okf-check: --layouts asks git what this repository tracks, and \
             git did not answer. Run it inside a checkout."
        );
        return ExitCode::FAILURE;
    };
    let forked = okf_tools::layouts::forked(tracked.lines());
    if forked.is_empty() {
        println!(
            "no forked layouts — okf-tools owns {} shared file(s), and this \
             repository tracks none of them.",
            okf_tools::layouts::owned_paths().len()
        );
        return ExitCode::SUCCESS;
    }
    println!("{} forked layout file(s):", forked.len());
    for path in &forked {
        println!("  {path}");
    }
    println!(
        "\nokf-assemble writes these on every build, so a tracked copy \
         shadows the shared one and stops receiving its fixes. Delete the \
         copy, or add the change to okf-tools where four tenants get it."
    );
    ExitCode::FAILURE
}

/// What git says this repository tracks.
///
/// `git ls-files` rather than a filesystem walk, and the distinction is the
/// whole check: `okf-assemble` writes the shared set into the working tree on
/// every build, so a walk would report every tenant as a fork the moment it
/// built.
fn tracked_files() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "--", "layouts", "justfile", "static"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn unknown_argument(accepted: &[&str]) -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|arg| !accepted.contains(&arg.as_str()))
}
