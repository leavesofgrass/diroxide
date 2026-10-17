//! Archives built in memory for the tests: ordinary ones through the
//! format crates' own writers, and hostile ones byte by byte. Nothing here
//! writes a file with an odd name; an archive that holds hostile names is
//! bytes in memory, or a file with a plain name in a temporary folder.

#![allow(dead_code)]

use std::io::{Cursor, Write};

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder};

/// The CRC-32 of some bytes, as zip, 7z and xz store it.
pub fn crc(data: &[u8]) -> u32 {
    let mut hasher = flate2::Crc::new();
    hasher.update(data);
    hasher.sum()
}

/// Raw deflate of some bytes, at the default level.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// A gzip stream of some bytes.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// An xz stream of some bytes at a preset, with the default check.
pub fn xz(data: &[u8], preset: u32) -> Vec<u8> {
    let options = lzma_rust2::XzOptions::with_preset(preset);
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), options).unwrap();
    writer.write_all(data).unwrap();
    writer.finish().unwrap()
}

/// Bytes that do not compress: a fixed pseudo-random sequence.
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// How a member is stored in a raw zip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Stored as it is.
    Stored,
    /// Deflated.
    Deflated,
}

/// One member of a raw zip, with every field the tests may need to lie
/// about.
#[derive(Clone, Debug)]
pub struct RawMember {
    /// The name in the central directory.
    pub name: Vec<u8>,
    /// The name in the local header, when it differs.
    pub local_name: Option<Vec<u8>>,
    /// The bytes stored after the local header, already compressed.
    pub data: Vec<u8>,
    /// The method field.
    pub method: u16,
    /// The CRC field.
    pub crc: u32,
    /// The uncompressed size field.
    pub size: u32,
    /// The general-purpose flags (bit 11: the name is UTF-8).
    pub flags: u16,
    /// The high byte of "version made by": 0 for DOS, 3 for Unix.
    pub system: u8,
    /// The external attributes (a Unix mode in the high half).
    pub external: u32,
}

impl RawMember {
    /// A member with its contents, stored or deflated, all fields honest.
    pub fn new(name: &str, contents: &[u8], method: Method) -> Self {
        let (data, method) = match method {
            Method::Stored => (contents.to_vec(), 0),
            Method::Deflated => (deflate(contents), 8),
        };
        Self {
            name: name.as_bytes().to_vec(),
            local_name: None,
            data,
            method,
            crc: crc(contents),
            size: contents.len() as u32,
            flags: 1 << 11,
            system: 3,
            external: 0o100644 << 16,
        }
    }

    /// A member with a raw name, stored, honest.
    pub fn raw_name(name: &[u8], contents: &[u8]) -> Self {
        let mut member = Self::new("x", contents, Method::Stored);
        member.name = name.to_vec();
        member
    }

    /// A folder entry: a name ending in a slash and no data.
    pub fn folder(name: &str) -> Self {
        let mut member = Self::new(name, b"", Method::Stored);
        member.external = (0o040755 << 16) | 0x10;
        member
    }

    /// A symbolic link to `target` (a Unix mode, the target as data).
    pub fn symlink(name: &str, target: &str) -> Self {
        let mut member = Self::new(name, target.as_bytes(), Method::Stored);
        member.external = 0o120777 << 16;
        member
    }
}

/// A zip written byte by byte, so the tests can make every kind of lie.
#[derive(Default)]
pub struct RawZip {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u64,
}

impl RawZip {
    /// An empty raw zip.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes before the first member (a self-extracting stub).
    pub fn prefix(mut self, bytes: &[u8]) -> Self {
        self.out.extend_from_slice(bytes);
        self
    }

