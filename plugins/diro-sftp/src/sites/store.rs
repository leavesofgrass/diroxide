//! The sites file, `sites.toml` in the plugin's state folder (answers 83,
//! 94; R24 section 3.2): read and written with comment-keeping edits and
//! an atomic replace.
//!
//! - **No secret is ever in it.** A file in which any table holds a key
//!   that names a password, a passphrase or a token, or a `secret` whose
//!   value is not one of the three mode words, is refused whole and said
//!   ("Sites file refused: it holds a password."), with the key and the
//!   site named so the owner can remove it. Nothing is read from such a
//!   file, and nothing is written over it.
//! - **The owner's words stay.** Comments, blank lines and the order of
//!   keys and sites survive a save; an edited field keeps the comment
//!   after it.
//! - **A bad site is left out, not the file.** A site whose host, user or
//!   port fails the address rules is counted and said; the others are
//!   offered.
//! - **Atomic and private.** A save writes `sites.toml.diro-new` beside
//!   the file and renames it over the old one; on Unix the file is the
//!   owner's alone (mode 0600). It names hosts and users, not secrets.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

use super::model::{SecretMode, Site, SiteError};
use super::words;

/// The file's name in the plugin's state folder.
pub const FILE_NAME: &str = "sites.toml";

/// The name a save writes first, then renames over [`FILE_NAME`].
pub const NEW_FILE_NAME: &str = "sites.toml.diro-new";

/// The largest sites file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The table of shared bookmarks, for every site.
pub const SHARED: &str = "shared";

/// The array of site tables.
pub const SITE: &str = "site";

/// Keys that hold a secret wherever they appear: refused.
const SECRET_KEYS: [&str; 12] = [
    "pass",
    "pw",
    "pwd",
    "pin",
    "otp",
    "token",
    "secret_value",
    "private_key",
    "key_data",
    "api_key",
    "auth",
    "cookie",
];

/// Parts of a key's name that mean a secret: refused.
const SECRET_PARTS: [&str; 5] = ["password", "passwd", "passphrase", "credential", "token"];

