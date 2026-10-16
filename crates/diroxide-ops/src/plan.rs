//! Plans: what a job will do, computed before anything is touched.
//!
//! A plan is a prediction (R14): the steps in order, the entry the planner
//! saw at each step's subject (re-checked by identity before the step
//! runs), the counts and totals in [`PlanFacts`], what was left out and
//! why, and the questions to ask before the start ([`Preflight`]). The
//! listing spells hidden characters out as `[U+XXXX]` and says counts in
//! words, so a confirmation shows exactly what will happen.

use std::collections::HashMap;

use diroxide_core::catalog;
use diroxide_core::format::{count, human_size};
use diroxide_core::names::spell_out;
use diroxide_core::{
    Attributes, ConflictPolicy, Entry, EntryKind, JobId, JobKind, JobOrigin, Message, Place,
};
use diroxide_vfs::{AttributeChange, LinkRequest, TrashGate};

/// One step of a plan, acting on one entry.
///
/// A copy step on a folder makes the folder at its target, or merges into
/// the folder already there; what is inside it has steps of its own. A
/// move step on a folder moves the whole folder (a rename where it can,
/// else its contents one by one); a move across drives is planned entry by
/// entry instead, as a copy step for each folder, a move step for each
/// file and a removal of each emptied folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Copy one entry.
    Copy {
        /// The source.
        from: Place,
        /// The target, including its name.
        to: Place,
    },
    /// Move one entry.
    Move {
        /// The source.
        from: Place,
        /// The target, including its name.
        to: Place,
    },
    /// Give one entry a new name in its folder.
    Rename {
        /// The entry.
        from: Place,
        /// The same folder, the new name.
        to: Place,
    },
    /// A rename through a temporary name, for a case-only change on a
    /// place that would refuse it in one step, or for two names that swap
    /// (R10): the entry goes to `via`, then to `to`.
    RenameHop {
        /// The entry.
        from: Place,
        /// The temporary place, an unfinished name in the same folder.
        via: Place,
        /// The final place.
        to: Place,
    },
    /// Make one folder.
    MakeFolder {
        /// The new folder.
        place: Place,
    },
    /// Make a link.
    Link {
        /// Where the link goes.
        place: Place,
        /// Its kind and target.
        request: LinkRequest,
    },
    /// Move one entry to the trash.
    Trash {
        /// The entry.
        place: Place,
    },
    /// Bring one entry back from the trash.
    Restore {
        /// Where it was before it was trashed.
        place: Place,
    },
    /// Delete one entry for good.
    Remove {
        /// The entry.
        place: Place,
    },
    /// Remove a folder the job made, if it is still empty (undo of a make
    /// folder, deepest first).
    RemoveEmptyFolder {
        /// The folder.
        place: Place,
    },
    /// Change attributes or times.
    SetAttributes {
        /// The entry.
        place: Place,
        /// What changes.
        change: AttributeChange,
    },
}

impl Step {
    /// The place the step acts on first: the source, or the place itself.
    pub fn subject(&self) -> &Place {
        match self {
            Step::Copy { from, .. }
            | Step::Move { from, .. }
            | Step::Rename { from, .. }
            | Step::RenameHop { from, .. } => from,
            Step::MakeFolder { place }
            | Step::Link { place, .. }
            | Step::Trash { place }
            | Step::Restore { place }
            | Step::Remove { place }
            | Step::RemoveEmptyFolder { place }
            | Step::SetAttributes { place, .. } => place,
        }
    }

    /// The place the step makes or changes at its end, for the
    /// reconciliation: the target of a copy, move or rename, a new folder
    /// or link. `None` for steps that remove.
    pub fn result(&self) -> Option<&Place> {
        match self {
            Step::Copy { to, .. }
            | Step::Move { to, .. }
            | Step::Rename { to, .. }
            | Step::RenameHop { to, .. } => Some(to),
            Step::MakeFolder { place } | Step::Link { place, .. } | Step::Restore { place } => {
                Some(place)
            }
            Step::SetAttributes { place, .. } => Some(place),
            Step::Trash { .. } | Step::Remove { .. } | Step::RemoveEmptyFolder { .. } => None,
        }
    }

    /// Whether the step takes something away from where it is: a move,
    /// the trash, a removal. These re-check the entry most strictly.
    pub fn removes(&self) -> bool {
        matches!(
            self,
            Step::Move { .. }
                | Step::Trash { .. }
                | Step::Remove { .. }
                | Step::RemoveEmptyFolder { .. }
        )
    }

