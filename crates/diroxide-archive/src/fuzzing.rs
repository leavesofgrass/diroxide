//! The bodies of the `archive_*` fuzz targets (research/archives.md
//! section 12), as plain functions, so the libFuzzer entry points in
//! `fuzz/fuzz_targets/` and the ordinary tests call the same code: the
//! checked-in seeds and every crash found later replay on Windows and
//! macOS in normal CI, with no fuzzer. Each body panics only when a
//! property fails; bad input is an error, never a panic.

use diroxide_core::Place;
use diroxide_core::place::PathStyle;

use crate::limits::Limits;
use crate::names::{is_junk, member_path, normalize_member_name};

/// `archive_names`: the member-name rules, `Place` parsing with `!`,
/// `join` and `is_junk`, over each line of the input. The properties: a
/// result never holds `..`, a leading slash, a drive, NUL or a control
/// character; normalizing twice gives the same result; and a member place
/// that is printed and parsed again is equal to the original.
pub fn names(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n').take(64) {
        let _ = is_junk(line);
        if let Some(once) = normalize_member_name(line) {
            assert_safe(&once);
            assert_eq!(normalize_member_name(&once).as_deref(), Some(once.as_str()));
        }
        let limits = Limits::default();
        if let Ok(once) = member_path(line, &limits) {
            assert_safe(&once.path);
            let twice = member_path(&once.path, &limits).map(|p| p.path);
            assert_eq!(twice.as_deref(), Ok(once.path.as_str()));
            round_trip(&once.path);
        }
    }
}

/// The safe-path properties.
fn assert_safe(path: &str) {
    assert!(!path.starts_with('/'), "a leading slash in {path:?}");
    assert!(!path.contains('\\'), "a backslash in {path:?}");
    assert!(
        !path.chars().any(char::is_control),
        "a control character in {path:?}"
    );
    for part in path.split('/') {
        assert!(
            !part.is_empty() && part != "." && part != "..",
            "a bad part in {path:?}"
        );
        let bytes = part.as_bytes();
        assert!(
            !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'),
            "a drive in {path:?}"
        );
    }
}

/// Joins each part of a safe member path under an archive's root, one by
/// one as a listing does, and checks the place prints and parses back
/// equal; a part `join` refuses (an archive marker) ends the walk.
fn round_trip(path: &str) {
    let Ok(container) = Place::parse_with("/srv/course.zip", PathStyle::Unix) else {
        return;
    };
    let mut place = container.into_archive();
    for part in path.split('/') {
        match place.join(part) {
            Ok(next) => place = next,
            Err(_) => return,
        }
        let printed = place.to_string();
        let parsed = Place::parse_with(&printed, PathStyle::Unix);
        assert_eq!(parsed.as_ref(), Ok(&place), "{printed:?}");
    }
}

/// `archive_sniff`: the signature sniffer and the decision with the name
/// never panic, and agree: a decision's format is its signature's, or a
/// zip by its name when the bytes carry none. The first byte picks a
/// name; the rest are the head of the file.
pub fn sniff(data: &[u8]) {
    const NAMES: [&str; 8] = [
        "a.zip",
        "a.7z",
        "a.tar",
        "a.tar.gz",
        "a.tar.xz",
        "a.xz",
        "a.dat",
        "a.zip.001",
    ];
    let Some((&pick, head)) = data.split_first() else {
        return;
    };
    let name = NAMES[usize::from(pick) % NAMES.len()];
    let signature = crate::sniff::sniff_signature(head);
    if head.len() >= 512 {
        let _ = crate::sniff::tar_checksum_ok(&head[..512]);
    }
    if let Ok(sniffed) = crate::sniff::decide(name, head) {
        assert_eq!(sniffed.signature, signature);
        match signature {
            Some(signature) => {
                let format = signature.format();
                assert!(format.is_some());
                assert!(
                    format == Some(sniffed.format)
                        || (format == Some(crate::ArchiveFormat::Xz)
                            && sniffed.format == crate::ArchiveFormat::TarXz)
                );
            }
            None => assert_eq!(sniffed.format, crate::ArchiveFormat::Zip),
        }
    }
}

