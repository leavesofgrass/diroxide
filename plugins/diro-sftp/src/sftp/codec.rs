//! The SFTP packet codec, version 3 as OpenSSH speaks it (R24 section 2.1;
//! draft-ietf-secsh-filexfer-02; answer 88), with no input or output of
//! its own: bytes in, packets out, and back.
//!
//! - **Framing:** a four-byte length (not counting itself), a type byte,
//!   the payload; integers big-endian; strings a four-byte length then the
//!   bytes. A length above the negotiated packet size is refused before
//!   anything is allocated ([`frame`]), and every string and count inside
//!   a packet is checked against what is left before it is read.
//! - **Names are bytes.** Version 3 declares no encoding for file names
//!   and OpenSSH passes bytes through, so every path and name here is a
//!   `Vec<u8>`; the provider decides how to show one that is not UTF-8
//!   (R3's `name_raw`), never this codec.
//! - **Attributes:** a flags word and the fields it announces; a flag bit
//!   version 3 does not know is a protocol error.
//! - **Both directions** encode and decode, so the fake server of the
//!   tests speaks the same codec as the client.

use std::fmt;

/// The protocol version this codec speaks.
pub const VERSION: u32 = 3;

/// The largest packet read when nothing else was agreed: OpenSSH's own
/// server limit, 256 KiB.
pub const DEFAULT_MAX_PACKET: usize = 256 * 1024;

/// Packet types (draft section 3).
pub mod kind {
    /// Client: open the session, with its version.
    pub const INIT: u8 = 1;
    /// Server: the session's version and extensions.
    pub const VERSION: u8 = 2;
    /// Open a file.
    pub const OPEN: u8 = 3;
    /// Close a handle.
    pub const CLOSE: u8 = 4;
    /// Read from a file.
    pub const READ: u8 = 5;
    /// Write to a file.
    pub const WRITE: u8 = 6;
    /// Attributes of a path, links not followed.
    pub const LSTAT: u8 = 7;
    /// Attributes of an open handle.
    pub const FSTAT: u8 = 8;
    /// Change a path's attributes.
    pub const SETSTAT: u8 = 9;
    /// Change an open handle's attributes.
    pub const FSETSTAT: u8 = 10;
    /// Open a folder for reading.
    pub const OPENDIR: u8 = 11;
    /// The next names of an open folder.
    pub const READDIR: u8 = 12;
    /// Remove a file.
    pub const REMOVE: u8 = 13;
    /// Make a folder.
    pub const MKDIR: u8 = 14;
    /// Remove a folder.
    pub const RMDIR: u8 = 15;
    /// A path made absolute.
    pub const REALPATH: u8 = 16;
    /// Attributes of a path, links followed.
    pub const STAT: u8 = 17;
    /// Rename.
    pub const RENAME: u8 = 18;
    /// A link's target.
    pub const READLINK: u8 = 19;
    /// Make a symbolic link.
    pub const SYMLINK: u8 = 20;
    /// Server: how a request ended.
    pub const STATUS: u8 = 101;
    /// Server: an open handle.
    pub const HANDLE: u8 = 102;
    /// Server: bytes read.
    pub const DATA: u8 = 103;
    /// Server: names and their attributes.
    pub const NAME: u8 = 104;
    /// Server: attributes.
    pub const ATTRS: u8 = 105;
    /// Client: an extension's request.
    pub const EXTENDED: u8 = 200;
    /// Server: an extension's answer.
    pub const EXTENDED_REPLY: u8 = 201;
}

/// Status codes (draft section 7). Codes 6 and 7 are client-local and no
/// server sends them.
pub mod status {
    /// Done.
    pub const OK: u32 = 0;
    /// The end of a file or of a folder's names.
    pub const EOF: u32 = 1;
    /// No such file or folder.
    pub const NO_SUCH_FILE: u32 = 2;
    /// Not allowed.
    pub const PERMISSION_DENIED: u32 = 3;
    /// Anything else; OpenSSH uses it for many cases.
    pub const FAILURE: u32 = 4;
    /// A packet the server could not read.
    pub const BAD_MESSAGE: u32 = 5;
    /// Not supported.
    pub const OP_UNSUPPORTED: u32 = 8;
}

