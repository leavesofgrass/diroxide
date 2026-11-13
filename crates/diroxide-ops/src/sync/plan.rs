//! The synchronize plan: a compare's result turned into actions, one per
//! row, that the owner reads and changes before anything is copied.
//!
//! Every row says its action first, in the short form: "Copy there:
//! plan.md, only here"; "Replace there: notes.txt, newer here"; "Trash
//! there: old.txt, only there" (question 135's short words).
//! Problems are said first and never acted on; a type that differs is
//! skipped, never resolved; the removals come next, every one listed,
//! to the trash where the place has one and "for good" where it has none.
//! The plan becomes one job of the engine ([`SyncPlan::job_plan`]): the
//! copies first, then the removals.

use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::spell_out;
use diroxide_core::{ConflictPolicy, EntryKind, JobId, JobKind, Place, Side};

use crate::error::OpsError;
use crate::plan::{Plan, Step};
use crate::sync::compare::{
    CompareBy, CompareRow, Comparison, RowState, SideFacts, at, panel, place_at, separator_of,
};

/// How a synchronize decides each row, in words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SyncMode {
    /// Copy what is missing or newer here to the other panel; nothing is
    /// removed ("Update the other panel").
    #[default]
    UpdateThere,
    /// Copy what is missing or newer there to this panel; nothing is
    /// removed ("Update this panel").
    UpdateHere,
    /// Copy each way, the newer file wins; equal times with different
    /// sizes are a problem for the owner, never a guess; nothing is
    /// removed.
    BothWays,
    /// Make the other panel the same as this one: copy what differs, and
    /// remove there what is not here, every removal listed.
    MirrorThere,
    /// Make this panel the same as the other one, every removal listed.
    MirrorHere,
}

impl SyncMode {
    /// Every mode, the default first.
    pub const ALL: [SyncMode; 5] = [
        SyncMode::UpdateThere,
        SyncMode::UpdateHere,
        SyncMode::BothWays,
        SyncMode::MirrorThere,
        SyncMode::MirrorHere,
    ];

    /// The mode in words, for the chooser and the plan's first line:
    /// "Update the other panel", "Both ways, newer wins", "Mirror to this
    /// panel".
    pub fn words(self) -> String {
        match self {
            SyncMode::UpdateThere => format!("Update {}", panel(Side::Right)),
            SyncMode::UpdateHere => format!("Update {}", panel(Side::Left)),
            SyncMode::BothWays => "Both ways, newer wins".to_owned(),
            SyncMode::MirrorThere => format!("Mirror to {}", panel(Side::Right)),
            SyncMode::MirrorHere => format!("Mirror to {}", panel(Side::Left)),
        }
    }

    /// Whether the mode removes.
    pub const fn removes(self) -> bool {
        matches!(self, SyncMode::MirrorThere | SyncMode::MirrorHere)
    }

    /// The action this mode suggests for a row.
    pub fn suggest(self, row: &CompareRow) -> SyncAction {
        use RowState as S;
        let state = row.state;
        if !matches!(state, S::Different | S::OnlyLeft | S::OnlyRight) {
            return SyncAction::Skip;
        }
        match self {
            SyncMode::UpdateThere | SyncMode::UpdateHere => {
                let to = if self == SyncMode::UpdateThere {
                    Side::Right
                } else {
                    Side::Left
                };
                let from = to.other();
                if state == RowState::only(from) || row.newer() == Some(from) {
                    SyncAction::CopyTo(to)
                } else {
                    SyncAction::Skip
                }
            }
            SyncMode::MirrorThere | SyncMode::MirrorHere => {
                let to = if self == SyncMode::MirrorThere {
                    Side::Right
                } else {
                    Side::Left
                };
                if state == RowState::only(to) {
                    SyncAction::RemoveFrom(to)
                } else {
                    SyncAction::CopyTo(to)
                }
            }
            SyncMode::BothWays => match (state, row.newer()) {
                (S::OnlyLeft, _) => SyncAction::CopyTo(Side::Right),
                (S::OnlyRight, _) => SyncAction::CopyTo(Side::Left),
                (_, Some(newer)) => SyncAction::CopyTo(newer.other()),
                (_, None) => SyncAction::Skip,
            },
        }
    }
}

