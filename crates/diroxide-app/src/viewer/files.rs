//! Stepping through the folder's files without leaving the viewer, as
//! Directory Opus's viewer pane and Total Commander's lister do ("report-2.txt,
//! 2 of 9"), and handing the file back: show it in its folder, copy its
//! path or its name.

use diroxide_core::format::grouped;
use diroxide_core::names::display_name;
use diroxide_core::{Effect, Message, Place};
use diroxide_vfs::Registry;

use super::{RowPos, Viewer, ViewerSource};

/// A file the viewer can step to: its place, and its size when the
/// listing gave one (for the large-file policy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sibling {
    /// The file.
    pub place: Place,
    /// Its listed size.
    pub size: Option<u64>,
}

impl Sibling {
    /// A sibling with its listed size.
    pub fn new(place: Place, size: Option<u64>) -> Self {
        Self { place, size }
    }
}

impl Viewer {
    /// The files the viewer steps through, in the panel's order: the
    /// panel's files (folders left out), the viewed one among them.
    pub fn set_siblings(&mut self, siblings: Vec<Sibling>) {
        self.siblings = siblings;
    }

    /// Where the viewed file is among its siblings, 1-based, and how many
    /// there are: `(2, 9)`.
    pub fn file_position(&self) -> Option<(usize, usize)> {
        let at = self.siblings.iter().position(|s| s.place == self.place)?;
        Some((at + 1, self.siblings.len()))
    }

    /// The next file, opened in place: "report-2.txt, 2 of 9", then its
    /// encoding or policy line when it has one. At the last file: "Last
    /// file, 9 of 9".
    pub fn next_file(&mut self, registry: &Registry) -> Vec<Message> {
        self.step_file(registry, true)
    }

    /// The previous file: "report-1.txt, 1 of 9". At the first: "First
    /// file, 1 of 9".
    pub fn previous_file(&mut self, registry: &Registry) -> Vec<Message> {
        self.step_file(registry, false)
    }

    fn step_file(&mut self, registry: &Registry, forward: bool) -> Vec<Message> {
        let Some((at, of)) = self.file_position() else {
            return vec![Message::info("No other files to step to")];
        };
        let next = if forward { at + 1 } else { at.wrapping_sub(1) };
        if next == 0 || next > of {
            let which = if forward { "Last" } else { "First" };
            return vec![Message::info(format!(
                "{which} file, {} of {}",
                grouped(at as u64),
                grouped(of as u64)
            ))];
        }
        let sibling = self.siblings[next - 1].clone();
        let source = match ViewerSource::open(
            registry,
            &sibling.place,
            sibling.size,
            self.options.stream_cap(),
        ) {
            Ok(source) => source,
            Err(error) => {
                let name = sibling
                    .place
                    .name()
                    .map_or_else(String::new, |n| display_name(n).into_owned());
                return vec![error.message(&name)];
            }
        };
        self.show(sibling.place, source)
    }

    /// Shows another file in this viewer, keeping its settings (wrap, line
    /// numbers, the last search's words): "report-2.txt, 2 of 9".
    pub fn show(&mut self, place: Place, source: ViewerSource) -> Vec<Message> {
        self.name = place
            .name()
            .map_or_else(|| place.title(), |n| display_name(n).into_owned());
        self.title = crate::panel::place_title(&place);
        self.place = place;
        self.source = source;
        self.follow = false;
        self.cursor = RowPos::default();
        self.top = RowPos::default();
        self.hex_cursor = 0;
        self.hex_top = 0;
        if let Err(error) = self.load() {
            return vec![error.message(&self.name)];
        }
        let mut said = self.opening_lines();
        if let (Some(first), Some((at, of))) = (said.first_mut(), self.file_position()) {
            first.text = format!(
                "{}, {} of {}",
                self.name,
                grouped(at as u64),
                grouped(of as u64)
            );
        }
        said.extend(self.start_code());
        said
    }