/// Open flags (draft section 6.3); `TRUNC` and `EXCL` need `CREAT`.
pub mod open {
    /// Read.
    pub const READ: u32 = 0x01;
    /// Write.
    pub const WRITE: u32 = 0x02;
    /// Every write goes at the end.
    pub const APPEND: u32 = 0x04;
    /// Create when absent.
    pub const CREAT: u32 = 0x08;
    /// Cut to nothing.
    pub const TRUNC: u32 = 0x10;
    /// Fail when present.
    pub const EXCL: u32 = 0x20;
}

/// Attribute flags (draft section 5).
pub mod attr {
    /// The size is present.
    pub const SIZE: u32 = 0x0000_0001;
    /// The user and group numbers are present.
    pub const UIDGID: u32 = 0x0000_0002;
    /// The permissions are present.
    pub const PERMISSIONS: u32 = 0x0000_0004;
    /// The access and change times are present.
    pub const ACMODTIME: u32 = 0x0000_0008;
    /// Extended pairs follow.
    pub const EXTENDED: u32 = 0x8000_0000;
    /// Every bit version 3 knows.
    pub const KNOWN: u32 = SIZE | UIDGID | PERMISSIONS | ACMODTIME | EXTENDED;
}

/// Why bytes were not a packet.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// The length is above the agreed limit or below the smallest packet.
    #[error("a packet of {0} bytes is not accepted")]
    Length(usize),
    /// The packet ends before a field it announces.
    #[error("the packet is cut short")]
    Short,
    /// Bytes are left after the last field.
    #[error("the packet has bytes after its last field")]
    Trailing,
    /// A type this side does not take in this direction.
    #[error("packet type {0} is not expected here")]
    Type(u8),
    /// An attribute flag version 3 does not know.
    #[error("attribute flags 0x{0:08x} are not known")]
    AttrFlags(u32),
}

/// A file's attributes; `None` is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs {
    /// The size in bytes.
    pub size: Option<u64>,
    /// The user and group numbers.
    pub uid_gid: Option<(u32, u32)>,
    /// The permission bits and the file type bits.
    pub permissions: Option<u32>,
    /// The access and change times, whole seconds since 1970 (they end in
    /// 2106).
    pub times: Option<(u32, u32)>,
    /// Extended pairs: a name and its data.
    pub extended: Vec<(Vec<u8>, Vec<u8>)>,
}

/// One name of a NAME answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct NameEntry {
    /// The name's bytes.
    pub filename: Vec<u8>,
    /// The server's `ls -l` line; never shown and never trusted.
    pub longname: Vec<u8>,
    /// Its attributes.
    pub attrs: Attrs,
}

/// An extension's name and data, as INIT and VERSION carry them.
pub type Extension = (Vec<u8>, Vec<u8>);

