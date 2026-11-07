//! Change time and Change owner (alpha 4, lane AP, R30 commits 78 and
//! 79; R29 4.3), lists of settings in the pattern of the permissions
//! dialog (`docs/links-permissions.md`, "The dialog"): the first row says
//! what will happen and plans it on Enter, the rows after it are labeled
//! settings that Enter changes, and the plan's question starts on No. The
//! words of the rows, the refusals and the plan are AX's, in
//! `diroxide-ops`'s access module; the app's requests are in
//! [`crate::access`].
//!
//! - **Change time**: "Change the times of main.rs"; "Change: modified
//!   only" (Enter steps to modified and accessed, accessed only, all
//!   three, created only, where the place can set a created time); "Set
//!   to: now" (Enter steps to a date and time typed in ISO form, then the
//!   other panel's file's times); the times now, each with its weekday
//!   computed from the date ("Modified: Saturday, 2026-10-10 22:09");
//!   "Changed: set by the system" on Unix, whose Enter says why it cannot
//!   be set; "Apply to:" for a folder. The plan reads "Modified time to
//!   Saturday, 2026-10-10 22:09"; after the job the times the disk kept
//!   are read back, and a rounded one is said ("Modified time rounded by
//!   the disk"). Undo puts the old times back.
//! - **Change owner** on Unix: "Change owner of main.rs"; "Now: owner ada,
//!   group staff"; "Owner: ada, as now" and "Group: staff, as now", whose
//!   Enter opens a field that completes the user and group names this
//!   computer knows (Tab); "Apply to:" for a folder. The question is
//!   "Change owner of 3 items to root? No is the default. y or n"; a
//!   refusal is said whole ("Could not change the owner of a.txt: only an
//!   administrator can. Start diro as administrator, then try again.");
//!   the system's folders are refused before the dialog opens; nothing
//!   ever elevates. On Windows the owner is shown in the details, and
//!   Change owner hands the focused item to the system's Security dialog.
//!
//! The lists and fields are hosted under the Organize purpose, so the
//! window's test of every purpose keeps its count.

use std::collections::{HashMap, HashSet};

use diroxide_core::access::{AttributeEdit, Recursion};
use diroxide_core::format::count;
use diroxide_core::{
    Command, Effect, EntryKind, JobId, JobKind, JobRequest, ListModel, ListRow, Message, Place,
    PlaceKind, Side, Timestamp,
};
use diroxide_ops::access::{
    CHANGE_TIME_REFUSAL, OWNER_NOT_HERE, OwnerCommand, OwnerWords, SECURITY_DIALOG_FAILED,
    TimeKind, owner_command, owner_protection, security_dialog_line, time_row, time_words,
};
use diroxide_vfs::access::Caller;

use super::organize::Wants;
use super::{App, Then};
use crate::access::{AccessAsk, AccessNote, parse_time, typed_form};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::request::Request;

/// The help of both dialogs.
const FORM_HELP: &str = "Up and Down move through the settings, Enter changes one, Enter on the first row plans the change, Escape closes.";

/// What the access dialogs' jobs wait for.
#[derive(Clone, Debug, Default)]
pub(super) struct AccessState {
    /// The time jobs planned or running, each with the times it sets, so
    /// what the disk kept is read back when it ends.
    times: HashMap<JobId, Vec<(TimeKind, Timestamp)>>,
    /// The owner jobs planned or running.
    owners: HashSet<JobId>,
}

/// Which times Change time sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimeSet {
    Modified,
    ModifiedAccessed,
    Accessed,
    All,
    Created,
}

