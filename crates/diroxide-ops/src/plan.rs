//! Plans: what a job will do, computed before anything is touched.

use diroxide_core::names::spell_out;
use diroxide_core::{ConflictPolicy, JobId, JobKind, Place};
use diroxide_vfs::{AttributeChange, LinkRequest};

/// One step of a plan, acting on one entry.
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

    /// The step in words, with hidden characters spelled out: "copy
    /// D:\docs\a.txt to D:\backup\a.txt".
    pub fn describe(&self) -> String {
        let spell = |place: &Place| spell_out(&place.to_string()).into_owned();
        match self {
            Step::Copy { from, to } => format!("copy {} to {}", spell(from), spell(to)),
            Step::Move { from, to } => format!("move {} to {}", spell(from), spell(to)),
            Step::Rename { from, to } | Step::RenameHop { from, to, .. } => {
                format!("rename {} to {}", spell(from), spell(to))
            }
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
        }
    }

    /// The plan in one line, meaning first: "Copy 3 items to D:\backup".
    pub fn summary(&self) -> String {
        let verb = self.kind.verb();
        let mut first = verb.chars();
        let verb = match first.next() {
            Some(c) => c.to_uppercase().chain(first).collect::<String>(),
            None => String::new(),
        };
        let count = diroxide_core::format::count(self.steps.len() as u64, "item", "items");
        let destination = self.steps.iter().find_map(|step| match step {
            Step::Copy { to, .. } | Step::Move { to, .. } => to.parent(),
            _ => None,
        });
        match destination {
            Some(folder) => format!("{verb} {count} to {}", spell_out(&folder.to_string())),
            None => format!("{verb} {count}"),
        }
    }

    /// Every step in words, one per line, for the listing that a delete,
    /// move or overwrite shows before its confirmation; the note, when
    /// there is one, comes first.
    pub fn listing(&self) -> Vec<String> {
        self.note
            .iter()
            .cloned()
            .chain(self.steps.iter().map(Step::describe))
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(trash.summary(), "Move to the trash 1 item");
        assert_eq!(trash.steps[0].subject(), &p("/a/x"));
        assert_eq!(trash.listing()[0], "Not the focused row: report.pdf");
        assert!(trash.planned_results().is_empty());
    }
}
