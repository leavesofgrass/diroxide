//! The metadata commands in the app (alpha 4, lane MA, R30 commits 54 to
//! 56; the requests and words are in [`crate::metadata`]).
//!
//! - **Checksum** computes the SHA-256 of the marked files, or the focused
//!   one, as a job Cancel stops, the progress in the jobs line ("Computing
//!   checksums 40 percent"); BLAKE3 comes in the same read, so a pasted
//!   64-digit value can say "Matches BLAKE3, not SHA-256". It says the
//!   value's start, "SHA-256 of example-tool.zip, ab12 cd34" (38 cells),
//!   and lists the whole value in lines of eight groups (39 cells), then
//!   Copy the value, Compare with a pasted value (the verdict first: "Match:
//!   example-tool.zip, SHA-256", or "Does not match" and the first group
//!   that differs) and Save as `SHA256SUMS` beside the files, a taken name
//!   numbered "SHA256SUMS (2)", never overwritten. A pasted value of
//!   another length (128 digits, 32, 8) computes that algorithm first.
//!
//! - **Verify checksum** checks the focused file against the list beside
//!   it, found in R29's order (`NAME.sha256`, `.sha512`, `.md5`,
//!   `.blake3`, then `SHA256SUMS`, `SHA512SUMS`, `MD5SUMS`, `B3SUMS` and
//!   the same ending `.txt`), only its own line; on a list it checks every
//!   file the list names. Several lists beside a file are a Choose list
//!   ("SHA256SUMS, 12 files"). Each file is read again, as a job Cancel
//!   stops. The verdict comes first ("All 12 files match SHA256SUMS", "1
//!   differs, 1 missing, 1 match"), the failures next, "Matches BLAKE3,
//!   not SHA-256" where a value named no algorithm, then one line on the
//!   signature ("List not signed.") and what a match shows; the matches
//!   only on request. A match is never called safe or genuine.
//!
//! - **Strip metadata into a copy** asks first, starting on No ("Strip 3
//!   photos into copies?"), then copies each JPEG, PNG or WebP picture
//!   beside its original as `NAME-clean.EXT` (a taken name numbered),
//!   never over a file, as a job Cancel stops; it says "Stripped into
//!   photo-clean.jpg", what was removed and what is left, read again from
//!   the copy. Undo puts the copies in the trash.
//! - **Check before sharing** reads only: "3 of 5 files hold private
//!   details" first, then each file by name and its details, a place to
//!   two decimals, and what a strip cannot reach. Nothing is sent
//!   anywhere.
//! - **Remove the internet mark** reads the mark first ("No internet mark
//!   on setup.exe" when there is none), then asks, starting on No ("Remove
//!   the internet mark from setup.exe? Windows stops warning when you
//!   open it."), and removes it as a job; what it held is kept, so Undo
//!   writes it back ("Put back the internet mark on setup.exe") until the
//!   job engine's undo record can hold it (ML's request 2).
//!
//! The lists and questions of this lane are hosted under the Organize
//! purpose, so the window's test of every purpose keeps its count.

use std::collections::HashMap;

use diroxide_core::format::count;
use diroxide_core::names::display_name;
use diroxide_core::{Command, Effect, EntryKind, JobId, JobKind, ListModel, ListRow, Message};
use diroxide_core::{Place, Redraw, Side};
use diroxide_meta::checksum::list::{ListForm, write_list};
use diroxide_meta::checksum::{Algorithm, Digest};

use super::App;
use super::organize::Wants;
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::FinishedJob;
use crate::metadata::{MetaAsk, MetaNote, compare_lines, computed_line, saved_line};
use crate::request::Request;

/// The algorithms the Checksum command computes in one read: SHA-256, what
/// publishers post (R28 question 3.3), and BLAKE3, the other 64-digit
/// value a pasted one may be.
const CHECKSUM_ALGORITHMS: [Algorithm; 2] = [Algorithm::Sha256, Algorithm::Blake3];

/// The name a saved list takes.
const LIST_NAME: &str = "SHA256SUMS";

/// What the lane's jobs and lists hold.
#[derive(Clone, Debug, Default)]
pub(super) struct MetaState {
    /// The checksum jobs running, each with what it was for.
    checksums: HashMap<JobId, Pending>,
    /// The verify jobs running.
    verifies: std::collections::HashSet<JobId>,
    /// The strip jobs running.
    strips: std::collections::HashSet<JobId>,
    /// The internet mark removals running, each with its file.
    marks: HashMap<JobId, Place>,
    /// The internet marks removed, newest last, each with the newest job
    /// in the undo log then and what it held: Undo writes it back.
    mark_undo: Vec<(u64, Place, diroxide_meta::origin::MarkRemoval)>,
    /// The SHA-256 each file had this session, with its size and modified
    /// time then: the details' Checksum row says it while they are the
    /// same.
    known: HashMap<Place, (Facts, Digest)>,
}

/// A file's size and modified time, as the panel listed them.
type Facts = (Option<u64>, Option<diroxide_core::Timestamp>);

