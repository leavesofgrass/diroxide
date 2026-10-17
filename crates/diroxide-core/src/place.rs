//! Places: where things are. A place is a local path, a member path inside
//! an archive (`D:\docs\course.zip!week1`), or a remote path with a scheme
//! (`sftp://host/path`).
//!
//! Places are always full: a relative path, or a drive letter with no slash
//! such as `C:`, is refused, because Windows reads `C:` as "the current
//! folder on drive C". [`Place::resolve`] turns typed relative input into a
//! full place against a base.
//!
//! Parsing is textual and follows one path style, the platform's own by
//! default. `D:\photos` is a full path on Windows and a relative file name
//! on Linux; `/home` is a full path on Linux and drive-relative on Windows.
//! Lexical `..` is resolved at parse time, as a file manager's "up" does.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::PlaceError;

/// The archive extensions, lowercase, the longest first: a local file
/// with one of them is listed as an archive, and `!` after one starts a
/// member path. `.gz` is a one-member archive unless it holds a tar
/// (`notes.txt.gz!notes.txt`, W2a's request 2). A slice, so the formats of
/// later alphas (`.tar.bz2`, `.tar.zst`, `.bz2`, `.zst`, `.rar`, `.iso`)
/// join without breaking callers.
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tar.xz", ".zip", ".tgz", ".txz", ".tar", ".7z", ".xz", ".gz",
];

/// Files that are zip archives inside, lowercase: `!` after one of them
/// also starts a member path, so "Open as archive" on `report.docx` makes
/// a place that reads back. They are not listed as archives; Enter on one
/// opens it in its program, as its file type says.
pub const ZIP_DOCUMENT_EXTENSIONS: &[&str] = &[
    ".docx", ".xlsx", ".pptx", ".odt", ".ods", ".odp", ".epub", ".jar", ".apk",
];

/// Whether `name` ends with an archive extension, or with a zip-based
/// document's, followed by an optional part number (`backup.7z.001`): the
/// names after which `!` starts a member path, so a split archive opens as
/// one place. Case is ignored. `photo.jpg.001` is not one, and neither is
/// a name with no stem before the extension (`.zip`, `..zip`), which is a
/// hidden file's whole name (the fuzzing finding of W2b: a local name such
/// as `.zip!.xz` read as a member of an archive called `.zip`).
pub fn opens_as_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    let base = match crate::names::split_part(&lower) {
        Some(part) => part.base,
        None => lower.as_str(),
    };
    ARCHIVE_EXTENSIONS
        .iter()
        .chain(ZIP_DOCUMENT_EXTENSIONS)
        .any(|ext| {
            base.strip_suffix(ext)
                .is_some_and(|stem| !stem.is_empty() && !stem.bytes().all(|b| b == b'.'))
        })
}

/// Whether a name holds an archive marker: a `!` after a name that opens
/// as an archive (`inner.zip!x.txt`), which a place would read back as a
/// member of that archive. [`Place::join`] refuses such a name in every
/// kind of place, so every place reads back from its text as it was.
pub fn holds_archive_marker(name: &str) -> bool {
    !archive_splits(name).is_empty()
}

/// A path for showing and speaking, with the extended-length prefix that
/// canonicalizing adds on Windows removed: `\\?\D:\x` reads `D:\x`, and
/// `\\?\UNC\server\share` reads `\\server\share`. Other text is borrowed
/// unchanged. The prefix never reaches a user (spike four).
pub fn strip_extended_prefix(path: &str) -> Cow<'_, str> {
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return Cow::Borrowed(path);
    };
    match rest.get(..4) {
        Some(unc) if unc.eq_ignore_ascii_case(r"UNC\") => Cow::Owned(format!(r"\\{}", &rest[4..])),
        _ => Cow::Borrowed(rest),
    }
}

/// How local paths are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathStyle {
    /// Drive letters (`D:\photos`) and network shares (`\\server\share`).
    Windows,
    /// One root, `/`.
    Unix,
}

impl PathStyle {
    /// The style of the platform this was built for.
    pub const fn native() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Unix
        }
    }
}

/// What kind of place this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaceKind {
    /// A folder on this machine.
    Local,
    /// Inside an archive.
    Archive,
    /// On another machine, or served by a plugin.
    Remote,
}

impl PlaceKind {
    /// The kind in words: "folder", "archive", "remote folder".
    pub const fn word(self) -> &'static str {
        match self {
            PlaceKind::Local => "folder",
            PlaceKind::Archive => "archive",
            PlaceKind::Remote => "remote folder",
        }
    }
}

