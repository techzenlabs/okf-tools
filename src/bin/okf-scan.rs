//! Scan a tree for material that must not be published.
//!
//! Usage: `okf-scan [<path>] [--bare-9] [--exclude <prefix>]...`
//!
//! Exits non-zero on a finding, on a file it could not inspect, and on a run
//! that inspected nothing. The third is the one that matters: a scanner
//! pointed at the wrong directory otherwise reports a clean tree.

use std::path::PathBuf;
use std::process::ExitCode;

use okf_tools::scan::{self, Options};

const USAGE: &str = "usage: okf-scan [<path>] [--bare-9] [--exclude <prefix>]...\n\n\
     Fails on a finding, on a file it cannot inspect, and on a run that\n\
     inspected nothing at all.";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-scan: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let mut root: Option<PathBuf> = None;
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bare-9" => options.bare_nine_digit = true,
            "--exclude" => options.exclude.push(args.next().unwrap_or_default()),
            other if other.starts_with("--") => {
                eprintln!("okf-scan: unrecognised argument `{other}`\n\n{USAGE}");
                return Ok(ExitCode::FAILURE);
            }
            path => root = Some(PathBuf::from(path)),
        }
    }
    let root = root.unwrap_or_else(|| PathBuf::from("."));
    let report = scan::scan(&root, &options)?;

    for exclude in &options.exclude {
        println!("okf-scan: not scanning {exclude}");
    }
    for finding in &report.findings {
        println!("{finding}");
    }
    for path in &report.unreadable {
        println!("{path}: could not be inspected");
    }
    if let Some(reason) = report.failure_reason() {
        println!(
            "okf-scan: {reason} — {} file(s) inspected, {} skipped as binary.",
            report.scanned, report.binary
        );
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "okf-scan: clean — {} file(s) inspected, {} skipped as binary.",
        report.scanned, report.binary
    );
    Ok(ExitCode::SUCCESS)
}
