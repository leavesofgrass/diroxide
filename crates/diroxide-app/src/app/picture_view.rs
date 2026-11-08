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

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::names::display_name;
use diroxide_core::{Command, Effect, Entry, EntryKind, KeyColumn, Message, Place, Redraw};
use diroxide_vfs::{CancelToken, RunLimits, ToolRequest, ToolRunError, ToolRunner};

use super::App;
use crate::commands::ids;

/// Said in a terminal where a picture would show: how to open the window
/// (39 cells; lane VP, never a refusal).
pub const IN_THE_WINDOW: &str = "Pictures show in the window: diro --gui";

/// Said when Show picture runs in a terminal: the same words.
pub const NOT_IN_TERMINAL: &str = IN_THE_WINDOW;

/// Said when neither the viewer, the preview nor the panel shows a file.
pub const NOTHING_SHOWN: &str = "Show picture needs a picture or a PDF";

/// The group a picture's type is in.
const PICTURES: &str = "images";

/// The tools registry's id for ImageMagick.
pub const MAGICK: &str = "magick";

/// The tools registry's id for pdftoppm, which draws a PDF's pages.
pub const PDFTOPPM: &str = "pdftoppm";

/// The longest side, in pixels, a PDF page is drawn at.
pub const PAGE_SIDE: u32 = 2048;

/// Said when a picture needs ImageMagick and it is not found.
pub const NEEDS_MAGICK: &str = "Picture: not shown, needs ImageMagick";

/// Said when a PDF's pages need pdftoppm and it is not found.
pub const NEEDS_PDFTOPPM: &str = "Page pictures need pdftoppm";

/// How long a helper may draw one picture.
const HELPER_TIME: Duration = Duration::from_secs(10);

/// Where the picture to show is shown now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureSource {
    /// The viewer.
    Viewer,
    /// The quick view's preview region.
    Preview,
    /// The panel's focused row: F3 or Show picture with nothing open
    /// (lane VP).
    Panel,
}

/// A PDF page asked for as a picture (lane VP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageAsk {
    /// The page, 1-based.
    pub page: u32,
    /// The pages in all, when the PDF's properties said.
    pub pages: Option<u32>,
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
    /// For a PDF, the page drawn through pdftoppm (lane VP).
    pub page: Option<PageAsk>,
}

/// The line that says what shows a picture: "Enter shows the picture."
pub fn hint_words(key: &str) -> String {
    format!("{key} shows the picture.")
}

/// The line that says what shows a PDF's pages: "Enter shows the pages
/// as pictures." (lane VP).
pub fn page_hint_words(key: &str) -> String {
    format!("{key} shows the pages as pictures.")
}

/// A page said in words: "Page 1 of 3", or "Page 1" when the count is
/// not known.
pub fn page_words(page: u32, pages: Option<u32>) -> String {
    match pages {
        Some(pages) => format!("Page {page} of {pages}"),
        None => format!("Page {page}"),
    }
}

/// The helper programs the window's picture view runs (lane VP):
/// ImageMagick for the formats the built-in decoder cannot read, and
/// pdftoppm for a PDF's pages, both through the app's tools, each with
/// its install line when it is not found. A failure is the words to say.
#[derive(Clone)]
pub struct PictureHelpers {
    tools: Arc<dyn ToolRunner>,
    magick_install: Option<String>,
    pdftoppm_install: Option<String>,
}

impl std::fmt::Debug for PictureHelpers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureHelpers").finish_non_exhaustive()
    }
}

impl PictureHelpers {
    /// Helpers over a tools runner, with the install lines to say.
    pub fn new(
        tools: Arc<dyn ToolRunner>,
        magick_install: Option<String>,
        pdftoppm_install: Option<String>,
    ) -> Self {
        Self {
            tools,
            magick_install,
            pdftoppm_install,
        }
    }

    /// Whether ImageMagick is found.
    pub fn has_magick(&self) -> bool {
        self.tools.has(MAGICK)
    }

    /// The lines said when ImageMagick is needed and not found: the
    /// refusal in words, then the install line.
    pub fn magick_missing(&self) -> Vec<String> {
        std::iter::once(NEEDS_MAGICK.to_owned())
            .chain(self.magick_install.clone())
            .collect()
    }