/// What the client sends.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Request {
    /// Opens the session (no id).
    Init {
        /// The client's version, 3.
        version: u32,
        /// Extensions the client offers.
        extensions: Vec<Extension>,
    },
    /// Opens a file.
    Open {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
        /// The [`open`] flags.
        pflags: u32,
        /// The attributes for a new file.
        attrs: Attrs,
    },
    /// Closes a handle.
    Close {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
    },
    /// Reads from a file.
    Read {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
        /// Where.
        offset: u64,
        /// How many bytes at most.
        len: u32,
    },
    /// Writes to a file.
    Write {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
        /// Where.
        offset: u64,
        /// The bytes.
        data: Vec<u8>,
    },
    /// A path's attributes, a link not followed.
    Lstat {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// An open handle's attributes.
    Fstat {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
    },
    /// Changes a path's attributes.
    Setstat {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
        /// The attributes to set.
        attrs: Attrs,
    },
    /// Changes an open handle's attributes.
    Fsetstat {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
        /// The attributes to set.
        attrs: Attrs,
    },
    /// Opens a folder for reading.
    Opendir {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// The next names of an open folder.
    Readdir {
        /// The request id.
        id: u32,
        /// The handle.
        handle: Vec<u8>,
    },
    /// Removes a file.
    Remove {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// Makes a folder.
    Mkdir {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
        /// Its attributes.
        attrs: Attrs,
    },
    /// Removes a folder.
    Rmdir {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// Makes a path absolute.
    Realpath {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// A path's attributes, links followed.
    Stat {
        /// The request id.
        id: u32,
        /// The path.
        path: Vec<u8>,
    },
    /// Renames; plain RENAME does not replace an existing target.
    Rename {
        /// The request id.
        id: u32,
        /// The old path.
        from: Vec<u8>,
        /// The new path.
        to: Vec<u8>,
    },
    /// A link's target.
    Readlink {
        /// The request id.
        id: u32,
        /// The link's path.
        path: Vec<u8>,
    },
    /// Makes a symbolic link. The two strings go in the order given: the
    /// draft says link path then target, OpenSSH reads target then link
    /// path (its PROTOCOL file, 4.1), so the caller orders them for the
    /// server it speaks to.
    Symlink {
        /// The request id.
        id: u32,
        /// The first string.
        first: Vec<u8>,
        /// The second string.
        second: Vec<u8>,
    },
    /// An extension's request.
    Extended {
        /// The request id.
        id: u32,
        /// The extension's name (`posix-rename@openssh.com`).
        name: Vec<u8>,
        /// Its data, encoded by the extension.
        data: Vec<u8>,
    },
}

/// What the server answers.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Response {
    /// The session's version and the server's extensions (no id).
    Version {
        /// The server's version.
        version: u32,
        /// Its extensions, each a name and a version or data.
        extensions: Vec<Extension>,
    },
    /// How a request ended.
    Status {
        /// The request id.
        id: u32,
        /// The [`status`] code.
        code: u32,
        /// The server's words; never trusted, only cleaned and placed
        /// last.
        message: Vec<u8>,
        /// The words' language tag.
        language: Vec<u8>,
    },
    /// An open handle.
    Handle {
        /// The request id.
        id: u32,
        /// The handle (at most 256 bytes).
        handle: Vec<u8>,
    },
    /// Bytes read.
    Data {
        /// The request id.
        id: u32,
        /// The bytes.
        data: Vec<u8>,
    },
    /// Names and their attributes.
    Name {
        /// The request id.
        id: u32,
        /// The names.
        entries: Vec<NameEntry>,
    },
    /// Attributes.
    Attrs {
        /// The request id.
        id: u32,
        /// The attributes.
        attrs: Attrs,
    },
    /// An extension's answer.
    ExtendedReply {
        /// The request id.
        id: u32,
        /// Its data, decoded by the extension.
        data: Vec<u8>,
    },
}

fn shorten(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(32)]).into_owned();
    if bytes.len() > 32 {
        format!("{text}...({} bytes)", bytes.len())
    } else {
        text
    }
}

impl fmt::Debug for Request {
    /// Paths are shown cut; data never, only its length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Write {
                id, offset, data, ..
            } => write!(f, "Write(id {id}, offset {offset}, {} bytes)", data.len()),
            Request::Extended { id, name, data } => write!(
                f,
                "Extended(id {id}, {}, {} bytes)",
                shorten(name),
                data.len()
            ),
            other => write!(f, "{}(id {:?})", other.name(), other.id()),
        }
    }
}

impl fmt::Debug for Response {
    /// Data never shown, only its length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Response::Data { id, data } => write!(f, "Data(id {id}, {} bytes)", data.len()),
            Response::Status { id, code, .. } => write!(f, "Status(id {id}, code {code})"),
            Response::Name { id, entries } => write!(f, "Name(id {id}, {} names)", entries.len()),
            other => write!(f, "{}(id {:?})", other.name(), other.id()),
        }
    }
}

