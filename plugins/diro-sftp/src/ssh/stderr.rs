//! The standard-error classifier (R24 sections 1.6, 5.4; R25 commit 63):
//! when ssh ends before or during a session, the last of its standard
//! error decides one sentence, the cause first, in 40 cells with a usual
//! host name. Patterns only decide the sentence; ssh's and the server's
//! words never are the sentence. They may follow, cleaned and cut, as the
//! last line of a hint.
//!
//! A changed host key is read first and refused by S-A's
//! [`host_key_refusal`]; the rest follow in the order of the table.

use diroxide_plugins::protocol::{ErrorKind, RpcError};

use crate::askpass::dialogs::host_key_refusal;
use crate::lines;

/// Why ssh ended, as far as its words tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The host's key changed: refused, never accepted.
    ChangedKey,
    /// The host's key was not trusted: the owner said no, or ssh could
    /// not ask.
    KeyNotTrusted,
    /// The server refused the sign-in.
    SignInRefused,
    /// The server has no SFTP subsystem.
    NoSubsystem,
    /// Nothing answered in time.
    TimedOut,
    /// Nothing listens on that port.
    ConnectionRefused,
    /// The name does not resolve.
    UnknownHost,
    /// The network cannot reach the host.
    NoRoute,
    /// The owner's ssh configuration has an error.
    BadConfig,
    /// The server closed the connection.
    ClosedByServer,
    /// Nothing the table knows.
    Unknown,
}

/// The table: words ssh prints, matched without regard to case, and the
/// cause they mean, in the order they are tried.
pub const TABLE: &[(&str, Cause)] = &[
    ("remote host identification has changed", Cause::ChangedKey),
    ("host key verification failed", Cause::KeyNotTrusted),
    ("no matching host key type found", Cause::KeyNotTrusted),
    ("permission denied", Cause::SignInRefused),
    ("too many authentication failures", Cause::SignInRefused),
    ("no more authentication methods", Cause::SignInRefused),
    ("authentication failed", Cause::SignInRefused),
    ("subsystem request failed", Cause::NoSubsystem),
    ("connection timed out", Cause::TimedOut),
    ("operation timed out", Cause::TimedOut),
    ("timed out during banner exchange", Cause::TimedOut),
    ("connection refused", Cause::ConnectionRefused),
    ("could not resolve hostname", Cause::UnknownHost),
    ("name or service not known", Cause::UnknownHost),
    ("nodename nor servname provided", Cause::UnknownHost),
    ("no such host is known", Cause::UnknownHost),
    ("temporary failure in name resolution", Cause::UnknownHost),
    ("no route to host", Cause::NoRoute),
    ("network is unreachable", Cause::NoRoute),
    ("network is down", Cause::NoRoute),
    ("bad configuration option", Cause::BadConfig),
    ("bad owner or permissions", Cause::BadConfig),
    ("connection closed by", Cause::ClosedByServer),
    ("connection reset by", Cause::ClosedByServer),
    ("kex_exchange_identification", Cause::ClosedByServer),
    ("closed by remote host", Cause::ClosedByServer),
    ("broken pipe", Cause::ClosedByServer),
];

/// The cause ssh's standard error tells.
pub fn classify(stderr: &str) -> Cause {
    let text = stderr.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|(words, _)| text.contains(words))
        .map_or(Cause::Unknown, |(_, cause)| *cause)
}

