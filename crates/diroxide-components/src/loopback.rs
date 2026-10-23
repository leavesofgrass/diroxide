//! A small HTTP server on 127.0.0.1 for the tests: plain `std` sockets,
//! files served with scripted behaviors (R22 section 6.6, spike S3), every
//! request recorded. Nothing here goes past the loopback address.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

/// How a file is served.
#[derive(Clone, Debug)]
pub(crate) enum Mode {
    /// Ranges honored, `If-Range` checked against the entity tag.
    Honor,
    /// A range request answered `200` with the whole file.
    IgnoreRange,
    /// A range request answered `206` from the wrong byte.
    WrongRange,
    /// The first request gets the file; later ones get these bytes, with
    /// another entity tag (ranges honored for both). Used by the built-in
    /// client's tests, so a build without it does not construct it.
    #[cfg_attr(not(feature = "fetch"), allow(dead_code))]
    ChangesAfterFirst(Vec<u8>),
    /// The length promised, only this many body bytes sent, then closed.
    Truncate(usize),
    /// These bytes after the file (ranges honored).
    Longer(Vec<u8>),
    /// A redirect to this address.
    Redirect(String),
    /// `404`.
    NotFound,
    /// This many bytes, then nothing until the server stops.
    Stall(usize),
}

/// One file the server holds.
#[derive(Clone, Debug)]
pub(crate) struct Served {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mode: Mode,
}

/// One request the server saw.
#[derive(Clone, Debug, Default)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) range: Option<String>,
    pub(crate) if_range: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
}

impl Seen {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct State {
    files: HashMap<String, (Served, usize)>,
    seen: Vec<Seen>,
}

/// The server: started on a free port, stopped when dropped.
pub(crate) struct Server {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    accept: Option<thread::JoinHandle<()>>,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Server {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let addr = listener.local_addr().expect("its address");
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let state = Arc::clone(&state);
                    let stop = Arc::clone(&stop);
                    thread::spawn(move || {
                        let _ = handle(stream, &state, &stop);
                    });
                }
            })
        };
        Self {
            addr,
            state,
            stop,
            accept: Some(accept),
        }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path.trim_start_matches('/'))
    }

    pub(crate) fn serve(&self, path: &str, bytes: &[u8], mode: Mode) {
        lock(&self.state).files.insert(
            format!("/{}", path.trim_start_matches('/')),
            (
                Served {
                    bytes: bytes.to_vec(),
                    mode,
                },
                0,
            ),
        );
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        lock(&self.state).seen.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

fn handle(stream: TcpStream, state: &Mutex<State>, stop: &AtomicBool) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut seen = Seen {
        path: path.clone(),
        ..Seen::default()
    };
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.trim_end().split_once(':') {
            let value = value.trim().to_owned();
            if name.eq_ignore_ascii_case("range") {
                seen.range = Some(value.clone());
            }
            if name.eq_ignore_ascii_case("if-range") {
                seen.if_range = Some(value.clone());
            }
            seen.headers.push((name.to_owned(), value));
        }
    }
    let from: Option<u64> = seen
        .range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.trim_end_matches('-').parse().ok());
    let if_range = seen.if_range.clone();
    let served = {
        let mut state = lock(state);
        state.seen.push(seen);
        match state.files.get_mut(&path) {
            Some((served, hits)) => {
                *hits += 1;
                Some((served.clone(), *hits))
            }
            None => None,
        }
    };
    let mut out = stream;
    let Some((served, hits)) = served else {
        return respond(&mut out, "404 Not Found", &[], b"");
    };
    let (bytes, etag) = match &served.mode {
        Mode::ChangesAfterFirst(next) if hits > 1 => (next.clone(), "\"v2\""),
        Mode::Longer(extra) => ([served.bytes.clone(), extra.clone()].concat(), "\"v1\""),
        _ => (served.bytes.clone(), "\"v1\""),
    };
    let len = bytes.len() as u64;
    let tag = ("ETag", etag.to_owned());
    match &served.mode {
        Mode::NotFound => respond(&mut out, "404 Not Found", &[], b""),
        Mode::Redirect(to) => respond(&mut out, "302 Found", &[("Location", to.clone())], b""),
        Mode::IgnoreRange => respond(&mut out, "200 OK", &[tag], &bytes),
        Mode::WrongRange => match from {
            Some(from) if from > 0 => {
                let wrong = (from / 2) as usize;
                let range = format!("bytes {wrong}-{}/{len}", len - 1);
                respond(
                    &mut out,
                    "206 Partial Content",
                    &[tag, ("Content-Range", range)],
                    &bytes[wrong..],
                )
            }
            _ => respond(&mut out, "200 OK", &[tag], &bytes),
        },
        Mode::Truncate(keep) => {
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
            out.write_all(head.as_bytes())?;
            out.write_all(&bytes[..(*keep).min(bytes.len())])?;
            out.flush()
        }
        Mode::Stall(keep) => {
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
            out.write_all(head.as_bytes())?;
            out.write_all(&bytes[..(*keep).min(bytes.len())])?;
            out.flush()?;
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        }
        Mode::Honor | Mode::ChangesAfterFirst(_) | Mode::Longer(_) => {
            let fresh = if_range.as_deref().is_none_or(|v| v == etag);
            match from {
                Some(from) if fresh && from >= len => respond(
                    &mut out,
                    "416 Range Not Satisfiable",
                    &[("Content-Range", format!("bytes */{len}"))],
                    b"",
                ),
                Some(from) if fresh => {
                    let range = format!("bytes {from}-{}/{len}", len - 1);
                    respond(
                        &mut out,
                        "206 Partial Content",
                        &[tag, ("Content-Range", range)],
                        &bytes[from as usize..],
                    )
                }
                _ => respond(&mut out, "200 OK", &[tag], &bytes),
            }
        }
    }
}

fn respond(
    out: &mut TcpStream,
    status: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(body)?;
    out.flush()?;
    // Read what the client may still send, so closing does not reset it.
    let _ = out.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = out.read(&mut [0u8; 64]);
    Ok(())
}