/// `archive_tar`: an uncompressed tar listed under the strict limits, its
/// index made into rows, its first 64 members read through the chain.
pub fn tar(data: &[u8]) {
    let source = crate::source::Source::from_bytes(data.to_vec());
    walk_tar(source, crate::read_tar::Wrapper::Plain);
}

/// `archive_stream`: the first byte picks the wrapper around a tar (even
/// for gzip, odd for xz), the rest is the stream, decoded
/// under the strict limits' output cap, as a tar or as one member.
pub fn stream(data: &[u8]) {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    let wrapper = if pick % 2 == 0 {
        crate::read_tar::Wrapper::Gzip
    } else {
        crate::read_tar::Wrapper::Xz {
            mem_limit_kb: crate::read_xz::mem_limit_kb(&Limits::strict()),
        }
    };
    let source = crate::source::Source::from_bytes(rest.to_vec());
    if crate::read_tar::holds_tar(&source, wrapper) {
        walk_tar(source, wrapper);
    } else {
        let limits = Limits::strict();
        let single = crate::read_tar::SingleBackend::new(source, wrapper);
        let claims = crate::guard::Claims {
            name: "fuzz".to_owned(),
            ..crate::guard::Claims::default()
        };
        let mut guarded = crate::guard::Guarded::new(
            single.read(),
            claims,
            limits,
            crate::guard::Budget::new(&limits),
            crate::guard::Allowances::default(),
            diroxide_vfs::CancelToken::new(),
        );
        let _ = std::io::copy(&mut guarded, &mut std::io::sink());
    }
}

/// Lists a tar and reads its first members, all under the strict limits.
fn walk_tar(source: crate::source::Source, wrapper: crate::read_tar::Wrapper) {
    let limits = Limits::strict();
    let ctx = diroxide_vfs::OpContext::new();
    let allow = crate::guard::Allowances::default();
    let Ok(index) = crate::read_tar::list(&source, wrapper, "fuzz.tar", &limits, &ctx, allow)
    else {
        return;
    };
    rows(&index);
    let backend = crate::read_tar::TarBackend::new(source, wrapper);
    let budget = crate::guard::Budget::new(&limits);
    for id in (0..index.len() as u32).take(64) {
        let Some(record) = index.record(id).copied() else {
            continue;
        };
        if record.kind != crate::index::RecordKind::File {
            continue;
        }
        if let Ok(reader) = backend.read(&index, id, &ctx.cancel, allow, 1 << 20) {
            drain(reader, &index, id, &record, &limits, &budget, &ctx.cancel);
        }
    }
}

/// `archive_xz`: an xz stream decoded with the strict limits' memory cap,
/// with and without several streams, under an output cap; its index
/// walked for the size, and its end and first dictionary checked. Nothing
/// may panic, hang or allocate past the cap.
pub fn xz(data: &[u8]) {
    let limits = Limits::strict();
    let source = crate::source::Source::from_bytes(data.to_vec());
    let _ = crate::read_xz::check(
        &source,
        "fuzz.xz",
        &limits,
        crate::guard::Allowances::default(),
    );
    let _ = crate::read_xz::uncompressed_size(&source, &limits);
    let cap = limits.max_member_size;
    for multiple in [true, false] {
        let reader = lzma_rust2::XzReader::new_mem_limit(
            data,
            multiple,
            crate::read_xz::mem_limit_kb(&limits),
        );
        let _ = std::io::copy(&mut std::io::Read::take(reader, cap), &mut std::io::sink());
    }
}

/// The size an xz file's indexes give, for the tests: `None` when they
/// cannot be read.
pub fn xz_size_of(data: &[u8]) -> Option<u64> {
    let source = crate::source::Source::from_bytes(data.to_vec());
    crate::read_xz::uncompressed_size(&source, &Limits::default())
}

