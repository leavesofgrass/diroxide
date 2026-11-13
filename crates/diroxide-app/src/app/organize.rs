//! The labels, bookmarks, folder views and History list in the app (W3l,
//! plan commits 22 to 25): what the app holds for them, their commands'
//! routes, what their dialogs are for, and the hooks the app's dispatch,
//! keys, dialogs and undo call. The behavior is in `labeling`,
//! `bookmarking` and `folder_views`.
//!
//! The rules are the app's: every list and question is a dialog under the
//! contract (modal, named, Escape closes, the focus returns); every row
//! and sentence says its meaning in its first 40 cells; one announcement
//! per change; nothing is written but through the runtime, atomically, in
//! the state folder.

use std::collections::HashSet;

use diroxide_config::ColumnSpec;
use diroxide_core::{
    Command, CommandId, Effect, Key, Layer, ListModel, Message, Place, RequestId, Side, SortSpec,
};

use super::{App, Purpose, typed};
use crate::bookmarks::Bookmarks;
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::keymap::Lookup;
use crate::labels::{LabelStore, LabelUndo};
use crate::views::FolderViews;

/// What the app holds for the labels, bookmarks and folder views.
#[derive(Clone, Debug, Default)]
pub(super) struct Organize {
    /// The labels and the labeled places.
    pub(super) labels: LabelStore,
    /// The label changes to undo, newest last, each with the newest job
    /// in the undo log when it was made, so Undo takes whichever came
    /// last.
    pub(super) label_undo: Vec<(u64, LabelUndo)>,
    /// The Labels list's check for missing items.
    pub(super) check: Option<Check>,
    /// The bookmarks.
    pub(super) bookmarks: Bookmarks,
    /// The folders' own views.
    pub(super) views: FolderViews,
    /// Each panel's usual view while a folder's own view is applied, put
    /// back on leaving that folder.
    pub(super) usual: [Option<Look>; 2],
}

impl Organize {
    /// What the app starts with.
    pub(super) fn new(labels: LabelStore, bookmarks: Bookmarks, views: FolderViews) -> Self {
        Self {
            labels,
            bookmarks,
            views,
            ..Self::default()
        }
    }
}

/// The Labels list's check for labeled items that are gone.
#[derive(Clone, Debug)]
pub(super) struct Check {
    /// The request the runtime answers.
    pub(super) request: RequestId,
    /// The keys of the places found gone, once the answer came.
    pub(super) missing: Option<HashSet<String>>,
    /// How many places could not be looked at.
    pub(super) unreadable: usize,
}

/// A panel's view: what a folder's own view changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Look {
    pub(super) sort: SortSpec,
    pub(super) show_hidden: bool,
    pub(super) columns: Vec<ColumnSpec>,
}

/// A bookmark setting typed in a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BookmarkField {
    Name,
    Category,
    Alias,
}

