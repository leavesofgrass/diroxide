//! Bookmarks in the app (plan commit 24): the Bookmarks list on its keys,
//! its first row by the hotlist rule ("Add this folder" or "Remove this
//! folder"), Enter going there, an alias letter jumping, the rename key
//! opening a bookmark's settings as a list (name, category, alias,
//! sticky, what it opens), the trash key removing it after a question
//! that starts on No; Bookmark this folder and Bookmark the focused item
//! from the palette and the Go menu; bookmarks first in the Go to place
//! list, kind first ("Bookmark, Reports").

use diroxide_core::{Command, Effect, ListModel, ListRow, Message, Place, Redraw, Side};

use super::App;
use super::organize::{BookmarkField, Wants};
use crate::bookmarks::{Bookmark, check_alias, check_text};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};

/// The rows of a bookmark's settings list, by id.
mod rows {
    pub(super) const NAME: &str = "name";
    pub(super) const CATEGORY: &str = "category";
    pub(super) const ALIAS: &str = "alias";
    pub(super) const STICKY: &str = "sticky";
    pub(super) const OPENS: &str = "opens";
    pub(super) const PLACE: &str = "place";
}

impl App {
    /// The Bookmarks command: with an `alias`, jumps to that bookmark;
    /// otherwise opens the list.
    pub(super) fn bookmarks_command(&mut self, side: Side, command: &Command) -> Vec<Effect> {
        if let Some(alias) = command.text("alias") {
            let found = alias
                .chars()
                .next()
                .and_then(|c| self.organize.bookmarks.by_alias(c))
                .map(|b| b.place.clone());
            return match found {
                Some(place) => self.go_to_bookmark(side, &place),
                None => {
                    let text = format!("No bookmark has the alias {alias}.");
                    vec![self.say(Message::info(text))]
                }
            };
        }
        self.bookmarks_list(side)
    }

    /// The Bookmarks list's help, from the live keys.
    fn bookmarks_help(&self) -> String {
        let mut parts = vec![
            "Enter goes there".to_owned(),
            "an alias letter jumps".to_owned(),
        ];
        let edit = self.key(&ids::RENAME);
        if !edit.is_empty() {
            parts.push(format!("{edit} edits"));
        }
        let remove = self.key(&ids::TRASH);
        if !remove.is_empty() {
            parts.push(format!("{remove} removes"));
        }
        parts.push("Escape closes".to_owned());
        format!("{}.", parts.join(", "))
    }

    fn bookmarks_model(&self, side: Side) -> ListModel {
        let mut list = self.organize.bookmarks.list(self.panel(side).place());
        list.set_help(self.bookmarks_help());
        list
    }