/// `archive_7z`: a 7z opened under the strict limits, with the pre-scan of
/// its coders; an odd first byte gives a password from the next eight
/// bytes. Its index is made into rows, its first 64 members read through
/// the chain, and the whole visited in order.
pub fn sevenz(data: &[u8]) {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    let (password, bytes) = if pick % 2 == 1 && rest.len() >= 8 {
        let (password, bytes) = rest.split_at(8);
        let text = String::from_utf8_lossy(password).into_owned();
        (Some(diroxide_core::Secret::new(text)), bytes)
    } else {
        (None, rest)
    };
    let limits = Limits::strict();
    let cancel = diroxide_vfs::CancelToken::new();
    let source = crate::source::Source::from_bytes(bytes.to_vec());
    let ask = || password.clone();
    let allow = crate::guard::Allowances::default();
    let Ok((index, backend)) = crate::read_7z::open(source, "fuzz.7z", &limits, allow, &ask) else {
        return;
    };
    rows(&index);
    let budget = crate::guard::Budget::new(&limits);
    for id in (0..index.len() as u32).take(64) {
        let Some(record) = index.record(id).copied() else {
            continue;
        };
        let crate::index::Locator::SevenZ(file) = record.locator else {
            continue;
        };
        if record.kind != crate::index::RecordKind::File {
            continue;
        }
        if let Ok(reader) = backend.read(file as usize, password.clone(), cancel.clone()) {
            drain(reader, &index, id, &record, &limits, &budget, &cancel);
        }
    }
    let mut visited = 0u64;
    let _ = backend.visit(password.as_ref(), &cancel, &|_| true, &mut |_, reader| {
        visited += 1;
        let mut capped = std::io::Read::take(reader, limits.max_member_size);
        let _ = std::io::copy(&mut capped, &mut std::io::sink());
        Ok(visited < 256)
    });
}

/// `archive_zip`: a zip opened under the strict limits, its index made
/// into rows, and its first 64 members read through the limiter chain
/// with one job budget. Nothing may panic, hang or allocate from a claim.
pub fn zip(data: &[u8]) {
    let limits = Limits::strict();
    let cancel = diroxide_vfs::CancelToken::new();
    let source = crate::source::Source::from_bytes(data.to_vec());
    let Ok((index, backend)) = crate::read_zip::open(source, "fuzz.zip", &limits, &cancel) else {
        return;
    };
    rows(&index);
    let budget = crate::guard::Budget::new(&limits);
    for id in (0..index.len() as u32).take(64) {
        let Some(record) = index.record(id).copied() else {
            continue;
        };
        if record.kind != crate::index::RecordKind::File {
            continue;
        }
        let Ok(reader) = backend.read(&index, id, None, cancel.clone()) else {
            continue;
        };
        drain(reader, &index, id, &record, &limits, &budget, &cancel);
    }
}

/// Every record of an index as a row: nothing in the words may panic.
fn rows(index: &crate::index::Index) {
    for id in 0..index.len() as u32 {
        if let Some(entry) = index.entry(id) {
            let _ = crate::index::member_row(&entry);
        }
        let _ = index.path(id);
    }
    let _ = index.find("a/b");
}

/// Reads a member to its end through the chain, discarding the bytes.
fn drain(
    reader: impl std::io::Read,
    index: &crate::index::Index,
    id: u32,
    record: &crate::index::Record,
    limits: &Limits,
    budget: &crate::guard::Budget,
    cancel: &diroxide_vfs::CancelToken,
) {
    let claims = crate::guard::Claims {
        name: index.name(id).to_owned(),
        size: Some(record.size),
        packed: record.packed(),
        crc32: record
            .flags
            .contains(crate::index::RecordFlags::HAS_CRC)
            .then_some(record.crc32),
    };
    let mut guarded = crate::guard::Guarded::new(
        reader,
        claims,
        *limits,
        budget.clone(),
        crate::guard::Allowances::default(),
        cancel.clone(),
    );
    let _ = std::io::copy(&mut guarded, &mut std::io::sink());
}