/// A reader over a packet's payload that checks every length.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if n > self.bytes.len() {
            return Err(CodecError::Short);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, CodecError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn string(&mut self) -> Result<Vec<u8>, CodecError> {
        let len = self.u32()? as usize;
        // Checked against what is left before anything is allocated.
        Ok(self.take(len)?.to_vec())
    }

    fn attrs(&mut self) -> Result<Attrs, CodecError> {
        let flags = self.u32()?;
        if flags & !attr::KNOWN != 0 {
            return Err(CodecError::AttrFlags(flags));
        }
        let mut attrs = Attrs::default();
        if flags & attr::SIZE != 0 {
            attrs.size = Some(self.u64()?);
        }
        if flags & attr::UIDGID != 0 {
            attrs.uid_gid = Some((self.u32()?, self.u32()?));
        }
        if flags & attr::PERMISSIONS != 0 {
            attrs.permissions = Some(self.u32()?);
        }
        if flags & attr::ACMODTIME != 0 {
            attrs.times = Some((self.u32()?, self.u32()?));
        }
        if flags & attr::EXTENDED != 0 {
            let count = self.u32()? as usize;
            // Each pair takes at least eight bytes.
            if count > self.bytes.len() / 8 {
                return Err(CodecError::Short);
            }
            for _ in 0..count {
                attrs.extended.push((self.string()?, self.string()?));
            }
        }
        Ok(attrs)
    }

    fn extensions(&mut self) -> Result<Vec<Extension>, CodecError> {
        let mut out = Vec::new();
        while !self.bytes.is_empty() {
            out.push((self.string()?, self.string()?));
        }
        Ok(out)
    }

    fn end(&self) -> Result<(), CodecError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(CodecError::Trailing)
        }
    }
}

/// A writer of a packet's payload.
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new(kind: u8) -> Self {
        // Room for the length, filled in by `finish`.
        Self {
            bytes: vec![0, 0, 0, 0, kind],
        }
    }

    fn u32(&mut self, n: u32) -> &mut Self {
        self.bytes.extend_from_slice(&n.to_be_bytes());
        self
    }

    fn u64(&mut self, n: u64) -> &mut Self {
        self.bytes.extend_from_slice(&n.to_be_bytes());
        self
    }

    fn string(&mut self, s: &[u8]) -> &mut Self {
        let len = u32::try_from(s.len()).unwrap_or(u32::MAX);
        self.u32(len);
        self.bytes.extend_from_slice(s);
        self
    }

    fn attrs(&mut self, a: &Attrs) -> &mut Self {
        let mut flags = 0;
        if a.size.is_some() {
            flags |= attr::SIZE;
        }
        if a.uid_gid.is_some() {
            flags |= attr::UIDGID;
        }
        if a.permissions.is_some() {
            flags |= attr::PERMISSIONS;
        }
        if a.times.is_some() {
            flags |= attr::ACMODTIME;
        }
        if !a.extended.is_empty() {
            flags |= attr::EXTENDED;
        }
        self.u32(flags);
        if let Some(size) = a.size {
            self.u64(size);
        }
        if let Some((uid, gid)) = a.uid_gid {
            self.u32(uid).u32(gid);
        }
        if let Some(permissions) = a.permissions {
            self.u32(permissions);
        }
        if let Some((atime, mtime)) = a.times {
            self.u32(atime).u32(mtime);
        }
        if !a.extended.is_empty() {
            self.u32(u32::try_from(a.extended.len()).unwrap_or(u32::MAX));
            for (name, data) in &a.extended {
                self.string(name).string(data);
            }
        }
        self
    }

    fn extensions(&mut self, extensions: &[Extension]) -> &mut Self {
        for (name, data) in extensions {
            self.string(name).string(data);
        }
        self
    }

    fn finish(&mut self) -> Vec<u8> {
        let len = u32::try_from(self.bytes.len() - 4).unwrap_or(u32::MAX);
        self.bytes[..4].copy_from_slice(&len.to_be_bytes());
        std::mem::take(&mut self.bytes)
    }
}

