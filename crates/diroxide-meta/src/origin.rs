//! Extended items in words, and the origin row (R12, "Extended
//! attributes and streams"; R28 3.2; R30 commit 48).
//!
//! The internet mark is where a file came from, as the system or the
//! downloading program wrote it beside the file:
//!
//! - **Windows:** the stream `Zone.Identifier`, a small INI text with
//!   `ZoneId=3` for the internet and sometimes `HostUrl` and
//!   `ReferrerUrl`.
//! - **macOS:** the attribute `com.apple.quarantine`, a semicolon list of
//!   flags, a time, the downloading program and an id.
//! - **Linux:** `user.xdg.origin.url` (and `user.xdg.referrer.url`),
//!   where a program such as curl wrote it.
//!
//! The row says "Origin: from the internet", or the host only: "Origin:
//! downloaded from example.org". It is private: the full address is a
//! request away ([`InternetMark::address`]) and is never sent anywhere.
//!
//! **Remove the internet mark** is an operation the app runs as a job
//! that asks first, its answer starting on No ([`removal_question`] and
//! [`removal_warning`]): [`remove_internet_mark`] removes the mark's
//! items and gives back their bytes, so [`restore_internet_mark`] undoes
//! it. The stream's path with its colon is built only in the platform
//! module of `diroxide-vfs`.

use diroxide_core::Place;
use diroxide_core::details::DetailGroup;
use diroxide_core::names::display_name;
use diroxide_vfs::{ExtendedAttributes, ExtendedItem, ExtendedKind, OpContext, Provider, VfsError};

use crate::field::Field;
use crate::reader::MetaError;

/// The Windows stream of the internet mark.
pub const ZONE_STREAM: &str = "Zone.Identifier";
/// The macOS attribute of the internet mark.
pub const QUARANTINE: &str = "com.apple.quarantine";
/// Where a file came from, as a Linux program wrote it.
pub const XDG_ORIGIN: &str = "user.xdg.origin.url";
/// The page a file was found on, as a Linux program wrote it.
pub const XDG_REFERRER: &str = "user.xdg.referrer.url";
/// Tags in the file system, a comma list (Baloo, Dolphin).
pub const XDG_TAGS: &str = "user.xdg.tags";
/// A comment in the file system.
pub const XDG_COMMENT: &str = "user.xdg.comment";

/// The most bytes of one extended item read for its words.
pub const ITEM_CAP: usize = 64 * 1024;

/// The names that carry an internet mark, in the order they are read.
pub const MARK_ITEMS: [&str; 4] = [ZONE_STREAM, QUARANTINE, XDG_ORIGIN, XDG_REFERRER];

/// Where a file came from, as its mark says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InternetMark {
    /// The zone of a Windows mark: 0 this computer, 1 the local network,
    /// 2 a trusted site, 3 the internet, 4 a restricted site.
    pub zone: Option<u8>,
    /// The address the file came from.
    pub host_url: Option<String>,
    /// The page it was found on.
    pub referrer_url: Option<String>,
    /// The program that downloaded it (macOS).
    pub agent: Option<String>,
}

