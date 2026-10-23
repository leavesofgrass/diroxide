//! Where a password may live (answer 83; R24 section 3.4; ADR-0011):
//! nowhere by default, this session's memory, or the system keyring,
//! each chosen per site.
//!
//! - **The system keyring** is the operating system's own store: Windows
//!   Credential Manager, the macOS keychain, the Secret Service on Linux.
//!   The plugin reaches it through the pure-Rust `keyring-core` and one
//!   store crate per system, which only call the store; the store guards
//!   the secret. The entry is service `diroxide-sftp`, account
//!   `site:ID:user@host`. A machine with no store says so and keeps the
//!   password for this session only.
//! - **This session** ([`SessionSecrets`]) keeps a typed password in the
//!   plugin's memory, wiped on exit, on Forget password, when it is
//!   refused, and when the owner's own Disconnect closes the last
//!   connection.
//! - **Never in a file, a log or a message.** Every value here is a
//!   [`Secret`], whose `Debug` says only that it is hidden and whose
//!   buffer is wiped when dropped; bytes read from a store are wiped as
//!   soon as they are wrapped.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use diroxide_core::Secret;
use zeroize::Zeroize;

use super::words;

/// The keyring's service name for every entry of this plugin.
pub const SERVICE: &str = "diroxide-sftp";

/// Why a store could not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// This machine has no key store (a server with no Secret Service).
    #[error("{}", words::NO_KEY_STORE)]
    NoStore,
    /// The store refused; its reason, cleaned, never a secret.
    #[error("{}", words::KEY_STORE_FAILED)]
    Failed(String),
}

/// A store of passwords by account.
pub trait KeyStore: Send + Sync + fmt::Debug {
    /// Its name in words, for `doctor`: "Windows Credential Manager".
    fn name(&self) -> String;

    /// The password stored for an account; `None` when there is none.
    fn get(&self, account: &str) -> Result<Option<Secret>, KeyError>;

    /// Stores a password for an account, replacing an older one.
    fn set(&self, account: &str, secret: &Secret) -> Result<(), KeyError>;

    /// Removes an account's password; `false` when there was none.
    fn delete(&self, account: &str) -> Result<bool, KeyError>;
}

/// A store in memory: the tests' keyring, and nothing else's.
#[derive(Default)]
pub struct MemoryKeyring {
    entries: Mutex<HashMap<String, Secret>>,
}

impl fmt::Debug for MemoryKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryKeyring")
            .field("entries", &self.len())
            .finish()
    }
}

impl MemoryKeyring {
    /// An empty store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The accounts it holds, sorted, for tests.
    pub fn accounts(&self) -> Vec<String> {
        let mut accounts: Vec<String> = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        accounts.sort();
        accounts
    }
}

impl KeyStore for MemoryKeyring {
    fn name(&self) -> String {
        "a test keyring in memory".to_owned()
    }

    fn get(&self, account: &str) -> Result<Option<Secret>, KeyError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), KeyError> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<bool, KeyError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account)
            .is_some())
    }
}

/// No store at all.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoKeyring;

impl KeyStore for NoKeyring {
    fn name(&self) -> String {
        "none here".to_owned()
    }

    fn get(&self, _: &str) -> Result<Option<Secret>, KeyError> {
        Err(KeyError::NoStore)
    }

    fn set(&self, _: &str, _: &Secret) -> Result<(), KeyError> {
        Err(KeyError::NoStore)
    }

    fn delete(&self, _: &str) -> Result<bool, KeyError> {
        Err(KeyError::NoStore)
    }
}

/// The operating system's own store, opened on first use.
#[derive(Default)]
pub struct SystemKeyring {
    store: OnceLock<Option<Arc<keyring_core::CredentialStore>>>,
}

impl fmt::Debug for SystemKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemKeyring")
            .field("name", &self.name())
            .finish()
    }
}

/// The store's reason, cleaned and short: never a secret, which no store
/// error carries.
fn reason(error: &keyring_core::Error) -> String {
    crate::lines::cleaned(&error.to_string(), 80)
}

