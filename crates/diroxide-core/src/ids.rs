//! Identifiers shared by every crate: the two sides, and numeric ids for
//! jobs, requests, dialogs and tabs.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// One of the two panels. One is active; the other is the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The left panel.
    Left,
    /// The right panel.
    Right,
}

impl Side {
    /// Both sides, left first.
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    /// The other side: the target of copy and move.
    pub const fn other(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Zero for the left side, one for the right, for indexing pairs.
    pub const fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
        }
    }

    /// The side in one lowercase word, as said aloud: "left" or "right".
    pub const fn word(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

macro_rules! numeric_id {
    ($(#[$meta:meta])* $name:ident, $word:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Wraps a raw number. Ids from an [`IdSource`] start at one.
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            /// The raw number.
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($word, " {}"), self.0)
            }
        }
    };
}

numeric_id!(
    /// A job in the job engine: a copy, a delete, a search, a conversion.
    JobId,
    "job"
);
numeric_id!(
    /// A request to a worker, such as listing a place, so that a late answer
    /// to an old request can be recognized and dropped.
    RequestId,
    "request"
);
numeric_id!(
    /// An open dialog.
    DialogId,
    "dialog"
);
numeric_id!(
    /// A folder tab in a panel.
    TabId,
    "tab"
);

/// Hands out increasing ids, starting at one. Safe to share between threads.
#[derive(Debug, Default)]
pub struct IdSource(AtomicU64);

impl IdSource {
    /// A new source whose first id is one.
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// The next raw number.
    pub fn next_raw(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The next job id.
    pub fn job(&self) -> JobId {
        JobId::new(self.next_raw())
    }

    /// The next request id.
    pub fn request(&self) -> RequestId {
        RequestId::new(self.next_raw())
    }

    /// The next dialog id.
    pub fn dialog(&self) -> DialogId {
        DialogId::new(self.next_raw())
    }

    /// The next tab id.
    pub fn tab(&self) -> TabId {
        TabId::new(self.next_raw())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_swap_and_index() {
        assert_eq!(Side::Left.other(), Side::Right);
        assert_eq!(Side::Right.other(), Side::Left);
        assert_eq!(Side::Right.index(), 1);
        assert_eq!(Side::Left.to_string(), "left");
    }

    #[test]
    fn ids_increase_from_one() {
        let ids = IdSource::new();
        assert_eq!(ids.job(), JobId::new(1));
        assert_eq!(ids.request().get(), 2);
        assert_eq!(ids.dialog().to_string(), "dialog 3");
    }
}