    /// The local header of a member, as bytes.
    pub fn local_header(member: &RawMember) -> Vec<u8> {
        let name = member.local_name.as_ref().unwrap_or(&member.name);
        let mut h = Vec::with_capacity(30 + name.len());
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes());
        h.extend_from_slice(&member.flags.to_le_bytes());
        h.extend_from_slice(&member.method.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&0x5021u16.to_le_bytes());
        h.extend_from_slice(&member.crc.to_le_bytes());
        h.extend_from_slice(&(member.data.len() as u32).to_le_bytes());
        h.extend_from_slice(&member.size.to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name);
        h
    }

    /// The central record of a member whose local header is at `offset`.
    fn central_record(member: &RawMember, offset: u64, compressed: u64) -> Vec<u8> {
        let mut c = Vec::with_capacity(46 + member.name.len());
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&((u16::from(member.system) << 8) | 20).to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&member.flags.to_le_bytes());
        c.extend_from_slice(&member.method.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0x5021u16.to_le_bytes());
        c.extend_from_slice(&member.crc.to_le_bytes());
        c.extend_from_slice(&(compressed as u32).to_le_bytes());
        c.extend_from_slice(&member.size.to_le_bytes());
        c.extend_from_slice(&(member.name.len() as u16).to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&member.external.to_le_bytes());
        c.extend_from_slice(&(offset.min(0xFFFF_FFFF) as u32).to_le_bytes());
        c.extend_from_slice(&member.name);
        c
    }

    /// Adds a member: its local header, its data, and its central record.
    pub fn member(mut self, member: &RawMember) -> Self {
        let offset = self.out.len() as u64;
        self.out.extend_from_slice(&Self::local_header(member));
        self.out.extend_from_slice(&member.data);
        let record = Self::central_record(member, offset, member.data.len() as u64);
        self.central.extend_from_slice(&record);
        self.count += 1;
        self
    }

    /// Adds only a central record, pointing at bytes already written at
    /// `offset` with a compressed size of `compressed`: the overlap.
    pub fn central_only(mut self, member: &RawMember, offset: u64, compressed: u64) -> Self {
        self.central
            .extend_from_slice(&Self::central_record(member, offset, compressed));
        self.count += 1;
        self
    }

    /// Writes raw bytes where the next member would go.
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.out.extend_from_slice(bytes);
        self
    }

    /// Where the next byte goes.
    pub fn position(&self) -> u64 {
        self.out.len() as u64
    }

    /// The archive, with the true entry count.
    pub fn finish(self, comment: &[u8]) -> Vec<u8> {
        let count = self.count;
        self.finish_claiming(comment, count)
    }

    /// The archive, with the end record claiming `claimed` entries.
    pub fn finish_claiming(mut self, comment: &[u8], claimed: u64) -> Vec<u8> {
        let cd_offset = self.out.len() as u64;
        let cd_size = self.central.len() as u64;
        let central = std::mem::take(&mut self.central);
        self.out.extend_from_slice(&central);
        let zip64 = claimed > 0xFFFE || cd_offset > 0xFFFF_FFFE;
        if zip64 {
            let record_at = self.out.len() as u64;
            self.out.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
            self.out.extend_from_slice(&44u64.to_le_bytes());
            self.out.extend_from_slice(&45u16.to_le_bytes());
            self.out.extend_from_slice(&45u16.to_le_bytes());
            self.out.extend_from_slice(&0u32.to_le_bytes());
            self.out.extend_from_slice(&0u32.to_le_bytes());
            self.out.extend_from_slice(&claimed.to_le_bytes());
            self.out.extend_from_slice(&claimed.to_le_bytes());
            self.out.extend_from_slice(&cd_size.to_le_bytes());
            self.out.extend_from_slice(&cd_offset.to_le_bytes());
            self.out.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
            self.out.extend_from_slice(&0u32.to_le_bytes());
            self.out.extend_from_slice(&record_at.to_le_bytes());
            self.out.extend_from_slice(&1u32.to_le_bytes());
        }
        let short = |n: u64| if zip64 { 0xFFFF } else { n as u16 };
        let long = |n: u64| if zip64 { 0xFFFF_FFFF } else { n as u32 };
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&short(claimed).to_le_bytes());
        self.out.extend_from_slice(&short(claimed).to_le_bytes());
        self.out.extend_from_slice(&long(cd_size).to_le_bytes());
        self.out.extend_from_slice(&long(cd_offset).to_le_bytes());
        self.out
            .extend_from_slice(&(comment.len() as u16).to_le_bytes());
        self.out.extend_from_slice(comment);
        self.out
    }
}

