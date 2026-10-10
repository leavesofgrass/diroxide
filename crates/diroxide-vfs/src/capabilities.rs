//! What a provider can do at a place.

/// What a provider can do at a place. Commands that need a missing
/// capability are listed as unavailable, with the reason in words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// Entries can be created, changed and removed.
    pub writable: bool,
    /// Entries can be moved to a trash and restored.
    pub trash: bool,
    /// Symbolic links can be made and read.
    pub links: bool,
    /// Permissions or attributes can be changed.
    pub attributes: bool,
    /// Names that differ only by case are different entries.
    pub case_sensitive: bool,
    /// The longest name, in UTF-16 units on Windows and bytes elsewhere.
    pub max_name_length: Option<usize>,
    /// Windows device names (CON, NUL, COM1 and the rest) are reserved.
    pub reserved_names: bool,
    /// Changes can be watched.
    pub watch: bool,
}

impl Capabilities {
    /// Nothing but reading: the capabilities of a stub or an unknown place.
    pub const fn read_only() -> Self {
        Self {
            writable: false,
            trash: false,
            links: false,
            attributes: false,
            case_sensitive: true,
            max_name_length: None,
            reserved_names: false,
            watch: false,
        }
    }

    /// The words for what cannot be done here, such as "read only, no
    /// trash", or an empty string when everything can.
    pub fn limits_in_words(&self) -> String {
        let mut words = Vec::new();
        if !self.writable {
            words.push("read only");
        }
        if self.writable && !self.trash {
            words.push("no trash");
        }
        words.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_said_in_words() {
        assert_eq!(Capabilities::read_only().limits_in_words(), "read only");
        let local = Capabilities {
            writable: true,
            ..Capabilities::read_only()
        };
        assert_eq!(local.limits_in_words(), "no trash");
    }
}