    /// The Bookmarks list for a panel.
    pub(super) fn bookmarks_list(&mut self, side: Side) -> Vec<Effect> {
        let list = self.bookmarks_model(side);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::BOOKMARKS),
            },
            Wants::Bookmarks { side },
        )
    }

    /// Rebuilds an open Bookmarks list after a change, its focus kept.
    pub(super) fn refresh_bookmarks_list(&mut self, side: Side) -> Vec<Effect> {
        let list = self.bookmarks_model(side);
        if self.replace_open_list(|w| matches!(w, Wants::Bookmarks { .. }), list) {
            vec![Effect::Redraw(Redraw::All)]
        } else {
            Vec::new()
        }
    }

    /// The first row: adds this folder, or asks to remove it.
    pub(super) fn this_folder_row(&mut self, side: Side) -> Vec<Effect> {
        let here = self.panel(side).place().clone();
        if self.organize.bookmarks.is_bookmarked(&here) {
            return self.ask_remove_bookmark(side, here);
        }
        let mut effects = self.close_all();
        effects.extend(self.add_bookmark(Bookmark::new(here, false)));
        effects
    }

    /// Adds a bookmark and says so: "Bookmarked Reports, in work".
    fn add_bookmark(&mut self, bookmark: Bookmark) -> Vec<Effect> {
        let text = match self.organize.bookmarks.add(bookmark) {
            Ok(added) => added.added_sentence(),
            Err(why) => return vec![self.say(Message::info(why))],
        };
        self.save_bookmarks();
        vec![self.say(Message::info(text))]
    }

    /// Bookmark this folder.
    pub(super) fn bookmark_folder(&mut self, side: Side) -> Vec<Effect> {
        let here = self.panel(side).place().clone();
        self.add_bookmark(Bookmark::new(here, false))
    }

    /// Bookmark the focused item: going there opens its folder with it
    /// focused.
    pub(super) fn bookmark_item(&mut self, side: Side) -> Vec<Effect> {
        let Some(place) = self.panel(side).focused_place() else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        self.add_bookmark(Bookmark::new(place, true))
    }

    /// Goes to a bookmark: its use is counted and saved; an item bookmark
    /// opens its folder with the item focused. The listing says where the
    /// panel is, so nothing more is said.
    pub(super) fn go_to_bookmark(&mut self, side: Side, place: &Place) -> Vec<Effect> {
        let Some(bookmark) = self.organize.bookmarks.find(place).cloned() else {
            return vec![self.say(Message::info("That bookmark is gone."))];
        };
        let now = self.now();
        self.organize.bookmarks.used(place, now);
        self.save_bookmarks();
        let mut effects = self.close_all();
        match (
            bookmark.item,
            bookmark.place.parent(),
            bookmark.place.name(),
        ) {
            (true, Some(parent), Some(name)) => {
                let name = name.to_owned();
                effects.extend(self.open(side, parent, Some(name)));
            }
            _ => effects.extend(self.open(side, bookmark.place, None)),
        }
        effects
    }

    /// The question before a bookmark is removed, starting on No.
    pub(super) fn ask_remove_bookmark(&mut self, side: Side, place: Place) -> Vec<Effect> {
        let Some(bookmark) = self.organize.bookmarks.find(&place) else {
            return vec![self.say(Message::info("That bookmark is gone."))];
        };
        let question = with_default_no(Message::question(&format!(
            "Remove bookmark {}",
            bookmark.name
        )));
        let listing = vec![diroxide_core::names::spell_out(&place.to_string()).into_owned()];
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::BOOKMARKS),
            },
            Wants::RemoveBookmark { side, place },
        )
    }

    /// Yes to the question: the bookmark goes, the list beneath updates.
    pub(super) fn remove_bookmark(&mut self, side: Side, place: &Place) -> Vec<Effect> {
        let Some(removed) = self.organize.bookmarks.remove(place) else {
            return vec![self.say(Message::info("That bookmark is gone."))];
        };
        self.save_bookmarks();
        let mut effects = self.refresh_bookmarks_list(side);
        let text = format!("Removed bookmark {}", removed.name);
        effects.push(self.say(Message::info(text)));
        effects
    }

    /// A bookmark's settings, as a list: each row a labeled setting with
    /// its value, meaning first; Enter changes it.
    pub(super) fn bookmark_form(
        &mut self,
        side: Side,
        place: Place,
        focus: Option<&str>,
    ) -> Vec<Effect> {
        let Some(bookmark) = self.organize.bookmarks.find(&place).cloned() else {
            return vec![self.say(Message::info("That bookmark is gone."))];
        };
        let alias = match bookmark.alias {
            Some(alias) => format!("Alias: {alias}"),
            None => "Alias: none".to_owned(),
        };
        let category = if bookmark.category.is_empty() {
            "Category: none".to_owned()
        } else {
            format!("Category: {}", bookmark.category)
        };
        let sticky = if bookmark.sticky {
            "Sticky: on, kept at the top"
        } else {
            "Sticky: off"
        };
        let opens = if bookmark.item {
            "Opens: its folder, on the item"
        } else {
            "Opens: the place itself"
        };
        let rows = vec![
            ListRow::new(rows::NAME, format!("Name: {}", bookmark.name)),
            ListRow::new(rows::CATEGORY, category),
            ListRow::new(rows::ALIAS, alias),
            ListRow::new(rows::STICKY, sticky),
            ListRow::new(rows::OPENS, opens),
            ListRow::new(
                rows::PLACE,
                format!(
                    "Place: {}",
                    diroxide_core::names::display_name(&bookmark.place.to_string())
                ),
            ),
        ];
        let mut list = ListModel::new(format!("Bookmark {}", bookmark.name)).with_rows(rows);
        list.set_help("Enter changes the setting, Escape goes back to the bookmarks.");
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::BOOKMARKS),
            },
            Wants::BookmarkForm { side, place },
        )
    }

    /// Enter on a bookmark setting.
    pub(super) fn bookmark_form_row(&mut self, side: Side, place: Place, row: &str) -> Vec<Effect> {
        let Some(bookmark) = self.organize.bookmarks.find(&place).cloned() else {
            return vec![self.say(Message::info("That bookmark is gone."))];
        };
        let field = |label: &str, text: String, source: CompletionSource| {
            EntryField::new(label, source).with_text(text)
        };
        let (entry, which) = match row {
            rows::NAME => (
                field(
                    "Bookmark name",
                    bookmark.name.clone(),
                    CompletionSource::None,
                ),
                BookmarkField::Name,
            ),
            rows::CATEGORY => (
                field(
                    "Category",
                    bookmark.category.clone(),
                    CompletionSource::Values(self.organize.bookmarks.categories()),
                ),
                BookmarkField::Category,
            ),
            rows::ALIAS => (
                field(
                    "Alias, one letter or digit",
                    bookmark.alias.map(String::from).unwrap_or_default(),
                    CompletionSource::None,
                ),
                BookmarkField::Alias,
            ),
            rows::STICKY | rows::OPENS => {
                let sticky = row == rows::STICKY;
                let edited = self.organize.bookmarks.edit(&place, |b| {
                    if sticky {
                        b.sticky = !b.sticky;
                    } else {
                        b.item = !b.item;
                    }
                });
                let text = match edited {
                    Ok(b) if sticky && b.sticky => "Sticky: on, kept at the top".to_owned(),
                    Ok(_) if sticky => "Sticky: off".to_owned(),
                    Ok(b) if b.item => "Opens: its folder, on the item".to_owned(),
                    Ok(_) => "Opens: the place itself".to_owned(),
                    Err(why) => why,
                };
                self.save_bookmarks();
                let mut effects = self.close_top();
                effects.extend(self.bookmark_form(side, place, Some(row)));
                effects.push(self.say(Message::info(text)));
                return effects;
            }
            _ => {
                let text = format!(
                    "Place: {}",
                    diroxide_core::names::display_name(&bookmark.place.to_string())
                );
                return vec![self.say(Message::info(text))];
            }
        };
        let mut effects = self.close_top();
        effects.extend(self.open_organize(
            Dialog::edit(entry),
            Wants::BookmarkText {
                side,
                place,
                field: which,
            },
        ));
        effects
    }

    /// Enter in a bookmark's text setting: checked, saved, and said; the
    /// settings list comes back on the same row. A mistake is said and
    /// the field stays.
    pub(super) fn bookmark_text_typed(
        &mut self,
        side: Side,
        place: Place,
        field: BookmarkField,
        text: &str,
    ) -> Vec<Effect> {
        let edited = match field {
            BookmarkField::Name => check_text(text, "name", false).and_then(|name| {
                self.organize
                    .bookmarks
                    .edit(&place, |b| b.name = name)
                    .map(|b| format!("Name: {}", b.name))
            }),
            BookmarkField::Category => check_text(text, "category", true).and_then(|category| {
                self.organize
                    .bookmarks
                    .edit(&place, |b| b.category = category)
                    .map(|b| {
                        if b.category.is_empty() {
                            "Category: none".to_owned()
                        } else {
                            format!("Category: {}", b.category)
                        }
                    })
            }),
            BookmarkField::Alias => check_alias(text).and_then(|alias| {
                self.organize
                    .bookmarks
                    .edit(&place, |b| b.alias = alias)
                    .map(|b| match b.alias {
                        Some(alias) => format!("Alias: {alias}"),
                        None => "Alias: none".to_owned(),
                    })
            }),
        };
        let said = match edited {
            Ok(said) => said,
            Err(why) => return vec![self.say(Message::error(why))],
        };
        self.save_bookmarks();
        let row = match field {
            BookmarkField::Name => rows::NAME,
            BookmarkField::Category => rows::CATEGORY,
            BookmarkField::Alias => rows::ALIAS,
        };
        let mut effects = self.close_top();
        effects.extend(self.bookmark_form(side, place, Some(row)));
        effects.push(self.say(Message::info(said)));
        effects
    }

    /// The bookmarks as Go to place candidates, first: each its place,
    /// its name after it.
    pub(super) fn bookmark_place_candidates(&self) -> Vec<crate::entry::Candidate> {
        self.organize
            .bookmarks
            .ordered()
            .into_iter()
            .map(|b| {
                crate::entry::Candidate::new(b.place.to_string(), format!("bookmark {}", b.name))
            })
            .collect()
    }

    /// In the Go to place list, a bookmarked place's row says its kind
    /// first: "Bookmark, Reports, in work", its whole place last.
    pub(super) fn bookmark_rows_first(&self, list: &mut ListModel) {
        let rows: Vec<ListRow> = list
            .rows()
            .iter()
            .map(|row| {
                let bookmark = Place::parse(&row.id)
                    .ok()
                    .and_then(|place| self.organize.bookmarks.find(&place));
                match bookmark {
                    Some(b) => ListRow::new(row.id.clone(), format!("Bookmark, {}", b.name))
                        .with_details([
                            b.where_words(),
                            diroxide_core::names::display_name(&b.place.to_string()).into_owned(),
                        ]),
                    None => row.clone(),
                }
            })
            .collect();
        let focus = list.focused().map(|r| r.id.clone());
        list.set_rows(rows);
        if let Some(focus) = focus {
            list.focus_id(&focus);
        }
    }

    /// A place chosen from the Go to place list: its bookmark's use is
    /// counted, when it has one.
    pub(super) fn bookmark_chosen(&mut self, place: &Place) {
        let now = self.now();
        if self.organize.bookmarks.used(place, now) {
            self.save_bookmarks();
        }
    }
}
