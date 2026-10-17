//! The viewer (F3): a file read as a list of lines, from the first screen
//! at once however large the file is, with every state said in words.
//!
//! What it holds (research/alpha2-plan.md parts 2.4 and 3.4, commits 38 to
//! 47): the bytes from a seekable read or a forward-only stream
//! ([`ViewerSource`]); the encoding chosen from the first bytes and said
//! when it is not plain UTF-8 ("Encoding: UTF-16 (byte-order mark)"); a
//! lazy, sparse line index, one offset every thousandth line, built only
//! as far as needed and in slices ([`Viewer::work`]), so the input thread
//! never waits on a whole file; the large-file policy for a stream, whose
//! decoded bytes are kept up to the `member_cache_mib` cap and said beyond
//! it ("Showing the first 64 MB of 800 MB.").
//!
//! How a frontend drives it: [`Viewer::open`] (or [`Viewer::from_source`])
//! returns the viewer and what to say; [`Viewer::set_size`] gives the
//! rows and columns; [`Viewer::rows`] gives the rows to draw, each one
//! text run, meaning first, and [`Viewer::cursor_row`] the row the
//! hardware cursor sits on (its first cell); every command is a plain
//! method that returns the [`Message`]s to say, in order; [`Viewer::work`]
//! carries on a scan the last command left pending, while
//! [`Viewer::is_busy`] says one is. Methods read from the source, a few
//! blocks at a time; nothing here touches the screen.
//!
//! The viewer is `Send`, so a frontend may keep it on a worker thread.

mod index;
mod source;
mod text;

use std::fmt;

use diroxide_config::{Highlighting, Settings, ViewerSettings};
use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::display_name;
use diroxide_core::{Explain, Message, Place};
use diroxide_preview::encoding::{SNIFF_BYTES, binary_kind};
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

pub use source::ViewerSource;

use index::{Found, LineIndex};
use text::{TextForm, neutralize, read_line};

/// How many bytes of one line are shown at most; the rest of a longer
/// line is cut, and the row says so.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// How many bytes a command may scan before it leaves the rest to
/// [`Viewer::work`]: about a hundredth of a second.
pub const COMMAND_BUDGET: u64 = 8 * 1024 * 1024;

/// How many bytes one call of [`Viewer::work`] scans.
pub const WORK_BUDGET: u64 = 32 * 1024 * 1024;

/// A file this small is indexed to its end when it opens, so its line
/// count is known at once.
pub const INDEX_AT_OPEN: u64 = 8 * 1024 * 1024;

/// Why the viewer could not read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ViewerError {
    /// The provider failed.
    #[error(transparent)]
    Read(#[from] VfsError),
}

impl From<std::io::Error> for ViewerError {
    fn from(error: std::io::Error) -> Self {
        ViewerError::Read(VfsError::from(error))
    }
}

impl Explain for ViewerError {
    fn next_step(&self) -> String {
        match self {
            ViewerError::Read(inner) => inner.next_step(),
        }
    }
}

impl ViewerError {
    /// The message, the verb first: "Could not read notes.txt: CAUSE.
    /// NEXT STEP."
    pub fn message(&self, name: &str) -> Message {
        Message::from_error("read", name, self)
    }
}

/// What the viewer reads from the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerOptions {
    /// Behave for a screen reader: eight bytes a hex row in automatic mode,
    /// no colors.
    pub screen_reader: bool,
    /// The `[viewer]` table.
    pub settings: ViewerSettings,
    /// Whether code is colored.
    pub highlighting: Highlighting,
}

impl ViewerOptions {
    /// The options from the app's settings.
    pub fn from_settings(settings: &Settings, screen_reader: bool) -> Self {
        Self {
            screen_reader,
            settings: settings.viewer,
            highlighting: settings.highlighting,
        }
    }

    /// The bytes a stream keeps at most: `member_cache_mib`, at least 1 MiB.
    pub fn stream_cap(&self) -> u64 {
        self.settings.member_cache_mib.max(1) * 1024 * 1024
    }
}

impl Default for ViewerOptions {
    fn default() -> Self {
        Self {
            screen_reader: true,
            settings: ViewerSettings::default(),
            highlighting: Highlighting::Auto,
        }
    }
}

impl ViewerSource {
    /// Opens a place for the viewer: a seekable read where the provider
    /// offers one, a stream kept up to `cap` decoded bytes otherwise (a
    /// member of a zip); `size` is the listed size, for the policy line.
    pub fn open(
        registry: &Registry,
        place: &Place,
        size: Option<u64>,
        cap: u64,
    ) -> Result<Self, ViewerError> {
        let provider = registry.resolve(place)?;
        let ctx = OpContext::new();
        match provider.open_seekable(place, &ctx) {
            Ok(reader) => Ok(Self::seekable(reader)?),
            Err(_) => {
                let reader = provider.read(place, &ctx)?;
                Ok(Self::stream(reader, size, cap))
            }
        }
    }
}

