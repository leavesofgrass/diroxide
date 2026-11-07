//! Owner and group names on Unix, read from `/etc/passwd` and
//! `/etc/group` once and kept for half a minute, so a listing of 100,000
//! entries makes two small reads instead of 200,000 lookups, and no libc
//! function that is unsafe across threads is ever called (research
//! file-operations.md, "Large directories").
//!
//! Users that only a directory service knows (LDAP, macOS Directory
//! Services) are not in these files; their rows show the number, as
//! `ls -n` would ("owner lookup falls back to the number", permissions.md
//! section 12).
#![cfg_attr(not(unix), allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::provider::Principal;

/// How long a read of the two files is trusted.
const KEEP_FOR: Duration = Duration::from_secs(30);

/// The names of users and groups by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Owners {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
    /// Users' full names, the first part of the description field, where
    /// one is given and differs from the name.
    full_names: HashMap<u32, String>,
}

impl Owners {
    /// Reads the two files' text: `name:password:uid:...` per user line and
    /// `name:password:gid:members` per group line. Comments, blank lines
    /// and malformed lines are skipped; the first name for a number wins.
    pub(crate) fn parse(passwd: &str, group: &str) -> Self {
        let users = by_number(passwd);
        let full_names = full_names(passwd, &users);
        Self {
            users,
            groups: by_number(group),
            full_names,
        }
    }

    /// A user's full name, where the account's description gives one
    /// that differs from the name: "Ada Example".
    pub(crate) fn full_name(&self, uid: u32) -> Option<&str> {
        self.full_names.get(&uid).map(String::as_str)
    }

    /// A user or group by name or number, for a change of owner: the
    /// number as the id and the name where known. A number this computer
    /// has no name for is still a principal (an owner can be any number);
    /// an unknown name is `None`.
    pub(crate) fn principal(&self, text: &str, group: bool) -> Option<Principal> {
        let (number, name) = if let Ok(number) = text.parse::<u32>() {
            let name = if group {
                self.groups.get(&number)
            } else {
                self.users.get(&number)
            };
            (number, name.cloned())
        } else {
            let number = if group {
                self.group_number(text)
            } else {
                self.user_number(text)
            }?;
            (number, Some(text.to_owned()))
        };
        Some(Principal {
            id: number.to_string(),
            name,
        })
    }

    /// The user's name, or the number when no name is known.
    pub(crate) fn user(&self, uid: u32) -> String {
        self.users
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }

    /// The group's name, or the number when no name is known.
    pub(crate) fn group(&self, gid: u32) -> String {
        self.groups
            .get(&gid)
            .cloned()
            .unwrap_or_else(|| gid.to_string())
    }

    /// The user's number for a name, for setting an owner by name.
    pub(crate) fn user_number(&self, name: &str) -> Option<u32> {
        self.users
            .iter()
            .find_map(|(id, n)| (n == name).then_some(*id))
    }

    /// The group's number for a name.
    pub(crate) fn group_number(&self, name: &str) -> Option<u32> {
        self.groups
            .iter()
            .find_map(|(id, n)| (n == name).then_some(*id))
    }

    /// The names as this machine has them now, read at most every half
    /// minute.
    pub(crate) fn shared() -> Arc<Owners> {
        /// The last read and when it was made.
        type Cached = Option<(Instant, Arc<Owners>)>;
        static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        let mut slot = match cache.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some((read_at, owners)) = slot.as_ref()
            && read_at.elapsed() < KEEP_FOR
        {
            return Arc::clone(owners);
        }
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        let owners = Arc::new(Owners::parse(&read("/etc/passwd"), &read("/etc/group")));
        *slot = Some((Instant::now(), Arc::clone(&owners)));
        owners
    }
}

/// The full names in a password file's description fields (the part
/// before the first comma), kept only where one is given and differs from
/// the user's name.
fn full_names(passwd: &str, users: &HashMap<u32, String>) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for line in passwd.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        let (Some(name), Some(number), Some(description)) =
            (fields.first(), fields.get(2), fields.get(4))
        else {
            continue;
        };
        let Ok(number) = number.parse::<u32>() else {
            continue;
        };
        let full = description.split(',').next().unwrap_or("").trim();
        if full.is_empty() || full == *name || users.get(&number).map(String::as_str) != Some(*name)
        {
            continue;
        }
        map.entry(number).or_insert_with(|| full.to_owned());
    }
    map
}

fn by_number(text: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(':');
        let (Some(name), Some(_), Some(number)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if let Ok(number) = number.parse::<u32>() {
            map.entry(number).or_insert_with(|| name.to_owned());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_files_and_numbers_fill_the_gaps() {
        let owners = Owners::parse(
            "# comment\nroot:x:0:0:root:/root:/bin/sh\nada:x:1000:1000:Ada Example:/home/ada:/bin/bash\nbroken\n:x:5:5\nada2:x:1000:1000::/:/bin/sh\n",
            "root:x:0:\nstaff:x:50:ada\n",
        );
        assert_eq!(owners.user(0), "root");
        assert_eq!(owners.user(1000), "ada");
        assert_eq!(owners.user(4242), "4242");
        assert_eq!(owners.user(5), "5");
        assert_eq!(owners.group(50), "staff");
        assert_eq!(owners.group(7), "7");
        assert_eq!(owners.user_number("ada"), Some(1000));
        assert_eq!(owners.group_number("staff"), Some(50));
        assert_eq!(owners.group_number("nobody"), None);
        assert_eq!(owners.full_name(1000), Some("Ada Example"));
        assert_eq!(owners.full_name(0), None, "root's description is its name");
    }

    #[test]
    fn a_principal_is_found_by_name_or_number() {
        let owners = Owners::parse(
            "ada:x:1000:1000:Ada Example,,,:/home/ada:/bin/bash\n",
            "staff:x:50:ada\n",
        );
        let ada = owners.principal("ada", false).expect("ada is known");
        assert_eq!(
            (ada.id.as_str(), ada.name.as_deref()),
            ("1000", Some("ada"))
        );
        let by_number = owners.principal("1000", false).expect("a number");
        assert_eq!(by_number.name.as_deref(), Some("ada"));
        let unknown_number = owners.principal("4242", false).expect("any number");
        assert_eq!(unknown_number.name, None);
        assert_eq!(owners.principal("bob", false), None);
        let staff = owners.principal("staff", true).expect("a group");
        assert_eq!(staff.id, "50");
        assert_eq!(owners.principal("ada", true), None, "not a group");
    }
}
