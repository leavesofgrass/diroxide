//! Every sentence the plugin says, in one place, so each is checked
//! against the 40-cell rule: the meaning first, words before symbols,
//! nothing by color. The host adds the plugin's name last to every line
//! the plugin causes (answer 90), so these lines never carry it.

use diroxide_plugins::services::clean_text;

/// One line of the owner's Braille display, in cells.
pub const MAX_LINE_CELLS: usize = 40;

/// Text from outside (ssh's standard error, a server's words) made safe
/// to show: control, format and direction characters and terminal escapes
/// removed, spaces folded, cut to `max` characters. It is never the
/// sentence itself, only words placed after the plugin's own.
pub fn cleaned(text: &str, max: usize) -> String {
    clean_text(text, max)
}

/// A host from the owner's ssh configuration, as a row with nothing saved:
/// "nas, from ssh config".
pub fn config_host_row(name: &str) -> String {
    format!("{name}, from ssh config")
}

/// The next step after "Not connected": "Open the place again to
/// connect."
pub const OPEN_TO_CONNECT: &str = "Open the place again to connect.";

/// The next step while connections are not built: said, never a dead
/// end in silence.
pub const NOT_BUILT_HINT: &str = "SFTP connections come in a later build.";

/// "Not connected to build.example."
pub fn not_connected(host: &str) -> String {
    format!("Not connected to {host}.")
}

/// "Cannot connect to build.example yet."
pub fn not_built_yet(host: &str) -> String {
    format!("Cannot connect to {host} yet.")
}

/// Say Status's line for the ssh in use: "ssh: OpenSSH for Windows 9.5
/// in System32".
pub fn which_ssh(version: &str, origin: &str) -> String {
    format!("ssh: {version} {origin}")
}

/// No OpenSSH anywhere, with the way to add it on this system.
pub fn ssh_not_found(windows: bool) -> Vec<String> {
    let fix = if windows {
        "Add it in Settings, Optional features."
    } else {
        "Install the OpenSSH client package."
    };
    vec!["OpenSSH was not found.".to_owned(), fix.to_owned()]
}

/// The `ssh_program` setting is not a full path.
pub fn setting_not_absolute() -> String {
    "Its path must be a full path.".to_owned()
}

/// The `ssh_program` setting does not answer as OpenSSH.
pub fn setting_not_openssh() -> String {
    "It did not answer as OpenSSH.".to_owned()
}

/// An ssh below 8.4 cannot ask a password with no console window: keys
/// and the agent only (answer 85), with the way to update on Windows.
pub fn too_old_for_passwords(windows: bool) -> Vec<String> {
    let mut out = vec![
        "This ssh is too old for passwords.".to_owned(),
        "Use a key, or update OpenSSH to 8.4.".to_owned(),
    ];
    if windows {
        out.push("winget install Microsoft.OpenSSH.Preview".to_owned());
    }
    out
}

/// "Connection lost: build.example." (R24 section 1.7).
pub fn connection_lost(host: &str) -> String {
    format!("Connection lost: {host}.")
}

/// The next step after a lost connection.
pub const OPEN_TO_RECONNECT: &str = "Open the place again to reconnect.";

/// "Connecting to build.example.", the progress line while ssh starts.
pub fn connecting(host: &str) -> String {
    format!("Connecting to {host}.")
}

/// "Cannot connect to build.example.", when no pattern says more.
pub fn cannot_connect(host: &str) -> String {
    format!("Cannot connect to {host}.")
}

/// "No answer from build.example."
pub fn no_answer(host: &str) -> String {
    format!("No answer from {host}.")
}

/// An entry the server does not have.
pub const NOT_THERE: &str = "It does not exist on the server.";

/// The server refused for want of rights.
pub fn denied() -> String {
    "Permission denied by the server.".to_owned()
}

/// The server does not offer what was asked.
pub fn server_cannot() -> String {
    "The server cannot do this.".to_owned()
}

/// The server did not understand a request.
pub fn not_understood() -> String {
    "The server did not understand.".to_owned()
}

/// "Could not list the folder.", for a plain failure.
pub fn could_not(what: &str) -> String {
    format!("Could not {what}.")
}

/// A cancelled request.
pub const CANCELLED: &str = "Cancelled.";

/// A shell startup file printed text before the protocol began.
pub const SHELL_PRINTED: &str = "The server's shell printed text first.";

/// The way out of [`SHELL_PRINTED`].
pub const SHELL_PRINTED_HINT: &str = "Make its startup file quiet for sftp.";

/// The far end answered, but not in SFTP.
pub const NOT_SFTP: &str = "The server did not speak SFTP.";

/// "2 names skipped: unsafe.", for names a listing could not show.
pub fn names_skipped(count: usize) -> String {
    if count == 1 {
        "1 name skipped: unsafe.".to_owned()
    } else {
        format!("{count} names skipped: unsafe.")
    }
}

/// A folder where a file was meant.
pub const IS_A_FOLDER: &str = "A folder has that name.";

/// A name already taken, and the write must not replace it.
pub const NAME_TAKEN: &str = "The name is taken.";