/// The seeds of each target, generated, so the checked-in files under
/// `fixtures/archives/seeds/` are reproducible: a test compares them, and
/// writes them when `DIRO_WRITE_SEEDS=1` is set.
pub fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let names_lines: &[&str] = &[
        "../x",
        "a/../../x",
        "/etc/passwd",
        r"C:\win\x.dll",
        r"\\?\C:\x",
        r"\\?\\\?\C:\x",
        r"/\\?\C:\x",
        r"\??\C:\x",
        r"\\.\C:\dev",
        r"\\?\UNC\srv\share\x",
        "a/C:/x",
        "a/.. /x",
        "a\u{0}b",
        "tab\there",
        "CON",
        "a.txt:b",
        "caf\u{e9}.txt",
        "cafe\u{301}.txt",
        "inner.zip!x.txt",
        "photo\u{202e}gpj.exe",
        "PROGRA~1/x",
        "__MACOSX/._a",
        "trailing ",
        "ok/name!.txt",
    ];
    let mut seeds = vec![(
        "archive_names",
        "hostile-names.txt",
        names_lines.join("\n").into_bytes(),
    )];
    let mut tar = vec![4u8];
    tar.extend_from_slice(&[0u8; 512]);
    tar[1 + 257..1 + 262].copy_from_slice(b"ustar");
    let signatures: [(&'static str, Vec<u8>); 9] = [
        ("zip.bin", [&[0u8][..], b"PK\x03\x04"].concat()),
        (
            "seven-z.bin",
            [&[1u8][..], &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]].concat(),
        ),
        ("gzip.bin", vec![3, 0x1F, 0x8B, 0x08]),
        (
            "xz.bin",
            [&[4u8][..], &[0xFD, b'7', b'z', b'X', b'Z', 0]].concat(),
        ),
        ("bzip2.bin", [&[6u8][..], b"BZh9"].concat()),
        ("rar.bin", [&[6u8][..], b"Rar!\x1A\x07\x00"].concat()),
        ("tar.bin", tar),
        ("empty-zip.bin", [&[0u8][..], b"PK\x05\x06"].concat()),
        ("nothing.bin", vec![6, b'h', b'i']),
    ];
    for (file, bytes) in signatures {
        seeds.push(("archive_sniff", file, bytes));
    }
    let course: &[(&str, u8, &[u8])] = &[
        ("week1", ROUND_FOLDER, b""),
        ("week1/notes.txt", ROUND_FILE, b"the notes, the notes"),
        ("week1/empty.txt", ROUND_FILE, b""),
        ("caf\u{e9}.txt", ROUND_FILE, b"accents"),
        (
            "week2/deep/data.bin",
            ROUND_FILE,
            &[0, 1, 2, 3, 255, 254, 253, 0, 0, 0],
        ),
        ("../climbs.txt", ROUND_FILE, b"refused"),
        ("dup.txt", ROUND_FILE, b"first"),
        ("DUP.txt", ROUND_FILE, b"second"),
    ];
    let roundtrips: [(&'static str, u8, u8, u8); 9] = [
        ("zip.bin", 0, 0, 0),
        ("zip-locked.bin", 0, 1, ROUND_LOCKED),
        ("seven-z-solid.bin", 1, 0, ROUND_SOLID),
        ("seven-z-blocks.bin", 1, 1, 0),
        (
            "seven-z-locked.bin",
            1,
            0,
            ROUND_SOLID | ROUND_LOCKED | ROUND_NAMES,
        ),
        ("tar.bin", 2, 0, 0),
        ("tar-gz.bin", 3, 1, 0),
        ("tar-xz.bin", 4, 0, 0),
        ("xz.bin", 5, 0, 0),
    ];
    for (file, format, level, flags) in roundtrips {
        seeds.push((
            "archive_roundtrip",
            file,
            roundtrip_input(format, level, flags, course),
        ));
    }
    seeds
}

/// A round-trip entry that is a file.
pub const ROUND_FILE: u8 = 0;
/// A round-trip entry that is a folder.
pub const ROUND_FOLDER: u8 = 1;
/// A round trip into a solid 7z.
pub const ROUND_SOLID: u8 = 1;
/// A round trip with a password (zip and 7z).
pub const ROUND_LOCKED: u8 = 2;
/// A round trip with the 7z names hidden too.
pub const ROUND_NAMES: u8 = 4;