/// A remote location: a scheme, an optional user, a host, an optional port
/// and an absolute path with forward slashes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Remote {
    scheme: String,
    user: Option<String>,
    host: String,
    port: Option<u16>,
    path: String,
}

impl Remote {
    /// A remote location. The scheme is lowercase letters, digits, `+`, `-`
    /// or `.`, at least two characters, starting with a letter; the host is
    /// not empty; the path is normalized to start with `/`.
    pub fn new(scheme: &str, host: &str, path: &str) -> Result<Self, PlaceError> {
        let address = format!("{scheme}://{host}{path}");
        if !valid_scheme(scheme) || host.is_empty() || host.contains(['/', '@']) {
            return Err(PlaceError::BadRemote(address));
        }
        Ok(Self {
            scheme: scheme.to_owned(),
            user: None,
            host: host.to_owned(),
            port: None,
            path: normalize_slash_path(path),
        })
    }

    /// The same location with a user name.
    pub fn with_user(mut self, user: &str) -> Self {
        self.user = if user.is_empty() {
            None
        } else {
            Some(user.to_owned())
        };
        self
    }

    /// The same location with a port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// The scheme, such as `sftp`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The user name, if one was given.
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The host name or address.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, if one was given.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The absolute path on the host, with forward slashes.
    pub fn path(&self) -> &str {
        &self.path
    }

    fn with_path(&self, path: String) -> Self {
        Self {
            path,
            ..self.clone()
        }
    }
}

impl fmt::Display for Remote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://", self.scheme)?;
        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Repr {
    Local(String),
    Archive(Box<Place>, String),
    Remote(Remote),
}

/// A location any provider can serve. See the module documentation.
///
/// A place written as text round-trips: `Place::parse(&p.to_string())`
/// gives `p` back. In TOML and JSON a place is that text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Place(Repr);

impl Place {
    /// Reads a full place written in the platform's own path style.
    pub fn parse(text: &str) -> Result<Self, PlaceError> {
        Self::parse_with(text, PathStyle::native())
    }

    /// Reads a full place written in the given path style.
    pub fn parse_with(text: &str, style: PathStyle) -> Result<Self, PlaceError> {
        if text.is_empty() {
            return Err(PlaceError::Empty);
        }
        if let Some(split) = archive_split(text) {
            let container = Self::parse_with(&text[..split], style)?;
            let member = normalize_member(&text[split + 1..])?;
            return Ok(Self(Repr::Archive(Box::new(container), member)));
        }
        if let Some((scheme, rest)) = text.split_once("://")
            && valid_scheme(scheme)
        {
            return if scheme == "file" {
                parse_file_url(text, rest, style)
            } else {
                parse_remote(text, scheme, rest).map(|remote| Self(Repr::Remote(remote)))
            };
        }
        normalize_local(text, style).map(|path| Self(Repr::Local(path)))
    }

    /// Reads typed input as a place: a full place as it is, or a relative
    /// path (`docs`, `../photos`, `course.zip!week1`) against `base`, in the
    /// platform's own path style.
    pub fn resolve(input: &str, base: &Place) -> Result<Self, PlaceError> {
        Self::resolve_with(input, base, PathStyle::native())
    }

    /// [`Place::resolve`] in the given path style.
    pub fn resolve_with(input: &str, base: &Place, style: PathStyle) -> Result<Self, PlaceError> {
        let rooted = style == PathStyle::Windows
            && (input.starts_with('\\') || input.starts_with('/'))
            && !input.starts_with("\\\\")
            && !input.starts_with("//");
        if rooted && let Some(drive) = base.windows_drive() {
            return Self::parse_with(&format!("{drive}{input}"), style);
        }
        match Self::parse_with(input, style) {
            Err(PlaceError::NotAbsolute(_)) => {}
            other => return other,
        }
        if let Some(split) = archive_split(input) {
            let container = Self::resolve_with(&input[..split], base, style)?;
            let member = normalize_member(&input[split + 1..])?;
            return Ok(Self(Repr::Archive(Box::new(container), member)));
        }
        let backslash_separates = !matches!(&base.0, Repr::Local(path) if path.starts_with('/'));
        let mut place = base.clone();
        for part in input.split(|c| c == '/' || (backslash_separates && c == '\\')) {
            match part {
                "" | "." => {}
                ".." => {
                    if let Some(parent) = place.parent() {
                        place = parent;
                    }
                }
                name => place = place.join(name)?,
            }
        }
        Ok(place)
    }

