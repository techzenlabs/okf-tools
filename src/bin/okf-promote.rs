//! Copy a page from a private bundle into a client-facing one.
//!
//! Usage:
//!
//! ```text
//! okf-promote --propose <source-path> --to <bundle> [--draft <file>] [--dry-run]
//! okf-promote --refresh [<page>…] [--dry-run]
//! okf-promote --drift
//! ```
//!
//! `--propose` runs from the source repository's root; `--refresh` and
//! `--drift` run from the destination bundle's root, because both read the
//! private repository to answer a question about the public one and therefore
//! run only where both are checked out.
//!
//! The resolution report goes to stderr and the drafted page to stdout, so
//! `okf-promote --propose … > draft.md` hands you something to edit while the
//! report stays where you can read it. Nothing is written into the destination
//! bundle until the report is empty.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use okf_tools::promote::{self, Bundle, Git, Item, Proposal, Severity};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-promote: {err}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage: okf-promote --propose <source-path> --to <bundle> [--draft <file>] [--dry-run]
       okf-promote --refresh [<page>…] [--dry-run]
       okf-promote --drift

  --propose  draft a page and report every link the destination cannot hold.
             Runs from the source repository's root. Writes nothing while the
             report has an unresolved item.
  --refresh  redraw a promoted page from its source and report what the source
             has grown since. Runs from the destination bundle's root. Never
             overwrites the page: it records the new commit and nothing else.
  --drift    list promoted pages whose source has moved.
  --draft    a hand-restated draft to review and install in place of the
             source's own text.
  --dry-run  report, and write nothing.";

/// What the command line asked for.
struct Args {
    mode: Mode,
    to: Option<String>,
    draft: Option<PathBuf>,
    dry_run: bool,
    positional: Vec<String>,
}

#[derive(PartialEq, Eq)]
enum Mode {
    Propose,
    Refresh,
    Drift,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = None;
    let mut to = None;
    let mut draft = None;
    let mut dry_run = false;
    let mut positional = Vec::new();

    let mut index = 0;
    while index < raw.len() {
        let Some(arg) = raw.get(index) else { break };
        let value = |name: &str| -> Result<String, String> {
            raw.get(index.saturating_add(1))
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--propose" => mode = Some(Mode::Propose),
            "--refresh" => mode = Some(Mode::Refresh),
            "--drift" => mode = Some(Mode::Drift),
            "--dry-run" => dry_run = true,
            "--to" => {
                to = Some(value("--to")?);
                index = index.saturating_add(1);
            }
            "--draft" => {
                draft = Some(PathBuf::from(value("--draft")?));
                index = index.saturating_add(1);
            }
            other if other.starts_with("--") => {
                return Err(format!("unrecognised flag `{other}`"));
            }
            other => positional.push(other.to_owned()),
        }
        index = index.saturating_add(1);
    }
    let mode =
        mode.ok_or_else(|| "one of --propose, --refresh or --drift is required".to_owned())?;
    Ok(Args {
        mode,
        to,
        draft,
        dry_run,
        positional,
    })
}

fn run() -> anyhow::Result<ExitCode> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(why) => {
            eprintln!("okf-promote: {why}\n\n{USAGE}");
            return Ok(ExitCode::FAILURE);
        }
    };
    let cwd = std::env::current_dir()?;
    match args.mode {
        Mode::Propose => propose(&cwd, &args),
        Mode::Refresh => refresh(&cwd, &args),
        Mode::Drift => drift(&cwd),
    }
}

fn propose(cwd: &Path, args: &Args) -> anyhow::Result<ExitCode> {
    let Some(source_relative) = args.positional.first() else {
        eprintln!("okf-promote: --propose needs a source path\n\n{USAGE}");
        return Ok(ExitCode::FAILURE);
    };
    let Some(to) = args.to.as_deref() else {
        eprintln!("okf-promote: --propose needs --to <bundle>\n\n{USAGE}");
        return Ok(ExitCode::FAILURE);
    };
    let source = Bundle::open(cwd, "source")?;
    let Some(entry) = source.config.promote.destination(to) else {
        eprintln!("okf-promote: no [[promote.destination]] named `{to}`");
        return Ok(ExitCode::FAILURE);
    };
    let destination_repo = cwd.join(&entry.path);
    let destination = Bundle::open(&destination_repo, to)?;

    let draft_body = match &args.draft {
        Some(path) => Some(std::fs::read_to_string(path)?),
        None => None,
    };
    let proposal = promote::propose(
        &source,
        source_relative,
        to,
        &destination,
        draft_body.as_deref(),
        &Git,
    )?;

    print_report(&proposal);
    print!("{}", proposal.draft);

    if proposal.blocked() {
        eprintln!(
            "\nnothing written: {} unresolved item(s) stand.",
            proposal.unresolved().count()
        );
        return Ok(ExitCode::FAILURE);
    }
    if args.dry_run {
        eprintln!("\n--dry-run: nothing written.");
        return Ok(ExitCode::SUCCESS);
    }
    promote::install(&destination, &proposal)?;
    eprintln!("\nwrote {to}:{}", proposal.destination_path);
    if promote::write_source_pointer(&source, source_relative, &proposal.url)? {
        eprintln!("pointed {source_relative} at {}", proposal.url);
    }
    Ok(ExitCode::SUCCESS)
}

