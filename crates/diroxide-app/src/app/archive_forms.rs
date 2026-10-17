//! The pack and unpack dialogs as lists of settings, and Test archive
//! (plan commit 61). Each dialog is a list the owner moves through with
//! the arrows: its first row says what will happen and runs it on Enter
//! ("Pack 2 items into reports.zip"), and every other row is a labeled
//! setting with its value, meaning first ("Format: zip", "Password: none
//! (zip names stay visible)"); Enter changes it. A setting the format
//! cannot have stays in the list and says why ("Solid: not for zip").
//! The plan then shows its rows under its question, which starts on No.

use diroxide_archive::{ArchiveFormat, CompressionLevel, FolderRule, PackRequest, UnpackRequest};
use diroxide_core::{
    Command, ConflictPolicy, Effect, JobKind, ListModel, ListRow, Message, Place, PlaceKind,
    Secret, SecretKind, SecretRequest, Side, catalog,
};

use super::{App, Flow, Purpose, Then};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::panel::place_leaf;
use crate::request::Request;

/// The settings of a pack, as its dialog shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackForm {
    pub side: Side,
    pub sources: Vec<Place>,
    /// The folder the archive goes into.
    pub folder: Place,
    /// Whether the folder is the other panel's (the orthodox target).
    pub other_panel: bool,
    pub name: String,
    pub format: ArchiveFormat,
    pub level: CompressionLevel,
    pub password: Option<Secret>,
    pub hide_names: bool,
    pub solid: bool,
    pub test_after: bool,
    pub note: Option<String>,
}

/// The settings of an unpack, as its dialog shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnpackForm {
    pub side: Side,
    pub archive: Place,
    pub members: Vec<String>,
    pub destination: Place,
    pub folder_rule: FolderRule,
    pub flatten: bool,
    pub conflicts: ConflictPolicy,
    pub restore_links: bool,
}

/// Which list of choices a pack setting opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackChoice {
    Format,
    Level,
}

const FORMATS: [ArchiveFormat; 6] = [
    ArchiveFormat::Zip,
    ArchiveFormat::SevenZ,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarXz,
    ArchiveFormat::Xz,
];

const LEVELS: [CompressionLevel; 5] = [
    CompressionLevel::Fastest,
    CompressionLevel::Fast,
    CompressionLevel::Normal,
    CompressionLevel::Maximum,
    CompressionLevel::Ultra,
];

const POLICIES: [ConflictPolicy; 5] = [
    ConflictPolicy::Ask,
    ConflictPolicy::Skip,
    ConflictPolicy::Overwrite,
    ConflictPolicy::KeepBoth,
    ConflictPolicy::Newer,
];

fn format_word(format: ArchiveFormat) -> &'static str {
    diroxide_archive::pack::format_word(format)
}

fn level_words(level: CompressionLevel, format: ArchiveFormat) -> String {
    match level.number_for(format) {
        Some(n) => format!("{} ({n} of 9)", level.word()),
        None => format!("none for {}", format_word(format)),
    }
}

fn policy_words(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Ask => "ask",
        ConflictPolicy::Skip => "skip it",
        ConflictPolicy::Overwrite => "replace it, the old one to the trash",
        ConflictPolicy::KeepBoth => "keep both",
        ConflictPolicy::Rename => "rename the new one",
        ConflictPolicy::Newer => "replace it if newer",
    }
}

fn folder_words(rule: FolderRule) -> &'static str {
    match rule {
        FolderRule::WhenSeveral => "made when several items are at the top",
        FolderRule::Always => "always made",
        FolderRule::Never => "never made",
    }
}

/// A name with its archive extension changed to the format's: a format
/// change keeps the name in step.
fn with_extension(name: &str, format: ArchiveFormat) -> String {
    let lower = name.to_lowercase();
    let stem = FORMATS
        .iter()
        .flat_map(|f| [f.extension().to_owned()])
        .chain([".tgz".to_owned(), ".txz".to_owned()])
        .filter(|ext| lower.ends_with(ext.as_str()) && lower.len() > ext.len())
        .max_by_key(String::len)
        .map_or(name, |ext| &name[..name.len() - ext.len()]);
    format!("{stem}{}", format.extension())
}