    /// What kind of place this is.
    pub fn kind(&self) -> PlaceKind {
        match &self.0 {
            Repr::Local(_) => PlaceKind::Local,
            Repr::Archive(..) => PlaceKind::Archive,
            Repr::Remote(_) => PlaceKind::Remote,
        }
    }

    /// The local path, for a local place.
    pub fn local_path(&self) -> Option<&str> {
        match &self.0 {
            Repr::Local(path) => Some(path),
            _ => None,
        }
    }

    /// The archive's own place and the member path (empty at the archive's
    /// root), for a place inside an archive.
    pub fn archive(&self) -> Option<(&Place, &str)> {
        match &self.0 {
            Repr::Archive(container, member) => Some((container, member)),
            _ => None,
        }
    }

    /// The remote location, for a remote place.
    pub fn remote(&self) -> Option<&Remote> {
        match &self.0 {
            Repr::Remote(remote) => Some(remote),
            _ => None,
        }
    }

    /// The root of the archive at this place: `D:\docs\course.zip` becomes
    /// `D:\docs\course.zip!`.
    pub fn into_archive(self) -> Self {
        Self(Repr::Archive(Box::new(self), String::new()))
    }

    /// Whether this is a top: a drive, a share, `/`, or a remote root. The
    /// root of an archive is not a top: its parent is the archive's folder.
    pub fn is_root(&self) -> bool {
        match &self.0 {
            Repr::Local(path) => split_local(path).1.is_empty(),
            Repr::Archive(..) => false,
            Repr::Remote(remote) => remote.path == "/",
        }
    }

    /// The last name in the place: a folder's name, an archive member's
    /// name, the archive's file name at an archive's root, or `None` at a
    /// top.
    pub fn name(&self) -> Option<&str> {
        match &self.0 {
            Repr::Local(path) => {
                let (_, rest, sep) = split_local(path);
                (!rest.is_empty()).then(|| rest.rsplit(sep).next().unwrap_or(rest))
            }
            Repr::Archive(container, member) => {
                if member.is_empty() {
                    container.name()
                } else {
                    member.rsplit('/').next()
                }
            }
            Repr::Remote(remote) => remote.path.rsplit('/').next().filter(|n| !n.is_empty()),
        }
    }

    /// The place one level up, or `None` at a top. Up from an archive's
    /// root is the folder that holds the archive.
    pub fn parent(&self) -> Option<Self> {
        match &self.0 {
            Repr::Local(path) => {
                let (prefix, rest, sep) = split_local(path);
                if rest.is_empty() {
                    return None;
                }
                let up = rest.rfind(sep).map_or("", |at| &rest[..at]);
                Some(Self(Repr::Local(format!("{prefix}{up}"))))
            }
            Repr::Archive(container, member) => {
                if member.is_empty() {
                    container.parent()
                } else {
                    let up = member.rfind('/').map_or("", |at| &member[..at]);
                    Some(Self(Repr::Archive(container.clone(), up.to_owned())))
                }
            }
            Repr::Remote(remote) => {
                if remote.path == "/" {
                    return None;
                }
                let up = remote.path.rfind('/').map_or("", |at| &remote.path[..at]);
                let up = if up.is_empty() { "/" } else { up };
                Some(Self(Repr::Remote(remote.with_path(up.to_owned()))))
            }
        }
    }

    /// The place of a child called `name`. The name must be one name: not
    /// empty, not `.` or `..`, with no `/` (and no `\` in a Windows path or
    /// an archive), and no NUL. The name may not hold an archive marker
    /// (`inner.zip!x.txt`, see [`holds_archive_marker`]), which would read
    /// back as a member of an archive: inside an archive the archive
    /// provider flags such a member ([`PlaceError::ArchiveMarker`]), and in
    /// a local or remote folder such a file is left out and said by name
    /// ([`PlaceError::ReadsAsArchive`]), so a place written as text always
    /// reads back as itself (W2b's finding).
    pub fn join(&self, name: &str) -> Result<Self, PlaceError> {
        let backslash_ok = matches!(&self.0, Repr::Local(path) if path.starts_with('/'))
            || matches!(&self.0, Repr::Remote(_));
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\0'])
            || (!backslash_ok && name.contains('\\'))
        {
            return Err(PlaceError::BadName(name.to_owned()));
        }
        if holds_archive_marker(name) {
            return Err(if matches!(self.0, Repr::Archive(..)) {
                PlaceError::ArchiveMarker(name.to_owned())
            } else {
                PlaceError::ReadsAsArchive(name.to_owned())
            });
        }
        Ok(match &self.0 {
            Repr::Local(path) => {
                let (_, rest, sep) = split_local(path);
                if rest.is_empty() {
                    Self(Repr::Local(format!("{path}{name}")))
                } else {
                    Self(Repr::Local(format!("{path}{sep}{name}")))
                }
            }
            Repr::Archive(container, member) => {
                let member = if member.is_empty() {
                    name.to_owned()
                } else {
                    format!("{member}/{name}")
                };
                Self(Repr::Archive(container.clone(), member))
            }
            Repr::Remote(remote) => {
                let path = if remote.path == "/" {
                    format!("/{name}")
                } else {
                    format!("{}/{name}", remote.path)
                };
                Self(Repr::Remote(remote.with_path(path)))
            }
        })
    }

