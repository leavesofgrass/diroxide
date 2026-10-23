//! Every sentence the sites lane says (R24 sections 3.4 to 3.6): sites,
//! secrets, the keyring and `doctor`, kept apart from the provider's
//! lines in [`crate::lines`] and checked the same way. Each carries its
//! meaning in the first 40 cells, words before symbols; the host adds
//! the plugin's name last (answer 90), so none carries it.

/// The state word of a saved site with no connection.
pub const SAVED: &str = "saved";

/// The state word of a site whose connection is open.
pub const CONNECTED: &str = "connected";

/// The state word of a site whose connection was lost.
pub const LOST: &str = "lost";

/// The state word of a host from the ssh configuration, nothing saved.
pub const FROM_CONFIG: &str = "from ssh config";

/// The Sites list's title, its counts first: "Sites, 3 saved, 5 from ssh
/// config".
pub fn sites_title(saved: usize, from_config: usize) -> String {
    format!("Sites, {saved} saved, {from_config} from ssh config")
}

/// The sites file was refused because a line holds a secret.
pub const FILE_HOLDS_SECRET: &str = "Sites file refused: it holds a password.";

/// What to do about it, the field named: "Remove password from build."
pub fn remove_secret_field(key: &str, site: &str) -> String {
    format!("Remove {key} from {site}.")
}

/// Why a password is never in the file.
pub const NO_SECRET_IN_FILES: &str = "diroxide keeps no password in a file.";

/// The sites file could not be read as TOML.
pub const FILE_NOT_TOML: &str = "Sites file not read: it is not TOML.";

/// The sites file could not be read at all.
pub const FILE_UNREADABLE: &str = "Sites file could not be read.";

/// The sites file is larger than any list of sites needs.
pub const FILE_TOO_LARGE: &str = "Sites file not read: it is too large.";

/// The sites file could not be written.
pub const FILE_NOT_SAVED: &str = "Sites file could not be saved.";

/// Sites left out of the list, with the count first: "2 sites skipped:
/// not valid."
pub fn sites_skipped(count: usize) -> String {
    if count == 1 {
        "1 site skipped: not valid.".to_owned()
    } else {
        format!("{count} sites skipped: not valid.")
    }
}

/// A command that needs a site row got none.
pub const NO_SITE_HERE: &str = "No site on this row.";

/// The next step for it.
pub const NO_SITE_HINT: &str = "Open the Sites list and pick a site.";

/// A command that needs a connection found none.
pub fn not_connected(name: &str) -> String {
    format!("Not connected to {name}.")
}

/// A site was saved: "Site saved: build."
pub fn site_saved(name: &str) -> String {
    format!("Site saved: {name}.")
}

/// A site was deleted: "Site deleted: build."
pub fn site_deleted(name: &str) -> String {
    format!("Site deleted: {name}.")
}

/// Nothing was changed: the owner said no or left a question.
pub const NOTHING_CHANGED: &str = "Nothing was changed.";

/// The question before a delete, the site named: "Delete the site build?"
pub fn delete_question(name: &str) -> String {
    format!("Delete the site {name}?")
}

/// What a delete keeps.
pub const DELETE_KEEPS: &str = "Its saved password goes too.";

/// Disconnected by the owner: "Disconnected from build."
pub fn disconnected(name: &str) -> String {
    format!("Disconnected from {name}.")
}

/// A connect the owner started: "Connecting to build.example."
pub fn connecting(host: &str) -> String {
    format!("Connecting to {host}.")
}

/// A connect that worked: "Connected to build.example."
pub fn connected(host: &str) -> String {
    format!("Connected to {host}.")
}

/// A field of the new-site questions was refused, the reason after.
pub fn field_refused(field: &str) -> String {
    format!("Not saved: the {field} was not valid.")
}

// --- Secrets and the keyring (R24 section 3.4; answer 83).

/// The question after a successful login, never before.
pub const REMEMBER_QUESTION: &str = "Remember this password? y or n";

/// Where a remembered password went.
pub const SAVED_IN_KEYRING: &str = "Saved in the system keyring, not a file.";

/// A machine with no key store: nothing is saved there.
pub const NO_KEY_STORE: &str = "No key store here; nothing saved.";

/// What happens then.
pub const KEPT_THIS_SESSION: &str = "Kept for this session only.";

/// The key store refused to save.
pub const KEY_STORE_FAILED: &str = "The key store did not save it.";

/// The question before forgetting a saved password: "Forget the saved
/// password for build?"
pub fn forget_question(name: &str) -> String {
    format!("Forget the saved password for {name}?")
}

/// The answer: "Password forgotten for build."
pub fn forgotten(name: &str) -> String {
    format!("Password forgotten for {name}.")
}

/// Nothing to forget: "No password saved for build."
pub fn nothing_to_forget(name: &str) -> String {
    format!("No password saved for {name}.")
}

/// A stored password the server refused: it is not used again.
pub const STORED_REFUSED: &str = "Saved password refused; asking you.";

/// The secret modes, each as the owner reads it.
pub fn mode_words(mode: &str) -> &'static str {
    match mode {
        "session" => "Remember for this session",
        "keyring" => "Remember in the system keyring",
        _ => "Never remember",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::MAX_LINE_CELLS;

    #[test]
    fn every_line_fits_one_braille_line() {
        for line in [
            sites_title(3, 5),
            sites_title(120, 1500),
            FILE_HOLDS_SECRET.to_owned(),
            remove_secret_field("password", "build"),
            NO_SECRET_IN_FILES.to_owned(),
            FILE_NOT_TOML.to_owned(),
            FILE_UNREADABLE.to_owned(),
            FILE_TOO_LARGE.to_owned(),
            FILE_NOT_SAVED.to_owned(),
            sites_skipped(1),
            sites_skipped(12),
            NO_SITE_HERE.to_owned(),
            NO_SITE_HINT.to_owned(),
            not_connected("build"),
            site_saved("build"),
            site_deleted("build"),
            NOTHING_CHANGED.to_owned(),
            delete_question("build"),
            DELETE_KEEPS.to_owned(),
            disconnected("build"),
            connecting("build.example"),
            connected("build.example"),
            field_refused("host name"),
            REMEMBER_QUESTION.to_owned(),
            SAVED_IN_KEYRING.to_owned(),
            NO_KEY_STORE.to_owned(),
            KEPT_THIS_SESSION.to_owned(),
            KEY_STORE_FAILED.to_owned(),
            forget_question("build"),
            forgotten("build"),
            nothing_to_forget("build"),
            STORED_REFUSED.to_owned(),
            mode_words("never").to_owned(),
            mode_words("session").to_owned(),
            mode_words("keyring").to_owned(),
        ] {
            assert!(line.chars().count() <= MAX_LINE_CELLS, "{line}");
        }
    }

    #[test]
    fn the_golden_lines_of_r24() {
        assert_eq!(sites_title(3, 5), "Sites, 3 saved, 5 from ssh config");
        assert_eq!(REMEMBER_QUESTION, "Remember this password? y or n");
        assert_eq!(SAVED_IN_KEYRING, "Saved in the system keyring, not a file.");
        assert_eq!(NO_KEY_STORE, "No key store here; nothing saved.");
        assert_eq!(connecting("build.example"), "Connecting to build.example.");
        assert_eq!(connected("build.example"), "Connected to build.example.");
    }
}
