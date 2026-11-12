//! The photo helpers in the app (alpha 4, lane AP, R30 commits 83 to 85;
//! the model and its words are in [`crate::photos`]). Each is a command
//! of the palette and the menus, in the terminal as in the window; none
//! draws a picture.
//!
//! - **Next day** and **Previous day** (Go menu) move to the first
//!   picture of the next or previous date, by date taken or else the
//!   modified time: "Next day: October 12, 9 pictures, Monday", the date
//!   from the pictures and the weekday from the calendar.
//! - **Sort by date taken** (Sort menu): "Sorted by date taken, oldest
//!   first", then the column note when some pictures have no date: "Date
//!   taken; 12 use modified time".
//! - **Select the screenshots**, **Select the bursts** and **Select
//!   pictures with no date** (Select menu): the screenshots group's
//!   files; pictures from one camera within the burst gap ("Burst, 7
//!   pictures in 2 seconds"); the pictures with no date taken.
//! - **Review pictures** (Tools menu) steps through the marked pictures,
//!   or every picture, from the focused one: "Review: 3 of 14,
//!   IMG_2041.jpg, photo", each answer one key: K keeps, L labels, M moves
//!   to a place you pick, T sends to the trash, S skips, B goes back.
//!   Moves and the trash are jobs Undo takes back as it does any other.
//! - **Move into month folders** (Tools menu): "Move 214 pictures into 9
//!   month folders? No is the default. y or n", folders like `2026-10`
//!   from the date taken or the modified time (the plan's listing says
//!   how many use it), made where missing, never over a file, a name
//!   taken asked about; one job, so one Undo puts everything back.
//!
//! The dates come from each picture's own Exif block, read once for a
//! panel's pictures on a worker when a helper first needs them, and
//! again when the folder changes. The lists and questions are hosted
//! under the Organize purpose, so the window's test of every purpose
//! keeps its count.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{
    Command, ConflictPolicy, Effect, Entry, EntryKind, JobKind, JobRequest, Key, ListModel,
    ListRow, Message, Move, Place, Side, SortKey,
};

use super::organize::Wants;
use super::{App, Flow, Then};
use crate::columns::ColumnCell;
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::photos as words;
use crate::photos::{
    Kind, Moment, NO_DATE, NO_DATE_LINE, PhotoAsk, PhotoFact, PhotoField, PhotoNote, Picture,
    TAKEN, burst_line, bursts, day_line, fallback_note, month_question, month_row, months,
    review_line, step_day,
};
use crate::request::Request;

/// The screenshots group's id in the shipped file types.
const SCREENSHOTS: &str = "screenshots";

/// The pictures group's id.
const IMAGES: &str = "images";

/// What the photo helpers hold.
#[derive(Clone, Debug, Default)]
pub(super) struct PhotoState {
    /// Each panel's pictures' facts, for the listing they were read for.
    facts: [Option<Facts>; 2],
    /// A command waiting for its panel's facts.
    pending: Option<Pending>,
    /// The place the review last moved a picture to, offered first.
    last_move: Option<String>,
}

/// One panel's pictures' facts.
#[derive(Clone, Debug)]
struct Facts {
    listing: u64,
    by_name: HashMap<String, PhotoFact>,
}

/// A command waiting for facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    side: Side,
    listing: u64,
    command: PhotoCommand,
}

/// The photo helpers that need the pictures' facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhotoCommand {
    NextDay,
    PreviousDay,
    SortTaken,
    SelectBursts,
    SelectNoDate,
    Review,
    MonthFolders,
}

/// A review's answer for one picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Done {
    Kept,
    Labeled,
    Moved(String),
    Trashed,
    Skipped,
}

impl Done {
    fn words(&self) -> String {
        match self {
            Done::Kept => "kept".to_owned(),
            Done::Labeled => "labeled".to_owned(),
            Done::Moved(to) => format!("moved to {to}"),
            Done::Trashed => "in the trash".to_owned(),
            Done::Skipped => "skipped".to_owned(),
        }
    }
}

