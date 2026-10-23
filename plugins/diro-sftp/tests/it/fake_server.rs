//! The fake SFTP server and the fake ssh (R25 commit 55): one test per
//! fault mode, spoken to with the codec alone (the multiplexer comes
//! next), and the server behind the fake ssh's pipes.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diro_sftp::sftp::codec::{DEFAULT_MAX_PACKET, StatVfs, frame, open, status};
use diro_sftp::sftp::fake::{FakeFs, Faults, spawn};
use diro_sftp::sftp::{Request, Response};
use diro_sftp::ssh::Launch;
use serde_json::json;

use crate::fakes::fake_ssh_in;

/// A raw client: requests out, answers in on a channel (an `Err` when the
/// stream ended, with the bytes left over).
struct Raw {
    out: Box<dyn Write + Send>,
    answers: Receiver<Result<Response, usize>>,
}

impl Raw {
    fn new(input: impl Read + Send + 'static, out: impl Write + Send + 'static) -> Self {
        let (tx, answers) = mpsc::channel();
        thread::spawn(move || {
            let mut input = input;
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 65536];
            loop {
                while let Ok(Some((body, used))) = frame(&buffer, DEFAULT_MAX_PACKET) {
                    let answer = Response::decode(body).unwrap();
                    buffer.drain(..used);
                    if tx.send(Ok(answer)).is_err() {
                        return;
                    }
                }
                match input.read(&mut chunk) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(Err(buffer.len()));
                        return;
                    }
                    Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                }
            }
        });
        Self {
            out: Box::new(out),
            answers,
        }
    }

    fn send(&mut self, request: &Request) {
        self.out.write_all(&request.encode()).unwrap();
        self.out.flush().unwrap();
    }

    fn next(&self) -> Option<Result<Response, usize>> {
        self.answers.recv_timeout(Duration::from_secs(5)).ok()
    }

    fn quiet(&self) -> bool {
        self.answers
            .recv_timeout(Duration::from_millis(300))
            .is_err()
    }

    fn call(&mut self, request: &Request) -> Response {
        self.send(request);
        self.next().unwrap().unwrap()
    }

    fn init(&mut self) -> Vec<(Vec<u8>, Vec<u8>)> {
        match self.call(&Request::Init {
            version: 3,
            extensions: vec![],
        }) {
            Response::Version {
                version: 3,
                extensions,
            } => extensions,
            other => panic!("{other:?}"),
        }
    }
}

fn server(fs: FakeFs, faults: Faults) -> (Raw, Arc<Mutex<FakeFs>>) {
    let fs = Arc::new(Mutex::new(fs));
    let (input, out) = spawn(Arc::clone(&fs), faults);
    (Raw::new(input, out), fs)
}

fn handle_of(response: Response) -> Vec<u8> {
    match response {
        Response::Handle { handle, .. } => handle,
        other => panic!("{other:?}"),
    }
}

