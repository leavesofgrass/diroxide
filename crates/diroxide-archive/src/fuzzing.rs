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
    let _ = backend.visit(password.as_ref(), &cancel, &mut |_, reader| {
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
    seeds
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
