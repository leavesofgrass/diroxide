//! What a site holds (R24 section 3.2; answers 83 and 94): where to
//! connect, how, where to start, and which of the three secret modes it
//! uses. **No secret is ever part of a site**: the mode says where a
//! password may live (nowhere, this session's memory, or the system
//! keyring), never the password.
//!
//! Every field that reaches ssh is checked by the same rules as a typed
//! address ([`crate::ssh::target`]): a host or alias of plain characters
//! that never starts with a dash, a plain user name, a port from 1 to
//! 65535. A site that fails a check is not offered; it is counted and
//! said.

use std::fmt;

use crate::lines;
use crate::plugin::SCHEME;
use crate::ssh::options::Remote;
use crate::ssh::target::{Target, TargetError, check_host, check_user};

/// The longest site id, in characters.
pub const MAX_ID_CHARS: usize = 32;

/// The longest spoken name kept, in characters.
pub const MAX_NAME_CHARS: usize = 80;

/// The longest note kept, in characters.
pub const MAX_NOTE_CHARS: usize = 500;

/// The most bookmarks one site keeps.
pub const MAX_BOOKMARKS: usize = 100;

/// Where a site's password may live (answer 83), safest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SecretMode {
    /// Nowhere (the default): ssh asks each time, or the agent answers.
    #[default]
    Never,
    /// In the plugin's memory while it runs, after the owner typed it.
    Session,
    /// In the system keyring, after the owner said yes to "Remember this
    /// password?" following a successful login.
    Keyring,
}

impl SecretMode {
    /// The word in the sites file.
    pub const fn word(self) -> &'static str {
        match self {
            SecretMode::Never => "never",
            SecretMode::Session => "session",
            SecretMode::Keyring => "keyring",
        }
    }

    /// The mode for its word; `None` for anything else.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "never" => Some(SecretMode::Never),
            "session" => Some(SecretMode::Session),
            "keyring" => Some(SecretMode::Keyring),
            _ => None,
        }
    }
}

/// Why a site was refused, as the owner hears it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SiteError {
    /// The id is missing or holds an odd character.
    #[error("The site's id must be plain letters and digits.")]
    Id,
    /// The name is empty once cleaned.
    #[error("The site has no name.")]
    Name,
    /// Neither a host nor an alias, or both.
    #[error("A site names a host or an ssh alias, one of the two.")]
    HostOrAlias,
    /// The host, user or port failed the address rules.
    #[error("{0}")]
    Target(#[from] TargetError),
    /// A start folder on the server that is not a full path.
    #[error("The start folder must be a full path on the server.")]
    StartRemote,
    /// An SFTP server program that is not a plain full path.
    #[error("The SFTP server program must be a full path of plain characters.")]
    Program,
    /// A key file named by an empty path.
    #[error("The key file's path is empty.")]
    KeyFile,
    /// A secret mode that is not one of the three words.
    #[error("The secret mode must be never, session or keyring.")]
    SecretMode,
}

/// What a site changes in how ssh is started for it: its key file, the
/// agent, and the far end's SFTP server program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteOptions {
    /// A key file, given to ssh as `-i` with `IdentitiesOnly=yes`.
    pub key_file: Option<std::path::PathBuf>,
    /// Whether an unlocked key is handed to the agent.
    pub add_key_to_agent: bool,
    /// The far end's SFTP server program, checked.
    pub sftp_server_program: Option<String>,
}

/// One saved site.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Site {
    /// Its id: random, plain, never spoken.
    pub id: String,
    /// Its spoken name, meaning first.
    pub name: String,
    /// Its folder of sites, such as `Work/Servers`; empty for none.
    pub group: String,
    /// The host to connect to, when the site names one.
    pub host: Option<String>,
    /// The ssh alias it names instead: ssh supplies the rest.
    pub alias: Option<String>,
    /// The port, when not ssh's choice.
    pub port: Option<u16>,
    /// The user, when not ssh's choice.
    pub user: Option<String>,
    /// A key file, which becomes `-i` with `IdentitiesOnly=yes`.
    pub key_file: Option<String>,
    /// The folder on the server the active panel opens.
    pub start_remote: Option<String>,
    /// The local folder the other panel opens.
    pub start_local: Option<String>,
    /// Folders on the server to go to quickly.
    pub bookmarks: Vec<String>,
    /// Where its password may live.
    pub secret: SecretMode,
    /// Whether an unlocked key is handed to the agent.
    pub add_key_to_agent: bool,
    /// The far end's SFTP server program, for a server with no subsystem.
    pub sftp_server_program: Option<String>,
    /// The owner's note.
    pub note: String,
}

