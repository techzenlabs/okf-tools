//! `okf-serve` end to end: the binary, a socket, and real requests.
//!
//! The unit tests in `src/serve.rs` cover path resolution and the header
//! predicate. What they cannot cover is the thing this binary exists for —
//! that an unauthenticated request to the *origin* is refused by the process.
//! That is a property of a running server answering a real socket, and
//! asserting it any other way asserts a function rather than a deployment.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a panicking assertion is the point of a test"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A running `okf-serve`, killed when the test drops it.
struct Serving {
    child: Child,
    port: u16,
    root: PathBuf,
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A port nobody else holds, found by binding and letting go.
///
/// Racy against other processes in principle and not in practice, and the
/// alternative — parsing the bound address out of the child's stderr — is
/// racier, because the log line arrives after the listener does and a test
/// that reads it can hang.
///
/// Racy against *this* process in practice, measured: these tests run in
/// parallel threads, and after one thread drops its listener the kernel can
/// hand the same port to the next thread's `bind(:0)`. Both children then
/// race for one port, the loser exits, both readiness probes connect to the
/// winner, and the losing test fails mid-flight with `ConnectionRefused` (the
/// winner's test finished and killed it) or `ConnectionReset` (killed
/// mid-request). Seen both ways before this set was added: a port is never
/// handed out twice within the process.
fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static CLAIMED: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    let claimed = CLAIMED.get_or_init(|| Mutex::new(HashSet::new()));
    loop {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        if claimed.lock().unwrap().insert(port) {
            return port;
        }
    }
}

fn binary() -> PathBuf {
    // The integration test binary sits beside the ones cargo built.
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("okf-serve")
}

fn child_is_ready(
    child: &mut Child,
    port: u16,
    expected_body: &str,
    identity_header: Option<&str>,
) -> bool {
    endpoint_is_ready(port, expected_body, identity_header, || {
        matches!(child.try_wait(), Ok(None))
    })
}