/// A zip of plain members through the `zip` crate's own writer, deflated,
/// with a fixed time; a name ending in a slash is a folder.
pub fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::from_date_and_time(2026, 10, 16, 12, 0, 0).unwrap());
    for (name, data) in members {
        if let Some(folder) = name.strip_suffix('/') {
            writer.add_directory(folder, options).unwrap();
        } else {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

/// A zip of one stored member: the shape a nested archive takes when its
/// packer did not compress it.
pub fn zip_stored(name: &str, data: &[u8]) -> Vec<u8> {
    RawZip::new()
        .member(&RawMember::new(name, data, Method::Stored))
        .finish(b"")
}

/// A zip of one deflated member.
pub fn zip_deflated(name: &str, data: &[u8]) -> Vec<u8> {
    RawZip::new()
        .member(&RawMember::new(name, data, Method::Deflated))
        .finish(b"")
}

/// The quoted-overlap zip (Fifield, USENIX WOOT 2019, the non-recursive
/// bomb): each member's deflate stream quotes the next member's local
/// header in a stored block, then runs on into that member's data, so
/// every member shares the one kernel at the end. `files` members, the
/// kernel `kernel` zero bytes.
pub fn quoted_overlap(files: usize, kernel: usize) -> Vec<u8> {
    assert!(files >= 1);
    let zeros = vec![0u8; kernel];
    // Member i's output and its compressed data, built from the last back.
    let mut outputs = vec![Vec::new(); files];
    let mut datas = vec![Vec::new(); files];
    let mut headers = vec![Vec::new(); files];
    outputs[files - 1] = zeros;
    datas[files - 1] = deflate(&outputs[files - 1]);
    for i in (0..files).rev() {
        let member = RawMember {
            name: format!("{i:04}.bin").into_bytes(),
            local_name: None,
            data: datas[i].clone(),
            method: 8,
            crc: crc(&outputs[i]),
            size: outputs[i].len() as u32,
            flags: 0,
            system: 0,
            external: 0,
        };
        headers[i] = RawZip::local_header(&member);
        if i > 0 {
            // Member i-1 quotes member i's header in a stored block (not
            // the final block), then continues into member i's data.
            let header = &headers[i];
            let len = header.len() as u16;
            let mut quoted = vec![0x00];
            quoted.extend_from_slice(&len.to_le_bytes());
            quoted.extend_from_slice(&(!len).to_le_bytes());
            quoted.extend_from_slice(header);
            quoted.extend_from_slice(&datas[i]);
            datas[i - 1] = quoted;
            let mut output = header.clone();
            output.extend_from_slice(&outputs[i]);
            outputs[i - 1] = output;
        }
    }
    // The first member's header has to be rebuilt now that its data and
    // output are known.
    let first = RawMember {
        name: b"0000.bin".to_vec(),
        local_name: None,
        data: datas[0].clone(),
        method: 8,
        crc: crc(&outputs[0]),
        size: outputs[0].len() as u32,
        flags: 0,
        system: 0,
        external: 0,
    };
    headers[0] = RawZip::local_header(&first);
    let mut zip = RawZip::new().raw(&headers[0]).raw(&datas[0]);
    // Member i's local header sits inside member i-1's data, after the
    // five-byte stored-block header that quotes it.
    let mut offset = 0u64;
    for i in 0..files {
        let member = RawMember {
            name: format!("{i:04}.bin").into_bytes(),
            local_name: None,
            data: Vec::new(),
            method: 8,
            crc: crc(&outputs[i]),
            size: outputs[i].len() as u32,
            flags: 0,
            system: 0,
            external: 0,
        };
        zip = zip.central_only(&member, offset, datas[i].len() as u64);
        offset += headers[i].len() as u64 + 5;
    }
    zip.finish(b"")
}

/// A tar through the `tar` crate's writer, GNU headers, fixed times; a
/// name ending in a slash is a folder.
pub fn tar_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in members {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(1_760_000_000);
        if let Some(folder) = name.strip_suffix('/') {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            builder
                .append_data(&mut header, format!("{folder}/"), std::io::empty())
                .unwrap();
        } else {
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, name, *data).unwrap();
        }
    }
    builder.into_inner().unwrap()
}

