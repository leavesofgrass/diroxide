//! Archives in the panels (plan commit 59): entering and leaving them, the
//! title's words and the line said on opening ("Opened course.zip: 10
//! items, 48 KB."), Open as archive, the read-only refusals in words, the
//! limit questions a listing asks, and forgetting a locked archive's
//! password when the panel leaves it.
//!
//! Enter and Right open an archive like a folder and Left and Backspace
//! leave it (navigation's own commands); nothing is extracted to browse.
//! The runtime lists a place inside an archive through the archive
//! provider and sends [`ArchiveFacts`] before the listing, which this
//! module keeps per panel.

use diroxide_core::format::count;
use diroxide_core::{Effect, Message, Place, PlaceKind, RequestId, Side};

use super::{App, Purpose};
use crate::archives::ArchiveFacts;
use crate::dialog::{Dialog, with_default_no};
use crate::panel::place_leaf;
use crate::request::{AskId, Reply, Request};
use crate::state;

/// The sentence for a change asked inside an archive (W2a, P2b).
pub(crate) const READ_ONLY: &str = diroxide_archive::READ_ONLY;

/// Whether `place` is `container` or inside it, at any depth of nesting.
pub(crate) fn inside(place: &Place, container: &Place) -> bool {
    match place.archive() {
        Some((outer, _)) => outer == container || inside(outer, container),
        None => false,
    }
}

/// A title's words split at its last ", in ": the place's own words and
/// where it is ("course.zip, archive" and "D:\docs").
fn split_title(words: &str) -> (&str, Option<&str>) {
    match words.rfind(", in ") {
        Some(at) => (&words[..at], Some(&words[at + ", in ".len()..])),
        None => (words, None),
    }
}

impl App {
    /// An archive's facts arrived for a listing: kept until its listing
    /// comes.
    pub(super) fn archive_opened(
        &mut self,
        request: RequestId,
        facts: ArchiveFacts,
    ) -> Vec<Effect> {
        if Side::BOTH
            .into_iter()
            .any(|side| self.panel(side).waits_for(request))
        {
            self.pending_archive.retain(|(r, _)| *r != request);
            self.pending_archive.push((request, facts));
        }
        Vec::new()
    }

    /// The facts of the listing `request` brought, for the panel on
    /// `side` that now shows `place`; a locked archive the panel left is
    /// forgotten, so its password goes with it.
    pub(super) fn take_archive_facts(&mut self, side: Side, request: RequestId, place: &Place) {
        let mut facts = self
            .pending_archive
            .iter()
            .position(|(r, _)| *r == request)
            .map(|at| self.pending_archive.remove(at).1)
            .filter(|f| &f.place == place);
        let previous = self.archive_facts[side.index()].take();
        self.archive_entered[side.index()] = match (&facts, &previous) {
            (Some(now), Some(before)) => now.container != before.container,
            (Some(_), None) => true,
            _ => false,
        };
        if let Some(previous) = previous {
            let same = facts
                .as_ref()
                .is_some_and(|f| f.container == previous.container);
            if same {
                if let Some(f) = facts.as_mut() {
                    f.locked |= previous.locked;
                }
            } else if previous.locked && !inside(place, &previous.container) {
                self.request(Request::ForgetArchive(previous.container));
            }
        }
        self.archive_facts[side.index()] = facts;
    }

    /// What a panel's archive facts say, when it shows a place inside an
    /// archive.
    pub(super) fn archive_facts_of(&self, side: Side) -> Option<&ArchiveFacts> {
        self.archive_facts[side.index()]
            .as_ref()
            .filter(|f| &f.place == self.panel(side).place())
    }

    /// The words a place inside an archive is titled by: the provider's,
    /// once listed ("a.zip, 7z archive, in D:\docs"), else the place's own
    /// ("week1, in course.zip").
    pub(super) fn archive_title_words(&self, side: Side, place: &Place) -> String {
        self.archive_facts[side.index()]
            .as_ref()
            .filter(|f| &f.place == place)
            .map_or_else(|| place.title(), |f| f.title.clone())
    }