/// Finds one whole packet at the start of `buffer`: `Ok(None)` when more
/// bytes are needed, `Ok(Some((body, used)))` with the type byte and
/// payload and the bytes it used, and an error, before anything is
/// allocated, when its length is above `max` or below one byte.
pub fn frame(buffer: &[u8], max: usize) -> Result<Option<(&[u8], usize)>, CodecError> {
    let Some(head) = buffer.get(..4) else {
        return Ok(None);
    };
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if len == 0 || len > max {
        return Err(CodecError::Length(len));
    }
    Ok(buffer.get(4..4 + len).map(|body| (body, 4 + len)))
}

impl Request {
    /// The request's name, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Request::Init { .. } => "Init",
            Request::Open { .. } => "Open",
            Request::Close { .. } => "Close",
            Request::Read { .. } => "Read",
            Request::Write { .. } => "Write",
            Request::Lstat { .. } => "Lstat",
            Request::Fstat { .. } => "Fstat",
            Request::Setstat { .. } => "Setstat",
            Request::Fsetstat { .. } => "Fsetstat",
            Request::Opendir { .. } => "Opendir",
            Request::Readdir { .. } => "Readdir",
            Request::Remove { .. } => "Remove",
            Request::Mkdir { .. } => "Mkdir",
            Request::Rmdir { .. } => "Rmdir",
            Request::Realpath { .. } => "Realpath",
            Request::Stat { .. } => "Stat",
            Request::Rename { .. } => "Rename",
            Request::Readlink { .. } => "Readlink",
            Request::Symlink { .. } => "Symlink",
            Request::Extended { .. } => "Extended",
        }
    }

    /// The request id; `None` for INIT.
    pub fn id(&self) -> Option<u32> {
        match self {
            Request::Init { .. } => None,
            Request::Open { id, .. }
            | Request::Close { id, .. }
            | Request::Read { id, .. }
            | Request::Write { id, .. }
            | Request::Lstat { id, .. }
            | Request::Fstat { id, .. }
            | Request::Setstat { id, .. }
            | Request::Fsetstat { id, .. }
            | Request::Opendir { id, .. }
            | Request::Readdir { id, .. }
            | Request::Remove { id, .. }
            | Request::Mkdir { id, .. }
            | Request::Rmdir { id, .. }
            | Request::Realpath { id, .. }
            | Request::Stat { id, .. }
            | Request::Rename { id, .. }
            | Request::Readlink { id, .. }
            | Request::Symlink { id, .. }
            | Request::Extended { id, .. } => Some(*id),
        }
    }

    /// The packet, its length first.
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Init {
                version,
                extensions,
            } => {
                w = Writer::new(kind::INIT);
                w.u32(*version).extensions(extensions);
            }
            Request::Open {
                id,
                path,
                pflags,
                attrs,
            } => {
                w = Writer::new(kind::OPEN);
                w.u32(*id).string(path).u32(*pflags).attrs(attrs);
            }
            Request::Close { id, handle } => {
                w = Writer::new(kind::CLOSE);
                w.u32(*id).string(handle);
            }
            Request::Read {
                id,
                handle,
                offset,
                len,
            } => {
                w = Writer::new(kind::READ);
                w.u32(*id).string(handle).u64(*offset).u32(*len);
            }
            Request::Write {
                id,
                handle,
                offset,
                data,
            } => {
                w = Writer::new(kind::WRITE);
                w.u32(*id).string(handle).u64(*offset).string(data);
            }
            Request::Lstat { id, path } => {
                w = Writer::new(kind::LSTAT);
                w.u32(*id).string(path);
            }
            Request::Fstat { id, handle } => {
                w = Writer::new(kind::FSTAT);
                w.u32(*id).string(handle);
            }
            Request::Setstat { id, path, attrs } => {
                w = Writer::new(kind::SETSTAT);
                w.u32(*id).string(path).attrs(attrs);
            }
            Request::Fsetstat { id, handle, attrs } => {
                w = Writer::new(kind::FSETSTAT);
                w.u32(*id).string(handle).attrs(attrs);
            }
            Request::Opendir { id, path } => {
                w = Writer::new(kind::OPENDIR);
                w.u32(*id).string(path);
            }
            Request::Readdir { id, handle } => {
                w = Writer::new(kind::READDIR);
                w.u32(*id).string(handle);
            }
            Request::Remove { id, path } => {
                w = Writer::new(kind::REMOVE);
                w.u32(*id).string(path);
            }
            Request::Mkdir { id, path, attrs } => {
                w = Writer::new(kind::MKDIR);
                w.u32(*id).string(path).attrs(attrs);
            }
            Request::Rmdir { id, path } => {
                w = Writer::new(kind::RMDIR);
                w.u32(*id).string(path);
            }
            Request::Realpath { id, path } => {
                w = Writer::new(kind::REALPATH);
                w.u32(*id).string(path);
            }
            Request::Stat { id, path } => {
                w = Writer::new(kind::STAT);
                w.u32(*id).string(path);
            }
            Request::Rename { id, from, to } => {
                w = Writer::new(kind::RENAME);
                w.u32(*id).string(from).string(to);
            }
            Request::Readlink { id, path } => {
                w = Writer::new(kind::READLINK);
                w.u32(*id).string(path);
            }
            Request::Symlink { id, first, second } => {
                w = Writer::new(kind::SYMLINK);
                w.u32(*id).string(first).string(second);
            }
            Request::Extended { id, name, data } => {
                w = Writer::new(kind::EXTENDED);
                w.u32(*id).string(name);
                w.bytes.extend_from_slice(data);
            }
        }
        w.finish()
    }

    /// Reads a request from a packet's body (its type byte and payload).
    pub fn decode(body: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader { bytes: body };
        let kind = r.u8()?;
        let request = match kind {
            kind::INIT => Request::Init {
                version: r.u32()?,
                extensions: r.extensions()?,
            },
            kind::EXTENDED => {
                let id = r.u32()?;
                let name = r.string()?;
                let data = r.bytes.to_vec();
                r.bytes = &[];
                Request::Extended { id, name, data }
            }
            _ => {
                if !(kind::OPEN..=kind::SYMLINK).contains(&kind) {
                    return Err(CodecError::Type(kind));
                }
                let id = r.u32()?;
                match kind {
                    kind::OPEN => Request::Open {
                        id,
                        path: r.string()?,
                        pflags: r.u32()?,
                        attrs: r.attrs()?,
                    },
                    kind::CLOSE => Request::Close {
                        id,
                        handle: r.string()?,
                    },
                    kind::READ => Request::Read {
                        id,
                        handle: r.string()?,
                        offset: r.u64()?,
                        len: r.u32()?,
                    },
                    kind::WRITE => Request::Write {
                        id,
                        handle: r.string()?,
                        offset: r.u64()?,
                        data: r.string()?,
                    },
                    kind::LSTAT => Request::Lstat {
                        id,
                        path: r.string()?,
                    },
                    kind::FSTAT => Request::Fstat {
                        id,
                        handle: r.string()?,
                    },
                    kind::SETSTAT => Request::Setstat {
                        id,
                        path: r.string()?,
                        attrs: r.attrs()?,
                    },
                    kind::FSETSTAT => Request::Fsetstat {
                        id,
                        handle: r.string()?,
                        attrs: r.attrs()?,
                    },
                    kind::OPENDIR => Request::Opendir {
                        id,
                        path: r.string()?,
                    },
                    kind::READDIR => Request::Readdir {
                        id,
                        handle: r.string()?,
                    },
                    kind::REMOVE => Request::Remove {
                        id,
                        path: r.string()?,
                    },
                    kind::MKDIR => Request::Mkdir {
                        id,
                        path: r.string()?,
                        attrs: r.attrs()?,
                    },
                    kind::RMDIR => Request::Rmdir {
                        id,
                        path: r.string()?,
                    },
                    kind::REALPATH => Request::Realpath {
                        id,
                        path: r.string()?,
                    },
                    kind::STAT => Request::Stat {
                        id,
                        path: r.string()?,
                    },
                    kind::RENAME => Request::Rename {
                        id,
                        from: r.string()?,
                        to: r.string()?,
                    },
                    kind::READLINK => Request::Readlink {
                        id,
                        path: r.string()?,
                    },
                    kind::SYMLINK => Request::Symlink {
                        id,
                        first: r.string()?,
                        second: r.string()?,
                    },
                    other => return Err(CodecError::Type(other)),
                }
            }
        };
        r.end()?;
        Ok(request)
    }
}

