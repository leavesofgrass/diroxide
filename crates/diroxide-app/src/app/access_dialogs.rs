//! Links, permissions and the item information list (plan commit 66), as
//! lists of settings like every dialog of alpha.2: the first row says what
//! will happen and does it on Enter, the others are labeled settings that
//! Enter changes; the plan's rows come under its question, which starts on
//! No.
//!
//! - Make symbolic link: a link to each selected entry in the other panel,
//!   its kind (symbolic, hard, junction) and how its target is written
//!   (absolute, relative) in [`LinkEdit`], filled into the request; a
//!   folder link that fails for the privilege on Windows ("Cannot link:
//!   needs developer mode.") is offered a junction, starting on No.
//! - Change link target: the focused link pointed elsewhere, as one step
//!   Undo reverses.
//! - Change permissions: the owner's, the group's and others' rights, or
//!   on Windows the read-only, hidden and archive marks, and what the
//!   change applies to inside a folder, in an [`AttributeEdit`]; the
//!   plan's rows are the change in words ("Owner can read and write, group
//!   can read, others can read (644)").
//! - Details (Alt+Enter): the item information list from the preview's
//!   detail rows ("Type: zip archive", "Size: 48 KB (49,152 bytes)"), an
//!   archive's entries counted on a worker ("Entries: 10 items"), then the
//!   times, the attributes, the owner and the path.

use diroxide_core::access::{
    AttributeEdit, LinkEdit, LinkForm, LinkKind, ModeChange, Recursion, WindowsChange,
};
use diroxide_core::{
    Attributes, Command, Effect, EntryKind, JobKind, JobRequest, ListModel, ListRow, Message,
    Place, PlaceKind, Side,
};

use super::{App, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::panel::place_leaf;

/// The settings of a new link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkSettings {
    pub side: Side,
    pub targets: Vec<Place>,
    pub folder: Place,
    pub kind: LinkKind,
    pub form: LinkForm,
    pub note: Option<String>,
}

/// The settings of a change of permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccessForm {
    pub side: Side,
    pub targets: Vec<Place>,
    pub name: String,
    pub kind: EntryKind,
    /// The Unix mode now, or the Windows marks now.
    pub now: Attributes,
    /// The mode or the marks to set.
    pub then: u32,
    pub recursion: Recursion,
    pub any_folder: bool,
    pub note: Option<String>,
}

/// The rights a class can have, in the order Enter steps through them.
const RIGHTS: [(u32, &str); 5] = [
    (0o6, "read and write"),
    (0o4, "read only"),
    (0o7, "read, write and run"),
    (0o5, "read and run"),
    (0o0, "no access"),
];

fn rights_words(bits: u32) -> &'static str {
    RIGHTS
        .iter()
        .find(|(b, _)| *b == bits)
        .map_or("its own mix", |(_, w)| w)
}

fn next_rights(bits: u32) -> u32 {
    let at = RIGHTS
        .iter()
        .position(|(b, _)| *b == bits)
        .unwrap_or(RIGHTS.len() - 1);
    RIGHTS[(at + 1) % RIGHTS.len()].0
}

/// The Windows marks the dialog changes, with their words.
const MARKS: [(u32, &str, &str); 3] = [
    (diroxide_core::access::READ_ONLY, "read-only", "Read only"),
    (diroxide_core::access::HIDDEN, "hidden", "Hidden"),
    (
        diroxide_core::access::ARCHIVE,
        "archive",
        "Marked for backup",
    ),
];

impl LinkSettings {
    fn summary(&self) -> String {
        let what = match self.targets.as_slice() {
            [one] => one.name().map_or_else(String::new, |n| {
                diroxide_core::names::display_name(n).into_owned()
            }),
            many => diroxide_core::format::count(many.len() as u64, "item", "items"),
        };
        format!("Make a link to {what} in {}", place_leaf(&self.folder))
    }