/// Why the sites file was not used.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A key that holds a secret, in the site or table named.
    #[error("{}", words::FILE_HOLDS_SECRET)]
    HoldsSecret {
        /// The key.
        key: String,
        /// The site's name, or the table's.
        site: String,
    },
    /// Not TOML.
    #[error("{}", words::FILE_NOT_TOML)]
    NotToml(String),
    /// Too large to be a sites file.
    #[error("{}", words::FILE_TOO_LARGE)]
    TooLarge,
    /// Could not be read.
    #[error("{}", words::FILE_UNREADABLE)]
    Unreadable(#[source] io::Error),
    /// Could not be written.
    #[error("{}", words::FILE_NOT_SAVED)]
    NotSaved(#[source] io::Error),
    /// A site to add or change failed a check.
    #[error("{0}")]
    Site(#[from] SiteError),
    /// A site to change or remove is not in the file.
    #[error("{}", words::NO_SITE_HERE)]
    NoSuchSite,
}

impl StoreError {
    /// The refusal in lines, one fact a line: the refusal, then what to
    /// do, then why.
    pub fn lines(&self) -> Vec<String> {
        match self {
            StoreError::HoldsSecret { key, site } => vec![
                self.to_string(),
                words::remove_secret_field(key, site),
                words::NO_SECRET_IN_FILES.to_owned(),
            ],
            StoreError::NotToml(detail) => vec![
                self.to_string(),
                crate::lines::cleaned(detail, crate::lines::MAX_LINE_CELLS * 2),
            ],
            StoreError::Unreadable(e) | StoreError::NotSaved(e) => {
                vec![self.to_string(), crate::lines::cleaned(&e.to_string(), 80)]
            }
            _ => vec![self.to_string()],
        }
    }
}

/// Whether a key names a secret.
fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEYS.contains(&key.as_str()) || SECRET_PARTS.iter().any(|part| key.contains(part))
}

/// Finds a planted secret in a table and everything under it: `site` is
/// the name said with it; `site_table` says whether `secret` is a mode
/// here.
fn find_secret(table: &dyn TableLike, site: &str, site_table: bool) -> Option<StoreError> {
    for (key, item) in table.iter() {
        let planted = || StoreError::HoldsSecret {
            key: crate::lines::cleaned(key, 20),
            site: site.to_owned(),
        };
        if is_secret_key(key) {
            return Some(planted());
        }
        if key.eq_ignore_ascii_case("secret") {
            let mode = item.as_str().and_then(SecretMode::from_word);
            if !site_table || mode.is_none() {
                return Some(planted());
            }
        }
        if let Some(inner) = item.as_table_like()
            && let Some(found) = find_secret(inner, site, false)
        {
            return Some(found);
        }
        if let Some(tables) = item.as_array_of_tables() {
            for inner in tables {
                if let Some(found) = find_secret(inner, site, false) {
                    return Some(found);
                }
            }
        }
        if let Some(array) = item.as_array() {
            for value in array {
                if let Some(inner) = value.as_inline_table()
                    && let Some(found) = find_secret(inner, site, false)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn text(table: &Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

fn texts(table: &Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads one site table; `Err` with the reason for one that cannot be
/// used.
fn read_site(table: &Table) -> Result<Site, SiteError> {
    let port = match table.get("port").and_then(Item::as_integer) {
        None => None,
        Some(n) => Some(
            u16::try_from(n)
                .ok()
                .filter(|p| *p != 0)
                .ok_or(SiteError::Target(crate::ssh::TargetError::Port))?,
        ),
    };
    let secret = match table.get("secret").and_then(Item::as_str) {
        None => SecretMode::Never,
        Some(word) => SecretMode::from_word(word).ok_or(SiteError::SecretMode)?,
    };
    Site {
        id: text(table, "id").unwrap_or_default(),
        name: text(table, "name").unwrap_or_default(),
        group: text(table, "group").unwrap_or_default(),
        host: text(table, "host"),
        alias: text(table, "alias"),
        port,
        user: text(table, "user"),
        key_file: text(table, "key_file"),
        start_remote: text(table, "start_remote"),
        start_local: text(table, "start_local"),
        bookmarks: texts(table, "bookmarks"),
        secret,
        add_key_to_agent: table
            .get("add_key_to_agent")
            .and_then(Item::as_bool)
            .unwrap_or(false),
        sftp_server_program: text(table, "sftp_server_program"),
        note: table
            .get("note")
            .and_then(Item::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
    .checked()
}

/// Sets a key's value, keeping the comment after the old one; `None`
/// removes the key.
fn set(table: &mut Table, key: &str, new: Option<Value>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(mut value) => {
            if let Some(Item::Value(old)) = table.get_mut(key) {
                *value.decor_mut() = old.decor().clone();
                *old = value;
            } else {
                table.insert(key, Item::Value(value));
            }
        }
    }
}

fn opt_text(text: Option<&str>) -> Option<Value> {
    text.filter(|t| !t.is_empty()).map(Value::from)
}

/// Writes a site's fields into its table, in R24's order for a new one.
fn write_site(table: &mut Table, site: &Site) {
    set(table, "id", Some(Value::from(site.id.as_str())));
    set(table, "name", Some(Value::from(site.name.as_str())));
    set(table, "group", opt_text(Some(&site.group)));
    set(table, "host", opt_text(site.host.as_deref()));
    set(table, "alias", opt_text(site.alias.as_deref()));
    set(table, "port", site.port.map(|p| Value::from(i64::from(p))));
    set(table, "user", opt_text(site.user.as_deref()));
    set(table, "key_file", opt_text(site.key_file.as_deref()));
    set(
        table,
        "start_remote",
        opt_text(site.start_remote.as_deref()),
    );
    set(table, "start_local", opt_text(site.start_local.as_deref()));
    let bookmarks = (!site.bookmarks.is_empty())
        .then(|| Value::Array(site.bookmarks.iter().map(String::as_str).collect::<Array>()));
    set(table, "bookmarks", bookmarks);
    set(table, "secret", Some(Value::from(site.secret.word())));
    set(
        table,
        "add_key_to_agent",
        site.add_key_to_agent.then(|| Value::from(true)),
    );
    set(
        table,
        "sftp_server_program",
        opt_text(site.sftp_server_program.as_deref()),
    );
    set(table, "note", opt_text(Some(&site.note)));
}

/// The sites file, read.
#[derive(Debug)]
pub struct SitesFile {
    path: Option<PathBuf>,
    doc: DocumentMut,
    sites: Vec<Site>,
    shared_bookmarks: Vec<String>,
    skipped: Vec<String>,
}

impl SitesFile {
    /// The sites in `text`; `path` is where a save goes (`None` keeps the
    /// file in memory only). A planted secret refuses the whole file.
    pub fn parse(text: &str, path: Option<PathBuf>) -> Result<Self, StoreError> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| StoreError::NotToml(e.message().to_owned()))?;
        let mut sites = Vec::new();
        let mut skipped = Vec::new();
        for (key, item) in doc.iter() {
            let checked = match (key, item) {
                (SITE, Item::ArrayOfTables(tables)) => {
                    for table in tables {
                        let name = table
                            .get("name")
                            .and_then(Item::as_str)
                            .map(|n| crate::lines::cleaned(n, 30))
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| "a site".to_owned());
                        if let Some(found) = find_secret(table, &name, true) {
                            return Err(found);
                        }
                    }
                    continue;
                }
                (key, Item::Table(table)) => find_secret(table, key, false),
                (key, Item::ArrayOfTables(tables)) => {
                    tables.iter().find_map(|t| find_secret(t, key, false))
                }
                (key, Item::Value(Value::InlineTable(table))) => find_secret(table, key, false),
                (key, _) if is_secret_key(key) || key.eq_ignore_ascii_case("secret") => {
                    Some(StoreError::HoldsSecret {
                        key: crate::lines::cleaned(key, 20),
                        site: "the file".to_owned(),
                    })
                }
                _ => None,
            };
            if let Some(found) = checked {
                return Err(found);
            }
        }
        if let Some(tables) = doc.get(SITE).and_then(Item::as_array_of_tables) {
            for table in tables {
                match read_site(table) {
                    Ok(site) if sites.iter().any(|s: &Site| s.id == site.id) => {
                        skipped.push(format!("{}: the id is used twice.", site.name));
                    }
                    Ok(site) => sites.push(site),
                    Err(e) => skipped.push(e.to_string()),
                }
            }
        }
        let shared_bookmarks = doc
            .get(SHARED)
            .and_then(Item::as_table)
            .map(|t| texts(t, "bookmarks"))
            .unwrap_or_default();
        Ok(Self {
            path,
            doc,
            sites,
            shared_bookmarks,
            skipped,
        })
    }

    /// The sites file in the state folder `dir`; a missing file is an
    /// empty list.
    pub fn load(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(FILE_NAME);
        match fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX_FILE_BYTES => return Err(StoreError::TooLarge),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Self::parse("", Some(path));
            }
            Err(e) => return Err(StoreError::Unreadable(e)),
        }
        let text = fs::read_to_string(&path).map_err(StoreError::Unreadable)?;
        Self::parse(&text, Some(path))
    }

    /// The sites that can be used, in the file's order.
    pub fn sites(&self) -> &[Site] {
        &self.sites
    }

    /// The site with an id.
    pub fn site(&self, id: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.id == id)
    }

    /// The bookmarks every site shares.
    pub fn shared_bookmarks(&self) -> &[String] {
        &self.shared_bookmarks
    }

    /// The sites left out, each with its reason.
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// The file's text as a save would write it.
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    fn tables(&mut self) -> &mut ArrayOfTables {
        if !self
            .doc
            .get(SITE)
            .is_some_and(|item| item.is_array_of_tables())
        {
            self.doc
                .insert(SITE, Item::ArrayOfTables(ArrayOfTables::new()));
        }
        self.doc
            .get_mut(SITE)
            .and_then(Item::as_array_of_tables_mut)
            .unwrap_or_else(|| unreachable!("inserted above"))
    }

    fn index_of(&mut self, id: &str) -> Option<usize> {
        self.tables()
            .iter()
            .position(|t| t.get("id").and_then(Item::as_str) == Some(id))
    }

    /// Adds a site at the end, checked.
    pub fn add(&mut self, site: Site) -> Result<(), StoreError> {
        let site = site.checked()?;
        let mut table = Table::new();
        write_site(&mut table, &site);
        self.tables().push(table);
        self.sites.push(site);
        Ok(())
    }

    /// Changes a site in place, its comments kept.
    pub fn update(&mut self, site: Site) -> Result<(), StoreError> {
        let site = site.checked()?;
        let index = self.index_of(&site.id).ok_or(StoreError::NoSuchSite)?;
        let tables = self.tables();
        let table = tables.get_mut(index).ok_or(StoreError::NoSuchSite)?;
        write_site(table, &site);
        if let Some(old) = self.sites.iter_mut().find(|s| s.id == site.id) {
            *old = site;
        }
        Ok(())
    }

    /// Removes a site; the removed one.
    pub fn remove(&mut self, id: &str) -> Result<Site, StoreError> {
        let index = self.index_of(id).ok_or(StoreError::NoSuchSite)?;
        self.tables().remove(index);
        let at = self
            .sites
            .iter()
            .position(|s| s.id == id)
            .ok_or(StoreError::NoSuchSite)?;
        Ok(self.sites.remove(at))
    }

    /// Writes the file: the new text beside it, then renamed over it.
    pub fn save(&self) -> Result<(), StoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let folder = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(folder).map_err(StoreError::NotSaved)?;
        let new = folder.join(NEW_FILE_NAME);
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&new).map_err(StoreError::NotSaved)?;
        file.write_all(self.text().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(StoreError::NotSaved)?;
        drop(file);
        fs::rename(&new, path).map_err(StoreError::NotSaved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = "\
# The owner's sites. Comments stay.

[shared]
bookmarks = [\"/var/log\"]

[[site]]
id = \"k3f9\"            # never spoken
name = \"build\"
group = \"Work/Servers\"
host = \"build.example\"
port = 22
user = \"alice\"
start_remote = \"/srv/projects\"
secret = \"never\"

# The NAS, through the ssh config.
[[site]]
id = \"n4s0\"
name = \"nas\"
alias = \"nas\"
secret = \"keyring\"
";

    #[test]
    fn a_round_trip_keeps_every_comment() {
        let file = SitesFile::parse(SAMPLE, None).unwrap();
        assert_eq!(file.text(), SAMPLE);
        assert_eq!(file.sites().len(), 2);
        assert_eq!(file.shared_bookmarks(), ["/var/log"]);
        assert_eq!(file.sites()[1].secret, SecretMode::Keyring);
        assert!(file.sites()[1].is_alias());
    }

    #[test]
    fn an_edit_keeps_comments_and_order() {
        let mut file = SitesFile::parse(SAMPLE, None).unwrap();
        let mut build = file.site("k3f9").unwrap().clone();
        build.port = Some(2222);
        build.secret = SecretMode::Session;
        file.update(build).unwrap();
        let text = file.text();
        assert!(text.contains("# The owner's sites. Comments stay."));
        assert!(text.contains("id = \"k3f9\"            # never spoken"));
        assert!(text.contains("# The NAS, through the ssh config."));
        assert!(text.contains("port = 2222"));
        assert!(text.contains("secret = \"session\""));
        assert!(text.find("k3f9") < text.find("n4s0"));
        let again = SitesFile::parse(&text, None).unwrap();
        assert_eq!(again.site("k3f9").unwrap().port, Some(2222));
    }

    #[test]
    fn add_and_remove_change_only_their_site() {
        let mut file = SitesFile::parse(SAMPLE, None).unwrap();
        file.add(Site::new("a1b2", "web", "web.example")).unwrap();
        assert_eq!(file.sites().len(), 3);
        let removed = file.remove("k3f9").unwrap();
        assert_eq!(removed.name, "build");
        let text = file.text();
        assert!(!text.contains("build.example"));
        assert!(text.contains("# The NAS, through the ssh config."));
        assert!(text.contains("web.example"));
        assert!(matches!(file.remove("k3f9"), Err(StoreError::NoSuchSite)));
        let refused = file.add(Site::new("-x", "bad", "bad.example"));
        assert!(matches!(refused, Err(StoreError::Site(SiteError::Id))));
    }

    #[test]
    fn a_planted_password_refuses_the_whole_file_in_words() {
        for (line, planted, key, site) in [
            (
                "start_remote = \"/srv/projects\"",
                "password = \"hunter2\"",
                "password",
                "build",
            ),
            (
                "start_remote = \"/srv/projects\"",
                "Password = \"hunter2\"",
                "Password",
                "build",
            ),
            (
                "start_remote = \"/srv/projects\"",
                "key_passphrase = \"x\"",
                "key_passphrase",
                "build",
            ),
            (
                "secret = \"never\"",
                "secret = \"hunter2\"",
                "secret",
                "build",
            ),
            ("secret = \"keyring\"", "token = \"x\"", "token", "nas"),
            (
                "start_remote = \"/srv/projects\"",
                "extra = { pass = \"x\" }",
                "pass",
                "build",
            ),
        ] {
            let text = SAMPLE.replace(line, planted);
            let refused = SitesFile::parse(&text, None).unwrap_err();
            assert_eq!(
                refused.to_string(),
                "Sites file refused: it holds a password."
            );
            let lines = refused.lines();
            assert_eq!(lines[1], format!("Remove {key} from {site}."), "{planted}");
            assert_eq!(lines[2], "diroxide keeps no password in a file.");
        }
        let top = format!("password = \"x\"\n{SAMPLE}");
        assert!(matches!(
            SitesFile::parse(&top, None),
            Err(StoreError::HoldsSecret { .. })
        ));
        let shared = SAMPLE.replace("[shared]", "[shared]\nsecret = \"keyring\"");
        assert!(matches!(
            SitesFile::parse(&shared, None),
            Err(StoreError::HoldsSecret { .. })
        ));
    }

    #[test]
    fn a_bad_site_is_skipped_and_counted() {
        let text = format!(
            "{SAMPLE}\n[[site]]\nid = \"z9\"\nname = \"evil\"\nhost = \"-oProxyCommand=x\"\n"
        );
        let file = SitesFile::parse(&text, None).unwrap();
        assert_eq!(file.sites().len(), 2);
        assert_eq!(file.skipped(), ["The host name starts with a dash."]);
        assert!(matches!(
            SitesFile::parse("[[site]\n", None),
            Err(StoreError::NotToml(_))
        ));
    }

    #[test]
    fn a_save_replaces_the_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = SitesFile::load(dir.path()).unwrap();
        assert!(file.sites().is_empty());
        file.add(Site::new("a1b2", "web", "web.example")).unwrap();
        file.save().unwrap();
        assert!(!dir.path().join(NEW_FILE_NAME).exists());
        let again = SitesFile::load(dir.path()).unwrap();
        assert_eq!(again.sites()[0].name, "web");
        assert!(!again.text().contains("password"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