impl Response {
    /// The answer's name, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Response::Version { .. } => "Version",
            Response::Status { .. } => "Status",
            Response::Handle { .. } => "Handle",
            Response::Data { .. } => "Data",
            Response::Name { .. } => "Name",
            Response::Attrs { .. } => "Attrs",
            Response::ExtendedReply { .. } => "ExtendedReply",
        }
    }

    /// The request id it answers; `None` for VERSION.
    pub fn id(&self) -> Option<u32> {
        match self {
            Response::Version { .. } => None,
            Response::Status { id, .. }
            | Response::Handle { id, .. }
            | Response::Data { id, .. }
            | Response::Name { id, .. }
            | Response::Attrs { id, .. }
            | Response::ExtendedReply { id, .. } => Some(*id),
        }
    }

    /// The packet, its length first.
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Response::Version {
                version,
                extensions,
            } => {
                w = Writer::new(kind::VERSION);
                w.u32(*version).extensions(extensions);
            }
            Response::Status {
                id,
                code,
                message,
                language,
            } => {
                w = Writer::new(kind::STATUS);
                w.u32(*id).u32(*code).string(message).string(language);
            }
            Response::Handle { id, handle } => {
                w = Writer::new(kind::HANDLE);
                w.u32(*id).string(handle);
            }
            Response::Data { id, data } => {
                w = Writer::new(kind::DATA);
                w.u32(*id).string(data);
            }
            Response::Name { id, entries } => {
                w = Writer::new(kind::NAME);
                w.u32(*id)
                    .u32(u32::try_from(entries.len()).unwrap_or(u32::MAX));
                for entry in entries {
                    w.string(&entry.filename)
                        .string(&entry.longname)
                        .attrs(&entry.attrs);
                }
            }
            Response::Attrs { id, attrs } => {
                w = Writer::new(kind::ATTRS);
                w.u32(*id).attrs(attrs);
            }
            Response::ExtendedReply { id, data } => {
                w = Writer::new(kind::EXTENDED_REPLY);
                w.u32(*id);
                w.bytes.extend_from_slice(data);
            }
        }
        w.finish()
    }

    /// Reads an answer from a packet's body (its type byte and payload).
    pub fn decode(body: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader { bytes: body };
        let kind = r.u8()?;
        let response = match kind {
            kind::VERSION => Response::Version {
                version: r.u32()?,
                extensions: r.extensions()?,
            },
            kind::STATUS => {
                let id = r.u32()?;
                let code = r.u32()?;
                // Some old servers end after the code.
                let (message, language) = if r.bytes.is_empty() {
                    (Vec::new(), Vec::new())
                } else {
                    (r.string()?, r.string()?)
                };
                Response::Status {
                    id,
                    code,
                    message,
                    language,
                }
            }
            kind::HANDLE => Response::Handle {
                id: r.u32()?,
                handle: r.string()?,
            },
            kind::DATA => Response::Data {
                id: r.u32()?,
                data: r.string()?,
            },
            kind::NAME => {
                let id = r.u32()?;
                let count = r.u32()? as usize;
                // Each name takes at least twelve bytes.
                if count > r.bytes.len() / 12 {
                    return Err(CodecError::Short);
                }
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(NameEntry {
                        filename: r.string()?,
                        longname: r.string()?,
                        attrs: r.attrs()?,
                    });
                }
                Response::Name { id, entries }
            }
            kind::ATTRS => Response::Attrs {
                id: r.u32()?,
                attrs: r.attrs()?,
            },
            kind::EXTENDED_REPLY => {
                let id = r.u32()?;
                let data = r.bytes.to_vec();
                r.bytes = &[];
                Response::ExtendedReply { id, data }
            }
            other => return Err(CodecError::Type(other)),
        };
        r.end()?;
        Ok(response)
    }
}