impl TimeSet {
    /// The choices in the order Enter steps through them; the created
    /// time only where the place can set one.
    fn choices(created: bool) -> &'static [TimeSet] {
        if created {
            &[
                TimeSet::Modified,
                TimeSet::ModifiedAccessed,
                TimeSet::Accessed,
                TimeSet::All,
                TimeSet::Created,
            ]
        } else {
            &[
                TimeSet::Modified,
                TimeSet::ModifiedAccessed,
                TimeSet::Accessed,
            ]
        }
    }

    fn words(self) -> &'static str {
        match self {
            TimeSet::Modified => "modified only",
            TimeSet::ModifiedAccessed => "modified and accessed",
            TimeSet::Accessed => "accessed only",
            TimeSet::All => "all three",
            TimeSet::Created => "created only",
        }
    }

    fn kinds(self) -> &'static [TimeKind] {
        match self {
            TimeSet::Modified => &[TimeKind::Modified],
            TimeSet::ModifiedAccessed => &[TimeKind::Modified, TimeKind::Accessed],
            TimeSet::Accessed => &[TimeKind::Accessed],
            TimeSet::All => &[TimeKind::Modified, TimeKind::Accessed, TimeKind::Created],
            TimeSet::Created => &[TimeKind::Created],
        }
    }
}

/// An entry's three times, as the panel listed them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Times {
    modified: Option<Timestamp>,
    accessed: Option<Timestamp>,
    created: Option<Timestamp>,
}

impl Times {
    fn of(entry: &diroxide_core::Entry) -> Self {
        Self {
            modified: entry.modified,
            accessed: entry.accessed,
            created: entry.created,
        }
    }

    fn get(&self, kind: TimeKind) -> Option<Timestamp> {
        match kind {
            TimeKind::Modified => self.modified,
            TimeKind::Accessed => self.accessed,
            TimeKind::Created => self.created,
            TimeKind::Changed => None,
        }
    }
}

/// Where Change time's new time comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TimeSource {
    /// The clock, when the plan is made.
    Now,
    /// A date and time typed.
    Typed(Timestamp),
    /// The other panel's focused entry: each time from the same time of
    /// it, else its modified time.
    Other { name: String, times: Times },
}

/// Change time's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TimeForm {
    side: Side,
    targets: Vec<Place>,
    /// The entry's name, or "3 items".
    what: String,
    any_folder: bool,
    /// The first entry's times now.
    now: Times,
    set: TimeSet,
    source: TimeSource,
    recursion: Recursion,
    /// Whether the place can set a created time.
    created: bool,
    note: Option<String>,
}

/// Change owner's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OwnerForm {
    side: Side,
    targets: Vec<Place>,
    what: String,
    any_folder: bool,
    now_owner: Option<String>,
    now_group: Option<String>,
    /// The new owner, typed or completed; `None` keeps it.
    owner: Option<String>,
    /// The new group; `None` keeps it.
    group: Option<String>,
    recursion: Recursion,
    /// Whether the place is this computer's, whose names complete.
    local: bool,
    note: Option<String>,
}

/// What a dialog of this lane is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AccessWants {
    /// Change time's list.
    TimeForm(Box<TimeForm>),
    /// The field a date and time is typed in.
    TypedTime(Box<TimeForm>),
    /// Change owner's list.
    OwnerForm(Box<OwnerForm>),
    /// The field the new owner is typed in.
    OwnerName(Box<OwnerForm>),
    /// The field the new group is typed in.
    GroupName(Box<OwnerForm>),
}

/// What a dialog of this lane acts on: the marked entries or the focused
/// one, the selection's note, the first entry as listed, and whether a
/// folder is among them.
struct Targets {
    places: Vec<Place>,
    note: Option<String>,
    first: diroxide_core::Entry,
    any_folder: bool,
}

/// "main.rs", or "3 items".
fn what_of(targets: &[Place]) -> String {
    match targets {
        [one] => one
            .name()
            .map(diroxide_core::names::display_name)
            .unwrap_or_default()
            .into_owned(),
        many => count(many.len() as u64, "item", "items"),
    }
}

