//! Assemble a tenant's Hugo content tree from its `site.toml`.
//!
//! Usage:
//!
//! ```text
//! okf-assemble [--tarball] [--local <id>=<path>]... [--pinned <id>=<path>@<rev>]...
//! okf-assemble --update <id>
//! okf-assemble --bootstrap [--tenant <name>] [--scan <dir>]
//! okf-assemble --verify-raw
//! okf-assemble --bundles [--check]
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use okf_tools::assemble::{self, Options};
use okf_tools::bootstrap;
use okf_tools::manifest::{self, Manifest};
use okf_tools::sitegen;

const USAGE: &str = "usage: okf-assemble [--tarball] [--local <id>=<path>]... \
     [--pinned <id>=<path>@<rev>]...\n\
     \x20      okf-assemble --update <id>\n\
     \x20      okf-assemble --bootstrap [--tenant <name>] [--scan <dir>]\n\
     \x20      okf-assemble --verify-raw\n\
     \x20      okf-assemble --bundles [--check]\n\n\
     Reads site.toml in the current directory and nothing else. --local points\n\
     one bundle at a working tree for this invocation only and is never\n\
     written to the manifest; the pages are stamped as a local build. --pinned\n\
     hands over a source already fetched at <rev>: the rev is verified against\n\
     the manifest's pin, a mismatch refuses the assembly, and a verified pin\n\
     is not stamped. --bundles regenerates nix/bundles.nix from the manifest\n\
     and fetches nothing; with --check it writes nothing and fails when the\n\
     tracked file and the manifest disagree.";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-assemble: {err}");
            ExitCode::FAILURE
        }
    }
}

/// What `--bundles` should do: regenerate the file, or assert it is current.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BundlesMode {
    Write,
    Check,
}

/// What the command line asked for.
#[derive(Default)]
struct Args {
    tarball: bool,
    verify_raw: bool,
    bootstrap: bool,
    bundles: Option<BundlesMode>,
    update: Option<String>,
    tenant: Option<String>,
    scan: Option<PathBuf>,
    mermaid: Option<PathBuf>,
    locals: BTreeMap<String, PathBuf>,
    pinned: BTreeMap<String, assemble::Pinned>,
}

fn parse() -> Result<Args, String> {
    let mut args = Args::default();
    let mut check = false;
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--tarball" => args.tarball = true,
            "--verify-raw" => args.verify_raw = true,
            "--bootstrap" => args.bootstrap = true,
            "--bundles" => args.bundles = Some(BundlesMode::Write),
            "--check" => check = true,
            "--update" => args.update = Some(value("--update")?),
            "--tenant" => args.tenant = Some(value("--tenant")?),
            "--scan" => args.scan = Some(PathBuf::from(value("--scan")?)),
            "--mermaid" => args.mermaid = Some(PathBuf::from(value("--mermaid")?)),
            "--local" => {
                let pair = value("--local")?;
                let (id, path) = pair
                    .split_once('=')
                    .ok_or_else(|| format!("--local wants <id>=<path>, not `{pair}`"))?;
                args.locals.insert(id.to_owned(), PathBuf::from(path));
            }
            "--pinned" => {
                let triple = value("--pinned")?;
                let (id, rest) = triple
                    .split_once('=')
                    .ok_or_else(|| format!("--pinned wants <id>=<path>@<rev>, not `{triple}`"))?;
                // The *last* `@`, so a path holding one still parses; the rev
                // is the fixed-shape half.
                let (path, rev) = rest
                    .rsplit_once('@')
                    .ok_or_else(|| format!("--pinned wants <id>=<path>@<rev>, not `{triple}`"))?;
                if !manifest::is_commit(rev) {
                    return Err(format!(
                        "--pinned {id}: `{rev}` is not a 40-character commit"
                    ));
                }
                args.pinned.insert(
                    id.to_owned(),
                    assemble::Pinned {
                        path: PathBuf::from(path),
                        rev: rev.to_owned(),
                    },
                );
            }
            other => return Err(format!("unrecognised argument `{other}`")),
        }
    }
    if check {
        if args.bundles.is_none() {
            return Err("--check belongs to --bundles".to_owned());
        }
        args.bundles = Some(BundlesMode::Check);
    }
    Ok(args)
}