/// A tar header block written by hand: name, size field and type, with a
/// correct checksum.
pub fn tar_header(name: &[u8], size_field: u64, kind: u8) -> [u8; 512] {
    let mut h = [0u8; 512];
    let n = name.len().min(100);
    h[..n].copy_from_slice(&name[..n]);
    h[100..107].copy_from_slice(b"0000644");
    h[108..115].copy_from_slice(b"0000000");
    h[116..123].copy_from_slice(b"0000000");
    let size = format!("{size_field:011o}");
    h[124..135].copy_from_slice(size.as_bytes());
    h[136..147].copy_from_slice(b"15066011400");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    let field = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(field.as_bytes());
    h
}

/// A tar header for a symbolic link to `target`.
pub fn tar_symlink(name: &[u8], target: &[u8]) -> [u8; 512] {
    let mut h = tar_header(name, 0, b'2');
    h[157..157 + target.len().min(100)].copy_from_slice(&target[..target.len().min(100)]);
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// Pads data to a whole number of 512-byte blocks.
pub fn tar_pad(out: &mut Vec<u8>) {
    let rem = out.len() % 512;
    if rem != 0 {
        out.resize(out.len() + 512 - rem, 0);
    }
}

/// A PAX record line: "LEN key=value\n" with LEN counting itself.
pub fn pax_record(key: &str, value: &str) -> String {
    let body = format!(" {key}={value}\n");
    let mut len = body.len() + 1;
    loop {
        let text = format!("{len}{body}");
        if text.len() == len {
            return text;
        }
        len = text.len();
    }
}

/// A tar whose PAX header says the member is `pax_size` bytes while its
/// own header says `header_size`; the data is `pax_size` bytes, and a
/// second member follows.
pub fn tar_pax_disagrees(pax_size: u64, header_size: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let records = pax_record("size", &pax_size.to_string());
    out.extend_from_slice(&tar_header(b"PaxHeaders/a.txt", records.len() as u64, b'x'));
    out.extend_from_slice(records.as_bytes());
    tar_pad(&mut out);
    out.extend_from_slice(&tar_header(b"a.txt", header_size, b'0'));
    out.extend(std::iter::repeat_n(b'a', pax_size as usize));
    tar_pad(&mut out);
    out.extend_from_slice(&tar_header(b"b.txt", 2, b'0'));
    out.extend_from_slice(b"bb");
    tar_pad(&mut out);
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// A 7z through the `sevenz-rust2` writer: members with contents (a name
/// ending in a slash is a folder), solid or one block each, with an
/// optional password, which also encrypts the header.
pub fn sevenz_of(members: &[(&str, &[u8])], solid: bool, password: Option<&str>) -> Vec<u8> {
    use sevenz_rust2::encoder_options::AesEncoderOptions;
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    let mut methods = Vec::new();
    if let Some(password) = password {
        methods.push(
            EncoderConfiguration::new(EncoderMethod::AES256_SHA256).with_options(
                AesEncoderOptions::new(sevenz_rust2::Password::from(password)).into(),
            ),
        );
        writer.set_encrypt_header(true);
    }
    methods.push(EncoderConfiguration::new(EncoderMethod::LZMA2));
    writer.set_content_methods(methods);
    if solid {
        let mut entries = Vec::new();
        let mut readers = Vec::new();
        for (name, data) in members {
            entries.push(ArchiveEntry::new_file(name));
            readers.push(sevenz_rust2::SourceReader::new(Cursor::new(data.to_vec())));
        }
        writer.push_archive_entries(entries, readers).unwrap();
    } else {
        for (name, data) in members {
            if let Some(folder) = name.strip_suffix('/') {
                writer
                    .push_archive_entry::<&[u8]>(ArchiveEntry::new_directory(folder), None)
                    .unwrap();
            } else {
                writer
                    .push_archive_entry(ArchiveEntry::new_file(name), Some(*data))
                    .unwrap();
            }
        }
    }
    writer.finish().unwrap().into_inner()
}

/// Rewrites the start header's next-header CRC and its own CRC after the
/// next header was patched in place.
pub fn sevenz_fix_crcs(archive: &mut [u8]) {
    let offset = u64::from_le_bytes(archive[12..20].try_into().unwrap()) as usize;
    let size = u64::from_le_bytes(archive[20..28].try_into().unwrap()) as usize;
    let next = crc(&archive[32 + offset..32 + offset + size]);
    archive[28..32].copy_from_slice(&next.to_le_bytes());
    let start = crc(&archive[12..32]);
    archive[8..12].copy_from_slice(&start.to_le_bytes());
}

/// The next header of a 7z, in place.
fn next_header(archive: &mut [u8]) -> &mut [u8] {
    let offset = u64::from_le_bytes(archive[12..20].try_into().unwrap()) as usize;
    let size = u64::from_le_bytes(archive[20..28].try_into().unwrap()) as usize;
    &mut archive[32 + offset..32 + offset + size]
}

/// A 7z whose encoded header's LZMA coder asks for a dictionary of
/// `dictionary` bytes: the header is what a reader decodes first, at open.
/// Three hundred long names make the writer compress its header.
pub fn sevenz_header_dictionary(dictionary: u32) -> Vec<u8> {
    let names: Vec<String> = (0..300)
        .map(|i| format!("folder/a-rather-long-file-name-{i:04}.txt"))
        .collect();
    let members: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let mut archive = sevenz_of(&members, true, None);
    let header = next_header(&mut archive);
    assert_eq!(header[0], 0x17, "the writer compressed its header");
    // The coder record: flags 0x23 (a 3-byte id with properties), the
    // LZMA id 03 01 01, five property bytes.
    let at = header
        .windows(5)
        .position(|w| w == [0x23, 0x03, 0x01, 0x01, 0x05])
        .unwrap_or_else(|| panic!("no LZMA coder in the encoded header: {header:02x?}"));
    header[at + 6..at + 10].copy_from_slice(&dictionary.to_le_bytes());
    sevenz_fix_crcs(&mut archive);
    archive
}

/// A 7z with a plain header whose content block's LZMA2 coder asks for
/// the dictionary of code `code` (40 is the largest, 4 GiB less one).
pub fn sevenz_content_dictionary(code: u8) -> Vec<u8> {
    let mut archive = sevenz_of(&[("a.txt", b"hello")], false, None);
    let header = next_header(&mut archive);
    assert_eq!(header[0], 0x01, "a small header stays plain");
    // Flags 0x21 (a 1-byte id with properties), LZMA2 id 0x21, one
    // property byte.
    let at = header
        .windows(3)
        .position(|w| w == [0x21, 0x21, 0x01])
        .unwrap_or_else(|| panic!("no LZMA2 coder in the header: {header:02x?}"));
    header[at + 3] = code;
    sevenz_fix_crcs(&mut archive);
    archive
}

/// A 7z of empty files written by hand, with a plain (not encoded) header:
/// the fast way to a 7z of a million entries for spike M1.
pub fn sevenz_empty_files(names: impl Iterator<Item = String>) -> Vec<u8> {
    let mut utf16 = Vec::new();
    let mut count = 0u64;
    for name in names {
        for unit in name.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        utf16.extend_from_slice(&[0, 0]);
        count += 1;
    }
    let bits = vec![0xFFu8; count.div_ceil(8) as usize];
    // kHeader, kFilesInfo, the count.
    let mut header = vec![0x01, 0x05];
    write_number(&mut header, count);
    // kEmptyStream and kEmptyFile: every entry is an empty file.
    for property in [0x0E, 0x0F] {
        header.push(property);
        write_number(&mut header, bits.len() as u64);
        header.extend_from_slice(&bits);
    }
    // kName: not external, then the names.
    header.push(0x11);
    write_number(&mut header, utf16.len() as u64 + 1);
    header.push(0);
    header.extend_from_slice(&utf16);
    // The end of the files info, then of the header.
    header.push(0x00);
    header.push(0x00);
    let mut out = vec![0u8; 32];
    out[..6].copy_from_slice(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]);
    out[7] = 4;
    out[12..20].copy_from_slice(&0u64.to_le_bytes());
    out[20..28].copy_from_slice(&(header.len() as u64).to_le_bytes());
    out.extend_from_slice(&header);
    sevenz_fix_crcs(&mut out);
    out
}

/// 7z's variable-length number: a first byte whose high bits count the
/// extra bytes, then those bytes, little end first.
pub fn write_number(out: &mut Vec<u8>, value: u64) {
    let mut extra = 0usize;
    while extra < 8 && value >= (1u64 << (7 * (extra + 1))) {
        extra += 1;
    }
    if extra == 8 {
        out.push(0xFF);
        out.extend_from_slice(&value.to_le_bytes());
        return;
    }
    let mask = !(0xFFu8 >> extra);
    let high = if extra < 8 {
        ((value >> (8 * extra)) as u8) & (0xFFu8 >> (extra + 1))
    } else {
        0
    };
    out.push(mask | high);
    for i in 0..extra {
        out.push((value >> (8 * i)) as u8);
    }
}

/// The traditional PKWARE stream cipher, for a ZipCrypto fixture: the zip
/// crate reads it and, rightly, will not write it.
struct ZipCrypto {
    keys: [u32; 3],
    table: [u32; 256],
}

impl ZipCrypto {
    fn new(password: &[u8]) -> Self {
        let mut table = [0u32; 256];
        for (n, slot) in table.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        let mut cipher = Self {
            keys: [0x1234_5678, 0x2345_6789, 0x3456_7890],
            table,
        };
        for &b in password {
            cipher.update(b);
        }
        cipher
    }

    fn crc_step(&self, crc: u32, b: u8) -> u32 {
        (crc >> 8) ^ self.table[((crc ^ u32::from(b)) & 0xFF) as usize]
    }

    fn update(&mut self, plain: u8) {
        self.keys[0] = self.crc_step(self.keys[0], plain);
        self.keys[1] = self.keys[1]
            .wrapping_add(self.keys[0] & 0xFF)
            .wrapping_mul(134_775_813)
            .wrapping_add(1);
        self.keys[2] = self.crc_step(self.keys[2], (self.keys[1] >> 24) as u8);
    }

    fn encrypt(&mut self, plain: u8) -> u8 {
        let temp = (self.keys[2] | 2) & 0xFFFF;
        let key = ((temp * (temp ^ 1)) >> 8) as u8;
        self.update(plain);
        plain ^ key
    }
}

/// A zip of one deflated member under ZipCrypto with `password`.
pub fn zipcrypto_zip(name: &str, contents: &[u8], password: &str) -> Vec<u8> {
    let mut member = RawMember::new(name, contents, Method::Deflated);
    let mut cipher = ZipCrypto::new(password.as_bytes());
    let mut header: Vec<u8> = noise(11, 3);
    header.push((member.crc >> 24) as u8);
    let mut data: Vec<u8> = header.iter().map(|&b| cipher.encrypt(b)).collect();
    data.extend(member.data.iter().map(|&b| cipher.encrypt(b)));
    member.data = data;
    member.flags |= 1;
    RawZip::new().member(&member).finish(b"")
}