impl TimeForm {
    fn rows(&self, offset: diroxide_core::UtcOffset) -> Vec<ListRow> {
        let source = match &self.source {
            TimeSource::Now => "Set to: now".to_owned(),
            TimeSource::Typed(time) => format!("Set to: {}", time_words(*time, offset)),
            TimeSource::Other { name, .. } => format!("Set to: the times of {name}"),
        };
        let mut rows = vec![
            ListRow::new("go", format!("Change the times of {}", self.what)),
            ListRow::new("set", format!("Change: {}", self.set.words())),
            ListRow::new("source", source),
            ListRow::new(
                "modified",
                time_row(TimeKind::Modified, self.now.modified, offset, true),
            ),
            ListRow::new(
                "accessed",
                time_row(TimeKind::Accessed, self.now.accessed, offset, true),
            ),
            ListRow::new(
                "created",
                time_row(TimeKind::Created, self.now.created, offset, self.created),
            ),
        ];
        if cfg!(unix) {
            rows.push(ListRow::new(
                "changed",
                time_row(TimeKind::Changed, None, offset, false),
            ));
        }
        if self.any_folder {
            rows.push(ListRow::new("apply", self.recursion.row()));
        }
        rows
    }

    fn list(&self, offset: diroxide_core::UtcOffset) -> ListModel {
        let mut list = ListModel::new("Change time").with_rows(self.rows(offset));
        list.set_help(FORM_HELP);
        list
    }

    /// The time each chosen kind is set to, the clock giving `now`.
    fn values(&self, now: Timestamp) -> Vec<(TimeKind, Timestamp)> {
        let now = Timestamp::from_unix(now.unix_seconds(), 0);
        self.set
            .kinds()
            .iter()
            .filter(|kind| **kind != TimeKind::Created || self.created)
            .filter_map(|kind| {
                let time = match &self.source {
                    TimeSource::Now => Some(now),
                    TimeSource::Typed(time) => Some(*time),
                    TimeSource::Other { times, .. } => times.get(*kind).or(times.modified),
                };
                time.map(|t| (*kind, t))
            })
            .collect()
    }
}

impl OwnerForm {
    fn rows(&self, caller: &Caller) -> Vec<ListRow> {
        let now = match (&self.now_owner, &self.now_group) {
            (Some(owner), Some(group)) => format!("Now: owner {owner}, group {group}"),
            (Some(owner), None) => format!("Now: owner {owner}"),
            (None, Some(group)) => format!("Now: group {group}"),
            (None, None) => "Now: owner and group not known".to_owned(),
        };
        let owner = match (&self.owner, &self.now_owner) {
            (Some(owner), _) => format!("Owner: {owner}"),
            (None, Some(now)) => format!("Owner: {now}, as now"),
            (None, None) => "Owner: as now".to_owned(),
        };
        let group = match (&self.group, &self.now_group) {
            (Some(group), _) => group_words(group, caller),
            (None, Some(now)) => format!("Group: {now}, as now"),
            (None, None) => "Group: as now".to_owned(),
        };
        let mut rows = vec![
            ListRow::new("go", format!("Change owner of {}", self.what)),
            ListRow::new("now", now),
            ListRow::new("owner", owner),
            ListRow::new("group", group),
        ];
        if self.any_folder {
            rows.push(ListRow::new("apply", self.recursion.row()));
        }
        rows
    }

    fn list(&self, caller: &Caller) -> ListModel {
        let mut list = ListModel::new("Change owner").with_rows(self.rows(caller));
        list.set_help(FORM_HELP);
        list
    }
}

/// "Group: staff (one of your groups)" (33); "Group: wheel (not one of
/// yours)"; "Group: wheel" for an administrator or a group the system
/// does not name.
fn group_words(group: &str, caller: &Caller) -> String {
    let gid = diroxide_vfs::access::find_group(group)
        .and_then(|g| g.id.parse::<u32>().ok())
        .or_else(|| group.parse().ok());
    match gid.and_then(|gid| caller.in_group(gid)) {
        _ if caller.administrator => format!("Group: {group}"),
        Some(true) => format!("Group: {group} (one of your groups)"),
        Some(false) => format!("Group: {group} (not one of yours)"),
        None => format!("Group: {group}"),
    }
}

