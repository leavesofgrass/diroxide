//! Checksum lists, read and written (R28 3.2 "The checksum file forms";
//! R29 3.5): the GNU form (`HASH  NAME`, or `HASH *NAME` for the binary
//! marker; `SHA256SUMS`), the BSD tagged form (`SHA256 (NAME) = HASH`, as
//! GNU's `--tag` writes it), one file's list (`report.pdf.sha256` holding
//! `HASH` alone or a GNU line), and `.sfv` (`NAME CRC32`).
//!
//! Spike M5 checked each form against `sha256sum -c` (GNU coreutils 9.7)
//! and `shasum -c` in the development container:
//!
//! - `#` lines and blank lines are passed over silently by GNU, even
//!   with `--strict`; `shasum` counts a `#` line as badly formatted. This
//!   reader passes over both, and `;` lines in `.sfv`.
//! - CR LF endings are read by both; a leading byte-order mark makes GNU
//!   call the first line badly formatted. This reader takes the mark off.
//! - A line starting with a backslash has an escaped name: `\\` is a
//!   backslash, `\n` a line feed, `\r` a carriage return (GNU reads all
//!   three; `shasum` keeps `\r` as two characters). Without the leading
//!   backslash a name is taken as it is. The writer escapes the same way,
//!   and GNU reads its lines back.
//! - Upper-case digits are accepted.
//! - GNU reads `../outside.txt` and a backslash as part of a name; this
//!   reader refuses a name that leaves the list's folder or is absolute,
//!   with the name spelled out, and takes `/` and `\` both as folder
//!   separators.
//!
//! Names are relative to the list's own folder.

use diroxide_core::names::spell_out;

use super::{Algorithm, Digest};

/// The form of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListForm {
    /// `HASH  NAME` lines: `SHA256SUMS`.
    Gnu,
    /// `SHA256 (NAME) = HASH` lines.
    Bsd,
    /// One file's value, alone or as one GNU line: `report.pdf.sha256`.
    OneFile,
    /// `NAME CRC32` lines: `.sfv`.
    Sfv,
}

/// One line of a list: a name and the value it should have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    /// The line it is on, from 1.
    pub line: usize,
    /// The algorithm the line names (BSD) or the list's name says.
    pub algorithm: Option<Algorithm>,
    /// The value, lowercase hexadecimal.
    pub hex: String,
    /// The name relative to the list's folder, `/` between folders.
    pub name: String,
    /// The GNU binary marker (`*`), kept when written back.
    pub binary: bool,
}

impl ListEntry {
    /// The algorithms the value can be: the named one, or those of its
    /// length (64 digits: SHA-256 or BLAKE3).
    pub fn candidates(&self) -> Vec<Algorithm> {
        match self.algorithm {
            Some(algorithm) if algorithm.hex_len() == self.hex.len() => vec![algorithm],
            _ => Algorithm::for_length(self.hex.len()),
        }
    }

    /// The name's folders and file, in order.
    pub fn parts(&self) -> Vec<&str> {
        self.name
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect()
    }
}

/// A line that was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListProblem {
    /// The line is not in any form: "A line cannot be read: line 4".
    Unreadable {
        /// The line, from 1.
        line: usize,
    },
    /// The name leaves the list's folder or is absolute, spelled out:
    /// "Skipped a name that leaves the folder: ../x".
    LeavesFolder {
        /// The line, from 1.
        line: usize,
        /// The name, its hidden characters spelled out.
        name: String,
    },
}

impl ListProblem {
    /// The problem in words, the kind first.
    pub fn words(&self) -> String {
        match self {
            ListProblem::Unreadable { line } => format!("A line cannot be read: line {line}"),
            ListProblem::LeavesFolder { name, .. } => {
                format!("Skipped a name that leaves the folder: {name}")
            }
        }
    }
}

/// A list as read: its entries in order and the lines it passed over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChecksumList {
    /// The form most of its lines take, if any were read.
    pub form: Option<ListForm>,
    /// The entries, in order.
    pub entries: Vec<ListEntry>,
    /// The lines not taken.
    pub problems: Vec<ListProblem>,
}

/// The longest list read, in bytes: 16 MiB.
pub const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