/// A name for the new archive: one source's own name without its
/// extension, or the folder's name for several.
fn first_name(sources: &[Place], folder_of_sources: &Place) -> String {
    let base = match sources {
        [one] => one
            .name()
            .map(|n| diroxide_core::names::split_extension(n).0.to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "archive".to_owned()),
        _ => folder_of_sources
            .name()
            .map(|n| diroxide_core::names::split_extension(n).0.to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "archive".to_owned()),
    };
    let base: String = base
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    format!("{base}.zip")
}

impl PackForm {
    fn summary(&self) -> String {
        format!(
            "Pack {} into {}",
            diroxide_core::format::count(self.sources.len() as u64, "item", "items"),
            diroxide_core::names::display_name(&self.name)
        )
    }

    /// The dialog's rows: what will happen, then each setting.
    fn list(&self) -> ListModel {
        let takes = diroxide_archive::pack::takes_password(self.format);
        let folder = diroxide_core::names::spell_out(&self.folder.to_string()).into_owned();
        let mut rows = vec![
            ListRow::new("go", self.summary()),
            ListRow::new(
                "name",
                format!("Name: {}", diroxide_core::names::display_name(&self.name)),
            ),
            ListRow::new("format", format!("Format: {}", format_word(self.format))),
            ListRow::new(
                "level",
                format!("Level: {}", level_words(self.level, self.format)),
            ),
            ListRow::new(
                "where",
                if self.other_panel {
                    format!("Where: the other panel, {folder}")
                } else {
                    format!("Where: {folder}")
                },
            ),
        ];
        rows.push(ListRow::new(
            "password",
            match (self.format, &self.password) {
                (ArchiveFormat::Zip, None) => "Password: none (zip names stay visible)".to_owned(),
                (ArchiveFormat::Zip, Some(_)) => "Password: AES-256 (zip names visible)".to_owned(),
                (ArchiveFormat::SevenZ, None) => "Password: none".to_owned(),
                (ArchiveFormat::SevenZ, Some(_)) => "Password: AES-256".to_owned(),
                (format, _) => format!("Password: not for {}; use 7z", format_word(format)),
            },
        ));
        rows.push(ListRow::new(
            "names",
            match (self.format, &self.password) {
                (ArchiveFormat::SevenZ, Some(_)) if self.hide_names => "Hide names: on".to_owned(),
                (ArchiveFormat::SevenZ, Some(_)) => "Hide names: off".to_owned(),
                (ArchiveFormat::SevenZ, None) => "Hide names: needs a password".to_owned(),
                (format, _) => format!("Hide names: not for {}", format_word(format)),
            },
        ));
        rows.push(ListRow::new(
            "solid",
            match self.format {
                ArchiveFormat::SevenZ if self.solid => "Solid: on".to_owned(),
                ArchiveFormat::SevenZ => "Solid: off".to_owned(),
                format => format!("Solid: not for {}", format_word(format)),
            },
        ));
        rows.push(ListRow::new(
            "test",
            if self.test_after {
                "Test after packing: on"
            } else {
                "Test after packing: off"
            },
        ));
        let _ = takes;
        let mut list = ListModel::new("Pack").with_rows(rows);
        list.set_help(
            "Up and Down move through the settings, Enter changes one, Enter on the first row packs, Escape closes.",
        );
        list
    }

    fn request(&self) -> Result<PackRequest, String> {
        let destination = self
            .folder
            .join(&self.name)
            .map_err(|e| Message::from_error("pack", &self.name, &e).text)?;
        let mut request = PackRequest::new(self.sources.clone(), destination, self.format);
        request.level = self.level;
        request.solid = self.format == ArchiveFormat::SevenZ && self.solid;
        request.password = self
            .password
            .clone()
            .filter(|_| diroxide_archive::pack::takes_password(self.format));
        request.encrypt_names =
            self.format == ArchiveFormat::SevenZ && request.password.is_some() && self.hide_names;
        request.test_after = self.test_after;
        request.conflicts = ConflictPolicy::Ask;
        Ok(request)
    }
}

