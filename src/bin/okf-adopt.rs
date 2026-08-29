//! Read-only checks to run while adopting OKF.
//!
//! Usage: `okf-adopt --survey-branches`, run anywhere in the repository.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("okf-adopt: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() != ["--survey-branches"] {
        eprintln!(
            "usage: okf-adopt --survey-branches\n\n\
             Reads local remote-tracking refs and reports branch-only Markdown.\n\
             Fetch first when the local refs need refreshing. Nothing is written."
        );
        return Ok(ExitCode::FAILURE);
    }

    let cwd = std::env::current_dir()?;
    let repo_root = git_toplevel(&cwd)?;
    let (bundle_root, config) = okf_tools::open_bundle(&repo_root)?;
    for branch in okf_tools::adopt::survey_branches(&repo_root, &bundle_root, &config)? {
        println!("{}:", branch.branch);
        for finding in branch.findings {
            println!("  {}\t{}", finding.classification.label(), finding.path);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn git_toplevel(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse --show-toplevel failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}