    /// Whether this place is `root` or lies inside it, following parents
    /// (into archives too). Windows paths compare without regard to case.
    /// A recursive operation uses this to never leave its root.
    pub fn is_within(&self, root: &Place) -> bool {
        let mut current = Some(self.clone());
        while let Some(place) = current {
            if place.same_as(root) {
                return true;
            }
            current = place.parent();
        }
        false
    }

    fn same_as(&self, other: &Place) -> bool {
        match (&self.0, &other.0) {
            (Repr::Local(a), Repr::Local(b)) if !a.starts_with('/') => {
                a.to_lowercase() == b.to_lowercase()
            }
            (Repr::Archive(a, m), Repr::Archive(b, n)) => m == n && a.same_as(b),
            _ => self == other,
        }
    }

    fn windows_drive(&self) -> Option<&str> {
        match &self.0 {
            Repr::Local(path) if path.as_bytes().get(1) == Some(&b':') => Some(&path[..2]),
            Repr::Archive(container, _) => container.windows_drive(),
            _ => None,
        }
    }

    /// The title line's words, the meaning first: "week1, in course.zip",
    /// "photos, in D:\", "Drive D:\", "course.zip, archive, in D:\docs",
    /// "notes, on example.org".
    pub fn title(&self) -> String {
        match &self.0 {
            Repr::Local(path) => match (self.name(), self.parent()) {
                (Some(name), Some(parent)) => format!("{name}, in {parent}"),
                _ if path.starts_with('/') => "Root folder /".to_owned(),
                _ if path.starts_with("\\\\") => format!("Share {path}"),
                _ => format!("Drive {path}"),
            },
            Repr::Archive(container, member) => {
                let archive = container.name().unwrap_or("archive");
                if member.is_empty() {
                    match container.parent() {
                        Some(folder) => format!("{archive}, archive, in {folder}"),
                        None => format!("{archive}, archive"),
                    }
                } else {
                    let name = self.name().unwrap_or(member);
                    match member.rfind('/') {
                        Some(at) => format!("{name}, in {archive}!{}", &member[..at]),
                        None => format!("{name}, in {archive}"),
                    }
                }
            }
            Repr::Remote(remote) => match self.name() {
                Some(name) => format!("{name}, on {}", remote.host),
                None => format!("{}, {}", remote.host, remote.scheme.to_uppercase()),
            },
        }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Local(path) => f.write_str(path),
            Repr::Archive(container, member) => write!(f, "{container}!{member}"),
            Repr::Remote(remote) => remote.fmt(f),
        }
    }
}

impl TryFrom<String> for Place {
    type Error = PlaceError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Place> for String {
    fn from(place: Place) -> Self {
        place.to_string()
    }
}

fn valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.".contains(c))
}

/// The byte indexes of the `!` characters that start a member path, left
/// to right: each follows a name that opens as an archive (see
/// [`opens_as_archive`]), the name being what comes after the last slash,
/// backslash or earlier split. So `/srv/wow!.zip!x` splits after
/// `wow!.zip`, and `/srv/.zip!.xz` does not split at all.
fn archive_splits(text: &str) -> Vec<usize> {
    let mut splits = Vec::new();
    let mut start = 0;
    for (at, c) in text.char_indices() {
        match c {
            '/' | '\\' => start = at + 1,
            '!' if opens_as_archive(&text[start..at]) => {
                splits.push(at);
                start = at + 1;
            }
            _ => {}
        }
    }
    splits
}

/// The byte index of the last `!` that starts a member path (see
/// [`archive_splits`]): the last split, so nested archives work.
fn archive_split(text: &str) -> Option<usize> {
    archive_splits(text).last().copied()
}