    /// The lines said when pdftoppm is not found.
    pub fn pdftoppm_missing(&self) -> Vec<String> {
        std::iter::once(NEEDS_PDFTOPPM.to_owned())
            .chain(self.pdftoppm_install.clone())
            .collect()
    }

    /// A picture the decoder cannot read, as PNG through ImageMagick: its
    /// first frame, turned upright, at most `max_bytes` of output in ten
    /// seconds. Only a file on a local disk.
    pub fn magick_png(&self, path: &Path, max_bytes: u64) -> Result<Vec<u8>, Vec<String>> {
        if !self.has_magick() {
            return Err(self.magick_missing());
        }
        let mut input = path.as_os_str().to_owned();
        input.push("[0]");
        let args: Vec<std::ffi::OsString> = vec![
            "-limit".into(),
            "memory".into(),
            "256MiB".into(),
            input,
            "-auto-orient".into(),
            "png:-".into(),
        ];
        let request = ToolRequest::new(MAGICK, args)
            .with_limits(RunLimits::new(HELPER_TIME, usize_cap(max_bytes)));
        self.run_png(&request, "ImageMagick")
            .map_err(|line| vec![line])
    }

    /// A PDF's page as PNG through pdftoppm, its longest side
    /// [`PAGE_SIDE`] pixels, at most `max_bytes` in ten seconds.
    pub fn page_png(&self, path: &Path, page: u32, max_bytes: u64) -> Result<Vec<u8>, Vec<String>> {
        if !self.tools.has(PDFTOPPM) {
            return Err(self.pdftoppm_missing());
        }
        let page = page.max(1).to_string();
        let args: Vec<std::ffi::OsString> = vec![
            "-f".into(),
            page.clone().into(),
            "-l".into(),
            page.into(),
            "-png".into(),
            "-singlefile".into(),
            "-scale-to".into(),
            PAGE_SIDE.to_string().into(),
            path.as_os_str().to_owned(),
        ];
        let request = ToolRequest::new(PDFTOPPM, args)
            .with_limits(RunLimits::new(HELPER_TIME, usize_cap(max_bytes)));
        self.run_png(&request, "pdftoppm")
            .map_err(|line| vec![line])
    }

    fn run_png(&self, request: &ToolRequest, name: &str) -> Result<Vec<u8>, String> {
        match self.tools.run(request, &CancelToken::new()) {
            Ok(output) if output.succeeded() && output.stdout.starts_with(b"\x89PNG") => {
                Ok(output.stdout)
            }
            Ok(_) => Err(format!("Picture: not shown, {name} could not draw it")),
            Err(ToolRunError::NotInstalled { .. }) => Err(format!("Picture: {name} not found")),
            Err(error) => Err(format!("Picture: not shown, {error}")),
        }
    }
}

