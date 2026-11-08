//! A single-value flag given twice, and two modes given together, are refused.
//!
//! Every binary here parses its own command line by hand, and each used to let
//! a second `--root`, `--to` or `--as-of=` replace the first without a word,
//! and run one of two modes given together while dropping the other. Each case
//! below is now a usage error that names the flag, on stderr, with the exit
//! code every one of these binaries already uses for a usage error (1), and
//! with nothing on stdout because nothing ran.

#![expect(
    clippy::expect_used,
    reason = "a panicking assertion is the point of a test"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// An empty working directory: no bundle, no manifest, no repository. Every
/// refusal happens before any of those is looked for.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let sequence = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "okf-repeated-flags-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(binary: &str, cwd: &Path, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("the binary should run")
}

const PROMOTE: &str = env!("CARGO_BIN_EXE_okf-promote");
const SERVE: &str = env!("CARGO_BIN_EXE_okf-serve");
const ASSEMBLE: &str = env!("CARGO_BIN_EXE_okf-assemble");
const CHECK: &str = env!("CARGO_BIN_EXE_okf-check");
const MIGRATE: &str = env!("CARGO_BIN_EXE_okf-migrate");
const SCAN: &str = env!("CARGO_BIN_EXE_okf-scan");

/// The binary's file name, which is the prefix of each of its messages.
fn name(binary: &str) -> &str {
    Path::new(binary)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a binary path has a file name")
}

