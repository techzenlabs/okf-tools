//! Serve an assembled site, and refuse to serve it unidentified.
//!
//! See [`okf_tools::serve`] for why the refusal lives in the process rather
//! than in the proxy in front of it.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use okf_tools::serve::{Resolution, content_type, identified, resolve};
use tiny_http::{Header, Request, Response, Server, StatusCode};

const USAGE: &str = "usage: okf-serve --root <dir> [--bind <addr>] [--port <n>]\n\
     \x20                [--require-header <name>] [--threads <n>]\n\n\
     Serves the files under <dir> and nothing else. A directory resolves to\n\
     its index.html or to 404; it is never a listing.\n\n\
     --require-header makes an identity header mandatory: a request without\n\
     it gets 401 and never touches the filesystem. That is the Entra\n\
     boundary — `--require-header X-MS-CLIENT-PRINCIPAL` behind App Service\n\
     Easy Auth — and it is what stops the origin serving anything if the\n\
     proxy is ever bypassed. Without the flag the bind address is the\n\
     boundary, which is the tailnet deployment.";

/// How much of a request head is read before it is refused.
///
/// `tiny_http` enforces its own limits; this is the header map this process
/// keeps, and an unbounded one is a way to spend memory on a request that
/// will be answered with 401 anyway.
const MAX_HEADERS: usize = 64;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("okf-serve: {err}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    root: PathBuf,
    bind: IpAddr,
    port: u16,
    require_header: Option<String>,
    threads: usize,
}

fn run() -> anyhow::Result<ExitCode> {
    let Some(options) = parse()? else {
        return Ok(ExitCode::FAILURE);
    };

    // Fail closed on the two things that would otherwise be discovered by a
    // reader. A root that is not there serves 404 to everything and looks
    // like a deploy that worked.
    if !options.root.is_dir() {
        anyhow::bail!(
            "--root {} is not a directory; refusing to serve nothing at all",
            options.root.display()
        );
    }
    let root = options.root.canonicalize()?;

    let address = SocketAddr::new(options.bind, options.port);
    let server = Server::http(address)
        .map_err(|err| anyhow::anyhow!("cannot listen on {address}: {err}"))?;
    let bound = server
        .server_addr()
        .to_ip()
        .map_or_else(|| address.to_string(), |addr| addr.to_string());

    let missing = std::fs::read(root.join("404.html")).ok();
    match &options.require_header {
        Some(name) => eprintln!(
            "okf-serve: {} on http://{bound}/, refusing any request without {name}",
            root.display()
        ),
        None => eprintln!(
            "okf-serve: {} on http://{bound}/, no identity required — the bind address is the boundary",
            root.display()
        ),
    }

    let shared = Arc::new(Shared {
        root,
        required: options.require_header,
        missing,
        server,
    });

    let mut workers = Vec::new();
    for _ in 1..options.threads {
        let shared = Arc::clone(&shared);
        workers.push(std::thread::spawn(move || shared.serve_forever()));
    }
    shared.serve_forever();
    for worker in workers {
        drop(worker.join());
    }
    Ok(ExitCode::SUCCESS)
}

struct Shared {
    root: PathBuf,
    required: Option<String>,
    /// The site's own 404 page, read once at startup. A site that ships one
    /// gets it; a site that does not gets a sentence.
    missing: Option<Vec<u8>>,
    server: Server,
}

impl Shared {
    fn serve_forever(&self) {
        while let Ok(request) = self.server.recv() {
            let method = request.method().as_str().to_owned();
            let url = request.url().to_owned();
            let status = self.answer(request);
            eprintln!("{status} {method} {url}");
        }
    }

    fn answer(&self, request: Request) -> u16 {
        let head_only = request.method() == &tiny_http::Method::Head;
        if !matches!(
            request.method(),
            tiny_http::Method::Get | tiny_http::Method::Head
        ) {
            return send(
                request,
                405,
                "text/plain; charset=utf-8",
                b"method not allowed",
                false,
            );
        }

        // Before the filesystem, deliberately. An unidentified request never
        // resolves a path, so nothing downstream of here is reachable without
        // an identity.
        let mut headers = BTreeMap::new();
        for header in request.headers().iter().take(MAX_HEADERS) {
            headers.insert(
                header.field.as_str().as_str().to_owned(),
                header.value.as_str().to_owned(),
            );
        }
        if !identified(&headers, self.required.as_deref()) {
            return send(
                request,
                401,
                "text/plain; charset=utf-8",
                b"unauthenticated",
                false,
            );
        }

        let url = request.url().to_owned();
        match resolve(&self.root, &url) {
            Resolution::File(path) => match std::fs::read(&path) {
                Ok(body) => {
                    let kind = content_type(&path);
                    send(request, 200, kind, &body, head_only)
                }
                Err(_) => self.not_found(request, head_only),
            },
            // A refusal gets the same answer as a missing file, deliberately:
            // a path that tried to leave the root learns nothing about
            // whether what it reached for exists.
            Resolution::Missing | Resolution::Refused => self.not_found(request, head_only),
        }
    }

    fn not_found(&self, request: Request, head_only: bool) -> u16 {
        match &self.missing {
            Some(body) => send(request, 404, "text/html; charset=utf-8", body, head_only),
            None => send(
                request,
                404,
                "text/plain; charset=utf-8",
                b"not found",
                head_only,
            ),
        }
    }
}

fn send(request: Request, status: u16, kind: &str, body: &[u8], head_only: bool) -> u16 {
    let empty: &[u8] = b"";
    let payload = if head_only { empty } else { body };
    let mut response = Response::from_data(payload).with_status_code(StatusCode(status));
    for (name, value) in [
        ("Content-Type", kind),
        // A docs site is rebuilt by changing a pinned commit and restarting
        // the unit, so a cached page is a page from a different build. Cheap
        // to revalidate on a tailnet; wrong to keep.
        ("Cache-Control", "no-cache"),
        ("X-Content-Type-Options", "nosniff"),
        // The pages carry no frames and no cross-origin embeds, and the
        // corpus is a client's documentation.
        ("Referrer-Policy", "no-referrer"),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    drop(request.respond(response));
    status
}

fn parse() -> anyhow::Result<Option<Options>> {
    let mut root: Option<PathBuf> = None;
    let mut bind: IpAddr = IpAddr::from([127, 0, 0, 1]);
    let mut port: u16 = 8080;
    let mut require_header: Option<String> = None;
    let mut threads: usize = 8;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || -> anyhow::Result<String> {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{arg} needs a value\n\n{USAGE}"))
        };
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(value()?)),
            "--bind" => bind = value()?.parse()?,
            "--port" => port = value()?.parse()?,
            "--threads" => threads = value()?.parse()?,
            "--require-header" => {
                let name = value()?;
                if name.trim().is_empty() {
                    anyhow::bail!("--require-header needs a header name, not an empty string");
                }
                require_header = Some(name);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => {
                eprintln!("okf-serve: unrecognised argument `{other}`\n\n{USAGE}");
                return Ok(None);
            }
        }
    }

    let Some(root) = root else {
        eprintln!("okf-serve: --root is required\n\n{USAGE}");
        return Ok(None);
    };
    if threads == 0 {
        anyhow::bail!("--threads must be at least 1");
    }
    Ok(Some(Options {
        root: Path::new(&root).to_path_buf(),
        bind,
        port,
        require_header,
        threads,
    }))
}
