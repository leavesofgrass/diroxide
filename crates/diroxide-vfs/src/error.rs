//! Errors of providers, with a class the job engine uses to choose the
//! recovery (research/file-operations.md, "Errors").

use std::io;

use diroxide_core::{Explain, PlaceError};
use thiserror::Error;

/// Why a provider operation failed. The text is the cause, in lowercase
/// words, for "Could not VERB OBJECT: CAUSE." messages; no next step
/// names a key, since keys are said from the live keymap.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VfsError {
    /// The operation was cancelled.
    #[error("it was cancelled")]
    Cancelled,
    /// The place does not exist.
    #[error("it does not exist")]
    NotFound,
    /// The system refused access.
    #[error("permission was denied")]
    PermissionDenied,
    /// The name is already taken.
    #[error("the name is already taken")]
    AlreadyExists,
    /// A folder that must be empty is not.
    #[error("the folder is not empty")]
    NotEmpty,
    /// A folder was expected and this is not one.
    #[error("it is not a folder")]
    NotAFolder,
    /// The provider cannot do this.
    #[error("the {provider} provider cannot {operation}")]
    Unsupported {
        /// The provider's scheme.
        provider: String,
        /// What was asked, as a verb phrase: "move to the trash".
        operation: String,
    },
    /// No provider is registered for the place.
    #[error("nothing is set up to open {0} places")]
    NoProvider(String),
    /// The place cannot be used on this system.
    #[error(transparent)]
    Place(#[from] PlaceError),
    /// The name is not valid Unicode, so it cannot be used safely yet.
    #[error("the name is not valid Unicode")]
    NameNotUnicode,
    /// The drive is full.
    #[error("the drive is full")]
    StorageFull,
    /// The user's quota on the drive is used up.
    #[error("the quota on this drive is used up")]
    QuotaExceeded,
    /// The drive or file system is read only.
    #[error("the drive is read only")]
    ReadOnlyFilesystem,
    /// A rename cannot cross to another drive or volume; the engine copies
    /// and removes instead.
    #[error("it is on another drive")]
    CrossesDevices,
    /// Another program has the file open.
    #[error("another program is using it")]
    InUse,
    /// The network place went away (a stale handle, the network down, the
    /// host unreachable, the connection reset or timed out).
    #[error("the network connection was lost")]
    NetworkLost,
    /// The name is not allowed here, or too long for this drive.
    #[error("the name is not allowed on this drive")]
    NameInvalid,
    /// The file is too large for this drive (4 GiB on FAT).
    #[error("it is too large for this drive")]
    FileTooLarge,
    /// Too many links, or a loop of links.
    #[error("there are too many links")]
    TooManyLinks,
    /// The data needs a password that was not given.
    #[error("a password is needed")]
    PasswordRequired,
    /// The password was wrong, or the data is damaged (some formats cannot
    /// tell the two apart).
    #[error("the password is wrong, or the data is damaged")]
    WrongPassword,
    /// The data is damaged.
    #[error("it is damaged: {what}")]
    Corrupt {
        /// What is damaged, in words: "the central directory".
        what: String,
    },
    /// A limit that guards against hostile data was reached.
    #[error("it passes the limit for {what}: {limit}")]
    LimitReached {
        /// What the limit is for: "bytes unpacked".
        what: String,
        /// The limit in words: "64 GB".
        limit: String,
    },
    /// The data uses something this program does not read.
    #[error("it uses {what}, which is not supported")]
    UnsupportedFeature {
        /// What it uses: "compression method 99".
        what: String,
    },
    /// A safety rule refused it.
    #[error("it was refused: {why}")]
    Unsafe {
        /// Why, in words: "the name leaves the archive".
        why: String,
    },
    /// The trash cannot take it here (the trash gate): a removable or
    /// network drive, or a path too long for the trash.
    #[error("the trash cannot take it here: {why}")]
    TrashRefused {
        /// Why, in words: "this drive has no trash".
        why: String,
    },
    /// Any other I/O failure, with the system's words.
    #[error("{message}")]
    Io {
        /// The kind of failure.
        kind: io::ErrorKind,
        /// The system's description, in lowercase.
        message: String,
    },
}