/// Reads a list's bytes. `list_name` is the list's own file name: it
/// says the algorithm (`SHA256SUMS`, `files.md5`), whether it is `.sfv`,
/// and, for `report.pdf.sha256` holding a value alone, the file named.
pub fn parse_list(bytes: &[u8], list_name: &str) -> ChecksumList {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let bytes = &bytes[..bytes.len().min(MAX_LIST_BYTES)];
    let text = String::from_utf8_lossy(bytes);
    let named = Algorithm::from_list_name(list_name);
    let sfv = list_name.to_ascii_lowercase().ends_with(".sfv");
    let mut list = ChecksumList::default();
    let mut forms = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = index + 1;
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || (sfv && trimmed.starts_with(';')) {
            continue;
        }
        let parsed = if sfv {
            parse_sfv(raw).map(|e| (e, ListForm::Sfv))
        } else {
            parse_bsd(raw)
                .map(|e| (e, ListForm::Bsd))
                .or_else(|| parse_gnu(raw).map(|e| (e, ListForm::Gnu)))
                .or_else(|| one_file(raw, list_name, named).map(|e| (e, ListForm::OneFile)))
        };
        let Some((mut entry, form)) = parsed else {
            list.problems.push(ListProblem::Unreadable { line });
            continue;
        };
        entry.line = line;
        if entry.algorithm.is_none() {
            entry.algorithm = if sfv { Some(Algorithm::Crc32) } else { named };
        }
        match checked_name(&entry.name) {
            Some(name) => {
                entry.name = name;
                forms.push(form);
                list.entries.push(entry);
            }
            None => list.problems.push(ListProblem::LeavesFolder {
                line,
                name: spell_out(&entry.name).into_owned(),
            }),
        }
    }
    list.form = [
        ListForm::Gnu,
        ListForm::Bsd,
        ListForm::OneFile,
        ListForm::Sfv,
    ]
    .into_iter()
    .max_by_key(|f| forms.iter().filter(|g| *g == f).count())
    .filter(|_| !forms.is_empty());
    list
}

/// Whether text is all hexadecimal digits of a known length.
fn is_value(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_hexdigit())
        && !Algorithm::for_length(text.len()).is_empty()
}

/// Undoes GNU's escapes: `\\`, `\n`, `\r`. `None` for any other
/// backslash pair, which GNU would not have written.
fn unescape(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

/// A GNU line: an optional backslash (the name is escaped), the value,
/// a space, a space or a star, the name.
fn parse_gnu(raw: &str) -> Option<ListEntry> {
    let (escaped, rest) = match raw.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let space = rest.find(' ')?;
    let (hex, after) = rest.split_at(space);
    if !is_value(hex) {
        return None;
    }
    let after = &after[1..];
    let (binary, name) = match after.chars().next()? {
        '*' => (true, &after[1..]),
        ' ' => (false, &after[1..]),
        _ => return None,
    };
    let name = if escaped {
        unescape(name)?
    } else {
        name.to_owned()
    };
    (!name.is_empty()).then(|| ListEntry {
        line: 0,
        algorithm: None,
        hex: hex.to_ascii_lowercase(),
        name,
        binary,
    })
}

/// A BSD line: `TAG (NAME) = HASH`, optionally escaped as GNU does.
fn parse_bsd(raw: &str) -> Option<ListEntry> {
    let (escaped, rest) = match raw.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let open = rest.find(" (")?;
    let algorithm = Algorithm::from_tag(&rest[..open])?;
    let close = rest.rfind(") = ")?;
    if close < open + 2 {
        return None;
    }
    let name = &rest[open + 2..close];
    let hex = rest[close + 4..].trim();
    if !is_value(hex) || hex.len() != algorithm.hex_len() {
        return None;
    }
    let name = if escaped {
        unescape(name)?
    } else {
        name.to_owned()
    };
    (!name.is_empty()).then(|| ListEntry {
        line: 0,
        algorithm: Some(algorithm),
        hex: hex.to_ascii_lowercase(),
        name,
        binary: false,
    })
}

/// An `.sfv` line: the name, a space, eight digits.
fn parse_sfv(raw: &str) -> Option<ListEntry> {
    let raw = raw.trim_end();
    let space = raw.rfind([' ', '\t'])?;
    let (name, crc) = (raw[..space].trim_end(), raw[space + 1..].trim());
    if crc.len() != 8 || !crc.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
        return None;
    }
    Some(ListEntry {
        line: 0,
        algorithm: Some(Algorithm::Crc32),
        hex: crc.to_ascii_lowercase(),
        name: name.to_owned(),
        binary: false,
    })
}

