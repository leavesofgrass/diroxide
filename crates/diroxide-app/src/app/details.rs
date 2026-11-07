//! The details view (alpha 4, R30 commit 41; R28 2.2 and 3.2): Details
//! lists every fact about the focused entry as a row in the order of
//! facts, Say Details says its summary in one line, Copy details puts its
//! rows on the clipboard, and Show all, the list's last row, adds every
//! tag ExifTool reads.
//!
//! - **The list opens at once** with what the panel knows (the type, the
//!   name, the size, the dates, the place, the owner), "Privacy: reading"
//!   and "Checksum: not computed"; the file's own fields come from a
//!   worker ([`crate::details`]) and join the list in their groups,
//!   the focus kept on its row, the status line saying how many came.
//! - **Groups, in order** (core's `DetailGroup`, R12's order of facts):
//!   what it is, what is in it, size, dates, privacy and origin, the
//!   owner's own (labels), place, technical, system (the owner, the
//!   checksum), and how it was read ("Not read: picture facts, the file
//!   ends too soon"). A fact the file does not have is not listed, except
//!   the privacy and checksum rows.
//! - **Say Details** says the summary only ("PNG, 1920 by 1080, 8-bit
//!   RGBA, 2.3 MB"); the focus stays where it is.
//! - **Copy details** copies the rows shown, or reads them first from a
//!   panel, private rows left out ("Copied 14 details, 2 private left
//!   out"); **Copy all details** keeps them.
//! - Neither has a default key (R30 7.2): the palette, the File menu, a
//!   bank button.

use diroxide_core::details::{DetailGroup, DetailRow};
use diroxide_core::format::count;
use diroxide_core::{
    Command, Effect, Entry, EntryKind, ListModel, ListRow, Message, Place, Redraw, Timestamp,
};
use diroxide_filetype::FileType;

use super::{App, Purpose};
use crate::commands::ids;
use crate::details::{
    self as words, DetailsAsk, DetailsNote, NOT_COMPUTED, READING, SHOW_ALL, SHOW_ALL_READING,
    SHOW_ALL_ROW, Wanted,
};
use crate::dialog::Dialog;
use crate::request::Request;

/// What the details list and its commands wait for.
#[derive(Clone, Debug, Default)]
pub(super) struct DetailsState {
    /// The last request's number.
    next: u64,
    /// The details list open now.
    shown: Option<Shown>,
    /// A Copy details from a panel, waiting for the file's fields.
    copy: Option<Copying>,
    /// A Say Details waiting for its summary.
    summary: Option<u64>,
    /// A Read text in picture waiting for Tesseract: its job.
    picture: Option<diroxide_core::JobId>,
    /// How many words Tesseract read in each picture this session, for
    /// the details row (alpha 4, R30 commit 44).
    picture_words: std::collections::HashMap<Place, usize>,
}

/// The details list's facts.
#[derive(Clone, Debug)]
struct Shown {
    place: Place,
    title: String,
    /// Whether it is a file or an archive: only those get the privacy,
    /// checksum and Show all rows.
    file: bool,
    /// The request for its fields.
    generation: u64,
    /// What the panel knows.
    base: Vec<DetailRow>,
    /// The fields read, once they came.
    read: Option<Vec<DetailRow>>,
    /// What could not be read.
    unread: Vec<String>,
    /// An archive's entries as the archive provider counted them.
    entries: Option<String>,
    /// Show all.
    more: More,
}

/// Show all's state.
#[derive(Clone, Debug, Default)]
enum More {
    #[default]
    NotAsked,
    Reading(u64),
    Rows(Vec<DetailRow>),
    Said(String),
}

/// A Copy details waiting for the fields.
#[derive(Clone, Debug)]
struct Copying {
    generation: u64,
    title: String,
    base: Vec<DetailRow>,
    all: bool,
}

fn time_words(time: Option<Timestamp>, clock: &dyn diroxide_core::Clock) -> Option<String> {
    time.map(|t| {
        let civil = t.to_civil(clock.offset());
        format!("{}, {}", civil.long_date(), civil.clock_time())
    })
}

/// Whether the entry gets the file's own rows: a file or an archive.
fn file_like(entry: &Entry) -> bool {
    matches!(entry.kind, EntryKind::File | EntryKind::Archive)
}