fn run() -> anyhow::Result<ExitCode> {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("okf-assemble: {message}\n\n{USAGE}");
            return Ok(ExitCode::FAILURE);
        }
    };
    let root = std::env::current_dir()?;
    let manifest_path = root.join("site.toml");

    if args.bootstrap {
        return run_bootstrap(&args, &root, &manifest_path);
    }
    if args.verify_raw {
        return run_verify(&root);
    }

    let manifest = Manifest::load(&manifest_path)?;

    if let Some(mode) = args.bundles {
        return run_bundles(&manifest, &root, mode == BundlesMode::Check);
    }

    let allow = manifest::read_credentials_allow(&root.join("credentials.allow"))?;
    manifest.check_credentials(allow.as_ref())?;

    if let Some(id) = args.update {
        return run_update(manifest, &manifest_path, &id);
    }

    // The scan this build is about to run has to have been armed, and the
    // place to say so is here: `okf-assemble` is step one of every `just
    // build`, so an unarmed tenant goes red before it fetches anything rather
    // than after it has published. The assertion is on *supply* -- a file the
    // caller named, holding terms -- and never on a tracked path, because a
    // roster committed to a site repository is the disclosure the deny list
    // exists to prevent. See `manifest::check_deny_list_supplied`.
    let terms = manifest::check_deny_list_supplied()?;
    println!("scan deny list: {terms} term(s) supplied by the caller.");

    let mut options = Options::new(root);
    options.locals = args.locals;
    options.pinned = args.pinned;
    options.mermaid = args.mermaid;
    options.tarball = args.tarball;
    let outcome = assemble::assemble(&manifest, &options)?;
    println!(
        "assembled {} bundle(s): {} file(s), {} index.md renamed to _index.md, \
         {} file(s) had links rewritten.",
        outcome.bundles, outcome.files, outcome.renamed, outcome.rewritten
    );
    if !outcome.local.is_empty() {
        println!(
            "local build: {} — the manifest was not modified.",
            outcome.local.join(", ")
        );
    }
    if !outcome.pinned.is_empty() {
        println!(
            "pinned source(s) verified against site.toml: {}.",
            outcome.pinned.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Regenerate `nix/bundles.nix`, or with `check` assert it is current.
///
/// The check writes nothing and fetches nothing, which is what lets a
/// derivation with no network run it against a tenant's tracked tree: the
/// tree disagreeing with the tool that produced it is a reproducibility
/// failure, and a hand-edited pin is exactly that.
fn run_bundles(
    manifest: &Manifest,
    root: &std::path::Path,
    check: bool,
) -> anyhow::Result<ExitCode> {
    if check {
        let problems = sitegen::check_bundles_nix(manifest, root)?;
        if problems.is_empty() {
            println!("{} agrees with site.toml.", sitegen::BUNDLES_NIX_PATH);
            return Ok(ExitCode::SUCCESS);
        }
        for problem in problems {
            eprintln!("okf-assemble: {problem}");
        }
        return Ok(ExitCode::FAILURE);
    }
    sitegen::write_bundles_nix(manifest, root)?;
    let opted_in = manifest
        .bundles
        .iter()
        .filter(|b| !b.fetch.is_empty())
        .count();
    if opted_in == 0 {
        println!(
            "no bundle sets fetch; {} is not generated for this tenant.",
            sitegen::BUNDLES_NIX_PATH
        );
    } else {
        println!(
            "wrote {} with {opted_in} bundle(s); commit the diff.",
            sitegen::BUNDLES_NIX_PATH
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Resolve one bundle's `ref` to a commit and write it back.
///
/// The command stops there rather than assembling, which makes a roll-forward
/// a one-line diff somebody reviews instead of drift nobody sees.
fn run_update(
    mut manifest: Manifest,
    path: &std::path::Path,
    id: &str,
) -> anyhow::Result<ExitCode> {
    let Some(bundle) = manifest.bundles.iter_mut().find(|b| b.id == id) else {
        eprintln!("okf-assemble: no bundle `{id}` in {}", path.display());
        return Ok(ExitCode::FAILURE);
    };
    let Some(line) = assemble::resolve_ref(&bundle.repo, &bundle.git_ref) else {
        eprintln!(
            "okf-assemble: {} does not resolve {} at {}",
            id, bundle.git_ref, bundle.repo
        );
        return Ok(ExitCode::FAILURE);
    };
    if line == bundle.rev {
        println!("{id} is already at {line}.");
        return Ok(ExitCode::SUCCESS);
    }
    let was = bundle.rev.clone();
    bundle.rev.clone_from(&line);
    manifest.save(path)?;
    println!("{id}: {was} -> {line}\nReview the diff before building.");
    Ok(ExitCode::SUCCESS)
}

fn run_bootstrap(
    args: &Args,
    root: &std::path::Path,
    manifest_path: &std::path::Path,
) -> anyhow::Result<ExitCode> {
    if !bootstrap::is_empty_manifest(manifest_path) {
        eprintln!(
            "okf-assemble: {} already has bundles. Bootstrap writes a draft and \
             refuses to overwrite a reviewed manifest.",
            manifest_path.display()
        );
        return Ok(ExitCode::FAILURE);
    }
    let scan = args
        .scan
        .clone()
        .or_else(|| root.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| root.to_path_buf());
    let tenant = args
        .tenant
        .clone()
        .or_else(|| root.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .unwrap_or_else(|| "tenant".to_owned());

    let found = bootstrap::discover(&scan);
    let draft = bootstrap::draft(&tenant, found);
    let manifest = Manifest {
        tenant: tenant.clone(),
        bundles: draft.bundles,
        ..Manifest::default()
    };
    manifest.save(manifest_path)?;
    bootstrap::write_deferred(&root.join("deferred.toml"), &draft.deferred)?;
    println!(
        "drafted {} bundle(s) into {} and deferred {} with no remote into deferred.toml.\n\
         Both are drafts. Edit them, then run okf-assemble.",
        manifest.bundles.len(),
        manifest_path.display(),
        draft.deferred.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// The build gate: every rendered `.md` equals the source it came from.
fn run_verify(root: &std::path::Path) -> anyhow::Result<ExitCode> {
    let report = sitegen::verify_raw(root)?;
    if report.is_clean() {
        println!(
            "raw markdown is byte-identical over all {} page(s), \
             and every one of them also rendered HTML.",
            report.compared
        );
        return Ok(ExitCode::SUCCESS);
    }
    if report.compared == 0 && report.missing_raw.is_empty() {
        eprintln!(
            "okf-assemble: nothing to compare. Assemble and run hugo before \
             --verify-raw."
        );
        return Ok(ExitCode::FAILURE);
    }
    for pair in &report.differing {
        eprintln!(
            "okf-assemble: public/{}/index.md differs from content/{}",
            pair.rendered, pair.source
        );
    }
    for path in &report.missing_raw {
        eprintln!("okf-assemble: content/{path} rendered no raw markdown");
    }
    for path in &report.missing_html {
        eprintln!("okf-assemble: content/{path} rendered no HTML page");
    }
    for path in &report.unmatched {
        eprintln!("okf-assemble: public/{path} has no source in content/");
    }
    Ok(ExitCode::FAILURE)
}