fn endpoint_is_ready(
    port: u16,
    expected_body: &str,
    identity_header: Option<&str>,
    mut child_is_running: impl FnMut() -> bool,
) -> bool {
    if !child_is_running() {
        return false;
    }

    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let timeout = Some(std::time::Duration::from_secs(1));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let mut request =
        String::from("GET /.okf-ready.txt HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(header) = identity_header {
        request.push_str(header);
        request.push_str(": readiness\r\n");
    }
    request.push_str("\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    let answered = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        == Some("200")
        && body == expected_body;
    answered && child_is_running()
}

fn start(label: &str, extra: &[&str]) -> Serving {
    let root = std::env::temp_dir().join(format!("okf-serve-e2e-{label}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("section")).unwrap();
    std::fs::create_dir_all(root.join("listing")).unwrap();
    std::fs::write(root.join("index.html"), "<h1>home</h1>").unwrap();
    std::fs::write(root.join("404.html"), "<h1>Page not found</h1>").unwrap();
    std::fs::write(root.join("section/index.html"), "<h1>section</h1>").unwrap();
    std::fs::write(root.join("listing/secret.html"), "<h1>not linked</h1>").unwrap();
    std::fs::write(root.join("style.css"), "body{}").unwrap();
    let readiness_body = format!("okf-serve-ready:{label}");
    std::fs::write(root.join(".okf-ready.txt"), &readiness_body).unwrap();
    let identity_header = extra
        .windows(2)
        .find_map(|args| (args[0] == "--require-header").then_some(args[1]));

    let port = free_port();
    let mut args: Vec<String> = vec![
        "--root".into(),
        root.display().to_string(),
        "--port".into(),
        port.to_string(),
        "--threads".into(),
        "2".into(),
    ];
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    let child = Command::new(binary())
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut serving = Serving { child, port, root };
    // Wait for the listener rather than sleeping a guess.
    for _ in 0..200 {
        if child_is_ready(&mut serving.child, port, &readiness_body, identity_header) {
            return serving;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("okf-serve did not start listening on {port}");
}

#[test]
fn a_transient_listener_is_not_child_readiness() {
    let mut serving = start("readiness", &[]);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let transient_port = listener.local_addr().unwrap().port();

    assert!(!child_is_ready(
        &mut serving.child,
        transient_port,
        "okf-serve-ready:readiness",
        None,
    ));
}

#[test]
fn an_answer_is_not_readiness_after_the_child_exits() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut child = Command::new("sh")
        .args(["-c", "read _"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let child_input = child.stdin.take().unwrap();
    let child_is_running = Arc::new(AtomicBool::new(true));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let responder_child_is_running = Arc::clone(&child_is_running);
    let responder = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        {
            let mut reader = BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
        }
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready");
        drop(child_input);
        child.wait().unwrap();
        responder_child_is_running.store(false, Ordering::Release);
    });

    let ready = endpoint_is_ready(port, "ready", None, || {
        child_is_running.load(Ordering::Acquire)
    });
    responder.join().unwrap();
    assert!(!ready);
}

struct Answer {
    status: u16,
    headers: Vec<String>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        let wanted = format!("{}:", name.to_ascii_lowercase());
        self.headers
            .iter()
            .find(|line| line.to_ascii_lowercase().starts_with(&wanted))
            .and_then(|line| line.split_once(':'))
            .map(|(_, value)| value.trim())
    }
}

fn request(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        use std::fmt::Write as _;
        let _ = writeln!(head, "{name}: {value}\r");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        headers.push(line);
    }
    let mut body = Vec::new();
    reader.read_to_end(&mut body).unwrap();
    Answer {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

/// The tailnet deployment: no flag, and the bind address is the boundary.
#[test]
fn without_the_flag_it_serves() {
    let s = start("open", &[]);
    let home = request(s.port, "GET", "/", &[]);
    assert_eq!(home.status, 200);
    assert!(home.body.contains("home"), "{}", home.body);
    assert_eq!(
        home.header("Content-Type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(home.header("X-Content-Type-Options"), Some("nosniff"));

    let css = request(s.port, "GET", "/style.css", &[]);
    assert_eq!(css.status, 200);
    assert_eq!(css.header("Content-Type"), Some("text/css; charset=utf-8"));

    let section = request(s.port, "GET", "/section/", &[]);
    assert_eq!(section.status, 200);
    assert!(section.body.contains("section"));
}

/// **The property the binary exists for.** An unauthenticated request to the
/// origin is refused because the process refuses it, not because a proxy
/// happened to be in front. Asserted against a running socket, because that
/// is the only place the claim is about.
#[test]
fn with_the_flag_an_unidentified_request_is_refused() {
    let s = start("gated", &["--require-header", "X-MS-CLIENT-PRINCIPAL"]);

    for path in ["/", "/index.html", "/section/", "/style.css", "/404.html"] {
        let answer = request(s.port, "GET", path, &[]);
        assert_eq!(answer.status, 401, "{path} served without an identity");
        assert!(
            !answer.body.contains("home") && !answer.body.contains("section"),
            "the 401 body leaked page content for {path}"
        );
    }

    // An empty value is not an identity. A proxy that strips the value and
    // leaves the name behind has authenticated nobody.
    let blank = request(s.port, "GET", "/", &[("X-MS-CLIENT-PRINCIPAL", "   ")]);
    assert_eq!(blank.status, 401);

    // And with one, the same server serves.
    let ok = request(
        s.port,
        "GET",
        "/",
        &[("x-ms-client-principal", "eyJ1c2VyIjoiZXhhbXBsZSJ9")],
    );
    assert_eq!(ok.status, 200);
    assert!(ok.body.contains("home"));
}

/// A directory is never a listing, even when it holds files. `okf-assemble`
/// writes an `_index.md` for every section, so a directory with no
/// `index.html` is a build that went wrong — and naming its contents to a
/// reader is not how to say so.
#[test]
fn a_directory_without_an_index_is_not_a_listing() {
    let s = start("listing", &[]);
    let answer = request(s.port, "GET", "/listing/", &[]);
    assert_eq!(answer.status, 404);
    assert!(
        !answer.body.contains("secret"),
        "the directory listed its contents: {}",
        answer.body
    );
    assert!(answer.body.contains("Page not found"));
}

/// The site's own 404 page, with the status that makes it one. Hugo publishes
/// no `/404.html` unless a layout exists, so a site that has one has it on
/// purpose.
#[test]
fn a_missing_path_gets_the_site_s_own_404_page_and_a_404_status() {
    let s = start("notfound", &[]);
    let answer = request(s.port, "GET", "/nowhere/at/all/", &[]);
    assert_eq!(answer.status, 404);
    assert!(answer.body.contains("Page not found"));
}

/// Traversal, and the answer being indistinguishable from a miss. A path that
/// tried to leave the root learns nothing about whether what it reached for
/// is there.
#[test]
fn traversal_is_refused_and_looks_exactly_like_a_miss() {
    let s = start("traversal", &[]);
    let outside = s.root.parent().map(Path::to_path_buf).unwrap();
    std::fs::write(outside.join("okf-serve-e2e-target.txt"), "not yours").unwrap();

    for path in [
        "/../okf-serve-e2e-target.txt",
        "/section/../../okf-serve-e2e-target.txt",
        "/%2e%2e/okf-serve-e2e-target.txt",
    ] {
        let answer = request(s.port, "GET", path, &[]);
        assert_eq!(answer.status, 404, "{path}");
        assert!(
            !answer.body.contains("not yours"),
            "{path} escaped the root"
        );
    }
    let _ = std::fs::remove_file(outside.join("okf-serve-e2e-target.txt"));
}

/// HEAD answers with the headers and no body; anything that is not GET or
/// HEAD is refused rather than guessed at.
#[test]
fn head_carries_no_body_and_a_post_is_refused() {
    let s = start("methods", &[]);
    let head = request(s.port, "HEAD", "/", &[]);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty(), "HEAD returned a body: {}", head.body);

    let post = request(s.port, "POST", "/", &[]);
    assert_eq!(post.status, 405);
}

/// Fail closed on a root that is not there. Serving 404 to everything looks
/// exactly like a deploy that worked.
#[test]
fn a_root_that_is_not_a_directory_refuses_to_start() {
    let output = Command::new(binary())
        .args(["--root", "/nonexistent/okf-serve", "--port", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is not a directory"),
        "did not say why: {stderr}"
    );
}
