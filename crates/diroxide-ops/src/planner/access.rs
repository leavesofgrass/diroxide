//! The permissions, attributes and times plan (research/permissions.md 6.5
//! to 7; research/alpha2-plan.md commit 27).

use std::collections::BTreeMap;

use diroxide_core::{Attributes, EntryKind, JobKind, JobRequest, Place};
use diroxide_vfs::AttributeChange;

use super::{Planner, Removal};
use crate::access::{
    AttributeEdit, AttributeFacts, ModeChange, change_words, group_row, mode_line, time_words,
};
use crate::engine::{Engine, JobControl, Quiet};
use crate::error::OpsError;
use crate::plan::{Plan, Preflight, Step};
use crate::walk::Found;

/// One planned change, before the steps are ordered.
struct Planned {
    place: Place,
    change: AttributeChange,
    /// Whether it goes after what is inside the folder: a folder whose
    /// owner loses list or enter, and every folder's times.
    after_contents: bool,
}

impl Planner<'_> {
    /// Permissions, attributes and times: every entry the edit reaches,
    /// with its new values computed from what it has, those that already
    /// match left out, ordered so a folder stays reachable while its
    /// contents change.
    fn attributes(&mut self, edit: &AttributeEdit) -> Result<(), OpsError> {
        if edit.is_empty() {
            return Err(OpsError::NoChange);
        }
        let mut planned: Vec<Planned> = Vec::new();
        let mut groups: BTreeMap<(bool, String), u64> = BTreeMap::new();
        let mut single_mode: Option<(u32, EntryKind)> = None;
        let mut created_note = false;
        let mut loses_access = 0u64;
        let mut everyone: Option<(Place, u64)> = None;
        let request = self.request;
        for source in &request.sources {
            if edit.recursion.goes_inside() {
                self.anchor(source, Removal::Permanent)?;
            }
            let name = Self::name_of(source)?;
            let provider = self.provider(source)?;
            let entry = provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            if entry.kind == EntryKind::Link {
                return Err(OpsError::LinkSelected(
                    diroxide_core::names::spell_out(&name).into_owned(),
                ));
            }
            let found = if edit.recursion.goes_inside() && entry.kind == EntryKind::Folder {
                self.walk(&*provider, source, entry)?
            } else {
                vec![Found {
                    place: source.clone(),
                    entry,
                    path: Vec::new(),
                }]
            };
            let capabilities = provider.capabilities(source);
            for item in found {
                let inside = item.depth() > 0;
                let kind = item.entry.kind;
                if kind == EntryKind::Link {
                    self.plan.facts.left_out.push((
                        item.place.clone(),
                        "a link: links have no permissions of their own".to_owned(),
                    ));
                    continue;
                }
                if inside && !edit.recursion.takes(kind) {
                    continue;
                }
                let mut change = AttributeChange::default();
                let mut after_contents = false;
                match (edit.mode, item.entry.attributes) {
                    (Some(mode), Some(Attributes::Unix(current))) => {
                        let new = mode.apply(current, kind, inside);
                        if new != current {
                            change.attributes = Some(Attributes::Unix(new));
                            change.expect = item.entry.attributes;
                            let words = change_words(current, new, kind);
                            *groups
                                .entry((kind == EntryKind::Folder, words))
                                .or_insert(0) += 1;
                            single_mode = Some((new, kind));
                            let owner_lost = (current & !new) & 0o500;
                            if kind == EntryKind::Folder && owner_lost != 0 {
                                after_contents = true;
                            }
                            let reads = if kind == EntryKind::Folder {
                                0o500
                            } else {
                                0o400
                            };
                            if current & reads != 0 && new & reads != current & reads {
                                loses_access += 1;
                            }
                            let opens = new & 0o002 != 0 && current & 0o002 == 0;
                            let sticky_folder = kind == EntryKind::Folder && new & 0o1000 != 0;
                            if opens && !sticky_folder {
                                let (_, n) = everyone.get_or_insert((item.place.clone(), 0));
                                *n += 1;
                            }
                            if kind != EntryKind::Folder
                                && new & 0o4000 != 0
                                && current & 0o4000 == 0
                            {
                                self.plan.facts.questions.push(Preflight::RunsAsOwner {
                                    place: item.place.clone(),
                                });
                            }
                        }
                    }
                    (Some(_), None) => {
                        self.plan.facts.left_out.push((
                            item.place.clone(),
                            "its permissions could not be read".to_owned(),
                        ));
                        continue;
                    }
                    _ => {}
                }
                if let (Some(windows), Some(Attributes::Windows(current))) =
                    (edit.windows, item.entry.attributes)
                {
                    let new = windows.apply(current);
                    if new != current {
                        change.attributes = Some(Attributes::Windows(new));
                        change.expect = item.entry.attributes;
                    }
                }
                if edit.modified.is_some() && edit.modified != item.entry.modified {
                    change.modified = edit.modified;
                }
                if edit.accessed.is_some() && edit.accessed != item.entry.accessed {
                    change.accessed = edit.accessed;
                }
                if edit.created.is_some() && edit.created != item.entry.created {
                    if capabilities.access.created_time.is_possible() {
                        change.created = edit.created;
                    } else {
                        created_note = true;
                    }
                }
                if change.is_empty() {
                    continue;
                }
                if kind == EntryKind::Folder && change.attributes.is_none() {
                    // A folder's times are set after its contents.
                    after_contents = true;
                }
                self.plan.facts.count(&item.place, &item.entry);
                self.plan.expected.insert(item.place.clone(), item.entry);
                planned.push(Planned {
                    place: item.place,
                    change,
                    after_contents,
                });
            }
        }
        self.plan.steps.extend(order(planned));
        if let Some((place, items)) = everyone {
            self.plan
                .facts
                .questions
                .push(Preflight::EveryoneCanChange { place, items });
        }
        if loses_access > 0 {
            self.plan.facts.questions.push(Preflight::LosesAccess {
                items: loses_access,
            });
        }
        let mut rows = vec![edit.recursion.row()];
        let steps = self.plan.steps.len();
        match (single_mode, groups.len()) {
            (Some((mode, kind)), 1) if steps == 1 => rows.push(mode_line(mode, kind)),
            _ => rows.extend(groups.iter().map(|((folder, words), n)| {
                let kind = if *folder {
                    EntryKind::Folder
                } else {
                    EntryKind::File
                };
                group_row(*n, kind, words)
            })),
        }
        if edit.recursion.goes_inside() && matches!(edit.mode, Some(ModeChange::Bits { .. })) {
            rows.push("Files keep their run setting.".to_owned());
            rows.push("Folders stay enterable.".to_owned());
        }
        if let Some(windows) = edit.windows {
            rows.extend(windows.rows());
        }
        if let Some(time) = edit.modified {
            rows.push(format!(
                "Modified time to {}",
                time_words(time, edit.offset)
            ));
        }
        if let Some(time) = edit.created {
            rows.push(format!("Created time to {}", time_words(time, edit.offset)));
        }
        if let Some(time) = edit.accessed {
            rows.push(format!(
                "Accessed time to {}",
                time_words(time, edit.offset)
            ));
        }
        if created_note {
            rows.push("Created time not set: this drive cannot keep one.".to_owned());
        }
        if self.plan.steps.is_empty() && self.plan.facts.left_out.is_empty() {
            rows.push(if edit.times_only() {
                "Nothing to change: the times already match.".to_owned()
            } else {
                "Nothing to change: permissions already match.".to_owned()
            });
        }
        // Bytes are not what this job is about.
        self.plan.facts.counted = false;
        self.plan.facts.attributes = Some(AttributeFacts {
            edit: edit.clone(),
            rows,
        });
        Ok(())
    }
}