/// What a dialog of these lists is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Wants {
    /// Set or toggle a label on these places: a field with the labels
    /// listed under it.
    Label {
        side: Side,
        places: Vec<Place>,
        /// The item's name when there is one item.
        one: Option<String>,
        toggle: bool,
    },
    /// The Labels list; Enter opens a label's items as search results
    /// (W3a).
    Labels,
    /// The Bookmarks list for a panel.
    Bookmarks { side: Side },
    /// The question before a bookmark is removed.
    RemoveBookmark { side: Side, place: Place },
    /// A bookmark's settings, as a list.
    BookmarkForm { side: Side, place: Place },
    /// One of a bookmark's settings being typed.
    BookmarkText {
        side: Side,
        place: Place,
        field: BookmarkField,
    },
    /// The History list for a panel.
    History { side: Side },
    /// A list of the file types' lane (alpha 4, lane TA): Actions for
    /// this file, Open with, the encoding chooser. Hosted here so the
    /// window's test of every purpose keeps its count.
    Types(super::type_actions::TypeWants),
    /// The Convert command's lists and question (alpha 4, lane AW),
    /// hosted here for the same reason.
    Convert(super::converting::ConvertWants),
    /// The Watched folders list, a task's actions and its questions
    /// (alpha 4, lane AW), hosted here for the same reason.
    Watched(super::watched::WatchedWants),
    /// Set note: the note typed for these places (alpha 4, lane MA).
    Note {
        side: Side,
        places: Vec<Place>,
        /// The item's name when there is one item.
        one: Option<String>,
    },
    /// Set field: `name: value` typed for these places.
    Field {
        side: Side,
        places: Vec<Place>,
        /// The item's name when there is one item.
        one: Option<String>,
    },
    /// The question before the columns read many files, for a sort or
    /// Fill columns (alpha 4, lane MA).
    ReadColumns {
        side: Side,
        fill: super::columns_ui::Fill,
    },
    /// A list or a field of the metadata commands (alpha 4, lane MA).
    Meta(super::metadata_ui::MetaWants),
    /// A question of the Encryption submenu (alpha 4, lane AE).
    Crypt(super::crypt_ui::CryptWants),
    /// A list or a field of Change time and Change owner (alpha 4, lane
    /// AP).
    Access(super::access_ui::AccessWants),
    /// The review, its move field and the month folders' question (alpha
    /// 4, lane AP).
    Photos(super::photos_ui::PhotoWants),
    /// The tab list, Save tab set's name and the saved sets (alpha 5,
    /// lane TB).
    Tabs(super::tabbing::TabWants),
    /// The dialogs of batch rename (alpha 5, lane RA; reserved by lane
    /// P5), hosted here for the same reason.
    Rename(super::renaming::RenameWants),
    /// The dialogs of compare folders and synchronize (alpha 5, lane SA;
    /// reserved by lane P5).
    Sync(super::syncing::SyncWants),
    /// The dialogs of duplicates (alpha 5, lane DA; reserved by lane P5).
    Dups(super::duplicates::DupsWants),
    /// The dialogs of the flat view, collections and recent files (alpha
    /// 5, lane LA; reserved by lane P5).
    Lists(super::collections::ListsWants),
}

impl Wants {
    /// Whether the dialog's list stays open while typing narrows it, as
    /// the palette's does: Escape then closes the dialog at once.
    pub(super) fn list_always(&self) -> bool {
        matches!(self, Wants::Label { .. })
    }