/// What a synchronize does with one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncAction {
    /// Copy the entry to this side, replacing the one there: the engine
    /// sends the old one to the trash first where the side has one.
    CopyTo(Side),
    /// Remove the entry from this side: to the trash where the side has
    /// one, else for good.
    RemoveFrom(Side),
    /// Nothing.
    Skip,
}

/// One row of a plan: the compare's row, the action and whether the
/// owner left it out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncRow {
    /// The compare's row.
    pub row: CompareRow,
    /// What will be done.
    pub action: SyncAction,
    /// What the mode suggested, for a reset.
    pub suggested: SyncAction,
    /// Whether the row takes part; Space toggles it.
    pub included: bool,
    /// The row of the folder it is in, when that folder has a row.
    pub parent: Option<usize>,
}

/// Why a row is a problem: said first, never acted on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SyncProblem {
    /// A file on one side, a folder or a link on the other; or two names
    /// that differ only by case.
    TypeDiffers,
    /// A side could not be read, with the reason in words.
    Unreadable(Option<Side>, String),
    /// The compare was cancelled before it reached the row.
    NotCompared,
    /// The two differ and neither is newer: the owner chooses a
    /// direction.
    NeedsDirection,
    /// The side the action writes to cannot be written.
    ReadOnly(Side),
    /// The folder it would be copied into is not made: left out, or a
    /// problem itself.
    FolderLeftOut,
}

/// Where a row stands in the plan.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowStatus {
    /// A problem: skipped, said first.
    Problem(SyncProblem),
    /// Acts, and cannot be undone (no trash there) or replaces a newer
    /// file.
    Caution,
    /// Acts.
    Acts,
    /// Would act, and the owner left it out.
    LeftOut,
    /// A folder whose removal keeps it, because something inside is left
    /// out: what is included inside still goes.
    Kept,
    /// Nothing to do, and not the same: newer on the side it would go
    /// to, only on the side an update keeps, or skipped by the filter.
    Skipped,
    /// The same on both sides; every mode skips it.
    Same,
}

/// A synchronize plan, from a compare and a mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPlan {
    /// The mode.
    pub mode: SyncMode,
    /// The place here (the left root).
    pub left: Place,
    /// The place there (the right root).
    pub right: Place,
    /// The facts here, as the compare read them.
    pub left_facts: SideFacts,
    /// The facts there.
    pub right_facts: SideFacts,
    /// How the compare verified its rows, said with the plan: "Compared by
    /// size and time; times within 2 seconds count as the same."
    pub verified: String,
    /// Whether the compare was cancelled before its end.
    pub partial: bool,
    /// The rows, in the compare's order.
    pub rows: Vec<SyncRow>,
    /// What the compare left out, with the reason in words.
    pub left_out: Vec<(Place, String)>,
}

/// Turns a compare into a plan: each row the action its mode suggests,
/// every row included. Problems are skipped and said first.
pub fn plan_sync(result: &Comparison, mode: SyncMode) -> SyncPlan {
    let mut stack: Vec<usize> = Vec::new();
    let mut rows = Vec::with_capacity(result.rows.len());
    for (index, row) in result.rows.iter().enumerate() {
        while let Some(&top) = stack.last() {
            if row.is_under(&result.rows[top].names) {
                break;
            }
            stack.pop();
        }
        let parent = stack
            .last()
            .copied()
            .filter(|&top| result.rows[top].names.len() + 1 == row.names.len());
        let action = mode.suggest(row);
        rows.push(SyncRow {
            row: row.clone(),
            action,
            suggested: action,
            included: true,
            parent,
        });
        if row.is_folder() {
            stack.push(index);
        }
    }
    let verified = match result.criteria.by {
        CompareBy::Content => "Verified by content where the sizes match.".to_owned(),
        by => by.line(result.criteria.tolerance),
    };
    SyncPlan {
        mode,
        left: result.left.clone(),
        right: result.right.clone(),
        left_facts: result.left_facts.clone(),
        right_facts: result.right_facts.clone(),
        verified,
        partial: result.cancelled,
        rows,
        left_out: result.left_out.clone(),
    }
}