/// This system's store, or `None` when there is none.
fn open_store() -> Option<Arc<keyring_core::CredentialStore>> {
    #[cfg(windows)]
    {
        let store: Arc<keyring_core::CredentialStore> =
            windows_native_keyring_store::Store::new().ok()?;
        Some(store)
    }
    #[cfg(target_os = "macos")]
    {
        let store: Arc<keyring_core::CredentialStore> =
            apple_native_keyring_store::keychain::Store::new().ok()?;
        Some(store)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let store: Arc<keyring_core::CredentialStore> =
            zbus_secret_service_keyring_store::Store::new().ok()?;
        Some(store)
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

impl SystemKeyring {
    /// The system's store, not yet opened.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn store(&self) -> Result<&Arc<keyring_core::CredentialStore>, KeyError> {
        self.store
            .get_or_init(open_store)
            .as_ref()
            .ok_or(KeyError::NoStore)
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, KeyError> {
        self.store()?
            .build(SERVICE, account, None)
            .map_err(|e| KeyError::Failed(reason(&e)))
    }
}

impl KeyStore for SystemKeyring {
    fn name(&self) -> String {
        if cfg!(windows) {
            "Windows Credential Manager".to_owned()
        } else if cfg!(target_os = "macos") {
            "the macOS keychain".to_owned()
        } else {
            "the Secret Service".to_owned()
        }
    }

    fn get(&self, account: &str) -> Result<Option<Secret>, KeyError> {
        match self.entry(account)?.get_secret() {
            Ok(mut bytes) => {
                let text = String::from_utf8(bytes.clone()).ok();
                bytes.zeroize();
                Ok(text.map(Secret::new))
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(keyring_core::Error::NoStorageAccess(e)) => {
                Err(KeyError::Failed(crate::lines::cleaned(&e.to_string(), 80)))
            }
            Err(e) => Err(KeyError::Failed(reason(&e))),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), KeyError> {
        self.entry(account)?
            .set_secret(secret.expose().as_bytes())
            .map_err(|e| KeyError::Failed(reason(&e)))
    }

    fn delete(&self, account: &str) -> Result<bool, KeyError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(KeyError::Failed(reason(&e))),
        }
    }
}

/// Passwords kept for this session, by keyring account.
#[derive(Default)]
pub struct SessionSecrets {
    entries: Mutex<HashMap<String, Secret>>,
}

impl fmt::Debug for SessionSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("SessionSecrets")
            .field("entries", &count)
            .finish()
    }
}

impl SessionSecrets {
    /// The password kept for an account.
    pub fn get(&self, account: &str) -> Option<Secret> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned()
    }

    /// Keeps a password for an account.
    pub fn set(&self, account: &str, secret: Secret) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned(), secret);
    }

    /// Forgets an account's password; whether there was one.
    pub fn forget(&self, account: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account)
            .is_some()
    }

    /// Forgets every password: the plugin is closing, or the owner closed
    /// the last connection.
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Whether it keeps any.
    pub fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "canary-7f3a-not-a-password";

    #[test]
    fn the_memory_store_round_trips_and_never_shows_a_secret() {
        let store = MemoryKeyring::new();
        let account = "site:k3f9:ada@build.example";
        assert_eq!(store.get(account).unwrap(), None);
        store.set(account, &Secret::new(CANARY.to_owned())).unwrap();
        assert_eq!(store.get(account).unwrap().unwrap().expose(), CANARY);
        assert!(!format!("{store:?}").contains(CANARY));
        assert!(store.delete(account).unwrap());
        assert!(!store.delete(account).unwrap());
        assert!(store.is_empty());
    }

    #[test]
    fn no_store_says_so_in_words() {
        let none = NoKeyring;
        assert_eq!(none.get("x"), Err(KeyError::NoStore));
        assert_eq!(
            none.set("x", &Secret::new("y".to_owned()))
                .unwrap_err()
                .to_string(),
            "No key store here; nothing saved."
        );
    }

    #[test]
    fn session_secrets_are_kept_forgotten_and_cleared() {
        let session = SessionSecrets::default();
        session.set("a", Secret::new(CANARY.to_owned()));
        assert_eq!(session.get("a").unwrap().expose(), CANARY);
        assert!(!format!("{session:?}").contains(CANARY));
        assert!(session.forget("a"));
        assert!(!session.forget("a"));
        session.set("b", Secret::new("x".to_owned()));
        session.clear();
        assert!(session.is_empty());
    }

    /// Spike SF9's round trip in the real store of this system, under a
    /// test account that names no one. It writes to the owner's keyring,
    /// so it runs only when the owner says so.
    #[test]
    #[ignore = "writes to this system's keyring; run only when the owner says so"]
    fn the_system_store_round_trips() {
        let store = SystemKeyring::new();
        let account = "site:sf9-test:ada@keyring.example";
        store.set(account, &Secret::new(CANARY.to_owned())).unwrap();
        assert_eq!(store.get(account).unwrap().unwrap().expose(), CANARY);
        assert!(store.delete(account).unwrap());
        assert_eq!(store.get(account).unwrap(), None);
    }
}
