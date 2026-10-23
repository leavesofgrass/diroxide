//! curl and wget from PATH: the transports when the built-in client is
//! off or cannot reach the server (answers 40 and 44).
//!
//! Each runs with an argument list, no shell and no console window, its
//! standard output read as the file's bytes and its standard error kept
//! for the reason. curl's list starts with `-q`, so a planted `.curlrc`
//! (one that says `--insecure`, say) is never read, then the safe
//! arguments of the tools registry (fail on HTTP errors, quiet, redirects
//! over `https` only), at most five redirects, a 30-second connect limit,
//! a speed floor (slower than 1,000 bytes a second for 60 seconds stops
//! it), the pin as the size ceiling, the project's neutral User-Agent, and
//! `--continue-at` with the exact byte to resume from. A private source's
//! token goes in as a configuration line on standard input
//! (`--config -`), never as an argument. wget gets `--no-config` first,
//! `https` only, and never a token; it always fetches the whole file.
//!
//! The exit codes become sentences ([`curl_error`], [`wget_error`]); the
//! table was checked against curl 8 on Windows 11 and the loopback server
//! (spike S4 in the W4c report).

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_tools::CURL_SAFE_ARGS;

use crate::USER_AGENT;
use crate::fetch::{FetchError, Fetched, Fetcher, Request, primed};

/// The most bytes of a program's standard error kept for its reason.
const STDERR_KEEP: usize = 8 << 10;

/// curl from PATH.
#[derive(Clone, Debug)]
pub struct CurlFetcher {
    program: PathBuf,
    /// Plain `http` too: only for the loopback server in tests.
    allow_http: bool,
    /// Environment changes for the child: only the tests' `CURL_HOME`.
    env: Vec<(OsString, Option<OsString>)>,
}

impl CurlFetcher {
    /// curl at `program`, as the tools registry found it.
    pub fn new(program: PathBuf) -> Self {
        Self {
            program,
            allow_http: false,
            env: Vec::new(),
        }
    }

    /// curl allowed to fetch plain `http` from the loopback server, with
    /// the proxy variables cleared: tests only.
    #[cfg(test)]
    pub(crate) fn for_loopback(program: PathBuf) -> Self {
        let mut fetcher = Self::new(program);
        fetcher.allow_http = true;
        for name in [
            "http_proxy",
            "HTTP_PROXY",
            "https_proxy",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "all_proxy",
        ] {
            fetcher.env.push((name.into(), None));
        }
        fetcher
    }

    /// The same with `name` set to `value` for the child: tests only.
    #[cfg(test)]
    pub(crate) fn with_env(mut self, name: &str, value: &std::path::Path) -> Self {
        self.env.push((name.into(), Some(value.into())));
        self
    }

    /// The argument list for one request, `-q` first. The token is never
    /// in it: with a token, `--config -` reads it from standard input.
    pub fn arguments(&self, request: &Request<'_>) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        let mut safe = CURL_SAFE_ARGS.iter().peekable();
        while let Some(arg) = safe.next() {
            args.push((*arg).into());
            if *arg == "--proto" && self.allow_http {
                safe.next();
                args.push("=http,https".into());
            }
        }
        let numbers: [(&str, String); 5] = [
            ("--max-redirs", "5".to_owned()),
            ("--connect-timeout", "30".to_owned()),
            ("--speed-limit", "1000".to_owned()),
            ("--speed-time", "60".to_owned()),
            ("--max-filesize", request.limit.to_string()),
        ];
        for (name, value) in numbers {
            args.push(name.into());
            args.push(value.into());
        }
        args.push("--user-agent".into());
        args.push(USER_AGENT.into());
        if request.from > 0 {
            args.push("--continue-at".into());
            args.push(request.from.to_string().into());
            if let Some(validator) = request.if_range {
                args.push("--header".into());
                args.push(format!("If-Range: {validator}").into());
            }
        }
        if request.token.is_some() {
            args.push("--config".into());
            args.push("-".into());
        }
        args.push("--output".into());
        args.push("-".into());
        args.push("--url".into());
        args.push(request.address.into());
        args
    }
}

impl Fetcher for CurlFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        let mut command = Command::new(&self.program);
        command.args(self.arguments(request));
        for (name, value) in &self.env {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        let config = request.token.map(|token| {
            // One line of curl's configuration; the token never reaches
            // the command line or a file.
            format!(
                "header = \"Authorization: Bearer {}\"\n",
                token.expose().replace(['"', '\\', '\n', '\r'], "")
            )
        });
        let reader = spawn(command, config, request, curl_error)?;
        Ok(Fetched {
            reader: primed(Box::new(reader))?,
            start: request.from,
            validator: None,
        })
    }
}

