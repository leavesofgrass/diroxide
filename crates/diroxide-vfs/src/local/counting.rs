//! Counts of what the local provider asks the system for each entry, so
//! the tests can hold a listing and a stat to their cost (PF, alpha 5):
//! the handles opened, the details read by name, and the full paths
//! built. Each count is the calling thread's own, so tests running side
//! by side never see each other's. Outside this crate's tests every
//! count compiles to nothing.

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static OPENS: Cell<u64> = const { Cell::new(0) };
    static LOOKUPS: Cell<u64> = const { Cell::new(0) };
    static PATHS: Cell<u64> = const { Cell::new(0) };
}

/// An entry was opened, a handle made and closed.
#[inline]
pub(crate) fn opened() {
    #[cfg(test)]
    OPENS.with(|count| count.set(count.get() + 1));
}

/// An entry's details were read by its name without a handle the
/// provider keeps (`lstat`, or a folder entry's stat on Unix).
#[inline]
pub(crate) fn looked_up() {
    #[cfg(test)]
    LOOKUPS.with(|count| count.set(count.get() + 1));
}

/// An entry's full path was built.
#[inline]
pub(crate) fn path_built() {
    #[cfg(test)]
    PATHS.with(|count| count.set(count.get() + 1));
}

/// What one thread asked for since the last [`take`].
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Handles opened.
    pub(crate) opens: u64,
    /// Details read by name.
    pub(crate) lookups: u64,
    /// Full paths built.
    pub(crate) paths: u64,
}

/// This thread's counts, which start again from nothing.
#[cfg(test)]
pub(crate) fn take() -> Counts {
    Counts {
        opens: OPENS.with(|count| count.replace(0)),
        lookups: LOOKUPS.with(|count| count.replace(0)),
        paths: PATHS.with(|count| count.replace(0)),
    }
}