/// The input of the round-trip target for these choices: a format (0 zip,
/// 1 7z, 2 tar, 3 tar.gz, 4 tar.xz, 5 xz), a level (0 fastest, 1 fast),
/// flags ([`ROUND_SOLID`], [`ROUND_LOCKED`], [`ROUND_NAMES`]), then each
/// entry as its name's length, its name, its kind ([`ROUND_FILE`] or
/// [`ROUND_FOLDER`]), its contents' length in two bytes and its contents.
pub fn roundtrip_input(format: u8, level: u8, flags: u8, entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut out = vec![format, level, flags];
    for (name, kind, data) in entries {
        let name = name.as_bytes();
        let name = &name[..name.len().min(255)];
        out.push(name.len() as u8);
        out.extend_from_slice(name);
        out.push(*kind);
        let data = &data[..data.len().min(usize::from(u16::MAX))];
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// A secret source that always gives one password.
struct Fixed(&'static str);

impl diroxide_vfs::SecretSource for Fixed {
    fn ask(
        &self,
        _request: &diroxide_core::SecretRequest,
        _cancel: &diroxide_vfs::CancelToken,
    ) -> Option<diroxide_core::Secret> {
        Some(diroxide_core::Secret::new(self.0.to_owned()))
    }
}

/// `archive_roundtrip`: a list of entries (plain and hostile names, small
/// contents, from the input as [`roundtrip_input`] lays it out) is
/// written in a format at a level by diroxide's own writers, read back by
/// its own readers under the strict limits, and compared: every file the
/// writer took reads back whole, every folder is there, nothing else
/// is. The writers and the readers are tested against each other. Only
/// the fast levels are used, so a run stays quick and small.
pub fn roundtrip(data: &[u8]) {
    use std::collections::HashSet;
    use std::io::{Cursor, Read};

    use crate::format::ArchiveFormat;
    use crate::request::CompressionLevel;
    use crate::write::{InMemory, ItemKind, NoWatch, Records, Settings, WriteItem};

    let [format, level, flags, rest @ ..] = data else {
        return;
    };
    let format = match format % 6 {
        0 => ArchiveFormat::Zip,
        1 => ArchiveFormat::SevenZ,
        2 => ArchiveFormat::Tar,
        3 => ArchiveFormat::TarGz,
        4 => ArchiveFormat::TarXz,
        _ => ArchiveFormat::Xz,
    };
    let level = if level % 2 == 0 {
        CompressionLevel::Fastest
    } else {
        CompressionLevel::Fast
    };
    let locked =
        flags & ROUND_LOCKED != 0 && matches!(format, ArchiveFormat::Zip | ArchiveFormat::SevenZ);
    let mut items: Vec<WriteItem> = Vec::new();
    let mut contents: Vec<Vec<u8>> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut at = rest;
    while items.len() < 32 {
        let [len, tail @ ..] = at else { break };
        let len = usize::from(*len);
        if tail.len() < len + 3 {
            break;
        }
        let name = String::from_utf8_lossy(&tail[..len]).into_owned();
        let kind = tail[len];
        let size = usize::from(u16::from_le_bytes([tail[len + 1], tail[len + 2]]));
        let body = &tail[len + 3..];
        let size = size.min(body.len());
        let bytes = body[..size].to_vec();
        at = &body[size..];
        // The writer takes only safe paths, as the pack plan makes them;
        // a name that would clash with another, or sit under a file, is
        // left out, as the plan leaves it.
        let Some(path) = normalize_member_name(&name) else {
            continue;
        };
        if path.split('/').any(|part| part.len() > 200) {
            continue;
        }
        let lower = path.to_lowercase();
        let parents: Vec<String> = lower
            .match_indices('/')
            .map(|(i, _)| lower[..i].to_owned())
            .collect();
        let clashes = taken.contains(&lower)
            || parents
                .iter()
                .any(|p| taken.contains(&format!("{p}\u{0}file")))
            || taken.iter().any(|t| t.starts_with(&format!("{lower}/")));
        if clashes {
            continue;
        }
        let folder = kind & 1 == ROUND_FOLDER;
        if format == ArchiveFormat::Xz && (folder || !items.is_empty()) {
            continue;
        }
        taken.insert(lower.clone());
        if !folder {
            taken.insert(format!("{lower}\u{0}file"));
        }
        for parent in parents {
            taken.insert(parent);
        }
        items.push(WriteItem {
            name: path,
            kind: if folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            },
            size: if folder { 0 } else { bytes.len() as u64 },
            modified: Some(diroxide_core::Timestamp::from_unix(1_760_000_000, 0)),
            mode: Some(if folder { 0o755 } else { 0o644 }),
            owner: None,
        });
        contents.push(if folder { Vec::new() } else { bytes });
    }
    if items.is_empty() {
        return;
    }
    let password = locked.then_some("fuzz-password");
    let settings = Settings {
        format,
        level,
        solid: flags & ROUND_SOLID != 0,
        block_bytes: 1 << 16,
        password,
        encrypt_names: locked && flags & ROUND_NAMES != 0,
        threads: 1,
        keep_owner: false,
    };
    let records = Records::new(items.len());
    let written = crate::write::write_archive_seekable(
        Cursor::new(Vec::new()),
        &items,
        &InMemory(&contents),
        &settings,
        &NoWatch,
        &records,
    );
    let bytes = match written {
        Ok(cursor) => cursor.into_inner(),
        Err(failure) => panic!("the writer failed on a plain list: {failure:?}"),
    };
    let provider = crate::provider::ArchiveProvider::new(Limits::strict());
    let mut ctx = diroxide_vfs::OpContext::new();
    if let Some(password) = password {
        ctx = ctx.with_secrets(std::sync::Arc::new(Fixed(password)));
    }
    // A fixed name: the members' own names may hold a marker a place
    // would split at.
    let name = format!("round{}", format.extension());
    let Ok(place) = Place::parse_with(&format!("/fuzz/{name}"), PathStyle::Unix) else {
        return;
    };
    let opened = provider
        .open_bytes(
            Box::new(Cursor::new(bytes)),
            &place,
            &ctx,
            crate::guard::Allowances::all(),
        )
        .unwrap_or_else(|error| panic!("the reader refused what the writer made: {error}"));
    let index = opened.index();
    let mut files = 0usize;
    for (item, data) in items.iter().zip(&contents) {
        let path = if format == ArchiveFormat::Xz {
            crate::read_tar::single_name(&name)
        } else {
            item.name.clone()
        };
        let id = index.find(&path).unwrap_or_else(|| {
            let names: Vec<String> = (0..index.len() as u32).map(|id| index.path(id)).collect();
            panic!("{path} is missing from the {format:?} archive, which holds {names:?}")
        });
        let record = index.record(id).copied();
        match item.kind {
            ItemKind::Folder => assert_eq!(
                record.map(|r| r.kind),
                Some(crate::index::RecordKind::Folder),
                "{path} is a folder"
            ),
            _ => {
                files += 1;
                let mut reader = provider
                    .read_record(&opened, id, &ctx, crate::guard::Allowances::all(), None)
                    .unwrap_or_else(|error| panic!("{path} cannot be read: {error}"));
                let mut back = Vec::new();
                reader
                    .read_to_end(&mut back)
                    .unwrap_or_else(|error| panic!("{path} reads short: {error}"));
                assert_eq!(&back, data, "{path} reads back as written");
            }
        }
    }
    let listed = (0..index.len() as u32)
        .filter(|&id| {
            index
                .record(id)
                .is_some_and(|r| r.kind == crate::index::RecordKind::File)
        })
        .count();
    assert_eq!(listed, files, "nothing more than was written");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_seeds_hold() {
        for (target, _, bytes) in seeds() {
            match target {
                "archive_names" => names(&bytes),
                "archive_sniff" => sniff(&bytes),
                _ => {}
            }
        }
        names(b"");
        names(&[0xFF, 0xFE, b'\n', b'/', b'.', b'.']);
    }
}