    /// The dialog's purpose as [`App::dialog_purpose`] gives it: the W3l
    /// lists are "Organize"; every list and question the alpha 4 lanes
    /// host here has its own name (R30's commit 114, the fold lane), so
    /// the window's test of every purpose opens each one. The matches
    /// list every variant, so a dialog added here later needs a name.
    pub(super) fn purpose(&self) -> &'static str {
        use super::access_ui::AccessWants;
        use super::converting::ConvertWants;
        use super::crypt_ui::CryptWants;
        use super::metadata_ui::MetaWants;
        use super::photos_ui::PhotoWants;
        use super::tabbing::TabWants;
        use super::type_actions::TypeWants;
        use super::watched::WatchedWants;
        match self {
            Wants::Label { .. }
            | Wants::Labels
            | Wants::Bookmarks { .. }
            | Wants::RemoveBookmark { .. }
            | Wants::BookmarkForm { .. }
            | Wants::BookmarkText { .. }
            | Wants::History { .. } => "Organize",
            Wants::Types(wants) => match wants {
                TypeWants::Actions { .. } => "TypeActions",
                TypeWants::Encoding { .. } => "Encoding",
                TypeWants::OpenWith { .. } => "OpenWith",
            },
            Wants::Convert(wants) => match wants {
                ConvertWants::Recipes { .. } => "ConvertRecipes",
                ConvertWants::Run { .. } => "ConvertPlan",
                ConvertWants::Reports(_) => "ConvertReports",
            },
            Wants::Watched(wants) => match wants {
                WatchedWants::List(_) => "WatchedFolders",
                WatchedWants::Actions(_) => "WatchedActions",
                WatchedWants::Approve { .. } => "WatchedApprove",
                WatchedWants::CatchUp { .. } => "WatchedCatchUp",
                WatchedWants::PutBack { .. } => "WatchedPutBack",
            },
            Wants::Note { .. } => "Note",
            Wants::Field { .. } => "Field",
            Wants::ReadColumns { .. } => "ReadColumns",
            Wants::Meta(wants) => match wants {
                MetaWants::Checksums(_) => "Checksums",
                MetaWants::Compare(_) => "ChecksumCompare",
                MetaWants::VerifyLists { .. } => "VerifyLists",
                MetaWants::VerifyReport { .. } => "VerifyReport",
                MetaWants::StripQuestion { .. } => "StripQuestion",
                MetaWants::MarkQuestion { .. } => "MarkQuestion",
                MetaWants::Lines => "MetaLines",
            },
            Wants::Crypt(wants) => match wants {
                CryptWants::TrashOriginals { .. } => "TrashOriginals",
                CryptWants::Recipients { .. } => "GpgRecipients",
                CryptWants::Signer { .. } => "GpgSigner",
                CryptWants::Run { .. } => "GpgPlan",
                CryptWants::MinisignKey { .. } => "MinisignKey",
            },
            Wants::Access(wants) => match wants {
                AccessWants::TimeForm(_) => "TimeForm",
                AccessWants::TypedTime(_) => "TypedTime",
                AccessWants::OwnerForm(_) => "OwnerForm",
                AccessWants::OwnerName(_) => "OwnerName",
                AccessWants::GroupName(_) => "GroupName",
            },
            Wants::Photos(wants) => match wants {
                PhotoWants::Review(_) => "Review",
                PhotoWants::ReviewMove(_) => "ReviewMove",
                PhotoWants::Months(_) => "MonthFolders",
            },
            Wants::Tabs(wants) => match wants {
                TabWants::List { .. } => "TabList",
                TabWants::CloseLocked { .. } => "TabCloseLocked",
                TabWants::SaveName => "TabSetName",
                TabWants::Sets => "TabSets",
            },
            // Reserved for alpha 5's app lanes (lane P5): each lane names
            // its own purposes.
            Wants::Rename(wants) => wants.purpose(),
            Wants::Sync(wants) => wants.purpose(),
            Wants::Dups(wants) => wants.purpose(),
            Wants::Lists(wants) => wants.purpose(),
        }
    }
}

/// What a command of these lists does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    SetLabel,
    ToggleLabel,
    ClearLabels,
    Labels,
    Favorite(usize),
    Bookmarks,
    BookmarkFolder,
    BookmarkItem,
    History,
    ForgetPlace,
    SaveView,
    ForgetView,
    Note,
    Field,
}

