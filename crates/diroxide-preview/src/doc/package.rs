//! The container read path (alpha 4, R30 commit 36; R28 part 2.2): a
//! zip-based document (DOCX, EPUB, ODT, XLSX, ODS) cannot be read from
//! the quick view's first megabyte, so it is read through a seekable
//! source with caps of its own ([`ContainerCaps`]): 64 MiB for the
//! container, 50,000 members, 32 MiB for one unpacked XML part, 64 MiB
//! for any other part, 256 MiB unpacked in all, the nesting cut at 256
//! levels, 10,000 table rows; a sheet also 200 columns and a 50 MB file. Every reader runs under the same guard as
//! the quick view's (a thread, a catch, a deadline).
//!
//! Before any reader sees the package, [`check`] walks it: too many
//! members, members whose bytes overlap (the same compressed bytes read
//! as many files, a way to build a bomb), a member that claims more than
//! a thousand times its packed size, and a member that unpacks past its
//! cap (counted while unpacking, never trusted from the header) are all
//! refused in words. An XML part nested deeper than the cut is flattened
//! ([`super::xmldepth`]) and the package rewritten with it, so no reader
//! ever recurses past the cut. The member rules are textweaver's
//! (`package.rs`, carried under the owner's relicensing, answer 41).

use std::io::{self, Cursor, Read, SeekFrom, Write};

use diroxide_vfs::ReadSeek;

use super::xmldepth::{FLAT_ELEMENT, MAX_NESTING, NESTING_NOTE, deepest, limit_depth_with};
use super::{DocError, DocView, ReadOptions};

/// The caps of the container read path. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerCaps {
    /// The largest container read, in bytes.
    pub max_container: u64,
    /// The most members.
    pub max_members: usize,
    /// The largest unpacked XML part.
    pub max_part: u64,
    /// The largest unpacked part of any other kind (a picture).
    pub max_member: u64,
    /// The most bytes unpacked from all the parts.
    pub max_total: u64,
    /// How deep an XML part may nest before it is flattened.
    pub max_nesting: usize,
    /// The most rows of one table or sheet.
    pub max_table_rows: usize,
    /// The most columns of one sheet or delimited table.
    pub max_columns: usize,
    /// The largest spreadsheet or delimited file loaded (50 MB).
    pub max_sheet_bytes: u64,
}

impl Default for ContainerCaps {
    fn default() -> Self {
        Self {
            max_container: 64 * 1024 * 1024,
            max_members: 50_000,
            max_part: 32 * 1024 * 1024,
            max_member: 64 * 1024 * 1024,
            max_total: 256 * 1024 * 1024,
            max_nesting: MAX_NESTING,
            max_table_rows: 10_000,
            max_columns: 200,
            max_sheet_bytes: 50_000_000,
        }
    }
}

/// The highest ratio of unpacked to packed size a member larger than
/// [`RATIO_FLOOR`] may claim: XML packs ten to fifty times, deflate
/// cannot pass about 1,030, but bzip2 can.
pub const MAX_RATIO: u64 = 1_000;

/// Members smaller than this (1 MiB) are not held to [`MAX_RATIO`].
pub const RATIO_FLOOR: u64 = 1024 * 1024;

/// A package that passed [`check`].
#[derive(Debug)]
pub struct Checked {
    /// The package's bytes, rewritten when a part was flattened.
    pub bytes: Vec<u8>,
    /// Whether an XML part was flattened.
    pub flattened: bool,
    /// How many members it has.
    pub members: usize,
    /// How many bytes its parts unpack to.
    pub unpacked: u64,
}

/// Whether a member is an XML part, which readers parse and the nesting
/// cut applies to.
pub fn is_xml_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".xml", ".rels", ".opf", ".ncx", ".xhtml", ".html", ".htm"]
        .iter()
        .any(|e| lower.ends_with(e))
}

fn refused(words: impl Into<String>) -> DocError {
    DocError::Refused(words.into())
}

/// Walks a package's members against the caps, unpacking each once to
/// count it, and flattens the XML parts nested past the cut. `what` names
/// the format for "not valid".
pub fn check(bytes: Vec<u8>, caps: &ContainerCaps, what: &str) -> Result<Checked, DocError> {
    let not_valid = || DocError::NotValid(what.to_owned());
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(|_| not_valid())?;
    let members = zip.len();
    if members > caps.max_members {
        return Err(refused(format!(
            "it holds more than {} files",
            diroxide_core::format::grouped(caps.max_members as u64)
        )));
    }
    let mut extents = Vec::with_capacity(members);
    for n in 0..members {
        let member = zip.by_index_raw(n).map_err(|_| not_valid())?;
        let (size, packed) = (member.size(), member.compressed_size());
        let xml = member.name().is_ok_and(|name| is_xml_part(&name));
        let cap = if xml { caps.max_part } else { caps.max_member };
        if size > cap {
            return Err(refused("a part is too large to read"));
        }
        if size > RATIO_FLOOR && size / packed.max(1) > MAX_RATIO {
            return Err(refused(
                "a part claims to unpack to over 1,000 times its size",
            ));
        }
        extents.push((member.header_start(), packed));
    }
    extents.sort_unstable();
    if extents
        .windows(2)
        .any(|w| w[0].0.saturating_add(w[0].1) > w[1].0)
    {
        return Err(refused("its parts overlap"));
    }
    let mut unpacked = 0u64;
    let mut flat: Vec<(usize, String, String)> = Vec::new();
    for n in 0..members {
        let mut member = zip.by_index(n).map_err(|_| not_valid())?;
        if member.is_dir() {
            continue;
        }
        let name = member.name().map_err(|_| not_valid())?.into_owned();
        let xml = is_xml_part(&name);
        let cap = if xml { caps.max_part } else { caps.max_member };
        let left = caps.max_total.saturating_sub(unpacked);
        let read = if xml {
            let mut part = Vec::new();
            (&mut member)
                .take(cap.min(left).saturating_add(1))
                .read_to_end(&mut part)
                .map_err(|_| not_valid())?;
            let count = part.len() as u64;
            if count <= cap.min(left)
                && let Ok(text) = String::from_utf8(part)
                && deepest(&text) > caps.max_nesting
            {
                let (open, close) = flat_wrapper(&name, &text);
                if let Some(flattened) = limit_depth_with(&text, caps.max_nesting, &open, &close) {
                    flat.push((n, name, flattened));
                }
            }
            count
        } else {
            io::copy(
                &mut (&mut member).take(cap.min(left).saturating_add(1)),
                &mut io::sink(),
            )
            .map_err(|_| not_valid())?
        };
        if read > cap {
            return Err(refused("a part unpacks to more than it says"));
        }
        unpacked += read;
        if unpacked > caps.max_total {
            return Err(refused(format!(
                "it unpacks to more than {}",
                diroxide_core::format::human_size(caps.max_total)
            )));
        }
    }
    drop(zip);
    if flat.is_empty() {
        return Ok(Checked {
            bytes,
            flattened: false,
            members,
            unpacked,
        });
    }
    let rewritten = rewrite(&bytes, &flat).map_err(|_| not_valid())?;
    Ok(Checked {
        bytes: rewritten,
        flattened: true,
        members,
        unpacked,
    })
}