/// The names a field completes: the users, or the groups the caller may
/// choose, the caller's own first.
fn names(group: bool, caller: &Caller) -> Vec<String> {
    let principals = if group {
        diroxide_vfs::access::known_groups()
    } else {
        diroxide_vfs::access::known_users()
    };
    let mut names: Vec<(bool, String)> = principals
        .into_iter()
        .filter_map(|p| {
            let gid = p.id.parse::<u32>().ok();
            let mine = group && gid.is_some_and(|gid| caller.in_group(gid) == Some(true));
            if group
                && !caller.administrator
                && gid.is_some_and(|g| caller.in_group(g) == Some(false))
            {
                return None;
            }
            p.name.map(|name| (!mine, name))
        })
        .collect();
    names.sort();
    names.into_iter().map(|(_, name)| name).collect()
}

impl App {
    /// Change time and Change owner; `None` for commands that are not
    /// theirs.
    pub(super) fn access_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        if *id != ids::CHANGE_TIME && *id != ids::CHANGE_OWNER {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(if *id == ids::CHANGE_TIME {
            self.change_time()
        } else {
            self.change_owner()
        })
    }

    /// The marked entries or the focused one, with the first's entry; or
    /// what is said instead.
    fn access_targets(&mut self, side: Side) -> Result<Targets, Vec<Effect>> {
        let Some((targets, note)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return Err(vec![self.say(Message::status(empty))]);
        };
        if self.panel(side).place().kind() == PlaceKind::Archive {
            return Err(vec![self.say(Message::error(super::archive::READ_ONLY))]);
        }
        let panel = self.panel(side);
        let entries: Vec<&diroxide_core::Entry> = targets
            .iter()
            .filter_map(|p| p.name().and_then(|n| panel.entry(n)))
            .collect();
        let Some(first) = entries.first().map(|e| (*e).clone()) else {
            let empty = panel.list().empty_text().to_owned();
            return Err(vec![self.say(Message::status(empty))]);
        };
        let any_folder = entries.iter().any(|e| e.kind == EntryKind::Folder);
        Ok(Targets {
            places: targets,
            note,
            first,
            any_folder,
        })
    }

    /// What a panel's place can change about access: as its listing said,
    /// or, until one says, what a disk of this computer allows (the times;
    /// a created time on Windows and macOS; the owner and group elsewhere,
    /// the owner with an administrator), which the plan checks again.
    fn access_capabilities(&self, side: Side) -> diroxide_vfs::Capabilities {
        use diroxide_vfs::Can;
        let panel = self.panel(side);
        if let Some(capabilities) = panel.capabilities() {
            return capabilities;
        }
        let mut capabilities = diroxide_vfs::Capabilities::read_only();
        if panel.place().kind() == PlaceKind::Local {
            capabilities.writable = true;
            capabilities.access.times = Can::Yes;
            if cfg!(any(windows, target_vendor = "apple")) {
                capabilities.access.created_time = Can::Yes;
            }
            if cfg!(unix) {
                capabilities.access.owner = Can::WithAdministrator;
                capabilities.access.group = Can::Yes;
            }
        }
        capabilities
    }

    /// Change time: the dialog, for the marked entries or the focused one.
    fn change_time(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Targets {
            places: targets,
            note,
            first,
            any_folder,
        } = match self.access_targets(side) {
            Ok(found) => found,
            Err(effects) => return effects,
        };
        let access = self.access_capabilities(side).access;
        if !access.times.is_possible() {
            let text = format!(
                "Could not change the times of {}: this place does not keep them.",
                first.display_name()
            );
            return vec![self.say(Message::error(text))];
        }
        let form = TimeForm {
            side,
            what: what_of(&targets),
            targets,
            any_folder,
            now: Times::of(&first),
            set: TimeSet::Modified,
            source: TimeSource::Now,
            recursion: Recursion::ThisItem,
            created: access.created_time.is_possible(),
            note,
        };
        self.show_time_form(form, None)
    }