    /// The title line of a panel inside an archive, the archive's words
    /// first as part three of the alpha.2 plan has them: "course.zip,
    /// archive, in D:\docs, left panel"; "week1, in course.zip, left
    /// panel". A long folder is cut from its left so the place's own words
    /// and the panel always fit.
    pub(super) fn archive_title_line(
        &self,
        place: &Place,
        side_words: &str,
        tail: &str,
        cells: usize,
    ) -> String {
        let words =
            diroxide_core::names::display_name(&self.archive_title_words(self.active, place))
                .into_owned();
        let (head, parent) = split_title(&words);
        let Some(parent) = parent else {
            return format!("{head}, {side_words}{tail}");
        };
        let whole = format!("{head}, in {parent}, {side_words}{tail}");
        if whole.chars().count() <= cells {
            return whole;
        }
        let room = cells.saturating_sub(
            head.chars().count()
                + ", in ...".len()
                + 2
                + side_words.chars().count()
                + tail.chars().count(),
        );
        let chars: Vec<char> = parent.chars().collect();
        let start = chars.len().saturating_sub(room.max(1));
        let kept: String = chars[start..].iter().collect();
        format!("{head}, in ...{kept}, {side_words}{tail}")
    }

    /// What is said when a panel lands inside an archive: the opening line
    /// at an archive's root ("Opened course.zip: 10 items, 48 KB."), the
    /// folder, its count and the archive inside it ("week1, 3 items, in
    /// course.zip"), then any note ("Over 500,000 entries: ...").
    pub(super) fn archive_landing(&self, side: Side) -> Option<String> {
        let facts = self.archive_facts_of(side)?;
        let panel = self.panel(side);
        let mut line = match facts.place.archive() {
            Some((_, "")) if self.archive_entered[side.index()] => facts.opened.clone(),
            _ => {
                let (_, parent) = split_title(&facts.title);
                let items = count(panel.list().len() as u64, "item", "items");
                match parent {
                    Some(parent) => format!("{}, {items}, in {parent}", place_leaf(&facts.place)),
                    None => format!("{}, {items}", place_leaf(&facts.place)),
                }
            }
        };
        for note in &facts.notes {
            line.push(' ');
            line.push_str(note);
            if !note.ends_with('.') {
                line.push('.');
            }
        }
        Some(line)
    }

    /// The read-only refusal for a change asked of a panel inside an
    /// archive: "Archives are read only here. Use Pack to make a new one."
    pub(super) fn read_only_refusal(&mut self, side: Side) -> Option<Vec<Effect>> {
        (self.panel(side).place().kind() == PlaceKind::Archive)
            .then(|| vec![self.say(Message::error(READ_ONLY))])
    }

    /// Open as archive: the focused file opened as an archive by what its
    /// bytes say, whatever its name (a `.jar`, a `.docx`, a file named
    /// oddly).
    pub(super) fn open_as_archive(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if entry.is_folder_like() && entry.kind != diroxide_core::EntryKind::Archive {
            let text = format!("Not a file: {}", entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        self.open(side, place.into_archive(), None)
    }

    /// A limit's question from a listing or a job: a confirmation that
    /// starts on No ("Expands to 40 GB from 40 KB. Unpack? No is the
    /// default. y or n"); the answer goes back to the worker.
    pub(super) fn ask_limit(
        &mut self,
        ask: AskId,
        question: &diroxide_archive::Question,
    ) -> Vec<Effect> {
        let message = with_default_no(question.message());
        self.open_dialog(
            Dialog::Confirm {
                question: message,
                listing: Vec::new(),
                on_yes: diroxide_core::Command::new(crate::commands::ids::YES),
            },
            Purpose::Limit { ask },
        )
    }

    /// The answer to a limit's question.
    pub(super) fn answer_limit(&mut self, ask: AskId, yes: bool) {
        self.request(Request::Answer {
            ask,
            reply: Reply::Confirm(yes),
        });
    }

    /// The sweep at start: the spool folder, and the folders packs wrote
    /// into.
    pub(super) fn sweep_at_start(&mut self) {
        let folders: Vec<Place> = self
            .history_of(state::keys::PACK_FOLDERS)
            .iter()
            .filter_map(|text| Place::parse(text).ok())
            .collect();
        self.request(Request::Sweep { folders });
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
    fn a_place_is_inside_its_archive_at_any_depth() {
        let zip = p("/d/course.zip");
        assert!(inside(&p("/d/course.zip!week1"), &zip));
        assert!(inside(&p("/d/course.zip!inner.zip!a"), &zip));
        assert!(!inside(&p("/d/other.zip!week1"), &zip));
        assert!(!inside(&p("/d"), &zip));
    }

    #[test]
    fn titles_split_at_the_last_in() {
        assert_eq!(
            split_title("course.zip, archive, in D:\\docs"),
            ("course.zip, archive", Some("D:\\docs"))
        );
        assert_eq!(
            split_title("week1, in course.zip"),
            ("week1", Some("course.zip"))
        );
        assert_eq!(
            split_title("course.zip, archive"),
            ("course.zip, archive", None)
        );
    }
}