#[test]
fn it_lists_reads_and_writes() {
    let mut fs = FakeFs::default();
    for i in 0..250 {
        fs.add_file(format!("/home/ada/many/f{i:03}").as_bytes(), b"x");
    }
    fs.add_file(b"/home/ada/a.txt", b"hello world");
    fs.add_file(b"/home/ada/\xffodd", b"bytes");
    let (mut c, fs) = server(fs, Faults::default());
    let extensions = c.init();
    assert!(extensions.iter().any(|(n, _)| n == b"statvfs@openssh.com"));
    // REALPATH of "." is the home folder.
    match c.call(&Request::Realpath {
        id: 1,
        path: b".".to_vec(),
    }) {
        Response::Name { entries, .. } => assert_eq!(entries[0].filename, b"/home/ada"),
        other => panic!("{other:?}"),
    }
    // A listing in batches, then the end.
    let dir = handle_of(c.call(&Request::Opendir {
        id: 2,
        path: b"many".to_vec(),
    }));
    let mut sizes = Vec::new();
    loop {
        match c.call(&Request::Readdir {
            id: 3,
            handle: dir.clone(),
        }) {
            Response::Name { entries, .. } => sizes.push(entries.len()),
            Response::Status { code, .. } => {
                assert_eq!(code, status::EOF);
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(sizes, [100, 100, 50]);
    // A name that is not UTF-8 is served as its bytes.
    let home = handle_of(c.call(&Request::Opendir {
        id: 4,
        path: b"/home/ada".to_vec(),
    }));
    match c.call(&Request::Readdir {
        id: 5,
        handle: home,
    }) {
        Response::Name { entries, .. } => {
            assert!(entries.iter().any(|e| e.filename == b"\xffodd"));
        }
        other => panic!("{other:?}"),
    }
    // A ranged read.
    let file = handle_of(c.call(&Request::Open {
        id: 6,
        path: b"a.txt".to_vec(),
        pflags: open::READ,
        attrs: Default::default(),
    }));
    match c.call(&Request::Read {
        id: 7,
        handle: file.clone(),
        offset: 6,
        len: 100,
    }) {
        Response::Data { data, .. } => assert_eq!(data, b"world"),
        other => panic!("{other:?}"),
    }
    // A new file: created only once, written, closed.
    let new = handle_of(c.call(&Request::Open {
        id: 8,
        path: b"new.bin".to_vec(),
        pflags: open::WRITE | open::CREAT | open::EXCL,
        attrs: Default::default(),
    }));
    assert!(matches!(
        c.call(&Request::Open {
            id: 9,
            path: b"new.bin".to_vec(),
            pflags: open::WRITE | open::CREAT | open::EXCL,
            attrs: Default::default(),
        }),
        Response::Status {
            code: status::FAILURE,
            ..
        }
    ));
    for (i, part) in [&b"abc"[..], b"def"].iter().enumerate() {
        assert!(matches!(
            c.call(&Request::Write {
                id: 10,
                handle: new.clone(),
                offset: (i * 3) as u64,
                data: part.to_vec(),
            }),
            Response::Status {
                code: status::OK,
                ..
            }
        ));
    }
    assert!(matches!(
        c.call(&Request::Close {
            id: 11,
            handle: new
        }),
        Response::Status {
            code: status::OK,
            ..
        }
    ));
    assert_eq!(
        fs.lock().unwrap().file(b"/home/ada/new.bin"),
        Some(&b"abcdef"[..])
    );
    // posix-rename replaces; statvfs counts.
    match c.call(&Request::Extended {
        id: 12,
        name: b"posix-rename@openssh.com".to_vec(),
        data: diro_sftp::sftp::codec::strings(&[b"new.bin", b"a.txt"]),
    }) {
        Response::Status { code, .. } => assert_eq!(code, status::OK),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fs.lock().unwrap().file(b"/home/ada/a.txt"),
        Some(&b"abcdef"[..])
    );
    match c.call(&Request::Extended {
        id: 13,
        name: b"statvfs@openssh.com".to_vec(),
        data: diro_sftp::sftp::codec::strings(&[b"/"]),
    }) {
        Response::ExtendedReply { data, .. } => {
            assert!(StatVfs::decode(&data).unwrap().free_to_user() > 0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stall_answers_nothing_more() {
    let (mut c, _) = server(
        FakeFs::default(),
        Faults {
            stall_after: Some(1),
            ..Faults::default()
        },
    );
    c.init();
    c.call(&Request::Stat {
        id: 1,
        path: b".".to_vec(),
    });
    c.send(&Request::Stat {
        id: 2,
        path: b".".to_vec(),
    });
    assert!(c.quiet());
}

#[test]
fn a_short_reply_reads_less_than_asked() {
    let mut fs = FakeFs::default();
    fs.add_file(b"/home/ada/big.bin", &[7u8; 5000]);
    let (mut c, _) = server(
        fs,
        Faults {
            short_reads: Some(1000),
            ..Faults::default()
        },
    );
    c.init();
    let file = handle_of(c.call(&Request::Open {
        id: 1,
        path: b"big.bin".to_vec(),
        pflags: open::READ,
        attrs: Default::default(),
    }));
    match c.call(&Request::Read {
        id: 2,
        handle: file,
        offset: 0,
        len: 32768,
    }) {
        Response::Data { data, .. } => assert_eq!(data.len(), 1000),
        other => panic!("{other:?}"),
    }
}

#[test]
fn answers_come_out_of_order() {
    let (mut c, _) = server(
        FakeFs::default(),
        Faults {
            out_of_order: true,
            ..Faults::default()
        },
    );
    c.init();
    for id in 1..=4 {
        c.send(&Request::Stat {
            id,
            path: b".".to_vec(),
        });
    }
    let ids: Vec<u32> = (0..4)
        .map(|_| c.next().unwrap().unwrap().id().unwrap())
        .collect();
    assert_eq!(ids, [4, 3, 2, 1]);
}

#[test]
fn a_reused_id_answers_twice() {
    let mut fs = FakeFs::default();
    fs.add_file(b"/home/ada/a.txt", b"abc");
    let (mut c, _) = server(
        fs,
        Faults {
            reuse_ids: true,
            ..Faults::default()
        },
    );
    c.init();
    let file = handle_of(c.call(&Request::Open {
        id: 1,
        path: b"a.txt".to_vec(),
        pflags: open::READ,
        attrs: Default::default(),
    }));
    c.send(&Request::Read {
        id: 5,
        handle: file,
        offset: 0,
        len: 10,
    });
    let first = c.next().unwrap().unwrap();
    let second = c.next().unwrap().unwrap();
    assert_eq!(first.id(), Some(5));
    assert_eq!(second, first);
}

#[test]
fn the_stream_drops_mid_packet() {
    let (mut c, _) = server(
        FakeFs::default(),
        Faults {
            drop_after_bytes: Some(40),
            ..Faults::default()
        },
    );
    c.send(&Request::Init {
        version: 3,
        extensions: vec![],
    });
    // The VERSION answer is longer than 40 bytes: the stream ends inside it.
    match c.next().unwrap() {
        Err(left) => assert_eq!(left, 40),
        Ok(other) => panic!("{other:?}"),
    }
}

#[test]
fn a_server_without_statvfs_says_so() {
    let (mut c, _) = server(
        FakeFs::default(),
        Faults {
            no_statvfs: true,
            ..Faults::default()
        },
    );
    let extensions = c.init();
    assert!(!extensions.iter().any(|(n, _)| n == b"statvfs@openssh.com"));
    match c.call(&Request::Extended {
        id: 1,
        name: b"statvfs@openssh.com".to_vec(),
        data: diro_sftp::sftp::codec::strings(&[b"/"]),
    }) {
        Response::Status { code, .. } => assert_eq!(code, status::OP_UNSUPPORTED),
        other => panic!("{other:?}"),
    }
}

/// The fake ssh serves the fake server on its pipes, as `ssh -s sftp`
/// does: binary bytes both ways, and closing its input ends it.
#[test]
fn the_fake_ssh_serves_sftp_on_its_pipes() {
    let root = tempfile::tempdir().unwrap();
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"script": [{"serve": {"files": {"/home/ada/a.bin": "abc"}}}, {"exit": 0}]}),
    );
    let mut process = Launch {
        program: ssh,
        args: vec!["--".into(), "build.example".into(), "sftp".into()],
        askpass: None,
    }
    .spawn()
    .unwrap();
    let stdin = process.take_stdin().unwrap();
    let stdout = process.take_stdout().unwrap();
    let mut c = Raw::new(stdout, stdin);
    assert!(!c.init().is_empty());
    match c.call(&Request::Stat {
        id: 1,
        path: b"a.bin".to_vec(),
    }) {
        Response::Attrs { attrs, .. } => assert_eq!(attrs.size, Some(3)),
        other => panic!("{other:?}"),
    }
    drop(c);
    assert_eq!(
        process
            .wait_timeout(Duration::from_secs(10))
            .and_then(|s| s.code()),
        Some(0)
    );
}