    fn show_time_form(&mut self, form: TimeForm, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list(self.clock.offset());
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Access(AccessWants::TimeForm(Box::new(form))),
        )
    }

    /// The other panel's focused entry, as a source of times.
    fn other_times(&self, side: Side) -> Option<TimeSource> {
        let entry = self.panel(side.other()).focused_entry()?;
        entry.modified?;
        Some(TimeSource::Other {
            name: entry.display_name().into_owned(),
            times: Times::of(entry),
        })
    }

    /// Enter on a row of Change time.
    fn time_form_row(&mut self, mut form: TimeForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "go" => {
                let values = form.values(self.now());
                if values.is_empty() {
                    let line = "Nothing to change: no time to set from there";
                    effects.push(self.say(Message::info(line)));
                    effects.extend(self.show_time_form(form, Some(row)));
                    return effects;
                }
                let mut edit = AttributeEdit::new(form.recursion);
                for (kind, time) in &values {
                    edit = match kind {
                        TimeKind::Modified => edit.with_modified(*time),
                        TimeKind::Accessed => edit.with_accessed(*time),
                        TimeKind::Created => edit.with_created(*time),
                        TimeKind::Changed => edit,
                    };
                }
                edit.offset = self.clock.offset();
                let job = self.next_job_id();
                let request =
                    JobRequest::new(job, JobKind::SetTimes, form.targets.clone()).changing(edit);
                self.access.times.insert(job, values);
                effects.extend(self.plan_job(
                    form.side,
                    request,
                    form.note.clone(),
                    Then::Report,
                    true,
                ));
                return effects;
            }
            "set" => {
                let choices = TimeSet::choices(form.created);
                let at = choices.iter().position(|s| *s == form.set).unwrap_or(0);
                form.set = choices[(at + 1) % choices.len()];
            }
            "source" => match form.source {
                TimeSource::Now => {
                    let offset = self.clock.offset();
                    let start = form.now.modified.unwrap_or_else(|| self.now());
                    let field =
                        EntryField::new("Set to, as 2026-10-10 22:09:", CompletionSource::None)
                            .with_text(typed_form(start, offset));
                    effects.extend(self.open_organize(
                        Dialog::edit(field),
                        Wants::Access(AccessWants::TypedTime(Box::new(form))),
                    ));
                    return effects;
                }
                TimeSource::Typed(_) => {
                    form.source = self.other_times(form.side).unwrap_or(TimeSource::Now);
                }
                TimeSource::Other { .. } => form.source = TimeSource::Now,
            },
            "changed" => {
                effects.push(self.say(Message::info(CHANGE_TIME_REFUSAL)));
            }
            "apply" => form.recursion = next_recursion(form.recursion),
            _ => {}
        }
        effects.extend(self.show_time_form(form, Some(row)));
        effects
    }

    /// The typed date: read, then back to the list; a date that cannot be
    /// read is said, and the field stays to fix it.
    fn time_typed(&mut self, mut form: TimeForm, text: &str) -> Vec<Effect> {
        let offset = self.clock.offset();
        let now = self.now();
        match parse_time(text, offset, now) {
            Ok(time) => {
                let mut effects = self.close_top();
                form.source = TimeSource::Typed(time);
                effects.extend(self.show_time_form(form, Some("source")));
                effects
            }
            Err(line) => vec![self.say(Message::error(line))],
        }
    }

    /// Change owner: the app's own dialog where the place keeps owners,
    /// Windows' Security dialog on a disk of Windows, else why not.
    fn change_owner(&mut self) -> Vec<Effect> {
        let side = self.active;
        let place = self.panel(side).place().clone();
        let capabilities = self.access_capabilities(side);
        let scheme = if place.kind() == PlaceKind::Local {
            "file"
        } else {
            "other"
        };
        match owner_command(&capabilities, scheme) {
            OwnerCommand::NotHere => vec![self.say(Message::info(OWNER_NOT_HERE))],
            OwnerCommand::SecurityDialog => self.security_dialog(side),
            OwnerCommand::Dialog => self.owner_dialog(side),
        }
    }

    /// The Windows hand-off: the focused item to the system's Security
    /// dialog, never waited for.
    fn security_dialog(&mut self, side: Side) -> Vec<Effect> {
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let line = security_dialog_line(&entry.display_name());
        self.request(Request::Access(Box::new(AccessAsk::SecurityDialog {
            place,
        })));
        vec![self.say(Message::status(line))]
    }

    fn owner_dialog(&mut self, side: Side) -> Vec<Effect> {
        let Targets {
            places: targets,
            note,
            first,
            any_folder,
        } = match self.access_targets(side) {
            Ok(found) => found,
            Err(effects) => return effects,
        };
        // The system's folders are refused before anything is asked.
        let typed = OwnerWords {
            owner: Some(""),
            group: None,
        };
        for target in &targets {
            if let Some(why) = owner_protection(target) {
                let shown = diroxide_core::names::spell_out(&target.to_string()).into_owned();
                return vec![self.say(Message::error(typed.protected(&shown, why)))];
            }
        }
        if first.kind == EntryKind::Link {
            let text = format!(
                "Could not change the owner of {}: links are changed through their targets.",
                first.display_name()
            );
            return vec![self.say(Message::error(text))];
        }
        let form = OwnerForm {
            side,
            what: what_of(&targets),
            targets,
            any_folder,
            now_owner: first.owner.clone(),
            now_group: first.group.clone(),
            owner: None,
            group: None,
            recursion: Recursion::ThisItem,
            local: self.panel(side).place().kind() == PlaceKind::Local,
            note,
        };
        self.show_owner_form(form, None)
    }

    fn show_owner_form(&mut self, form: OwnerForm, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list(&Caller::current());
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Access(AccessWants::OwnerForm(Box::new(form))),
        )
    }

    /// Enter on a row of Change owner.
    fn owner_form_row(&mut self, mut form: OwnerForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "go" => {
                if form.owner.is_none() && form.group.is_none() {
                    let line = "Nothing to change: choose a new owner or group.";
                    effects.push(self.say(Message::info(line)));
                    effects.extend(self.show_owner_form(form, Some("owner")));
                    return effects;
                }
                let mut edit = AttributeEdit::new(form.recursion);
                if let Some(owner) = &form.owner {
                    edit = edit.with_owner(owner.clone());
                }
                if let Some(group) = &form.group {
                    edit = edit.with_group(group.clone());
                }
                let job = self.next_job_id();
                let request = JobRequest::new(job, JobKind::SetAttributes, form.targets.clone())
                    .changing(edit);
                self.access.owners.insert(job);
                effects.extend(self.plan_job(
                    form.side,
                    request,
                    form.note.clone(),
                    Then::Report,
                    true,
                ));
                return effects;
            }
            "owner" | "group" => {
                let group = row == "group";
                let caller = Caller::current();
                let names = if form.local {
                    names(group, &caller)
                } else {
                    Vec::new()
                };
                let (label, text, wants) = if group {
                    (
                        format!("New group of {}:", form.what),
                        form.group.clone().or(form.now_group.clone()),
                        AccessWants::GroupName(Box::new(form)),
                    )
                } else {
                    (
                        format!("New owner of {}:", form.what),
                        form.owner.clone().or(form.now_owner.clone()),
                        AccessWants::OwnerName(Box::new(form)),
                    )
                };
                let source = if names.is_empty() {
                    CompletionSource::None
                } else {
                    CompletionSource::Values(names)
                };
                let field = EntryField::new(label, source).with_text(text.unwrap_or_default());
                effects.extend(self.open_organize(Dialog::edit(field), Wants::Access(wants)));
                return effects;
            }
            "apply" => form.recursion = next_recursion(form.recursion),
            _ => {}
        }
        effects.extend(self.show_owner_form(form, Some(row)));
        effects
    }

    /// A new owner or group typed: kept as typed (a name or a number; the
    /// plan looks it up), an empty field keeps it as it is.
    fn owner_typed(&mut self, mut form: OwnerForm, text: &str, group: bool) -> Vec<Effect> {
        let mut effects = self.close_top();
        let typed = text.trim();
        let now = if group {
            form.now_group.as_deref()
        } else {
            form.now_owner.as_deref()
        };
        let chosen = (!typed.is_empty() && Some(typed) != now).then(|| typed.to_owned());
        if group {
            form.group = chosen;
        } else {
            form.owner = chosen;
        }
        let row = if group { "group" } else { "owner" };
        effects.extend(self.show_owner_form(form, Some(row)));
        effects
    }

    /// Enter in one of this lane's dialogs.
    pub(super) fn access_accept(
        &mut self,
        wants: AccessWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            AccessWants::TimeForm(form) => match row {
                Some(row) => self.time_form_row(*form, &row),
                None => Vec::new(),
            },
            AccessWants::TypedTime(form) => self.time_typed(*form, text),
            AccessWants::OwnerForm(form) => match row {
                Some(row) => self.owner_form_row(*form, &row),
                None => Vec::new(),
            },
            AccessWants::OwnerName(form) => self.owner_typed(*form, text, false),
            AccessWants::GroupName(form) => self.owner_typed(*form, text, true),
        }
    }

    /// Escape in one of this lane's dialogs, after it closed: a field goes
    /// back to its list, unchanged.
    pub(super) fn access_escape(&mut self, wants: AccessWants) -> Vec<Effect> {
        match wants {
            AccessWants::TypedTime(form) => self.show_time_form(*form, Some("source")),
            AccessWants::OwnerName(form) => self.show_owner_form(*form, Some("owner")),
            AccessWants::GroupName(form) => self.show_owner_form(*form, Some("group")),
            AccessWants::TimeForm(_) | AccessWants::OwnerForm(_) => Vec::new(),
        }
    }

    /// What the runtime answered: the times read back, or the Security
    /// dialog's hand-off.
    pub(super) fn access_note(&mut self, note: AccessNote) -> Vec<Effect> {
        match note {
            AccessNote::ReadBack { lines, .. } => lines
                .into_iter()
                .map(|line| self.say(Message::warning(line)))
                .collect(),
            AccessNote::SecurityDialog { result, .. } => match result {
                Ok(()) => Vec::new(),
                Err(_) => vec![self.say(Message::error(SECURITY_DIALOG_FAILED))],
            },
        }
    }

    /// A job of this lane ended (the job engine's report): a time job's
    /// times are read back; an owner job whose undo needs an
    /// administrator says so.
    pub(super) fn access_job_ended(&mut self, report: &diroxide_ops::JobReport) -> Vec<Effect> {
        if let Some(asked) = self.access.times.remove(&report.job) {
            if !report.done.is_empty() {
                self.request(Request::Access(Box::new(AccessAsk::ReadBack {
                    job: report.job,
                    places: report.done.clone(),
                    asked,
                })));
            }
            return Vec::new();
        }
        if self.access.owners.remove(&report.job)
            && !report.done.is_empty()
            && let Some(line) =
                diroxide_ops::access::undo_owner_line(&report.undo.attributes, &Caller::current())
        {
            return vec![self.say(Message::info(line))];
        }
        Vec::new()
    }
}