    /// The step in words, with hidden characters spelled out: "copy
    /// D:\docs\a.txt to D:\backup\a.txt".
    pub fn describe(&self) -> String {
        let spell = |place: &Place| spell_out(&place.to_string()).into_owned();
        match self {
            Step::Copy { from, to } => format!("copy {} to {}", spell(from), spell(to)),
            Step::Move { from, to } => format!("move {} to {}", spell(from), spell(to)),
            Step::Rename { from, to } => format!("rename {} to {}", spell(from), spell(to)),
            Step::RenameHop { from, via, to } => format!(
                "rename {} to {}, through the temporary name {}",
                spell(from),
                spell(to),
                via.name()
                    .map_or_else(String::new, |n| spell_out(n).into_owned())
            ),
            Step::MakeFolder { place } => format!("make the folder {}", spell(place)),
            Step::Link { place, request } => format!(
                "make a link {} to {}",
                spell(place),
                spell_out(&request.target)
            ),
            Step::Trash { place } => format!("move {} to the trash", spell(place)),
            Step::Restore { place } => format!("restore {} from the trash", spell(place)),
            Step::Remove { place } => format!("delete {} for good", spell(place)),
            Step::RemoveEmptyFolder { place } => {
                format!("remove the empty folder {}", spell(place))
            }
            Step::SetAttributes { place, .. } => {
                format!("change the attributes of {}", spell(place))
            }
        }
    }
}

/// A question the plan asks before the job starts, decided by the user.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Preflight {
    /// The destination has less room than the job needs. Going on is
    /// allowed: compression, clones and sparse files can need less.
    NotEnoughRoom {
        /// The destination's volume.
        drive: Place,
        /// The bytes the job writes.
        needed: u64,
        /// The bytes free.
        free: u64,
    },
    /// A file is larger than the destination's largest file.
    TooLarge {
        /// The file.
        place: Place,
        /// Its size.
        size: u64,
        /// The largest file the destination takes.
        max: u64,
    },
    /// A target name the destination refuses (a reserved name on a
    /// Windows-rules place, a name too long): rename it or skip it, never
    /// renamed silently.
    NameRefused {
        /// The target.
        place: Place,
        /// Why, in words: "Name needs care: is a Windows device".
        line: String,
    },
    /// The trash cannot take these items: the place offers the listed
    /// permanent delete instead, as its own job.
    NoTrash {
        /// Why.
        gate: TrashGate,
        /// The items left out of the trash job.
        places: Vec<Place>,
    },
}

impl Preflight {
    /// The question's lines, the fact first, the numbers second, the
    /// question last, ending "y or n" or naming its choices. The answer
    /// starts on No.
    pub fn lines(&self) -> Vec<String> {
        let name = |place: &Place| {
            place
                .name()
                .map_or_else(|| place.to_string(), |n| spell_out(n).into_owned())
        };
        match self {
            Preflight::NotEnoughRoom {
                drive,
                needed,
                free,
            } => vec![
                format!("Not enough room on {}", spell_out(&drive.to_string())),
                format!("Needs {}, {} free.", human_size(*needed), human_size(*free)),
                Message::question("Go on anyway").text,
            ],
            Preflight::TooLarge { place, size, max } => vec![
                format!("Too large for this drive: {}", name(place)),
                format!(
                    "It is {}; the drive takes up to {}.",
                    human_size(*size),
                    human_size(*max)
                ),
                Message::question("Skip it").text,
            ],
            Preflight::NameRefused { place, line } => vec![
                format!("Name not allowed there: {}", name(place)),
                line.clone(),
                "Rename it, or skip it.".to_owned(),
            ],
            Preflight::NoTrash { gate, places } => {
                let items = places.len() as u64;
                let mut lines = vec![gate.why()];
                if let Some(question) = gate.question(items) {
                    lines.insert(0, question.text);
                }
                lines
            }
        }
    }
}