/// Every row in the order of facts: what the panel knows, the fields
/// read, the archive's count, a file's privacy row ("reading" until the
/// fields come) and what was not read.
fn assemble(
    base: &[DetailRow],
    read: Option<&[DetailRow]>,
    entries: Option<&str>,
    file: bool,
    unread: &[String],
) -> Vec<DetailRow> {
    let mut rows = base.to_vec();
    if let Some(read) = read {
        rows.extend(read.iter().cloned());
    }
    if let Some(entries) = entries {
        rows.push(DetailRow::new(
            "archive.entries",
            "Entries",
            entries,
            DetailGroup::Content,
        ));
    }
    if file {
        let privacy = read.map_or_else(|| READING.to_owned(), words::privacy_words);
        rows.push(DetailRow::new(
            "file.privacy",
            "Privacy",
            privacy,
            DetailGroup::Flags,
        ));
    }
    rows.extend(
        unread
            .iter()
            .enumerate()
            .map(|(i, line)| words::unread_row(i, line)),
    );
    words::ordered(rows)
}

impl Shown {
    /// Every row in the order of facts ([`assemble`]).
    fn rows(&self) -> Vec<DetailRow> {
        assemble(
            &self.base,
            self.read.as_deref(),
            self.entries.as_deref(),
            self.file,
            &self.unread,
        )
    }

    /// Every row, Show all's own after the rest.
    fn all_rows(&self) -> Vec<DetailRow> {
        let mut rows = self.rows();
        if let More::Rows(more) = &self.more {
            rows.extend(more.iter().cloned());
        }
        rows
    }

    /// The list: each row its words, the label first; Show all last,
    /// then the tags it read.
    fn list(&self) -> ListModel {
        let mut list_rows: Vec<ListRow> = self
            .rows()
            .iter()
            .map(|row| ListRow::new(row.key.clone(), words::row_text(row)))
            .collect();
        if self.file {
            let line = match &self.more {
                More::NotAsked => SHOW_ALL.to_owned(),
                More::Reading(_) => SHOW_ALL_READING.to_owned(),
                More::Rows(more) => format!(
                    "Show all: {} below, from ExifTool",
                    count(more.len() as u64, "tag", "tags")
                ),
                More::Said(line) => line.clone(),
            };
            list_rows.push(ListRow::new(SHOW_ALL_ROW, line));
            if let More::Rows(more) = &self.more {
                list_rows.extend(
                    more.iter()
                        .map(|row| ListRow::new(row.key.clone(), words::row_text(row))),
                );
            }
        }
        let mut list = ListModel::new(self.title.clone()).with_rows(list_rows);
        list.set_help(if self.file {
            "Up and Down read, Enter on Show all asks ExifTool for every tag, Escape closes."
        } else {
            "Up and Down read, Escape closes."
        });
        list
    }
}

