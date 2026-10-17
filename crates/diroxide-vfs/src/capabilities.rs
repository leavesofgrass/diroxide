//! What a provider can do at a place.

/// Whether something can be done, and with what rights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Can {
    /// It cannot be done here.
    #[default]
    No,
    /// Only an administrator (root, an elevated process) can do it.
    WithAdministrator,
    /// The user can do it.
    Yes,
}

impl Can {
    /// Whether anyone can do it here.
    pub const fn is_possible(self) -> bool {
        !matches!(self, Can::No)
    }
}

/// What can be read and changed about access, per place (research
/// permissions.md 9.3): a FAT stick, an SMB share and the home disk all
/// differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AccessCapabilities {
    /// Unix permission bits, or SFTP permissions.
    pub mode: Can,
    /// Windows attributes; macOS hidden and locked.
    pub os_attributes: Can,
    /// The owner.
    pub owner: Can,
    /// The group.
    pub group: Can,
    /// The modified and accessed times.
    pub times: Can,
    /// The created time (Windows and macOS; Linux cannot set it).
    pub created_time: Can,
    /// Access lists.
    pub acl: Can,
    /// Inode flags (chattr, chflags).
    pub inode_flags: Can,
    /// Whether security labels can be read.
    pub labels: bool,
}

impl AccessCapabilities {
    /// Whether any of the access facts can be changed.
    pub fn any(&self) -> bool {
        [
            self.mode,
            self.os_attributes,
            self.owner,
            self.group,
            self.times,
            self.created_time,
            self.acl,
            self.inode_flags,
        ]
        .iter()
        .any(|can| can.is_possible())
    }

    /// Whether every possible change needs an administrator.
    pub fn administrator_only(&self) -> bool {
        let all = [
            self.mode,
            self.os_attributes,
            self.owner,
            self.group,
            self.times,
            self.created_time,
            self.acl,
            self.inode_flags,
        ];
        all.contains(&Can::WithAdministrator) && !all.contains(&Can::Yes)
    }
}

/// What a provider can do at a place. Commands that need a missing
/// capability are listed as unavailable, with the reason in words. Read
/// per place, never per operating system: a removable drive has no trash
/// on Windows, a FAT stick stores no permissions.
///
/// The struct is `non_exhaustive`, so later alphas add facts without
/// breaking providers: start from [`Capabilities::read_only`] and set the
/// fields that differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Capabilities {
    /// Entries can be created, changed and removed.
    pub writable: bool,
    /// Entries can be moved to a trash (behind the trash gate on Windows).
    pub trash: bool,
    /// Trashed entries can be restored to their places.
    pub restore: bool,
    /// Symbolic links can be read and made.
    pub links: bool,
    /// Permissions or attributes can be changed: any of [`Self::access`].
    pub attributes: bool,
    /// What about access can be read and changed.
    pub access: AccessCapabilities,
    /// Names that differ only by case are different entries.
    pub case_sensitive: bool,
    /// Names that differ only by Unicode normalization (a precomposed
    /// `é` and `e` with a combining accent) are the same entry, as on
    /// APFS and NTFS lookups.
    pub normalization_insensitive: bool,
    /// The longest name, in UTF-16 units on Windows and bytes elsewhere.
    pub max_name_length: Option<usize>,
    /// Windows device names (CON, NUL, COM1 and the rest) are reserved,
    /// and the other Windows name rules apply.
    pub reserved_names: bool,
    /// Changes can be watched.
    pub watch: bool,
    /// Files can be read with random access ([`crate::Provider::open_seekable`])
    /// without a spool.
    pub random_read: bool,
    /// The provider can clone a file or copy it on the server
    /// ([`crate::Provider::copy_within`]), a fast path tried before the
    /// destination is touched. Distinct from `writable`: a place may take
    /// new files and still have no clone.
    pub clone_copy: bool,
    /// A file or link can be renamed over another in one step
    /// ([`crate::Provider::rename_replacing`]), so the name is never
    /// empty: what makes changing a link's target atomic (W2c's request
    /// 2). A folder is never replaced.
    pub replace_rename: bool,
}

impl Capabilities {
    /// Nothing but reading: the capabilities of a stub or an unknown place.
    pub const fn read_only() -> Self {
        Self {
            writable: false,
            trash: false,
            restore: false,
            links: false,
            attributes: false,
            access: AccessCapabilities {
                mode: Can::No,
                os_attributes: Can::No,
                owner: Can::No,
                group: Can::No,
                times: Can::No,
                created_time: Can::No,
                acl: Can::No,
                inode_flags: Can::No,
                labels: false,
            },
            case_sensitive: true,
            normalization_insensitive: false,
            max_name_length: None,
            reserved_names: false,
            watch: false,
            random_read: false,
            clone_copy: false,
            replace_rename: false,
        }
    }

    /// The words for what cannot be done here, such as "read only, no
    /// trash", "no permissions here" or "needs an administrator", or an
    /// empty string when everything can.
    pub fn limits_in_words(&self) -> String {
        let mut words = Vec::new();
        if !self.writable {
            words.push("read only");
        }
        if self.writable && !self.trash {
            words.push("no trash");
        }
        if self.writable && !self.access.any() {
            words.push("no permissions here");
        } else if self.access.administrator_only() {
            words.push("needs an administrator");
        }
        words.join(", ")
    }

    /// Whether links can be made here, in words, the fact first: "Links
    /// can be made here", "Cannot link here: this place holds no links".
    /// On Windows a symbolic link may also need Developer Mode or an
    /// administrator; that is said when the link is made, from the
    /// system's own answer, since only the attempt tells.
    pub fn links_in_words(&self) -> &'static str {
        match (self.links, self.writable) {
            (false, _) => "Cannot link here: this place holds no links",
            (true, false) => "Cannot link here: the place is read only",
            (true, true) if cfg!(windows) => {
                "Links can be made here; symbolic links may need Developer Mode"
            }
            (true, true) => "Links can be made here",
        }
    }

    /// Whether a new link can be made here: links held and the place
    /// writable.
    pub const fn can_link(&self) -> bool {
        self.links && self.writable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn links_are_said_in_words() {
        let mut caps = Capabilities::read_only();
        assert!(!caps.can_link());
        assert_eq!(
            caps.links_in_words(),
            "Cannot link here: this place holds no links"
        );
        caps.links = true;
        assert_eq!(
            caps.links_in_words(),
            "Cannot link here: the place is read only"
        );
        caps.writable = true;
        assert!(caps.can_link());
        assert!(caps.links_in_words().starts_with("Links can be made here"));
        for words in [
            caps.links_in_words(),
            Capabilities::read_only().links_in_words(),
        ] {
            assert!(
                within_cells(words, "Links can be made here")
                    || within_cells(words, "Cannot link here"),
                "{words}"
            );
        }
    }

    #[test]
    fn limits_are_said_in_words() {
        assert_eq!(Capabilities::read_only().limits_in_words(), "read only");
        let mut local = Capabilities::read_only();
        local.writable = true;
        assert_eq!(local.limits_in_words(), "no trash, no permissions here");
        local.trash = true;
        local.access.mode = Can::Yes;
        assert_eq!(local.limits_in_words(), "");
        local.access.mode = Can::WithAdministrator;
        assert_eq!(local.limits_in_words(), "needs an administrator");
    }
}