fn refresh(cwd: &Path, args: &Args) -> anyhow::Result<ExitCode> {
    let destination = Bundle::open(cwd, "destination")?;
    let pages: Vec<String> = if args.positional.is_empty() {
        promote::promoted_pages(&destination)
            .into_iter()
            .map(|(page, _)| page)
            .collect()
    } else {
        args.positional.clone()
    };
    if pages.is_empty() {
        println!("No page in this bundle carries `promoted_from`.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut blocked = 0_usize;
    for page in &pages {
        let found = promote::refresh(&destination, page, &Git)?;
        if !found.moved() {
            println!("{page}: source unchanged at {}", found.provenance.rev);
            continue;
        }
        println!(
            "{page}: source moved {} → {}",
            short(&found.provenance.rev),
            short(&found.current)
        );
        if found.new_items.is_empty() {
            if args.dry_run {
                println!("  no new unresolved item; --dry-run, commit not recorded.");
            } else {
                promote::bump_rev(&destination, page, &found.current)?;
                println!(
                    "  no new unresolved item; recorded {}",
                    short(&found.current)
                );
            }
            continue;
        }
        blocked = blocked.saturating_add(1);
        println!(
            "  {} item(s) the source has grown since:",
            found.new_items.len()
        );
        for item in &found.new_items {
            for line in item_lines(item, "  ") {
                println!("{line}");
            }
        }
        println!(
            "  the promoted page is unchanged. Resolve these into it by hand, then\n  \
             re-run to record the commit."
        );
    }
    if blocked > 0 {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn drift(cwd: &Path) -> anyhow::Result<ExitCode> {
    let destination = Bundle::open(cwd, "destination")?;
    let moved = promote::drift(&destination, &Git)?;
    let total = promote::promoted_pages(&destination).len();
    if moved.is_empty() {
        println!("{total} promoted page(s), none whose source has moved.");
        return Ok(ExitCode::SUCCESS);
    }
    println!("page\trepo\tsource\trecorded\tcurrent");
    for row in &moved {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.page,
            row.provenance.repo,
            row.provenance.path,
            short(&row.provenance.rev),
            short(&row.current)
        );
    }
    println!();
    println!(
        "{} of {total} promoted page(s) have a source that moved. Each one is a\n\
         re-promotion decision: run --refresh to see what the source grew.",
        moved.len()
    );
    Ok(ExitCode::FAILURE)
}

fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

fn print_report(proposal: &Proposal) {
    let unresolved = proposal.unresolved().count();
    let notes = proposal.items.len().saturating_sub(unresolved);
    eprintln!(
        "resolution report for {} → {}:{}",
        proposal.source_path, proposal.destination, proposal.destination_path
    );
    if proposal.items.is_empty() {
        eprintln!("  clean: every link resolves inside the destination bundle.");
        return;
    }
    eprintln!(
        "  {unresolved} unresolved, {notes} {}. Lines are in the draft on stdout.",
        if notes == 1 { "note" } else { "notes" }
    );
    for item in &proposal.items {
        eprintln!();
        for line in item_lines(item, "  ") {
            eprintln!("{line}");
        }
    }
}

/// One item, rendered. The caller picks the stream: a proposal's report goes to
/// stderr so the draft can have stdout, and a refresh has no draft to hand out.
fn item_lines(item: &Item, indent: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let severity = match item.severity() {
        Severity::Unresolved => "UNRESOLVED",
        Severity::Note => "note",
    };
    let where_ = if item.line == 0 {
        String::new()
    } else {
        format!("  line {}", item.line)
    };
    lines.push(format!("{indent}{severity}  {}{where_}", item.kind.label()));
    if !item.subject.is_empty() {
        lines.push(format!("{indent}    subject: {}", item.subject));
    }
    if !item.sentence.is_empty() {
        lines.push(format!("{indent}    in:      {}", item.sentence));
    }
    for (n, line) in wrap(&item.replacement, 68).into_iter().enumerate() {
        let label = if n == 0 { "replace:" } else { "        " };
        lines.push(format!("{indent}    {label} {line}"));
    }
    lines
}

/// Wrap on whitespace at `width`, never splitting a word.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len().saturating_add(word.len()) >= width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