/// What a running checksum job was started for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    /// The Checksum command: its list when it ends, and each file's facts
    /// when it started.
    List(HashMap<Place, Facts>),
    /// A pasted value of another algorithm's length: the verdict when it
    /// ends, with the list it came from.
    Compare {
        shown: Box<ChecksumShown>,
        pasted: String,
    },
}

/// The checksums one Checksum command computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChecksumShown {
    /// The folder the files are in.
    folder: Place,
    /// Each file read: its name and its values, SHA-256 first.
    files: Vec<(String, Vec<Digest>)>,
    /// Each file that could not be read: its name and why.
    failed: Vec<(String, String)>,
}

impl ChecksumShown {
    /// The title and the first line said: "SHA-256 of example-tool.zip,
    /// ab12 cd34", or "SHA-256 of 3 files".
    fn title(&self) -> String {
        match self.files.as_slice() {
            [(name, digests)] if self.failed.is_empty() => digests
                .first()
                .map(|d| computed_line(d, name))
                .unwrap_or_default(),
            files => {
                let mut title =
                    format!("SHA-256 of {}", count(files.len() as u64, "file", "files"));
                if !self.failed.is_empty() {
                    title.push_str(&format!(", {} unreadable", self.failed.len()));
                }
                title
            }
        }
    }

    /// The list: the whole value in lines of eight groups for one file,
    /// each file's short value for several; what could not be read; then
    /// the actions.
    fn list(&self) -> ListModel {
        let mut rows = Vec::new();
        match self.files.as_slice() {
            [(_, digests)] if self.failed.is_empty() => {
                if let Some(digest) = digests.first() {
                    for (i, line) in digest.lines().into_iter().enumerate() {
                        rows.push(ListRow::new(format!("line:{i}"), line));
                    }
                }
                rows.push(ListRow::new("copy", "Copy the value"));
            }
            files => {
                for (i, (name, digests)) in files.iter().enumerate() {
                    if let Some(digest) = digests.first() {
                        rows.push(ListRow::new(
                            format!("file:{i}"),
                            format!("{}: {}", display_name(name), digest.short_value()),
                        ));
                    }
                }
                for (name, why) in &self.failed {
                    rows.push(ListRow::new(
                        format!("failed:{name}"),
                        format!("Unreadable: {}, {why}", display_name(name)),
                    ));
                }
                if !files.is_empty() {
                    rows.push(ListRow::new("copy", "Copy all as SHA256SUMS lines"));
                }
            }
        }
        if !self.files.is_empty() {
            if self.files.len() == 1 {
                rows.push(ListRow::new("compare", "Compare with a pasted value"));
            }
            rows.push(ListRow::new("save", format!("Save as {LIST_NAME}")));
        }
        let mut list = ListModel::new(self.title()).with_rows(rows);
        list.set_help("Enter on a value copies it; Enter on an action does it. Escape closes.");
        list
    }

    /// The list's text in the GNU form, names relative to the folder.
    fn list_text(&self) -> Option<String> {
        let entries: Vec<(String, Digest)> = self
            .files
            .iter()
            .filter_map(|(name, digests)| digests.first().map(|d| (name.clone(), d.clone())))
            .collect();
        write_list(&entries, ListForm::Gnu).ok()
    }
}

/// What a dialog of this lane is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MetaWants {
    /// The Checksum command's list.
    Checksums(Box<ChecksumShown>),
    /// The field a value is pasted in, to compare with what was computed.
    Compare(Box<ChecksumShown>),
    /// The lists found beside a file, to choose one.
    VerifyLists {
        /// The file.
        place: Place,
    },
    /// What Verify checksum found, with the matches for the row that
    /// lists them.
    VerifyReport {
        /// The matches, each "Match: a.txt, SHA-256".
        matches: Vec<String>,
    },
    /// The question before a strip.
    StripQuestion {
        /// The pictures.
        places: Vec<Place>,
    },
    /// The question before Remove the internet mark.
    MarkQuestion {
        /// The file.
        place: Place,
    },
    /// A list of lines to read: a strip's or a check's result.
    Lines,
}

impl App {
    /// The metadata commands; `None` for commands that are not theirs.
    pub(super) fn meta_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        let ours = [
            ids::CHECKSUM,
            ids::VERIFY_CHECKSUM,
            ids::STRIP_METADATA,
            ids::CHECK_BEFORE_SHARING,
            ids::REMOVE_INTERNET_MARK,
        ];
        if !ours.contains(id) {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(if *id == ids::CHECKSUM {
            self.checksum_command()
        } else if *id == ids::VERIFY_CHECKSUM {
            self.verify_command(None)
        } else if *id == ids::STRIP_METADATA {
            self.strip_command()
        } else if *id == ids::CHECK_BEFORE_SHARING {
            self.check_command()
        } else {
            self.mark_command()
        })
    }

