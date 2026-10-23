//! The multiplexer and the session (R25 commit 56) against the fake
//! server: the handshake, the extension table and limits, 64 requests of
//! 32 KiB in flight, replies out of order, a reused id, a stall, a stream
//! that drops, a shell that prints first, and the handle cache.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use diro_sftp::rpc::Cancel;
use diro_sftp::sftp::codec::{Attrs, open, status};
use diro_sftp::sftp::fake::{FakeFs, Faults, duplex, serve, spawn};
use diro_sftp::sftp::mux::SftpError;
use diro_sftp::sftp::session::{MAX_HANDLES, Session, SessionConfig};

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn session_with(
    fs: FakeFs,
    faults: Faults,
    config: &SessionConfig,
) -> (Result<Session, SftpError>, Arc<Mutex<FakeFs>>) {
    let fs = Arc::new(Mutex::new(fs));
    let (input, output) = spawn(Arc::clone(&fs), faults);
    (Session::start(input, output, config), fs)
}

fn open_session(fs: FakeFs, faults: Faults) -> (Session, Arc<Mutex<FakeFs>>) {
    let (session, fs) = session_with(fs, faults, &SessionConfig::default());
    (session.unwrap(), fs)
}

fn read_all(session: &Session, path: &[u8], len: u64) -> Result<Vec<u8>, SftpError> {
    let handle = session.open(path, open::READ, Attrs::default(), None)?;
    let mut out = Vec::new();
    session.read_pipelined(&handle, 0, len, None, &mut |bytes| {
        out.extend_from_slice(bytes);
        true
    })?;
    session.close(&handle)?;
    Ok(out)
}

#[test]
fn the_handshake_reads_the_extensions_limits_and_home() {
    let (session, _) = open_session(FakeFs::default(), Faults::default());
    assert!(session.extensions().has("posix-rename@openssh.com"));
    assert!(session.extensions().has("statvfs@openssh.com"));
    assert!(!session.extensions().has("copy-data"));
    assert_eq!(session.limits().unwrap().max_read, 255 * 1024);
    assert_eq!(session.chunk(), 32 * 1024);
    assert_eq!(session.requests(), 64);
    assert_eq!(session.home(), b"/home/ada");
    assert!(session.statvfs(b"/", None).unwrap().unwrap().free_to_user() > 0);
    let (without, _) = open_session(
        FakeFs::default(),
        Faults {
            no_statvfs: true,
            ..Faults::default()
        },
    );
    // Not offered: not known, never zero.
    assert_eq!(without.statvfs(b"/", None).unwrap(), None);
}

/// Answer 88: 64 requests of 32 KiB in flight, delivered in order.
#[test]
fn a_large_read_keeps_64_requests_in_flight() {
    let mut fs = FakeFs::default();
    let data = pattern(3 * 1024 * 1024 + 123);
    fs.add_file(b"/home/ada/big.bin", &data);
    let (session, _) = open_session(fs, Faults::default());
    let read = read_all(&session, b"big.bin", u64::MAX).unwrap();
    assert_eq!(read, data);
    let most = session.mux().max_in_flight();
    assert!(most > 1 && most <= 64, "{most} in flight");
    assert_eq!(session.mux().in_flight(), 0);
}

#[test]
fn short_replies_are_asked_again_for_the_rest() {
    let mut fs = FakeFs::default();
    let data = pattern(100 * 1024);
    fs.add_file(b"/home/ada/a.bin", &data);
    let (session, _) = open_session(
        fs,
        Faults {
            short_reads: Some(1000),
            ..Faults::default()
        },
    );
    assert_eq!(read_all(&session, b"a.bin", u64::MAX).unwrap(), data);
}