/// The prefix of an XML part's root element: `w` for `<w:document>`.
fn root_prefix(text: &str) -> Option<&str> {
    let mut rest = text;
    loop {
        let at = rest.find('<')?;
        rest = &rest[at + 1..];
        if rest.starts_with(['?', '!']) {
            continue;
        }
        let name = rest
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()?;
        return name.split_once(':').map(|(prefix, _)| prefix);
    }
}

/// The prefix a part binds to a namespace: `text` for OpenDocument's
/// text namespace.
fn prefix_of<'a>(text: &'a str, namespace: &str) -> Option<&'a str> {
    let at = text.find(&format!("=\"{namespace}\""))?;
    let before = &text[..at];
    let start = before.rfind("xmlns:")? + "xmlns:".len();
    Some(&before[start..]).filter(|p| !p.is_empty() && !p.contains(char::is_whitespace))
}

/// The wrapper a flattened part's leaves go in, so the format's reader
/// takes them as its own text: a Word part's in a paragraph and a run, an
/// OpenDocument part's in a paragraph, a page's in a paragraph; any other
/// part's in [`FLAT_ELEMENT`].
fn flat_wrapper(name: &str, text: &str) -> (String, String) {
    const ODF_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("word/") {
        let w = root_prefix(text).unwrap_or("w");
        return (format!("<{w}:p><{w}:r>"), format!("</{w}:r></{w}:p>"));
    }
    if lower == "content.xml" || lower.ends_with("/content.xml") {
        let t = prefix_of(text, ODF_TEXT).unwrap_or("text");
        return (format!("<{t}:p>"), format!("</{t}:p>"));
    }
    if [".xhtml", ".html", ".htm"]
        .iter()
        .any(|e| lower.ends_with(e))
    {
        return ("<p>".to_owned(), "</p>".to_owned());
    }
    (format!("<{FLAT_ELEMENT}>"), format!("</{FLAT_ELEMENT}>"))
}

/// The package with the flattened parts written in place of the deep
/// ones, every other member copied as it was packed.
fn rewrite(bytes: &[u8], flat: &[(usize, String, String)]) -> zip::result::ZipResult<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for n in 0..zip.len() {
        if let Some((_, name, text)) = flat.iter().find(|(at, ..)| *at == n) {
            out.start_file(name.as_str(), stored)?;
            out.write_all(text.as_bytes())?;
        } else {
            out.raw_copy_file(zip.by_index_raw(n)?)?;
        }
    }
    Ok(out.finish()?.into_inner())
}

/// Reads a whole container from a seekable source: at most
/// `caps.max_container` bytes, refused as too large past that.
pub fn read_all(source: &mut dyn ReadSeek, caps: &ContainerCaps) -> Result<Vec<u8>, DocError> {
    let len = source
        .seek(SeekFrom::End(0))
        .map_err(|_| DocError::Failed)?;
    if len > caps.max_container {
        return Err(DocError::TooLarge);
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| DocError::Failed)?;
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    source
        .take(caps.max_container + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DocError::Failed)?;
    if bytes.len() as u64 > caps.max_container {
        return Err(DocError::TooLarge);
    }
    Ok(bytes)
}

/// Reads a container document from a seekable source: the container
/// read whole within its cap, then on the guard's thread checked by
/// [`check`] (`what` names the format) and read by `read`, with the
/// nesting note when a part was flattened. The format readers (carta's
/// DOCX, EPUB and ODT, the sheets) are the `read` functions.
pub fn read_seekable_with<F>(
    source: &mut dyn ReadSeek,
    options: &ReadOptions,
    caps: &ContainerCaps,
    what: &str,
    read: F,
) -> Result<DocView, DocError>
where
    F: FnOnce(Vec<u8>, &ContainerCaps) -> Result<DocView, DocError> + Send + 'static,
{
    let bytes = read_all(source, caps)?;
    let caps = *caps;
    let what = what.to_owned();
    options.guard.run(move || {
        let checked = check(bytes, &caps, &what)?;
        let mut view = read(checked.bytes, &caps)?;
        if checked.flattened {
            view.add_note(NESTING_NOTE);
        }
        Ok(view)
    })?
}

#[cfg(test)]
pub(crate) mod tests;
