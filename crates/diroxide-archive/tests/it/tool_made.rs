//! Made files of the formats read through 7-Zip (alpha 4, R30 commit 81):
//! each built here, in memory, by its published layout, with stored data
//! so no codec is needed: RAR 5, CAB, ARJ, LZH, RPM, cpio and ar. The
//! sniffer is held to each made file, and the tests with 7-Zip read them.

#![allow(dead_code)]

use crate::build::{crc, gzip};

/// One member: its name and bytes.
pub type Made<'a> = (&'a str, &'a [u8]);

/// The files every made archive holds: a folder's member and a top one.
pub const MEMBERS: [Made<'static>; 2] = [
    ("docs/notes.txt", b"the notes, read with 7-Zip"),
    ("readme.txt", b"read me first"),
];

/// A unix ar archive (`!<arch>`), GNU names ending in a slash.
pub fn ar(members: &[Made<'_>]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let name = name.rsplit('/').next().unwrap_or(name);
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            format!("{name}/"),
            1_762_000_000,
            0,
            0,
            "100644",
            data.len()
        );
        assert_eq!(header.len(), 60);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// A cpio archive in the "newc" form (`070701`), with its trailer.
pub fn cpio(members: &[Made<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut entry = |name: &str, mode: u32, data: &[u8], ino: u32| {
        let fields = [
            ino,
            mode,
            0,
            0,
            1,
            1_762_000_000,
            data.len() as u32,
            0,
            0,
            0,
            0,
            name.len() as u32 + 1,
            0,
        ];
        out.extend_from_slice(b"070701");
        for field in fields {
            out.extend_from_slice(format!("{field:08X}").as_bytes());
        }
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    };
    for (n, (name, data)) in members.iter().enumerate() {
        entry(name, 0o100_644, data, n as u32 + 1);
    }
    entry("TRAILER!!!", 0, b"", 0);
    out
}

/// A Cabinet file (`MSCF`) of one folder of stored data in one block.
pub fn cab(members: &[Made<'_>]) -> Vec<u8> {
    let names: Vec<String> = members.iter().map(|(n, _)| n.replace('/', "\\")).collect();
    let files_size: usize = names.iter().map(|n| 16 + n.len() + 1).sum();
    let header = 36;
    let folder = 8;
    let coff_files = header + folder;
    let coff_data = coff_files + files_size;
    let data: Vec<u8> = members.iter().flat_map(|(_, d)| d.to_vec()).collect();
    let total = coff_data + 8 + data.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"MSCF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(coff_files as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&[3, 1]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(coff_data as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut offset = 0u32;
    for ((_, bytes), name) in members.iter().zip(&names) {
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        // 2026-11-01, 12:00:00 in DOS form.
        out.extend_from_slice(&(((2026 - 1980) << 9) | (11 << 5) | 1u16).to_le_bytes());
        out.extend_from_slice(&(12u16 << 11).to_le_bytes());
        out.extend_from_slice(&0x20u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        offset += bytes.len() as u32;
    }
    assert_eq!(out.len(), coff_data);
    // A data block whose checksum is zero, which means "not checked".
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// A variable-length number of RAR 5: seven bits a byte, low first.
fn vint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A RAR 5 block: its CRC, its size, then `body` (type, flags, fields).
fn rar_block(body: &[u8], out: &mut Vec<u8>) {
    let mut sized = Vec::new();
    vint(body.len() as u64, &mut sized);
    sized.extend_from_slice(body);
    out.extend_from_slice(&crc(&sized).to_le_bytes());
    out.extend_from_slice(&sized);
}

/// A RAR 5 archive of stored files (method 0), each with its time and
/// CRC, written by the published layout (`Rar!` 1A 07 01 00).
pub fn rar5(members: &[Made<'_>]) -> Vec<u8> {
    let mut out = b"Rar!\x1A\x07\x01\x00".to_vec();
    // The main archive header: type 1, no flags, no archive flags.
    rar_block(&[1, 0, 0], &mut out);
    for (name, data) in members {
        let mut body = Vec::new();
        vint(2, &mut body);
        vint(0x0002, &mut body);
        vint(data.len() as u64, &mut body);
        vint(0x0002 | 0x0004, &mut body);
        vint(data.len() as u64, &mut body);
        vint(0o100_644, &mut body);
        body.extend_from_slice(&1_762_000_000u32.to_le_bytes());
        body.extend_from_slice(&crc(data).to_le_bytes());
        vint(0, &mut body);
        vint(1, &mut body);
        vint(name.len() as u64, &mut body);
        body.extend_from_slice(name.as_bytes());
        rar_block(&body, &mut out);
        out.extend_from_slice(data);
    }
    // The end of the archive.
    rar_block(&[5, 0, 0], &mut out);
    out
}

/// The CRC-16 of LZH (the ARC polynomial, reflected).
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// An LZH archive of stored members (`-lh0-`, level 0 headers), ending
/// in a zero byte.
pub fn lzh(members: &[Made<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in members {
        let name = name.replace('/', "\\");
        let mut header = Vec::new();
        header.extend_from_slice(b"-lh0-");
        header.extend_from_slice(&(data.len() as u32).to_le_bytes());
        header.extend_from_slice(&(data.len() as u32).to_le_bytes());
        header.extend_from_slice(
            &((12u32 << 11) | ((((2026 - 1980) << 9) | (11 << 5) | 1) << 16)).to_le_bytes(),
        );
        header.push(0x20);
        header.push(0);
        header.push(name.len() as u8);
        header.extend_from_slice(name.as_bytes());
        header.extend_from_slice(&crc16(data).to_le_bytes());
        let sum = header.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        out.push(header.len() as u8);
        out.push(sum);
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
    }
    out.push(0);
    out
}

/// One ARJ header: the id, the basic header's size, the basic header,
/// its CRC-32, and no extended header.
fn arj_header(basic: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&[0x60, 0xEA]);
    out.extend_from_slice(&(basic.len() as u16).to_le_bytes());
    out.extend_from_slice(basic);
    out.extend_from_slice(&crc(basic).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
}

/// An ARJ archive of stored members (method 0), its main header named
/// `made.arj`.
pub fn arj(members: &[Made<'_>]) -> Vec<u8> {
    let time = (12u32 << 11) | ((((2026 - 1980) << 9) | (11 << 5) | 1) << 16);
    let mut out = Vec::new();
    let mut main = vec![30, 11, 1, 0, 0, 0, 2, 0];
    main.extend_from_slice(&time.to_le_bytes());
    main.extend_from_slice(&time.to_le_bytes());
    main.extend_from_slice(&0u32.to_le_bytes());
    main.extend_from_slice(&0u32.to_le_bytes());
    main.extend_from_slice(&0u16.to_le_bytes());
    main.extend_from_slice(&0u16.to_le_bytes());
    main.extend_from_slice(&[0, 0]);
    main.extend_from_slice(b"made.arj\0\0");
    arj_header(&main, &mut out);
    for (name, data) in members {
        let mut basic = vec![30, 11, 1, 0, 0, 0, 0, 0];
        basic.extend_from_slice(&time.to_le_bytes());
        basic.extend_from_slice(&(data.len() as u32).to_le_bytes());
        basic.extend_from_slice(&(data.len() as u32).to_le_bytes());
        basic.extend_from_slice(&crc(data).to_le_bytes());
        basic.extend_from_slice(&0u16.to_le_bytes());
        basic.extend_from_slice(&0x20u16.to_le_bytes());
        basic.extend_from_slice(&[0, 0]);
        basic.extend_from_slice(name.as_bytes());
        basic.extend_from_slice(b"\0\0");
        arj_header(&basic, &mut out);
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&[0x60, 0xEA, 0, 0]);
    out
}

/// An RPM header structure with no entries: its magic, the count and
/// size zero.
fn rpm_empty_header(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0x8E, 0xAD, 0xE8, 0x01, 0, 0, 0, 0]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
}

/// An RPM package (lead `ED AB EE DB`, a signature header, a header, then
/// a gzip payload of a newc cpio), the smallest the layout allows.
pub fn rpm(members: &[Made<'_>]) -> Vec<u8> {
    let mut out = vec![0xED, 0xAB, 0xEE, 0xDB, 3, 0];
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    let mut name = b"made-1.0-1".to_vec();
    name.resize(66, 0);
    out.extend_from_slice(&name);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&5u16.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]);
    assert_eq!(out.len(), 96);
    rpm_empty_header(&mut out);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    rpm_empty_header(&mut out);
    out.extend_from_slice(&gzip(&cpio(members)));
    out
}