/// The free-space answer of `statvfs@openssh.com`: eleven numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StatVfs {
    /// The block size.
    pub bsize: u64,
    /// The fragment size, the unit of the counts.
    pub frsize: u64,
    /// Blocks in all.
    pub blocks: u64,
    /// Free blocks.
    pub bfree: u64,
    /// Free blocks for a user who is not root.
    pub bavail: u64,
    /// File nodes in all.
    pub files: u64,
    /// Free file nodes.
    pub ffree: u64,
    /// Free file nodes for a user who is not root.
    pub favail: u64,
    /// The file system's id.
    pub fsid: u64,
    /// Mount flags.
    pub flag: u64,
    /// The longest name.
    pub namemax: u64,
}

impl StatVfs {
    /// The bytes a user who is not root may still write.
    pub fn free_to_user(&self) -> u64 {
        self.bavail.saturating_mul(self.frsize)
    }

    /// The size of the file system.
    pub fn total(&self) -> u64 {
        self.blocks.saturating_mul(self.frsize)
    }

    /// Reads the extension's answer data.
    pub fn decode(data: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader { bytes: data };
        let v = Self {
            bsize: r.u64()?,
            frsize: r.u64()?,
            blocks: r.u64()?,
            bfree: r.u64()?,
            bavail: r.u64()?,
            files: r.u64()?,
            ffree: r.u64()?,
            favail: r.u64()?,
            fsid: r.u64()?,
            flag: r.u64()?,
            namemax: r.u64()?,
        };
        r.end()?;
        Ok(v)
    }

