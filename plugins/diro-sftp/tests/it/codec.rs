//! The SFTP codec (R25 commit 54): golden packets built by hand from the
//! draft's field lists (`fixtures/sftp/packets/*.hex`), round trips of
//! any packet, the bounds, and the fuzz seeds replayed.

use std::path::Path;

use diro_sftp::sftp::codec::{
    DEFAULT_MAX_PACKET, Limits, StatVfs, frame, kind, open, read_strings, status, strings,
};
use diro_sftp::sftp::fuzzing;
use diro_sftp::sftp::{Attrs, CodecError, NameEntry, Request, Response};
use proptest::prelude::*;

fn fixtures() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/sftp"))
}

fn golden(name: &str) -> Vec<u8> {
    let text =
        std::fs::read_to_string(fixtures().join("packets").join(format!("{name}.hex"))).unwrap();
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

fn body(bytes: &[u8]) -> &[u8] {
    let (body, used) = frame(bytes, DEFAULT_MAX_PACKET).unwrap().unwrap();
    assert_eq!(used, bytes.len());
    body
}

#[test]
fn the_golden_requests_decode_and_encode_byte_for_byte() {
    for (name, expected) in [
        (
            "init",
            Request::Init {
                version: 3,
                extensions: vec![],
            },
        ),
        (
            "open-create-exclusive",
            Request::Open {
                id: 1,
                path: b"/srv/a.txt".to_vec(),
                pflags: open::READ | open::WRITE | open::CREAT | open::EXCL,
                attrs: Attrs {
                    permissions: Some(0o100_644),
                    ..Attrs::default()
                },
            },
        ),
        (
            "read",
            Request::Read {
                id: 9,
                handle: b"h1".to_vec(),
                offset: 32768,
                len: 32768,
            },
        ),
        (
            "write",
            Request::Write {
                id: 10,
                handle: b"h1".to_vec(),
                offset: 0,
                data: b"hello".to_vec(),
            },
        ),
        (
            "extended-posix-rename",
            Request::Extended {
                id: 11,
                name: b"posix-rename@openssh.com".to_vec(),
                data: strings(&[b"/a", b"/b"]),
            },
        ),
    ] {
        let bytes = golden(name);
        assert_eq!(
            Request::decode(body(&bytes)),
            Ok(expected.clone()),
            "{name}"
        );
        assert_eq!(expected.encode(), bytes, "{name}");
    }
}

#[test]
fn the_golden_answers_decode_and_encode_byte_for_byte() {
    for (name, expected) in [
        (
            "version-openssh",
            Response::Version {
                version: 3,
                extensions: vec![
                    (b"posix-rename@openssh.com".to_vec(), b"1".to_vec()),
                    (b"statvfs@openssh.com".to_vec(), b"2".to_vec()),
                    (b"limits@openssh.com".to_vec(), b"1".to_vec()),
                ],
            },
        ),
        (
            "status-eof",
            Response::Status {
                id: 7,
                code: status::EOF,
                message: b"End of file".to_vec(),
                language: vec![],
            },
        ),
        (
            "data",
            Response::Data {
                id: 9,
                data: b"hello".to_vec(),
            },
        ),
        (
            "name-two",
            Response::Name {
                id: 3,
                entries: vec![
                    NameEntry {
                        filename: b"a.txt".to_vec(),
                        longname: b"-rw-r--r--    1 ada ada 5 Nov 14 2023 a.txt".to_vec(),
                        attrs: Attrs {
                            size: Some(5),
                            permissions: Some(0o100_644),
                            times: Some((1_700_000_000, 1_700_000_001)),
                            ..Attrs::default()
                        },
                    },
                    // A name that is not UTF-8 travels as its bytes.
                    NameEntry {
                        filename: b"\xffname not utf8".to_vec(),
                        longname: vec![],
                        attrs: Attrs::default(),
                    },
                ],
            },
        ),
        (
            "attrs-extended",
            Response::Attrs {
                id: 4,
                attrs: Attrs {
                    extended: vec![(b"x@example".to_vec(), b"y".to_vec())],
                    ..Attrs::default()
                },
            },
        ),
    ] {
        let bytes = golden(name);
        assert_eq!(
            Response::decode(body(&bytes)),
            Ok(expected.clone()),
            "{name}"
        );
        assert_eq!(expected.encode(), bytes, "{name}");
    }
}

#[test]
fn lengths_are_checked_before_anything_is_allocated() {
    // A length over the limit, and the smallest that is wrong.
    assert_eq!(
        frame(&[0xff, 0xff, 0xff, 0xf0, 1], DEFAULT_MAX_PACKET),
        Err(CodecError::Length(0xffff_fff0))
    );
    assert_eq!(frame(&[0, 0, 0, 0], 64), Err(CodecError::Length(0)));
    // Not enough bytes yet: wait for more.
    assert_eq!(frame(&[0, 0, 0], 64), Ok(None));
    assert_eq!(frame(&[0, 0, 0, 9, 1, 2], 64), Ok(None));
    // A string that claims more than the packet holds.
    let mut lying = vec![kind::DATA, 0, 0, 0, 1];
    lying.extend_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(Response::decode(&lying), Err(CodecError::Short));
    // A NAME that claims four billion names in a few bytes.
    let mut bomb = vec![kind::NAME, 0, 0, 0, 1];
    bomb.extend_from_slice(&u32::MAX.to_be_bytes());
    bomb.extend_from_slice(&[0; 20]);
    assert_eq!(Response::decode(&bomb), Err(CodecError::Short));
    // Attribute flags version 3 does not know.
    let mut flags = vec![kind::ATTRS, 0, 0, 0, 1];
    flags.extend_from_slice(&0x0000_0100u32.to_be_bytes());
    assert_eq!(Response::decode(&flags), Err(CodecError::AttrFlags(0x100)));
    // Bytes after the last field, and a type in the wrong direction.
    assert_eq!(
        Request::decode(&[kind::INIT, 0, 0, 0, 3, 0]),
        Err(CodecError::Short)
    );
    assert_eq!(
        Response::decode(&[kind::HANDLE, 0, 0, 0, 1, 0, 0, 0, 0, 7]),
        Err(CodecError::Trailing)
    );
    assert_eq!(Response::decode(&[kind::OPEN]), Err(CodecError::Type(3)));
    assert_eq!(Request::decode(&[kind::STATUS]), Err(CodecError::Type(101)));
}

#[test]
fn an_old_server_status_without_words_reads() {
    let old = [kind::STATUS, 0, 0, 0, 2, 0, 0, 0, 4];
    assert_eq!(
        Response::decode(&old),
        Ok(Response::Status {
            id: 2,
            code: status::FAILURE,
            message: vec![],
            language: vec![],
        })
    );
}

#[test]
fn the_extension_answers_read() {
    let vfs = StatVfs {
        bsize: 4096,
        frsize: 4096,
        blocks: 1000,
        bfree: 600,
        bavail: 500,
        namemax: 255,
        ..StatVfs::default()
    };
    let back = StatVfs::decode(&vfs.encode()).unwrap();
    assert_eq!(back, vfs);
    assert_eq!(back.free_to_user(), 2_048_000);
    assert_eq!(back.total(), 4_096_000);
    let limits = Limits {
        max_packet: 262_144,
        max_read: 261_120,
        max_write: 261_120,
        max_open_handles: 0,
    };
    assert_eq!(Limits::decode(&limits.encode()), Ok(limits));
    assert_eq!(Limits::decode(&[0; 31]), Err(CodecError::Short));
    assert_eq!(
        read_strings(&strings(&[b"/a", b"/b"]), 2).unwrap(),
        [b"/a".to_vec(), b"/b".to_vec()]
    );
}

#[test]
fn data_never_shows_in_debug_output() {
    let write = Request::Write {
        id: 1,
        handle: b"h".to_vec(),
        offset: 0,
        data: b"secret-ish file contents".to_vec(),
    };
    let text = format!("{write:?}");
    assert_eq!(text, "Write(id 1, offset 0, 24 bytes)");
}

#[test]
fn the_fuzz_seeds_replay() {
    let seeds = fixtures().join("seeds").join("sftp_packet");
    let mut n = 0;
    for entry in std::fs::read_dir(seeds).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        fuzzing::packet(&bytes);
        // Cut anywhere, and stuck together, still nothing panics.
        for cut in 0..bytes.len() {
            fuzzing::packet(&bytes[..cut]);
        }
        let mut twice = bytes.clone();
        twice.extend_from_slice(&bytes);
        fuzzing::packet(&twice);
        n += 1;
    }
    assert!(n >= 10);
}