/// wget from PATH: the last transport, whole files only, no token.
#[derive(Clone, Debug)]
pub struct WgetFetcher {
    program: PathBuf,
}

impl WgetFetcher {
    /// wget at `program`, as the tools registry found it.
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }

    /// The argument list for one request, `--no-config` first.
    pub fn arguments(&self, request: &Request<'_>) -> Vec<OsString> {
        [
            "--no-config",
            "--quiet",
            "--https-only",
            "--max-redirect=5",
            "--tries=1",
            "--timeout=60",
        ]
        .into_iter()
        .map(OsString::from)
        .chain([
            format!("--user-agent={USER_AGENT}").into(),
            "--output-document=-".into(),
            "--".into(),
            request.address.into(),
        ])
        .collect()
    }
}

impl Fetcher for WgetFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        if request.token.is_some() {
            return Err(FetchError::Refused(
                "a private source needs the built-in client or curl".to_owned(),
            ));
        }
        let mut command = Command::new(&self.program);
        command.args(self.arguments(request));
        let reader = spawn(command, None, request, wget_error)?;
        Ok(Fetched {
            reader: primed(Box::new(reader))?,
            start: 0,
            validator: None,
        })
    }
}

/// The sentence for curl's exit code, with its standard error for an HTTP
/// status (`--fail` says "The requested URL returned error: 404").
pub fn curl_error(code: i32, stderr: &str) -> FetchError {
    match code {
        1 => FetchError::Refused("the address or a redirect is not https".to_owned()),
        3 => FetchError::Other("the address is not valid".to_owned()),
        5 | 6 => FetchError::Resolve,
        7 => FetchError::Connect,
        18 | 52 | 55 | 56 => FetchError::Lost,
        22 => match http_status(stderr) {
            Some(404 | 410) => FetchError::Missing,
            Some(416) => FetchError::CannotResume,
            Some(status) => FetchError::Http(status),
            None => FetchError::Http(0),
        },
        23 => FetchError::Other("curl could not hand the bytes on".to_owned()),
        28 => FetchError::Timeout,
        33 | 36 => FetchError::CannotResume,
        35 | 58 | 59 | 60 | 77 | 80 | 83 | 90 | 91 => FetchError::Tls,
        47 => FetchError::Redirect,
        63 => FetchError::TooLarge,
        other => FetchError::Other(format!("curl stopped with code {other}")),
    }
}

/// The sentence for wget's exit code.
pub fn wget_error(code: i32, _stderr: &str) -> FetchError {
    match code {
        3 => FetchError::Other("wget could not hand the bytes on".to_owned()),
        4 => FetchError::Connect,
        5 => FetchError::Tls,
        8 => FetchError::Http(0),
        other => FetchError::Other(format!("wget stopped with code {other}")),
    }
}