/// The steps in order: the walk's order, folder before contents, except a
/// folder marked to go after its contents, which follows the last entry
/// inside it.
fn order(planned: Vec<Planned>) -> Vec<Step> {
    let mut steps = Vec::with_capacity(planned.len());
    let mut waiting: Vec<Planned> = Vec::new();
    let step = |planned: Planned| Step::SetAttributes {
        place: planned.place,
        change: planned.change,
    };
    for item in planned {
        while waiting
            .last()
            .is_some_and(|folder| !item.place.is_within(&folder.place))
        {
            if let Some(folder) = waiting.pop() {
                steps.push(step(folder));
            }
        }
        if item.after_contents {
            waiting.push(item);
        } else {
            steps.push(step(item));
        }
    }
    while let Some(folder) = waiting.pop() {
        steps.push(step(folder));
    }
    steps
}

impl Engine {
    /// Plans a permissions, attributes or times job: the request names the
    /// entries (and `SetAttributes` or `SetTimes` as its kind), and its
    /// edit (`JobRequest::attributes`, P2b) what changes and where; a
    /// request without one is refused as `NoChange`. A selected link is
    /// refused; a change that goes inside folders is refused at a drive
    /// root or a protected folder; the rows say the change in words, the
    /// recursion first.
    pub fn plan_attributes(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        let edit = request.attributes.as_ref().ok_or(OpsError::NoChange)?;
        if request.sources.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let quiet = Quiet;
        let control = JobControl::new();
        let mut planner = Planner::new(self, request, &control, &quiet);
        planner.plan.kind = match request.kind {
            JobKind::SetTimes => JobKind::SetTimes,
            _ if edit.times_only() => JobKind::SetTimes,
            _ => JobKind::SetAttributes,
        };
        planner.attributes(edit)?;
        Ok(planner.finish())
    }
}