/// What the planner found: counts, totals, the unusual things, what was
/// left out, and the questions before the start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanFacts {
    /// The items selected.
    pub items: u64,
    /// Files the job touches, inside folders too.
    pub files: u64,
    /// Folders the job touches, the selected ones included.
    pub folders: u64,
    /// Links (acted on as links, never followed).
    pub links: u64,
    /// The bytes of the files.
    pub bytes: u64,
    /// Whether the bytes were counted (a move by rename does not count).
    pub counted: bool,
    /// Entries marked read-only.
    pub read_only: u64,
    /// Names with hidden characters, spelled out in the listing.
    pub hidden_names: u64,
    /// Files with other hard links.
    pub hard_linked: u64,
    /// Files with holes: their length is more than their disk use.
    pub sparse: u64,
    /// Files that are only online (cloud placeholders); reading them
    /// downloads them.
    pub online_only: u64,
    /// The largest file and its size.
    pub largest: Option<(Place, u64)>,
    /// Where results go.
    pub destination: Option<Place>,
    /// What the plan leaves out, each with its reason in words; the run
    /// reports them as skipped.
    pub left_out: Vec<(Place, String)>,
    /// What to ask before the start.
    pub questions: Vec<Preflight>,
    /// Text the user types to confirm, instead of y: the count of a large
    /// permanent delete, or the name of a protected folder.
    pub typed: Option<String>,
    /// Step ranges, start and end, that run whole: no pause or cancel
    /// between their steps (a swap of names through a temporary name).
    pub whole: Vec<(usize, usize)>,
    /// Who asked for the job, for the journal and the undo record.
    pub origin: JobOrigin,
}

impl PlanFacts {
    /// The entries counted: files, folders and links.
    pub fn entries(&self) -> u64 {
        self.files + self.folders + self.links
    }

    /// Counts one entry the job touches.
    pub(crate) fn count(&mut self, place: &Place, entry: &Entry) {
        match entry.kind {
            EntryKind::Folder => self.folders += 1,
            EntryKind::Link => self.links += 1,
            _ => {
                self.files += 1;
                let size = entry.size.unwrap_or(0);
                self.bytes += size;
                if self.largest.as_ref().is_none_or(|(_, top)| size > *top) {
                    self.largest = Some((place.clone(), size));
                }
                if entry.hard_links.is_some_and(|links| links > 1) {
                    self.hard_linked += 1;
                }
                if is_sparse(entry) {
                    self.sparse += 1;
                }
            }
        }
        if is_read_only(entry) {
            self.read_only += 1;
        }
        if entry.needs_care && diroxide_core::names::has_hidden(&entry.name) {
            self.hidden_names += 1;
        }
        if is_online_only(entry) {
            self.online_only += 1;
        }
    }
}

/// Whether an entry is marked read-only: the Windows attribute, or no
/// write bit for its owner.
pub(crate) fn is_read_only(entry: &Entry) -> bool {
    match entry.attributes {
        Some(Attributes::Windows(flags)) => flags & 0x1 != 0,
        Some(Attributes::Unix(mode)) => mode & 0o200 == 0,
        None => false,
    }
}

fn is_sparse(entry: &Entry) -> bool {
    match (entry.attributes, entry.blocks, entry.size) {
        (Some(Attributes::Windows(flags)), _, _) => flags & 0x200 != 0,
        (_, Some(blocks), Some(size)) => blocks.saturating_mul(512) + 4096 < size,
        _ => false,
    }
}

fn is_online_only(entry: &Entry) -> bool {
    matches!(entry.attributes, Some(Attributes::Windows(flags)) if flags & (0x40000 | 0x400000) != 0)
}

/// A group of the listing: the steps in one folder, under a title that
/// names the folder and the count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingGroup {
    /// "In D:\old, 3 items".
    pub title: String,
    /// The steps, one line each.
    pub lines: Vec<String>,
}

/// What a job will do, step by step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The job.
    pub job: JobId,
    /// What kind of job it is.
    pub kind: JobKind,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// What to do when a name is taken.
    pub conflicts: ConflictPolicy,
    /// A line said before the listing, such as the frozen selection's
    /// "Not the focused row: report.pdf" (plan section 5.2).
    pub note: Option<String>,
    /// The counts, the totals, what was left out and the questions.
    pub facts: PlanFacts,
    /// The entry the planner saw at each step's subject, by place: the run
    /// re-checks it before the step and skips a step whose entry changed.
    /// Empty for a plan made by hand, which is not re-checked.
    pub expected: HashMap<Place, Entry>,
}