const ROUTES: [(CommandId, Do); 20] = [
    (ids::LABEL_SET, Do::SetLabel),
    (ids::LABEL_TOGGLE, Do::ToggleLabel),
    (ids::LABEL_CLEAR, Do::ClearLabels),
    (ids::LABELS, Do::Labels),
    (ids::LABEL_NOTE, Do::Note),
    (ids::LABEL_FIELD, Do::Field),
    (ids::LABEL_FAVORITE_1, Do::Favorite(1)),
    (ids::LABEL_FAVORITE_2, Do::Favorite(2)),
    (ids::LABEL_FAVORITE_3, Do::Favorite(3)),
    (ids::LABEL_FAVORITE_4, Do::Favorite(4)),
    (ids::LABEL_FAVORITE_5, Do::Favorite(5)),
    (ids::LABEL_FAVORITE_6, Do::Favorite(6)),
    (ids::LABEL_FAVORITE_7, Do::Favorite(7)),
    (ids::BOOKMARKS, Do::Bookmarks),
    (ids::BOOKMARK_ADD, Do::BookmarkFolder),
    (ids::BOOKMARK_ITEM, Do::BookmarkItem),
    (ids::HISTORY_LIST, Do::History),
    (ids::FORGET_PLACE, Do::ForgetPlace),
    (ids::SAVE_VIEW, Do::SaveView),
    (ids::FORGET_VIEW, Do::ForgetView),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

impl App {
    /// Runs a command of the labels, bookmarks, views or the History list;
    /// `None` when the command is another's. With a dialog or the viewer
    /// open it says what is in the way, as the panel's commands do.
    pub(super) fn organize_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        if self.viewing.is_some() {
            let close = self.key(&ids::CLOSE_VIEWER);
            let text = if close.is_empty() {
                "Close the viewer first.".to_owned()
            } else {
                format!("Close the viewer first. {close} closes it.")
            };
            return Some(vec![self.say(Message::status(text))]);
        }
        let side = self.active;
        Some(match action {
            Do::SetLabel => self.label_prompt(side, false, command),
            Do::ToggleLabel => self.label_prompt(side, true, command),
            Do::ClearLabels => self.clear_labels(side),
            Do::Labels => self.labels_list(),
            Do::Favorite(number) => self.favorite_label(side, number),
            Do::Bookmarks => self.bookmarks_command(side, command),
            Do::BookmarkFolder => self.bookmark_folder(side),
            Do::BookmarkItem => self.bookmark_item(side),
            Do::History => self.history_list(side),
            Do::ForgetPlace => {
                let place = self.panel(side).place().clone();
                self.forget_place(&place)
            }
            Do::SaveView => self.save_view(side),
            Do::ForgetView => self.forget_view(side),
            Do::Note => self.note_prompt(side, command),
            Do::Field => self.field_prompt(side, command),
        })
    }

    /// The purpose of the dialog on top, when it is one of these lists'.
    pub(super) fn organize_top(&self) -> Option<Wants> {
        match self.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Organize(wants)) => Some((**wants).clone()),
            _ => None,
        }
    }

    /// Opens a dialog of these lists.
    pub(super) fn open_organize(&mut self, dialog: Dialog, wants: Wants) -> Vec<Effect> {
        self.open_dialog(dialog, Purpose::Organize(Box::new(wants)))
    }

    /// A key in one of these lists before the dialog's own keys: an alias
    /// letter jumps in the Bookmarks list; the keys that trash a file
    /// remove the focused bookmark, forget the focused place, or take the
    /// label off the focused item; the key that renames a file edits the
    /// focused bookmark. `None` when the key is the dialog's.
    pub(super) fn organize_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let wants = self.organize_top()?;
        let in_choose = matches!(self.dialogs.last(), Some((_, Dialog::Choose { .. })));
        if !in_choose {
            return None;
        }
        let file_command = match self.keymap.lookup(&[Layer::Panel], key) {
            Lookup::Command(id) => Some(id),
            Lookup::NoMatch => None,
        };
        let removes = file_command
            .as_ref()
            .is_some_and(|id| *id == ids::TRASH || *id == ids::DELETE_PERMANENTLY);
        let renames = file_command.as_ref().is_some_and(|id| *id == ids::RENAME);
        let row = self.top_row_id();
        match wants {
            Wants::Bookmarks { side } => {
                if let Some(letter) = typed(key)
                    && !key.mods.ctrl
                    && !key.mods.alt
                    && let Some(place) = self
                        .organize
                        .bookmarks
                        .by_alias(letter)
                        .map(|b| b.place.clone())
                {
                    return Some(self.go_to_bookmark(side, &place));
                }
                let place = row
                    .filter(|r| r != crate::bookmarks::THIS_FOLDER_ROW)
                    .and_then(|r| Place::parse(&r).ok())?;
                if removes {
                    return Some(self.ask_remove_bookmark(side, place));
                }
                if renames {
                    return Some(self.bookmark_form(side, place, None));
                }
                // The copy and move keys copy or move the panel's marked
                // items, or its focused one, into the bookmark (W3a).
                let kind = match file_command {
                    Some(id) if id == ids::COPY => Some(diroxide_core::JobKind::Copy),
                    Some(id) if id == ids::MOVE => Some(diroxide_core::JobKind::Move),
                    _ => None,
                };
                if let Some(kind) = kind {
                    return Some(self.send_to_bookmark(side, kind, place));
                }
                None
            }
            Wants::History { .. } if removes => {
                let place = row.and_then(|r| Place::parse(&r).ok())?;
                Some(self.forget_place(&place))
            }
            // The review's one-key answers (lane AP).
            Wants::Photos(wants) => self.photo_key(&wants, key),
            // Space changes a copy's mark in the duplicates list (lane AS).
            Wants::Dups(wants) => self.dups_key(&wants, key),
            // Space leaves a synchronize plan row out or takes it back.
            Wants::Sync(wants) => self.sync_key(&wants, key),
            _ => None,
        }
    }

    /// The focused row's id of the dialog on top.
    pub(super) fn top_row_id(&self) -> Option<String> {
        match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => list.focused().map(|r| r.id.clone()),
            _ => None,
        }
    }

    /// Enter in one of these dialogs.
    pub(super) fn organize_accept(
        &mut self,
        wants: Wants,
        row: Option<String>,
        text: String,
        in_list: bool,
    ) -> Vec<Effect> {
        match wants {
            Wants::Label {
                side,
                places,
                one,
                toggle,
            } => self.label_chosen(side, places, one, toggle, row, &text, in_list),
            // The label's items as search results (W3a).
            Wants::Labels => match row {
                Some(label) => self.labeled_results(&label),
                None => Vec::new(),
            },
            Wants::Bookmarks { side } => match row {
                Some(row) if row == crate::bookmarks::THIS_FOLDER_ROW => self.this_folder_row(side),
                Some(row) => match Place::parse(&row) {
                    Ok(place) => self.go_to_bookmark(side, &place),
                    Err(_) => Vec::new(),
                },
                None => Vec::new(),
            },
            Wants::BookmarkForm { side, place } => match row {
                Some(row) => self.bookmark_form_row(side, place, &row),
                None => Vec::new(),
            },
            Wants::BookmarkText { side, place, field } => {
                self.bookmark_text_typed(side, place, field, &text)
            }
            Wants::History { side } => match row.and_then(|r| Place::parse(&r).ok()) {
                Some(place) => {
                    let mut effects = self.close_all();
                    effects.extend(self.open(side, place, None));
                    effects
                }
                None => Vec::new(),
            },
            Wants::RemoveBookmark { .. } => self.dialog_escape(),
            Wants::Types(wants) => self.type_lists_accept(wants, row),
            Wants::Convert(wants) => self.convert_accept(wants, row),
            Wants::Watched(wants) => self.watched_accept(wants, row),
            Wants::Note { side, places, one } => self.note_typed(side, places, one, &text),
            Wants::Field { side, places, one } => self.field_typed(side, places, one, &text),
            Wants::ReadColumns { .. } => self.dialog_escape(),
            Wants::Meta(wants) => self.meta_accept(wants, row, &text),
            Wants::Crypt(wants) => self.crypt_accept(wants, &text),
            Wants::Access(wants) => self.access_accept(wants, row, &text),
            Wants::Photos(wants) => self.photo_accept(wants, row, &text),
            Wants::Tabs(wants) => self.tabs_accept(wants, row, &text),
            // Reserved for alpha 5's app lanes (lane P5).
            Wants::Rename(wants) => self.rename_accept(wants, row, &text),
            Wants::Sync(wants) => self.sync_accept(wants, row, &text),
            Wants::Dups(wants) => self.dups_accept(wants, row, &text),
            Wants::Lists(wants) => self.lists_accept(wants, row, &text),
        }
    }

    /// Yes to one of these questions.
    pub(super) fn organize_yes(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::RemoveBookmark { side, place } => self.remove_bookmark(side, &place),
            Wants::Convert(wants) => self.convert_yes(wants),
            Wants::Watched(wants) => self.watched_yes(wants),
            Wants::ReadColumns { side, fill } => self.start_fill(side, fill),
            Wants::Meta(wants) => self.meta_yes(wants),
            Wants::Crypt(wants) => self.crypt_yes(wants),
            Wants::Photos(wants) => self.photo_yes(wants),
            Wants::Tabs(wants) => self.tabs_yes(wants),
            // Reserved for alpha 5's app lanes (lane P5).
            Wants::Rename(wants) => self.rename_yes(wants),
            Wants::Sync(wants) => self.sync_yes(wants),
            Wants::Dups(wants) => self.dups_yes(wants),
            Wants::Lists(wants) => self.lists_yes(wants),
            _ => Vec::new(),
        }
    }

    /// Escape on one of these dialogs, after it closed: a question says
    /// nothing changed; a bookmark's settings go back to the list.
    pub(super) fn organize_escape(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::RemoveBookmark { .. } => {
                vec![self.say(Message::info("Nothing removed."))]
            }
            Wants::ReadColumns { .. } => {
                vec![self.say(Message::info("Nothing read; the order stays."))]
            }
            Wants::Meta(wants) => self.meta_escape(wants),
            Wants::Crypt(wants) => self.crypt_escape(wants),
            Wants::Access(wants) => self.access_escape(wants),
            Wants::Photos(wants) => self.photo_escape(wants),
            Wants::BookmarkForm { side, .. } => self.refresh_bookmarks_list(side),
            Wants::BookmarkText { side, place, .. } => self.bookmark_form(side, place, None),
            Wants::Convert(wants) => self.convert_escape(wants),
            Wants::Watched(wants) => self.watched_escape(wants),
            Wants::Tabs(wants) => self.tabs_escape(wants),
            // Reserved for alpha 5's app lanes (lane P5).
            Wants::Rename(wants) => self.rename_escape(wants),
            Wants::Sync(wants) => self.sync_escape(wants),
            Wants::Dups(wants) => self.dups_escape(wants),
            Wants::Lists(wants) => self.lists_escape(wants),
            _ => Vec::new(),
        }
    }

    /// The candidates of one of these fields for the text typed so far;
    /// `None` when the field has no list of its own.
    pub(super) fn organize_candidates(&self, wants: &Wants, text: &str) -> Option<ListModel> {
        match wants {
            Wants::Label { places, .. } => Some(self.label_choices(places, text)),
            _ => None,
        }
    }

    /// Replaces the list of the open dialog of these lists that `pick`
    /// chooses, where it is in the stack, keeping its focus by row id.
    pub(super) fn replace_open_list(
        &mut self,
        pick: impl Fn(&Wants) -> bool,
        list: ListModel,
    ) -> bool {
        let index = self.opens.iter().rposition(|open| match &open.purpose {
            Purpose::Organize(wants) => pick(wants),
            _ => false,
        });
        let Some(index) = index else {
            return false;
        };
        if let Some((_, Dialog::Choose { list: open, .. })) = self.dialogs.get_mut(index) {
            let focus = open.focused().map(|r| r.id.clone());
            let position = open.focus_index();
            *open = list;
            let kept = focus.is_some_and(|id| open.focus_id(&id));
            if !kept && let Some(position) = position {
                let last = open.len().saturating_sub(1);
                open.move_focus(diroxide_core::Move::To(position.min(last)));
            }
            return true;
        }
        false
    }

    /// Saves the labels through the runtime.
    pub(super) fn save_labels(&mut self) {
        let store = self.organize.labels.clone();
        self.request(crate::request::Request::SaveLabels(Box::new(store)));
    }

    /// Saves the bookmarks through the runtime.
    pub(super) fn save_bookmarks(&mut self) {
        let bookmarks = self.organize.bookmarks.clone();
        self.request(crate::request::Request::SaveBookmarks(Box::new(bookmarks)));
    }

    /// Saves the folder views through the runtime.
    pub(super) fn save_views(&mut self) {
        let views = self.organize.views.clone();
        self.request(crate::request::Request::SaveViews(Box::new(views)));
    }

    /// The labels' names, for the filter language's lints and completion
    /// (`LintContext` and `CompletionContext`): the wiring lane passes
    /// these.
    pub fn label_names(&self) -> Vec<String> {
        self.organize.labels.names()
    }

    /// The labels and the labeled places.
    pub fn labels(&self) -> &LabelStore {
        &self.organize.labels
    }

    /// The bookmarks.
    pub fn bookmarks(&self) -> &Bookmarks {
        &self.organize.bookmarks
    }

    /// The folders' own views.
    pub fn folder_views(&self) -> &FolderViews {
        &self.organize.views
    }

    /// The filter language's `bookmarked` fact (and the entry's whole
    /// path) for a panel's entries; the labels are on the entries
    /// themselves. The wiring lane joins it with the panel's own facts.
    pub fn bookmark_lookup(&self, side: Side) -> crate::bookmarks::BookmarkLookup<'_> {
        crate::bookmarks::BookmarkLookup::new(self.panel(side).place(), &self.organize.bookmarks)
    }
}