/// A member path with forward slashes, no leading slash, `.` removed and
/// `..` resolved; refused if it climbs out of the archive.
fn normalize_member(text: &str) -> Result<String, PlaceError> {
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(PlaceError::MemberEscapes(text.to_owned()));
                }
            }
            name => parts.push(name),
        }
    }
    Ok(parts.join("/"))
}

/// An absolute path with forward slashes; `..` stops at the root.
fn normalize_slash_path(text: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    format!("/{}", parts.join("/"))
}

fn parse_remote(text: &str, scheme: &str, rest: &str) -> Result<Remote, PlaceError> {
    let bad = || PlaceError::BadRemote(text.to_owned());
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (user, host_port) = match authority.rfind('@') {
        Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
        None => (None, authority),
    };
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let close = bracketed.find(']').ok_or_else(bad)?;
        let after = &bracketed[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port),
            None if after.is_empty() => None,
            None => return Err(bad()),
        };
        (&host_port[..close + 2], port)
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    let mut remote = Remote::new(scheme, host, path).map_err(|_| bad())?;
    if let Some(user) = user {
        if user.is_empty() {
            return Err(bad());
        }
        remote = remote.with_user(user);
    }
    if let Some(port) = port {
        remote = remote.with_port(port.parse().map_err(|_| bad())?);
    }
    Ok(remote)
}

fn parse_file_url(text: &str, rest: &str, style: PathStyle) -> Result<Place, PlaceError> {
    let Some(path) = rest.strip_prefix('/') else {
        return Err(PlaceError::BadRemote(text.to_owned()));
    };
    let local = match style {
        PathStyle::Unix => format!("/{path}"),
        PathStyle::Windows => path.to_owned(),
    };
    normalize_local(&local, style).map(|path| Place(Repr::Local(path)))
}

/// Splits a normalized local path into its root prefix (ending with the
/// separator), the rest, and the separator.
fn split_local(path: &str) -> (&str, &str, char) {
    if let Some(rest) = path.strip_prefix('/') {
        return ("/", rest, '/');
    }
    if let Some(unc) = path.strip_prefix("\\\\") {
        let mut seps = unc.match_indices('\\').map(|(at, _)| at + 2);
        let _server_end = seps.next();
        return match seps.next() {
            Some(share_end) => (&path[..=share_end], &path[share_end + 1..], '\\'),
            None => (path, "", '\\'),
        };
    }
    let prefix_len = 3.min(path.len());
    (&path[..prefix_len], &path[prefix_len..], '\\')
}

fn normalize_local(text: &str, style: PathStyle) -> Result<String, PlaceError> {
    match style {
        PathStyle::Unix => {
            if !text.starts_with('/') {
                return Err(PlaceError::NotAbsolute(text.to_owned()));
            }
            Ok(normalize_slash_path(text))
        }
        PathStyle::Windows => normalize_windows(text),
    }
}