    /// The viewed file, for the panel to show with the cursor on it, and
    /// what to say: "Documents, report.txt".
    pub fn show_in_folder(&self) -> (Place, Message) {
        let folder = self
            .place
            .parent()
            .map_or_else(String::new, |parent| crate::panel::place_leaf(&parent));
        let said = if folder.is_empty() {
            self.name.clone()
        } else {
            format!("{folder}, {}", self.name)
        };
        (self.place.clone(), Message::info(said))
    }

    /// The viewed file's full path on the clipboard
    /// ([`Effect::SetClipboardText`], P2b), and what to say: "Path copied:
    /// D:\docs\report.txt".
    pub fn copy_path(&self) -> (Effect, Message) {
        let path = self.place.to_string();
        let said = format!("Path copied: {}", display_name(&path));
        (Effect::SetClipboardText(path), Message::info(said))
    }

    /// The viewed file's name on the clipboard
    /// ([`Effect::SetClipboardText`], P2b), and what to say: "Name copied:
    /// report.txt".
    pub fn copy_name(&self) -> (Effect, Message) {
        let name = self.place.name().unwrap_or_default().to_owned();
        let said = format!("Name copied: {}", self.name);
        (Effect::SetClipboardText(name), Message::info(said))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, texts};
    use super::*;
    use crate::viewer::ViewerOptions;
    use diroxide_core::EntryKind;
    use diroxide_vfs::OpContext;

    fn siblings(registry: &Registry, folder: &Place) -> Vec<Sibling> {
        let provider = registry.resolve(folder).unwrap();
        let mut entries = provider.list(folder, &OpContext::new()).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
            .into_iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| Sibling::new(folder.join(&e.name).unwrap(), e.size))
            .collect()
    }

    #[test]
    fn stepping_through_a_folder_says_each_file_and_its_place() {
        let registry = Registry::with_local();
        let folder = fixture("folder");
        let first = folder.join("report-1.txt").unwrap();
        let (mut viewer, _) =
            Viewer::open(&registry, &first, None, ViewerOptions::default()).unwrap();
        viewer.set_siblings(siblings(&registry, &folder));
        assert_eq!(viewer.file_position(), Some((1, 4)));
        assert_eq!(
            texts(&viewer.previous_file(&registry)),
            ["First file, 1 of 4"]
        );
        viewer.jump("2");
        assert_eq!(
            texts(&viewer.next_file(&registry)),
            ["report-2.txt, 2 of 4"]
        );
        assert_eq!(viewer.cursor().line, 0, "a new file starts at its top");
        assert_eq!(viewer.rows()[0].text, "The second report.");
        assert!(viewer.title().starts_with("report-2.txt, in "));
        assert_eq!(
            texts(&viewer.next_file(&registry)),
            ["report-3.txt, 3 of 4", "Encoding: UTF-16 (byte-order mark)"]
        );
        assert_eq!(
            texts(&viewer.next_file(&registry)),
            [
                "report-4.bin, 4 of 4",
                "report-4.bin looks binary; hex mode"
            ]
        );
        assert_eq!(texts(&viewer.next_file(&registry)), ["Last file, 4 of 4"]);
        assert_eq!(
            texts(&viewer.previous_file(&registry)),
            ["report-3.txt, 3 of 4", "Encoding: UTF-16 (byte-order mark)"]
        );
        let (place, said) = viewer.show_in_folder();
        assert_eq!(said.text, "folder, report-3.txt");
        assert_eq!(place.name(), Some("report-3.txt"));
        let (effect, said) = viewer.copy_name();
        assert_eq!(effect, Effect::SetClipboardText("report-3.txt".into()));
        assert_eq!(said.text, "Name copied: report-3.txt");
        let (effect, said) = viewer.copy_path();
        let Effect::SetClipboardText(path) = effect else {
            panic!("the path goes on the clipboard as text: {effect:?}");
        };
        assert!(path.ends_with("report-3.txt"), "{path}");
        assert!(said.text.starts_with("Path copied: "));
        assert!(Effect::SetClipboardText(path).is_for_frontend());
        let mut alone = super::super::tests::memory(b"x");
        assert_eq!(
            texts(&alone.next_file(&registry)),
            ["No other files to step to"]
        );
    }
}