impl UnpackForm {
    fn name(&self) -> String {
        self.archive.name().map_or_else(String::new, |n| {
            diroxide_core::names::display_name(n).into_owned()
        })
    }

    fn summary(&self) -> String {
        let what = if self.members.is_empty() {
            self.name()
        } else {
            format!(
                "{} of {}",
                diroxide_core::format::count(self.members.len() as u64, "item", "items"),
                self.name()
            )
        };
        format!("Unpack {what} into {}", place_leaf(&self.destination))
    }

    fn list(&self) -> ListModel {
        let folder = diroxide_core::names::spell_out(&self.destination.to_string()).into_owned();
        let rows = vec![
            ListRow::new("go", self.summary()),
            ListRow::new("where", format!("Where: the other panel, {folder}")),
            ListRow::new(
                "folder",
                format!("Folder: {}", folder_words(self.folder_rule)),
            ),
            ListRow::new(
                "paths",
                if self.flatten {
                    "Paths: names only, folders left out"
                } else {
                    "Paths: kept"
                },
            ),
            ListRow::new(
                "taken",
                format!("When a name is taken: {}", policy_words(self.conflicts)),
            ),
            ListRow::new(
                "links",
                if self.restore_links {
                    "Links: safe ones restored"
                } else {
                    "Links: left out, each one said"
                },
            ),
        ];
        let mut list = ListModel::new("Unpack").with_rows(rows);
        list.set_help(
            "Up and Down move through the settings, Enter changes one, Enter on the first row unpacks, Escape closes.",
        );
        list
    }

    fn request(&self) -> UnpackRequest {
        let mut request = UnpackRequest::new(self.archive.clone(), self.destination.clone());
        request.members = self.members.clone();
        request.folder_rule = self.folder_rule;
        request.flatten = self.flatten;
        request.conflicts = self.conflicts;
        request.restore_links = self.restore_links;
        request
    }
}

/// The next value in a cycle of choices.
fn next_of<T: Copy + PartialEq>(all: &[T], now: T) -> T {
    let at = all.iter().position(|v| *v == now).unwrap_or(0);
    all[(at + 1) % all.len()]
}

impl App {
    /// The archive a command acts on: the focused archive file, or, inside
    /// an archive, the archive itself.
    fn archive_here(&self, side: Side) -> Option<Place> {
        let panel = self.panel(side);
        if let Some((container, _)) = panel.place().archive() {
            return Some(container.clone());
        }
        let entry = panel.focused_entry()?;
        let place = panel.focused_place()?;
        (entry.kind == diroxide_core::EntryKind::Archive
            || ArchiveFormat::from_name(&entry.name).is_some())
        .then_some(place)
    }