/// A value alone in a one-file list, `report.pdf.sha256`: the file is the
/// list's name without its extension.
fn one_file(raw: &str, list_name: &str, named: Option<Algorithm>) -> Option<ListEntry> {
    let hex = raw.trim();
    let algorithm = named?;
    if !is_value(hex) || hex.len() != algorithm.hex_len() {
        return None;
    }
    let base = list_name.rsplit(['/', '\\']).next()?;
    let (file, ext) = base.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case(algorithm.extension()) || file.is_empty() {
        return None;
    }
    Some(ListEntry {
        line: 0,
        algorithm: Some(algorithm),
        hex: hex.to_ascii_lowercase(),
        name: file.to_owned(),
        binary: false,
    })
}

/// A name made relative and checked: `/` and `\` both separate folders;
/// `.` parts dropped; `None` when it is absolute (a leading separator, a
/// drive letter, a share) or a part is `..`.
fn checked_name(name: &str) -> Option<String> {
    if name.starts_with(['/', '\\']) {
        return None;
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let parts: Vec<&str> = name
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") {
        return None;
    }
    Some(parts.join("/"))
}

/// GNU's escape for a name holding a backslash, a line feed or a
/// carriage return: the name escaped, and whether the line needs the
/// leading backslash.
fn escape(name: &str) -> (bool, String) {
    if !name.contains(['\\', '\n', '\r']) {
        return (false, name.to_owned());
    }
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    (true, out)
}

/// Why a list could not be written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteProblem {
    /// An `.sfv` list holds CRC-32 values only.
    #[error("an SFV list holds CRC-32 values only, not {0}")]
    NotCrc(&'static str),
    /// An `.sfv` list cannot hold a name with a line break, spelled out.
    #[error("an SFV list cannot hold the name {0}")]
    SfvName(String),
    /// A one-file list holds one value.
    #[error("a one-file list holds one value, not {0}")]
    NotOne(usize),
}

/// Writes a list: one line each, LF endings, names relative to the
/// list's folder as the caller gives them, `/` between folders. A name
/// holding a backslash is escaped as GNU escapes it, and GNU reads it
/// back whole; this reader takes a backslash as a folder separator, so
/// such a name (legal on Linux only) reads back as folders. GNU and
/// one-file lines are
/// `HASH  NAME` (two spaces, as `sha256sum` writes on Linux), escaped
/// as GNU escapes; BSD lines are `TAG (NAME) = HASH`; `.sfv` lines are
/// `NAME CRC32` with upper-case digits.
pub fn write_list(entries: &[(String, Digest)], form: ListForm) -> Result<String, WriteProblem> {
    if form == ListForm::OneFile && entries.len() != 1 {
        return Err(WriteProblem::NotOne(entries.len()));
    }
    let mut out = String::new();
    for (name, digest) in entries {
        match form {
            ListForm::Gnu | ListForm::OneFile => {
                let (escaped, name) = escape(name);
                if escaped {
                    out.push('\\');
                }
                out.push_str(&format!("{}  {name}\n", digest.hex()));
            }
            ListForm::Bsd => {
                let (escaped, name) = escape(name);
                if escaped {
                    out.push('\\');
                }
                out.push_str(&format!(
                    "{} ({name}) = {}\n",
                    digest.algorithm.tag(),
                    digest.hex()
                ));
            }
            ListForm::Sfv => {
                if digest.algorithm != Algorithm::Crc32 {
                    return Err(WriteProblem::NotCrc(digest.algorithm.name()));
                }
                if name.contains(['\n', '\r']) {
                    return Err(WriteProblem::SfvName(spell_out(name).into_owned()));
                }
                out.push_str(&format!("{name} {}\n", digest.hex().to_ascii_uppercase()));
            }
        }
    }
    Ok(out)
}