/// A row's place in the file: a line and, when the line wraps, which of
/// its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowPos {
    /// The line, 0-based (said 1-based).
    pub line: u64,
    /// The part of a wrapped line, 0 for its first.
    pub part: usize,
}

/// One row to draw: one text run, meaning first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerRow {
    /// Where it is.
    pub pos: RowPos,
    /// The text as drawn and read: the line, or its part.
    pub text: String,
    /// Whether the cursor is on this row.
    pub focused: bool,
}

/// What the viewer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewerMode {
    /// Lines of text.
    #[default]
    Text,
    /// Rows of hex.
    Hex,
}

/// A line as read and shown.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ShownLine {
    offset: u64,
    next: u64,
    text: String,
    cut: bool,
    last: bool,
}

/// What a scan was started for, finished by [`Viewer::work`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum Task {
    /// Land on a line: 0-based, with the words to say when there.
    Line { line: u64, landing: Landing },
}

/// What to say on landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Landing {
    /// Nothing: a move the screen reader follows by itself.
    #[cfg_attr(not(test), expect(dead_code, reason = "jump, the next commit"))]
    Quiet,
    /// "Line 4,000 of 4,000, the last".
    End,
}

/// The viewer. See the module documentation.
pub struct Viewer {
    place: Place,
    name: String,
    title: String,
    options: ViewerOptions,
    source: ViewerSource,
    form: TextForm,
    index: LineIndex,
    mode: ViewerMode,
    binary: Option<&'static str>,
    cursor: RowPos,
    top: RowPos,
    width: usize,
    height: usize,
    lines: std::collections::BTreeMap<u64, ShownLine>,
    task: Option<Task>,
}

