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

/// How long a read of the two files is trusted.
const KEEP_FOR: Duration = Duration::from_secs(30);

/// The names of users and groups by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Owners {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

impl Owners {
    /// Reads the two files' text: `name:password:uid:...` per user line and
    /// `name:password:gid:members` per group line. Comments, blank lines
    /// and malformed lines are skipped; the first name for a number wins.
    pub(crate) fn parse(passwd: &str, group: &str) -> Self {
        Self {
            users: by_number(passwd),
            groups: by_number(group),
        }
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
    }
}