impl InternetMark {
    /// A Windows `Zone.Identifier` stream's text: `[ZoneTransfer]` with
    /// `ZoneId`, `HostUrl` and `ReferrerUrl`, keys in any case, a
    /// byte-order mark and CR LF allowed. `None` when it holds none of
    /// them.
    pub fn from_zone_identifier(bytes: &[u8]) -> Option<Self> {
        let text = text_of(bytes);
        let mut mark = InternetMark::default();
        let mut found = false;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "zoneid" => {
                    mark.zone = value.parse().ok();
                    found |= mark.zone.is_some();
                }
                "hosturl" if !value.is_empty() => {
                    mark.host_url = Some(value.to_owned());
                    found = true;
                }
                "referrerurl" if !value.is_empty() => {
                    mark.referrer_url = Some(value.to_owned());
                    found = true;
                }
                _ => {}
            }
        }
        found.then_some(mark)
    }

    /// A macOS quarantine value: `FLAGS;TIME;AGENT;ID`, the agent being
    /// the downloading program.
    pub fn from_quarantine(bytes: &[u8]) -> Option<Self> {
        let text = text_of(bytes);
        let mut parts = text.split(';');
        let flags = parts.next()?.trim();
        if flags.is_empty() || !flags.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let _time = parts.next();
        let agent = parts
            .next()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned);
        Some(InternetMark {
            agent,
            ..InternetMark::default()
        })
    }

    /// A Linux `user.xdg.origin.url` value.
    pub fn from_origin_url(bytes: &[u8]) -> Option<Self> {
        let url = text_of(bytes).trim().to_owned();
        (!url.is_empty()).then(|| InternetMark {
            host_url: Some(url),
            ..InternetMark::default()
        })
    }

    /// The host the file came from: the address's host, else the page's.
    pub fn host(&self) -> Option<String> {
        self.host_url
            .as_deref()
            .and_then(host_of)
            .or_else(|| self.referrer_url.as_deref().and_then(host_of))
    }

    /// Where it came from in words: "from the internet", "downloaded from
    /// example.org", "from the local network".
    pub fn words(&self) -> String {
        match self.zone {
            Some(0) => "from this computer".to_owned(),
            Some(1) => "from the local network".to_owned(),
            Some(2) => "from a trusted site".to_owned(),
            Some(4) => match self.host() {
                Some(host) => format!("from a restricted site, {host}"),
                None => "from a restricted site".to_owned(),
            },
            _ => match self.host() {
                Some(host) => format!("downloaded from {host}"),
                None => "from the internet".to_owned(),
            },
        }
    }

    /// Whether the mark makes the system warn before the file opens:
    /// the internet and restricted zones, and every macOS or Linux mark.
    pub fn warns(&self) -> bool {
        self.zone.is_none_or(|zone| zone >= 3)
    }

    /// The origin row, private: "Origin: downloaded from example.org".
    pub fn field(&self) -> Field {
        Field::new("file.origin", "Origin", self.words(), DetailGroup::Source).private()
    }

    /// The full address, cleaned, for the one command that shows it.
    pub fn address(&self) -> Option<String> {
        self.host_url
            .as_deref()
            .or(self.referrer_url.as_deref())
            .map(crate::clean::clean_text)
    }
}