/// A path whose folder is not a folder.
pub const NOT_A_FOLDER: &str = "Its folder is not a folder.";

/// A second write to one file while the first runs.
pub const WRITE_RUNNING: &str = "Another write to this file is running.";

/// A chunk at the wrong offset.
pub const OUT_OF_ORDER: &str = "A chunk came out of order.";

/// A write session that is no longer open.
pub const WRITE_ENDED: &str = "The write session ended.";

/// The server holds a size other than the bytes sent.
pub const SIZE_DIFFERS: &str = "Size differs after writing.";

/// "The server holds 12 bytes of 20 sent."
pub fn size_held(held: u64, sent: u64) -> String {
    format!("The server holds {held} bytes of {sent} sent.")
}

/// SFTP has no trash (answer 86).
pub const NO_TRASH: &str = "SFTP has no trash.";

/// What a delete over SFTP does instead.
pub const NO_TRASH_HINT: &str = "Delete lists each item, then asks.";

/// A replacing rename on a server without `posix-rename`.
pub const NO_ATOMIC_RENAME: &str = "This server cannot rename over a file.";

/// Links on a server that cannot make them.
pub const NO_LINKS: &str = "This server cannot make links.";

/// Dates SFTP version 3 can carry.
pub const DATES_KEPT: &str = "SFTP keeps dates from 1970 to 2106.";

/// "Permissions now 644.", after a change the server kept.
pub fn permissions_now(mode: u32) -> String {
    format!("Permissions now {mode:o}.")
}

/// "Server kept 600, not 644.", after a change the server did not keep.
pub fn permissions_kept(held: u32, asked: u32) -> String {
    format!("Server kept {held:o}, not {asked:o}.")
}

/// "Permissions sent: 644. Not read back.", when the server says nothing.
pub fn permissions_unknown(asked: u32) -> String {
    format!("Permissions sent: {asked:o}. Not read back.")
}

/// `copy_hint`'s words: both places on one server that copies.
pub const COPY_ON_SERVER: &str = "Copied on the server.";

/// `copy_hint`'s words: the places are on two servers or accounts.
pub const COPY_OTHER_SERVER: &str = "Another server: copied through here.";

/// `copy_hint`'s words: the server lacks `copy-data`.
pub const NO_SERVER_COPY: &str = "This server cannot copy on its own.";

/// `copy_hint`'s words: not connected, so nothing is asked.
pub const COPY_NOT_CONNECTED: &str = "Not connected: copied through here.";

/// A server-side copy of anything but a file.
pub const COPY_FILES_ONLY: &str = "Only files are copied on the server.";

/// "Copying on build.example.", while a server-side copy runs.
pub fn copying_on(host: &str) -> String {
    format!("Copying on {host}.")
}

/// "Home on build.example", the home folder's volume.
pub fn home_on(host: &str) -> String {
    format!("Home on {host}")
}

/// A server without `statvfs@openssh.com`.
pub const NO_FREE_SPACE: &str = "This server does not report free space.";

/// A host key the owner did not accept, or ssh could not ask about.
pub const KEY_NOT_TRUSTED: &str = "Host key not trusted. Not connected.";

/// The next step after [`KEY_NOT_TRUSTED`].
pub const KEY_NOT_TRUSTED_HINT: &str = "Accept the new key only if you know it.";

/// "Sign-in refused by build.example."
pub fn sign_in_refused(host: &str) -> String {
    format!("Sign-in refused by {host}.")
}

/// The next step after a refused sign-in.
pub const SIGN_IN_HINT: &str = "Check the user name, key or password.";

/// "No SFTP service on build.example."
pub fn no_sftp_on(host: &str) -> String {
    format!("No SFTP service on {host}.")
}

/// The next step for a server with no SFTP subsystem.
pub const NO_SFTP_HINT: &str = "Ask for the sftp subsystem to be on.";

/// The next step when the network or the address is in doubt.
pub const CHECK_NETWORK: &str = "Check the address and the network.";

/// "Connection refused by build.example."
pub fn refused_by(host: &str) -> String {
    format!("Connection refused by {host}.")
}

/// The next step after a refused connection.
pub const IS_SSH_RUNNING: &str = "Is its SSH server running on that port?";

/// "Unknown host: build.example."
pub fn unknown_host(host: &str) -> String {
    format!("Unknown host: {host}.")
}

/// The next step for a name that does not resolve.
pub const CHECK_NAME: &str = "Check the name, or your network.";

/// "No route to build.example."
pub fn no_route(host: &str) -> String {
    format!("No route to {host}.")
}

/// The owner's ssh configuration has an error.
pub const BAD_CONFIG: &str = "Your ssh config has an error.";

/// The next step after [`BAD_CONFIG`].
pub const BAD_CONFIG_HINT: &str = "Fix the line ssh names, then try again.";

/// "Connection closed by build.example."
pub fn closed_by(host: &str) -> String {
    format!("Connection closed by {host}.")
}

/// A full disk or a quota, from the server's words.
pub const NO_SPACE: &str = "The server's disk or quota is full.";

/// A file larger than the server takes.
pub const TOO_LARGE: &str = "The file is too large for the server.";

