//! Hex mode: the file as rows of bytes through the preview crate's one
//! hex row function, eight bytes a row in screen reader mode (the row is
//! then 40 characters), sixteen otherwise, as the `hex_row` setting says;
//! a byte cursor that Left and Right move, saying each byte; and Say
//! Byte: "Byte 16 is 48 hex, the letter H".

use diroxide_core::Message;
use diroxide_core::format::grouped;
use diroxide_preview::hex::{HexLayout, byte_words, hex_row};

use super::{RowPos, Viewer, ViewerMode, ViewerRow, percent};

impl Viewer {
    /// The layout of a hex row for this file and these settings.
    pub fn hex_layout(&self) -> HexLayout {
        let wanted = self
            .options
            .settings
            .hex_row
            .bytes_per_row(self.options.screen_reader);
        HexLayout::for_len(self.source.available(), wanted)
    }

    /// The byte the hex cursor is on.
    pub fn hex_cursor(&self) -> u64 {
        self.hex_cursor
    }

    /// Switches between text and hex: "Hex mode, 8 bytes a row", "Text
    /// mode". The cursor keeps its place in the file.
    pub fn toggle_hex(&mut self) -> Vec<Message> {
        match self.mode {
            ViewerMode::Text => {
                let at = self.shown(self.cursor.line).map_or(0, |s| s.offset);
                self.mode = ViewerMode::Hex;
                self.hex_cursor = at.min(self.source.available().saturating_sub(1));
                self.keep_hex_visible();
                let n = self.hex_layout().bytes_per_row;
                vec![Message::info(format!("Hex mode, {n} bytes a row"))]
            }
            ViewerMode::Hex => {
                self.mode = ViewerMode::Text;
                let offset = self.hex_cursor;
                let mut said = vec![Message::info("Text mode")];
                said.extend(self.start_task(super::Task::Offset {
                    offset,
                    landing: super::Landing::Quiet,
                }));
                said
            }
        }
    }

    /// The hex rows to draw: each row's number in [`RowPos::line`].
    pub(super) fn hex_rows(&mut self) -> Vec<ViewerRow> {
        let layout = self.hex_layout();
        let n = layout.bytes_per_row as u64;
        let len = self.source.available();
        let mut rows = Vec::with_capacity(self.height);
        let mut offset = self.hex_top;
        let cursor_row = layout.row_start(self.hex_cursor);
        while rows.len() < self.height && (offset < len || (len == 0 && rows.is_empty())) {
            let bytes = self
                .source
                .chunk(offset, n as usize)
                .map(<[u8]>::to_vec)
                .unwrap_or_default();
            rows.push(ViewerRow {
                pos: RowPos {
                    line: offset / n,
                    part: 0,
                },
                text: hex_row(offset, &bytes, layout),
                prefix: 0,
                from: 0,
                focused: offset == cursor_row,
            });
            offset += n;
        }
        rows
    }

    /// Scrolls so the hex cursor's row is on the screen.
    pub(super) fn keep_hex_visible(&mut self) {
        let layout = self.hex_layout();
        let n = layout.bytes_per_row as u64;
        let row = layout.row_start(self.hex_cursor);
        self.hex_top = layout.row_start(self.hex_top);
        if row < self.hex_top {
            self.hex_top = row;
        } else if row >= self.hex_top + n * self.height as u64 {
            self.hex_top = row.saturating_sub(n * (self.height as u64 - 1));
        }
    }

    /// Moves the hex cursor by `delta` bytes, kept inside the file.
    pub(super) fn hex_move(&mut self, delta: i64) {
        let last = self.source.available().saturating_sub(1);
        self.hex_cursor = self.hex_cursor.saturating_add_signed(delta).min(last);
        self.keep_hex_visible();
    }

    /// Moves the hex cursor to a byte.
    pub(super) fn hex_go(&mut self, offset: u64) {
        self.hex_cursor = offset.min(self.source.available().saturating_sub(1));
        self.keep_hex_visible();
    }