impl SyncPlan {
    /// The facts of one side.
    pub fn facts(&self, side: Side) -> &SideFacts {
        match side {
            Side::Left => &self.left_facts,
            Side::Right => &self.right_facts,
        }
    }

    /// The root of one side.
    pub fn root(&self, side: Side) -> &Place {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    /// The place of a row on one side.
    pub fn place_of(&self, index: usize, side: Side) -> Result<Place, OpsError> {
        place_at(self.root(side), &self.row(index)?.row.names)
    }

    fn row(&self, index: usize) -> Result<&SyncRow, OpsError> {
        self.rows
            .get(index)
            .ok_or_else(|| OpsError::Refused(format!("There is no row {}", index + 1)))
    }

    /// The rows inside a row, by index: those after it whose names start
    /// with its names.
    fn inside(&self, index: usize) -> std::ops::Range<usize> {
        let names = &self.rows[index].row.names;
        let end = self.rows[index + 1..]
            .iter()
            .position(|row| !row.row.is_under(names))
            .map_or(self.rows.len(), |at| index + 1 + at);
        index + 1..end
    }

    /// The problem a row has, before what its folder does is looked at.
    fn own_problem(&self, row: &SyncRow) -> Option<SyncProblem> {
        match row.row.state {
            RowState::TypeDiffers => return Some(SyncProblem::TypeDiffers),
            RowState::NotCompared => return Some(SyncProblem::NotCompared),
            RowState::Unreadable => {
                let why = row.row.reasons().join(", ");
                return Some(SyncProblem::Unreadable(row.row.unreadable_side(), why));
            }
            _ => {}
        }
        if row.action == SyncAction::Skip
            && row.row.state == RowState::Different
            && row.row.newer().is_none()
        {
            return Some(SyncProblem::NeedsDirection);
        }
        match row.action {
            SyncAction::CopyTo(side) | SyncAction::RemoveFrom(side)
                if !self.facts(side).writable =>
            {
                Some(SyncProblem::ReadOnly(side))
            }
            _ => None,
        }
    }

    /// Whether a row acts: included, an action, and no problem of its own
    /// or from its folder.
    pub fn acts(&self, index: usize) -> bool {
        matches!(
            self.status(index),
            Some(RowStatus::Acts | RowStatus::Caution)
        )
    }

    /// Where a row stands; `None` for a row that is not there.
    pub fn status(&self, index: usize) -> Option<RowStatus> {
        let row = self.rows.get(index)?;
        if let Some(problem) = self.own_problem(row) {
            return Some(RowStatus::Problem(problem));
        }
        match row.action {
            SyncAction::Skip => Some(if row.row.state == RowState::Same {
                RowStatus::Same
            } else {
                RowStatus::Skipped
            }),
            _ if !row.included => Some(RowStatus::LeftOut),
            SyncAction::CopyTo(to) => {
                if self.folder_missing(index, to) {
                    return Some(RowStatus::Problem(SyncProblem::FolderLeftOut));
                }
                let replaces = row.row.entry(to).is_some();
                let newer_there = row.row.newer() == Some(to);
                if newer_there || (replaces && !self.facts(to).trash) {
                    Some(RowStatus::Caution)
                } else {
                    Some(RowStatus::Acts)
                }
            }
            SyncAction::RemoveFrom(from) => {
                if row.row.is_folder() && !self.whole(index) {
                    return Some(RowStatus::Kept);
                }
                if self.facts(from).trash {
                    Some(RowStatus::Acts)
                } else {
                    Some(RowStatus::Caution)
                }
            }
        }
    }

    /// The status's place in the listing: problems first, then the
    /// removals (plan section 5.17: listed first), then what cannot be
    /// undone or replaces a newer file, the other copies, kept folders,
    /// what the owner left out, the skips, the same.
    fn rank(&self, index: usize) -> Option<u8> {
        let status = self.status(index)?;
        let removal = matches!(self.rows[index].action, SyncAction::RemoveFrom(_));
        Some(match status {
            RowStatus::Problem(_) => 0,
            RowStatus::Acts | RowStatus::Caution if removal => 1,
            RowStatus::Caution => 2,
            RowStatus::Acts => 3,
            RowStatus::Kept => 4,
            RowStatus::LeftOut => 5,
            RowStatus::Skipped => 6,
            RowStatus::Same => 7,
        })
    }

    /// Whether a copy to `to` lands in a folder that will not be there:
    /// a folder above it missing on that side whose own copy does not act.
    fn folder_missing(&self, index: usize, to: Side) -> bool {
        let mut parent = self.rows[index].parent;
        while let Some(at) = parent {
            let folder = &self.rows[at];
            if folder.row.entry(to).is_some() {
                return false;
            }
            let makes = folder.included
                && folder.action == SyncAction::CopyTo(to)
                && self.own_problem(folder).is_none();
            if !makes {
                return true;
            }
            parent = folder.parent;
        }
        false
    }

    /// Whether a folder's removal takes all of it: every row inside is an
    /// included removal from the same side with no problem.
    fn whole(&self, index: usize) -> bool {
        let action = self.rows[index].action;
        self.inside(index).all(|at| {
            let row = &self.rows[at];
            row.included && row.action == action && self.own_problem(row).is_none()
        })
    }

    /// Space: includes or leaves out a row, and everything inside it that
    /// has an action. Returns the row's line again, to say. Refused for a
    /// row with nothing to do or a problem.
    pub fn toggle(&mut self, index: usize) -> Result<String, OpsError> {
        let row = self.row(index)?;
        let place = row.row.where_words(separator_of(&self.left));
        if let Some(problem) = self.own_problem(row) {
            let why = match problem {
                SyncProblem::NeedsDirection => "choose a direction first".to_owned(),
                SyncProblem::TypeDiffers => "a type that differs is never resolved".to_owned(),
                SyncProblem::ReadOnly(side) => format!("{} is read-only", panel(side)),
                SyncProblem::NotCompared => "it was not compared".to_owned(),
                SyncProblem::Unreadable(..) | SyncProblem::FolderLeftOut => {
                    "it could not be read".to_owned()
                }
            };
            return Err(OpsError::Refused(format!("Cannot include {place}: {why}")));
        }
        if row.action == SyncAction::Skip {
            return Err(OpsError::Refused(format!("Nothing to do: {place}")));
        }
        let included = !row.included;
        for at in std::iter::once(index).chain(self.inside(index)) {
            if self.rows[at].action != SyncAction::Skip {
                self.rows[at].included = included;
            }
        }
        Ok(self.line(index))
    }

    /// The direction keys ("copy to other", "copy to this"): copy this
    /// row to `to` from the other side, and everything inside it that is
    /// on that side. Returns the line to say. Refused for a problem of the
    /// compare, a folder on both sides (its files have their own rows),
    /// an entry not on the other side, or a side that cannot be written.
    pub fn set_direction(&mut self, index: usize, to: Side) -> Result<String, OpsError> {
        let row = self.row(index)?;
        let from = to.other();
        let place = row.row.where_words(separator_of(&self.left));
        if row.row.state.is_problem() {
            return Err(OpsError::Refused(format!(
                "Cannot copy {place}: {}",
                row.row.state.words().to_lowercase()
            )));
        }
        if row.row.entry(from).is_none() {
            return Err(OpsError::Refused(format!(
                "Cannot copy {place} {}: it is not {}",
                at(to),
                at(from)
            )));
        }
        if row.row.is_folder() && row.row.left.is_some() && row.row.right.is_some() {
            return Err(OpsError::Refused(format!(
                "Choose the direction for what is inside {place}"
            )));
        }
        if !self.facts(to).writable {
            return Err(OpsError::Refused(format!(
                "Cannot copy {place}: {} is read-only",
                panel(to)
            )));
        }
        for at in std::iter::once(index).chain(self.inside(index)) {
            let row = &mut self.rows[at];
            let movable = row.row.entry(from).is_some()
                && !row.row.state.is_problem()
                && !matches!(row.row.state, RowState::Same | RowState::Skipped);
            if movable || at == index {
                row.action = SyncAction::CopyTo(to);
                row.included = true;
            }
        }
        Ok(self.line(index))
    }

    /// Puts a row, and what is inside it, back to what the mode
    /// suggested, included. Returns the line to say.
    pub fn reset(&mut self, index: usize) -> Result<String, OpsError> {
        self.row(index)?;
        for at in std::iter::once(index).chain(self.inside(index)) {
            let row = &mut self.rows[at];
            row.action = row.suggested;
            row.included = true;
        }
        Ok(self.line(index))
    }

    /// One row in words, its action first, in the short form: "Copy
    /// there: plan.md, only here"; "Replace there: notes.txt, newer
    /// here"; "Trash there: old.txt, only there"; "Delete for good there:
    /// old.txt, no trash there"; "Type differs: photos, folder here, file
    /// there"; "Left
    /// out: report.pdf, would copy there".
    pub fn line(&self, index: usize) -> String {
        let (Some(row), Some(status)) = (self.rows.get(index), self.status(index)) else {
            return String::new();
        };
        let place = row.row.where_words(separator_of(&self.left));
        let reasons = row.row.reasons();
        let with = |head: String, reasons: &[String]| {
            if reasons.is_empty() {
                head
            } else {
                format!("{head}, {}", reasons.join(", "))
            }
        };
        match status {
            RowStatus::Problem(problem) => match problem {
                SyncProblem::TypeDiffers => with(format!("Type differs: {place}"), &reasons),
                SyncProblem::Unreadable(side, why) => {
                    let side = side.map(|side| format!(" {}", at(side)));
                    format!("Unreadable{}: {place}, {why}", side.unwrap_or_default())
                }
                SyncProblem::NotCompared => format!("Not compared: {place}"),
                SyncProblem::NeedsDirection => {
                    with(format!("Choose a direction: {place}"), &reasons)
                }
                SyncProblem::ReadOnly(side) => format!("Read-only {}: {place}", at(side)),
                SyncProblem::FolderLeftOut => format!("Folder left out: {place}"),
            },
            RowStatus::LeftOut => format!("Left out: {place}, would {}", self.verb(row)),
            RowStatus::Kept => format!(
                "Kept: {place}, something inside is left out; would {}",
                self.verb(row)
            ),
            RowStatus::Same => with(format!("Same: {place}"), &reasons),
            RowStatus::Skipped => {
                let why = match row.row.state {
                    RowState::Different => row
                        .row
                        .newer()
                        .map_or_else(|| reasons.join(", "), |side| format!("newer {}", at(side))),
                    RowState::OnlyLeft => format!("only {}, kept", at(Side::Left)),
                    RowState::OnlyRight => format!("only {}, kept", at(Side::Right)),
                    _ => reasons.join(", "),
                };
                format!("Skip: {place}, {why}")
            }
            RowStatus::Acts | RowStatus::Caution => match row.action {
                // A file there already: the short words of question 135,
                // "Replace there: notes.txt, newer here" (lane AS).
                SyncAction::CopyTo(to) if row.row.entry(to).is_some() && !row.row.is_folder() => {
                    let mut head = if row.row.newer() == Some(to) {
                        "Replace newer".to_owned()
                    } else {
                        "Replace".to_owned()
                    };
                    if !self.facts(to).trash {
                        head.push_str(" for good");
                    }
                    let mut parts = vec![format!("{head} {}: {place}", at(to))];
                    parts.extend(reasons);
                    if !self.facts(to).trash {
                        parts.push(format!("no trash {}", at(to)));
                    }
                    parts.join(", ")
                }
                SyncAction::CopyTo(to) => {
                    let from = to.other();
                    let parts: Vec<String> = match row.row.state {
                        RowState::OnlyLeft | RowState::OnlyRight if row.row.is_folder() => {
                            vec!["new folder".to_owned()]
                        }
                        RowState::OnlyLeft | RowState::OnlyRight => {
                            vec![format!("only {}", at(from))]
                        }
                        _ if row.row.newer() == Some(to) => {
                            let mut parts = vec![format!("replaces a newer file {}", at(to))];
                            parts.extend(reasons.into_iter().filter(|r| !r.starts_with("newer")));
                            parts
                        }
                        _ => reasons,
                    };
                    with(format!("Copy {}: {place}", at(to)), &parts)
                }
                SyncAction::RemoveFrom(from) if self.facts(from).trash => {
                    format!("Trash {}: {place}, only {}", at(from), at(from))
                }
                SyncAction::RemoveFrom(from) => {
                    format!(
                        "Delete for good {}: {place}, no trash {}",
                        at(from),
                        at(from)
                    )
                }
                SyncAction::Skip => format!("Skip: {place}"),
            },
        }
    }

    /// The action as a phrase, lowercase: "copy there", "be removed there",
    /// "be removed there for good".
    fn verb(&self, row: &SyncRow) -> String {
        match row.action {
            SyncAction::CopyTo(to) => format!("copy {}", at(to)),
            SyncAction::RemoveFrom(from) if self.facts(from).trash => {
                format!("be removed {}", at(from))
            }
            SyncAction::RemoveFrom(from) => format!("be removed {} for good", at(from)),
            SyncAction::Skip => "skip".to_owned(),
        }
    }

    /// How many rows stand where, and the bytes to copy.
    pub fn counts(&self) -> SyncCounts {
        let mut counts = SyncCounts::default();
        for index in 0..self.rows.len() {
            let row = &self.rows[index];
            let Some(status) = self.status(index) else {
                continue;
            };
            counts.all += 1;
            match status {
                RowStatus::Problem(_) => counts.problems += 1,
                RowStatus::LeftOut => counts.left_out += 1,
                RowStatus::Kept => counts.kept += 1,
                RowStatus::Skipped => counts.skipped += 1,
                RowStatus::Same => counts.same += 1,
                RowStatus::Acts | RowStatus::Caution => match row.action {
                    SyncAction::CopyTo(to) => {
                        counts.copies[to.index()] += 1;
                        if row.row.kind() != EntryKind::Folder {
                            let from = to.other();
                            counts.bytes += row.row.entry(from).and_then(|e| e.size).unwrap_or(0);
                        }
                        if row.row.entry(to).is_some()
                            && !row.row.is_folder()
                            && !self.facts(to).trash
                        {
                            counts.for_good += 1;
                        }
                    }
                    SyncAction::RemoveFrom(from) => {
                        if self.facts(from).trash {
                            counts.trashed += 1;
                        } else {
                            counts.removed_for_good += 1;
                            counts.for_good += 1;
                        }
                    }
                    SyncAction::Skip => {}
                },
            }
        }
        counts
    }

    /// The plan in one line, meaning first, problems first, adding up to
    /// the rows: "Update the other panel: 2 problems, 4 to copy there, 2
    /// skipped, 1 same; 5 bytes to copy"; "Mirror to the other panel:
    /// nothing to do, 214 same".
    pub fn summary(&self) -> String {
        let counts = self.counts();
        let mut parts = Vec::new();
        if counts.problems > 0 {
            parts.push(count(counts.problems, "problem", "problems"));
        }
        for side in [Side::Right, Side::Left] {
            if counts.trashed > 0 && self.removes_from(side) && self.facts(side).trash {
                parts.push(format!(
                    "{} to remove {}",
                    grouped(counts.trashed),
                    at(side)
                ));
            }
            if counts.removed_for_good > 0 && self.removes_from(side) && !self.facts(side).trash {
                parts.push(format!(
                    "{} to remove {} for good",
                    grouped(counts.removed_for_good),
                    at(side)
                ));
            }
        }
        for to in [Side::Right, Side::Left] {
            let n = counts.copies[to.index()];
            if n > 0 {
                parts.push(format!("{} to copy {}", grouped(n), at(to)));
            }
        }
        if counts.acting() == 0 {
            parts.push("nothing to do".to_owned());
        }
        if counts.kept > 0 {
            parts.push(format!("{} kept", grouped(counts.kept)));
        }
        if counts.left_out > 0 {
            parts.push(format!("{} left out", grouped(counts.left_out)));
        }
        if counts.skipped > 0 {
            parts.push(format!("{} skipped", grouped(counts.skipped)));
        }
        if counts.same > 0 {
            parts.push(format!("{} same", grouped(counts.same)));
        }
        let mut line = format!("{}: {}", self.mode.words(), parts.join(", "));
        if counts.bytes > 0 {
            line.push_str(&format!("; {} to copy", human_size(counts.bytes)));
        }
        line
    }

    /// Whether any acting row removes from this side.
    fn removes_from(&self, side: Side) -> bool {
        (0..self.rows.len()).any(|index| {
            self.rows[index].action == SyncAction::RemoveFrom(side) && self.acts(index)
        })
    }

    /// The lines that go with the summary: a partial compare, what cannot
    /// be undone, how the rows were verified.
    pub fn notes(&self) -> Vec<String> {
        let counts = self.counts();
        let mut notes = Vec::new();
        if self.partial {
            notes.push("The compare was cancelled: this plan is partial.".to_owned());
        }
        if counts.for_good > 0 {
            notes.push(format!(
                "{} cannot be undone: no trash there.",
                count(counts.for_good, "change", "changes")
            ));
        }
        notes.push(self.verified.clone());
        notes
    }

    /// The row indexes in the order the listing says them
    /// ([`RowStatus`]): problems first, then the removals, the cautions,
    /// the copies, the kept folders, what the owner left out, the skips;
    /// the same rows last, only when `same` is set. Each group keeps the
    /// compare's order.
    pub fn order(&self, same: bool) -> Vec<usize> {
        let mut order: Vec<(u8, usize)> = (0..self.rows.len())
            .filter_map(|index| self.rank(index).map(|rank| (rank, index)))
            .filter(|(rank, _)| same || *rank != 7)
            .collect();
        order.sort();
        order.into_iter().map(|(_, index)| index).collect()
    }

    /// The plan as plain lines for a dry run (`diro sync --dry-run`) or a
    /// Braille display: the summary, its notes, the rows problems first
    /// ([`SyncPlan::order`]), then what the compare left out.
    pub fn lines(&self, same: bool) -> Vec<String> {
        let mut lines = vec![self.summary()];
        lines.extend(self.notes());
        lines.extend(self.order(same).into_iter().map(|index| self.line(index)));
        lines.extend(
            self.left_out.iter().map(|(place, why)| {
                format!("Not compared: {}, {why}", spell_out(&place.to_string()))
            }),
        );
        lines
    }

    /// The steps the plan runs, each with its row, in order: the copies
    /// there, the copies here, each folder before what is inside it; then
    /// the removals, a whole folder to the trash in one step, or entry by
    /// entry, deepest first, where it goes for good.
    pub fn steps(&self) -> Result<Vec<(usize, Step)>, OpsError> {
        let mut steps = Vec::new();
        for to in [Side::Right, Side::Left] {
            for index in 0..self.rows.len() {
                if self.rows[index].action == SyncAction::CopyTo(to) && self.acts(index) {
                    steps.push((
                        index,
                        Step::Copy {
                            from: self.place_of(index, to.other())?,
                            to: self.place_of(index, to)?,
                        },
                    ));
                }
            }
        }
        for from in [Side::Right, Side::Left] {
            let trash = self.facts(from).trash;
            let mut removes = Vec::new();
            let mut covered: Option<usize> = None;
            for index in 0..self.rows.len() {
                if let Some(folder) = covered {
                    if self.rows[index].row.is_under(&self.rows[folder].row.names) {
                        continue;
                    }
                    covered = None;
                }
                if self.rows[index].action != SyncAction::RemoveFrom(from) || !self.acts(index) {
                    continue;
                }
                let place = self.place_of(index, from)?;
                if trash {
                    if self.rows[index].row.is_folder() {
                        covered = Some(index);
                    }
                    steps.push((index, Step::Trash { place }));
                } else {
                    removes.push((index, Step::Remove { place }));
                }
            }
            // For good, entry by entry: what is inside first.
            removes.reverse();
            steps.extend(removes);
        }
        Ok(steps)
    }

    /// The plan as one job of the engine: its steps, the entries the
    /// compare saw at each step's subject (the engine checks them again
    /// before each step), the counts, and the summary as its note. A
    /// replace is the engine's overwrite, the old file to the trash first
    /// where the place has one. The job is a [`JobKind::Synchronize`]: the
    /// hooks hear its copies as a copy and its removals as a delete
    /// ([`crate::hooks::HookCall::all`]).
    pub fn job_plan(&self, job: JobId) -> Result<Plan, OpsError> {
        Ok(self.plan_of(&self.steps()?, job))
    }

    /// The engine's plan for some of this plan's steps, each with its row.
    pub(crate) fn plan_of(&self, steps: &[(usize, Step)], job: JobId) -> Plan {
        let mut plan = Plan::new(
            job,
            JobKind::Synchronize,
            steps.iter().map(|(_, step)| step.clone()).collect(),
            ConflictPolicy::Overwrite,
        );
        for (index, step) in steps {
            let side = match self.rows[*index].action {
                SyncAction::CopyTo(to) => to.other(),
                SyncAction::RemoveFrom(from) => from,
                SyncAction::Skip => continue,
            };
            let Some(entry) = self.rows[*index].row.entry(side) else {
                continue;
            };
            let place = step.subject().clone();
            match step {
                Step::Copy { .. } => plan.facts.count(&place, entry),
                _ if entry.kind == EntryKind::Folder => plan.facts.folders += 1,
                _ if entry.kind == EntryKind::Link => plan.facts.links += 1,
                _ => plan.facts.files += 1,
            }
            plan.expected.insert(place, entry.clone());
        }
        plan.facts.counted = true;
        plan.facts.left_out = self.left_out.clone();
        plan.note = Some(self.summary());
        plan
    }
}

/// How many rows of a plan stand where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncCounts {
    /// Every row.
    pub all: u64,
    /// Problems.
    pub problems: u64,
    /// Copies to each side, by [`Side::index`].
    pub copies: [u64; 2],
    /// Removals to the trash.
    pub trashed: u64,
    /// Removals for good, where a side has no trash.
    pub removed_for_good: u64,
    /// Changes that cannot be undone: removals for good and replaces where
    /// a side has no trash. Counted again here, beside their own counts.
    pub for_good: u64,
    /// Folders kept because something inside is left out.
    pub kept: u64,
    /// Rows the owner left out.
    pub left_out: u64,
    /// Rows with nothing to do that are not the same.
    pub skipped: u64,
    /// Rows the same on both sides.
    pub same: u64,
    /// The bytes the copies read.
    pub bytes: u64,
}

impl SyncCounts {
    /// The rows that act: copies and removals.
    pub const fn acting(&self) -> u64 {
        self.copies[0] + self.copies[1] + self.trashed + self.removed_for_good
    }

    /// The categories of the summary, which add up to [`SyncCounts::all`].
    pub const fn sum(&self) -> u64 {
        self.problems + self.acting() + self.kept + self.left_out + self.skipped + self.same
    }
}