/// A folder that is not empty.
pub const NOT_EMPTY: &str = "The folder is not empty.";

/// What ssh last printed, cleaned and placed last in a hint.
pub fn ssh_said(text: &str) -> Option<String> {
    let last = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| cleaned(line, 160))
        .filter(|line| !line.is_empty())?;
    Some(format!("ssh said: {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_provider_lines_fit_and_put_the_cause_first() {
        for line in [
            connection_lost("build.example"),
            OPEN_TO_RECONNECT.to_owned(),
            connecting("build.example"),
            cannot_connect("build.example"),
            no_answer("build.example"),
            NOT_THERE.to_owned(),
            denied(),
            server_cannot(),
            not_understood(),
            could_not("list the folder"),
            SHELL_PRINTED.to_owned(),
            SHELL_PRINTED_HINT.to_owned(),
            NOT_SFTP.to_owned(),
            names_skipped(1),
            names_skipped(12_000),
            IS_A_FOLDER.to_owned(),
            NAME_TAKEN.to_owned(),
            NOT_A_FOLDER.to_owned(),
            WRITE_RUNNING.to_owned(),
            OUT_OF_ORDER.to_owned(),
            WRITE_ENDED.to_owned(),
            SIZE_DIFFERS.to_owned(),
            NO_TRASH.to_owned(),
            NO_TRASH_HINT.to_owned(),
            NO_ATOMIC_RENAME.to_owned(),
            NO_LINKS.to_owned(),
            DATES_KEPT.to_owned(),
            permissions_now(0o4755),
            permissions_kept(0o4755, 0o2777),
            permissions_unknown(0o4755),
            COPY_ON_SERVER.to_owned(),
            COPY_OTHER_SERVER.to_owned(),
            NO_SERVER_COPY.to_owned(),
            COPY_NOT_CONNECTED.to_owned(),
            COPY_FILES_ONLY.to_owned(),
            copying_on("build.example"),
            home_on("build.example"),
            NO_FREE_SPACE.to_owned(),
            KEY_NOT_TRUSTED.to_owned(),
            KEY_NOT_TRUSTED_HINT.to_owned(),
            sign_in_refused("build.example"),
            SIGN_IN_HINT.to_owned(),
            no_sftp_on("build.example"),
            NO_SFTP_HINT.to_owned(),
            CHECK_NETWORK.to_owned(),
            refused_by("build.example"),
            IS_SSH_RUNNING.to_owned(),
            unknown_host("build.example"),
            CHECK_NAME.to_owned(),
            no_route("build.example"),
            BAD_CONFIG.to_owned(),
            BAD_CONFIG_HINT.to_owned(),
            closed_by("build.example"),
            NO_SPACE.to_owned(),
            TOO_LARGE.to_owned(),
            NOT_EMPTY.to_owned(),
        ] {
            assert!(line.chars().count() <= MAX_LINE_CELLS, "{line}");
        }
        assert_eq!(
            ssh_said("debug\n  Connection reset by peer\u{1B}[0m \n\n").as_deref(),
            Some("ssh said: Connection reset by peer")
        );
        assert_eq!(ssh_said(" \n"), None);
    }

    #[test]
    fn many_line_messages_keep_one_fact_a_line() {
        assert_eq!(
            too_old_for_passwords(true),
            [
                "This ssh is too old for passwords.",
                "Use a key, or update OpenSSH to 8.4.",
                "winget install Microsoft.OpenSSH.Preview",
            ]
        );
        assert_eq!(too_old_for_passwords(false).len(), 2);
        assert_eq!(
            ssh_not_found(false),
            [
                "OpenSSH was not found.",
                "Install the OpenSSH client package."
            ]
        );
        for line in too_old_for_passwords(true)
            .into_iter()
            .chain(ssh_not_found(true))
            .chain(ssh_not_found(false))
        {
            assert!(line.chars().count() <= MAX_LINE_CELLS, "{line}");
        }
    }

    #[test]
    fn the_lines_keep_their_meaning_in_one_braille_line() {
        for (made, expected) in [
            (
                not_connected("build.example"),
                "Not connected to build.example.",
            ),
            (
                not_built_yet("build.example"),
                "Cannot connect to build.example yet.",
            ),
            (
                OPEN_TO_CONNECT.to_owned(),
                "Open the place again to connect.",
            ),
            (
                NOT_BUILT_HINT.to_owned(),
                "SFTP connections come in a later build.",
            ),
            (
                which_ssh("OpenSSH for Windows 9.5", "in System32"),
                "ssh: OpenSSH for Windows 9.5 in System32",
            ),
            (
                which_ssh("OpenSSH 10.3", "from PATH"),
                "ssh: OpenSSH 10.3 from PATH",
            ),
            (setting_not_absolute(), "Its path must be a full path."),
            (config_host_row("nas"), "nas, from ssh config"),
            (
                cleaned("Bad \u{1B}[31mowner\u{202E} on file", 40),
                "Bad owner on file",
            ),
            (setting_not_openssh(), "It did not answer as OpenSSH."),
        ] {
            assert_eq!(made, expected);
            assert!(made.chars().count() <= MAX_LINE_CELLS, "{made}");
        }
    }
}