/// A review under way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Review {
    side: Side,
    folder: Place,
    /// Each picture's name and kind, in the panel's order.
    pictures: Vec<(String, Kind)>,
    /// The picture asked about now.
    at: usize,
    /// What was answered for each.
    done: Vec<Option<Done>>,
}

impl Review {
    fn title(&self) -> String {
        let (name, kind) = &self.pictures[self.at];
        let name = diroxide_core::names::display_name(name);
        let line = review_line(self.at + 1, self.pictures.len(), &name, *kind);
        match &self.done[self.at] {
            Some(done) => format!("{line}, {}", done.words()),
            None => line,
        }
    }

    fn list(&self) -> ListModel {
        let rows = [
            ("keep", "Keep", "K"),
            ("label", "Label", "L"),
            ("move", "Move to a place you pick", "M"),
            ("trash", "Trash", "T"),
            ("skip", "Skip", "S"),
            ("back", "Back", "B"),
        ]
        .into_iter()
        .map(|(id, label, key)| ListRow::new(id, label).with_details([key]))
        .collect();
        let mut list = ListModel::new(self.title()).with_rows(rows);
        list.set_help(
            "Each answer is one key: K keeps, L labels, M moves to a place you pick, T sends to the trash, S skips, B goes back. Escape ends the review.",
        );
        list
    }

    fn place(&self) -> Option<Place> {
        self.folder.join(&self.pictures[self.at].0).ok()
    }

    /// "Review ended: 3 kept, 1 moved, 1 in the trash, 2 skipped".
    fn tally(&self, finished: bool) -> String {
        let mut kept = 0;
        let mut labeled = 0;
        let mut moved = 0;
        let mut trashed = 0;
        let mut skipped = 0;
        for done in self.done.iter().flatten() {
            match done {
                Done::Kept => kept += 1,
                Done::Labeled => labeled += 1,
                Done::Moved(_) => moved += 1,
                Done::Trashed => trashed += 1,
                Done::Skipped => skipped += 1,
            }
        }
        let mut parts = Vec::new();
        for (n, word) in [
            (kept, "kept"),
            (labeled, "labeled"),
            (moved, "moved"),
            (trashed, "in the trash"),
            (skipped, "skipped"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        let head = if finished {
            format!(
                "Review done: {}",
                count(self.pictures.len() as u64, "picture", "pictures")
            )
        } else {
            format!("Review ended at {} of {}", self.at + 1, self.pictures.len())
        };
        if parts.is_empty() {
            head
        } else {
            format!("{head}, {}", parts.join(", "))
        }
    }
}

/// What a dialog of the photo helpers is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PhotoWants {
    /// The review's list of answers.
    Review(Box<Review>),
    /// The field the review's move place is typed in.
    ReviewMove(Box<Review>),
    /// The question before the month folders, with the moves Yes plans.
    Months(Box<MonthMoves>),
}

/// The month folders' moves, planned by the engine on Yes (lane AF:
/// `Engine::plan_moves`, so the move's checks and the plugins'
/// before-move hooks are any move's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MonthMoves {
    /// The job, the panel's folder its destination.
    request: JobRequest,
    /// Each picture and its month's folder.
    moves: Vec<(Place, Place)>,
}