/// Bytes as text: UTF-8 with a byte-order mark taken off, UTF-16 by its
/// mark, anything else as UTF-8 with stand-ins.
fn text_of(bytes: &[u8]) -> String {
    let utf16 = |little: bool, rest: &[u8]| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| {
                if little {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(true, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(false, rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_owned(),
    }
}

/// The host of a web or file-transfer address, lowercase, without the
/// user, the port or the path: `https://ada@Downloads.Example.org:8443/a`
/// gives `downloads.example.org`. `None` for anything else
/// (`about:internet`, a bare word).
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | "ftps" | "sftp"
    ) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let plain = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    plain.then_some(host)
}

/// One item's bytes, at most [`ITEM_CAP`] of them, or `None` when it is
/// missing or larger.
fn read_item(
    extended: &dyn ExtendedAttributes,
    place: &Place,
    name: &str,
    ctx: &OpContext,
) -> Option<Vec<u8>> {
    extended.read_extended(place, name, ITEM_CAP, ctx).ok()
}

/// The internet mark among a file's items, read from the first of
/// [`MARK_ITEMS`] it carries.
pub fn mark_among(
    extended: &dyn ExtendedAttributes,
    place: &Place,
    items: &[ExtendedItem],
    ctx: &OpContext,
) -> Option<InternetMark> {
    let has = |name: &str| items.iter().any(|item| item.name == name);
    if has(ZONE_STREAM)
        && let Some(mark) = read_item(extended, place, ZONE_STREAM, ctx)
            .as_deref()
            .and_then(InternetMark::from_zone_identifier)
    {
        return Some(mark);
    }
    if has(QUARANTINE)
        && let Some(mark) = read_item(extended, place, QUARANTINE, ctx)
            .as_deref()
            .and_then(InternetMark::from_quarantine)
    {
        return Some(mark);
    }
    let origin = has(XDG_ORIGIN)
        .then(|| read_item(extended, place, XDG_ORIGIN, ctx))
        .flatten()
        .and_then(|b| InternetMark::from_origin_url(&b));
    let referrer = has(XDG_REFERRER)
        .then(|| read_item(extended, place, XDG_REFERRER, ctx))
        .flatten()
        .map(|b| text_of(&b).trim().to_owned())
        .filter(|r| !r.is_empty());
    match (origin, referrer) {
        (Some(mut mark), referrer) => {
            mark.referrer_url = referrer;
            Some(mark)
        }
        (None, Some(referrer)) => Some(InternetMark {
            referrer_url: Some(referrer),
            ..InternetMark::default()
        }),
        (None, None) => None,
    }
}

/// The internet mark of the file at `place`, when its provider reads
/// extended items and the file carries one.
pub fn read_mark(
    provider: &dyn Provider,
    place: &Place,
    ctx: &OpContext,
) -> Result<Option<InternetMark>, MetaError> {
    let Some(extended) = provider.as_extended() else {
        return Ok(None);
    };
    let items = list(extended, place, ctx)?;
    Ok(mark_among(extended, place, &items, ctx))
}

fn list(
    extended: &dyn ExtendedAttributes,
    place: &Place,
    ctx: &OpContext,
) -> Result<Vec<ExtendedItem>, MetaError> {
    match extended.list_extended(place, ctx) {
        Ok(items) => Ok(items),
        // A drive that holds none has none.
        Err(VfsError::Unsupported { .. }) => Ok(Vec::new()),
        Err(error) => Err(MetaError::Place(error)),
    }
}

/// The extended items of a file as fields, background cost (one list
/// and a few small reads): "Streams: 2, Zone.Identifier, extra" or
/// "Extended attributes: 3, ...", the origin row, and on Linux the tags
/// and comment other programs keep there, read only. Nothing when the
/// place's provider reads no extended items (a remote place: "not
/// available here" is the cell's word).
pub fn extended_fields(
    provider: &dyn Provider,
    place: &Place,
    ctx: &OpContext,
) -> Result<Vec<Field>, MetaError> {
    let Some(extended) = provider.as_extended() else {
        return Ok(Vec::new());
    };
    let items = list(extended, place, ctx)?;
    let mut fields = Vec::new();
    if !items.is_empty() {
        let streams = items.iter().all(|item| item.kind == ExtendedKind::Stream);
        let label = if streams {
            "Streams"
        } else {
            "Extended attributes"
        };
        let names: Vec<String> = items
            .iter()
            .map(|item| {
                let name = display_name(&item.name).into_owned();
                if item.system_managed {
                    format!("{name} (managed by the system, read only)")
                } else {
                    name
                }
            })
            .collect();
        fields.push(Field::new(
            "file.extended",
            label,
            format!("{}, {}", items.len(), names.join(", ")),
            DetailGroup::System,
        ));
    }
    if let Some(mark) = mark_among(extended, place, &items, ctx) {
        fields.push(mark.field());
    }
    let has = |name: &str| items.iter().any(|item| item.name == name);
    if has(XDG_TAGS)
        && let Some(tags) = read_item(extended, place, XDG_TAGS, ctx)
    {
        let tags = text_of(&tags)
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        if !tags.is_empty() {
            fields.push(Field::new(
                "file.tags",
                "Tags in the file system",
                tags,
                DetailGroup::Yours,
            ));
        }
    }
    if has(XDG_COMMENT)
        && let Some(comment) = read_item(extended, place, XDG_COMMENT, ctx)
    {
        let comment = text_of(&comment);
        if !comment.trim().is_empty() {
            fields.push(
                Field::new(
                    "file.comment",
                    "Comment in the file system",
                    comment,
                    DetailGroup::Yours,
                )
                .private(),
            );
        }
    }
    Ok(fields.into_iter().map(Field::cleaned).collect())
}

/// The question before Remove the internet mark, its answer starting on
/// No: "Remove the internet mark from setup.exe?"
pub fn removal_question(name: &str) -> String {
    format!("Remove the internet mark from {}?", display_name(name))
}

/// The sentence after the question: what removing the mark changes on
/// this system.
pub fn removal_warning() -> &'static str {
    if cfg!(windows) {
        "Windows will stop warning when you open it."
    } else if cfg!(target_vendor = "apple") {
        "macOS will stop warning when you open it."
    } else {
        "It will no longer say where it came from."
    }
}