/// The class of an error, which decides the recovery the engine offers:
/// whether retrying can help, whether to retry on its own, and which of
/// Retry, Skip, Skip all like this and Cancel comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Cancelled: no message beyond the summary.
    Cancelled,
    /// The item is gone, or a different kind than expected.
    Missing,
    /// The name is taken.
    Exists,
    /// Permission was denied.
    Access,
    /// In use by another program: retrying after closing it can help.
    InUse,
    /// The drive is full, or the quota used up.
    Storage,
    /// The drive is read only.
    ReadOnly,
    /// The name is not allowed, or too long.
    Name,
    /// The file is too large for the drive.
    TooLarge,
    /// The network was lost: retried on its own, three times.
    Network,
    /// A password is needed or was wrong.
    Secret,
    /// Damaged data, a limit, or something not supported.
    Data,
    /// A safety rule refused it, or the provider cannot do it.
    Refused,
    /// Anything else.
    Other,
}

/// Where the cause lies, for the result list and the reports (R13, R14,
/// R16): this one item, a rule, or the surroundings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorScope {
    /// Something about this item: missing, taken, locked, damaged.
    File,
    /// A rule refused it: a safety rule, a limit, a name rule.
    Rule,
    /// The surroundings: the drive is full or read only, the network is
    /// lost.
    Environment,
}

impl ErrorClass {
    /// Whether retrying can help, so Retry is offered first.
    pub const fn retry_first(self) -> bool {
        matches!(self, ErrorClass::InUse | ErrorClass::Network)
    }

    /// Whether the engine retries on its own before asking (three times,
    /// five seconds apart, a setting).
    pub const fn retries_on_its_own(self) -> bool {
        matches!(self, ErrorClass::Network)
    }

    /// Whether Retry is offered at all.
    pub const fn can_retry(self) -> bool {
        !matches!(
            self,
            ErrorClass::Cancelled
                | ErrorClass::ReadOnly
                | ErrorClass::Refused
                | ErrorClass::TooLarge
                | ErrorClass::Data
        )
    }

    /// Where the cause lies.
    pub const fn scope(self) -> ErrorScope {
        match self {
            ErrorClass::Storage | ErrorClass::ReadOnly | ErrorClass::Network => {
                ErrorScope::Environment
            }
            ErrorClass::Name | ErrorClass::TooLarge | ErrorClass::Refused => ErrorScope::Rule,
            _ => ErrorScope::File,
        }
    }
}

impl VfsError {
    /// An "unsupported" error for a provider and an operation.
    pub fn unsupported(provider: &str, operation: &str) -> Self {
        Self::Unsupported {
            provider: provider.to_owned(),
            operation: operation.to_owned(),
        }
    }

    /// The error's class, for the engine's choice of recovery.
    pub fn class(&self) -> ErrorClass {
        match self {
            VfsError::Cancelled => ErrorClass::Cancelled,
            VfsError::NotFound | VfsError::NotAFolder => ErrorClass::Missing,
            VfsError::AlreadyExists | VfsError::NotEmpty => ErrorClass::Exists,
            VfsError::PermissionDenied => ErrorClass::Access,
            VfsError::InUse => ErrorClass::InUse,
            VfsError::StorageFull | VfsError::QuotaExceeded => ErrorClass::Storage,
            VfsError::ReadOnlyFilesystem => ErrorClass::ReadOnly,
            VfsError::NameInvalid | VfsError::NameNotUnicode | VfsError::Place(_) => {
                ErrorClass::Name
            }
            VfsError::FileTooLarge => ErrorClass::TooLarge,
            VfsError::NetworkLost => ErrorClass::Network,
            VfsError::PasswordRequired | VfsError::WrongPassword => ErrorClass::Secret,
            VfsError::Corrupt { .. }
            | VfsError::LimitReached { .. }
            | VfsError::UnsupportedFeature { .. } => ErrorClass::Data,
            VfsError::Unsupported { .. }
            | VfsError::NoProvider(_)
            | VfsError::Unsafe { .. }
            | VfsError::TrashRefused { .. } => ErrorClass::Refused,
            VfsError::CrossesDevices | VfsError::TooManyLinks | VfsError::Io { .. } => {
                ErrorClass::Other
            }
        }
    }
}

/// Windows error codes the standard library leaves without a kind of
/// their own, or whose kind is not stable (winerror.h).
fn from_windows_code(code: i32) -> Option<VfsError> {
    Some(match code {
        // Sharing and lock violations.
        32 | 33 => VfsError::InUse,
        // Not the same device: a move to another drive.
        17 => VfsError::CrossesDevices,
        // The disk is write-protected.
        19 => VfsError::ReadOnlyFilesystem,
        // The disk is full, or the handle's disk is.
        39 | 112 => VfsError::StorageFull,
        // The quota is used up.
        1295 => VfsError::QuotaExceeded,
        // The file is too large for the file system.
        223 => VfsError::FileTooLarge,
        // A name the system will not take, or a path too long for it.
        123 | 206 => VfsError::NameInvalid,
        // The links could not be resolved (a loop).
        1921 => VfsError::TooManyLinks,
        // The network went away: not listed, bad path, device gone,
        // unexpected error, name deleted, bad name, timeout, unreachable
        // network or host, connection aborted.
        51 | 53 | 55 | 59 | 64 | 67 | 121 | 1231 | 1232 | 1236 => VfsError::NetworkLost,
        _ => return None,
    })
}