    /// The extension's answer data.
    pub fn encode(&self) -> Vec<u8> {
        [
            self.bsize,
            self.frsize,
            self.blocks,
            self.bfree,
            self.bavail,
            self.files,
            self.ffree,
            self.favail,
            self.fsid,
            self.flag,
            self.namemax,
        ]
        .iter()
        .flat_map(|n| n.to_be_bytes())
        .collect()
    }
}

/// The answer of `limits@openssh.com`: the server's own limits, 0 for no
/// limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Limits {
    /// The longest packet.
    pub max_packet: u64,
    /// The longest read.
    pub max_read: u64,
    /// The longest write.
    pub max_write: u64,
    /// The most handles open at once.
    pub max_open_handles: u64,
}

impl Limits {
    /// Reads the extension's answer data.
    pub fn decode(data: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader { bytes: data };
        let v = Self {
            max_packet: r.u64()?,
            max_read: r.u64()?,
            max_write: r.u64()?,
            max_open_handles: r.u64()?,
        };
        r.end()?;
        Ok(v)
    }

    /// The extension's answer data.
    pub fn encode(&self) -> Vec<u8> {
        [
            self.max_packet,
            self.max_read,
            self.max_write,
            self.max_open_handles,
        ]
        .iter()
        .flat_map(|n| n.to_be_bytes())
        .collect()
    }
}

/// An extension request's data made of strings (`posix-rename`,
/// `statvfs`, `hardlink`): each a length and its bytes.
pub fn strings(parts: &[&[u8]]) -> Vec<u8> {
    let mut w = Writer::default();
    for part in parts {
        w.string(part);
    }
    w.bytes
}

/// Reads an extension request's data made of strings.
pub fn read_strings(data: &[u8], count: usize) -> Result<Vec<Vec<u8>>, CodecError> {
    let mut r = Reader { bytes: data };
    let mut out = Vec::with_capacity(count.min(8));
    for _ in 0..count {
        out.push(r.string()?);
    }
    r.end()?;
    Ok(out)
}
