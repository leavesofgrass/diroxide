//! curl and wget as the tools registry finds them on a test PATH: the
//! stand-in program records what it was given, so these tests check the
//! argument lists that really reach the program, that a token travels on
//! standard input only, and the order of the transports. Nothing here
//! opens a connection: the stand-in never does, and the built-in client
//! is left out.

#![cfg(feature = "fake-tools")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use diroxide_components::{FetchError, Fetcher, Request, StandardFetcher, USER_AGENT};
use diroxide_core::Secret;
use diroxide_tools::ToolRegistry;
use diroxide_vfs::CancelToken;

const ADDRESS: &str = "https://example.invalid/diro-x-1.0/a.zip";

/// A folder with the stand-in copied in as each of `names`.
fn tools(names: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for name in names {
        let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        fs::copy(env!("CARGO_BIN_EXE_diroxide-fake-tool"), bin.join(file)).unwrap();
    }
    (dir, bin)
}

fn fetcher(bin: &Path) -> StandardFetcher {
    let registry = ToolRegistry::builtin().with_path(Some(bin.as_os_str().to_owned()));
    StandardFetcher::from_tools(&registry).without_builtin()
}

fn request<'a>(cancel: &'a CancelToken, token: Option<&'a Secret>) -> Request<'a> {
    Request {
        address: ADDRESS,
        from: 0,
        if_range: None,
        limit: 1000,
        token,
        cancel,
    }
}

fn args(bin: &Path, name: &str) -> Option<Vec<String>> {
    fs::read_to_string(bin.join(format!("{name}-args.txt")))
        .ok()
        .map(|t| t.lines().map(str::to_owned).collect())
}

fn read_all(fetcher: &dyn Fetcher, request: &Request<'_>) -> Result<Vec<u8>, FetchError> {
    let mut bytes = Vec::new();
    fetcher
        .open(request)?
        .reader
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::from_io(&e))?;
    Ok(bytes)
}

#[test]
fn curl_gets_the_safe_list_and_its_bytes_arrive() {
    let (_dir, bin) = tools(&["curl"]);
    fs::write(bin.join("curl-stdout.bin"), b"zip bytes").unwrap();
    let fetcher = fetcher(&bin);
    assert_eq!(fetcher.transports(), ["curl"]);
    let cancel = CancelToken::new();
    assert_eq!(
        read_all(&fetcher, &request(&cancel, None)).unwrap(),
        b"zip bytes"
    );
    let seen = args(&bin, "curl").unwrap();
    assert_eq!(
        seen,
        [
            "-q",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "30",
            "--speed-limit",
            "1000",
            "--speed-time",
            "60",
            "--max-filesize",
            "1000",
            "--user-agent",
            USER_AGENT,
            "--no-buffer",
            "--output",
            "-",
            "--url",
            ADDRESS,
        ]
    );
    assert_eq!(fs::read(bin.join("curl-stdin.txt")).unwrap(), b"");
}

#[test]
fn a_token_reaches_curl_on_standard_input_only() {
    let (_dir, bin) = tools(&["curl"]);
    fs::write(bin.join("curl-stdout.bin"), b"x").unwrap();
    let token = Secret::new("t0ken-123".to_owned());
    let cancel = CancelToken::new();
    read_all(&fetcher(&bin), &request(&cancel, Some(&token))).unwrap();
    let seen = args(&bin, "curl").unwrap();
    assert!(seen.iter().all(|a| !a.contains("t0ken")), "{seen:?}");
    assert!(seen.windows(2).any(|w| w == ["--config", "-"]));
    assert_eq!(
        fs::read_to_string(bin.join("curl-stdin.txt")).unwrap(),
        "header = \"Authorization: Bearer t0ken-123\"\n"
    );
}

#[test]
fn wget_tries_when_curl_cannot_connect_and_not_when_the_server_answered() {
    let (_dir, bin) = tools(&["curl", "wget"]);
    fs::write(bin.join("curl-exit.txt"), "7").unwrap();
    fs::write(bin.join("wget-stdout.bin"), b"from wget").unwrap();
    let fetcher = fetcher(&bin);
    assert_eq!(fetcher.transports(), ["curl", "wget"]);
    let cancel = CancelToken::new();
    assert_eq!(
        read_all(&fetcher, &request(&cancel, None)).unwrap(),
        b"from wget"
    );
    let wget = args(&bin, "wget").unwrap();
    assert_eq!(wget[0], "--no-config");
    assert_eq!(wget.last().unwrap(), ADDRESS);
    fs::remove_file(bin.join("wget-args.txt")).unwrap();
    // "Not found" is the server's answer: final, wget is not asked.
    fs::write(bin.join("curl-exit.txt"), "22").unwrap();
    fs::write(
        bin.join("curl-stderr.txt"),
        "curl: (22) The requested URL returned error: 404",
    )
    .unwrap();
    assert_eq!(
        read_all(&fetcher, &request(&cancel, None)).unwrap_err(),
        FetchError::Missing
    );
    assert_eq!(args(&bin, "wget"), None);
    // A private source never goes to wget.
    fs::write(bin.join("curl-exit.txt"), "7").unwrap();
    let token = Secret::new("t".to_owned());
    assert_eq!(
        read_all(&fetcher, &request(&cancel, Some(&token))).unwrap_err(),
        FetchError::Connect
    );
    assert_eq!(args(&bin, "wget"), None);
}

#[test]
fn with_neither_program_the_answer_names_curl() {
    let (_dir, bin) = tools(&[]);
    let fetcher = fetcher(&bin);
    assert!(!fetcher.can_download());
    let cancel = CancelToken::new();
    let error = read_all(&fetcher, &request(&cancel, None)).unwrap_err();
    assert_eq!(error, FetchError::NoClient);
    assert_eq!(error.reason(), "curl was not found");
}