impl fmt::Debug for Viewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Viewer")
            .field("place", &self.place)
            .field("mode", &self.mode)
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl Viewer {
    /// Opens a place through the registry and says what it found: the
    /// title, then the encoding, the policy or the binary reason when there
    /// is one. `size` is the listed size, when the panel knows it.
    pub fn open(
        registry: &Registry,
        place: &Place,
        size: Option<u64>,
        options: ViewerOptions,
    ) -> Result<(Self, Vec<Message>), ViewerError> {
        let source = ViewerSource::open(registry, place, size, options.stream_cap())?;
        Self::from_source(place.clone(), source, options)
    }

    /// A viewer over a source already open.
    pub fn from_source(
        place: Place,
        mut source: ViewerSource,
        options: ViewerOptions,
    ) -> Result<(Self, Vec<Message>), ViewerError> {
        let head = source.chunk(0, SNIFF_BYTES)?.to_vec();
        let more_follows = source.total().is_none_or(|t| t > head.len() as u64);
        let form = TextForm::from_head(&head, more_follows);
        let binary = binary_kind(&head);
        let name = place
            .name()
            .map_or_else(|| place.title(), |n| display_name(n).into_owned());
        let mut viewer = Self {
            title: crate::panel::place_title(&place),
            place,
            name,
            options,
            index: LineIndex::new(form.start(), form.newline),
            source,
            form,
            mode: ViewerMode::Text,
            binary,
            cursor: RowPos::default(),
            top: RowPos::default(),
            width: 80,
            height: 24,
            lines: std::collections::BTreeMap::new(),
            task: None,
        };
        if viewer.binary.is_some() {
            viewer.mode = ViewerMode::Hex;
        }
        let available = viewer.source.available();
        if viewer.source.is_complete() && available <= INDEX_AT_OPEN {
            viewer.index.scan(&mut viewer.source, u64::MAX)?;
        } else {
            viewer.index.scan(&mut viewer.source, COMMAND_BUDGET)?;
        }
        let said = viewer.opening_lines();
        Ok((viewer, said))
    }

    /// What is said when the file opens: the title, then each policy line.
    fn opening_lines(&self) -> Vec<Message> {
        let mut said = vec![Message::info(self.title.clone())];
        if let Some(reason) = self.binary_line() {
            said.push(Message::info(reason));
        } else if let Some(line) = self.form.choice.line() {
            said.push(Message::info(line));
        }
        if let Some(policy) = self.policy_line() {
            said.push(Message::info(policy));
        }
        said
    }

    /// "report.bin looks binary; hex mode", when the first bytes are not
    /// text.
    fn binary_line(&self) -> Option<String> {
        self.binary
            .map(|_| format!("{} looks binary; hex mode", self.name))
    }

    /// The large-file policy, for a stream over its cap: "Showing the first
    /// 64 MB of 800 MB."
    pub fn policy_line(&self) -> Option<String> {
        let cap = self.source.cap()?;
        if !(self.source.will_cap() || self.source.is_capped()) {
            return None;
        }
        let shown = human_size(cap);
        Some(match self.source.total() {
            Some(total) => format!("Showing the first {shown} of {}.", human_size(total)),
            None => format!("Showing the first {shown}."),
        })
    }

    /// The place viewed.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The file's name as shown.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The title line, the name first: "notes.txt, in course.zip".
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Text or hex.
    pub fn mode(&self) -> ViewerMode {
        self.mode
    }

    /// The options.
    pub fn options(&self) -> &ViewerOptions {
        &self.options
    }

    /// The row the cursor is on.
    pub fn cursor(&self) -> RowPos {
        self.cursor
    }

    /// The file's lines, once the index has reached its end.
    pub fn line_count(&self) -> Option<u64> {
        self.index.total()
    }

    /// The memory the line index takes, in bytes.
    pub fn index_memory(&self) -> usize {
        self.index.memory()
    }

    /// The rows and columns the frontend draws the lines in.
    pub fn set_size(&mut self, width: usize, height: usize) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.keep_cursor_visible();
    }

    /// The rows the lines get.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The columns a row gets.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Whether a scan is still running for the last command.
    pub fn is_busy(&self) -> bool {
        self.task.is_some()
    }

    /// Whether there is work to do: a pending command, or an index not yet
    /// at the end of the file (so the line count is not yet known).
    pub fn wants_work(&self) -> bool {
        self.task.is_some() || !self.index.is_done()
    }

    /// Carries on: a pending command's scan first, then the index toward
    /// the end of the file, `budget` bytes at most. Returns what the
    /// finished command says.
    pub fn work(&mut self, budget: u64) -> Vec<Message> {
        if let Some(task) = self.task.clone() {
            return self.run_task(task, budget);
        }
        if let Err(error) = self.index.scan(&mut self.source, budget) {
            return vec![ViewerError::from(error).message(&self.name)];
        }
        Vec::new()
    }

    /// Runs [`Viewer::work`] until nothing is pending, for tests and for a
    /// caller that may wait.
    pub fn finish(&mut self) -> Vec<Message> {
        let mut said = Vec::new();
        while self.task.is_some() {
            said.extend(self.work(u64::MAX));
        }
        said
    }

    /// Stops a pending command (Escape while a scan runs).
    pub fn cancel(&mut self) -> Vec<Message> {
        match self.task.take() {
            Some(_) => vec![Message::info("Stopped")],
            None => Vec::new(),
        }
    }

    /// Say Status: "Line 120 of 4,000, 3 percent".
    pub fn status(&self) -> String {
        let line = self.cursor.line + 1;
        match self.index.total() {
            Some(0) => "Empty file".to_owned(),
            Some(total) => format!(
                "Line {} of {}, {} percent",
                grouped(line),
                grouped(total),
                percent(line, total)
            ),
            None => {
                let at = self
                    .lines
                    .get(&self.cursor.line)
                    .map_or(0, |shown| shown.offset);
                let size = self.source.available().max(1);
                format!("Line {}, {} percent", grouped(line), percent(at, size))
            }
        }
    }

    /// The row the cursor is on, counted from the first row drawn.
    pub fn cursor_row(&mut self) -> usize {
        self.rows()
            .iter()
            .position(|row| row.focused)
            .unwrap_or_default()
    }

    /// The rows to draw now, top to bottom.
    pub fn rows(&mut self) -> Vec<ViewerRow> {
        let mut rows = Vec::with_capacity(self.height);
        let mut line = self.top.line;
        while rows.len() < self.height {
            let Some(shown) = self.shown(line) else {
                break;
            };
            let mut text = shown.text.clone();
            if shown.cut {
                text.push_str(" (line cut)");
            }
            let pos = RowPos { line, part: 0 };
            rows.push(ViewerRow {
                focused: pos == self.cursor,
                pos,
                text,
            });
            if shown.last {
                break;
            }
            line += 1;
        }
        rows
    }

    /// The line as shown, read and kept when it is not yet.
    fn shown(&mut self, line: u64) -> Option<ShownLine> {
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

    /// Whether line `line` exists.
    fn has_line(&mut self, line: u64) -> bool {
        self.shown(line).is_some()
    }

    /// Down one row: the screen reader reads the new row itself.
    pub fn down(&mut self) -> Vec<Message> {
        let next = self.cursor.line + 1;
        if self.has_line(next) {
            self.cursor = RowPos {
                line: next,
                part: 0,
            };
            self.keep_cursor_visible();
        }
        Vec::new()
    }

    /// Up one row.
    pub fn up(&mut self) -> Vec<Message> {
        if self.cursor.line > 0 {
            self.cursor = RowPos {
                line: self.cursor.line - 1,
                part: 0,
            };
            self.keep_cursor_visible();
        }
        Vec::new()
    }

    /// Down one screen.
    pub fn page_down(&mut self) -> Vec<Message> {
        for _ in 0..self.height.saturating_sub(1).max(1) {
            let before = self.cursor;
            self.down();
            if self.cursor == before {
                break;
            }
        }
        Vec::new()
    }

    /// Up one screen.
    pub fn page_up(&mut self) -> Vec<Message> {
        for _ in 0..self.height.saturating_sub(1).max(1) {
            if self.cursor.line == 0 {
                break;
            }
            self.up();
        }
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
        self.start_task(Task::Line {
            line: u64::MAX,
            landing: Landing::End,
        })
    }

    fn start_task(&mut self, task: Task) -> Vec<Message> {
        self.task = Some(task.clone());
        self.run_task(task, COMMAND_BUDGET)
    }

    fn run_task(&mut self, task: Task, budget: u64) -> Vec<Message> {
        match task {
            Task::Line { line, landing } => {
                let target = if line == u64::MAX {
                    match self.index.scan(&mut self.source, budget) {
                        Ok(true) => self.index.lines_known().saturating_sub(1),
                        Ok(false) => return Vec::new(),
                        Err(error) => {
                            self.task = None;
                            return vec![ViewerError::from(error).message(&self.name)];
                        }
                    }
                } else {
                    line
                };
                match self.index.line_start(&mut self.source, target, budget) {
                    Ok(Found::Pending) => Vec::new(),
                    Ok(Found::At(_)) => {
                        self.task = None;
                        self.land(target, landing)
                    }
                    Ok(Found::PastEnd { last }) => {
                        self.task = None;
                        self.land(last, landing)
                    }
                    Err(error) => {
                        self.task = None;
                        vec![ViewerError::from(error).message(&self.name)]
                    }
                }
            }
        }
    }

    fn land(&mut self, line: u64, landing: Landing) -> Vec<Message> {
        self.cursor = RowPos { line, part: 0 };
        self.keep_cursor_visible();
        match landing {
            Landing::Quiet => Vec::new(),
            Landing::End => {
                let total = self.index.lines_known();
                vec![Message::info(format!(
                    "Line {} of {}, the last",
                    grouped(line + 1),
                    grouped(total)
                ))]
            }
        }
    }

    /// Scrolls so the cursor's row is on the screen, as little as needed.
    fn keep_cursor_visible(&mut self) {
        if self.cursor < self.top {
            self.top = self.cursor;
            return;
        }
        let span = self.cursor.line.saturating_sub(self.top.line);
        if span >= self.height as u64 {
            self.top = RowPos {
                line: self.cursor.line + 1 - self.height as u64,
                part: 0,
            };
        }
    }

    /// Reads further into a stream past its cap, by the cap again ("read
    /// more"): "Showing the first 128 MB of 800 MB.", or "Showing all of
    /// notes.txt." once it is whole.
    pub fn read_more(&mut self) -> Vec<Message> {
        let Some(cap) = self.source.cap() else {
            return vec![Message::info(format!("Showing all of {}.", self.name))];
        };
        if !self.source.is_capped() && !self.source.will_cap() {
            return vec![Message::info(format!("Showing all of {}.", self.name))];
        }
        self.source.raise_cap(cap);
        self.index.reopen();
        // The last line may have been cut by the old cap.
        if let Some((&last, _)) = self.lines.iter().next_back() {
            self.lines.remove(&last);
        }
        if let Err(error) = self.index.scan(&mut self.source, COMMAND_BUDGET) {
            return vec![ViewerError::from(error).message(&self.name)];
        }
        match self.policy_line() {
            Some(line) => vec![Message::info(line)],
            None => vec![Message::info(format!("Showing all of {}.", self.name))],
        }
    }

    /// How many lines are known so far, for a status that has no total.
    pub fn lines_known(&self) -> u64 {
        self.index.lines_known()
    }

    /// The words for a count of lines: "4,000 lines".
    pub fn lines_words(lines: u64) -> String {
        if lines >= 1000 {
            format!("{} lines", grouped(lines))
        } else {
            count(lines, "line", "lines")
        }
    }
}

/// A whole percent, rounded down, of `part` in `whole`.
fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }
    ((u128::from(part) * 100) / u128::from(whole)).min(100) as u64
}

#[cfg(test)]
mod tests;