/// Run `args` and assert the refusal: exit 1, `<binary>: <message>` and the
/// usage text on stderr, and nothing on stdout.
fn assert_refused(binary: &str, args: &[&str], message: &str) {
    let scratch = Scratch::new("refused");
    let output = run(binary, scratch.path(), args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected = format!("{}: {message}", name(binary));
    assert_eq!(
        output.status.code(),
        Some(1),
        "{args:?} should exit 1; stderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected),
        "{args:?} should say `{expected}`; stderr: {stderr}"
    );
    assert!(
        stderr.contains("usage:"),
        "{args:?} should print the usage; stderr: {stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "{args:?} should run nothing"
    );
}

/// Run `args` and assert the parser accepted them: whatever the run did next,
/// it was not a usage error. In an empty directory most of these then fail for
/// want of a bundle, which is the point: they got that far.
fn assert_parsed(binary: &str, cwd: &Path, args: &[&str]) -> Output {
    let output = run(binary, cwd, args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("usage:") && !stderr.contains("given twice"),
        "{args:?} should get past the parser; stderr: {stderr}"
    );
    output
}

/// Every unordered pair of `modes`, each given in both orders.
fn pairs<'a>(modes: &[&'a [&'a str]]) -> Vec<(Vec<&'a str>, String)> {
    let mut cases = Vec::new();
    for (i, first) in modes.iter().enumerate() {
        for second in modes.iter().skip(i.saturating_add(1)) {
            for (a, b) in [(first, second), (second, first)] {
                let args: Vec<&str> = a.iter().chain(b.iter()).copied().collect();
                let (Some(a_flag), Some(b_flag)) = (a.first(), b.first()) else {
                    continue;
                };
                cases.push((
                    args,
                    format!("{a_flag} and {b_flag} cannot be given together"),
                ));
            }
        }
    }
    cases
}

const COMMIT_A: &str = "0123456789abcdef0123456789abcdef01234567";
const COMMIT_B: &str = "89abcdef0123456789abcdef0123456789abcdef";

#[test]
fn okf_promote_refuses_a_repeated_flag() {
    for (args, flag) in [
        (vec!["--propose", "--propose", "page.md"], "--propose"),
        (vec!["--refresh", "--refresh"], "--refresh"),
        (vec!["--drift", "--drift"], "--drift"),
        (
            vec!["--propose", "page.md", "--to", "one", "--to", "two"],
            "--to",
        ),
        (
            vec![
                "--propose",
                "page.md",
                "--to",
                "one",
                "--draft",
                "a.md",
                "--draft",
                "b.md",
            ],
            "--draft",
        ),
    ] {
        assert_refused(PROMOTE, &args, &format!("{flag} is given twice"));
    }
}

#[test]
fn okf_promote_refuses_two_modes_together() {
    let cases = pairs(&[&["--propose"], &["--refresh"], &["--drift"]]);
    assert_eq!(cases.len(), 6);
    for (args, message) in cases {
        assert_refused(PROMOTE, &args, &message);
    }
}

#[test]
fn okf_promote_takes_each_flag_once() {
    let scratch = Scratch::new("promote-once");
    for args in [
        vec![
            "--propose",
            "page.md",
            "--to",
            "one",
            "--draft",
            "a.md",
            "--dry-run",
        ],
        vec!["--refresh", "page.md"],
        vec!["--drift"],
    ] {
        assert_parsed(PROMOTE, scratch.path(), &args);
    }
}

#[test]
fn okf_serve_refuses_a_repeated_flag() {
    for (args, flag) in [
        (vec!["--root", "a", "--root", "b"], "--root"),
        (
            vec!["--root", "a", "--bind", "127.0.0.1", "--bind", "0.0.0.0"],
            "--bind",
        ),
        (vec!["--root", "a", "--port", "1", "--port", "2"], "--port"),
        (
            vec!["--root", "a", "--threads", "1", "--threads", "2"],
            "--threads",
        ),
        (
            vec![
                "--root",
                "a",
                "--require-header",
                "X-One",
                "--require-header",
                "X-Two",
            ],
            "--require-header",
        ),
    ] {
        assert_refused(SERVE, &args, &format!("{flag} is given twice"));
    }
}

#[test]
fn okf_serve_takes_each_flag_once() {
    // A root that is not there is the first thing checked after the parse,
    // and it refuses before anything listens.
    let scratch = Scratch::new("serve-once");
    let output = assert_parsed(
        SERVE,
        scratch.path(),
        &[
            "--root",
            "absent",
            "--bind",
            "127.0.0.1",
            "--port",
            "1",
            "--threads",
            "1",
            "--require-header",
            "X-One",
        ],
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not a directory"));
}

#[test]
fn okf_assemble_refuses_a_repeated_flag() {
    let pinned_a = format!("alpha=/src/a@{COMMIT_A}");
    let pinned_b = format!("alpha=/src/b@{COMMIT_B}");
    for (args, flag) in [
        (vec!["--update", "alpha", "--update", "beta"], "--update"),
        (
            vec!["--bootstrap", "--tenant", "one", "--tenant", "two"],
            "--tenant",
        ),
        (vec!["--bootstrap", "--scan", "a", "--scan", "b"], "--scan"),
        (vec!["--mermaid", "a.js", "--mermaid", "b.js"], "--mermaid"),
        (vec!["--bootstrap", "--bootstrap"], "--bootstrap"),
        (vec!["--verify-raw", "--verify-raw"], "--verify-raw"),
        (vec!["--bundles", "--bundles"], "--bundles"),
        (
            vec!["--local", "alpha=/a", "--local", "alpha=/b"],
            "--local alpha",
        ),
        (
            vec!["--pinned", &pinned_a, "--pinned", &pinned_b],
            "--pinned alpha",
        ),
    ] {
        assert_refused(ASSEMBLE, &args, &format!("{flag} is given twice"));
    }
}

#[test]
fn okf_assemble_refuses_two_modes_together() {
    let cases = pairs(&[
        &["--bootstrap"],
        &["--verify-raw"],
        &["--bundles"],
        &["--update", "alpha"],
    ]);
    assert_eq!(cases.len(), 12);
    for (args, message) in cases {
        assert_refused(ASSEMBLE, &args, &message);
    }
}

#[test]
fn okf_assemble_takes_each_flag_once_and_list_flags_repeatedly() {
    // No site.toml here, so each run stops at the manifest: after the parse.
    let scratch = Scratch::new("assemble-once");
    let pinned_a = format!("alpha=/src/a@{COMMIT_A}");
    let pinned_b = format!("beta=/src/b@{COMMIT_B}");
    for args in [
        vec!["--update", "alpha"],
        vec!["--bundles", "--check"],
        vec![
            "--tarball",
            "--mermaid",
            "a.js",
            "--local",
            "alpha=/a",
            "--local",
            "beta=/b",
        ],
        vec!["--pinned", &pinned_a, "--pinned", &pinned_b],
    ] {
        let output = assert_parsed(ASSEMBLE, scratch.path(), &args);
        assert!(!output.status.success(), "{args:?} found no site.toml");
    }
}

#[test]
fn okf_check_refuses_a_repeated_flag() {
    for (args, flag) in [
        (vec!["--as-of=2026-01-01", "--as-of=2026-02-02"], "--as-of"),
        (vec!["--layouts", "--layouts"], "--layouts"),
        (vec!["--shared-paths", "--shared-paths"], "--shared-paths"),
    ] {
        assert_refused(CHECK, &args, &format!("{flag} is given twice"));
    }
}

#[test]
fn okf_check_refuses_two_modes_together() {
    let cases = pairs(&[&["--layouts"], &["--shared-paths"]]);
    assert_eq!(cases.len(), 2);
    for (args, message) in cases {
        assert_refused(CHECK, &args, &message);
    }
}

#[test]
fn okf_check_takes_each_flag_once() {
    let scratch = Scratch::new("check-once");
    let output = assert_parsed(CHECK, scratch.path(), &["--shared-paths"]);
    assert!(output.status.success());
    assert!(!output.stdout.is_empty(), "--shared-paths prints the paths");
    assert_parsed(CHECK, scratch.path(), &["--quiet", "--as-of=2026-01-01"]);
}

#[test]
fn okf_migrate_refuses_a_repeated_mode() {
    for flag in ["--report", "--dry-run", "--apply"] {
        assert_refused(MIGRATE, &[flag, flag], &format!("{flag} is given twice"));
    }
}

#[test]
fn okf_migrate_refuses_two_modes_together() {
    let cases = pairs(&[&["--report"], &["--dry-run"], &["--apply"]]);
    assert_eq!(cases.len(), 6);
    for (args, message) in cases {
        assert_refused(MIGRATE, &args, &message);
    }
}

#[test]
fn okf_migrate_takes_each_mode_once() {
    let scratch = Scratch::new("migrate-once");
    for args in [
        vec!["--report"],
        vec!["--dry-run", "docs"],
        vec!["--retype", "--apply", "docs", "notes"],
    ] {
        assert_parsed(MIGRATE, scratch.path(), &args);
    }
}

#[test]
fn okf_scan_refuses_a_second_path() {
    assert_refused(
        SCAN,
        &["content", "static"],
        "<path> is given twice, at `static`",
    );
}

#[test]
fn okf_scan_takes_one_path_and_repeated_excludes() {
    let scratch = Scratch::new("scan-once");
    std::fs::write(scratch.path().join("page.md"), "A plain page.\n")
        .expect("fixture should be written");
    std::fs::create_dir_all(scratch.path().join("public")).expect("public/ should be created");
    let root = scratch.path().display().to_string();
    let output = assert_parsed(
        SCAN,
        scratch.path(),
        &[&root, "--exclude", "public", "--exclude", "work"],
    );
    assert!(
        output.status.success(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