    fn list(&self) -> ListModel {
        let folder = diroxide_core::names::spell_out(&self.folder.to_string()).into_owned();
        let rows = vec![
            ListRow::new("go", self.summary()),
            ListRow::new(
                "kind",
                match self.kind {
                    LinkKind::Symbolic => "Kind: symbolic, a pointer to the place",
                    LinkKind::Hard => "Kind: hard, a second name, one drive",
                    LinkKind::Junction => "Kind: junction, folders on Windows",
                },
            ),
            ListRow::new(
                "form",
                match self.form {
                    LinkForm::Absolute => "Target written: absolute, the full path",
                    LinkForm::Relative => "Target written: relative, from the link",
                },
            ),
            ListRow::new("where", format!("Where: the other panel, {folder}")),
        ];
        let mut list = ListModel::new("Make link").with_rows(rows);
        list.set_help(
            "Up and Down move through the settings, Enter changes one, Enter on the first row makes the links, Escape closes.",
        );
        list
    }
}

impl AccessForm {
    fn unix(&self) -> bool {
        matches!(self.now, Attributes::Unix(_))
    }

    fn summary(&self) -> String {
        match self.targets.len() {
            1 => format!("Change permissions of {}", self.name),
            n => format!(
                "Change permissions of {}",
                diroxide_core::format::count(n as u64, "item", "items")
            ),
        }
    }

    fn list(&self) -> ListModel {
        let mut rows = vec![ListRow::new("go", self.summary())];
        match self.now {
            Attributes::Unix(mode) => {
                rows.push(ListRow::new(
                    "now",
                    format!("Now: {}", diroxide_ops::access::mode_line(mode, self.kind)),
                ));
                for (id, label, shift) in [
                    ("owner", "Owner", 6),
                    ("group", "Group", 3),
                    ("others", "Others", 0),
                ] {
                    let bits = (self.then >> shift) & 0o7;
                    rows.push(ListRow::new(id, format!("{label}: {}", rights_words(bits))));
                }
            }
            Attributes::Windows(_) => {
                for (bit, id, label) in MARKS {
                    let on = self.then & bit != 0;
                    rows.push(ListRow::new(
                        id,
                        format!("{label}: {}", if on { "on" } else { "off" }),
                    ));
                }
            }
        }
        if self.any_folder {
            rows.push(ListRow::new("apply", self.recursion.row()));
        }
        let mut list = ListModel::new("Permissions").with_rows(rows);
        list.set_help(
            "Up and Down move through the settings, Enter changes one, Enter on the first row plans the change, Escape closes.",
        );
        list
    }

    fn edit(&self) -> Option<AttributeEdit> {
        let edit = AttributeEdit::new(self.recursion);
        match self.now {
            Attributes::Unix(mode) => {
                let mode = mode & 0o777;
                let then = self.then & 0o777;
                if mode == then && !self.recursion.goes_inside() {
                    return None;
                }
                Some(edit.with_mode(ModeChange::Bits {
                    on: then & !mode,
                    off: mode & !then,
                }))
            }
            Attributes::Windows(bits) => {
                if bits == self.then && !self.recursion.goes_inside() {
                    return None;
                }
                Some(edit.with_windows(WindowsChange {
                    on: self.then & !bits,
                    off: bits & !self.then,
                }))
            }
        }
    }
}