#[test]
fn replies_out_of_order_reach_their_requests() {
    let mut fs = FakeFs::default();
    for i in 0..8 {
        fs.add_file(format!("/home/ada/f{i}").as_bytes(), &vec![0u8; i * 10]);
    }
    let data = pattern(500 * 1024);
    fs.add_file(b"/home/ada/big.bin", &data);
    let (session, _) = open_session(
        fs,
        Faults {
            out_of_order: true,
            ..Faults::default()
        },
    );
    let session = Arc::new(session);
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let session = Arc::clone(&session);
            thread::spawn(move || {
                let attrs = session.stat(format!("f{i}").as_bytes(), None).unwrap();
                assert_eq!(attrs.size, Some((i * 10) as u64));
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(read_all(&session, b"big.bin", u64::MAX).unwrap(), data);
}

#[test]
fn a_reused_id_is_dropped_and_counted() {
    let mut fs = FakeFs::default();
    let data = pattern(300 * 1024);
    fs.add_file(b"/home/ada/a.bin", &data);
    let (session, _) = open_session(
        fs,
        Faults {
            reuse_ids: true,
            ..Faults::default()
        },
    );
    assert_eq!(read_all(&session, b"a.bin", u64::MAX).unwrap(), data);
    // Give the reader a moment for the last duplicates.
    for _ in 0..100 {
        if session.mux().strays() >= 10 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(session.mux().strays() >= 10);
}

#[test]
fn a_stall_times_out_and_a_cancel_stops_at_once() {
    let config = SessionConfig {
        call_timeout: Duration::from_millis(400),
        ..SessionConfig::default()
    };
    // The handshake takes two answers: limits and realpath.
    let (session, _) = session_with(
        FakeFs::default(),
        Faults {
            stall_after: Some(2),
            ..Faults::default()
        },
        &config,
    );
    let session = session.unwrap();
    let started = Instant::now();
    assert_eq!(session.stat(b".", None), Err(SftpError::TimedOut));
    assert!(started.elapsed() >= Duration::from_millis(350));
    let cancel = Cancel::new();
    let stopper = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    assert_eq!(session.stat(b".", Some(&cancel)), Err(SftpError::Cancelled));
    assert!(started.elapsed() < Duration::from_millis(350));
    stopper.join().unwrap();
    assert_eq!(session.mux().in_flight(), 0);
}

#[test]
fn a_dropped_stream_loses_every_call() {
    let mut fs = FakeFs::default();
    fs.add_file(b"/home/ada/a.bin", &pattern(200 * 1024));
    // Enough for the handshake, then cut in the middle of the read.
    let (session, _) = open_session(
        fs,
        Faults {
            drop_after_bytes: Some(2000),
            ..Faults::default()
        },
    );
    let error = read_all(&session, b"a.bin", u64::MAX).unwrap_err();
    assert!(matches!(error, SftpError::Lost(_)), "{error:?}");
    assert!(session.mux().is_lost());
    assert!(matches!(session.stat(b".", None), Err(SftpError::Lost(_))));
}

#[test]
fn text_before_the_protocol_is_a_shell_printing() {
    let (client, server) = duplex();
    let (server_in, mut server_out) = server.split();
    thread::spawn(move || {
        let _ =
            server_out.write_all(b"Welcome to build.example!\r\n\x1b[1mHave a nice day\x1b[0m\r\n");
        let _ = serve(
            server_in,
            server_out,
            Arc::new(Mutex::new(FakeFs::default())),
            Faults::default(),
        );
    });
    let (input, output) = client.split();
    match Session::start(input, output, &SessionConfig::default()) {
        Err(SftpError::ShellChatter(text)) => {
            assert!(
                text.starts_with("Welcome to build.example! Have a nice day"),
                "{text}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn writes_check_every_answer() {
    let (session, fs) = open_session(FakeFs::default(), Faults::default());
    let data = pattern(1024 * 1024 + 5);
    let handle = session
        .open(
            b"new.bin",
            open::WRITE | open::CREAT | open::EXCL,
            Attrs::default(),
            None,
        )
        .unwrap();
    session.write_pipelined(&handle, 0, &data, None).unwrap();
    session.close(&handle).unwrap();
    assert_eq!(
        fs.lock().unwrap().file(b"/home/ada/new.bin"),
        Some(data.as_slice())
    );
    // A handle open for reading: every write is refused, and said once.
    let read_only = session
        .open(b"new.bin", open::READ, Attrs::default(), None)
        .unwrap();
    match session.write_pipelined(&read_only, 0, &data, None) {
        Err(SftpError::Status { code, .. }) => assert_eq!(code, status::PERMISSION_DENIED),
        other => panic!("{other:?}"),
    }
    assert_eq!(session.mux().in_flight(), 0);
}

#[test]
fn the_handle_cache_keeps_at_most_sixteen_open() {
    let mut fs = FakeFs::default();
    for i in 0..20 {
        fs.add_file(format!("/home/ada/f{i}").as_bytes(), b"x");
    }
    let (session, fs) = open_session(fs, Faults::default());
    let first = session.read_handle(b"f0", None).unwrap();
    for i in 0..20 {
        session
            .read_handle(format!("f{i}").as_bytes(), None)
            .unwrap();
    }
    assert_eq!(session.cached_handles(), MAX_HANDLES);
    let closes = fs
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|c| **c == "Close")
        .count();
    assert_eq!(closes, 20 - MAX_HANDLES);
    // A cached path gives its handle again without opening.
    let opens = fs
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|c| **c == "Open")
        .count();
    let again = session.read_handle(b"f19", None).unwrap();
    assert_ne!(again, first);
    assert_eq!(
        fs.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| **c == "Open")
            .count(),
        opens
    );
    session.close_handles();
    assert_eq!(session.cached_handles(), 0);
}

/// Spike SF4, against the fake only: a 64 MiB read with 64 requests in
/// flight and with one. A real link is the container tests' (S-C).
///
/// **Results, Friday, October 23, 2026, Windows 11, a debug build, in
/// memory (no latency):** 64 in flight, 464 MB/s, and 64 reached; 16 in
/// flight, 505 MB/s; one at a time, 267 MB/s. With no round trip to hide,
/// the window matters little here; on a real link it is what keeps the
/// pipe full.
#[test]
#[ignore = "spike SF4: a timing, run by hand"]
fn spike_sf4_throughput_against_the_fake() {
    let size = 64 * 1024 * 1024;
    let mut fs = FakeFs::default();
    fs.add_file(b"/home/ada/big.bin", &pattern(size));
    let fs = Arc::new(Mutex::new(fs));
    for requests in [64, 16, 1] {
        let (input, output) = spawn(Arc::clone(&fs), Faults::default());
        let session = Session::start(
            input,
            output,
            &SessionConfig {
                requests,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        let started = Instant::now();
        let read = read_all(&session, b"big.bin", u64::MAX).unwrap();
        let seconds = started.elapsed().as_secs_f64();
        assert_eq!(read.len(), size);
        println!(
            "SF4 {requests} in flight of 32 KiB: {:.0} MB/s ({seconds:.2} s), most in flight {}",
            size as f64 / 1e6 / seconds,
            session.mux().max_in_flight()
        );
    }
}