impl App {
    /// The photo helpers; `None` for commands that are not theirs.
    pub(super) fn photo_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        let needs = if *id == ids::NEXT_DAY {
            Some(PhotoCommand::NextDay)
        } else if *id == ids::PREVIOUS_DAY {
            Some(PhotoCommand::PreviousDay)
        } else if *id == ids::SORT_DATE_TAKEN {
            Some(PhotoCommand::SortTaken)
        } else if *id == ids::SELECT_BURSTS {
            Some(PhotoCommand::SelectBursts)
        } else if *id == ids::SELECT_NO_DATE {
            Some(PhotoCommand::SelectNoDate)
        } else if *id == ids::REVIEW_PICTURES {
            Some(PhotoCommand::Review)
        } else if *id == ids::MONTH_FOLDERS {
            Some(PhotoCommand::MonthFolders)
        } else if *id == ids::SELECT_SCREENSHOTS {
            None
        } else {
            return None;
        };
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        if self.viewing.is_some() {
            return Some(vec![self.say(Message::status("Close the viewer first."))]);
        }
        let side = self.active;
        Some(match needs {
            Some(command) => self.with_facts(side, command),
            None => self.select_screenshots(side),
        })
    }

    /// The groups an entry is in, by id.
    fn groups_ids(&self, entry: &Entry) -> Vec<String> {
        self.types
            .groups_of(entry, self.clock.as_ref())
            .map(|groups| groups.iter().map(|g| g.id.clone()).collect())
            .unwrap_or_default()
    }

    /// The panel's shown pictures, in its order, with the indexes of
    /// their rows: a file of the pictures group.
    fn picture_rows(&self, side: Side) -> Vec<(usize, Entry, bool)> {
        let panel = self.panel(side);
        panel
            .list()
            .rows()
            .iter()
            .enumerate()
            .filter_map(|(row, list_row)| {
                let entry = panel.entry(&list_row.id)?;
                if entry.kind != EntryKind::File {
                    return None;
                }
                let groups = self.groups_ids(entry);
                groups
                    .iter()
                    .any(|g| g == IMAGES || g == SCREENSHOTS)
                    .then(|| (row, entry.clone(), groups.iter().any(|g| g == SCREENSHOTS)))
            })
            .collect()
    }

    /// Runs `command` with the panel's pictures' facts, reading them
    /// first when the folder changed since they were read.
    fn with_facts(&mut self, side: Side, command: PhotoCommand) -> Vec<Effect> {
        let panel = self.panel(side);
        let listing = panel.entries_generation();
        let current = self.photos.facts[side.index()]
            .as_ref()
            .is_some_and(|f| f.listing == listing);
        if current || !panel.reads_contents() {
            return self.run_photo_command(side, command);
        }
        let folder = panel.place().clone();
        let places: Vec<Place> = self
            .picture_rows(side)
            .into_iter()
            .filter_map(|(_, entry, _)| folder.join(&entry.name).ok())
            .collect();
        if places.is_empty() {
            return vec![self.say(Message::info(words::NO_PICTURES))];
        }
        let line = words::reading_line(places.len());
        self.photos.pending = Some(Pending {
            side,
            listing,
            command,
        });
        self.request(Request::Photos(Box::new(PhotoAsk::Facts {
            side,
            listing,
            places,
        })));
        vec![self.say(Message::status(line))]
    }

    /// The runtime read a panel's pictures: their facts are kept, and a
    /// command waiting for them runs.
    pub(super) fn photo_note(&mut self, note: PhotoNote) -> Vec<Effect> {
        match note {
            PhotoNote::Facts {
                side,
                listing,
                facts,
            } => {
                if self.panel(side).entries_generation() != listing {
                    return Vec::new();
                }
                self.photos.facts[side.index()] = Some(Facts {
                    listing,
                    by_name: facts.into_iter().collect(),
                });
                match self.photos.pending.take() {
                    Some(pending) if pending.side == side && pending.listing == listing => {
                        if !self.dialogs.is_empty() || self.viewing.is_some() {
                            return Vec::new();
                        }
                        self.run_photo_command(side, pending.command)
                    }
                    other => {
                        self.photos.pending = other;
                        Vec::new()
                    }
                }
            }
        }
    }

    /// The panel's pictures as the helpers see them, in the panel's
    /// order, each with its row's index; facts not read (a remote place)
    /// leave every picture on its modified time.
    fn pictures(&self, side: Side) -> Vec<(usize, Picture)> {
        let offset = self.clock.offset();
        let facts = self.photos.facts[side.index()]
            .as_ref()
            .filter(|f| f.listing == self.panel(side).entries_generation());
        self.picture_rows(side)
            .into_iter()
            .filter_map(|(row, entry, screenshot)| {
                let fact = facts
                    .and_then(|f| f.by_name.get(&entry.name))
                    .cloned()
                    .unwrap_or_default();
                let (at, taken) = match fact.taken {
                    Some(at) => (at, true),
                    None => (Moment::of_time(entry.modified?, offset), false),
                };
                let kind = if screenshot {
                    Kind::Screenshot
                } else if fact.camera.is_some() {
                    Kind::Photo
                } else {
                    Kind::Picture
                };
                Some((
                    row,
                    Picture {
                        name: entry.name.clone(),
                        at,
                        taken,
                        camera: fact.camera,
                        kind,
                    },
                ))
            })
            .collect()
    }

    fn run_photo_command(&mut self, side: Side, command: PhotoCommand) -> Vec<Effect> {
        match command {
            PhotoCommand::NextDay => self.step_day(side, true),
            PhotoCommand::PreviousDay => self.step_day(side, false),
            PhotoCommand::SortTaken => self.sort_by_taken(side),
            PhotoCommand::SelectBursts => self.select_bursts(side),
            PhotoCommand::SelectNoDate => self.select_no_date(side),
            PhotoCommand::Review => self.start_review(side),
            PhotoCommand::MonthFolders => self.month_folders(side),
        }
    }

    /// Next day and Previous day: the first picture of the next or
    /// previous date, its row focused.
    fn step_day(&mut self, side: Side, forward: bool) -> Vec<Effect> {
        let found = self.pictures(side);
        if found.is_empty() {
            return vec![self.say(Message::info(words::NO_PICTURES))];
        }
        let offset = self.clock.offset();
        let panel = self.panel(side);
        let focus = panel.list().focus_index().unwrap_or(0);
        // From the focused picture's day, else the focused entry's.
        let from = found
            .iter()
            .find(|(row, _)| *row == focus)
            .map(|(_, p)| p.at.day())
            .or_else(|| {
                panel
                    .focused_entry()
                    .and_then(|e| e.modified)
                    .map(|t| Moment::of_time(t, offset).day())
            })
            .unwrap_or_else(|| found[0].1.at.day());
        let pictures: Vec<Picture> = found.iter().map(|(_, p)| p.clone()).collect();
        let Some((first, day, n)) = step_day(&pictures, from, forward) else {
            let line = if forward {
                words::NO_LATER_DAY
            } else {
                words::NO_EARLIER_DAY
            };
            return vec![self.say(Message::info(line))];
        };
        let row = found[first].0;
        self.panel_mut(side).list_mut().move_focus(Move::To(row));
        self.status.clear();
        let this_year = self.now().to_civil(offset).year;
        let line = day_line(forward, day, n, this_year);
        let mut effects = vec![self.say(Message::info(line))];
        effects.extend(self.focus_effects(side));
        effects
    }

    /// Sort by date taken: each picture's date taken, else its modified
    /// time, into the rows, then the sort.
    fn sort_by_taken(&mut self, side: Side) -> Vec<Effect> {
        let found = self.pictures(side);
        if found.is_empty() {
            return vec![self.say(Message::info(words::NO_PICTURES))];
        }
        let words: HashMap<&str, String> = found
            .iter()
            .map(|(_, p)| (p.name.as_str(), taken_words(p)))
            .collect();
        let cells: Vec<ColumnCell> = self
            .panel(side)
            .all_entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| ColumnCell {
                index: index as u32,
                name: entry.name.clone(),
                values: vec![(
                    TAKEN.to_owned(),
                    words.get(entry.name.as_str()).cloned().unwrap_or_default(),
                )],
            })
            .collect();
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].apply_column_cells(cells, clock.as_ref());
        let spec = self
            .panel(side)
            .sort()
            .choose(SortKey::Extra(TAKEN.to_owned()));
        self.panels[side.index()].set_sort(spec.clone(), clock.as_ref());
        self.taken_sorted(side, &spec)
    }

    /// The panel sorted by date taken: "Sorted by date taken, oldest
    /// first", then "Date taken; 12 use modified time" when some pictures
    /// have no date.
    pub(super) fn taken_sorted(
        &mut self,
        side: Side,
        spec: &diroxide_core::SortSpec,
    ) -> Vec<Effect> {
        let without = self
            .panel(side)
            .all_entries()
            .iter()
            .filter(|e| {
                e.extra
                    .get(TAKEN)
                    .is_some_and(|v| v.ends_with(" (modified)"))
            })
            .count();
        let mut line = taken_sort_line(spec.direction == diroxide_core::SortDirection::Ascending);
        if without > 0 {
            line = format!("{line}. {}.", fallback_note(without));
        }
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Selects the rows whose names `chosen` holds, keeping the marks
    /// before for Restore selection; returns how many are selected now.
    fn select_names(&mut self, side: Side, chosen: &HashSet<String>) -> usize {
        self.keep_selection(side);
        let list = self.panel_mut(side).list_mut();
        list.select_none();
        list.select_matching(|row| chosen.contains(&row.id));
        list.selected_count()
    }

    /// Select the screenshots: the files of the screenshots group.
    fn select_screenshots(&mut self, side: Side) -> Vec<Effect> {
        let chosen: HashSet<String> = self
            .picture_rows(side)
            .into_iter()
            .filter(|(_, _, screenshot)| *screenshot)
            .map(|(_, entry, _)| entry.name)
            .collect();
        if chosen.is_empty() {
            return vec![self.say(Message::info(words::NO_SCREENSHOTS))];
        }
        let total = self.select_names(side, &chosen);
        let line = words::selected_line(
            &count(chosen.len() as u64, "screenshot", "screenshots"),
            total,
        );
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Select the bursts: pictures from one camera within the gap.
    fn select_bursts(&mut self, side: Side) -> Vec<Effect> {
        let found = self.pictures(side);
        let pictures: Vec<Picture> = found.into_iter().map(|(_, p)| p).collect();
        let gap = self.settings.photos.burst_gap_seconds;
        let runs = bursts(&pictures, gap);
        if runs.is_empty() {
            let line = words::no_bursts_line(gap);
            return vec![self.say(Message::info(line))];
        }
        let chosen: HashSet<String> = runs
            .iter()
            .flatten()
            .map(|i| pictures[*i].name.clone())
            .collect();
        let total = self.select_names(side, &chosen);
        let line = match runs.as_slice() {
            [one] => format!("{}. {total} selected.", burst_line(&pictures, one)),
            many => words::selected_line(
                &format!(
                    "{}, {}",
                    count(many.len() as u64, "burst", "bursts"),
                    count(chosen.len() as u64, "picture", "pictures")
                ),
                total,
            ),
        };
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Select pictures with no date taken.
    fn select_no_date(&mut self, side: Side) -> Vec<Effect> {
        let chosen: HashSet<String> = self
            .pictures(side)
            .into_iter()
            .filter(|(_, p)| !p.taken)
            .map(|(_, p)| p.name)
            .collect();
        if chosen.is_empty() {
            return vec![self.say(Message::info(words::ALL_DATED))];
        }
        let total = self.select_names(side, &chosen);
        let what = format!(
            "{} with no date taken",
            count(chosen.len() as u64, "picture", "pictures")
        );
        let line = words::selected_line(&what, total);
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Review pictures: the marked pictures, or every picture, from the
    /// focused one.
    fn start_review(&mut self, side: Side) -> Vec<Effect> {
        let found = self.pictures(side);
        let marked: HashSet<String> = self
            .panel(side)
            .list()
            .selected_ids()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let focus = self.panel(side).list().focus_index().unwrap_or(0);
        let chosen: Vec<(usize, Picture)> = if marked.is_empty() {
            found
        } else {
            found
                .into_iter()
                .filter(|(_, p)| marked.contains(&p.name))
                .collect()
        };
        if chosen.is_empty() {
            return vec![self.say(Message::info(words::NO_PICTURES))];
        }
        let at = chosen
            .iter()
            .position(|(row, _)| *row == focus)
            .unwrap_or(0);
        let review = Review {
            side,
            folder: self.panel(side).place().clone(),
            done: vec![None; chosen.len()],
            pictures: chosen.into_iter().map(|(_, p)| (p.name, p.kind)).collect(),
            at,
        };
        self.show_review(review)
    }

    fn show_review(&mut self, review: Review) -> Vec<Effect> {
        // The panel follows the picture, so the quick view shows it.
        let (name, side) = (review.pictures[review.at].0.clone(), review.side);
        if let Some(row) = self
            .panel(side)
            .list()
            .rows()
            .iter()
            .position(|r| r.id == name)
        {
            self.panel_mut(side).list_mut().move_focus(Move::To(row));
        }
        let list = review.list();
        let mut effects = vec![self.redraw_panel(side)];
        effects.extend(self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Photos(PhotoWants::Review(Box::new(review))),
        ));
        effects
    }

    /// A key in the review: its one-key answers.
    pub(super) fn photo_key(&mut self, wants: &PhotoWants, key: &Key) -> Option<Vec<Effect>> {
        let PhotoWants::Review(review) = wants else {
            return None;
        };
        if key.mods.ctrl || key.mods.alt {
            return None;
        }
        let answer = match super::typed(key)?.to_ascii_lowercase() {
            'k' => "keep",
            'l' => "label",
            'm' => "move",
            't' => "trash",
            's' => "skip",
            'b' => "back",
            _ => return None,
        };
        Some(self.review_answer((**review).clone(), answer))
    }

    /// One answer in the review.
    fn review_answer(&mut self, mut review: Review, answer: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let place = review.place();
        match answer {
            "keep" => review.done[review.at] = Some(Done::Kept),
            "skip" => {
                if review.done[review.at].is_none() {
                    review.done[review.at] = Some(Done::Skipped);
                }
            }
            "back" => {
                if review.at == 0 {
                    effects.push(self.say(Message::status(words::FIRST_PICTURE)));
                    effects.extend(self.show_review(review));
                    return effects;
                }
                review.at -= 1;
                effects.extend(self.show_review(review));
                return effects;
            }
            "trash" => {
                let Some(place) = place else {
                    return effects;
                };
                let job = self.next_job_id();
                let request = JobRequest::new(job, JobKind::Trash, vec![place]);
                effects.extend(self.plan_job(review.side, request, None, Then::Report, false));
                review.done[review.at] = Some(Done::Trashed);
            }
            "label" => {
                // The label field opens over the review, which stays.
                let name = review.pictures[review.at].0.clone();
                review.done[review.at] = Some(Done::Labeled);
                let side = review.side;
                effects.extend(self.show_review_at(review));
                if let Some(place) = place {
                    let one = diroxide_core::names::display_name(&name).into_owned();
                    let label = format!("Set label on {one}");
                    let field = EntryField::new(label, CompletionSource::Labels);
                    effects.extend(self.open_organize(
                        Dialog::edit(field),
                        Wants::Label {
                            side,
                            places: vec![place],
                            one: Some(one),
                            toggle: false,
                        },
                    ));
                    effects.extend(self.refresh_candidates());
                }
                return effects;
            }
            "move" => {
                let name =
                    diroxide_core::names::display_name(&review.pictures[review.at].0).into_owned();
                let side = review.side;
                let start = self
                    .photos
                    .last_move
                    .clone()
                    .unwrap_or_else(|| self.panel(side.other()).place().to_string());
                let places: Vec<String> = self
                    .place_candidates(side, true)
                    .into_iter()
                    .map(|c| c.text)
                    .collect();
                let field =
                    EntryField::new(format!("Move {name} to"), CompletionSource::Values(places))
                        .with_text(start);
                effects.extend(self.open_organize(
                    Dialog::edit(field),
                    Wants::Photos(PhotoWants::ReviewMove(Box::new(review))),
                ));
                return effects;
            }
            _ => {}
        }
        effects.extend(self.review_next(review));
        effects
    }

    /// The review at the same picture again, without moving the panel.
    fn show_review_at(&mut self, review: Review) -> Vec<Effect> {
        let list = review.list();
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Photos(PhotoWants::Review(Box::new(review))),
        )
    }

    /// The next picture, or the end of the review with its tally.
    fn review_next(&mut self, mut review: Review) -> Vec<Effect> {
        if review.at + 1 >= review.pictures.len() {
            let line = review.tally(true);
            return vec![self.say(Message::done(line))];
        }
        review.at += 1;
        self.show_review(review)
    }

    /// The review's move place typed: the picture moves there as a job,
    /// Undo takes it back; the review goes on.
    fn review_moved(&mut self, mut review: Review, text: &str) -> Vec<Effect> {
        let side = review.side;
        let destination = match self.typed_place(side, text) {
            Ok(place) => place,
            Err(message) => return vec![self.say(message)],
        };
        let mut effects = self.close_top();
        let Some(place) = review.place() else {
            return effects;
        };
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Move, vec![place]).to(destination.clone());
        effects.extend(self.plan_job(side, request, None, Then::Report, false));
        self.photos.last_move = Some(text.trim().to_owned());
        review.done[review.at] = Some(Done::Moved(crate::panel::place_leaf(&destination)));
        effects.extend(self.review_next(review));
        effects
    }

    /// Move into month folders: the question and its rows made here, the
    /// moves planned by the job engine on Yes and run as one job.
    fn month_folders(&mut self, side: Side) -> Vec<Effect> {
        let found = self.pictures(side);
        let marked: HashSet<String> = self
            .panel(side)
            .list()
            .selected_ids()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let pictures: Vec<Picture> = found
            .into_iter()
            .map(|(_, p)| p)
            .filter(|p| marked.is_empty() || marked.contains(&p.name))
            .collect();
        if pictures.is_empty() {
            return vec![self.say(Message::info(words::NO_PICTURES))];
        }
        let folder = self.panel(side).place().clone();
        let by_month = months(&pictures);
        let mut moves = Vec::new();
        let mut rows = Vec::new();
        for (month, members) in &by_month {
            let Ok(target) = folder.join(month) else {
                continue;
            };
            let existing = self.panel(side).entry(month).map(|e| e.kind);
            if existing.is_some_and(|kind| kind != EntryKind::Folder) {
                let line = words::month_taken_line(month);
                return vec![self.say(Message::error(line))];
            }
            rows.push(month_row(month, members.len(), existing.is_none()));
            for i in members {
                if let Ok(from) = folder.join(&pictures[*i].name) {
                    moves.push((from, target.clone()));
                }
            }
        }
        let without = pictures.iter().filter(|p| !p.taken).count();
        if without > 0 {
            rows.push(format!("{}.", fallback_note(without)));
            rows.push(NO_DATE_LINE.to_owned());
        }
        let job = self.next_job_id();
        let mut request = JobRequest::new(
            job,
            JobKind::Move,
            moves.iter().map(|(from, _)| from.clone()).collect(),
        );
        request.destination = Some(folder.clone());
        request.conflicts = ConflictPolicy::Ask;
        let question = with_default_no(Message::question(&month_question(
            pictures.len(),
            by_month.len(),
        )));
        self.open_organize(
            Dialog::Confirm {
                question,
                listing: rows,
                on_yes: Command::new(ids::YES),
            },
            Wants::Photos(PhotoWants::Months(Box::new(MonthMoves { request, moves }))),
        )
    }

    /// Enter in one of the helpers' dialogs.
    pub(super) fn photo_accept(
        &mut self,
        wants: PhotoWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            PhotoWants::Review(review) => match row {
                Some(row) => self.review_answer(*review, &row),
                None => Vec::new(),
            },
            PhotoWants::ReviewMove(review) => self.review_moved(*review, text),
            PhotoWants::Months(_) => self.dialog_escape(),
        }
    }

    /// Yes to the month folders' question: the engine plans the moves
    /// (a hook's veto or a plan's own question is said then), and the job
    /// runs with no second question, its result said and its Undo kept as
    /// any job's.
    pub(super) fn photo_yes(&mut self, wants: PhotoWants) -> Vec<Effect> {
        let PhotoWants::Months(months) = wants else {
            return Vec::new();
        };
        let MonthMoves { mut request, moves } = *months;
        if request.errors.is_ask() {
            request.errors = self.settings.jobs.on_errors.policy();
        }
        self.flows.insert(
            request.id,
            Flow {
                side: self.active,
                note: None,
                then: Then::Report,
                confirm: false,
                kind: JobKind::Move,
            },
        );
        self.request(Request::PlanMoves {
            request: Box::new(request),
            moves,
        });
        Vec::new()
    }

    /// Escape on one of the helpers' dialogs, after it closed.
    pub(super) fn photo_escape(&mut self, wants: PhotoWants) -> Vec<Effect> {
        match wants {
            PhotoWants::Review(review) => {
                let line = review.tally(false);
                vec![self.say(Message::info(line))]
            }
            PhotoWants::ReviewMove(review) => self.show_review_at(*review),
            PhotoWants::Months(_) => vec![self.say(Message::info(words::NOTHING_MOVED))],
        }
    }

    /// A batch of column cells, before the rows take them: a picture with
    /// no date taken shows its modified time, marked so; a screenshot's
    /// kind is said as one (the readers know neither).
    pub(super) fn photo_cells(&self, side: Side, cells: &mut [ColumnCell]) {
        let panel = self.panel(side);
        let offset = self.clock.offset();
        for cell in cells.iter_mut() {
            let Some(entry) = panel
                .all_entries()
                .get(cell.index as usize)
                .filter(|e| e.name == cell.name)
            else {
                continue;
            };
            for (key, value) in &mut cell.values {
                match PhotoField::of(key) {
                    Some(PhotoField::Taken) if value == NO_DATE => {
                        *value = entry.modified.map_or_else(String::new, |t| {
                            format!("{} (modified)", Moment::of_time(t, offset).words())
                        });
                    }
                    Some(PhotoField::Kind)
                        if !value.is_empty()
                            && self.groups_ids(entry).iter().any(|g| g == SCREENSHOTS) =>
                    {
                        *value = Kind::Screenshot.word().to_owned();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A picture's Taken words: "2026-10-12 14:03", or its modified time
/// with "(modified)" after it.
fn taken_words(picture: &Picture) -> String {
    if picture.taken {
        picture.at.words()
    } else {
        format!("{} (modified)", picture.at.words())
    }
}

/// "Sorted by date taken, oldest first".
pub(super) fn taken_sort_line(ascending: bool) -> String {
    if ascending {
        "Sorted by date taken, oldest first".to_owned()
    } else {
        "Sorted by date taken, newest first".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn the_review_says_its_place_and_its_tally() {
        let mut review = Review {
            side: Side::Left,
            folder: Place::parse(if cfg!(windows) {
                "D:\\photos"
            } else {
                "/photos"
            })
            .unwrap(),
            pictures: vec![
                ("IMG_2041.jpg".into(), Kind::Photo),
                ("Screenshot (3).png".into(), Kind::Screenshot),
                ("c.png".into(), Kind::Picture),
            ],
            at: 0,
            done: vec![None, None, None],
        };
        assert_eq!(review.title(), "Review: 1 of 3, IMG_2041.jpg, photo");
        let rows: Vec<String> = review.list().rows().iter().map(|r| r.spoken()).collect();
        assert_eq!(
            rows,
            [
                "Keep, K",
                "Label, L",
                "Move to a place you pick, M",
                "Trash, T",
                "Skip, S",
                "Back, B"
            ]
        );
        review.done[0] = Some(Done::Moved("Keepers".into()));
        assert_eq!(
            review.title(),
            "Review: 1 of 3, IMG_2041.jpg, photo, moved to Keepers"
        );
        review.at = 1;
        review.done[1] = Some(Done::Trashed);
        assert_eq!(
            review.tally(false),
            "Review ended at 2 of 3, 1 moved, 1 in the trash"
        );
        review.done[2] = Some(Done::Kept);
        assert_eq!(
            review.tally(true),
            "Review done: 3 pictures, 1 kept, 1 moved, 1 in the trash"
        );
        for line in [
            review.title(),
            taken_sort_line(true),
            taken_sort_line(false),
        ] {
            assert!(within_cells(&line, &line[..line.len().min(36)]), "{line}");
        }
    }
}
