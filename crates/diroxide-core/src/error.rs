//! Errors of the core model, and [`Explain`], which every error type in the
//! workspace implements so the app can say "Could not VERB OBJECT: CAUSE.
//! NEXT STEP." the same way everywhere.

use std::fmt;

use thiserror::Error;

/// An error that can say what to do next. Its [`Display`](fmt::Display) is
/// the cause, in lowercase words with no final period ("the drive letter has
/// no slash after it"); [`Explain::next_step`] is a short sentence of advice,
/// or empty when there is none.
pub trait Explain: fmt::Display {
    /// What the user can do about it, as one sentence, or an empty string.
    fn next_step(&self) -> String;
}

/// Why text could not be read as a [`Place`](crate::Place).
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlaceError {
    /// Nothing was typed.
    #[error("the place is empty")]
    Empty,
    /// A relative path where a full one is needed.
    #[error("\"{0}\" is not a full path")]
    NotAbsolute(String),
    /// A drive letter with no slash, such as `C:` or `C:docs`, which Windows
    /// reads as the current folder on that drive.
    #[error("\"{0}\" has no slash after the drive letter")]
    AmbiguousDrive(String),
    /// A Windows path with a leading slash and no drive letter, which
    /// Windows reads against the current drive.
    #[error("\"{0}\" has no drive letter")]
    NoDrive(String),
    /// A Windows device path such as `\\.\PhysicalDrive0`.
    #[error("\"{0}\" is a device path")]
    DevicePath(String),
    /// An archive member path that climbs out of its archive.
    #[error("the member path \"{0}\" leaves its archive")]
    MemberEscapes(String),
    /// A name that cannot be one entry: empty, `.`, `..`, or with a
    /// separator in it.
    #[error("\"{0}\" is not a single name")]
    BadName(String),
    /// A remote address that could not be read, such as a missing host.
    #[error("the address \"{0}\" could not be read")]
    BadRemote(String),
    /// A name inside an archive with a `!` after an archive extension,
    /// which would read back as a nested archive.
    #[error("\"{0}\" contains an archive marker")]
    ArchiveMarker(String),
}

impl Explain for PlaceError {
    fn next_step(&self) -> String {
        match self {
            PlaceError::Empty => "Type a path, such as D:\\ or /home.".to_owned(),
            PlaceError::NotAbsolute(_) => {
                "Start with a drive and a slash, a slash, or a scheme such as sftp://.".to_owned()
            }
            PlaceError::AmbiguousDrive(text) => {
                let drive: String = text.chars().take(2).collect();
                format!("Write {drive}\\ for the top folder of the drive.")
            }
            PlaceError::NoDrive(_) => "Start with a drive letter, such as D:\\.".to_owned(),
            PlaceError::DevicePath(_) => "Open a folder instead of a device.".to_owned(),
            PlaceError::MemberEscapes(_) => {
                "The archive may be hostile; open it with care.".to_owned()
            }
            PlaceError::BadName(_) => "Use one name without slashes.".to_owned(),
            PlaceError::BadRemote(_) => "Write it as sftp://host/path.".to_owned(),
            PlaceError::ArchiveMarker(_) => {
                "Unpack the archive to reach this member by its name.".to_owned()
            }
        }
    }
}

/// An error of the core model.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A place could not be read.
    #[error(transparent)]
    Place(#[from] PlaceError),
    /// A command id that is not lowercase words joined by dots.
    #[error("\"{0}\" is not a command id")]
    BadCommandId(String),
    /// A key or chord that could not be read, such as "ctrl+".
    #[error("\"{0}\" is not a key")]
    BadKey(String),
}

impl Explain for CoreError {
    fn next_step(&self) -> String {
        match self {
            CoreError::Place(inner) => inner.next_step(),
            CoreError::BadCommandId(_) => {
                "Write a command id as lowercase words joined by dots, such as file.copy."
                    .to_owned()
            }
            CoreError::BadKey(_) => {
                "Write a key as modifiers and a key joined by plus signs, such as ctrl+alt+t."
                    .to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_the_cause_and_a_next_step() {
        let e = PlaceError::AmbiguousDrive("C:".to_owned());
        assert_eq!(e.to_string(), "\"C:\" has no slash after the drive letter");
        assert_eq!(e.next_step(), "Write C:\\ for the top folder of the drive.");
        let wrapped = CoreError::from(e.clone());
        assert_eq!(wrapped.to_string(), e.to_string());
    }
}