    /// Pack: the dialog of its settings, for the marked entries or the
    /// focused one, into the other panel.
    pub(super) fn pack_dialog(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, note)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let here = self.panel(side).place().clone();
        let other = self.panel(side.other()).place().clone();
        let (folder, other_panel) = if other.kind() == PlaceKind::Archive {
            (local_folder_of(&here), false)
        } else {
            (other, true)
        };
        let name = first_name(&sources, &local_folder_of(&here));
        let form = PackForm {
            side,
            sources,
            folder,
            other_panel,
            name,
            format: ArchiveFormat::Zip,
            level: CompressionLevel::Normal,
            password: None,
            hide_names: false,
            solid: true,
            test_after: true,
            note,
        };
        self.show_pack_form(form, None)
    }

    fn show_pack_form(&mut self, form: PackForm, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list();
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::PACK),
            },
            Purpose::PackForm(Box::new(form)),
        )
    }

    /// The pack dialog again after one of its fields closed, the focus on
    /// that setting.
    pub(super) fn pack_row_reopen(&mut self, form: PackForm, focus: &str) -> Vec<Effect> {
        self.show_pack_form(form, Some(focus))
    }

    /// Enter on a row of the pack dialog.
    pub(super) fn pack_row(&mut self, mut form: PackForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "go" => effects.extend(self.pack_go(form)),
            "name" => {
                let field = EntryField::new("Archive name", CompletionSource::None)
                    .with_text(form.name.clone());
                effects.extend(
                    self.open_dialog(Dialog::edit(field), Purpose::PackName(Box::new(form))),
                );
            }
            "format" => effects.extend(self.pack_choices(form, PackChoice::Format)),
            "level" if form.level.number_for(form.format).is_some() => {
                effects.extend(self.pack_choices(form, PackChoice::Level));
            }
            "password" if diroxide_archive::pack::takes_password(form.format) => {
                if form.password.take().is_some() {
                    effects.extend(self.show_pack_form(form, Some("password")));
                    effects
                        .push(self.say(Message::info("Password: none. Nothing will be locked.")));
                } else {
                    effects.extend(self.new_password_prompt(form, None));
                }
            }
            "names" if form.format == ArchiveFormat::SevenZ && form.password.is_some() => {
                form.hide_names = !form.hide_names;
                effects.extend(self.show_pack_form(form, Some("names")));
            }
            "solid" if form.format == ArchiveFormat::SevenZ => {
                form.solid = !form.solid;
                effects.extend(self.show_pack_form(form, Some("solid")));
            }
            "test" => {
                form.test_after = !form.test_after;
                effects.extend(self.show_pack_form(form, Some("test")));
            }
            other => {
                // A setting the format cannot have: its row says why.
                let said = form
                    .list()
                    .rows()
                    .iter()
                    .find(|r| r.id == other)
                    .map(|r| r.label.clone())
                    .unwrap_or_default();
                effects.extend(self.show_pack_form(form, Some(other)));
                effects.push(self.say(Message::info(said)));
            }
        }
        effects
    }

    fn pack_choices(&mut self, form: PackForm, choice: PackChoice) -> Vec<Effect> {
        let (title, rows, current): (&str, Vec<ListRow>, String) = match choice {
            PackChoice::Format => (
                "Format",
                FORMATS
                    .iter()
                    .map(|f| ListRow::new(format_word(*f), format_word(*f)))
                    .collect(),
                format_word(form.format).to_owned(),
            ),
            PackChoice::Level => (
                "Level",
                LEVELS
                    .iter()
                    .map(|l| ListRow::new(l.word(), level_words(*l, form.format)))
                    .collect(),
                form.level.word().to_owned(),
            ),
        };
        let mut list = ListModel::new(title).with_rows(rows);
        list.focus_id(&current);
        list.set_help("Up and Down choose, Enter sets it, Escape keeps it.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::PACK),
            },
            Purpose::PackChoice(Box::new(form), choice),
        )
    }

    /// A choice made for a pack setting.
    pub(super) fn pack_chose(
        &mut self,
        mut form: PackForm,
        choice: PackChoice,
        row: &str,
    ) -> Vec<Effect> {
        let mut effects = self.close_top();
        let focus = match choice {
            PackChoice::Format => {
                if let Some(format) = FORMATS.iter().find(|f| format_word(**f) == row) {
                    form.format = *format;
                    form.name = with_extension(&form.name, *format);
                }
                "format"
            }
            PackChoice::Level => {
                if let Some(level) = LEVELS.iter().find(|l| l.word() == row) {
                    form.level = *level;
                }
                "level"
            }
        };
        effects.extend(self.show_pack_form(form, Some(focus)));
        effects
    }

    /// The archive's name typed: its format follows its extension.
    pub(super) fn pack_named(&mut self, mut form: PackForm, text: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let name = text.trim().to_owned();
        if name.is_empty() {
            effects.extend(self.show_pack_form(form, Some("name")));
            effects.push(self.say(Message::info("No name typed. The name stays.")));
            return effects;
        }
        if let Some(line) = self.name_refusal(form.side.other(), &name, None) {
            effects.extend(self.show_pack_form(form, Some("name")));
            effects.push(self.say(Message::error(format!("{line}. The name stays."))));
            return effects;
        }
        match ArchiveFormat::from_name(&name) {
            Some(format) => {
                form.format = format;
                form.name = name;
            }
            None => form.name = with_extension(&name, form.format),
        }
        effects.extend(self.show_pack_form(form, Some("name")));
        effects
    }

    /// The masked prompt for a new archive password, typed twice.
    fn new_password_prompt(&mut self, form: PackForm, first: Option<Secret>) -> Vec<Effect> {
        let request = if first.is_none() {
            diroxide_archive::new_password_request(&form.name)
        } else {
            // The second entry says it is the same password again.
            SecretRequest {
                echo: false,
                ..SecretRequest::new(
                    SecretKind::ServerPrompt,
                    format!("Same password again for {}", form.name),
                )
            }
        };
        self.open_dialog(
            Dialog::secret(request),
            Purpose::NewPassword {
                form: Box::new(form),
                first,
            },
        )
    }

    /// Enter in the new-password prompt: the first entry, then the same
    /// again; two that differ, or an empty one, say so and ask again.
    pub(super) fn new_password_entered(
        &mut self,
        mut form: PackForm,
        first: Option<Secret>,
        typed: Secret,
    ) -> Vec<Effect> {
        let mut effects = self.close_top();
        if typed.is_empty() {
            effects.push(self.say(Message::info(catalog::SECRET_EMPTY)));
            effects.extend(self.new_password_prompt(form, None));
            return effects;
        }
        match first {
            None => effects.extend(self.new_password_prompt(form, Some(typed))),
            Some(first) if first == typed => {
                form.password = Some(first);
                if form.format == ArchiveFormat::SevenZ {
                    form.hide_names = true;
                }
                effects.extend(self.show_pack_form(form, Some("password")));
                effects.push(self.say(Message::info(
                    "Password set. It is kept only for this pack.",
                )));
            }
            Some(_) => {
                effects.push(self.say(Message::info(catalog::SECRETS_DIFFER)));
                effects.extend(self.new_password_prompt(form, None));
            }
        }
        effects
    }

    /// Escape in the new-password prompt: back to the dialog, no password.
    pub(super) fn new_password_cancelled(&mut self, form: PackForm) -> Vec<Effect> {
        let mut effects = self.show_pack_form(form, Some("password"));
        effects.push(self.say(Message::info("No password set.")));
        effects
    }

    /// Enter on the pack dialog's first row: the plan, on a worker.
    fn pack_go(&mut self, form: PackForm) -> Vec<Effect> {
        let request = match form.request() {
            Ok(request) => request,
            Err(sentence) => {
                let mut effects = self.show_pack_form(form, Some("name"));
                effects.push(self.say(Message::error(sentence)));
                return effects;
            }
        };
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side: form.side,
                note: form.note.clone(),
                then: Then::Report,
                confirm: true,
                kind: JobKind::Pack,
            },
        );
        self.request(Request::PlanPack {
            job,
            request: Box::new(request),
        });
        vec![self.say(Message::status("Planning the pack"))]
    }

    /// Unpack: the dialog of its settings, for the focused archive (or,
    /// inside one, its marked members), into the other panel.
    pub(super) fn unpack_dialog(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some(archive) = self.archive_here(side) else {
            return vec![self.say(Message::info(
                "Not an archive. Unpack needs an archive file.",
            ))];
        };
        let destination = self.panel(side.other()).place().clone();
        if destination.kind() == PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        let members = if self.panel(side).place().kind() == PlaceKind::Archive
            && self.panel(side).list().selected_count() > 0
        {
            self.panel(side)
                .targets()
                .iter()
                .filter_map(|p| p.archive().map(|(_, m)| m.to_owned()))
                .filter(|m| !m.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        let form = UnpackForm {
            side,
            archive,
            members,
            destination,
            folder_rule: FolderRule::WhenSeveral,
            flatten: false,
            conflicts: ConflictPolicy::Ask,
            restore_links: false,
        };
        self.show_unpack_form(form, None)
    }

    fn show_unpack_form(&mut self, form: UnpackForm, focus: Option<&str>) -> Vec<Effect> {
        let mut list = form.list();
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::UNPACK),
            },
            Purpose::UnpackForm(Box::new(form)),
        )
    }

    /// Enter on a row of the unpack dialog.
    pub(super) fn unpack_row(&mut self, mut form: UnpackForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "go" => {
                let request = form.request();
                let job = self.next_job_id();
                self.flows.insert(
                    job,
                    Flow {
                        side: form.side,
                        note: None,
                        then: Then::Report,
                        confirm: true,
                        kind: JobKind::Unpack,
                    },
                );
                self.request(Request::PlanUnpack {
                    job,
                    request: Box::new(request),
                });
                effects.push(self.say(Message::status("Planning the unpack")));
                return effects;
            }
            "folder" => {
                form.folder_rule = next_of(
                    &[
                        FolderRule::WhenSeveral,
                        FolderRule::Always,
                        FolderRule::Never,
                    ],
                    form.folder_rule,
                );
            }
            "paths" => form.flatten = !form.flatten,
            "taken" => form.conflicts = next_of(&POLICIES, form.conflicts),
            "links" => form.restore_links = !form.restore_links,
            _ => {}
        }
        let focus = if row == "where" { "where" } else { row };
        effects.extend(self.show_unpack_form(form, Some(focus)));
        effects
    }

    /// Test archive: every member read and checked, nothing written; the
    /// verdict said ("Archive is sound: 10 members.").
    pub(super) fn test_archive(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some(archive) = self.archive_here(side) else {
            return vec![self.say(Message::info(
                "Not an archive. Test archive needs an archive file.",
            ))];
        };
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side,
                note: None,
                then: Then::Report,
                confirm: false,
                kind: JobKind::TestArchive,
            },
        );
        let name = archive.name().map_or_else(String::new, |n| {
            diroxide_core::names::display_name(n).into_owned()
        });
        let line = format!("Test started: {name}");
        let now = self.now();
        self.jobs
            .begin(job, JobKind::TestArchive, line.clone(), now);
        self.request(Request::TestArchive { job, archive });
        vec![
            Effect::Redraw(diroxide_core::Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }
}