/// Unix error codes whose kinds are not stable in the standard library:
/// a loop of links (`ELOOP`) and a running program's file (`ETXTBSY`).
fn from_unix_code(code: i32) -> Option<VfsError> {
    const ETXTBSY: i32 = 26;
    let eloop = if cfg!(target_vendor = "apple") {
        62
    } else {
        40
    };
    if code == eloop {
        Some(VfsError::TooManyLinks)
    } else if code == ETXTBSY {
        Some(VfsError::InUse)
    } else {
        None
    }
}

/// The system's words for an error, as the cause of a sentence: lowercase
/// first, without the "(os error 21)" number or a final period, which
/// read badly aloud in "Could not copy x: CAUSE. NEXT STEP."
fn cause_words(error: &io::Error) -> String {
    let text = error.to_string();
    let text = match text.rfind(" (os error ") {
        Some(at) if text.ends_with(')') => &text[..at],
        _ => text.as_str(),
    };
    let text = text.trim().trim_end_matches('.');
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "an input or output error happened".to_owned(),
    }
}

impl From<io::Error> for VfsError {
    fn from(error: io::Error) -> Self {
        if let Some(code) = error.raw_os_error() {
            let known = if cfg!(windows) {
                from_windows_code(code)
            } else {
                from_unix_code(code)
            };
            if let Some(known) = known {
                return known;
            }
        }
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            io::ErrorKind::DirectoryNotEmpty => Self::NotEmpty,
            io::ErrorKind::NotADirectory => Self::NotAFolder,
            io::ErrorKind::StorageFull => Self::StorageFull,
            io::ErrorKind::QuotaExceeded => Self::QuotaExceeded,
            io::ErrorKind::ReadOnlyFilesystem => Self::ReadOnlyFilesystem,
            io::ErrorKind::CrossesDevices => Self::CrossesDevices,
            io::ErrorKind::ResourceBusy => Self::InUse,
            io::ErrorKind::FileTooLarge => Self::FileTooLarge,
            io::ErrorKind::InvalidFilename => Self::NameInvalid,
            io::ErrorKind::TooManyLinks => Self::TooManyLinks,
            io::ErrorKind::StaleNetworkFileHandle
            | io::ErrorKind::NetworkDown
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::TimedOut => Self::NetworkLost,
            kind => Self::Io {
                kind,
                message: cause_words(&error),
            },
        }
    }
}

