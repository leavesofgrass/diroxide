//! The transports against the loopback server: the real curl when the
//! tools registry finds one (spike S4), and the built-in client with the
//! `fetch` feature. Nothing here goes past 127.0.0.1.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use diroxide_core::Secret;
use diroxide_tools::{ToolRegistry, ToolStatus};
use diroxide_vfs::CancelToken;

use crate::USER_AGENT;
use crate::curl::CurlFetcher;
use crate::fetch::{FetchError, Fetcher, Request};
use crate::loopback::{Mode, Server};

/// Some bytes that are not all alike.
pub(crate) fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn request<'a>(address: &'a str, from: u64, cancel: &'a CancelToken) -> Request<'a> {
    Request {
        address,
        from,
        if_range: None,
        limit: 1 << 20,
        token: None,
        cancel,
    }
}

/// Opens and reads to the end: where the bytes start and what came.
fn fetch_all(fetcher: &dyn Fetcher, request: &Request<'_>) -> Result<(u64, Vec<u8>), FetchError> {
    let fetched = fetcher.open(request)?;
    let mut bytes = Vec::new();
    let mut reader = fetched.reader;
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::from_io(&e))?;
    Ok((fetched.start, bytes))
}

fn real_curl() -> Option<PathBuf> {
    match ToolRegistry::builtin().find("curl") {
        Ok(ToolStatus::Found(path)) => Some(path),
        _ => {
            eprintln!("curl is not on PATH here; the curl transport tests are skipped.");
            None
        }
    }
}

/// A loopback address where nothing listens.
fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/nothing.bin")
}

#[test]
fn curl_fetches_and_resumes_with_the_neutral_user_agent() {
    let Some(curl) = real_curl() else { return };
    let server = Server::start();
    let bytes = sample(10_000);
    server.serve("f.bin", &bytes, Mode::Honor);
    let curl = CurlFetcher::for_loopback(curl);
    let cancel = CancelToken::new();
    let url = server.url("f.bin");
    assert_eq!(
        fetch_all(&curl, &request(&url, 0, &cancel)).unwrap(),
        (0, bytes.clone())
    );
    let (start, rest) = fetch_all(&curl, &request(&url, 4000, &cancel)).unwrap();
    assert_eq!(start, 4000);
    assert_eq!(rest, bytes[4000..]);
    let seen = server.seen();
    assert_eq!(seen[1].range.as_deref(), Some("bytes=4000-"));
    for request in &seen {
        assert_eq!(request.header("user-agent"), Some(USER_AGENT));
    }
}

#[test]
fn curl_never_reads_a_planted_configuration() {
    let Some(program) = real_curl() else { return };
    let server = Server::start();
    server.serve("f.bin", b"abc", Mode::Honor);
    let home = tempfile::tempdir().unwrap();
    let planted = "header = \"X-Planted: yes\"\nuser-agent = \"planted\"\n";
    std::fs::write(home.path().join(".curlrc"), planted).unwrap();
    std::fs::write(home.path().join("_curlrc"), planted).unwrap();
    // Without -q, curl reads it: the plant works.
    let status = Command::new(&program)
        .args(["--silent", "--output"])
        .arg(home.path().join("out.bin"))
        .arg("--url")
        .arg(server.url("f.bin"))
        .env("CURL_HOME", home.path())
        .env_remove("http_proxy")
        .env_remove("HTTP_PROXY")
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let planted_seen = server.seen()[0].header("x-planted").is_some();
    // With the fetcher's list, -q first, it is never read.
    let curl = CurlFetcher::for_loopback(program).with_env("CURL_HOME", home.path());
    let cancel = CancelToken::new();
    let url = server.url("f.bin");
    fetch_all(&curl, &request(&url, 0, &cancel)).unwrap();
    let ours = server.seen().pop().unwrap();
    assert_eq!(ours.header("x-planted"), None);
    assert_eq!(ours.header("user-agent"), Some(USER_AGENT));
    assert!(
        planted_seen,
        "the planted .curlrc was not read even without -q, so this proves nothing"
    );
}