fn usize_cap(bytes: u64) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
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

    /// The helper programs the window's picture view runs (lane VP):
    /// ImageMagick and pdftoppm through the app's tools, with their
    /// install lines.
    pub fn picture_helpers(&self) -> PictureHelpers {
        let tools = self.tools_registry();
        let install = |id: &str| {
            tools.next_step(id).map(|step| match step {
                diroxide_tools::NextStep::Command(command) => format!("Install: {command}"),
                diroxide_tools::NextStep::Words(words) => words,
            })
        };
        PictureHelpers::new(self.tool_runner().0, install(MAGICK), install(PDFTOPPM))
    }

    /// Shows the picture the viewer or the preview shows now, or the
    /// panel's focused picture or PDF with nothing open (lane VP), or
    /// says why not.
    pub(super) fn show_picture(&mut self) -> Vec<Effect> {
        if self.keymap.column() == KeyColumn::Terminal {
            return vec![self.say(Message::info(NOT_IN_TERMINAL))];
        }
        let Some((place, name, from)) = self.picture_shown() else {
            return vec![self.say(Message::info(NOTHING_SHOWN))];
        };
        if self.is_pdf(&place, &name) {
            return self.show_page(place, name, from);
        }
        if !self.is_picture(&place, &name) {
            return vec![self.say(Message::info(not_a_picture(&name)))];
        }
        self.picture_ask = Some(PictureAsk {
            place,
            name,
            from,
            page: None,
        });
        vec![Effect::Redraw(Redraw::All)]
    }

    /// A PDF's page in the picture view (lane VP): the page the viewer's
    /// cursor is on when the viewer shows its words, else the first.
    fn show_page(&mut self, place: Place, name: String, from: PictureSource) -> Vec<Effect> {
        let (page, pages) = match (from, self.viewer()) {
            (PictureSource::Viewer, Some(viewer)) => (viewer.current_page(), viewer.pages()),
            _ => (1, None),
        };
        self.picture_ask = Some(PictureAsk {
            place,
            name,
            from,
            page: Some(PageAsk { page, pages }),
        });
        vec![Effect::Redraw(Redraw::All)]
    }

    /// F3 in the window on a picture with nothing open (lane VP, the
    /// owner's finding): the picture view at once, no hex on the way;
    /// `None` in the terminal and for anything else.
    pub(super) fn show_focused_picture(&mut self) -> Option<Vec<Effect>> {
        if self.keymap.column() == KeyColumn::Terminal || !self.dialogs.is_empty() {
            return None;
        }
        let panel = self.panel(self.active);
        let place = panel.focused_place()?;
        let name = panel.focused_entry()?.display_name().into_owned();
        if !self.is_picture(&place, &name) {
            return None;
        }
        self.picture_ask = Some(PictureAsk {
            place,
            name,
            from: PictureSource::Panel,
            page: None,
        });
        Some(vec![Effect::Redraw(Redraw::All)])
    }

    /// The file the preview or the viewer shows now, else the panel's
    /// focused file: its place, its name as shown, and where.
    fn picture_shown(&self) -> Option<(Place, String, PictureSource)> {
        if self.in_preview() {
            let shown = self.quick_view()?.shown()?;
            return Some((
                shown.place.clone(),
                shown.name.clone(),
                PictureSource::Preview,
            ));
        }
        if let Some(viewer) = self.viewer() {
            return Some((
                viewer.place().clone(),
                display_name(viewer.name()).into_owned(),
                PictureSource::Viewer,
            ));
        }
        let panel = self.panel(self.active);
        let entry = panel.focused_entry().filter(|e| !e.is_folder_like())?;
        Some((
            panel.focused_place()?,
            entry.display_name().into_owned(),
            PictureSource::Panel,
        ))
    }

    /// Whether a file is a PDF by its type (lane VP).
    pub(super) fn is_pdf(&self, place: &Place, name: &str) -> bool {
        let entry = self.listed_entry_of(place, name);
        if entry.is_folder_like() {
            return false;
        }
        let id = self
            .types
            .by_index(entry.type_index)
            .map(|t| t.id.clone())
            .or_else(|| self.types.detect(&entry.name, &[]).map(|d| d.type_id));
        id.as_deref() == Some("pdf")
    }

    /// The panel's entry for a place where a panel lists it, else one
    /// made from its name.
    fn listed_entry_of(&self, place: &Place, name: &str) -> Entry {
        let listed = [self.active, self.active.other()]
            .into_iter()
            .find_map(|side| {
                let panel = self.panel(side);
                (place.parent().as_ref() == Some(panel.place()))
                    .then(|| place.name().and_then(|n| panel.entry(n)).cloned())
                    .flatten()
            });
        listed.unwrap_or_else(|| Entry::new(place.name().unwrap_or(name), EntryKind::File))
    }

    /// Whether a file is a picture by its type: the panel's entry where a
    /// panel lists it (its checked type), else its name.
    pub(super) fn is_picture(&self, place: &Place, name: &str) -> bool {
        let entry = self.listed_entry_of(place, name);
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
        if key.is_empty() {
            return None;
        }
        if self.is_picture(place, name) {
            return Some(Message::info(hint_words(&key)));
        }
        // A PDF's words in the viewer: its pages show as pictures (lane
        // VP).
        let pdf_words = self
            .viewer()
            .is_some_and(|v| v.is_preview() && v.place() == place);
        (pdf_words && self.is_pdf(place, name)).then(|| Message::info(page_hint_words(&key)))
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
            page_hint_words("Enter"),
            page_words(12, Some(120)),
            NEEDS_MAGICK.to_owned(),
            NEEDS_PDFTOPPM.to_owned(),
        ] {
            assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
        }
        assert_eq!(hint_words("Enter"), "Enter shows the picture.");
        assert_eq!(not_a_picture("notes.txt"), "Not a picture: notes.txt");
    }
}