    /// The bytes of a hex row, as a signed step.
    pub(super) fn hex_step(&self) -> i64 {
        self.hex_layout().bytes_per_row as i64
    }

    /// Say Byte: "Byte 16 is 48 hex, the letter H", the offset counted
    /// from 0 as the rows count it.
    pub fn say_byte(&mut self) -> Vec<Message> {
        let offset = self.hex_cursor;
        match self.source.chunk(offset, 1).map(|b| b.first().copied()) {
            Ok(Some(byte)) => vec![Message::info(format!(
                "Byte {} is {}",
                grouped(offset),
                byte_words(byte)
            ))],
            Ok(None) => vec![Message::info("No bytes: the file is empty")],
            Err(error) => vec![super::ViewerError::from(error).message(&self.name)],
        }
    }

    /// Say Status in hex mode: "Offset 16 of 2,048 bytes, 0 percent".
    pub(super) fn hex_status(&self) -> String {
        let len = self.source.available();
        format!(
            "Offset {} of {} bytes, {} percent",
            grouped(self.hex_cursor),
            grouped(len),
            percent(self.hex_cursor + u64::from(len > 0), len.max(1))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{memory, open, texts};
    use super::*;

    #[test]
    fn hex_rows_fit_forty_cells_and_say_their_bytes() {
        let mut viewer = memory(b"0123456789abcdefHello world, and more text here.");
        viewer.set_size(40, 10);
        assert_eq!(texts(&viewer.toggle_hex()), ["Hex mode, 8 bytes a row"]);
        let rows = viewer.rows();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[2].text, "000010: 48 65 6c 6c 6f 20 77 6f Hello wo");
        assert!(rows.iter().all(|r| r.text.chars().count() <= 40));
        assert!(rows[0].focused);
        viewer.down();
        viewer.down();
        assert_eq!(
            texts(&viewer.say_byte()),
            ["Byte 16 is 48 hex, the letter H"]
        );
        assert_eq!(viewer.cursor_row(), 2);
        assert_eq!(texts(&viewer.right()), ["Byte 17 is 65 hex, the letter e"]);
        assert_eq!(texts(&viewer.left()), ["Byte 16 is 48 hex, the letter H"]);
        assert_eq!(viewer.status(), "Offset 16 of 48 bytes, 35 percent");
        viewer.end();
        assert_eq!(viewer.hex_cursor(), 40, "the last row's first byte");
        viewer.home();
        assert_eq!(viewer.hex_cursor(), 0);
        assert_eq!(texts(&viewer.jump("0x12")), ["At offset 18"]);
        assert_eq!(
            texts(&viewer.say_byte()),
            ["Byte 18 is 6c hex, the letter l"]
        );
        assert_eq!(
            texts(&viewer.search("\"world\"", false)),
            ["Match 1 of 1, offset 22"]
        );
        assert_eq!(viewer.hex_cursor(), 22);
        assert_eq!(texts(&viewer.toggle_hex())[0], "Text mode");
        assert_eq!(viewer.mode(), ViewerMode::Text);
    }

    #[test]
    fn sixteen_bytes_a_row_out_of_screen_reader_mode() {
        let mut viewer = memory(&[0x41; 64]);
        viewer.options.screen_reader = false;
        viewer.toggle_hex();
        let rows = viewer.rows();
        assert_eq!(rows.len(), 4);
        assert!(rows[1].text.starts_with("000010: 41 41"));
    }

    #[test]
    fn a_binary_file_opens_in_hex_with_its_rows() {
        let (mut viewer, said) = open("encoding/pixel.png");
        assert_eq!(said[1], "pixel.png looks binary; hex mode");
        let rows = viewer.rows();
        assert_eq!(rows[0].text, "000000: 89 50 4e 47 0d 0a 1a 0a .PNG....");
    }
}
