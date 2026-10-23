//! Fetching bytes: the [`Fetcher`] trait, files on this computer (a source
//! folder), and the choice of transport for an `https` address.
//!
//! The transports, in order (plan section 5.14, answers 40 and 44): the
//! built-in client (`ureq` with rustls, behind the `fetch` feature), then
//! `curl`, then `wget`, each found on PATH through the tools registry and
//! run with an argument list, never a shell. A later transport is tried
//! only when the one before it could not reach the server at all (no
//! answer, no connection, a TLS problem); an answer from the server, such
//! as "not found", is final. A token for a private source is a header for
//! the built-in client and a configuration line on standard input for
//! curl, never an argument, so it never shows in a process list; wget is
//! not used for a private source.
//!
//! The pins, not the transport, vouch for every file: whatever arrives is
//! checked by size and SHA-256 before it is used.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use diroxide_core::Secret;
use diroxide_tools::{ToolRegistry, ToolStatus};
use diroxide_vfs::CancelToken;

/// One request: an address, from a byte.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// An `https` address, or a path on this computer.
    pub address: &'a str,
    /// The byte to start from: 0 for the whole file, more to go on with a
    /// stopped download.
    pub from: u64,
    /// The validator the bytes already held were fetched under (an entity
    /// tag or a last-modified date), sent as `If-Range` so a file that
    /// changed comes whole instead of appended.
    pub if_range: Option<&'a str>,
    /// The pinned size: more than this is refused as it arrives.
    pub limit: u64,
    /// A private source's token, or none.
    pub token: Option<&'a Secret>,
    /// Stops a running transfer (kills a curl or wget it started).
    pub cancel: &'a CancelToken,
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("address", &self.address)
            .field("from", &self.from)
            .field("if_range", &self.if_range)
            .field("limit", &self.limit)
            .field("token", &self.token.map(|_| "[hidden]"))
            .finish_non_exhaustive()
    }
}

/// An answer to [`Fetcher::open`]: the bytes, where they start, and the
/// server's validator.
pub struct Fetched {
    /// The bytes from `start` to the end of the file. A failure while
    /// reading is an [`io::Error`] whose inner error is a [`FetchError`].
    pub reader: Box<dyn Read + Send>,
    /// Where the bytes start: the `from` asked for when the source went on
    /// from there, else 0 (the whole file again).
    pub start: u64,
    /// The server's entity tag, else its last-modified date, for the next
    /// resume's `If-Range`.
    pub validator: Option<String>,
}

impl fmt::Debug for Fetched {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fetched")
            .field("start", &self.start)
            .field("validator", &self.validator)
            .finish_non_exhaustive()
    }
}

/// Why a fetch failed, each with its words.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// The file is not there: a 404, or a missing file in a source folder.
    Missing,
    /// The server answered with another error status.
    Http(u16),
    /// The host's name could not be found.
    Resolve,
    /// No connection could be made.
    Connect,
    /// The server took too long, or stopped sending.
    Timeout,
    /// The secure connection failed (a certificate, a handshake).
    Tls,
    /// The server cannot go on from where the download stopped.
    CannotResume,
    /// Too many redirects.
    Redirect,
    /// More bytes than the pin allows.
    TooLarge,
    /// The connection was cut before the file was whole.
    Lost,
    /// The address was refused before anything was sent (a protocol that
    /// is not allowed, a private source for a transport that cannot carry
    /// its token).
    Refused(String),
    /// No program in this build or on PATH can fetch from the network.
    NoClient,
    /// Something else, in the transport's words.
    Other(String),
}

impl FetchError {
    /// The cause in words, lowercase, for "Not downloaded: ..." and the
    /// house form.
    pub fn reason(&self) -> String {
        match self {
            FetchError::Missing => "the file is not there".to_owned(),
            FetchError::Http(code) => format!("the server answered {code}"),
            FetchError::Resolve => "could not find the host".to_owned(),
            FetchError::Connect => "could not connect".to_owned(),
            FetchError::Timeout => "the server took too long".to_owned(),
            FetchError::Tls => "the secure connection failed".to_owned(),
            FetchError::CannotResume => "the server cannot resume".to_owned(),
            FetchError::Redirect => "too many redirects".to_owned(),
            FetchError::TooLarge => "the file is larger than its pin".to_owned(),
            FetchError::Lost => "the connection was lost".to_owned(),
            FetchError::Refused(why) => why.clone(),
            FetchError::NoClient => "curl was not found".to_owned(),
            FetchError::Other(words) => words.clone(),
        }
    }