impl Plan {
    /// A plan with steps and no note.
    pub fn new(job: JobId, kind: JobKind, steps: Vec<Step>, conflicts: ConflictPolicy) -> Self {
        Self {
            job,
            kind,
            steps,
            conflicts,
            note: None,
            facts: PlanFacts::default(),
            expected: HashMap::new(),
        }
    }

    /// How many entries the plan touches: the counted entries, or the
    /// steps when nothing was counted.
    pub fn entries(&self) -> u64 {
        match self.facts.entries() {
            0 => self.steps.len() as u64,
            n => n,
        }
    }

    /// The plan in one line, meaning first: "Copy 3 items to D:\backup,
    /// 4.0 GB", "Delete 12 items for good, 2.3 GB, 2 read-only", "Move 3
    /// items to the trash".
    pub fn summary(&self) -> String {
        let items = count(self.entries(), "item", "items");
        let destination = self.facts.destination.clone().or_else(|| {
            self.steps.iter().find_map(|step| match step {
                Step::Copy { to, .. } | Step::Move { to, .. } => to.parent(),
                _ => None,
            })
        });
        let to = destination
            .map(|folder| format!(" to {}", spell_out(&folder.to_string())))
            .unwrap_or_default();
        let mut line = match self.kind {
            JobKind::Copy => format!("Copy {items}{to}"),
            JobKind::Move => format!("Move {items}{to}"),
            JobKind::Duplicate => format!("Duplicate {items}"),
            JobKind::Trash => format!("Move {items} to the trash"),
            JobKind::DeletePermanently => format!("Delete {items} for good"),
            JobKind::Rename => format!("Rename {items}"),
            JobKind::MakeFolder => format!(
                "Make {}",
                count(self.steps.len() as u64, "folder", "folders")
            ),
            JobKind::Link => format!(
                "Make {}{to}",
                count(self.steps.len() as u64, "link", "links")
            ),
            JobKind::Undo => format!("Undo {}", count(self.steps.len() as u64, "step", "steps")),
            other => {
                let verb = other.verb();
                let mut first = verb.chars();
                let verb: String = match first.next() {
                    Some(c) => c.to_uppercase().chain(first).collect(),
                    None => String::new(),
                };
                format!("{verb} {items}{to}")
            }
        };
        if self.facts.counted && self.facts.bytes > 0 {
            line.push_str(&format!(", {}", human_size(self.facts.bytes)));
        }
        if self.facts.read_only > 0 && self.kind == JobKind::DeletePermanently {
            line.push_str(&format!(", {} read-only", self.facts.read_only));
        }
        line
    }

    /// The unusual things in the plan, counts only: "Includes 3 links, 1
    /// sparse file, 12 online only." `None` when there are none.
    pub fn unusual(&self) -> Option<String> {
        let facts = &self.facts;
        let mut parts = Vec::new();
        if facts.links > 0 {
            parts.push(count(facts.links, "link", "links"));
        }
        if facts.read_only > 0 && self.kind != JobKind::DeletePermanently {
            parts.push(format!("{} read-only", facts.read_only));
        }
        if facts.hidden_names > 0 {
            parts.push(count(
                facts.hidden_names,
                "name with hidden characters",
                "names with hidden characters",
            ));
        }
        if facts.hard_linked > 0 {
            parts.push(count(
                facts.hard_linked,
                "file with other hard links",
                "files with other hard links",
            ));
        }
        if facts.sparse > 0 {
            parts.push(count(facts.sparse, "sparse file", "sparse files"));
        }
        if facts.online_only > 0 {
            parts.push(format!("{} online only", facts.online_only));
        }
        if !facts.left_out.is_empty() {
            parts.push(format!("{} left out", facts.left_out.len()));
        }
        (!parts.is_empty()).then(|| format!("Includes {}.", parts.join(", ")))
    }

    /// The confirmation's question, ending "y or n" (or the typed text),
    /// the answer starting on No: "Delete 3 items for good? y or n".
    pub fn question(&self) -> Message {
        let items = self.entries();
        if let Some(typed) = &self.facts.typed {
            return Message::new(
                format!("Type {typed} to go on, or Escape"),
                diroxide_core::Importance::High,
                diroxide_core::MessageKind::Question,
            );
        }
        match self.kind {
            JobKind::DeletePermanently => catalog::delete_for_good(items),
            _ => Message::question(&self.summary()),
        }
    }