    /// Strip metadata into a copy: the pictures among the marked files,
    /// or the focused one, asked about first, starting on No.
    fn strip_command(&mut self) -> Vec<Effect> {
        let side = self.active;
        let places: Vec<Place> = match self.meta_files(side, "Strip") {
            Ok(places) => places
                .into_iter()
                .filter(|p| p.name().is_some_and(crate::metadata::strips))
                .collect(),
            Err(line) => return vec![self.say(Message::status(line))],
        };
        if places.is_empty() {
            return vec![self.say(Message::status(
                "Strip works on JPEG, PNG and WebP pictures; none is chosen",
            ))];
        }
        let names: Vec<&str> = places.iter().filter_map(Place::name).collect();
        let question = with_default_no(Message::question(&diroxide_meta::strip::strip_question(
            &names,
        )));
        let listing = names.iter().map(|n| display_name(n).into_owned()).collect();
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::STRIP_METADATA),
            },
            Wants::Meta(MetaWants::StripQuestion { places }),
        )
    }

    /// Yes to one of this lane's questions.
    pub(super) fn meta_yes(&mut self, wants: MetaWants) -> Vec<Effect> {
        match wants {
            MetaWants::StripQuestion { places } => {
                let line = match places.as_slice() {
                    [one] => format!(
                        "Stripping {} into a copy",
                        one.name().map(display_name).unwrap_or_default()
                    ),
                    many => format!(
                        "Stripping {} into copies",
                        count(many.len() as u64, "photo", "photos")
                    ),
                };
                let job = self.next_job_id();
                let now = self.now();
                self.jobs
                    .begin(job, JobKind::StripMetadata, line.clone(), now);
                self.meta.strips.insert(job);
                self.request(Request::Meta(Box::new(MetaAsk::Strip { job, places })));
                vec![
                    Effect::Redraw(Redraw::JobsLine),
                    self.say(Message::status(line)),
                ]
            }
            MetaWants::MarkQuestion { place } => {
                let name = place
                    .name()
                    .map(display_name)
                    .unwrap_or_default()
                    .into_owned();
                let line = format!("Removing the internet mark from {name}");
                let job = self.next_job_id();
                let now = self.now();
                self.jobs
                    .begin(job, JobKind::EditMetadata, line.clone(), now);
                self.meta.marks.insert(job, place.clone());
                self.request(Request::Meta(Box::new(MetaAsk::RemoveMark { job, place })));
                vec![
                    Effect::Redraw(Redraw::JobsLine),
                    self.say(Message::status(line)),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Escape or No on one of this lane's questions: nothing changed.
    pub(super) fn meta_escape(&mut self, wants: MetaWants) -> Vec<Effect> {
        match wants {
            MetaWants::StripQuestion { .. } => vec![self.say(Message::info("Nothing stripped."))],
            MetaWants::MarkQuestion { .. } => {
                vec![self.say(Message::info("The internet mark stays."))]
            }
            _ => Vec::new(),
        }
    }

    /// Check before sharing: the marked files, or the focused one, read
    /// for their private details; read only.
    fn check_command(&mut self) -> Vec<Effect> {
        let side = self.active;
        let places = match self.meta_files(side, "Check before sharing") {
            Ok(places) => places,
            Err(line) => return vec![self.say(Message::status(line))],
        };
        let line = match places.as_slice() {
            [one] => format!(
                "Checking {} before sharing",
                one.name().map(display_name).unwrap_or_default()
            ),
            many => format!(
                "Checking {} before sharing",
                count(many.len() as u64, "file", "files")
            ),
        };
        let tools = self.tool_runner();
        self.request(Request::Meta(Box::new(MetaAsk::Check { places, tools })));
        vec![self.say(Message::status(line))]
    }

    /// Remove the internet mark: the focused file's mark read first, then
    /// the question.
    fn mark_command(&mut self) -> Vec<Effect> {
        let panel = self.active_panel();
        let Some(entry) = panel.focused_entry() else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
            let line = format!(
                "Remove the internet mark works on files: {} is a folder",
                entry.display_name()
            );
            return vec![self.say(Message::status(line))];
        }
        let Some(place) = panel.focused_place() else {
            return Vec::new();
        };
        self.request(Request::Meta(Box::new(MetaAsk::ReadMark { place })));
        Vec::new()
    }

    /// Opens a list of lines to read, its title said first; with a dialog
    /// or the viewer open, the title alone is said.
    fn lines_list(&mut self, title: String, lines: Vec<String>, done: bool) -> Vec<Effect> {
        if !self.dialogs.is_empty() || self.viewer_open() {
            let message = if done {
                Message::done(title)
            } else {
                Message::info(title)
            };
            return vec![self.say(message)];
        }
        let rows = lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| ListRow::new(format!("line:{i}"), line))
            .collect();
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help("Up and Down read, Escape closes.");
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Meta(MetaWants::Lines),
        )
    }

    /// What a strip made: the copies said, recorded for Undo, and the
    /// panels on their folders listed again.
    fn stripped(
        &mut self,
        job: JobId,
        results: Vec<(Place, crate::metadata::StripResult)>,
        cancelled: bool,
    ) -> Vec<Effect> {
        self.meta.strips.remove(&job);
        let copies: Vec<Place> = results
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok().map(|(copy, _)| copy.clone()))
            .collect();
        let failed: Vec<String> = results
            .iter()
            .filter_map(|(place, r)| {
                r.as_ref().err().map(|why| {
                    format!(
                        "{}: {why}",
                        place.name().map(display_name).unwrap_or_default()
                    )
                })
            })
            .collect();
        let all = results.len();
        let title = match results.as_slice() {
            [(_, Ok((copy, _)))] => format!(
                "Stripped into {}",
                copy.name().map(display_name).unwrap_or_default()
            ),
            [(place, Err(why))] => format!(
                "Could not strip {}: {why}",
                place.name().map(display_name).unwrap_or_default()
            ),
            _ => crate::metadata::stripped_line(copies.len(), all),
        };
        let summary = if cancelled {
            format!("Strip cancelled: {} done.", copies.len())
        } else {
            title.clone()
        };
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::StripMetadata,
            summary: summary.clone(),
            failed: failed.clone(),
            done: copies.len() as u64,
            cancelled,
            ..FinishedJob::default()
        });
        if !copies.is_empty() {
            let mut record = diroxide_ops::UndoRecord::for_job(job, JobKind::StripMetadata);
            record.created = copies.clone();
            self.undo.record(record);
        }
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        let mut lines = Vec::new();
        for (place, result) in &results {
            match result {
                Ok((copy, report)) => {
                    let copy_name = copy.name().unwrap_or_default();
                    let mut each = report.lines(copy_name);
                    if all > 1 {
                        each[0] = format!(
                            "{} stripped into {}",
                            place.name().map(display_name).unwrap_or_default(),
                            display_name(copy_name)
                        );
                    } else {
                        each.remove(0);
                    }
                    lines.extend(each);
                }
                Err(why) if all > 1 => lines.push(format!(
                    "Could not strip {}: {why}",
                    place.name().map(display_name).unwrap_or_default()
                )),
                Err(_) => {}
            }
        }
        if cancelled {
            effects.push(self.say(Message::info(summary)));
        } else if copies.is_empty() {
            effects.push(self.say(Message::warning(title)));
        } else {
            effects.extend(self.lines_list(title, lines, true));
        }
        let folders: Vec<Place> = copies.iter().filter_map(Place::parent).collect();
        effects.extend(self.list_again(&folders));
        effects
    }

    /// The panels on any of these folders are listed again, the focus
    /// kept.
    fn list_again(&mut self, folders: &[Place]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for side in Side::BOTH {
            let panel = self.panel(side);
            if folders.contains(panel.place()) {
                let place = panel.place().clone();
                let focus = panel.list().focused().map(|r| r.id.clone());
                effects.extend(self.open_as(side, place, focus, crate::panel::Load::AfterJob));
            }
        }
        effects
    }

    /// What Check before sharing found: the summary first, then each
    /// file's lines.
    fn checked(&mut self, reports: Vec<diroxide_meta::privacy::PrivacyReport>) -> Vec<Effect> {
        let summary = diroxide_meta::privacy::summary(&reports);
        let lines: Vec<String> = reports.iter().flat_map(|r| r.lines()).collect();
        let holding = reports.iter().any(|r| r.holds_private());
        self.lines_list(summary, lines, !holding)
    }

    /// A file's mark was read: none is said; one is asked about, starting
    /// on No.
    fn mark_read(&mut self, place: Place, result: Result<Option<String>, String>) -> Vec<Effect> {
        let name = place
            .name()
            .map(display_name)
            .unwrap_or_default()
            .into_owned();
        match result {
            Err(why) => vec![self.say(Message::error(format!(
                "Could not read the internet mark of {name}: {why}"
            )))],
            Ok(None) => vec![self.say(Message::info(diroxide_meta::origin::no_mark_words(&name)))],
            Ok(Some(origin)) => {
                if !self.dialogs.is_empty() {
                    return Vec::new();
                }
                let mut question = with_default_no(Message::question(
                    &diroxide_meta::origin::removal_question(&name),
                ));
                question.text = question.text.replacen(
                    "? No is the default.",
                    &format!(
                        "? {} No is the default.",
                        diroxide_meta::origin::removal_warning()
                    ),
                    1,
                );
                self.open_organize(
                    Dialog::Confirm {
                        question,
                        listing: vec![origin],
                        on_yes: Command::new(ids::REMOVE_INTERNET_MARK),
                    },
                    Wants::Meta(MetaWants::MarkQuestion { place }),
                )
            }
        }
    }

    /// The mark was removed: said, and what it held kept for Undo.
    fn mark_removed(
        &mut self,
        job: JobId,
        place: Place,
        result: Result<Option<diroxide_meta::origin::MarkRemoval>, String>,
    ) -> Vec<Effect> {
        self.meta.marks.remove(&job);
        let name = place
            .name()
            .map(display_name)
            .unwrap_or_default()
            .into_owned();
        let line = match &result {
            Ok(Some(_)) => diroxide_meta::origin::removed_words(&name),
            Ok(None) => diroxide_meta::origin::no_mark_words(&name),
            Err(why) => format!("Could not remove the internet mark from {name}: {why}"),
        };
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::EditMetadata,
            summary: line.clone(),
            failed: result.as_ref().err().cloned().into_iter().collect(),
            done: u64::from(matches!(result, Ok(Some(_)))),
            ..FinishedJob::default()
        });
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        match result {
            Ok(Some(removal)) => {
                let after = self.newest_job();
                self.meta.mark_undo.push((after, place, removal));
                effects.push(self.say(Message::done(line)));
            }
            Ok(None) => effects.push(self.say(Message::info(line))),
            Err(_) => effects.push(self.say(Message::error(line))),
        }
        effects
    }

    /// Undo, when the last internet mark removal came after the last job
    /// and the last label change: it is written back. `None` leaves Undo
    /// to the others.
    pub(super) fn undo_mark_removal(&mut self) -> Option<Vec<Effect>> {
        let (after_job, _, _) = self.meta.mark_undo.last()?;
        if self.newest_job() > *after_job {
            return None;
        }
        if self
            .organize
            .label_undo
            .last()
            .is_some_and(|(label_after, _)| label_after > after_job)
        {
            return None;
        }
        let (_, place, removal) = self.meta.mark_undo.pop()?;
        let name = place
            .name()
            .map(display_name)
            .unwrap_or_default()
            .into_owned();
        self.request(Request::Meta(Box::new(MetaAsk::RestoreMark {
            place,
            removal,
        })));
        Some(vec![self.say(Message::status(format!(
            "Putting back the internet mark on {name}"
        )))])
    }

    /// Verify checksum on the focused file, with the list chosen when
    /// several were found.
    fn verify_command(&mut self, list: Option<(Place, String)>) -> Vec<Effect> {
        let side = self.active;
        let (place, list) = match list {
            Some((place, list)) => (place, Some(list)),
            None => {
                let panel = self.panel(side);
                let Some(entry) = panel.focused_entry() else {
                    let empty = panel.list().empty_text().to_owned();
                    return vec![self.say(Message::status(empty))];
                };
                if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
                    let line = format!(
                        "Verify checksum works on files: {} is a folder",
                        entry.display_name()
                    );
                    return vec![self.say(Message::status(line))];
                }
                let Some(place) = panel.focused_place() else {
                    return Vec::new();
                };
                (place, None)
            }
        };
        let name = place
            .name()
            .map(display_name)
            .unwrap_or_default()
            .into_owned();
        let line = format!("Verifying {name}");
        let job = self.next_job_id();
        let now = self.now();
        self.jobs.begin(job, JobKind::Verify, line.clone(), now);
        self.meta.verifies.insert(job);
        self.request(Request::Meta(Box::new(MetaAsk::Verify {
            job,
            place,
            list,
        })));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::status(line)),
        ]
    }

    /// What Verify checksum found: the verdict said and listed first.
    fn verified(
        &mut self,
        job: JobId,
        place: Place,
        outcome: Result<crate::metadata::Verified, String>,
    ) -> Vec<Effect> {
        use crate::metadata::Verified;
        self.meta.verifies.remove(&job);
        let name = place
            .name()
            .map(display_name)
            .unwrap_or_default()
            .into_owned();
        let (summary, failed, cancelled) = match &outcome {
            Ok(Verified::Report { report, .. }) => (report.verdict(), report.failures(), false),
            Ok(Verified::Cancelled) => ("Verify cancelled.".to_owned(), Vec::new(), true),
            Ok(_) => (
                "Verify found no list to check.".to_owned(),
                Vec::new(),
                false,
            ),
            Err(why) => (
                format!("Could not verify {name}: {why}"),
                vec![why.clone()],
                false,
            ),
        };
        let done = match &outcome {
            Ok(Verified::Report { report, .. }) => report.counts().files() as u64,
            _ => 0,
        };
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::Verify,
            summary,
            failed,
            done,
            cancelled,
            ..FinishedJob::default()
        });
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        match outcome {
            Err(why) => {
                effects.push(self.say(Message::error(format!("Could not verify {name}: {why}"))))
            }
            Ok(Verified::Cancelled) => effects.push(self.say(Message::info("Verify cancelled."))),
            Ok(Verified::NoList) => {
                effects.push(self.say(Message::info(crate::metadata::no_list_line(&name))));
            }
            Ok(Verified::NoLine(list)) => {
                effects.push(self.say(Message::info(crate::metadata::no_line_line(&list, &name))));
            }
            Ok(Verified::Choose(lists)) => {
                if !self.dialogs.is_empty() {
                    return effects;
                }
                let rows = lists
                    .into_iter()
                    .map(|(id, row)| ListRow::new(id, row))
                    .collect();
                let mut list =
                    ListModel::new(format!("Checksum lists beside {name}")).with_rows(rows);
                list.set_help("Enter checks the file against that list. Escape closes.");
                effects.extend(self.open_organize(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Wants::Meta(MetaWants::VerifyLists { place }),
                ));
            }
            Ok(Verified::Report { report, signature }) => {
                let lines = crate::metadata::verify_lines(&report, signature.as_deref());
                let verdict = lines.first().cloned().unwrap_or_default();
                let message = if report.all_match() {
                    Message::done(verdict.clone())
                } else {
                    Message::warning(verdict.clone())
                };
                // The list's title is the verdict, said when it opens; with
                // a dialog or the viewer open, the verdict alone is said.
                if !self.dialogs.is_empty() || self.viewer_open() {
                    effects.push(self.say(message));
                    return effects;
                }
                let matches = report.matches();
                let mut rows: Vec<ListRow> = lines
                    .iter()
                    .enumerate()
                    .skip(1)
                    .map(|(i, line)| ListRow::new(format!("line:{i}"), line.clone()))
                    .collect();
                if !matches.is_empty() {
                    rows.push(ListRow::new(
                        "matches",
                        format!(
                            "Show the {}",
                            count(matches.len() as u64, "match", "matches")
                        ),
                    ));
                }
                let mut list = ListModel::new(verdict).with_rows(rows);
                list.set_help("Enter on Show the matches lists them. Escape closes.");
                effects.extend(self.open_organize(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Wants::Meta(MetaWants::VerifyReport { matches }),
                ));
            }
        }
        effects
    }

    /// The files a command acts on: the marked ones, or the focused one,
    /// folders left out; or the line said instead.
    fn meta_files(&self, side: Side, what: &str) -> Result<Vec<Place>, String> {
        let Some((places, _)) = self.frozen(side) else {
            return Err(self.panel(side).list().empty_text().to_owned());
        };
        let panel = self.panel(side);
        let files: Vec<Place> = places
            .into_iter()
            .filter(|place| {
                place
                    .name()
                    .and_then(|name| panel.entry(name))
                    .is_some_and(|e| matches!(e.kind, EntryKind::File | EntryKind::Archive))
            })
            .collect();
        if files.is_empty() {
            return Err(format!("{what} works on files; no file is chosen"));
        }
        Ok(files)
    }

    /// Checksum: the SHA-256 (and BLAKE3) of the marked files, or the
    /// focused one, as a job.
    fn checksum_command(&mut self) -> Vec<Effect> {
        let side = self.active;
        let places = match self.meta_files(side, "Checksum") {
            Ok(places) => places,
            Err(line) => return vec![self.say(Message::status(line))],
        };
        let line = match places.as_slice() {
            [one] => format!(
                "Computing the SHA-256 of {}",
                one.name().map(display_name).unwrap_or_default()
            ),
            many => format!(
                "Computing checksums of {}",
                count(many.len() as u64, "file", "files")
            ),
        };
        let panel = self.panel(side);
        let facts: HashMap<Place, Facts> = places
            .iter()
            .filter_map(|place| {
                let entry = panel.entry(place.name()?)?;
                Some((place.clone(), (entry.size, entry.modified)))
            })
            .collect();
        self.start_checksums(
            places,
            CHECKSUM_ALGORITHMS.to_vec(),
            Pending::List(facts),
            line,
        )
    }

    /// The SHA-256 the Checksum command computed for a file this session,
    /// while its size and modified time are as they were: "SHA-256: ba78
    /// 16bf, ending 15ad".
    pub(super) fn known_checksum(
        &self,
        place: &Place,
        entry: &diroxide_core::Entry,
    ) -> Option<String> {
        let ((size, modified), digest) = self.meta.known.get(place)?;
        (*size == entry.size && *modified == entry.modified).then(|| digest.short())
    }

    fn start_checksums(
        &mut self,
        places: Vec<Place>,
        algorithms: Vec<Algorithm>,
        pending: Pending,
        line: String,
    ) -> Vec<Effect> {
        let job = self.next_job_id();
        let now = self.now();
        self.jobs.begin(job, JobKind::Checksum, line.clone(), now);
        self.meta.checksums.insert(job, pending);
        self.request(Request::Meta(Box::new(MetaAsk::Checksums {
            job,
            places,
            algorithms,
        })));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::status(line)),
        ]
    }

    /// Cancel's words while a checksum or verify job runs.
    pub(super) fn meta_cancel_words(&self, job: JobId) -> Option<String> {
        if self.meta.checksums.contains_key(&job) {
            return Some("Checksum: cancelling".to_owned());
        }
        if self.meta.strips.contains(&job) {
            return Some("Strip: cancelling".to_owned());
        }
        self.meta
            .verifies
            .contains(&job)
            .then(|| "Verify checksum: cancelling".to_owned())
    }

    /// What a metadata worker found.
    pub(super) fn meta_note(&mut self, note: MetaNote) -> Vec<Effect> {
        match note {
            MetaNote::Checksums {
                job,
                results,
                cancelled,
            } => self.checksums_computed(job, results, cancelled),
            MetaNote::Saved { folder, result } => self.checksum_list_saved(&folder, result),
            MetaNote::Verified {
                job,
                place,
                outcome,
            } => self.verified(job, place, outcome),
            MetaNote::Stripped {
                job,
                results,
                cancelled,
            } => self.stripped(job, results, cancelled),
            MetaNote::Checked { reports } => self.checked(reports),
            MetaNote::Mark { place, result } => self.mark_read(place, result),
            MetaNote::MarkRemoved { job, place, result } => self.mark_removed(job, place, result),
            MetaNote::MarkRestored { place, result } => {
                let name = place
                    .name()
                    .map(display_name)
                    .unwrap_or_default()
                    .into_owned();
                match result {
                    Ok(()) => vec![self.say(Message::done(crate::metadata::restored_words(&name)))],
                    Err(why) => vec![self.say(Message::error(format!(
                        "Could not put back the internet mark on {name}: {why}"
                    )))],
                }
            }
        }
    }

    fn checksums_computed(
        &mut self,
        job: JobId,
        results: Vec<(Place, Result<Vec<Digest>, String>)>,
        cancelled: bool,
    ) -> Vec<Effect> {
        let pending = self.meta.checksums.remove(&job);
        let failed: Vec<String> = results
            .iter()
            .filter_map(|(place, result)| {
                result.as_ref().err().map(|why| {
                    format!(
                        "{}: {why}",
                        place.name().map(display_name).unwrap_or_default()
                    )
                })
            })
            .collect();
        let done = results.iter().filter(|(_, r)| r.is_ok()).count() as u64;
        let summary = if cancelled {
            format!("Checksum cancelled: {done} done.")
        } else if failed.is_empty() {
            format!("Checksum finished: {done} done.")
        } else {
            format!(
                "Checksum had {}: {done} done.",
                count(failed.len() as u64, "failure", "failures")
            )
        };
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::Checksum,
            summary: summary.clone(),
            failed,
            done,
            cancelled,
            ..FinishedJob::default()
        });
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        if cancelled {
            effects.push(self.say(Message::info(summary)));
            return effects;
        }
        let Some(folder) = results.first().and_then(|(place, _)| place.parent()) else {
            return effects;
        };
        let mut shown = ChecksumShown {
            folder,
            files: Vec::new(),
            failed: Vec::new(),
        };
        for (place, result) in results {
            let name = place.name().unwrap_or_default().to_owned();
            if let (Some(Pending::List(facts)), Ok(digests)) = (&pending, &result)
                && let (Some(facts), Some(digest)) = (facts.get(&place), digests.first())
            {
                self.meta
                    .known
                    .insert(place.clone(), (*facts, digest.clone()));
            }
            match result {
                Ok(digests) => shown.files.push((name, digests)),
                Err(why) => shown.failed.push((name, why)),
            }
        }
        match pending {
            Some(Pending::Compare {
                shown: from,
                pasted,
            }) => {
                let mut digests = from
                    .files
                    .first()
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default();
                if let Some((_, more)) = shown.files.first() {
                    digests.extend(more.iter().cloned());
                }
                let name = from
                    .files
                    .first()
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default();
                match compare_lines(&digests, &name, &pasted) {
                    Some(lines) => effects.push(self.say_verdict(&lines)),
                    None => effects.push(self.say(Message::info(summary))),
                }
                effects
            }
            _ => {
                if shown.files.is_empty() {
                    let line = shown
                        .failed
                        .first()
                        .map(|(name, why)| format!("Unreadable: {}, {why}", display_name(name)))
                        .unwrap_or(summary);
                    effects.push(self.say(Message::warning(line)));
                    return effects;
                }
                if !self.dialogs.is_empty() || self.viewer_open() {
                    effects.push(self.say(Message::info(shown.title())));
                    return effects;
                }
                let list = shown.list();
                effects.extend(self.open_organize(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Wants::Meta(MetaWants::Checksums(Box::new(shown))),
                ));
                effects
            }
        }
    }

    /// A verdict, its first line said first: a match as information, a
    /// difference as a warning.
    fn say_verdict(&mut self, lines: &[String]) -> Effect {
        let text = lines.join(". ");
        if lines.first().is_some_and(|l| l.starts_with("Match")) {
            self.say(Message::info(text))
        } else {
            self.say(Message::warning(text))
        }
    }

    /// Enter in one of this lane's dialogs.
    pub(super) fn meta_accept(
        &mut self,
        wants: MetaWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            MetaWants::Checksums(shown) => match row.as_deref() {
                Some("compare") => {
                    let name = shown
                        .files
                        .first()
                        .map(|(n, _)| n.clone())
                        .unwrap_or_default();
                    let field = EntryField::new(
                        format!(
                            "Compare with the SHA-256 of {}, paste the value",
                            display_name(&name)
                        ),
                        CompletionSource::None,
                    );
                    self.open_organize(Dialog::edit(field), Wants::Meta(MetaWants::Compare(shown)))
                }
                Some("save") => {
                    let Some(text) = shown.list_text() else {
                        return vec![self.say(Message::error("The list could not be written."))];
                    };
                    let mut effects = self.close_top();
                    self.request(Request::Meta(Box::new(MetaAsk::SaveList {
                        folder: shown.folder.clone(),
                        name: LIST_NAME.to_owned(),
                        text,
                    })));
                    effects.push(self.say(Message::status(format!("Saving {LIST_NAME}"))));
                    effects
                }
                Some("copy") if shown.files.len() > 1 => {
                    let text = shown.list_text().unwrap_or_default();
                    let line = format!(
                        "Copied the SHA-256 of {}",
                        count(shown.files.len() as u64, "file", "files")
                    );
                    vec![
                        Effect::SetClipboardText(text),
                        self.say(Message::info(line)),
                    ]
                }
                Some(row) if row == "copy" || row.starts_with("line:") => {
                    self.copy_value(shown.files.first())
                }
                Some(row) if row.starts_with("file:") => {
                    let at = row["file:".len()..].parse::<usize>().ok();
                    self.copy_value(at.and_then(|i| shown.files.get(i)))
                }
                _ => Vec::new(),
            },
            MetaWants::Compare(shown) => self.compare_pasted(*shown, text),
            MetaWants::VerifyLists { place } => match row {
                Some(list) => {
                    let mut effects = self.close_top();
                    effects.extend(self.verify_command(Some((place, list))));
                    effects
                }
                None => Vec::new(),
            },
            MetaWants::StripQuestion { .. } | MetaWants::MarkQuestion { .. } | MetaWants::Lines => {
                Vec::new()
            }
            MetaWants::VerifyReport { matches } => {
                if row.as_deref() != Some("matches") {
                    return Vec::new();
                }
                let rows: Vec<ListRow> = matches
                    .iter()
                    .enumerate()
                    .map(|(i, line)| ListRow::new(format!("match:{i}"), line.clone()))
                    .collect();
                if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() {
                    let title = format!("{}, matches", list.title());
                    let mut shown = ListModel::new(title).with_rows(rows);
                    shown.set_help("Escape closes.");
                    *list = shown;
                }
                vec![
                    Effect::Redraw(Redraw::All),
                    self.say(Message::status(format!(
                        "{} listed",
                        count(matches.len() as u64, "match", "matches")
                    ))),
                ]
            }
        }
    }

    /// Puts one file's SHA-256 on the clipboard and says whose it is.
    fn copy_value(&mut self, file: Option<&(String, Vec<Digest>)>) -> Vec<Effect> {
        let Some((name, digests)) = file else {
            return Vec::new();
        };
        let Some(digest) = digests.first() else {
            return Vec::new();
        };
        let line = format!(
            "Copied the {} of {}",
            digest.algorithm.name(),
            display_name(name)
        );
        vec![
            Effect::SetClipboardText(digest.hex()),
            self.say(Message::info(line)),
        ]
    }

    /// A pasted value: the verdict first; a value of another algorithm's
    /// length computes that one first.
    fn compare_pasted(&mut self, shown: ChecksumShown, pasted: &str) -> Vec<Effect> {
        let Some((name, digests)) = shown.files.first().cloned() else {
            return Vec::new();
        };
        if crate::metadata::pasted_digits(pasted).is_empty() {
            return vec![self.say(Message::status("Paste the value to compare first."))];
        }
        let mut effects = self.close_top();
        match compare_lines(&digests, &name, pasted) {
            Some(lines) => effects.push(self.say_verdict(&lines)),
            None => {
                let digits = crate::metadata::pasted_digits(pasted).len();
                let Some(algorithm) = crate::metadata::algorithm_for_pasted(digits) else {
                    return effects;
                };
                let Ok(place) = shown.folder.join(&name) else {
                    return effects;
                };
                let line = format!(
                    "Computing the {} of {}",
                    algorithm.name(),
                    display_name(&name)
                );
                effects.extend(self.start_checksums(
                    vec![place],
                    vec![algorithm],
                    Pending::Compare {
                        shown: Box::new(shown),
                        pasted: pasted.to_owned(),
                    },
                    line,
                ));
            }
        }
        effects
    }

    /// A list was saved: said with the name it got, and the panels on its
    /// folder listed again.
    fn checksum_list_saved(
        &mut self,
        folder: &Place,
        result: Result<String, String>,
    ) -> Vec<Effect> {
        let mut effects = match result {
            Ok(name) => vec![self.say(Message::done(saved_line(&name, folder)))],
            Err(why) => {
                vec![self.say(Message::error(format!("Could not save {LIST_NAME}: {why}")))]
            }
        };
        for side in Side::BOTH {
            let panel = self.panel(side);
            if panel.place() == folder {
                let focus = panel.list().focused().map(|r| r.id.clone());
                effects.extend(self.open_as(
                    side,
                    folder.clone(),
                    focus,
                    crate::panel::Load::AfterJob,
                ));
            }
        }
        effects
    }
}
