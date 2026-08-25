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

const USAGE: &str = "usage: okf-check [--quiet]\n\n\
     Validates the bundle against OKF v0.2. --quiet suppresses warnings.";

fn run() -> anyhow::Result<ExitCode> {
    if let Some(bad) = unknown_argument(&["--quiet"]) {
        eprintln!("okf-check: unrecognised argument `{bad}`\n\n{USAGE}");
        return Ok(ExitCode::FAILURE);
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

fn unknown_argument(accepted: &[&str]) -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|arg| !accepted.contains(&arg.as_str()))
}