    /// The line under a confirmation: the honest sentence for a permanent
    /// delete, else nothing.
    pub fn caution(&self) -> Option<&'static str> {
        (self.kind == JobKind::DeletePermanently).then_some(catalog::PERMANENT_DELETE_NOTE)
    }

    /// One step in words, with what the plan knows about its entry: ",
    /// read-only", ", a link".
    pub fn describe_step(&self, step: &Step) -> String {
        let mut line = step.describe();
        if let Some(entry) = self.expected.get(step.subject()) {
            if entry.kind == EntryKind::Link {
                line.push_str(", a link");
            }
            if is_read_only(entry) {
                line.push_str(", read-only");
            }
        }
        line
    }

    /// Every step in words, one per line, for the listing that a delete,
    /// move or overwrite shows before its confirmation; the note, when
    /// there is one, comes first.
    pub fn listing(&self) -> Vec<String> {
        self.note
            .iter()
            .cloned()
            .chain(self.steps.iter().map(|step| self.describe_step(step)))
            .collect()
    }

    /// The listing grouped by folder, in step order: each group is the
    /// steps whose subjects share a folder, titled "In D:\old, 3 items".
    pub fn groups(&self) -> Vec<ListingGroup> {
        let mut groups: Vec<(Option<Place>, ListingGroup)> = Vec::new();
        for step in &self.steps {
            let folder = step.subject().parent();
            let line = self.describe_step(step);
            match groups.last_mut() {
                Some((last, group)) if *last == folder => group.lines.push(line),
                _ => groups.push((
                    folder,
                    ListingGroup {
                        title: String::new(),
                        lines: vec![line],
                    },
                )),
            }
        }
        groups
            .into_iter()
            .map(|(folder, mut group)| {
                let items = count(group.lines.len() as u64, "item", "items");
                group.title = match folder {
                    Some(folder) => format!("In {}, {items}", spell_out(&folder.to_string())),
                    None => format!("At the top, {items}"),
                };
                group
            })
            .collect()
    }

    /// The places the plan will make or change, for the reconciliation
    /// after the run.
    pub fn planned_results(&self) -> Vec<Place> {
        self.steps
            .iter()
            .filter_map(Step::result)
            .cloned()
            .collect()
    }

    /// Leaves an item out, and everything inside it: its steps go, and it
    /// is reported as skipped with the reason. The app's answer to a
    /// question such as "Too large for this drive... Skip it?".
    pub fn leave_out(&mut self, place: &Place, reason: &str) {
        let before = self.steps.len();
        self.steps.retain(|step| {
            !step.subject().is_within(place) && !step.result().is_some_and(|r| r.is_within(place))
        });
        if self.steps.len() != before {
            self.facts.left_out.push((place.clone(), reason.to_owned()));
            self.facts.questions.retain(|question| match question {
                Preflight::TooLarge { place: p, .. } | Preflight::NameRefused { place: p, .. } => {
                    !p.is_within(place)
                }
                _ => true,
            });
            self.facts.whole.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::place::PathStyle;

    fn p(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[test]
    fn plans_say_what_they_will_do() {
        let plan = Plan::new(
            JobId::new(1),
            JobKind::Copy,
            vec![
                Step::Copy {
                    from: p("/a/x.txt"),
                    to: p("/b/x.txt"),
                },
                Step::Copy {
                    from: p("/a/C\u{F03A}"),
                    to: p("/b/C\u{F03A}"),
                },
            ],
            ConflictPolicy::Ask,
        );
        assert_eq!(plan.summary(), "Copy 2 items to /b");
        assert_eq!(plan.listing()[1], "copy /a/C[U+F03A] to /b/C[U+F03A]");
        assert_eq!(
            plan.planned_results(),
            vec![p("/b/x.txt"), p("/b/C\u{F03A}")]
        );
        let trash = Plan {
            kind: JobKind::Trash,
            steps: vec![Step::Trash { place: p("/a/x") }],
            note: Some("Not the focused row: report.pdf".into()),
            ..plan
        };
        assert_eq!(trash.summary(), "Move 1 item to the trash");
        assert_eq!(trash.steps[0].subject(), &p("/a/x"));
        assert_eq!(trash.listing()[0], "Not the focused row: report.pdf");
        assert!(trash.planned_results().is_empty());
    }

    fn file(name: &str, size: u64) -> Entry {
        Entry::new(name, EntryKind::File).with_size(size)
    }

    #[test]
    fn a_permanent_delete_says_its_counts_first() {
        let mut plan = Plan::new(
            JobId::new(2),
            JobKind::DeletePermanently,
            vec![
                Step::Remove { place: p("/old/a") },
                Step::Remove { place: p("/old/b") },
            ],
            ConflictPolicy::Skip,
        );
        let mut read_only = file("b", 1_200_000_000);
        read_only.attributes = Some(Attributes::Windows(0x1));
        plan.facts.count(&p("/old/a"), &file("a", 1_100_000_000));
        plan.facts.count(&p("/old/b"), &read_only);
        plan.facts.counted = true;
        plan.expected.insert(p("/old/b"), read_only);
        assert_eq!(
            plan.summary(),
            "Delete 2 items for good, 2.3 GB, 1 read-only"
        );
        assert_eq!(plan.question().text, "Delete 2 items for good? y or n");
        assert_eq!(plan.caution(), Some(catalog::PERMANENT_DELETE_NOTE));
        assert_eq!(plan.listing()[1], "delete /old/b for good, read-only");
        let groups = plan.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "In /old, 2 items");
        plan.facts.typed = Some("2".into());
        assert_eq!(plan.question().text, "Type 2 to go on, or Escape");
    }

    #[test]
    fn the_unusual_things_are_counted_in_words() {
        let mut plan = Plan::new(
            JobId::new(3),
            JobKind::Copy,
            Vec::new(),
            ConflictPolicy::Ask,
        );
        let mut link = Entry::new("l", EntryKind::Link);
        link.link = None;
        plan.facts.count(&p("/a/l"), &link);
        plan.facts.count(
            &p("/a/C\u{F03A}"),
            &Entry::new("C\u{F03A}", EntryKind::File),
        );
        let mut online = file("o", 5);
        online.attributes = Some(Attributes::Windows(0x400000));
        plan.facts.count(&p("/a/o"), &online);
        assert_eq!(
            plan.unusual().unwrap(),
            "Includes 1 link, 1 name with hidden characters, 1 online only."
        );
        assert!(within_cells(&plan.unusual().unwrap(), "Includes 1 link"));
    }

    #[test]
    fn leaving_out_an_item_takes_its_steps_and_says_why() {
        let mut plan = Plan::new(
            JobId::new(4),
            JobKind::Copy,
            vec![
                Step::Copy {
                    from: p("/a/big"),
                    to: p("/b/big"),
                },
                Step::Copy {
                    from: p("/a/f"),
                    to: p("/b/f"),
                },
                Step::Copy {
                    from: p("/a/f/x"),
                    to: p("/b/f/x"),
                },
            ],
            ConflictPolicy::Ask,
        );
        plan.facts.questions.push(Preflight::TooLarge {
            place: p("/a/big"),
            size: 5_200_000_000,
            max: 4_294_967_295,
        });
        assert_eq!(
            plan.facts.questions[0].lines()[0],
            "Too large for this drive: big"
        );
        assert_eq!(plan.facts.questions[0].lines()[2], "Skip it? y or n");
        plan.leave_out(&p("/a/big"), "skipped: too large for this drive");
        plan.leave_out(&p("/a/f"), "skipped by the user");
        assert!(plan.steps.is_empty());
        assert_eq!(plan.facts.left_out.len(), 2);
        assert!(plan.facts.questions.is_empty());
    }

    #[test]
    fn preflight_lines_put_the_fact_first() {
        let room = Preflight::NotEnoughRoom {
            drive: p("/"),
            needed: 4_000_000_000,
            free: 1_200_000_000,
        };
        assert_eq!(
            room.lines(),
            vec![
                "Not enough room on /".to_owned(),
                "Needs 4.0 GB, 1.2 GB free.".to_owned(),
                "Go on anyway? y or n".to_owned()
            ]
        );
        let trash = Preflight::NoTrash {
            gate: TrashGate::NoTrashHere(diroxide_vfs::VolumeKind::Removable),
            places: vec![p("/x"), p("/y"), p("/z")],
        };
        assert_eq!(
            trash.lines()[0],
            "This drive has no trash. Delete 3 items for good? y or n"
        );
        for line in room.lines().iter().chain(trash.lines().iter()) {
            assert!(line.chars().count() <= 60, "{line}");
        }
    }
}
