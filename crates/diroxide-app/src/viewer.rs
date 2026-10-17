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
//! it ("Showing the first 64 MB of 800 MB."); wrapping at the screen's
//! width or a fixed width, each continued row saying "continued" first;
//! line numbers; jumps to a line, an offset or a percent, each saying
//! where it landed; reload and follow.
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
mod jump;
mod rows;
mod source;
mod text;
mod wrap;

use std::collections::BTreeMap;
use std::fmt;

use diroxide_config::{Highlighting, Settings, ViewerSettings};
use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::display_name;
use diroxide_core::{Explain, Message, Place};
use diroxide_preview::encoding::{SNIFF_BYTES, binary_kind};
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

pub use jump::Jump;
pub use source::ViewerSource;
pub use wrap::CONTINUED;

use index::{Found, LineIndex};
use text::TextForm;

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
    /// The text as drawn and read: the line number when they are on, or
    /// "continued: " on a continued row, then the line's text or its part.
    pub text: String,
    /// The bytes of [`ViewerRow::text`] before the line's own text: the
    /// number or the "continued" cue.
    pub prefix: usize,
    /// Where the row's part starts in the line's shown text, in bytes, so a
    /// frontend can place the line's class runs on the row.
    pub from: usize,
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
    /// Land on a line: 0-based, `u64::MAX` for the last, with the words
    /// to say when there.
    Line { line: u64, landing: Landing },
    /// Land on the line that holds a byte.
    Offset { offset: u64, landing: Landing },
}

/// What to say on landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Landing {
    /// Nothing: a move the screen reader follows by itself.
    Quiet,
    /// "Line 4,000 of 4,000, the last".
    End,
    /// Where a jump landed: "At line 120 of 4,000".
    Jump(Jump),
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
    wrap: bool,
    wrap_width: usize,
    line_numbers: bool,
    follow: bool,
    lines: BTreeMap<u64, ShownLine>,
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
        source: ViewerSource,
        options: ViewerOptions,
    ) -> Result<(Self, Vec<Message>), ViewerError> {
        let name = place
            .name()
            .map_or_else(|| place.title(), |n| display_name(n).into_owned());
        let settings = options.settings;
        let mut viewer = Self {
            title: crate::panel::place_title(&place),
            place,
            name,
            options,
            index: LineIndex::new(0, text::Newline::Byte),
            source,
            form: TextForm::from_head(b"", false),
            mode: ViewerMode::Text,
            binary: None,
            cursor: RowPos::default(),
            top: RowPos::default(),
            width: 80,
            height: 24,
            wrap: settings.wrap,
            wrap_width: usize::try_from(settings.wrap_width).unwrap_or(usize::MAX),
            line_numbers: settings.line_numbers,
            follow: settings.follow,
            lines: BTreeMap::new(),
            task: None,
        };
        viewer.load()?;
        let said = viewer.opening_lines();
        Ok((viewer, said))
    }

    /// Reads the start of the source: the text form, the binary check and
    /// the first slice of the index.
    fn load(&mut self) -> Result<(), ViewerError> {
        let head = self.source.chunk(0, SNIFF_BYTES)?.to_vec();
        let more_follows = self.source.total().is_none_or(|t| t > head.len() as u64);
        self.form = TextForm::from_head(&head, more_follows);
        self.binary = binary_kind(&head);
        self.mode = if self.binary.is_some() {
            ViewerMode::Hex
        } else {
            ViewerMode::Text
        };
        self.index = LineIndex::new(self.form.start(), self.form.newline);
        self.lines.clear();
        self.task = None;
        let budget = if self.source.is_complete() && self.source.available() <= INDEX_AT_OPEN {
            u64::MAX
        } else {
            COMMAND_BUDGET
        };
        self.index.scan(&mut self.source, budget)?;
        Ok(())
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

    /// How many lines are known so far, for a status that has no total.
    pub fn lines_known(&self) -> u64 {
        self.index.lines_known()
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

    fn start_task(&mut self, task: Task) -> Vec<Message> {
        self.task = Some(task.clone());
        self.run_task(task, COMMAND_BUDGET)
    }

    fn run_task(&mut self, task: Task, budget: u64) -> Vec<Message> {
        let result = match task {
            Task::Line { line, landing } => self
                .find_line(line, budget)
                .map(|found| found.map(|(line, past)| (line, past, landing))),
            Task::Offset { offset, landing } => self
                .index
                .line_of(&mut self.source, offset, budget)
                .map(|found| found.map(|line| (line, false, landing))),
        };
        match result {
            Ok(None) => Vec::new(),
            Ok(Some((line, past, landing))) => {
                self.task = None;
                self.land(line, past, landing)
            }
            Err(error) => {
                self.task = None;
                vec![ViewerError::from(error).message(&self.name)]
            }
        }
    }

    /// Where line `line` is (`u64::MAX` for the last), within the budget:
    /// the line found and whether the one asked for was past the end.
    fn find_line(&mut self, line: u64, budget: u64) -> std::io::Result<Option<(u64, bool)>> {
        let target = if line == u64::MAX {
            if !self.index.scan(&mut self.source, budget)? {
                return Ok(None);
            }
            self.index.lines_known().saturating_sub(1)
        } else {
            line
        };
        Ok(
            match self.index.line_start(&mut self.source, target, budget)? {
                Found::Pending => None,
                Found::At(_) => Some((target, false)),
                Found::PastEnd { last } => Some((last, line != u64::MAX)),
            },
        )
    }

    fn land(&mut self, line: u64, past: bool, landing: Landing) -> Vec<Message> {
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
            Landing::Jump(jump) => vec![Message::info(self.landed(jump, line, past))],
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