/// The error for a connection that ssh ended: one sentence, the cause
/// first; the hint the next step, then what ssh said, cleaned, last.
pub fn refusal(stderr: &str, host: &str, port: Option<u16>) -> RpcError {
    if let Some(changed) = host_key_refusal(stderr, host, port) {
        return changed;
    }
    let (kind, sentence, next) = match classify(stderr) {
        Cause::ChangedKey | Cause::KeyNotTrusted => (
            ErrorKind::Untrusted,
            lines::KEY_NOT_TRUSTED.to_owned(),
            lines::KEY_NOT_TRUSTED_HINT,
        ),
        Cause::SignInRefused => (
            ErrorKind::AuthRequired,
            lines::sign_in_refused(host),
            lines::SIGN_IN_HINT,
        ),
        Cause::NoSubsystem => (
            ErrorKind::Unsupported,
            lines::no_sftp_on(host),
            lines::NO_SFTP_HINT,
        ),
        Cause::TimedOut => (
            ErrorKind::TimedOut,
            lines::no_answer(host),
            lines::CHECK_NETWORK,
        ),
        Cause::ConnectionRefused => (
            ErrorKind::Unavailable,
            lines::refused_by(host),
            lines::IS_SSH_RUNNING,
        ),
        Cause::UnknownHost => (
            ErrorKind::Unavailable,
            lines::unknown_host(host),
            lines::CHECK_NAME,
        ),
        Cause::NoRoute => (
            ErrorKind::Unavailable,
            lines::no_route(host),
            lines::CHECK_NETWORK,
        ),
        Cause::BadConfig => (
            ErrorKind::Unavailable,
            lines::BAD_CONFIG.to_owned(),
            lines::BAD_CONFIG_HINT,
        ),
        Cause::ClosedByServer => (
            ErrorKind::Unavailable,
            lines::closed_by(host),
            lines::OPEN_TO_RECONNECT,
        ),
        Cause::Unknown => (
            ErrorKind::Unavailable,
            lines::cannot_connect(host),
            lines::CHECK_NETWORK,
        ),
    };
    let hint = match lines::ssh_said(stderr) {
        Some(said) => format!("{next}\n{said}"),
        None => next.to_owned(),
    };
    RpcError::with_hint(kind, sentence, hint)
}

/// The reason said when a connection is lost: who closed it, when ssh's
/// words tell, otherwise that it was lost.
pub fn lost_reason(stderr: &str, host: &str) -> String {
    match classify(stderr) {
        Cause::ClosedByServer => lines::closed_by(host),
        _ => lines::connection_lost(host),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_tells_the_causes_apart() {
        for (text, cause) in [
            (
                "ada@build.example: Permission denied (publickey,password).",
                Cause::SignInRefused,
            ),
            (
                "Received disconnect from 10.0.0.7 port 22:2: Too many authentication failures",
                Cause::SignInRefused,
            ),
            ("subsystem request failed on channel 0", Cause::NoSubsystem),
            (
                "ssh: connect to host build.example port 22: Connection timed out",
                Cause::TimedOut,
            ),
            (
                "Connection timed out during banner exchange",
                Cause::TimedOut,
            ),
            (
                "ssh: connect to host build.example port 22: Connection refused",
                Cause::ConnectionRefused,
            ),
            (
                "ssh: Could not resolve hostname nas: No such host is known. ",
                Cause::UnknownHost,
            ),
            (
                "ssh: connect to host 10.0.0.7 port 22: Network is unreachable",
                Cause::NoRoute,
            ),
            (
                "C:\\Users\\ada/.ssh/config: line 3: Bad configuration option: colour",
                Cause::BadConfig,
            ),
            (
                "Connection closed by 10.0.0.7 port 22",
                Cause::ClosedByServer,
            ),
            (
                "kex_exchange_identification: read: Connection reset by peer",
                Cause::ClosedByServer,
            ),
            ("Host key verification failed.", Cause::KeyNotTrusted),
            ("something new", Cause::Unknown),
        ] {
            assert_eq!(classify(text), cause, "{text}");
        }
    }

    #[test]
    fn every_sentence_puts_the_cause_first_in_40_cells() {
        for (_, cause) in TABLE {
            let text = TABLE
                .iter()
                .find(|(_, c)| c == cause)
                .map(|(w, _)| (*w).to_owned())
                .unwrap_or_default();
            let error = refusal(&text, "build.example", None);
            assert!(error.message.chars().count() <= 40, "{}", error.message);
            for line in error.hint().unwrap_or_default().lines().take(1) {
                assert!(line.chars().count() <= 40, "{line}");
            }
        }
        let unknown = refusal("", "build.example", None);
        assert_eq!(unknown.message, "Cannot connect to build.example.");
        assert_eq!(
            lost_reason("Connection closed by 10.0.0.7 port 22", "build.example"),
            "Connection closed by build.example."
        );
        assert_eq!(
            lost_reason(
                "Timeout, server build.example not responding.",
                "build.example"
            ),
            "Connection lost: build.example."
        );
    }
}