    /// Whether the next transport should try: the server was never
    /// reached, so a different client may do better.
    pub fn is_transport(&self) -> bool {
        matches!(
            self,
            FetchError::Resolve
                | FetchError::Connect
                | FetchError::Timeout
                | FetchError::Tls
                | FetchError::NoClient
                | FetchError::Other(_)
        )
    }

    /// The error as an [`io::Error`], for a reader.
    pub fn into_io(self) -> io::Error {
        io::Error::other(self)
    }

    /// The [`FetchError`] inside an [`io::Error`] from a reader, or
    /// [`FetchError::Lost`] with the system's words for any other.
    pub fn from_io(error: &io::Error) -> FetchError {
        match error.get_ref().and_then(|e| e.downcast_ref::<FetchError>()) {
            Some(inner) => inner.clone(),
            None => match error.kind() {
                io::ErrorKind::TimedOut => FetchError::Timeout,
                io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe => FetchError::Lost,
                _ => FetchError::Other(error.to_string()),
            },
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason())
    }
}

impl std::error::Error for FetchError {}

/// Fetches files. The downloader takes one; tests pass
/// a fake fetcher that serves fixtures and records every request, so a
/// test fails if anything is fetched before a yes.
pub trait Fetcher: Send + Sync {
    /// Opens `request.address` from byte `request.from`.
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError>;
}

/// Whether an address is a path on this computer, not `http:` or `https:`.
pub fn is_local(address: &str) -> bool {
    let lower = address.to_ascii_lowercase();
    !lower.starts_with("http://") && !lower.starts_with("https://")
}

/// The path of a local address: `file://` or a plain path.
pub fn local_path(address: &str) -> PathBuf {
    match address.strip_prefix("file://") {
        // file:///home/x is /home/x; file:///D:/x is D:/x.
        Some(rest) => {
            let b = rest.as_bytes();
            if b.len() > 2 && b[0] == b'/' && b[2] == b':' {
                PathBuf::from(&rest[1..])
            } else {
                PathBuf::from(rest)
            }
        }
        None => PathBuf::from(address),
    }
}

/// Files on this computer: a source folder, a memory stick.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFetcher;

impl Fetcher for LocalFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        open_local(&local_path(request.address), request.from)
    }
}

fn open_local(path: &Path, from: u64) -> Result<Fetched, FetchError> {
    let meta = std::fs::metadata(path).map_err(|_| FetchError::Missing)?;
    if !meta.is_file() {
        return Err(FetchError::Missing);
    }
    let mut file = File::open(path).map_err(|e| FetchError::Other(e.to_string()))?;
    let start = if from > 0 && from <= meta.len() {
        file.seek(SeekFrom::Start(from))
            .map_err(|e| FetchError::Other(e.to_string()))?;
        from
    } else {
        0
    };
    Ok(Fetched {
        reader: Box::new(file),
        start,
        validator: None,
    })
}

/// The fetcher the program uses: a local address is read from disk; an
/// `https` address goes to the built-in client, then curl, then wget,
/// whichever this build and PATH have.
pub struct StandardFetcher {
    #[cfg(feature = "fetch")]
    http: Option<crate::http::HttpFetcher>,
    curl: Option<crate::curl::CurlFetcher>,
    wget: Option<crate::curl::WgetFetcher>,
}

impl fmt::Debug for StandardFetcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandardFetcher")
            .field("transports", &self.transports())
            .finish()
    }
}

impl StandardFetcher {
    /// The built-in client when this build has it, and curl and wget when
    /// the tools registry finds them (its PATH rule, then `tools.toml`).
    pub fn from_tools(tools: &ToolRegistry) -> Self {
        let found = |id: &str| match tools.find(id) {
            Ok(ToolStatus::Found(path)) => Some(path),
            _ => None,
        };
        Self {
            #[cfg(feature = "fetch")]
            http: Some(crate::http::HttpFetcher::new()),
            curl: found("curl").map(crate::curl::CurlFetcher::new),
            wget: found("wget").map(crate::curl::WgetFetcher::new),
        }
    }

    /// The same without the built-in client: curl, then wget, as a build
    /// without the `fetch` feature has them.
    #[cfg_attr(not(feature = "fetch"), allow(unused_mut))]
    pub fn without_builtin(mut self) -> Self {
        #[cfg(feature = "fetch")]
        {
            self.http = None;
        }
        self
    }

    /// Only files on this computer: a build or a test with no network.
    pub fn local_only() -> Self {
        Self {
            #[cfg(feature = "fetch")]
            http: None,
            curl: None,
            wget: None,
        }
    }

