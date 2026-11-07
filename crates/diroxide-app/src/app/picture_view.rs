//! Show picture (alpha 4, lane AE; G5b's request): the window's picture
//! view as the app's command, `view.picture`, so its key is the keymap's
//! and the Keys list, F1 help, the View menu, the palette and the
//! keyboard page all know it.
//!
//! - In the GUI column Enter runs it in the viewer; in the quick view's
//!   preview Enter is Open link, and on a picture, whose lines carry no
//!   link, it shows the picture instead. The palette and the View menu
//!   reach it anywhere.
//! - "Is a picture" is the file's type, through the type registry: a type
//!   in the Images group (a PNG, a JPEG, a camera's raw file). The window
//!   says in words what it cannot open.
//! - The app keeps the ask; the window takes it after the event
//!   ([`App::take_picture_ask`]), reads the picture on a thread of its
//!   own and shows it, as it does full screen. In a terminal the command
//!   says the window has the picture view.
//! - As the viewer opens a picture, or the focus enters a preview that
//!   shows one, the app says once what shows it: "Enter shows the
//!   picture." (the key from the live keymap; nothing when it has none).

use diroxide_core::names::display_name;
use diroxide_core::{Command, Effect, Entry, EntryKind, KeyColumn, Message, Place, Redraw};

use super::App;
use crate::commands::ids;

/// Said when Show picture runs in a terminal.
pub const NOT_IN_TERMINAL: &str = "Show picture works only in the window.";

/// Said when neither the viewer nor the preview shows a file.
pub const NOTHING_SHOWN: &str = "Show picture needs the viewer or preview";

/// The group a picture's type is in.
const PICTURES: &str = "images";

/// Where the picture to show is shown now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureSource {
    /// The viewer.
    Viewer,
    /// The quick view's preview region.
    Preview,
}

/// A picture the window is asked to show: Show picture ran on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureAsk {
    /// The file.
    pub place: Place,
    /// Its name as shown, hidden characters spelled out.
    pub name: String,
    /// Where it is shown, which the picture view goes back to.
    pub from: PictureSource,
}

/// The line that says what shows a picture: "Enter shows the picture."
pub fn hint_words(key: &str) -> String {
    format!("{key} shows the picture.")
}

/// The line when the file shown is not a picture: "Not a picture:
/// notes.txt".
pub fn not_a_picture(name: &str) -> String {
    format!("Not a picture: {name}")
}

impl App {
    /// The picture Show picture asked the window to show, taken: the
    /// window reads it after the event and opens its picture view.
    pub fn take_picture_ask(&mut self) -> Option<PictureAsk> {
        self.picture_ask.take()
    }

    /// Show picture; `None` for any other command.
    pub(super) fn picture_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::VIEW_PICTURE {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.show_picture())
    }

    /// Shows the picture the viewer or the preview shows now, or says
    /// why not.
    pub(super) fn show_picture(&mut self) -> Vec<Effect> {
        if self.keymap.column() == KeyColumn::Terminal {
            return vec![self.say(Message::info(NOT_IN_TERMINAL))];
        }
        let Some((place, name, from)) = self.picture_shown() else {
            return vec![self.say(Message::info(NOTHING_SHOWN))];
        };
        if !self.is_picture(&place, &name) {
            return vec![self.say(Message::info(not_a_picture(&name)))];
        }
        self.picture_ask = Some(PictureAsk { place, name, from });
        vec![Effect::Redraw(Redraw::All)]
    }

    /// The file the preview or the viewer shows now: its place, its name
    /// as shown, and where.
    fn picture_shown(&self) -> Option<(Place, String, PictureSource)> {
        if self.in_preview() {
            let shown = self.quick_view()?.shown()?;
            return Some((
                shown.place.clone(),
                shown.name.clone(),
                PictureSource::Preview,
            ));
        }
        let viewer = self.viewer()?;
        Some((
            viewer.place().clone(),
            display_name(viewer.name()).into_owned(),
            PictureSource::Viewer,
        ))
    }

    /// Whether a file is a picture by its type: the panel's entry where a
    /// panel lists it (its checked type), else its name.
    pub(super) fn is_picture(&self, place: &Place, name: &str) -> bool {
        let listed = [self.active, self.active.other()]
            .into_iter()
            .find_map(|side| {
                let panel = self.panel(side);
                (place.parent().as_ref() == Some(panel.place()))
                    .then(|| place.name().and_then(|n| panel.entry(n)).cloned())
                    .flatten()
            });
        let entry =
            listed.unwrap_or_else(|| Entry::new(place.name().unwrap_or(name), EntryKind::File));
        if entry.is_folder_like() {
            return false;
        }
        matches!(
            self.types.group_for(&entry, self.clock.as_ref()),
            Ok(Some(group)) if group.id == PICTURES
        )
    }

    /// The line said as a picture comes into the viewer or the preview:
    /// "Enter shows the picture.", in the window only and when Show
    /// picture has a key.
    pub(super) fn picture_hint(&self, place: &Place) -> Option<Message> {
        if self.keymap.column() == KeyColumn::Terminal {
            return None;
        }
        let key = self.key(&ids::VIEW_PICTURE);
        let name = place.name().unwrap_or_default();
        (!key.is_empty() && self.is_picture(place, name)).then(|| Message::info(hint_words(&key)))
    }

    /// The hint for the file the preview shows, as the focus enters it.
    pub(super) fn preview_picture_hint(&self) -> Option<Message> {
        let shown = self.quick_view()?.shown()?;
        self.picture_hint(&shown.place)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    #[test]
    fn the_lines_fit_a_braille_line_with_their_meaning_first() {
        for line in [
            NOT_IN_TERMINAL.to_owned(),
            NOTHING_SHOWN.to_owned(),
            hint_words("Enter"),
            not_a_picture("notes.txt"),
        ] {
            assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
        }
        assert_eq!(hint_words("Enter"), "Enter shows the picture.");
        assert_eq!(not_a_picture("notes.txt"), "Not a picture: notes.txt");
    }
}
