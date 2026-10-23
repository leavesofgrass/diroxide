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

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert_eq!(made, expected);
            assert!(made.chars().count() <= MAX_LINE_CELLS, "{made}");
        }
    }
}