fn normalize_windows(text: &str) -> Result<String, PlaceError> {
    let unified = text.replace('/', "\\");
    if unified.starts_with("\\\\.\\") {
        return Err(PlaceError::DevicePath(text.to_owned()));
    }
    let verbatim = unified
        .strip_prefix("\\\\?\\")
        .map(|rest| match rest.get(..4) {
            Some(unc) if unc.eq_ignore_ascii_case("UNC\\") => format!("\\\\{}", &rest[4..]),
            _ => rest.to_owned(),
        });
    let unified = verbatim.unwrap_or(unified);
    let bytes = unified.as_bytes();
    let (prefix, rest) = if let Some(unc) = unified.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        if server.is_empty() || share.is_empty() {
            return Err(PlaceError::NotAbsolute(text.to_owned()));
        }
        (
            format!("\\\\{server}\\{share}\\"),
            parts.next().unwrap_or(""),
        )
    } else if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.get(2) != Some(&b'\\') {
            return Err(PlaceError::AmbiguousDrive(text.to_owned()));
        }
        let drive = char::from(bytes[0].to_ascii_uppercase());
        (format!("{drive}:\\"), &unified[3..])
    } else if unified.starts_with('\\') {
        return Err(PlaceError::NoDrive(text.to_owned()));
    } else {
        return Err(PlaceError::NotAbsolute(text.to_owned()));
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    Ok(format!("{prefix}{}", parts.join("\\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Windows).unwrap()
    }

    fn unix(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[test]
    fn windows_paths_are_normalized() {
        assert_eq!(
            win("d:/photos//2026/./x/..").to_string(),
            "D:\\photos\\2026"
        );
        assert_eq!(win("D:\\").to_string(), "D:\\");
        assert_eq!(win("D:\\..\\..").to_string(), "D:\\");
        assert_eq!(
            win("\\\\server\\share\\a").to_string(),
            "\\\\server\\share\\a"
        );
        assert_eq!(win("\\\\?\\C:\\long").to_string(), "C:\\long");
        assert_eq!(win("\\\\?\\UNC\\srv\\sh\\x").to_string(), "\\\\srv\\sh\\x");
    }

    #[test]
    fn ambiguous_and_relative_windows_paths_are_refused() {
        let parse = |t| Place::parse_with(t, PathStyle::Windows);
        assert_eq!(parse("C:"), Err(PlaceError::AmbiguousDrive("C:".into())));
        assert_eq!(
            parse("C:docs"),
            Err(PlaceError::AmbiguousDrive("C:docs".into()))
        );
        assert_eq!(parse("docs"), Err(PlaceError::NotAbsolute("docs".into())));
        assert_eq!(parse("\\home"), Err(PlaceError::NoDrive("\\home".into())));
        assert_eq!(
            parse("\\\\.\\PhysicalDrive0"),
            Err(PlaceError::DevicePath("\\\\.\\PhysicalDrive0".into()))
        );
        assert_eq!(
            parse("\\\\server"),
            Err(PlaceError::NotAbsolute("\\\\server".into()))
        );
        assert_eq!(parse(""), Err(PlaceError::Empty));
    }

    #[test]
    fn unix_paths_are_normalized_and_backslash_is_a_name_character() {
        assert_eq!(unix("//home/./owner/../ada/").to_string(), "/home/ada");
        assert_eq!(unix("/a\\b").name(), Some("a\\b"));
        assert_eq!(
            Place::parse_with("D:\\photos", PathStyle::Unix),
            Err(PlaceError::NotAbsolute("D:\\photos".into()))
        );
    }

    #[test]
    fn archive_member_paths_parse_and_round_trip() {
        let p = win("D:\\docs\\course.zip!week1/");
        let (container, member) = p.archive().unwrap();
        assert_eq!(container.to_string(), "D:\\docs\\course.zip");
        assert_eq!(member, "week1");
        assert_eq!(p.to_string(), "D:\\docs\\course.zip!week1");
        assert_eq!(win(&p.to_string()), p);
        assert_eq!(p.title(), "week1, in course.zip");
        assert_eq!(p.kind(), PlaceKind::Archive);

        let root = win("D:\\docs\\course.ZIP!");
        assert_eq!(root.archive().unwrap().1, "");
        assert_eq!(root.title(), "course.ZIP, archive, in D:\\docs");
        assert_eq!(root.parent().unwrap().to_string(), "D:\\docs");
    }

    #[test]
    fn nested_archives_and_bangs_in_names() {
        let p = unix("/srv/outer.tar.gz!inner.7z!dir\\file!.txt");
        let (container, member) = p.archive().unwrap();
        assert_eq!(member, "dir/file!.txt");
        let (outer, inner) = container.archive().unwrap();
        assert_eq!(
            (outer.to_string().as_str(), inner),
            ("/srv/outer.tar.gz", "inner.7z")
        );
        assert_eq!(unix("/wow!/x").local_path(), Some("/wow!/x"));
        assert_eq!(unix("/a.zip!x/").title(), "x, in a.zip");
    }

    #[test]
    fn hostile_member_paths_are_refused() {
        assert_eq!(
            Place::parse_with("/a.zip!../../x", PathStyle::Unix),
            Err(PlaceError::MemberEscapes("../../x".into()))
        );
        assert_eq!(unix("/a.zip!x/../y").archive().unwrap().1, "y");
    }

    #[test]
    fn remote_places_parse() {
        let p = unix("sftp://ada@example.org:2222/home/ada/../ada/docs");
        let remote = p.remote().unwrap();
        assert_eq!(remote.scheme(), "sftp");
        assert_eq!(remote.user(), Some("ada"));
        assert_eq!(remote.host(), "example.org");
        assert_eq!(remote.port(), Some(2222));
        assert_eq!(remote.path(), "/home/ada/docs");
        assert_eq!(p.to_string(), "sftp://ada@example.org:2222/home/ada/docs");
        assert_eq!(p.title(), "docs, on example.org");
        assert_eq!(unix("sftp://[::1]:22").remote().unwrap().host(), "[::1]");
        assert_eq!(unix("sftp://host").title(), "host, SFTP");
        assert!(Place::parse_with("sftp://", PathStyle::Unix).is_err());
        assert!(Place::parse_with("sftp://h:99999/", PathStyle::Unix).is_err());
        assert_eq!(unix("file:///etc/hosts").to_string(), "/etc/hosts");
        assert_eq!(win("file:///C:/Users").to_string(), "C:\\Users");
    }

    #[test]
    fn names_parents_and_joins() {
        let p = win("D:\\photos\\2026");
        assert_eq!(p.name(), Some("2026"));
        assert_eq!(p.parent().unwrap().to_string(), "D:\\photos");
        assert_eq!(p.parent().unwrap().parent().unwrap().to_string(), "D:\\");
        assert!(win("D:\\").parent().is_none());
        assert!(win("D:\\").is_root());
        assert_eq!(win("D:\\").join("x").unwrap().to_string(), "D:\\x");
        assert_eq!(
            p.join("a.txt").unwrap().to_string(),
            "D:\\photos\\2026\\a.txt"
        );
        assert!(p.join("..").is_err());
        assert!(p.join("a\\b").is_err());
        assert!(unix("/").join("a\\b").is_ok());
        let share = win("\\\\srv\\sh\\");
        assert!(share.is_root());
        assert_eq!(share.join("x").unwrap().to_string(), "\\\\srv\\sh\\x");
        assert_eq!(share.join("x").unwrap().parent().unwrap(), share);
        assert_eq!(win("D:\\x").title(), "x, in D:\\");
        assert_eq!(win("D:\\").title(), "Drive D:\\");
        assert_eq!(unix("/").title(), "Root folder /");
        let archive = win("D:\\a.zip").into_archive();
        assert_eq!(archive.join("m").unwrap().to_string(), "D:\\a.zip!m");
        assert_eq!(archive.name(), Some("a.zip"));
    }

    #[test]
    fn resolve_handles_relative_input() {
        let base = win("D:\\docs");
        let resolve = |input| {
            Place::resolve_with(input, &base, PathStyle::Windows)
                .unwrap()
                .to_string()
        };
        assert_eq!(resolve("photos"), "D:\\docs\\photos");
        assert_eq!(resolve("..\\music/rock"), "D:\\music\\rock");
        assert_eq!(resolve("..\\..\\.."), "D:\\");
        assert_eq!(resolve("\\temp"), "D:\\temp");
        assert_eq!(resolve("E:\\x"), "E:\\x");
        assert_eq!(resolve("course.zip!week1"), "D:\\docs\\course.zip!week1");
        assert!(Place::resolve_with("C:x", &base, PathStyle::Windows).is_err());
        let ubase = unix("/home/ada");
        let r = Place::resolve_with("a\\b/c", &ubase, PathStyle::Unix).unwrap();
        assert_eq!(r.to_string(), "/home/ada/a\\b/c");
    }

    #[test]
    fn within_follows_parents_and_ignores_case_on_windows() {
        let root = win("D:\\Docs");
        assert!(win("d:\\docs\\a.zip!x/y").is_within(&root));
        assert!(root.is_within(&root));
        assert!(!win("D:\\Docs2").is_within(&root));
        assert!(!unix("/Home/x").is_within(&unix("/home")));
    }

    #[test]
    fn split_archives_open_as_one_place() {
        let p = unix("/srv/backup.7z.001!dir/file");
        let (container, member) = p.archive().unwrap();
        assert_eq!(container.to_string(), "/srv/backup.7z.001");
        assert_eq!(member, "dir/file");
        assert_eq!(unix(&p.to_string()), p);
        assert_eq!(unix("/srv/BACKUP.TAR.GZ.1000!x").archive().unwrap().1, "x");
        assert_eq!(
            unix("/srv/photo.jpg.001!x").local_path(),
            Some("/srv/photo.jpg.001!x")
        );
        assert_eq!(unix("/srv/a.7z.01!x").local_path(), Some("/srv/a.7z.01!x"));
        assert!(opens_as_archive("backup.7z.001"));
        assert!(!opens_as_archive("photo.jpg.001"));
    }

    #[test]
    fn zip_documents_open_as_archives_and_read_back() {
        let p = win("D:\\docs\\report.docx!word/document.xml");
        assert_eq!(p.archive().unwrap().1, "word/document.xml");
        assert_eq!(win(&p.to_string()), p);
        assert_eq!(
            win("D:\\docs\\report.docx").into_archive().to_string(),
            "D:\\docs\\report.docx!"
        );
        assert!(!ARCHIVE_EXTENSIONS.contains(&".docx"));
    }

    #[test]
    fn archive_markers_are_refused_inside_archives() {
        let root = unix("/a.zip").into_archive();
        assert_eq!(
            root.join("inner.zip!x.txt"),
            Err(PlaceError::ArchiveMarker("inner.zip!x.txt".into()))
        );
        assert!(root.join("Hello!.txt").is_ok());
        assert!(root.join("inner.zip").is_ok());
        assert!(unix("/srv").join("Hello!.txt").is_ok());
    }

    /// W2b's fuzzing finding: a local name holding `.zip!` read back as a
    /// place inside an archive. A name with no stem before the extension
    /// (`.zip`) opens as nothing, so `.zip!.xz` stays a local file; a name
    /// that does hold a marker (`a.zip!b.xz`) cannot be joined into a
    /// local or remote folder, so no place's text reads back as another.
    #[test]
    fn local_names_holding_a_marker_never_read_back_as_archives() {
        let hidden = unix("/fuzz/.zip!.xz");
        assert_eq!(hidden.local_path(), Some("/fuzz/.zip!.xz"));
        assert_eq!(unix("/fuzz").join(".zip!.xz"), Ok(hidden.clone()));
        assert_eq!(unix(&hidden.to_string()), hidden);
        assert_eq!(
            win(r"D:\fuzz\..zip!x").local_path(),
            Some(r"D:\fuzz\..zip!x")
        );
        assert!(!opens_as_archive(".zip"));
        assert!(!opens_as_archive("..zip"));
        assert!(!opens_as_archive(".7z.001"));
        assert!(opens_as_archive(".x.zip"));
        assert!(!holds_archive_marker(".zip!.xz"));
        // A real marker in a local or remote name is refused by name.
        for folder in [unix("/fuzz"), win(r"D:\fuzz"), unix("sftp://host/srv")] {
            assert_eq!(
                folder.join("round.zip!.xz"),
                Err(PlaceError::ReadsAsArchive("round.zip!.xz".into())),
                "{folder}"
            );
        }
        assert!(holds_archive_marker("round.zip!.xz"));
        // So the text is only ever an archive place.
        let p = unix("/fuzz/round.zip!.xz");
        assert_eq!(
            p.archive().map(|(c, m)| (c.to_string(), m)),
            Some(("/fuzz/round.zip".into(), ".xz"))
        );
        // A `!` inside an archive's own name is part of the name, and a
        // name with no stem after a split is a member, not an archive.
        let wow = unix("/srv/wow!.zip!x");
        assert_eq!(wow.archive().unwrap().0.to_string(), "/srv/wow!.zip");
        let dotted = unix("/a.zip!.7z!x");
        assert_eq!(
            dotted.archive().map(|(c, m)| (c.to_string(), m)),
            Some(("/a.zip".into(), ".7z!x"))
        );
        assert_eq!(unix(&dotted.to_string()), dotted);
    }

    /// W2a's request 2: a one-member gzip's member place parses, and a
    /// compressed tar stays one archive.
    #[test]
    fn a_gzip_member_place_parses() {
        let p = win(r"D:\docs\notes.txt.gz!notes.txt");
        let (container, member) = p.archive().unwrap();
        assert_eq!(container.to_string(), r"D:\docs\notes.txt.gz");
        assert_eq!(member, "notes.txt");
        assert_eq!(win(&p.to_string()), p);
        assert_eq!(p.title(), "notes.txt, in notes.txt.gz");
        assert!(opens_as_archive("NOTES.TXT.GZ"));
        assert!(opens_as_archive("logs.tar.gz"));
        assert!(!opens_as_archive(".gz"));
        assert_eq!(unix("/srv/logs.tar.gz!a/b").archive().unwrap().1, "a/b");
        assert!(ARCHIVE_EXTENSIONS.contains(&".gz"));
    }

    #[test]
    fn the_extended_prefix_is_never_shown() {
        assert_eq!(strip_extended_prefix(r"\\?\D:\x\y"), r"D:\x\y");
        assert_eq!(strip_extended_prefix(r"\\?\UNC\srv\sh\x"), r"\\srv\sh\x");
        assert_eq!(strip_extended_prefix(r"\\?\unc\srv\sh"), r"\\srv\sh");
        assert_eq!(strip_extended_prefix(r"D:\x"), r"D:\x");
        assert!(matches!(strip_extended_prefix("/home"), Cow::Borrowed(_)));
    }

    #[test]
    fn places_serialize_as_text() {
        let p = Place::parse(if cfg!(windows) {
            "D:\\a.zip!b"
        } else {
            "/a.zip!b"
        })
        .unwrap();
        let text: String = p.clone().into();
        assert_eq!(Place::try_from(text).unwrap(), p);
    }
}
