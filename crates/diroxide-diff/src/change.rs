//! The change model: one change is a stretch of lines that differs,
//! between stretches that are the same, and the change is the unit a
//! listener hears, one list row each (R18).

use std::ops::Range;

/// What happened to a stretch of lines, from before to after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// Lines in both files differ.
    Changed,
    /// Lines only in the after file.
    Added,
    /// Lines only in the before file.
    Removed,
}

impl ChangeKind {
    /// The kind as the first word of a row: "Changed", "Added", "Removed".
    pub const fn word(self) -> &'static str {
        match self {
            ChangeKind::Changed => "Changed",
            ChangeKind::Added => "Added",
            ChangeKind::Removed => "Removed",
        }
    }

    /// The kind with the direction turned around: an addition becomes a
    /// removal and the other way about.
    pub const fn swapped(self) -> ChangeKind {
        match self {
            ChangeKind::Changed => ChangeKind::Changed,
            ChangeKind::Added => ChangeKind::Removed,
            ChangeKind::Removed => ChangeKind::Added,
        }
    }
}

/// One change. Lines are counted from 0 in the ranges; rows say them from
/// 1 ([`Change::first_line`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Change {
    /// What happened.
    pub kind: ChangeKind,
    /// The before file's lines it covers. Empty for an addition, where it
    /// marks the place in the before file the lines were added.
    pub before: Range<usize>,
    /// The after file's lines it covers. Empty for a removal, where it
    /// marks the place in the after file the lines were removed from.
    pub after: Range<usize>,
}

impl Change {
    /// The change between two ranges, its kind read from which is empty.
    /// At least one range must hold a line.
    pub(crate) fn between(before: Range<usize>, after: Range<usize>) -> Change {
        let kind = match (before.is_empty(), after.is_empty()) {
            (true, false) => ChangeKind::Added,
            (false, true) => ChangeKind::Removed,
            _ => ChangeKind::Changed,
        };
        Change {
            kind,
            before,
            after,
        }
    }

    /// The same change read the other way: before and after exchanged.
    pub fn swapped(&self) -> Change {
        Change {
            kind: self.kind.swapped(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }

    /// The lines a row names, counted from 1: the after file's for a
    /// change or an addition, the before file's for a removal (R18's
    /// numbering rule).
    pub fn lines(&self) -> Range<usize> {
        let range = match self.kind {
            ChangeKind::Removed => &self.before,
            ChangeKind::Changed | ChangeKind::Added => &self.after,
        };
        range.start + 1..range.end + 1
    }

    /// The first line a row names, counted from 1.
    pub fn first_line(&self) -> usize {
        self.lines().start
    }

    /// The last line a row names, counted from 1.
    pub fn last_line(&self) -> usize {
        self.lines().end - 1
    }
}

/// The counts of a compare: changes by kind, and lines inside them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counts {
    /// Changes in all: the rows of the change list.
    pub changes: usize,
    /// Changes where lines in both files differ.
    pub changed: usize,
    /// Changes that add lines.
    pub added: usize,
    /// Changes that remove lines.
    pub removed: usize,
    /// Lines of the before file inside changes.
    pub lines_removed: usize,
    /// Lines of the after file inside changes.
    pub lines_added: usize,
}

impl Counts {
    /// The counts of a list of changes.
    pub fn of(changes: &[Change]) -> Counts {
        let mut counts = Counts {
            changes: changes.len(),
            ..Counts::default()
        };
        for change in changes {
            match change.kind {
                ChangeKind::Changed => counts.changed += 1,
                ChangeKind::Added => counts.added += 1,
                ChangeKind::Removed => counts.removed += 1,
            }
            counts.lines_removed += change.before.len();
            counts.lines_added += change.after.len();
        }
        counts
    }

    /// Whether the kinds add up to the total.
    pub fn adds_up(&self) -> bool {
        self.changed + self.added + self.removed == self.changes
    }

    /// The counts read the other way: additions and removals exchanged.
    pub fn swapped(&self) -> Counts {
        Counts {
            added: self.removed,
            removed: self.added,
            lines_removed: self.lines_added,
            lines_added: self.lines_removed,
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_empty_side() {
        assert_eq!(Change::between(0..0, 3..5).kind, ChangeKind::Added);
        assert_eq!(Change::between(2..3, 4..4).kind, ChangeKind::Removed);
        assert_eq!(Change::between(2..3, 2..4).kind, ChangeKind::Changed);
    }

    #[test]
    fn rows_number_lines_by_the_rule() {
        // Added lines 52 to 54 of the after file.
        let added = Change::between(40..40, 51..54);
        assert_eq!((added.first_line(), added.last_line()), (52, 54));
        // Removed line 12 of the before file.
        let removed = Change::between(11..12, 9..9);
        assert_eq!((removed.first_line(), removed.last_line()), (12, 12));
        // Changed: the after file's lines.
        let changed = Change::between(39..42, 39..41);
        assert_eq!(changed.lines(), 40..42);
    }

    #[test]
    fn counts_add_up_and_swap() {
        let changes = [
            Change::between(0..1, 0..1),
            Change::between(3..3, 3..6),
            Change::between(8..10, 11..11),
        ];
        let counts = Counts::of(&changes);
        assert!(counts.adds_up());
        assert_eq!((counts.changed, counts.added, counts.removed), (1, 1, 1));
        assert_eq!((counts.lines_removed, counts.lines_added), (3, 4));
        let swapped: Vec<Change> = changes.iter().map(Change::swapped).collect();
        assert_eq!(Counts::of(&swapped), counts.swapped());
        assert_eq!(counts.swapped().swapped(), counts);
    }
}
