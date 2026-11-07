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
use crate::dialog::Dialog;
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
}

impl App {
    /// The metadata commands; `None` for commands that are not theirs.
    pub(super) fn meta_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::CHECKSUM {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.checksum_command())
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

    /// Cancel's words while a checksum job runs.
    pub(super) fn meta_cancel_words(&self, job: JobId) -> Option<String> {
        self.meta
            .checksums
            .contains_key(&job)
            .then(|| "Checksum: cancelling".to_owned())
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
