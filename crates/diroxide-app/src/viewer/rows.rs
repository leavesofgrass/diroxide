//! The rows the viewer draws and the cursor's moves over them: each line
//! read as it is needed and kept near the screen, wrapped into parts when
//! wrapping is on, numbered when line numbers are on.

use diroxide_core::Message;
use diroxide_core::format::grouped;

use super::index::Found;
use super::text::{neutralize, read_line};
use super::wrap::{CONTINUED, cells, wrap};
use super::{COMMAND_BUDGET, MAX_LINE_BYTES, RowPos, ShownLine, Viewer, ViewerRow};

/// The words after a line cut at [`MAX_LINE_BYTES`].
const CUT: &str = " (line cut)";

/// One part of a line: its text range and the row's text.
struct Part {
    from: usize,
    prefix: usize,
    text: String,
}

impl Viewer {
    /// The rows to draw now, top to bottom.
    pub fn rows(&mut self) -> Vec<ViewerRow> {
        let mut rows = Vec::with_capacity(self.height);
        let mut pos = self.top;
        while rows.len() < self.height {
            let Some(parts) = self.parts(pos.line) else {
                break;
            };
            for (i, part) in parts.into_iter().enumerate().skip(pos.part) {
                if rows.len() >= self.height {
                    break;
                }
                let here = RowPos {
                    line: pos.line,
                    part: i,
                };
                rows.push(ViewerRow {
                    pos: here,
                    text: part.text,
                    prefix: part.prefix,
                    from: part.from,
                    focused: here == self.cursor,
                });
            }
            pos = RowPos {
                line: pos.line + 1,
                part: 0,
            };
        }
        rows
    }

    /// The row the cursor is on, counted from the first row drawn.
    pub fn cursor_row(&mut self) -> usize {
        self.rows()
            .iter()
            .position(|row| row.focused)
            .unwrap_or_default()
    }

    /// The line's shown text, as the rows and the token commands read it.
    pub fn line_text(&mut self, line: u64) -> Option<String> {
        self.shown(line).map(|shown| shown.text)
    }

    /// The columns a line's text wraps at: the fixed width when one is set,
    /// the screen's otherwise.
    pub fn wrap_columns(&self) -> usize {
        if self.wrap_width == 0 {
            self.width
        } else {
            self.wrap_width.min(self.width.max(1))
        }
    }

    /// A line's rows, or `None` past the end.
    fn parts(&mut self, line: u64) -> Option<Vec<Part>> {
        let shown = self.shown(line)?;
        let number = if self.line_numbers {
            format!("{}: ", grouped(line + 1))
        } else {
            String::new()
        };
        let mut text = shown.text;
        if shown.cut {
            text.push_str(CUT);
        }
        if !self.wrap {
            return Some(vec![Part {
                from: 0,
                prefix: number.len(),
                text: format!("{number}{text}"),
            }]);
        }
        let columns = self.wrap_columns();
        let first = columns.saturating_sub(cells(&number));
        let rest = columns.saturating_sub(cells(CONTINUED));
        Some(
            wrap(&text, first, rest)
                .into_iter()
                .enumerate()
                .map(|(i, range)| {
                    let head = if i == 0 { number.as_str() } else { CONTINUED };
                    Part {
                        from: range.start,
                        prefix: head.len(),
                        text: format!("{head}{}", &text[range]),
                    }
                })
                .collect(),
        )
    }

    /// How many rows a line takes.
    fn part_count(&mut self, line: u64) -> usize {
        self.parts(line).map_or(0, |parts| parts.len())
    }

    /// The line as shown, read and kept when it is not yet.
    pub(super) fn shown(&mut self, line: u64) -> Option<ShownLine> {
        if let Some(shown) = self.lines.get(&line) {
            return Some(shown.clone());
        }
        let offset = match line
            .checked_sub(1)
            .and_then(|before| self.lines.get(&before))
        {
            Some(before) if !before.last => before.next,
            Some(_) => return None,
            None => match self
                .index
                .line_start(&mut self.source, line, COMMAND_BUDGET)
            {
                Ok(Found::At(at)) => at,
                _ => return None,
            },
        };
        let raw = read_line(&mut self.source, self.form.newline, offset, MAX_LINE_BYTES).ok()?;
        if raw.last && raw.bytes.is_empty() && line > 0 {
            // After a final line end there is no line.
            if let Some(before) = self.lines.get_mut(&(line - 1)) {
                before.last = true;
            }
            return None;
        }
        let shown = ShownLine {
            offset,
            next: raw.next,
            text: neutralize(&self.form.decode_line(&raw.bytes)),
            cut: raw.cut,
            last: raw.last,
        };
        self.keep(line, shown.clone());
        Some(shown)
    }

    /// Keeps a shown line, dropping the ones far from the screen.
    fn keep(&mut self, line: u64, shown: ShownLine) {
        self.lines.insert(line, shown);
        let room = (self.height * 8).max(256);
        if self.lines.len() > room {
            let near = self.cursor.line;
            let far: Vec<u64> = self
                .lines
                .keys()
                .copied()
                .filter(|l| l.abs_diff(near) > room as u64 / 2)
                .collect();
            for l in far {
                self.lines.remove(&l);
            }
        }
    }