/// What a removal took away: each item's name and its bytes, so the
/// job's undo writes them back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarkRemoval {
    /// The items removed, with their bytes before.
    pub removed: Vec<(String, Vec<u8>)>,
}

/// The result line: "Removed the internet mark from setup.exe".
pub fn removed_words(name: &str) -> String {
    format!("Removed the internet mark from {}", display_name(name))
}

/// The line when there is nothing to remove: "No internet mark on
/// setup.exe".
pub fn no_mark_words(name: &str) -> String {
    format!("No internet mark on {}", display_name(name))
}

/// Removes the internet mark of the file at `place`: every item of
/// [`MARK_ITEMS`] it carries, each read first so the removal can be
/// undone. `Ok(None)` when it carries none; refused with
/// [`VfsError::Unsupported`] when the place's provider changes no
/// extended items.
pub fn remove_internet_mark(
    provider: &dyn Provider,
    place: &Place,
    ctx: &OpContext,
) -> Result<Option<MarkRemoval>, MetaError> {
    let extended = provider.as_extended().ok_or_else(|| {
        MetaError::Place(VfsError::unsupported(
            provider.scheme(),
            "remove an internet mark",
        ))
    })?;
    let items = list(extended, place, ctx)?;
    let mut removed = Vec::new();
    for name in MARK_ITEMS {
        if !items.iter().any(|item| item.name == name) {
            continue;
        }
        ctx.check()?;
        let before = extended.read_extended(place, name, ITEM_CAP, ctx)?;
        extended.remove_extended(place, name, ctx)?;
        removed.push((name.to_owned(), before));
    }
    Ok((!removed.is_empty()).then_some(MarkRemoval { removed }))
}