/// The status in curl's "returned error: 404" line.
fn http_status(stderr: &str) -> Option<u16> {
    let at = stderr.find("error: ")? + "error: ".len();
    stderr[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// Starts the program and returns its standard output as a reader that
/// turns a failing exit into its [`FetchError`].
fn spawn(
    mut command: Command,
    config: Option<String>,
    request: &Request<'_>,
    words: fn(i32, &str) -> FetchError,
) -> Result<ProgramReader, FetchError> {
    command
        .stdin(if config.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| FetchError::Other(format!("could not start the program: {e}")))?;
    if let (Some(config), Some(mut stdin)) = (config, child.stdin.take()) {
        // Written and closed at once; a failure shows as curl's own error.
        let _ = stdin.write_all(config.as_bytes());
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take().map(read_stderr);
    let child = Arc::new(Mutex::new(child));
    let finished = Arc::new(AtomicBool::new(false));
    // The watcher kills the program when the download is stopped, so a
    // read blocked on a stalled server returns at once.
    {
        let child = Arc::clone(&child);
        let finished = Arc::clone(&finished);
        let cancel = request.cancel.clone();
        thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if cancel.is_cancelled() {
                    let _ = child.lock().unwrap_or_else(PoisonError::into_inner).kill();
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
    }
    Ok(ProgramReader {
        stdout,
        stderr,
        child,
        finished,
        words,
        done: false,
    })
}

fn read_stderr(mut stderr: ChildStderr) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            if kept.len() < STDERR_KEEP {
                kept.extend_from_slice(&buf[..n]);
            }
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

/// A program's standard output; at its end, the program's exit decides
/// between the end of the file and a [`FetchError`].
struct ProgramReader {
    stdout: Option<ChildStdout>,
    stderr: Option<thread::JoinHandle<String>>,
    child: Arc<Mutex<Child>>,
    finished: Arc<AtomicBool>,
    words: fn(i32, &str) -> FetchError,
    done: bool,
}

impl ProgramReader {
    fn finish(&mut self) -> io::Result<()> {
        self.done = true;
        self.finished.store(true, Ordering::Relaxed);
        let status = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wait()?;
        let stderr = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        match status.code() {
            Some(0) => Ok(()),
            Some(code) => Err((self.words)(code, &stderr).into_io()),
            // Stopped by a signal: the watcher's kill, on a cancel.
            None => Err(FetchError::Other("the program was stopped".to_owned()).into_io()),
        }
    }
}

impl Read for ProgramReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        let n = match self.stdout.as_mut() {
            Some(out) => out.read(buf)?,
            None => 0,
        };
        if n == 0 {
            self.finish()?;
        }
        Ok(n)
    }
}

impl Drop for ProgramReader {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Relaxed);
        if !self.done {
            let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::Secret;
    use diroxide_vfs::CancelToken;

    fn request<'a>(cancel: &'a CancelToken) -> Request<'a> {
        Request {
            address: "https://example.invalid/diro-x-1.0/a.zip",
            from: 0,
            if_range: None,
            limit: 4096,
            token: None,
            cancel,
        }
    }

    fn text(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn curl_starts_with_q_and_never_carries_a_token() {
        let cancel = CancelToken::new();
        let curl = CurlFetcher::new("curl".into());
        let args = text(&curl.arguments(&request(&cancel)));
        assert_eq!(args[0], "-q");
        assert_eq!(
            args,
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
                "4096",
                "--user-agent",
                USER_AGENT,
                "--output",
                "-",
                "--url",
                "https://example.invalid/diro-x-1.0/a.zip",
            ]
        );
        let token = Secret::new("secret-t0ken".to_owned());
        let mut resumed = request(&cancel);
        resumed.from = 100;
        resumed.if_range = Some("\"v1\"");
        resumed.token = Some(&token);
        let args = text(&curl.arguments(&resumed));
        assert!(args.windows(2).any(|w| w == ["--continue-at", "100"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--header", "If-Range: \"v1\""])
        );
        assert!(args.windows(2).any(|w| w == ["--config", "-"]));
        assert!(args.iter().all(|a| !a.contains("t0ken")), "{args:?}");
        let loopback = CurlFetcher::for_loopback("curl".into());
        let args = text(&loopback.arguments(&request(&cancel)));
        assert!(args.windows(2).any(|w| w == ["--proto", "=http,https"]));
        assert!(args.windows(2).any(|w| w == ["--proto-redir", "=https"]));
    }

    #[test]
    fn wget_starts_with_no_config_and_refuses_a_token() {
        let cancel = CancelToken::new();
        let wget = WgetFetcher::new("wget".into());
        let args = text(&wget.arguments(&request(&cancel)));
        assert_eq!(args[0], "--no-config");
        assert!(args.contains(&"--https-only".to_owned()));
        assert!(args.contains(&format!("--user-agent={USER_AGENT}")));
        assert_eq!(
            args.last().unwrap(),
            "https://example.invalid/diro-x-1.0/a.zip"
        );
        let token = Secret::new("t".to_owned());
        let mut private = request(&cancel);
        private.token = Some(&token);
        assert!(matches!(wget.open(&private), Err(FetchError::Refused(_))));
    }

    #[test]
    fn exit_codes_become_sentences() {
        let table: [(i32, &str, FetchError); 14] = [
            (
                1,
                "",
                FetchError::Refused("the address or a redirect is not https".into()),
            ),
            (6, "", FetchError::Resolve),
            (7, "", FetchError::Connect),
            (18, "", FetchError::Lost),
            (
                22,
                "curl: (22) The requested URL returned error: 404",
                FetchError::Missing,
            ),
            (
                22,
                "curl: (22) The requested URL returned error: 416",
                FetchError::CannotResume,
            ),
            (
                22,
                "curl: (22) The requested URL returned error: 503",
                FetchError::Http(503),
            ),
            (28, "", FetchError::Timeout),
            (33, "", FetchError::CannotResume),
            (35, "", FetchError::Tls),
            (47, "", FetchError::Redirect),
            (56, "", FetchError::Lost),
            (60, "", FetchError::Tls),
            (63, "", FetchError::TooLarge),
        ];
        for (code, stderr, error) in table {
            assert_eq!(curl_error(code, stderr), error, "curl {code}");
        }
        assert_eq!(
            curl_error(99, ""),
            FetchError::Other("curl stopped with code 99".into())
        );
        assert_eq!(wget_error(4, ""), FetchError::Connect);
        assert_eq!(wget_error(5, ""), FetchError::Tls);
    }
}