/// The lists that may hold a file's value, in the order they are looked
/// for beside it (R29 3.5): its own `NAME.sha256`, `.sha512`, `.md5`,
/// `.blake3`, then `SHA256SUMS`, `SHA512SUMS`, `MD5SUMS`, `B3SUMS`, then
/// the same four ending in `.txt`.
pub fn sidecars_for(name: &str) -> Vec<String> {
    let mut names: Vec<String> = ["sha256", "sha512", "md5", "blake3"]
        .iter()
        .map(|ext| format!("{name}.{ext}"))
        .collect();
    let lists = ["SHA256SUMS", "SHA512SUMS", "MD5SUMS", "B3SUMS"];
    names.extend(lists.iter().map(|l| (*l).to_owned()));
    names.extend(lists.iter().map(|l| format!("{l}.txt")));
    names
}

/// A list's row in a Choose list: "SHA256SUMS, 12 files".
pub fn list_row(list_name: &str, list: &ChecksumList) -> String {
    format!(
        "{}, {}",
        diroxide_core::names::display_name(list_name),
        crate::words::count(list.entries.len() as u64, "file", "files")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060";
    const B: &str = "ff5d8507b6a72bee2debce2c0054798deaccdc5d8a1b945b6280ce8aa9cba52e";

    fn names(list: &ChecksumList) -> Vec<&str> {
        list.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn the_gnu_form_with_comments_markers_and_endings() {
        let text = format!(
            "\u{feff}# a comment line\r\n{A}  alpha.txt\r\n{} *beta.bin\n\n{A}  sub\\gamma.txt\n{A}  ./sub/./delta.txt\n",
            B.to_uppercase()
        );
        let list = parse_list(text.as_bytes(), "SHA256SUMS");
        assert_eq!(list.form, Some(ListForm::Gnu));
        assert_eq!(
            names(&list),
            ["alpha.txt", "beta.bin", "sub/gamma.txt", "sub/delta.txt"]
        );
        assert_eq!(list.entries[1].hex, B);
        assert!(list.entries[1].binary);
        assert_eq!(list.entries[0].algorithm, Some(Algorithm::Sha256));
        assert_eq!(list.entries[0].line, 2);
        assert!(list.problems.is_empty());
    }

    #[test]
    fn escaped_names_read_as_gnu_reads_them() {
        // Spike M5: GNU read these as back\slash.txt, new<LF>line.txt and
        // carriage<CR>return.txt; without the leading backslash the name
        // is as written. A backslash, once unescaped, separates folders
        // here, as it does in every name this reader takes.
        let text = format!(
            "\\{A}  back\\\\slash.txt\n\\{A}  new\\nline.txt\n\\{A}  carriage\\rreturn.txt\n{A}  plain\\\\kept.txt\n\\{A}  bad\\qescape\n"
        );
        let list = parse_list(text.as_bytes(), "SHA256SUMS");
        assert_eq!(
            names(&list),
            [
                "back/slash.txt",
                "new\nline.txt",
                "carriage\rreturn.txt",
                "plain/kept.txt"
            ]
        );
        assert_eq!(list.problems, [ListProblem::Unreadable { line: 5 }]);
    }

    #[test]
    fn a_name_that_leaves_the_folder_is_refused_and_spelled_out() {
        let text = format!(
            "{A}  ../outside.txt\n{A}  /etc/passwd\n{A}  C:\\Windows\\a.dll\n{A}  a/../../b\n{A}  \\\\server\\share\\x\n{A}  ..\\up.txt\n{A}  odd\u{202E}name/../../x\n"
        );
        let list = parse_list(text.as_bytes(), "SHA256SUMS");
        assert!(list.entries.is_empty(), "{:?}", list.entries);
        let words: Vec<String> = list.problems.iter().map(ListProblem::words).collect();
        assert_eq!(
            words[0],
            "Skipped a name that leaves the folder: ../outside.txt"
        );
        assert_eq!(words.len(), 7);
        assert!(words[6].contains("[U+202E]"));
    }

    #[test]
    fn bsd_one_file_and_sfv_forms() {
        let text = format!(
            "SHA256 (alpha.txt) = {A}\nMD5 (b (1).txt) = 9f9f90dbe3e5ee1218c86b8839db1995\n"
        );
        let list = parse_list(text.as_bytes(), "checksums.txt");
        assert_eq!(list.form, Some(ListForm::Bsd));
        assert_eq!(names(&list), ["alpha.txt", "b (1).txt"]);
        assert_eq!(list.entries[1].algorithm, Some(Algorithm::Md5));

        let alone = parse_list(format!("{A}\n").as_bytes(), "report.pdf.sha256");
        assert_eq!(alone.form, Some(ListForm::OneFile));
        assert_eq!(names(&alone), ["report.pdf"]);
        let as_gnu = parse_list(format!("{A} *report.pdf\n").as_bytes(), "report.pdf.sha256");
        assert_eq!(names(&as_gnu), ["report.pdf"]);

        let sfv = parse_list(
            b"; a comment\nalpha.txt 9F606EEC\nbeta.bin 7b35f858\nnot a line\n",
            "list.sfv",
        );
        assert_eq!(sfv.form, Some(ListForm::Sfv));
        assert_eq!(names(&sfv), ["alpha.txt", "beta.bin"]);
        assert_eq!(sfv.entries[0].hex, "9f606eec");
        assert_eq!(sfv.entries[0].algorithm, Some(Algorithm::Crc32));
        assert_eq!(sfv.problems, [ListProblem::Unreadable { line: 4 }]);
    }

    #[test]
    fn a_64_digit_value_with_no_name_for_its_algorithm_is_either() {
        let list = parse_list(format!("{A}  a.txt\n").as_bytes(), "checksums.txt");
        assert_eq!(list.entries[0].algorithm, None);
        assert_eq!(
            list.entries[0].candidates(),
            [Algorithm::Sha256, Algorithm::Blake3]
        );
    }

    #[test]
    fn written_lists_read_back_the_same() {
        let digest = |algorithm, hex: &str| Digest {
            algorithm,
            bytes: (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect(),
        };
        let entries = vec![
            ("alpha.txt".to_owned(), digest(Algorithm::Sha256, A)),
            ("sub/gamma.txt".to_owned(), digest(Algorithm::Sha256, B)),
            ("back\\slash".to_owned(), digest(Algorithm::Sha256, A)),
        ];
        let gnu = write_list(&entries, ListForm::Gnu).unwrap();
        assert_eq!(gnu.lines().next().unwrap(), format!("{A}  alpha.txt"));
        assert!(gnu.contains(&format!("{B}  sub/gamma.txt")));
        assert!(gnu.contains(&format!("\\{A}  back\\\\slash")));
        let read = parse_list(gnu.as_bytes(), "SHA256SUMS");
        assert_eq!(names(&read), ["alpha.txt", "sub/gamma.txt", "back/slash"]);
        let bsd = write_list(&entries[..1], ListForm::Bsd).unwrap();
        assert_eq!(bsd, format!("SHA256 (alpha.txt) = {A}\n"));
        let newline = vec![("new\nline".to_owned(), digest(Algorithm::Sha256, A))];
        let escaped = write_list(&newline, ListForm::Gnu).unwrap();
        assert_eq!(escaped, format!("\\{A}  new\\nline\n"));
        assert_eq!(
            names(&parse_list(escaped.as_bytes(), "SHA256SUMS")),
            ["new\nline"]
        );
        let crc = vec![("alpha.txt".to_owned(), digest(Algorithm::Crc32, "9f606eec"))];
        assert_eq!(
            write_list(&crc, ListForm::Sfv).unwrap(),
            "alpha.txt 9F606EEC\n"
        );
        assert_eq!(
            write_list(&entries[..1], ListForm::Sfv),
            Err(WriteProblem::NotCrc("SHA-256"))
        );
        assert_eq!(
            write_list(&entries, ListForm::OneFile),
            Err(WriteProblem::NotOne(3))
        );
    }

    #[test]
    fn sidecars_in_order_and_the_choose_row() {
        let names = sidecars_for("report.pdf");
        assert_eq!(names[0], "report.pdf.sha256");
        assert_eq!(names[3], "report.pdf.blake3");
        assert_eq!(names[4], "SHA256SUMS");
        assert_eq!(names[8], "SHA256SUMS.txt");
        let list = parse_list(format!("{A}  a\n{A}  b\n").as_bytes(), "SHA256SUMS");
        assert_eq!(list_row("SHA256SUMS", &list), "SHA256SUMS, 2 files");
        assert_eq!(
            ListProblem::Unreadable { line: 4 }.words(),
            "A line cannot be read: line 4"
        );
    }
}