    /// The transports it has, by name, in the order tried: "built-in",
    /// "curl", "wget".
    pub fn transports(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        #[cfg(feature = "fetch")]
        if self.http.is_some() {
            names.push("built-in");
        }
        if self.curl.is_some() {
            names.push("curl");
        }
        if self.wget.is_some() {
            names.push("wget");
        }
        names
    }

    /// Whether it can fetch from the network at all.
    pub fn can_download(&self) -> bool {
        !self.transports().is_empty()
    }
}

impl Fetcher for StandardFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        if is_local(request.address) {
            return LocalFetcher.open(request);
        }
        let mut transports: Vec<&dyn Fetcher> = Vec::new();
        #[cfg(feature = "fetch")]
        if let Some(http) = &self.http {
            transports.push(http);
        }
        if let Some(curl) = &self.curl {
            transports.push(curl);
        }
        if let Some(wget) = &self.wget
            && request.token.is_none()
        {
            transports.push(wget);
        }
        let mut last = FetchError::NoClient;
        for transport in transports {
            match transport.open(request) {
                Ok(fetched) => return Ok(fetched),
                Err(error) if error.is_transport() => last = error,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }
}

/// Reads the first bytes of `reader` before handing it on, so a transport
/// whose failure only shows when reading (a program that exits at once)
/// fails in [`Fetcher::open`], where the next transport can still try.
pub(crate) fn primed(mut reader: Box<dyn Read + Send>) -> Result<Box<dyn Read + Send>, FetchError> {
    let mut first = vec![0u8; 1 << 14];
    let n = loop {
        match reader.read(&mut first) {
            Ok(n) => break n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FetchError::from_io(&e)),
        }
    };
    first.truncate(n);
    Ok(Box::new(io::Cursor::new(first).chain(reader)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(address: &'a str, from: u64, cancel: &'a CancelToken) -> Request<'a> {
        Request {
            address,
            from,
            if_range: None,
            limit: 100,
            token: None,
            cancel,
        }
    }

    #[test]
    fn local_addresses_are_read_from_disk() {
        assert!(!is_local("https://x/y"));
        assert!(!is_local("HTTP://x/y"));
        assert!(is_local("D:/source/a"));
        assert_eq!(local_path("file:///D:/m/a"), PathBuf::from("D:/m/a"));
        assert_eq!(local_path("file:///srv/m/a"), PathBuf::from("/srv/m/a"));
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.bin");
        std::fs::write(&path, b"hello").unwrap();
        let address = path.to_string_lossy().into_owned();
        let cancel = CancelToken::new();
        let mut text = String::new();
        let fetched = LocalFetcher.open(&request(&address, 2, &cancel)).unwrap();
        assert_eq!(fetched.start, 2);
        fetched.reader.take(10).read_to_string(&mut text).unwrap();
        assert_eq!(text, "llo");
        let whole = LocalFetcher.open(&request(&address, 9, &cancel)).unwrap();
        assert_eq!(whole.start, 0, "past the end, the whole file again");
        let missing = tmp.path().join("nope").to_string_lossy().into_owned();
        assert_eq!(
            LocalFetcher
                .open(&request(&missing, 0, &cancel))
                .unwrap_err(),
            FetchError::Missing
        );
        let folder = tmp.path().to_string_lossy().into_owned();
        assert_eq!(
            LocalFetcher
                .open(&request(&folder, 0, &cancel))
                .unwrap_err(),
            FetchError::Missing
        );
    }

    #[test]
    fn with_no_transport_nothing_goes_to_the_network() {
        let fetcher = StandardFetcher::local_only();
        assert!(!fetcher.can_download());
        let cancel = CancelToken::new();
        assert_eq!(
            fetcher
                .open(&request("https://example.invalid/a", 0, &cancel))
                .unwrap_err(),
            FetchError::NoClient
        );
        assert_eq!(FetchError::NoClient.reason(), "curl was not found");
    }

    #[test]
    fn errors_travel_inside_io_errors() {
        let io = FetchError::CannotResume.into_io();
        assert_eq!(FetchError::from_io(&io), FetchError::CannotResume);
        let plain = io::Error::from(io::ErrorKind::ConnectionReset);
        assert_eq!(FetchError::from_io(&plain), FetchError::Lost);
        assert!(FetchError::Connect.is_transport());
        assert!(!FetchError::Missing.is_transport());
        let token = Secret::new("t0ken".to_owned());
        let cancel = CancelToken::new();
        let mut r = request("https://x/a", 0, &cancel);
        r.token = Some(&token);
        assert!(!format!("{r:?}").contains("t0ken"));
    }
}