/// The folder on this computer a place is in or at: an archive's folder.
fn local_folder_of(place: &Place) -> Place {
    match place.archive() {
        Some((container, _)) => match container.parent() {
            Some(parent) if parent.kind() != PlaceKind::Archive => parent,
            Some(parent) => local_folder_of(&parent),
            None => container.clone(),
        },
        None => place.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_change_keeps_the_name_in_step() {
        assert_eq!(
            with_extension("reports.zip", ArchiveFormat::SevenZ),
            "reports.7z"
        );
        assert_eq!(
            with_extension("logs.tar.gz", ArchiveFormat::TarXz),
            "logs.tar.xz"
        );
        assert_eq!(with_extension("notes", ArchiveFormat::Zip), "notes.zip");
        assert_eq!(with_extension("a.tgz", ArchiveFormat::Zip), "a.zip");
    }

    #[test]
    fn every_row_says_its_meaning_first_within_forty_cells() {
        let place = |t: &str| Place::parse_with(t, diroxide_core::place::PathStyle::Unix).unwrap();
        let form = PackForm {
            side: Side::Left,
            sources: vec![place("/d/a.txt"), place("/d/b.txt")],
            folder: place("/backup"),
            other_panel: true,
            name: "reports.zip".to_owned(),
            format: ArchiveFormat::Zip,
            level: CompressionLevel::Normal,
            password: None,
            hide_names: false,
            solid: true,
            test_after: true,
            note: None,
        };
        let said: Vec<String> = form.list().rows().iter().map(|r| r.label.clone()).collect();
        assert_eq!(
            said,
            [
                "Pack 2 items into reports.zip",
                "Name: reports.zip",
                "Format: zip",
                "Level: normal (6 of 9)",
                "Where: the other panel, /backup",
                "Password: none (zip names stay visible)",
                "Hide names: not for zip",
                "Solid: not for zip",
                "Test after packing: on",
            ]
        );
        for line in &said {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let unpack = UnpackForm {
            side: Side::Left,
            archive: place("/d/course.zip"),
            members: Vec::new(),
            destination: place("/out"),
            folder_rule: FolderRule::WhenSeveral,
            flatten: false,
            conflicts: ConflictPolicy::Ask,
            restore_links: false,
        };
        let said: Vec<String> = unpack
            .list()
            .rows()
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(said[0], "Unpack course.zip into out");
        assert!(said.iter().any(|l| l == "Links: left out, each one said"));
        for line in &said {
            let meaning = line.split(':').next().unwrap_or(line);
            assert!(diroxide_core::format::within_cells(line, meaning), "{line}");
        }
    }
}