impl fmt::Debug for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Site")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("group", &self.group)
            .field("host", &self.host)
            .field("alias", &self.alias)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}

/// Whether an id holds only plain characters.
pub fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with(['-', '.'])
}

/// A new random id: eight hexadecimal digits from the system's random
/// source.
pub fn new_id() -> String {
    crate::askpass::proof::random()
        .map(|bytes| bytes.iter().take(4).map(|b| format!("{b:02x}")).collect())
        .unwrap_or_else(|| format!("{:08x}", std::process::id()))
}

/// A group path cleaned: each part trimmed and cleaned, empty parts left
/// out, joined by `/`.
pub fn clean_group(group: &str) -> String {
    group
        .split('/')
        .map(|part| lines::cleaned(part, MAX_NAME_CHARS))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

impl Site {
    /// A site with an id, a name and a host, every other field its
    /// default.
    pub fn new(id: &str, name: &str, host: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            host: Some(host.to_owned()),
            ..Self::default()
        }
    }

    /// Checks every field ssh or a path will see, and cleans the spoken
    /// ones.
    pub fn checked(mut self) -> Result<Self, SiteError> {
        if !plain_id(&self.id) {
            return Err(SiteError::Id);
        }
        self.name = lines::cleaned(&self.name, MAX_NAME_CHARS);
        if self.name.is_empty() {
            return Err(SiteError::Name);
        }
        self.group = clean_group(&self.group);
        self.note = lines::cleaned(&self.note, MAX_NOTE_CHARS);
        match (&self.host, &self.alias) {
            (Some(_), None) | (None, Some(_)) => {}
            _ => return Err(SiteError::HostOrAlias),
        }
        if let Some(host) = &self.host {
            check_host(host)?;
        }
        if let Some(alias) = &self.alias {
            check_host(alias)?;
        }
        if let Some(user) = &self.user {
            check_user(user)?;
        }
        if self.port == Some(0) {
            return Err(SiteError::Target(TargetError::Port));
        }
        if let Some(start) = &self.start_remote
            && (!start.starts_with('/') || start.contains('\0'))
        {
            return Err(SiteError::StartRemote);
        }
        if let Some(program) = &self.sftp_server_program {
            Remote::program(program).map_err(|_| SiteError::Program)?;
        }
        if self
            .key_file
            .as_deref()
            .is_some_and(|k| k.trim().is_empty())
        {
            return Err(SiteError::KeyFile);
        }
        self.bookmarks
            .retain(|b| b.starts_with('/') && !b.contains('\0'));
        self.bookmarks.truncate(MAX_BOOKMARKS);
        Ok(self)
    }

    /// Whether the site only names an ssh alias.
    pub fn is_alias(&self) -> bool {
        self.alias.is_some()
    }

    /// The host as ssh is given it: the host, or the alias.
    pub fn host_or_alias(&self) -> &str {
        self.host
            .as_deref()
            .or(self.alias.as_deref())
            .unwrap_or_default()
    }

    /// Where it connects.
    pub fn target(&self) -> Result<Target, TargetError> {
        let host = self
            .host
            .as_deref()
            .map(|h| check_host(h).unwrap_or_else(|_| h.to_owned()));
        let host = host.as_deref().or(self.alias.as_deref()).unwrap_or("");
        Target::new(host, self.user.as_deref(), self.port)
    }

    /// The account part of a row and of the keyring entry: "alice@build.example",
    /// "build.example:2222", or the alias.
    pub fn account(&self) -> String {
        self.target()
            .map_or_else(|_| self.host_or_alias().to_owned(), |t| t.to_string())
    }

    /// The place its active panel opens: the start folder on the server,
    /// or the root until the server names the home folder.
    pub fn place(&self) -> String {
        let path = self.start_remote.as_deref().unwrap_or("/");
        format!("{SCHEME}://{}{path}", self.account())
    }

    /// Whether this site is the one a connection to `target` uses: the
    /// same host or alias, and the same user and port where the site names
    /// them.
    pub fn matches(&self, target: &Target) -> bool {
        let Ok(own) = self.target() else {
            return false;
        };
        own.host() == target.host()
            && (own.user().is_none() || own.user() == target.user())
            && (own.port().is_none() || own.port() == target.port())
    }

    /// What it changes in how ssh is started.
    pub fn options(&self) -> SiteOptions {
        SiteOptions {
            key_file: self.key_file.as_deref().map(std::path::PathBuf::from),
            add_key_to_agent: self.add_key_to_agent,
            sftp_server_program: self.sftp_server_program.clone(),
        }
    }

    /// The keyring account of its password: `site:ID:user@host`.
    pub fn keyring_account(&self) -> String {
        format!("site:{}:{}", self.id, self.account())
    }

    /// Its row in words, the state right after the name: "build, saved,
    /// alice@build.example".
    pub fn row(&self, state: &str) -> String {
        diroxide_plugins::lines::place_row(&self.name, state, Some(&self.account()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> Site {
        Site {
            user: Some("alice".into()),
            ..Site::new("k3f9", "build", "build.example")
        }
    }

    #[test]
    fn a_site_gives_its_target_place_and_row() {
        let site = site().checked().unwrap();
        assert_eq!(site.account(), "alice@build.example");
        assert_eq!(site.place(), "sftp://alice@build.example/");
        assert_eq!(site.row("saved"), "build, saved, alice@build.example");
        assert_eq!(site.keyring_account(), "site:k3f9:alice@build.example");
        let with_port = Site {
            port: Some(2222),
            start_remote: Some("/srv/projects".into()),
            ..site.clone()
        };
        assert_eq!(
            with_port.place(),
            "sftp://alice@build.example:2222/srv/projects"
        );
        assert!(site.matches(&Target::new("build.example", Some("alice"), None).unwrap()));
        assert!(!site.matches(&Target::new("build.example", Some("bob"), None).unwrap()));
        assert!(!site.matches(&Target::new("nas", Some("alice"), None).unwrap()));
        let any_user = Site::new("a1", "nas", "nas.example");
        assert!(any_user.matches(&Target::new("nas.example", Some("bob"), Some(22)).unwrap()));
    }

    #[test]
    fn hostile_fields_are_refused_in_words() {
        for (bad, error) in [
            (
                Site {
                    id: "../x".into(),
                    ..site()
                },
                SiteError::Id,
            ),
            (
                Site {
                    name: "\u{202E}".into(),
                    ..site()
                },
                SiteError::Name,
            ),
            (
                Site {
                    host: Some("-oProxyCommand=x".into()),
                    ..site()
                },
                SiteError::Target(TargetError::HostDash),
            ),
            (
                Site {
                    alias: Some("box".into()),
                    ..site()
                },
                SiteError::HostOrAlias,
            ),
            (
                Site {
                    user: Some("a b".into()),
                    ..site()
                },
                SiteError::Target(TargetError::UserCharacter),
            ),
            (
                Site {
                    port: Some(0),
                    ..site()
                },
                SiteError::Target(TargetError::Port),
            ),
            (
                Site {
                    start_remote: Some("srv".into()),
                    ..site()
                },
                SiteError::StartRemote,
            ),
            (
                Site {
                    sftp_server_program: Some("sftp-server; rm".into()),
                    ..site()
                },
                SiteError::Program,
            ),
        ] {
            assert_eq!(bad.checked().unwrap_err(), error);
        }
        for error in [
            SiteError::Id,
            SiteError::HostOrAlias,
            SiteError::StartRemote,
        ] {
            assert!(error.to_string().chars().count() <= 60, "{error}");
        }
    }

    #[test]
    fn names_groups_and_ids_are_cleaned() {
        let site = Site {
            name: "  build\u{1B}[31m  ".into(),
            group: "/Work//Servers/ ".into(),
            ..site()
        }
        .checked()
        .unwrap();
        assert_eq!(site.name, "build");
        assert_eq!(site.group, "Work/Servers");
        assert!(plain_id(&new_id()));
        assert_eq!(new_id().len(), 8);
    }

    #[test]
    fn the_mode_words_round_trip() {
        for mode in [SecretMode::Never, SecretMode::Session, SecretMode::Keyring] {
            assert_eq!(SecretMode::from_word(mode.word()), Some(mode));
        }
        assert_eq!(SecretMode::from_word("hunter2"), None);
        assert_eq!(SecretMode::default(), SecretMode::Never);
    }
}