    /// The row after `pos`, or `None` at the end.
    fn next_pos(&mut self, pos: RowPos) -> Option<RowPos> {
        if pos.part + 1 < self.part_count(pos.line) {
            return Some(RowPos {
                line: pos.line,
                part: pos.part + 1,
            });
        }
        let next = pos.line + 1;
        self.shown(next).map(|_| RowPos {
            line: next,
            part: 0,
        })
    }

    /// The row before `pos`, or `None` at the top.
    fn prev_pos(&mut self, pos: RowPos) -> Option<RowPos> {
        if pos.part > 0 {
            return Some(RowPos {
                line: pos.line,
                part: pos.part - 1,
            });
        }
        let line = pos.line.checked_sub(1)?;
        let parts = self.part_count(line).max(1);
        Some(RowPos {
            line,
            part: parts - 1,
        })
    }

    /// Down one row: the screen reader reads the new row itself.
    pub fn down(&mut self) -> Vec<Message> {
        if let Some(next) = self.next_pos(self.cursor) {
            self.cursor = next;
            self.keep_cursor_visible();
        }
        Vec::new()
    }

    /// Up one row.
    pub fn up(&mut self) -> Vec<Message> {
        if let Some(prev) = self.prev_pos(self.cursor) {
            self.cursor = prev;
            self.keep_cursor_visible();
        }
        Vec::new()
    }

    /// Down one screen.
    pub fn page_down(&mut self) -> Vec<Message> {
        for _ in 0..self.height.saturating_sub(1).max(1) {
            match self.next_pos(self.cursor) {
                Some(next) => self.cursor = next,
                None => break,
            }
        }
        self.keep_cursor_visible();
        Vec::new()
    }

    /// Up one screen.
    pub fn page_up(&mut self) -> Vec<Message> {
        for _ in 0..self.height.saturating_sub(1).max(1) {
            match self.prev_pos(self.cursor) {
                Some(prev) => self.cursor = prev,
                None => break,
            }
        }
        self.keep_cursor_visible();
        Vec::new()
    }

    /// The first line.
    pub fn home(&mut self) -> Vec<Message> {
        self.cursor = RowPos::default();
        self.keep_cursor_visible();
        Vec::new()
    }

    /// The last line: at once when the index has reached the end, after a
    /// scan otherwise ([`Viewer::work`]); says "Line 4,000 of 4,000, the
    /// last" when it lands.
    pub fn end(&mut self) -> Vec<Message> {
        self.start_task(super::Task::Line {
            line: u64::MAX,
            landing: super::Landing::End,
        })
    }

    /// Scrolls so the cursor's row is on the screen, as little as needed.
    pub(super) fn keep_cursor_visible(&mut self) {
        let parts = self.part_count(self.cursor.line).max(1);
        if self.cursor.part >= parts {
            self.cursor.part = parts - 1;
        }
        if self.cursor < self.top {
            self.top = self.cursor;
            return;
        }
        // Count the rows from the top to the cursor, as far as a screen.
        let mut pos = self.top;
        let mut rows = 0usize;
        let far = self.cursor.line.saturating_sub(self.top.line) > self.height as u64;
        if !far {
            while pos < self.cursor && rows < self.height {
                match self.next_pos(pos) {
                    Some(next) => pos = next,
                    None => break,
                }
                rows += 1;
            }
            if rows < self.height {
                return;
            }
        }
        // Put the cursor on the last row: walk back a screen from it.
        let mut top = self.cursor;
        for _ in 1..self.height {
            match self.prev_pos(top) {
                Some(prev) => top = prev,
                None => break,
            }
        }
        self.top = top;
    }

    /// Turns wrapping on or off: "Wrap on".
    pub fn toggle_wrap(&mut self) -> Vec<Message> {
        self.wrap = !self.wrap;
        self.cursor.part = 0;
        self.top.part = 0;
        self.keep_cursor_visible();
        vec![Message::info(if self.wrap {
            "Wrap on"
        } else {
            "Wrap off"
        })]
    }

    /// Whether lines wrap.
    pub fn wraps(&self) -> bool {
        self.wrap
    }

    /// Sets the wrap width: 0 for the screen's width. "Wrap at 40
    /// characters", "Wrap at the screen's width".
    pub fn set_wrap_width(&mut self, columns: usize) -> Vec<Message> {
        self.wrap_width = columns;
        self.cursor.part = 0;
        self.top.part = 0;
        self.keep_cursor_visible();
        let words = if columns == 0 {
            "Wrap at the screen's width".to_owned()
        } else {
            format!("Wrap at {} characters", grouped(columns as u64))
        };
        vec![Message::info(words)]
    }

    /// Turns line numbers on or off: "Line numbers on".
    pub fn toggle_line_numbers(&mut self) -> Vec<Message> {
        self.line_numbers = !self.line_numbers;
        self.cursor.part = 0;
        self.top.part = 0;
        self.keep_cursor_visible();
        vec![Message::info(if self.line_numbers {
            "Line numbers on"
        } else {
            "Line numbers off"
        })]
    }

    /// Whether lines are numbered.
    pub fn numbers_lines(&self) -> bool {
        self.line_numbers
    }
}