impl App {
    /// The details list's commands; `None` for commands that are not
    /// theirs.
    pub(super) fn details_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        if *id == ids::READ_PICTURE_TEXT {
            if !self.dialogs.is_empty() {
                return Some(vec![self.say(Message::status("Close this dialog first."))]);
            }
            return Some(self.read_picture_text());
        }
        if *id != ids::SAY_DETAILS && *id != ids::COPY_DETAILS && *id != ids::COPY_ALL_DETAILS {
            return None;
        }
        let in_details = matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Details(_))
        );
        if !self.dialogs.is_empty() && !in_details {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(if *id == ids::SAY_DETAILS {
            self.say_details()
        } else {
            self.copy_details(*id == ids::COPY_ALL_DETAILS)
        })
    }

    fn next_details(&mut self) -> u64 {
        self.details.next += 1;
        self.details.next
    }

    /// The focused entry, its place and its type, or what is said instead.
    fn details_target(&self) -> Result<(Entry, Place, Option<FileType>), String> {
        let panel = self.panel(self.active);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            return Err(panel.list().empty_text().to_owned());
        };
        // The type the background check settled on, else the name's
        // (lane TA).
        let file_type = self
            .types
            .by_index(entry.type_index)
            .or_else(|| {
                self.types
                    .detect(&entry.name, &[])
                    .and_then(|d| self.types.get(&d.type_id))
            })
            .cloned();
        Ok((entry.clone(), place, file_type))
    }

    /// A details request for the runtime.
    fn ask_details(
        &mut self,
        entry: &Entry,
        place: &Place,
        file_type: Option<FileType>,
        wanted: Wanted,
    ) -> u64 {
        let generation = self.next_details();
        let tools = self.tool_runner();
        let ask = DetailsAsk {
            generation,
            place: place.clone(),
            entry: entry.clone(),
            file_type,
            tools,
            cache: Some(self.preview.cache.clone()),
            wanted,
        };
        self.request(Request::Details(Box::new(ask)));
        generation
    }

    /// What the panel knows about an entry, as rows: its type, name, size,
    /// dates, flags, labels, place and owner, and the checksum row.
    fn base_rows(
        &self,
        entry: &Entry,
        place: &Place,
        file_type: Option<&FileType>,
    ) -> Vec<DetailRow> {
        let panel = self.panel(self.active);
        let clock = self.clock.as_ref();
        let mut rows = diroxide_preview::detail_rows(entry, file_type);
        rows.push(DetailRow::new(
            "name",
            "Name",
            entry.display_name().into_owned(),
            DetailGroup::Identity,
        ));
        if entry.needs_care {
            let issues = diroxide_vfs::check_name(&entry.name, &panel.names());
            for (i, issue) in issues.iter().enumerate() {
                rows.push(DetailRow::new(
                    format!("name.issue.{i}"),
                    "",
                    issue.line(),
                    DetailGroup::Identity,
                ));
            }
            rows.push(DetailRow::new(
                "name.spelled",
                "Spelled out",
                diroxide_core::names::spell_out(&entry.name),
                DetailGroup::Identity,
            ));
        }
        // The file's groups, its type's first ("File group: Images,
        // Screenshots"), and the line when its name and its first bytes
        // disagree (R28 4.3, lane TA).
        if let Ok(groups) = self.types.groups_of(entry, clock)
            && !groups.is_empty()
        {
            let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
            rows.push(DetailRow::new(
                "file.groups",
                "File group",
                names.join(", "),
                DetailGroup::Identity,
            ));
        }
        if let Some(line) = panel.disagreement(&entry.name) {
            rows.push(DetailRow::new(
                "file.type-check",
                "",
                line,
                DetailGroup::Flags,
            ));
        }
        if entry.kind == EntryKind::Folder {
            rows.push(DetailRow::new(
                "size",
                "Size",
                "folder size not computed",
                DetailGroup::Size,
            ));
        }
        for (key, label, time) in [
            ("time.modified", "Modified", entry.modified),
            ("time.created", "Created", entry.created),
            ("time.accessed", "Accessed", entry.accessed),
        ] {
            if let Some(words) = time_words(time, clock) {
                rows.push(DetailRow::new(key, label, words, DetailGroup::Dates));
            }
        }
        if let Some(attributes) = entry.attributes {
            rows.push(DetailRow::new(
                "file.attributes",
                "Attributes",
                attributes.words(entry.kind),
                DetailGroup::Flags,
            ));
        }
        if entry.hidden {
            rows.push(DetailRow::new(
                "file.hidden",
                "Hidden",
                "yes",
                DetailGroup::Flags,
            ));
        }
        let labels = self.organize.labels.labels_of(place);
        if !labels.is_empty() {
            rows.push(DetailRow::new(
                "labels",
                "Labels",
                labels.join(", "),
                DetailGroup::Yours,
            ));
        }
        // The owner's note and named fields beside the labels (R30 commit
        // 51): "Note: call about the invoice", "Project: Apollo".
        if let Some(note) = self.organize.labels.note_of(place) {
            rows.push(DetailRow::new(
                crate::labels::NOTE_KEY,
                "Note",
                note,
                DetailGroup::Yours,
            ));
        }
        if let Some(fields) = self.organize.labels.fields_of(place) {
            for (name, value) in fields {
                rows.push(DetailRow::new(
                    crate::labels::field_key(name),
                    crate::labels::field_label(name),
                    value.clone(),
                    DetailGroup::Yours,
                ));
            }
        }
        rows.push(DetailRow::new(
            "path",
            "Path",
            diroxide_core::names::spell_out(&place.to_string()),
            DetailGroup::Place,
        ));
        // What Tesseract read in it this session (R30 commit 44): "Text
        // in picture (machine read): 14 words".
        if let Some(words) = self.details.picture_words.get(place) {
            rows.push(DetailRow::new(
                "picture.text",
                crate::picture_text::TITLE,
                count(*words as u64, "word", "words"),
                DetailGroup::Content,
            ));
        }
        if let Some(owner) = &entry.owner {
            rows.push(DetailRow::new(
                "file.owner",
                "Owner",
                owner.clone(),
                DetailGroup::System,
            ));
        }
        if let Some(group) = &entry.group {
            rows.push(DetailRow::new(
                "file.owner-group",
                "Group",
                group.clone(),
                DetailGroup::System,
            ));
        }
        if file_like(entry) {
            // The value the Checksum command computed this session, while
            // the file is as it was (lane MA): "SHA-256: ba78 16bf, ending
            // 15ad".
            let value = self
                .known_checksum(place, entry)
                .unwrap_or_else(|| NOT_COMPUTED.to_owned());
            rows.push(DetailRow::new(
                "file.checksum",
                "Checksum",
                value,
                DetailGroup::System,
            ));
        }
        rows
    }

    /// Details: every fact about the focused entry, one a row, in the
    /// order of facts; the file's own fields read on a worker and added
    /// as they come.
    pub(super) fn details(&mut self) -> Vec<Effect> {
        let (entry, place, file_type) = match self.details_target() {
            Ok(target) => target,
            Err(empty) => return vec![self.say(Message::status(empty))],
        };
        let base = self.base_rows(&entry, &place, file_type.as_ref());
        let file = file_like(&entry);
        let generation = if file {
            self.ask_details(&entry, &place, file_type, Wanted::Fields)
        } else {
            self.next_details()
        };
        let shown = Shown {
            place: place.clone(),
            title: format!("Details of {}", entry.display_name()),
            file,
            generation,
            base,
            read: (!file).then(Vec::new),
            unread: Vec::new(),
            entries: None,
            more: More::NotAsked,
        };
        let list = shown.list();
        self.details.shown = Some(shown);
        let effects = self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::DETAILS),
            },
            Purpose::Details(place.clone()),
        );
        if entry.kind == EntryKind::Archive {
            self.request(Request::ArchiveEntries(place));
        }
        effects
    }

    /// Puts the open details list's rows in place again, the focus kept on
    /// its row; `focus` moves it to a row by id instead.
    fn refresh_details(&mut self, focus: Option<&str>) {
        let Some(shown) = self.details.shown.as_ref() else {
            return;
        };
        let place = shown.place.clone();
        let fresh = shown.list();
        let index = self
            .opens
            .iter()
            .rposition(|o| matches!(&o.purpose, Purpose::Details(p) if *p == place));
        if let Some(index) = index
            && let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index)
        {
            list.set_rows(fresh.rows().to_vec());
            list.set_help(fresh.help().to_owned());
            if let Some(id) = focus {
                list.focus_id(id);
            }
        }
    }

    /// The details list open on this place, if it is.
    fn shown_on(&mut self, place: &Place) -> Option<&mut Shown> {
        let open = self
            .opens
            .iter()
            .any(|o| matches!(&o.purpose, Purpose::Details(p) if p == place));
        self.details
            .shown
            .as_mut()
            .filter(|s| open && s.place == *place)
    }

    /// An archive's entries as the archive provider counted them: "Entries:
    /// 10 items", or why not, in the open list and on the status line.
    pub(super) fn details_entries_counted(&mut self, place: &Place, line: String) -> Vec<Effect> {
        let Some(shown) = self.shown_on(place) else {
            return Vec::new();
        };
        let value = line.strip_prefix("Entries: ").unwrap_or(&line).to_owned();
        shown.entries = Some(value);
        self.refresh_details(None);
        vec![Effect::Redraw(Redraw::All), self.say(Message::status(line))]
    }

    /// What a details worker read.
    pub(super) fn details_note(&mut self, note: DetailsNote) -> Vec<Effect> {
        match note {
            DetailsNote::Fields {
                generation,
                place,
                rows,
                unread,
            } => {
                if let Some(copy) = self.details.copy.take_if(|c| c.generation == generation) {
                    return self.finish_copy(copy, rows, unread);
                }
                let Some(shown) = self.shown_on(&place).filter(|s| s.generation == generation)
                else {
                    return Vec::new();
                };
                let read = rows.len() as u64;
                shown.read = Some(rows);
                shown.unread = unread;
                self.refresh_details(None);
                let line = format!("Read {}", count(read, "more detail", "more details"));
                vec![Effect::Redraw(Redraw::All), self.say(Message::status(line))]
            }
            DetailsNote::Summary {
                generation, line, ..
            } => {
                if self.details.summary != Some(generation) {
                    return Vec::new();
                }
                self.details.summary = None;
                vec![self.say(Message::info(line))]
            }
            DetailsNote::ShowAll {
                generation,
                place,
                result,
            } => {
                let Some(shown) = self
                    .shown_on(&place)
                    .filter(|s| matches!(s.more, More::Reading(g) if g == generation))
                else {
                    return Vec::new();
                };
                match result {
                    Ok(rows) if rows.is_empty() => {
                        let line = "More details: ExifTool found no more".to_owned();
                        shown.more = More::Said(line.clone());
                        self.refresh_details(None);
                        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
                    }
                    Ok(rows) => {
                        let first = rows.first().map(|r| r.key.clone());
                        let line = format!(
                            "{}, from ExifTool",
                            count(rows.len() as u64, "more tag", "more tags")
                        );
                        shown.more = More::Rows(rows);
                        self.refresh_details(first.as_deref());
                        let mut effects =
                            vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))];
                        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last()
                            && let Some(row) = list.focused()
                        {
                            effects.push(self.say(Message::info(row.label.clone())));
                        }
                        effects
                    }
                    Err(line) => {
                        shown.more = More::Said(line.clone());
                        self.refresh_details(None);
                        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
                    }
                }
            }
        }
    }

    /// Enter on Show all: ExifTool's tags for the shown file, read once.
    pub(super) fn details_show_all(&mut self, place: &Place) -> Vec<Effect> {
        let Some(shown) = self.shown_on(place) else {
            return Vec::new();
        };
        match &shown.more {
            More::NotAsked | More::Said(_) => {}
            More::Reading(_) => {
                return vec![self.say(Message::status(SHOW_ALL_READING))];
            }
            More::Rows(rows) => {
                let first = rows.first().map(|r| r.key.clone());
                if let Some(id) = first
                    && let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut()
                {
                    list.focus_id(&id);
                }
                return vec![Effect::Redraw(Redraw::All)];
            }
        }
        let place = place.clone();
        let entry = Entry::new(place.name().unwrap_or_default(), EntryKind::File);
        let generation = self.ask_details(&entry, &place, None, Wanted::ShowAll);
        if let Some(shown) = self.shown_on(&place) {
            shown.more = More::Reading(generation);
        }
        self.refresh_details(None);
        vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::status(SHOW_ALL_READING)),
        ]
    }

    /// Say Details: the focused file's summary in one line ("PNG, 1920 by
    /// 1080, 8-bit RGBA, 2.3 MB"), read on a worker with the previews'
    /// cache; the focus stays. In the details list, the shown file's.
    fn say_details(&mut self) -> Vec<Effect> {
        let (entry, place, file_type) = match self.details_target() {
            Ok(target) => target,
            Err(empty) => return vec![self.say(Message::status(empty))],
        };
        if !file_like(&entry) {
            let line = diroxide_preview::details(&entry, file_type.as_ref());
            return vec![self.say(Message::info(line))];
        }
        let generation = self.ask_details(&entry, &place, file_type, Wanted::Summary);
        self.details.summary = Some(generation);
        Vec::new()
    }

    /// Copy details: the details list's rows when it is open, else the
    /// focused entry's, read first; private rows left out unless `all`.
    fn copy_details(&mut self, all: bool) -> Vec<Effect> {
        if let Some(Purpose::Details(place)) = self.opens.last().map(|o| o.purpose.clone())
            && let Some(shown) = self.shown_on(&place)
        {
            let title = shown.title.clone();
            let rows = shown.all_rows();
            return self.put_details(&title, &rows, all);
        }
        let (entry, place, file_type) = match self.details_target() {
            Ok(target) => target,
            Err(empty) => return vec![self.say(Message::status(empty))],
        };
        let title = format!("Details of {}", entry.display_name());
        let base = self.base_rows(&entry, &place, file_type.as_ref());
        if !file_like(&entry) {
            let rows = words::ordered(base);
            return self.put_details(&title, &rows, all);
        }
        let generation = self.ask_details(&entry, &place, file_type, Wanted::Fields);
        self.details.copy = Some(Copying {
            generation,
            title,
            base,
            all,
        });
        vec![self.say(Message::status("Reading the details to copy"))]
    }

    /// A Copy details from a panel, once the fields came.
    fn finish_copy(
        &mut self,
        copy: Copying,
        rows: Vec<DetailRow>,
        unread: Vec<String>,
    ) -> Vec<Effect> {
        let rows = assemble(&copy.base, Some(&rows), None, true, &unread);
        self.put_details(&copy.title, &rows, copy.all)
    }

    /// Read text in picture (R30 commit 44): Tesseract reads the focused
    /// picture on a worker; the viewer shows the text when it comes.
    fn read_picture_text(&mut self) -> Vec<Effect> {
        let (entry, place, _) = match self.details_target() {
            Ok(target) => target,
            Err(empty) => return vec![self.say(Message::status(empty))],
        };
        if !file_like(&entry) {
            let line = format!("Not a file: {}", entry.display_name());
            return vec![self.say(Message::info(line))];
        }
        let job = self.next_job_id();
        let line = format!("Reading the text in {}", entry.display_name());
        let now = self.now();
        self.jobs
            .begin(job, diroxide_core::JobKind::Convert, line.clone(), now);
        self.details.picture = Some(job);
        let tools = self.tool_runner();
        self.request(Request::PictureText { job, place, tools });
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::status(line)),
        ]
    }

    /// Cancel's words while Tesseract reads: "Read text in picture:
    /// cancelling".
    pub(super) fn picture_cancel_words(&self, job: diroxide_core::JobId) -> Option<String> {
        (self.details.picture == Some(job)).then(|| "Read text in picture: cancelling".to_owned())
    }

    /// Tesseract's answer: the viewer over the text, its title said first
    /// ("Text in picture (machine read), photo.png, 14 words"); or the
    /// line that says why not ("No text: tesseract not found").
    pub(super) fn picture_text_read(
        &mut self,
        job: diroxide_core::JobId,
        place: &Place,
        result: Result<crate::picture_text::PictureText, String>,
    ) -> Vec<Effect> {
        let summary = match &result {
            Ok(text) => text.detail_line(),
            Err(line) => line.clone(),
        };
        self.jobs.finished(crate::jobs::FinishedJob {
            job,
            kind: diroxide_core::JobKind::Convert,
            summary,
            failed: result.as_ref().err().cloned().into_iter().collect(),
            ..crate::jobs::FinishedJob::default()
        });
        if self.details.picture != Some(job) {
            return vec![Effect::Redraw(Redraw::JobsLine)];
        }
        self.details.picture = None;
        let text = match result {
            Ok(text) => text,
            Err(line) => {
                return vec![
                    Effect::Redraw(Redraw::JobsLine),
                    self.say(Message::info(line)),
                ];
            }
        };
        self.details.picture_words.insert(place.clone(), text.words);
        let name = place
            .name()
            .map(diroxide_core::names::display_name)
            .unwrap_or_default()
            .into_owned();
        if !self.dialogs.is_empty() || self.viewer_open() {
            let line = format!("{}, in {name}", text.detail_line());
            return vec![self.say(Message::info(line))];
        }
        let title = format!(
            "{}, {name}, {}",
            crate::picture_text::TITLE,
            text.words_line()
        );
        let options =
            crate::viewer::ViewerOptions::from_settings(&self.settings, self.screen_reader);
        let (viewer, said) =
            match crate::viewer::Viewer::from_text(place.clone(), title, text.text(), options) {
                Ok(opened) => opened,
                Err(error) => return vec![self.say(error.message(&name))],
            };
        self.viewing = Some(super::viewing::Viewing {
            viewer: Box::new(viewer),
            side: self.active,
            row: place.name().unwrap_or_default().to_owned(),
            siblings: Vec::new(),
            last_follow: None,
        });
        let mut effects = vec![
            Effect::Redraw(Redraw::All),
            Effect::Redraw(Redraw::JobsLine),
        ];
        for message in said {
            effects.push(self.say(message));
        }
        effects
    }

    /// Puts rows on the clipboard and says how many: "Copied 14 details,
    /// 2 private left out".
    fn put_details(&mut self, title: &str, rows: &[DetailRow], all: bool) -> Vec<Effect> {
        let (text, copied, left) = words::copy_text(title, rows, all);
        let line = words::copied_words(copied, left, all);
        vec![
            Effect::SetClipboardText(text),
            self.say(Message::info(line)),
        ]
    }
}
