//! Every sentence the plugin says, in one place, so each is checked
//! against the 40-cell rule: the meaning first, words before symbols,
//! nothing by color. The host adds the plugin's name last to every line
//! the plugin causes (answer 90), so these lines never carry it.

/// One line of the owner's Braille display, in cells.
pub const MAX_LINE_CELLS: usize = 40;

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

#[cfg(test)]
mod tests {
    use super::*;

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
            (setting_not_openssh(), "It did not answer as OpenSSH."),
        ] {
            assert_eq!(made, expected);
            assert!(made.chars().count() <= MAX_LINE_CELLS, "{made}");
        }
    }
}