impl App {
    /// Make symbolic link: the dialog, for the marked entries or the
    /// focused one, the links in the other panel.
    pub(super) fn link_dialog(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((targets, note)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let folder = self.panel(side.other()).place().clone();
        if folder.kind() == PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        let form = LinkSettings {
            side,
            targets,
            folder,
            kind: LinkKind::Symbolic,
            form: LinkForm::Absolute,
            note,
        };
        self.show_link_form(form, None)
    }

    fn show_link_form(&mut self, form: LinkSettings, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list();
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::MAKE_LINK),
            },
            Purpose::LinkForm(Box::new(form)),
        )
    }

    /// Enter on a row of the link dialog.
    pub(super) fn link_row(&mut self, mut form: LinkSettings, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "go" => {
                let job = self.next_job_id();
                let request = JobRequest::new(job, JobKind::Link, form.targets.clone())
                    .to(form.folder.clone())
                    .linking(LinkEdit::new(form.kind, form.form));
                // Only a folder can be a junction, so only folders are
                // offered one if the privilege refuses the links.
                let panel = self.panel(form.side);
                let folders: Vec<Place> = form
                    .targets
                    .iter()
                    .filter(|p| {
                        p.name()
                            .and_then(|n| panel.entry(n))
                            .is_some_and(|e| e.kind == EntryKind::Folder)
                    })
                    .cloned()
                    .collect();
                let then = Then::Linked {
                    targets: folders,
                    folder: form.folder.clone(),
                    form: form.form,
                };
                effects.extend(self.plan_job(form.side, request, form.note.clone(), then, true));
                return effects;
            }
            "kind" => {
                form.kind = match form.kind {
                    LinkKind::Symbolic => LinkKind::Hard,
                    LinkKind::Hard => LinkKind::Junction,
                    LinkKind::Junction => LinkKind::Symbolic,
                };
            }
            "form" => {
                form.form = match form.form {
                    LinkForm::Absolute => LinkForm::Relative,
                    LinkForm::Relative => LinkForm::Absolute,
                };
            }
            _ => {}
        }
        effects.extend(self.show_link_form(form, Some(row)));
        effects
    }

    /// A link job ended with the privilege's refusal: a folder is offered a
    /// junction instead, starting on No.
    pub(super) fn offer_junction(&mut self, targets: Vec<Place>, folder: Place) -> Vec<Effect> {
        let question = with_default_no(diroxide_ops::links::junction_question());
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing: vec![diroxide_ops::links::NEEDS_DEVELOPER_MODE.to_owned()],
                on_yes: Command::new(ids::YES),
            },
            Purpose::JunctionInstead { targets, folder },
        )
    }

    /// Yes to the junction: the same links as junctions.
    pub(super) fn make_junctions(&mut self, targets: Vec<Place>, folder: Place) -> Vec<Effect> {
        let side = self.active;
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Link, targets)
            .to(folder)
            .linking(LinkEdit::new(LinkKind::Junction, LinkForm::Absolute));
        self.plan_job(side, request, None, Then::Report, false)
    }

    /// Change link target: the new target typed, with path completion.
    pub(super) fn relink_prompt(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if entry.kind != EntryKind::Link {
            let text = format!("Not a link: {}", entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let now = entry
            .link
            .as_ref()
            .map(|l| l.target.clone())
            .unwrap_or_default();
        let label = format!("Point {} to", entry.display_name());
        let field = EntryField::new(label, CompletionSource::Paths).with_text(now);
        self.open_dialog(Dialog::edit(field), Purpose::Relink { side, link: place })
    }

    /// The new target typed: the change planned, one step Undo reverses.
    pub(super) fn relink(&mut self, side: Side, link: Place, text: &str) -> Vec<Effect> {
        let target = match self.typed_place(side, text) {
            Ok(target) => target,
            Err(message) => return vec![self.say(message)],
        };
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Link, vec![link])
            .to(target)
            .linking(LinkEdit::retarget(LinkForm::Absolute));
        self.plan_job(side, request, None, Then::Report, true)
    }

    /// Change permissions: the dialog, for the marked entries or the
    /// focused one.
    pub(super) fn permissions_dialog(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((targets, note)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if self.panel(side).place().kind() == PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        let panel = self.panel(side);
        let entries: Vec<_> = targets
            .iter()
            .filter_map(|p| p.name().and_then(|n| panel.entry(n)))
            .collect();
        let Some(first) = entries.first() else {
            return vec![self.say(Message::status(panel.list().empty_text().to_owned()))];
        };
        if first.kind == EntryKind::Link {
            return vec![self.say(crate::messages::permissions::links_have_none(
                &first.display_name(),
            ))];
        }
        let Some(now) = first.attributes else {
            let text = format!(
                "Could not change permissions of {}: this place does not say them.",
                first.display_name()
            );
            return vec![self.say(Message::error(text))];
        };
        let then = match now {
            Attributes::Unix(mode) => mode & 0o777,
            Attributes::Windows(bits) => bits,
        };
        let form = AccessForm {
            side,
            name: first.display_name().into_owned(),
            kind: first.kind,
            any_folder: entries.iter().any(|e| e.kind == EntryKind::Folder),
            targets,
            now,
            then,
            recursion: Recursion::ThisItem,
            note,
        };
        self.show_access_form(form, None)
    }

    fn show_access_form(&mut self, form: AccessForm, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list();
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::PERMISSIONS),
            },
            Purpose::AccessForm(Box::new(form)),
        )
    }

    /// Enter on a row of the permissions dialog.
    pub(super) fn access_row(&mut self, mut form: AccessForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let shift = match row {
            "owner" => Some(6),
            "group" => Some(3),
            "others" => Some(0),
            _ => None,
        };
        if let Some(shift) = shift.filter(|_| form.unix()) {
            let bits = (form.then >> shift) & 0o7;
            form.then = (form.then & !(0o7 << shift)) | (next_rights(bits) << shift);
        } else if let Some((bit, _, _)) = MARKS.iter().find(|(_, id, _)| *id == row) {
            form.then ^= bit;
        } else if row == "apply" {
            let at = Recursion::ALL
                .iter()
                .position(|r| *r == form.recursion)
                .unwrap_or(0);
            form.recursion = Recursion::ALL[(at + 1) % Recursion::ALL.len()];
        } else if row == "go" {
            let Some(edit) = form.edit() else {
                effects.push(self.say(crate::messages::permissions::nothing_to_change()));
                return effects;
            };
            let job = self.next_job_id();
            let request =
                JobRequest::new(job, JobKind::SetAttributes, form.targets.clone()).changing(edit);
            effects.extend(self.plan_job(
                form.side,
                request,
                form.note.clone(),
                Then::Report,
                true,
            ));
            return effects;
        }
        effects.extend(self.show_access_form(form, Some(row)));
        effects
    }

    /// Copy path or name of the focused entry: the text on the system
    /// clipboard, and what was copied said.
    pub(super) fn copy_path_or_name(&mut self, name_only: bool) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let (text, said) = if name_only {
            (
                entry.name.clone(),
                format!("Name copied: {}", entry.display_name()),
            )
        } else {
            let path = place.to_string();
            let said = format!("Path copied: {}", diroxide_core::names::display_name(&path));
            (path, said)
        };
        vec![
            Effect::SetClipboardText(text),
            self.say(Message::info(said)),
        ]
    }

    /// An archive's entries counted for the open details list: the row
    /// goes in after the size.
    pub(super) fn archive_entries_counted(
        &mut self,
        place: &Place,
        line: Result<String, Message>,
    ) -> Vec<Effect> {
        let Some(Purpose::Details(shown)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return Vec::new();
        };
        if &shown != place {
            return Vec::new();
        }
        let line = match line {
            Ok(line) => line,
            Err(message) => format!("Entries: {}", message.text),
        };
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() {
            let mut rows = list.rows().to_vec();
            let at = rows
                .iter()
                .position(|r| r.label.starts_with("Size: "))
                .map_or(rows.len().min(2), |i| i + 1);
            rows.insert(at, ListRow::new("entries", line.clone()));
            let focus = list.focused().map(|r| r.id.clone());
            let title = list.title().to_owned();
            let help = list.help().to_owned();
            let mut updated = ListModel::new(title).with_rows(rows);
            updated.set_help(help);
            if let Some(focus) = focus {
                updated.focus_id(&focus);
            }
            *list = updated;
        }
        vec![
            Effect::Redraw(diroxide_core::Redraw::All),
            self.say(Message::status(line)),
        ]
    }
}