fn bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..40)
}

fn attrs() -> impl Strategy<Value = Attrs> {
    (
        proptest::option::of(any::<u64>()),
        proptest::option::of((any::<u32>(), any::<u32>())),
        proptest::option::of(any::<u32>()),
        proptest::option::of((any::<u32>(), any::<u32>())),
        proptest::collection::vec((bytes(), bytes()), 0..3),
    )
        .prop_map(|(size, uid_gid, permissions, times, extended)| Attrs {
            size,
            uid_gid,
            permissions,
            times,
            extended,
        })
}

fn request() -> impl Strategy<Value = Request> {
    prop_oneof![
        (
            any::<u32>(),
            proptest::collection::vec((bytes(), bytes()), 0..3)
        )
            .prop_map(|(version, extensions)| Request::Init {
                version,
                extensions
            }),
        (any::<u32>(), bytes(), any::<u32>(), attrs()).prop_map(|(id, path, pflags, attrs)| {
            Request::Open {
                id,
                path,
                pflags,
                attrs,
            }
        }),
        (any::<u32>(), bytes(), any::<u64>(), any::<u32>()).prop_map(
            |(id, handle, offset, len)| Request::Read {
                id,
                handle,
                offset,
                len
            }
        ),
        (any::<u32>(), bytes(), any::<u64>(), bytes()).prop_map(|(id, handle, offset, data)| {
            Request::Write {
                id,
                handle,
                offset,
                data,
            }
        }),
        (any::<u32>(), bytes(), attrs()).prop_map(|(id, path, attrs)| Request::Setstat {
            id,
            path,
            attrs
        }),
        (any::<u32>(), bytes(), bytes()).prop_map(|(id, from, to)| Request::Rename {
            id,
            from,
            to
        }),
        (any::<u32>(), bytes(), bytes()).prop_map(|(id, first, second)| Request::Symlink {
            id,
            first,
            second
        }),
        (any::<u32>(), bytes()).prop_map(|(id, path)| Request::Readdir { id, handle: path }),
        (any::<u32>(), bytes(), bytes()).prop_map(|(id, name, data)| Request::Extended {
            id,
            name,
            data
        }),
    ]
}