/// Undoes a removal: writes each removed item back.
pub fn restore_internet_mark(
    provider: &dyn Provider,
    place: &Place,
    removal: &MarkRemoval,
    ctx: &OpContext,
) -> Result<(), MetaError> {
    let extended = provider.as_extended().ok_or_else(|| {
        MetaError::Place(VfsError::unsupported(
            provider.scheme(),
            "restore an internet mark",
        ))
    })?;
    for (name, bytes) in &removal.removed {
        extended.write_extended(place, name, bytes, ctx)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use diroxide_vfs::LocalProvider;

    use super::*;

    #[test]
    fn a_zone_identifier_says_where_the_file_came_from() {
        let mark = InternetMark::from_zone_identifier(
            b"\xEF\xBB\xBF[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.org/page\r\nHostUrl=https://Example.org:8443/setup.exe\r\n",
        )
        .unwrap();
        assert_eq!(mark.zone, Some(3));
        assert_eq!(mark.field().line(), "Origin: downloaded from example.org");
        assert!(mark.field().private);
        assert!(mark.warns());
        assert_eq!(
            mark.address().as_deref(),
            Some("https://Example.org:8443/setup.exe")
        );
        let bare = InternetMark::from_zone_identifier(
            b"[ZoneTransfer]\nzoneid=3\nHostUrl=about:internet\n",
        )
        .unwrap();
        assert_eq!(bare.field().line(), "Origin: from the internet");
        let intranet =
            InternetMark::from_zone_identifier(b"[ZoneTransfer]\r\nZoneId=1\r\n").unwrap();
        assert_eq!(intranet.words(), "from the local network");
        assert!(!intranet.warns());
        assert_eq!(InternetMark::from_zone_identifier(b"nothing here"), None);
        // UTF-16, as some programs write it.
        let mut wide = vec![0xFF, 0xFE];
        for unit in "[ZoneTransfer]\r\nZoneId=3\r\n".encode_utf16() {
            wide.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            InternetMark::from_zone_identifier(&wide).unwrap().zone,
            Some(3)
        );
    }

    #[test]
    fn quarantine_and_origin_url_marks() {
        let mac = InternetMark::from_quarantine(b"0083;6542a1b2;Safari;0B7B4E1D-0000").unwrap();
        assert_eq!(mac.agent.as_deref(), Some("Safari"));
        assert_eq!(mac.words(), "from the internet");
        assert_eq!(InternetMark::from_quarantine(b"not;hex"), None);
        let linux =
            InternetMark::from_origin_url(b"https://downloads.example.org/a.tar.gz").unwrap();
        assert_eq!(linux.words(), "downloaded from downloads.example.org");
        assert_eq!(InternetMark::from_origin_url(b"  "), None);
    }

    #[test]
    fn hosts_are_only_hosts() {
        assert_eq!(
            host_of("https://ada@Downloads.Example.org:8443/a?b#c").as_deref(),
            Some("downloads.example.org")
        );
        assert_eq!(
            host_of("http://[2001:db8::1]:80/x").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(host_of("about:internet"), None);
        assert_eq!(host_of("javascript://x%0Aalert"), None);
        assert_eq!(host_of("https:///path"), None);
        assert_eq!(host_of("https://exa\u{202E}mple.org/"), None);
    }

    #[test]
    fn the_question_and_the_result_lines() {
        let lines = [
            removal_question("setup.exe"),
            removal_warning().to_owned(),
            removed_words("setup.exe"),
            no_mark_words("setup.exe"),
        ];
        for line in &lines {
            assert!(line.chars().count() <= 48, "{line}");
        }
        assert_eq!(lines[0], "Remove the internet mark from setup.exe?");
        assert_eq!(lines[2], "Removed the internet mark from setup.exe");
        assert_eq!(lines[3], "No internet mark on setup.exe");
        assert!(lines[0].chars().count() <= 40);
        assert!(lines[2].chars().count() <= 40);
        assert_eq!(
            removal_question("a\u{202E}b.exe"),
            "Remove the internet mark from a?b.exe?"
        );
    }

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap()
    }

    /// The whole road on a file of the test's own, marked by the test:
    /// the origin row, the removal with its bytes kept, the undo.
    #[test]
    fn a_file_of_our_own_is_marked_read_unmarked_and_restored() {
        let dir = scratch();
        let path = dir.path().join("setup.exe");
        std::fs::write(&path, b"not really a program").unwrap();
        let place = Place::parse(&path.display().to_string()).unwrap();
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        if !provider.capabilities(&place).extended_attributes {
            return;
        }
        let extended = provider.as_extended().unwrap();
        let (name, bytes): (&str, &[u8]) = if cfg!(windows) {
            (
                ZONE_STREAM,
                b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.org/setup.exe\r\n",
            )
        } else if cfg!(target_vendor = "apple") {
            (QUARANTINE, b"0083;6542a1b2;Safari;0B7B4E1D")
        } else {
            (XDG_ORIGIN, b"https://example.org/setup.exe")
        };
        if extended.write_extended(&place, name, bytes, &ctx).is_err() {
            return;
        }
        let fields = extended_fields(&provider, &place, &ctx).unwrap();
        let lines: Vec<String> = fields.iter().map(Field::line).collect();
        assert!(lines.iter().any(|l| l.starts_with("Origin: ")), "{lines:?}");
        assert!(read_mark(&provider, &place, &ctx).unwrap().is_some());
        let removal = remove_internet_mark(&provider, &place, &ctx)
            .unwrap()
            .unwrap();
        assert_eq!(removal.removed, [(name.to_owned(), bytes.to_vec())]);
        assert_eq!(read_mark(&provider, &place, &ctx).unwrap(), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"not really a program");
        assert_eq!(remove_internet_mark(&provider, &place, &ctx).unwrap(), None);
        restore_internet_mark(&provider, &place, &removal, &ctx).unwrap();
        assert!(read_mark(&provider, &place, &ctx).unwrap().is_some());
    }
}
