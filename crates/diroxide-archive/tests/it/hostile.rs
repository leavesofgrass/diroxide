//! The hostile archives of spike 4.1 (research/alpha2-plan.md part four),
//! built in memory, each with the sentence its case must end in: refused,
//! asked, flagged or skipped, with the reason. Hostile names live inside
//! these bytes only; nothing here creates a file with an odd name.

#![allow(dead_code)]

use crate::build::{
    Method, RawMember, RawZip, gzip, noise, quoted_overlap, sevenz_header_dictionary, sevenz_of,
    tar_header, tar_of, tar_pad, tar_pax_disagrees, xz, zip_deflated, zip_stored,
};

/// How a case must end, with words its sentence must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The archive or the member is refused; the message holds the words.
    Refused(&'static str),
    /// A question ending "y or n" is asked first; it holds the words.
    Asked(&'static str),
    /// The member is listed with a flag on its row; the row holds the
    /// words.
    Flagged(&'static str),
    /// The member is left out of the listing and counted; the skip line
    /// holds the words.
    Skipped(&'static str),
    /// Nothing is wrong: the member lists and reads.
    Fine,
}

/// What a case does with its archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Opens the archive and lists its root.
    List,
    /// Lists the folder at this member path.
    ListFolder(&'static str),
    /// Reads the member at this path to its end.
    Read(&'static str),
    /// Looks at the row of this member in its folder's listing.
    Row(&'static str),
}

/// One hostile case.
#[derive(Clone, Copy)]
pub struct Case {
    /// The case's name in reports.
    pub name: &'static str,
    /// The archive's file name (plain, always).
    pub file: &'static str,
    /// Builds the archive's bytes.
    pub bytes: fn() -> Vec<u8>,
    /// What the case does.
    pub act: Act,
    /// How it must end.
    pub expect: Expect,
    /// Whether the limit questions are answered yes, so the case shows
    /// what comes after the question.
    pub allow: bool,
}

const fn case(
    name: &'static str,
    file: &'static str,
    bytes: fn() -> Vec<u8>,
    act: Act,
    expect: Expect,
) -> Case {
    Case {
        name,
        file,
        bytes,
        act,
        expect,
        allow: false,
    }
}

const fn allowed(case: Case) -> Case {
    Case {
        allow: true,
        ..case
    }
}

/// The quoted-overlap bomb: ten members sharing one kernel of a MiB of
/// zeros.
pub fn overlap_bomb() -> Vec<u8> {
    quoted_overlap(10, 1 << 20)
}

/// A member that says it holds 4 GB over 1 KB of deflated data.
pub fn claims_four_gb() -> Vec<u8> {
    let mut member = RawMember::new("big.bin", &noise(1024, 7), Method::Deflated);
    member.size = 0xFFFF_FFF0;
    RawZip::new().member(&member).finish(b"")
}

/// A member that says it holds 10 bytes and inflates to a MiB.
pub fn claims_too_little() -> Vec<u8> {
    let mut member = RawMember::new("small.bin", &vec![b'z'; 1 << 20], Method::Deflated);
    member.size = 10;
    RawZip::new().member(&member).finish(b"")
}

/// A member whose local header names `a.txt` and whose central record
/// names `b.txt`.
pub fn names_disagree() -> Vec<u8> {
    let mut member = RawMember::new("b.txt", b"payload", Method::Stored);
    member.local_name = Some(b"a.txt".to_vec());
    RawZip::new().member(&member).finish(b"")
}

/// A 7z whose encoded header asks for a 4 GiB dictionary (the largest the
/// field holds): decoded at open, before any member.
pub fn sevenz_four_gib_dictionary() -> Vec<u8> {
    sevenz_header_dictionary(u32::MAX)
}

/// A 7z whose content block asks for a 4 GiB dictionary: decoded when a
/// member is read.
pub fn sevenz_four_gib_content_dictionary() -> Vec<u8> {
    crate::build::sevenz_content_dictionary(40)
}

/// A 7z with encrypted contents and names.
pub fn sevenz_encrypted_headers() -> Vec<u8> {
    sevenz_of(
        &[("secret.txt", b"the plans")],
        false,
        Some("canary-7f3a-not-a-password"),
    )
}

/// A tar whose PAX size (5) disagrees with its header's (0).
pub fn pax_size_disagrees() -> Vec<u8> {
    tar_pax_disagrees(5, 0)
}

/// A tar.gz of one member of 64 MiB of zeros: over 1,000 to 1.
pub fn gzip_of_zeros() -> Vec<u8> {
    gzip(&tar_of(&[("zeros.bin", &vec![0u8; 64 << 20])]))
}

/// A plain xz of zeros whose block asks for a 4 GiB dictionary: the
/// dictionary byte of its LZMA2 filter patched to 40, the header CRC fixed.
pub fn xz_block_over_cap() -> Vec<u8> {
    let mut out = xz(&vec![0u8; 1 << 16], 0);
    patch_xz_dictionary(&mut out, 40);
    out
}

/// Patches the first block's LZMA2 dictionary byte and fixes the block
/// header's CRC-32.
pub fn patch_xz_dictionary(out: &mut [u8], code: u8) {
    let block = 12;
    let header_size = (usize::from(out[block]) + 1) * 4;
    // The block header: size byte, flags, optional sizes, then the filter
    // flags: id 0x21 (LZMA2), properties size 1, the dictionary byte.
    let header = &mut out[block..block + header_size];
    let at = header
        .windows(2)
        .position(|w| w == [0x21, 0x01])
        .expect("an LZMA2 filter in the block header");
    header[at + 2] = code;
    let crc_at = header_size - 4;
    let sum = crate::build::crc(&header[..crc_at]);
    header[crc_at..].copy_from_slice(&sum.to_le_bytes());
}

/// A zip in a zip, `depth` deep, the innermost holding `a.txt`.
pub fn nested(depth: usize) -> Vec<u8> {
    let mut bytes = zip_deflated("a.txt", b"deep inside");
    for level in (1..depth).rev() {
        bytes = zip_deflated(&format!("z{}.zip", level + 1), &bytes);
    }
    bytes
}

/// Five deep: `z1.zip!z2.zip!z3.zip!z4.zip!z5.zip!a.txt`.
pub fn nested_five() -> Vec<u8> {
    nested(5)
}

/// Links out of the root, and a later member written through one.
pub fn links_out_of_root() -> Vec<u8> {
    RawZip::new()
        .member(&RawMember::symlink("up", "../../outside"))
        .member(&RawMember::symlink("link", "../../etc"))
        .member(&RawMember::new("link/passwd", b"root::0:0", Method::Stored))
        .member(&RawMember::new("plain.txt", b"fine", Method::Stored))
        .finish(b"")
}

/// The same trap in a tar: a link, then a member through it.
pub fn tar_links_out_of_root() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&crate::build::tar_symlink(b"link", b"../../etc"));
    out.extend_from_slice(&tar_header(b"link/passwd", 4, b'0'));
    out.extend_from_slice(b"root");
    tar_pad(&mut out);
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// The name list, one member each.
pub fn hostile_names() -> Vec<u8> {
    let long = format!("{}.txt", "n".repeat(300));
    let names: Vec<Vec<u8>> = vec![
        b"../x".to_vec(),
        b"/abs.txt".to_vec(),
        br"\\?\C:\x".to_vec(),
        b"a\0b.txt".to_vec(),
        b"CON".to_vec(),
        b"a.txt:b".to_vec(),
        long.into_bytes(),
        "caf\u{e9}.txt".as_bytes().to_vec(),
        "cafe\u{301}.txt".as_bytes().to_vec(),
        b"inner.zip!x.txt".to_vec(),
        b"README".to_vec(),
        b"readme".to_vec(),
        b"dup.txt".to_vec(),
        b"dup.txt".to_vec(),
        b"trailing ".to_vec(),
        "photo\u{202e}gpj.exe".as_bytes().to_vec(),
        b"__MACOSX/._junk".to_vec(),
        b"ok.txt".to_vec(),
    ];
    let mut zip = RawZip::new();
    for (i, name) in names.iter().enumerate() {
        zip = zip.member(&RawMember::raw_name(name, format!("member {i}").as_bytes()));
    }
    zip.finish(b"")
}

/// A plain archive that must simply work, as the control case.
pub fn plain_zip() -> Vec<u8> {
    crate::build::zip_of(&[("week1/", b""), ("week1/notes.txt", b"hello")])
}

/// A stored zip: the shortcut a nested archive may take.
pub fn stored_zip() -> Vec<u8> {
    zip_stored("a.txt", b"stored")
}

/// Every case of spike 4.1, with the sentence it must end in. Under
/// `Limits::strict()`: 10,000 entries, 16 MiB per member, 64 MiB in all,
/// a ratio of 100 to 1 after 1 MiB, a 16 MiB dictionary asked about and
/// 64 MiB refused, two archives deep.
pub fn cases() -> Vec<Case> {
    vec![
        case(
            "plain zip",
            "plain.zip",
            plain_zip,
            Act::Read("week1/notes.txt"),
            Expect::Fine,
        ),
        case(
            "quoted-overlap zip",
            "bomb.zip",
            overlap_bomb,
            Act::Read("0000.bin"),
            Expect::Refused("members overlap"),
        ),
        case(
            "4 GB claimed over 1 KB",
            "big.zip",
            claims_four_gb,
            Act::Read("big.bin"),
            Expect::Asked("big.bin is 4.3 GB"),
        ),
        allowed(case(
            "4 GB claimed over 1 KB, read anyway",
            "big.zip",
            claims_four_gb,
            Act::Read("big.bin"),
            Expect::Refused("shorter than its stated size"),
        )),
        case(
            "10 bytes claimed, a MiB inside",
            "small.zip",
            claims_too_little,
            Act::Read("small.bin"),
            Expect::Refused("longer than its stated size"),
        ),
        case(
            "local and central names disagree",
            "names.zip",
            names_disagree,
            Act::Read("b.txt"),
            Expect::Refused("local and central names differ"),
        ),
        case(
            "7z asking for a 4 GiB dictionary",
            "dict.7z",
            sevenz_four_gib_dictionary,
            Act::List,
            Expect::Refused("dictionary"),
        ),
        case(
            "7z content asking for a 4 GiB dictionary",
            "dict.7z",
            sevenz_four_gib_content_dictionary,
            Act::Read("a.txt"),
            Expect::Refused("dictionary"),
        ),
        case(
            "7z with encrypted headers, no password",
            "secret.7z",
            sevenz_encrypted_headers,
            Act::List,
            Expect::Refused("a password is needed"),
        ),
        case(
            "PAX size disagrees with the header",
            "pax.tar",
            pax_size_disagrees,
            Act::Row("a.txt"),
            Expect::Flagged("sizes disagree"),
        ),
        case(
            "PAX size wins when reading",
            "pax.tar",
            pax_size_disagrees,
            Act::Read("b.txt"),
            Expect::Fine,
        ),
        case(
            "gzip of zeros at over 1,000 to 1",
            "zeros.tar.gz",
            gzip_of_zeros,
            Act::List,
            Expect::Asked("Expands to"),
        ),
        case(
            "xz block over the dictionary cap",
            "big.xz",
            xz_block_over_cap,
            Act::Read("big"),
            Expect::Refused("dictionary"),
        ),
        case(
            "zip in a zip five deep",
            "z1.zip",
            nested_five,
            Act::ListFolder("z2.zip!z3.zip!z4.zip!z5.zip!"),
            Expect::Refused("the limit for nesting: 2 archives deep"),
        ),
        case(
            "a link out of the root",
            "links.zip",
            links_out_of_root,
            Act::Row("up"),
            Expect::Flagged("link to ../../outside, not followed"),
        ),
        case(
            "a member written through a link",
            "links.zip",
            links_out_of_root,
            Act::Row("link"),
            Expect::Flagged("link and folder share a name"),
        ),
        case(
            "a tar member written through a link",
            "links.tar",
            tar_links_out_of_root,
            Act::Row("link"),
            Expect::Flagged("link and folder share a name"),
        ),
        case(
            "a name that climbs out",
            "names.zip",
            hostile_names,
            Act::List,
            Expect::Skipped("Skipped ../x: name leaves the archive."),
        ),
        case(
            "an absolute name",
            "names.zip",
            hostile_names,
            Act::Row("abs.txt"),
            Expect::Flagged("leading slash removed"),
        ),
        case(
            "a long-path prefix and a drive",
            "names.zip",
            hostile_names,
            Act::Row("x"),
            Expect::Flagged("drive removed"),
        ),
        case(
            "a NUL in a name",
            "names.zip",
            hostile_names,
            Act::Row("a\u{fffd}b.txt"),
            Expect::Flagged("contains a control character"),
        ),
        case(
            "a Windows device name",
            "names.zip",
            hostile_names,
            Act::Row("CON"),
            Expect::Flagged("is a Windows device"),
        ),
        case(
            "a stream name",
            "names.zip",
            hostile_names,
            Act::Row("a.txt:b"),
            Expect::Flagged("reserved character :"),
        ),
        case(
            "a 300-byte name part",
            "names.zip",
            hostile_names,
            Act::List,
            Expect::Skipped("name part over 255 bytes"),
        ),
        case(
            "composed and decomposed twins",
            "names.zip",
            hostile_names,
            Act::Row("caf\u{e9}.txt"),
            Expect::Flagged("clashes with"),
        ),
        case(
            "a name holding an archive marker",
            "names.zip",
            hostile_names,
            Act::Row("inner.zip!x.txt"),
            Expect::Flagged("contains an archive marker"),
        ),
        case(
            "case twins",
            "names.zip",
            hostile_names,
            Act::Row("README"),
            Expect::Flagged("case clash with readme"),
        ),
        case(
            "a duplicate name",
            "names.zip",
            hostile_names,
            Act::Row("dup.txt"),
            Expect::Flagged("appears twice, last copy shown"),
        ),
        case(
            "a trailing space",
            "names.zip",
            hostile_names,
            Act::Row("trailing "),
            Expect::Flagged("ends with a space"),
        ),
        case(
            "a direction override",
            "names.zip",
            hostile_names,
            Act::Row("photo\u{202e}gpj.exe"),
            Expect::Flagged("hidden characters"),
        ),
        case(
            "junk is hidden",
            "names.zip",
            hostile_names,
            Act::ListFolder("__MACOSX"),
            Expect::Flagged("hidden"),
        ),
    ]
}