/// The next of the four "Apply to:" choices.
fn next_recursion(now: Recursion) -> Recursion {
    let at = Recursion::ALL.iter().position(|r| *r == now).unwrap_or(0);
    Recursion::ALL[(at + 1) % Recursion::ALL.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::UtcOffset;
    use diroxide_core::format::within_cells;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Timestamp {
        Timestamp::from_civil(year, month, day, hour, minute, 0, UtcOffset::UTC).unwrap()
    }

    fn form() -> TimeForm {
        TimeForm {
            side: Side::Left,
            targets: Vec::new(),
            what: "main.rs".into(),
            any_folder: true,
            now: Times {
                modified: Some(at(2026, 10, 10, 22, 9)),
                accessed: Some(at(2026, 10, 16, 8, 0)),
                created: None,
            },
            set: TimeSet::Modified,
            source: TimeSource::Now,
            recursion: Recursion::ThisItem,
            created: false,
            note: None,
        }
    }

    #[test]
    fn change_time_lists_its_settings_meaning_first_with_the_weekday() {
        let rows: Vec<String> = form()
            .rows(UtcOffset::UTC)
            .into_iter()
            .map(|r| r.label)
            .collect();
        let mut expected = vec![
            "Change the times of main.rs",
            "Change: modified only",
            "Set to: now",
            // October 10, 2026 is a Saturday: computed, never typed.
            "Modified: Saturday, 2026-10-10 22:09",
            "Accessed: Friday, 2026-10-16 08:00",
            "Created: not settable on this system",
        ];
        if cfg!(unix) {
            expected.push("Changed: set by the system");
        }
        expected.push("Apply to: this item only");
        assert_eq!(rows, expected);
        for row in &rows {
            assert!(within_cells(row, row), "{row}");
        }
        assert!(within_cells(
            CHANGE_TIME_REFUSAL,
            "Could not set the change time"
        ));
    }

    #[test]
    fn the_times_come_from_now_a_typed_date_or_the_other_file() {
        let now = at(2026, 10, 16, 9, 0);
        let mut form = form();
        form.set = TimeSet::ModifiedAccessed;
        assert_eq!(
            form.values(now),
            [(TimeKind::Modified, now), (TimeKind::Accessed, now)]
        );
        let typed = at(2026, 1, 2, 3, 4);
        form.source = TimeSource::Typed(typed);
        assert_eq!(
            form.rows(UtcOffset::UTC)[2].label,
            "Set to: Friday, 2026-01-02 03:04"
        );
        assert_eq!(
            form.values(now),
            [(TimeKind::Modified, typed), (TimeKind::Accessed, typed)]
        );
        // The other file's same time, else its modified time; a created
        // time only where the place can set one.
        let other_modified = at(2025, 5, 5, 5, 5);
        form.source = TimeSource::Other {
            name: "notes.txt".into(),
            times: Times {
                modified: Some(other_modified),
                accessed: None,
                created: None,
            },
        };
        assert_eq!(
            form.rows(UtcOffset::UTC)[2].label,
            "Set to: the times of notes.txt"
        );
        assert_eq!(
            form.values(now),
            [
                (TimeKind::Modified, other_modified),
                (TimeKind::Accessed, other_modified)
            ]
        );
        form.set = TimeSet::All;
        assert_eq!(form.values(now).len(), 2, "no created time here");
        assert_eq!(
            TimeSet::choices(false).len(),
            3,
            "all three and created only need a created time"
        );
        assert_eq!(TimeSet::choices(true).len(), 5);
    }

    #[test]
    fn change_owner_lists_now_and_the_new_names() {
        let mut form = OwnerForm {
            side: Side::Left,
            targets: Vec::new(),
            what: "main.rs".into(),
            any_folder: false,
            now_owner: Some("ada".into()),
            now_group: Some("staff".into()),
            owner: None,
            group: None,
            recursion: Recursion::ThisItem,
            local: true,
            note: None,
        };
        let me = Caller::user(1000, vec![1000, 50]);
        let labels = |form: &OwnerForm| -> Vec<String> {
            form.rows(&me).into_iter().map(|r| r.label).collect()
        };
        assert_eq!(
            labels(&form),
            [
                "Change owner of main.rs",
                "Now: owner ada, group staff",
                "Owner: ada, as now",
                "Group: staff, as now",
            ]
        );
        form.owner = Some("root".into());
        form.group = Some("50".into());
        let rows = labels(&form);
        assert_eq!(rows[2], "Owner: root");
        assert_eq!(rows[3], "Group: 50 (one of your groups)");
        assert!(within_cells(&rows[3], "Group: 50"));
        assert_eq!(group_words("51", &me), "Group: 51 (not one of yours)");
        assert_eq!(group_words("51", &Caller::administrator()), "Group: 51");
        assert_eq!(
            group_words("staff", &Caller::default()),
            "Group: staff",
            "a caller the system cannot name lets the system decide"
        );
        assert!(within_cells(OWNER_NOT_HERE, OWNER_NOT_HERE));
        assert!(within_cells(
            &security_dialog_line("report.pdf"),
            "Opening the Security dialog"
        ));
        assert!(within_cells(
            SECURITY_DIALOG_FAILED,
            "Could not open the Security dialog"
        ));
    }
}
