//! Secrets: passwords and passphrases, kept out of logs, debug output and
//! anything serialized, and wiped from memory when dropped (ADR-0009,
//! research/encryption.md).
//!
//! A [`Secret`] has no `Serialize`, so no job request, command, message or
//! saved file can hold one; its `Debug` says only "Secret(hidden)"; its
//! one way out is [`Secret::expose`], for the library that needs the
//! text. A [`SecretRequest`] says what is asked for, and carries no secret,
//! so it can travel anywhere.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::format::BRAILLE_CELLS;

/// A password or passphrase. Wiped when dropped; a clone is wiped too.
#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wraps a secret's text, taking it so no other copy is left.
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The text, for the library that needs it (an archive reader, an SSH
    /// client). Never for a message, a log or a file.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether nothing was typed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many characters were typed: what the masked prompt shows as
    /// asterisks, and all it ever shows.
    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }

    /// Adds a typed character. When the buffer must grow, the old one is
    /// wiped as it is dropped, so no copy is left behind.
    pub fn push(&mut self, c: char) {
        if self.0.capacity() - self.0.len() < c.len_utf8() {
            let mut grown = Zeroizing::new(String::with_capacity((self.0.len() + 4) * 2));
            grown.push_str(&self.0);
            self.0 = grown;
        }
        self.0.push(c);
    }

    /// Removes the last typed character, as Backspace does.
    pub fn pop(&mut self) -> Option<char> {
        self.0.pop()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

/// Two secrets are equal when their text is; for the dialog model's tests.
impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for Secret {}

/// What kind of secret is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SecretKind {
    /// The password of an archive (zip AES, 7z).
    ArchivePassword,
    /// The passphrase of an encrypted file (age, OpenPGP).
    FilePassphrase,
    /// The passphrase of a private key (SSH).
    KeyPassphrase,
    /// A password for a host (SFTP).
    HostPassword,
    /// A question a server asks in its own words (keyboard-interactive).
    ServerPrompt,
}

impl SecretKind {
    /// The word for the secret: "Password" or "Passphrase".
    pub const fn word(self) -> &'static str {
        match self {
            SecretKind::FilePassphrase | SecretKind::KeyPassphrase => "Passphrase",
            _ => "Password",
        }
    }
}

/// A request for a secret: what for, whose, which try. It holds no secret
/// and may be logged, serialized or shown.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretRequest {
    /// What kind of secret.
    pub kind: SecretKind,
    /// What it is for: an archive's name, a key file's name, a host; for a
    /// server prompt, the server's own words.
    pub subject: String,
    /// The user name, for a host password.
    pub user: Option<String>,
    /// Which try this is, from 1.
    pub attempt: u32,
    /// How many tries this operation allows (three, R9).
    pub attempts_allowed: u32,
    /// Whether the answer may be shown as it is typed: false for every
    /// secret; true only for a server question that is not one.
    pub echo: bool,
    /// Whether this is a new secret, typed twice to confirm.
    pub new_secret: bool,
}

impl SecretRequest {
    /// The tries an operation allows by default.
    pub const ATTEMPTS: u32 = 3;

    /// A first request for a secret of a kind, for a subject.
    pub fn new(kind: SecretKind, subject: impl Into<String>) -> Self {
        Self {
            kind,
            subject: subject.into(),
            user: None,
            attempt: 1,
            attempts_allowed: Self::ATTEMPTS,
            echo: false,
            new_secret: false,
        }
    }

    /// The name the prompt says, shortened in the middle with its
    /// extension kept, so "Password for" and the name's ends fit 40 cells.
    fn short_subject(&self, room: usize) -> String {
        let subject = match (&self.user, self.kind) {
            (Some(user), SecretKind::HostPassword) => format!("{user}@{}", self.subject),
            _ => self.subject.clone(),
        };
        let count = subject.chars().count();
        if count <= room || room < 8 {
            return subject;
        }
        let (stem, extension) = crate::names::split_extension(&subject);
        let keep_end = extension.chars().count().min(room / 2) + 2;
        let keep_start = room.saturating_sub(keep_end + 3);
        let start: String = stem.chars().take(keep_start).collect();
        let end: String = subject.chars().skip(count - keep_end).collect();
        format!("{start}...{end}")
    }

    /// The dialog's name, said when it opens: "Password for course.zip",
    /// "New password for Reports.7z".
    pub fn title(&self) -> String {
        if self.kind == SecretKind::ServerPrompt {
            return self.subject.clone();
        }
        let word = self.kind.word();
        let lead = if self.new_secret {
            format!("New {} for", word.to_lowercase())
        } else {
            format!("{word} for")
        };
        let room = BRAILLE_CELLS.saturating_sub(lead.chars().count() + 12);
        format!("{lead} {}", self.short_subject(room))
    }

    /// The terminal prompt, the meaning first and inside 40 cells where
    /// the name allows: "Password for course.zip (hidden): ".
    pub fn prompt_line(&self) -> String {
        if self.echo {
            format!("{}: ", self.title())
        } else {
            format!("{} (hidden): ", self.title())
        }
    }

    /// The same request, one try later.
    pub fn next_attempt(&self) -> Self {
        Self {
            attempt: self.attempt + 1,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::within_cells;

    const CANARY: &str = "canary-7f3a-not-a-password";

    #[test]
    fn a_secret_never_shows_in_debug_output() {
        let secret = Secret::new(CANARY.to_owned());
        assert_eq!(format!("{secret:?}"), "Secret(hidden)");
        assert!(!format!("{:?}", Some(secret.clone())).contains(CANARY));
        assert_eq!(secret.expose(), CANARY);
        assert_eq!(secret.char_count(), CANARY.len());
    }

    #[test]
    fn typing_and_backspace_change_only_the_buffer() {
        let mut secret = Secret::default();
        for c in "pässwörd".chars() {
            secret.push(c);
        }
        assert_eq!(secret.expose(), "pässwörd");
        assert_eq!(secret.char_count(), 8);
        assert_eq!(secret.pop(), Some('d'));
        assert_eq!(secret.expose(), "pässwör");
        assert!(Secret::default().pop().is_none());
    }

    #[test]
    fn a_request_holds_no_secret_and_reads_well() {
        let request = SecretRequest::new(SecretKind::ArchivePassword, "course.zip");
        assert_eq!(request.prompt_line(), "Password for course.zip (hidden): ");
        assert_eq!(request.title(), "Password for course.zip");
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains(CANARY));
        let new = SecretRequest {
            new_secret: true,
            ..SecretRequest::new(SecretKind::ArchivePassword, "Reports.7z")
        };
        assert_eq!(new.prompt_line(), "New password for Reports.7z (hidden): ");
        assert_eq!(request.next_attempt().attempt, 2);
    }

    #[test]
    fn a_long_name_is_shortened_in_the_middle() {
        let request = SecretRequest::new(
            SecretKind::ArchivePassword,
            "the-quarterly-financial-report-for-the-board-2026.zip",
        );
        let line = request.prompt_line();
        assert!(within_cells(&line, "Password for the-"));
        assert!(line.contains("...") && line.contains(".zip (hidden)"));
        assert!(line.chars().count() <= 40, "{line}");
        let host = SecretRequest {
            user: Some("ada".into()),
            ..SecretRequest::new(SecretKind::HostPassword, "example.org")
        };
        assert_eq!(host.title(), "Password for ada@example.org");
    }
}