#[test]
fn curl_refuses_a_redirect_to_plain_http() {
    let Some(curl) = real_curl() else { return };
    let server = Server::start();
    server.serve("f.bin", b"abc", Mode::Honor);
    server.serve("moved.bin", b"", Mode::Redirect(server.url("f.bin")));
    let curl = CurlFetcher::for_loopback(curl);
    let cancel = CancelToken::new();
    let url = server.url("moved.bin");
    let error = fetch_all(&curl, &request(&url, 0, &cancel)).unwrap_err();
    assert!(matches!(error, FetchError::Refused(_)), "{error:?}");
    assert_eq!(server.seen().len(), 1, "the redirect was not followed");
}

#[test]
fn curl_says_when_the_server_cannot_resume() {
    let Some(curl) = real_curl() else { return };
    let server = Server::start();
    let bytes = sample(5000);
    server.serve("ignore.bin", &bytes, Mode::IgnoreRange);
    server.serve("wrong.bin", &bytes, Mode::WrongRange);
    server.serve("whole.bin", &bytes, Mode::Honor);
    let curl = CurlFetcher::for_loopback(curl);
    let cancel = CancelToken::new();
    // A server that ignores the range answers 200 and one that answers
    // from the wrong byte answers 206: curl stops with code 33 for both,
    // before a byte is written.
    for name in ["ignore.bin", "wrong.bin"] {
        let url = server.url(name);
        let error = fetch_all(&curl, &request(&url, 1000, &cancel)).unwrap_err();
        assert_eq!(error, FetchError::CannotResume, "{name}");
    }
    // A 416 for a resume from the very end is "already whole" to curl:
    // no bytes, exit 0. The downloader never asks for that (a full .part
    // is hashed first), and the hash decides if it ever happens.
    let url = server.url("whole.bin");
    assert_eq!(
        fetch_all(&curl, &request(&url, 5000, &cancel)).unwrap(),
        (5000, Vec::new())
    );
}

#[test]
fn curl_exit_codes_match_the_sentences() {
    let Some(curl) = real_curl() else { return };
    let server = Server::start();
    let bytes = sample(5000);
    server.serve("cut.bin", &bytes, Mode::Truncate(1000));
    server.serve("long.bin", &bytes, Mode::Longer(vec![1; 100]));
    server.serve("gone.bin", &bytes, Mode::NotFound);
    let curl = CurlFetcher::for_loopback(curl);
    let cancel = CancelToken::new();
    for name in ["missing.bin", "gone.bin"] {
        let missing = server.url(name);
        assert_eq!(
            fetch_all(&curl, &request(&missing, 0, &cancel)).unwrap_err(),
            FetchError::Missing
        );
    }
    assert!(server.seen().iter().any(|seen| seen.path == "/gone.bin"));
    let closed = closed_port();
    assert_eq!(
        fetch_all(&curl, &request(&closed, 0, &cancel)).unwrap_err(),
        FetchError::Connect
    );
    let cut = server.url("cut.bin");
    assert_eq!(
        fetch_all(&curl, &request(&cut, 0, &cancel)).unwrap_err(),
        FetchError::Lost
    );
    let long = server.url("long.bin");
    let mut capped = request(&long, 0, &cancel);
    capped.limit = 5000;
    assert_eq!(fetch_all(&curl, &capped).unwrap_err(), FetchError::TooLarge);
}

#[test]
fn curl_takes_a_token_on_standard_input_only() {
    let Some(program) = real_curl() else { return };
    let server = Server::start();
    server.serve("private.bin", b"abc", Mode::Honor);
    let curl = CurlFetcher::for_loopback(program);
    let token = Secret::new("t0ken-for-the-test".to_owned());
    let cancel = CancelToken::new();
    let url = server.url("private.bin");
    let mut private = request(&url, 0, &cancel);
    private.token = Some(&token);
    assert!(
        curl.arguments(&private)
            .iter()
            .all(|a| !a.to_string_lossy().contains("t0ken"))
    );
    fetch_all(&curl, &private).unwrap();
    assert_eq!(
        server.seen()[0].header("authorization"),
        Some("Bearer t0ken-for-the-test")
    );
}