fn response() -> impl Strategy<Value = Response> {
    prop_oneof![
        (
            any::<u32>(),
            proptest::collection::vec((bytes(), bytes()), 0..3)
        )
            .prop_map(|(version, extensions)| Response::Version {
                version,
                extensions
            }),
        (any::<u32>(), any::<u32>(), bytes(), bytes()).prop_map(|(id, code, message, language)| {
            Response::Status {
                id,
                code,
                message,
                language,
            }
        }),
        (any::<u32>(), bytes()).prop_map(|(id, handle)| Response::Handle { id, handle }),
        (any::<u32>(), bytes()).prop_map(|(id, data)| Response::Data { id, data }),
        (
            any::<u32>(),
            proptest::collection::vec(
                (bytes(), bytes(), attrs()).prop_map(|(filename, longname, attrs)| NameEntry {
                    filename,
                    longname,
                    attrs
                }),
                0..5
            )
        )
            .prop_map(|(id, entries)| Response::Name { id, entries }),
        (any::<u32>(), attrs()).prop_map(|(id, attrs)| Response::Attrs { id, attrs }),
        (any::<u32>(), bytes()).prop_map(|(id, data)| Response::ExtendedReply { id, data }),
    ]
}

proptest! {
    #[test]
    fn every_request_round_trips(request in request()) {
        let bytes = request.encode();
        let (body, used) = frame(&bytes, usize::MAX).unwrap().unwrap();
        prop_assert_eq!(used, bytes.len());
        prop_assert_eq!(Request::decode(body), Ok(request));
    }

    #[test]
    fn every_answer_round_trips(response in response()) {
        let bytes = response.encode();
        let (body, used) = frame(&bytes, usize::MAX).unwrap().unwrap();
        prop_assert_eq!(used, bytes.len());
        prop_assert_eq!(Response::decode(body), Ok(response));
    }

    #[test]
    fn any_bytes_are_safe(data in proptest::collection::vec(any::<u8>(), 0..300)) {
        fuzzing::packet(&data);
    }
}