impl Explain for VfsError {
    fn next_step(&self) -> String {
        match self {
            VfsError::Cancelled => String::new(),
            VfsError::NotFound => "Reload the folder.".to_owned(),
            VfsError::PermissionDenied => {
                "Check the permissions, or use another folder.".to_owned()
            }
            VfsError::AlreadyExists => "Choose another name.".to_owned(),
            VfsError::NotEmpty => "Empty it first, or move it to the trash.".to_owned(),
            VfsError::NotAFolder => "Choose a folder.".to_owned(),
            VfsError::Unsupported { .. } => "Copy it to a local folder first.".to_owned(),
            VfsError::NoProvider(_) => "Check the address, or enable the plugin for it.".to_owned(),
            VfsError::Place(inner) => inner.next_step(),
            VfsError::NameNotUnicode => "Rename it with another program first.".to_owned(),
            VfsError::StorageFull => "Free some space, then retry.".to_owned(),
            VfsError::QuotaExceeded => "Free some of your space, then retry.".to_owned(),
            VfsError::ReadOnlyFilesystem => "Choose a drive you can write to.".to_owned(),
            VfsError::CrossesDevices => "Copy it, then remove the original.".to_owned(),
            VfsError::InUse => "Close it in the other program, then retry.".to_owned(),
            VfsError::NetworkLost => "Check the connection, then retry.".to_owned(),
            VfsError::NameInvalid => "Rename it with a shorter, plainer name.".to_owned(),
            VfsError::FileTooLarge => "Choose a drive that takes large files.".to_owned(),
            VfsError::TooManyLinks => "Check where the links point.".to_owned(),
            VfsError::PasswordRequired => "Enter the password and try again.".to_owned(),
            VfsError::WrongPassword => "Check the password and try again.".to_owned(),
            VfsError::Corrupt { .. } => "Get a fresh copy if you can.".to_owned(),
            VfsError::LimitReached { .. } => {
                "Raise the limit in Settings, or unpack it with another tool.".to_owned()
            }
            VfsError::UnsupportedFeature { .. } => {
                "Open it with a program that reads it.".to_owned()
            }
            VfsError::Unsafe { .. } => "Check what it is before you open it.".to_owned(),
            VfsError::TrashRefused { .. } => {
                "Delete it for good from its listing, or move it to a fixed drive first.".to_owned()
            }
            VfsError::Io { .. } => "Try again; if it keeps failing, check the disk.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::Message;
    use diroxide_core::format::within_cells;

    #[test]
    fn io_errors_map_to_words() {
        let missing = VfsError::from(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(missing, VfsError::NotFound);
        let m = Message::from_error("open", "notes.txt", &missing);
        assert_eq!(
            m.text,
            "Could not open notes.txt: it does not exist. Reload the folder."
        );
        let other = VfsError::from(io::Error::other("Disk on fire"));
        assert_eq!(other.to_string(), "disk on fire");
        assert_eq!(
            VfsError::from(io::Error::from(io::ErrorKind::StorageFull)),
            VfsError::StorageFull
        );
        assert_eq!(
            VfsError::from(io::Error::from(io::ErrorKind::TimedOut)),
            VfsError::NetworkLost
        );
    }

    #[test]
    fn classes_choose_the_recovery() {
        assert!(VfsError::NetworkLost.class().retries_on_its_own());
        assert!(VfsError::InUse.class().retry_first());
        assert!(!VfsError::ReadOnlyFilesystem.class().can_retry());
        assert_eq!(
            VfsError::StorageFull.class().scope(),
            ErrorScope::Environment
        );
        assert_eq!(VfsError::NameInvalid.class().scope(), ErrorScope::Rule);
        assert_eq!(VfsError::NotFound.class().scope(), ErrorScope::File);
    }

    #[test]
    fn platform_codes_map_to_their_classes() {
        let raw = |code| VfsError::from(io::Error::from_raw_os_error(code));
        if cfg!(windows) {
            assert_eq!(raw(17), VfsError::CrossesDevices);
            assert_eq!(raw(32), VfsError::InUse);
            assert_eq!(raw(112), VfsError::StorageFull);
            assert_eq!(raw(206), VfsError::NameInvalid);
            assert_eq!(raw(64), VfsError::NetworkLost);
            assert_eq!(raw(183), VfsError::AlreadyExists);
            assert_eq!(raw(145), VfsError::NotEmpty);
        } else {
            // EXDEV is 18 on Linux and macOS.
            assert_eq!(raw(18), VfsError::CrossesDevices);
            assert_eq!(raw(26), VfsError::InUse);
            let eloop = if cfg!(target_vendor = "apple") {
                62
            } else {
                40
            };
            assert_eq!(raw(eloop), VfsError::TooManyLinks);
        }
    }

    #[test]
    fn system_words_lose_the_number_and_the_period() {
        let error = io::Error::other("The device is not ready. (os error 21)");
        assert_eq!(cause_words(&error), "the device is not ready");
        let m = Message::from_error("open", "E:", &VfsError::from(error));
        assert!(!m.text.contains("os error"), "{}", m.text);
    }

    #[test]
    fn the_cause_comes_inside_40_cells() {
        let errors = [
            VfsError::Cancelled,
            VfsError::NotFound,
            VfsError::PermissionDenied,
            VfsError::AlreadyExists,
            VfsError::NotEmpty,
            VfsError::NotAFolder,
            VfsError::unsupported("file", "clone this file here"),
            VfsError::NameNotUnicode,
            VfsError::StorageFull,
            VfsError::QuotaExceeded,
            VfsError::ReadOnlyFilesystem,
            VfsError::CrossesDevices,
            VfsError::InUse,
            VfsError::NetworkLost,
            VfsError::NameInvalid,
            VfsError::FileTooLarge,
            VfsError::TooManyLinks,
            VfsError::PasswordRequired,
            VfsError::WrongPassword,
            VfsError::Unsafe {
                why: "it changed since it was listed".into(),
            },
            VfsError::TrashRefused {
                why: "this drive has no trash".into(),
            },
        ];
        for error in errors {
            let m = Message::from_error("copy", "report.pdf", &error);
            assert!(
                within_cells(&m.text, "Could not copy report.pdf"),
                "{}",
                m.text
            );
            assert!(!m.text.contains("F5"), "{}", m.text);
        }
    }
}