#[test]
fn a_stop_kills_curl_at_once() {
    let Some(curl) = real_curl() else { return };
    let server = Server::start();
    server.serve("slow.bin", &sample(100_000), Mode::Stall(20_000));
    let curl = CurlFetcher::for_loopback(curl);
    let cancel = CancelToken::new();
    let url = server.url("slow.bin");
    let fetched = curl.open(&request(&url, 0, &cancel)).unwrap();
    let stopper = cancel.clone();
    let started = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stopper.cancel();
    });
    let mut sink = Vec::new();
    let mut reader = fetched.reader;
    let result = reader.read_to_end(&mut sink);
    assert!(
        result.is_err(),
        "the read ends with an error, not a short file"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[cfg(feature = "fetch")]
mod builtin {
    use super::*;
    use crate::http::HttpFetcher;

    #[test]
    fn the_builtin_client_fetches_resumes_and_says_why_not() {
        let server = Server::start();
        let bytes = sample(8000);
        server.serve("f.bin", &bytes, Mode::Honor);
        server.serve("ignore.bin", &bytes, Mode::IgnoreRange);
        server.serve("wrong.bin", &bytes, Mode::WrongRange);
        server.serve("long.bin", &bytes, Mode::Longer(vec![0; 10]));
        let http = HttpFetcher::for_loopback();
        let cancel = CancelToken::new();
        let url = server.url("f.bin");
        assert_eq!(
            fetch_all(&http, &request(&url, 0, &cancel)).unwrap(),
            (0, bytes.clone())
        );
        let mut resumed = request(&url, 3000, &cancel);
        resumed.if_range = Some("\"v1\"");
        let fetched = http.open(&resumed).unwrap();
        assert_eq!(fetched.start, 3000);
        assert_eq!(fetched.validator.as_deref(), Some("\"v1\""));
        let seen = server.seen();
        assert_eq!(seen[1].if_range.as_deref(), Some("\"v1\""));
        assert_eq!(seen[1].header("user-agent"), Some(USER_AGENT));
        // A changed file: If-Range does not match, the whole file comes.
        let mut stale = request(&url, 3000, &cancel);
        stale.if_range = Some("\"v0\"");
        assert_eq!(fetch_all(&http, &stale).unwrap(), (0, bytes.clone()));
        let next = sample(8001);
        server.serve("changes.bin", &bytes, Mode::ChangesAfterFirst(next.clone()));
        let changes = server.url("changes.bin");
        let first = http.open(&request(&changes, 0, &cancel)).unwrap();
        assert_eq!(first.validator.as_deref(), Some("\"v1\""));
        drop(first);
        let mut again = request(&changes, 3000, &cancel);
        again.if_range = Some("\"v1\"");
        assert_eq!(fetch_all(&http, &again).unwrap(), (0, next));
        let ignore = server.url("ignore.bin");
        assert_eq!(
            fetch_all(&http, &request(&ignore, 3000, &cancel))
                .unwrap()
                .0,
            0
        );
        let wrong = server.url("wrong.bin");
        assert_eq!(
            fetch_all(&http, &request(&wrong, 3000, &cancel)).unwrap_err(),
            FetchError::CannotResume
        );
        assert_eq!(
            fetch_all(&http, &request(&url, 8000, &cancel)).unwrap_err(),
            FetchError::CannotResume
        );
        let missing = server.url("missing.bin");
        assert_eq!(
            fetch_all(&http, &request(&missing, 0, &cancel)).unwrap_err(),
            FetchError::Missing
        );
        let long = server.url("long.bin");
        let mut capped = request(&long, 0, &cancel);
        capped.limit = 8000;
        assert_eq!(fetch_all(&http, &capped).unwrap_err(), FetchError::TooLarge);
        let closed = closed_port();
        assert_eq!(
            fetch_all(&http, &request(&closed, 0, &cancel)).unwrap_err(),
            FetchError::Connect
        );
    }

    #[test]
    fn the_builtin_client_refuses_plain_http() {
        let cancel = CancelToken::new();
        let error = HttpFetcher::new()
            .open(&request("http://127.0.0.1:9/a.bin", 0, &cancel))
            .unwrap_err();
        assert!(matches!(error, FetchError::Refused(_)), "{error:?}");
    }
}
