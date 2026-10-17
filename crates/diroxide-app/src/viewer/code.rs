//! Code read by ear (research/alpha2-plan.md part 3.5; R17): highlighting
//! is silent in screen reader mode, and these commands are how it is
//! heard, all read from the same parse that gives sighted users their
//! colors. The point (a place in the cursor's line that Left and Right
//! move by token), the token's kind ("Function name: parse_args, line
//! 40"), the line by kind ("Line 40: keyword, function name, string"),
//! the outline as a list ("Outline: 14 symbols in main.rs"), next and
//! previous symbol, the bracket jump ("Matches ) on line 52"), skip
//! comment ("Next code, line 58"), indentation ("Indent level 2") with
//! IndentNav's moves, and the context ("In function main, in impl
//! Config").
//!
//! A text file within the highlight caps is read whole for these, with
//! or without a language: brackets, comments of a known language and
//! indentation work on any of them.

use std::sync::Arc;

use diroxide_core::format::{count, grouped};
use diroxide_core::{ListModel, ListRow, Message};
use diroxide_preview::classes::{ClassRun, TokenClass, class_at};
use diroxide_preview::highlight::{
    Caps, Language, Session, Symbol, SymbolKind, structure_symbols, validity,
};

use super::text::{neutralize, read_line};
use super::{MAX_LINE_BYTES, RowPos, Viewer, ViewerMode, ViewerRow};

/// The most kinds a line by kind names before "and 2 more".
const MOST_KINDS: usize = 4;

/// A small text file, read whole: its lines as shown, and once the worker
/// is done, their classes and the file's symbols.
#[derive(Debug, Default)]
pub(crate) struct CodeDoc {
    language: Option<Language>,
    lines: Arc<Vec<String>>,
    runs: Vec<Vec<ClassRun>>,
    symbols: Vec<Symbol>,
}

/// Where a bracket is: its line, its byte and the character.
type Bracket = (usize, usize, char);

impl Viewer {
    /// Gives the viewer the session's highlighter and starts on this file:
    /// a file within the caps is read whole, its language found by name
    /// and first line, and the worker started. What it says, once: the
    /// validity of JSON and TOML, or why highlighting is off ("Highlighting
    /// off: over 512 KiB"); nothing in the usual case. The viewer starts
    /// again by itself on reload and on the next file.
    pub fn set_highlighter(&mut self, session: Session) -> Vec<Message> {
        self.session = Some(session);
        self.start_code()
    }

    /// The caps from the settings.
    fn caps(&self) -> Caps {
        let s = &self.options.settings;
        Caps {
            max_bytes: s.highlight_max_kib.saturating_mul(1024),
            max_lines: s.highlight_max_lines,
            max_line_chars: s.highlight_max_line_chars,
            deadline: std::time::Duration::from_millis(s.highlight_deadline_ms),
        }
    }

    /// Reads the file whole when it is small and starts the highlighter.
    pub(super) fn start_code(&mut self) -> Vec<Message> {
        self.code = None;
        self.job = None;
        self.point = 0;
        let Some(session) = self.session.clone() else {
            return Vec::new();
        };
        if self.mode == ViewerMode::Hex {
            return Vec::new();
        }
        let first = self.shown(0).map(|s| s.text).unwrap_or_default();
        let language = Language::detect(self.place.name().unwrap_or_default(), &first);
        let caps = self.caps();
        let size = self.source.available();
        if size > caps.max_bytes || !self.source.is_complete() {
            return match language {
                Some(_) => vec![Message::info(
                    diroxide_preview::highlight::Refusal::TooLarge {
                        max_bytes: caps.max_bytes,
                    }
                    .line(),
                )],
                None => Vec::new(),
            };
        }
        let Some(lines) = self.read_all(caps.max_lines as usize + 1) else {
            return Vec::new();
        };
        let lines = Arc::new(lines);
        let mut said = Vec::new();
        if let Some(language) = language
            && let Some(valid) = validity(language, &lines.join("\n"))
        {
            said.push(Message::info(valid.line(language, size)));
        }
        let structure = language.and_then(|l| structure_symbols(l, &lines));
        self.code = Some(CodeDoc {
            language,
            lines: Arc::clone(&lines),
            runs: Vec::new(),
            symbols: structure.unwrap_or_default(),
        });
        let Some(language) = language else {
            return said;
        };
        match session.start(language, lines, size, caps) {
            Ok(job) => self.job = Some(job),
            Err(refusal) => said.push(Message::info(refusal.line())),
        }
        said
    }

    /// Every line as shown, or `None` past `most` lines.
    fn read_all(&mut self, most: usize) -> Option<Vec<String>> {
        let mut lines = Vec::new();
        let mut offset = self.form.start();
        loop {
            let raw =
                read_line(&mut self.source, self.form.newline, offset, MAX_LINE_BYTES).ok()?;
            if raw.last && raw.bytes.is_empty() && !lines.is_empty() {
                break;
            }
            lines.push(neutralize(&self.form.decode_line(&raw.bytes)));
            if lines.len() > most {
                return None;
            }
            if raw.last {
                break;
            }
            offset = raw.next;
        }
        Some(lines)
    }

    /// Takes the worker's result when it is ready: silent when it is, and
    /// "Highlighting stopped: took too long" when it overran.
    pub fn poll_highlight(&mut self) -> Vec<Message> {
        let Some(job) = self.job.as_mut() else {
            return Vec::new();
        };
        let Some(result) = job.poll() else {
            return Vec::new();
        };
        self.job = None;
        self.take_highlight(result)
    }

    /// Waits for the worker, for tests and the bench.
    pub fn wait_highlight(&mut self) -> Vec<Message> {
        let Some(mut job) = self.job.take() else {
            return Vec::new();
        };
        let result = job.wait();
        self.take_highlight(result)
    }

    fn take_highlight(
        &mut self,
        result: Result<
            diroxide_preview::highlight::Highlighted,
            diroxide_preview::highlight::Refusal,
        >,
    ) -> Vec<Message> {
        match result {
            Ok(done) => {
                if let Some(code) = self.code.as_mut() {
                    code.runs = done.runs;
                    code.symbols = done.symbols;
                }
                Vec::new()
            }
            Err(refusal) => vec![Message::info(refusal.line())],
        }
    }

    /// Whether the worker is still at it.
    pub fn is_highlighting(&self) -> bool {
        self.job.is_some()
    }

    /// The language found for this file.
    pub fn language(&self) -> Option<Language> {
        self.code.as_ref().and_then(|c| c.language)
    }

    /// Whether the file's lines carry classes.
    pub fn is_highlighted(&self) -> bool {
        self.code.as_ref().is_some_and(|c| !c.runs.is_empty())
    }

    /// A line's class runs, by bytes of its shown text.
    pub fn line_runs(&self, line: u64) -> &[ClassRun] {
        self.code
            .as_ref()
            .and_then(|c| c.runs.get(line as usize))
            .map_or(&[], Vec::as_slice)
    }

    /// A row's class runs, by bytes of the row's text: the line's runs cut
    /// to the row's part and moved past its prefix.
    pub fn row_runs(&self, row: &ViewerRow) -> Vec<ClassRun> {
        if self.mode == ViewerMode::Hex {
            return Vec::new();
        }
        let len = row.text.len().saturating_sub(row.prefix);
        let (from, to) = (row.from, row.from + len);
        self.line_runs(row.pos.line)
            .iter()
            .filter_map(|run| {
                let start = (run.start as usize).max(from);
                let end = (run.end as usize).min(to);
                (start < end).then(|| {
                    ClassRun::new(
                        (start - from + row.prefix) as u32,
                        (end - from + row.prefix) as u32,
                        run.class,
                    )
                })
            })
            .collect()
    }

    /// The point: a byte of the cursor's line that Left and Right move by
    /// token.
    pub fn point(&self) -> usize {
        self.point
    }

    /// The tokens of a line: each run, and each word between them.
    fn tokens(&mut self, line: u64) -> Vec<(usize, usize, TokenClass)> {
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        let runs = self.line_runs(line).to_vec();
        let mut tokens = Vec::new();
        let mut at = 0usize;
        let words = |from: usize, to: usize, tokens: &mut Vec<(usize, usize, TokenClass)>| {
            let mut start = None;
            for (i, c) in text[from..to].char_indices() {
                let i = from + i;
                if c.is_whitespace() {
                    if let Some(s) = start.take() {
                        tokens.push((s, i, TokenClass::Plain));
                    }
                } else if start.is_none() {
                    start = Some(i);
                }
            }
            if let Some(s) = start {
                tokens.push((s, to, TokenClass::Plain));
            }
        };
        for run in &runs {
            let (start, end) = (run.start as usize, run.end as usize);
            if start > at {
                words(at, start, &mut tokens);
            }
            if text[start..end].trim().is_empty() {
                at = end;
                continue;
            }
            // A run's text without its outer spaces.
            let inner = &text[start..end];
            let lead = inner.len() - inner.trim_start().len();
            let trail = inner.len() - inner.trim_end().len();
            tokens.push((start + lead, end - trail, run.class));
            at = end;
        }
        if text.len() > at {
            words(at, text.len(), &mut tokens);
        }
        tokens
    }

    /// The token at the point, or the next one after it.
    fn token_here(&mut self) -> Option<(String, TokenClass)> {
        let line = self.cursor.line;
        let text = self.line_text(line)?;
        let tokens = self.tokens(line);
        let token = tokens
            .iter()
            .find(|(_, end, _)| *end > self.point)
            .or(tokens.last())?;
        Some((text[token.0..token.1].to_owned(), token.2))
    }

    /// The bytes of the cursor's line the point's token takes, for a
    /// frontend to mark it (in sighted mode, with an attribute).
    pub fn point_token(&mut self) -> Option<std::ops::Range<usize>> {
        if self.mode == ViewerMode::Hex || self.point == 0 {
            return None;
        }
        let tokens = self.tokens(self.cursor.line);
        tokens
            .iter()
            .find(|(_, end, _)| *end > self.point)
            .map(|(start, end, _)| *start..*end)
    }

    /// Moves the point to the token after the one it is on (or before, in
    /// the indent) and says it ("parse_args").
    pub(super) fn token_right(&mut self) -> Vec<Message> {
        let tokens = self.tokens(self.cursor.line);
        let text = self.line_text(self.cursor.line).unwrap_or_default();
        let current = tokens
            .iter()
            .find(|(_, end, _)| *end > self.point)
            .map_or(self.point, |(start, _, _)| (*start).max(self.point));
        match tokens.iter().find(|(start, _, _)| *start > current) {
            Some(&(start, end, _)) => {
                self.point = start;
                self.cursor_to_point();
                vec![Message::info(text[start..end].to_owned())]
            }
            None => vec![Message::info("End of line")],
        }
    }

    /// Moves the point to the token before and says it.
    pub(super) fn token_left(&mut self) -> Vec<Message> {
        let tokens = self.tokens(self.cursor.line);
        let text = self.line_text(self.cursor.line).unwrap_or_default();
        match tokens
            .iter()
            .rev()
            .find(|(start, _, _)| *start < self.point)
        {
            Some(&(start, end, _)) => {
                self.point = start;
                self.cursor_to_point();
                vec![Message::info(text[start..end].to_owned())]
            }
            None => vec![Message::info("Start of line")],
        }
    }

    /// Puts the cursor on the row of its line that holds the point.
    fn cursor_to_point(&mut self) {
        let line = self.cursor.line;
        let point = self.point;
        self.land_on_column(line, point);
        self.point = point;
    }

    /// Say Token Kind: "Function name: parse_args, line 40". Plain text
    /// says "Plain: word, line 3".
    pub fn token_kind(&mut self) -> Vec<Message> {
        let line = grouped(self.cursor.line + 1);
        match self.token_here() {
            Some((text, class)) => vec![Message::info(format!(
                "{}: {text}, line {line}",
                class.label()
            ))],
            None => vec![Message::info(format!("Line {line} is blank"))],
        }
    }

    /// Say Line by Kind: "Line 40: keyword, function name, string", in the
    /// order of first appearance, repeats collapsed, plain text and
    /// punctuation left out, at most four kinds then "and 2 more".
    pub fn line_kinds(&mut self) -> Vec<Message> {
        let line = self.cursor.line;
        let mut kinds: Vec<TokenClass> = Vec::new();
        for run in self.line_runs(line) {
            if matches!(run.class, TokenClass::Plain | TokenClass::Punctuation) {
                continue;
            }
            if !kinds.contains(&run.class) {
                kinds.push(run.class);
            }
        }
        let n = grouped(line + 1);
        if kinds.is_empty() {
            let blank = self.line_text(line).is_none_or(|t| t.trim().is_empty());
            let what = if blank { "blank" } else { "plain text" };
            return vec![Message::info(format!("Line {n}: {what}"))];
        }
        let words: Vec<&str> = kinds.iter().take(MOST_KINDS).map(|k| k.word()).collect();
        let mut said = format!("Line {n}: {}", words.join(", "));
        let more = kinds.len().saturating_sub(MOST_KINDS);
        if more > 0 {
            said.push_str(&format!(" and {more} more"));
        }
        vec![Message::info(said)]
    }

    fn symbols(&self) -> &[Symbol] {
        self.code.as_ref().map_or(&[], |c| c.symbols.as_slice())
    }

    /// The outline as a list: titled "Outline: 14 symbols in main.rs", one
    /// row a symbol, kind first, the focus on the one at or before the
    /// cursor; each row's id is its line, for [`Viewer::outline_go`].
    pub fn outline(&self) -> ListModel {
        let symbols = self.symbols();
        let title = format!(
            "Outline: {} in {}",
            count(symbols.len() as u64, "symbol", "symbols"),
            self.name
        );
        let rows = symbols
            .iter()
            .map(|s| ListRow::new(s.line.to_string(), s.row()))
            .collect();
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_empty_text("No symbols");
        list.set_help("Enter goes to the symbol's line; Escape returns to the file.");
        let here = self.cursor.line as usize;
        if let Some(at) = symbols.iter().rposition(|s| s.line <= here) {
            list.move_focus(diroxide_core::Move::To(at));
        }
        list
    }

    /// Goes to the line an outline row names, saying the row.
    pub fn outline_go(&mut self, id: &str) -> Vec<Message> {
        let Ok(line) = id.parse::<usize>() else {
            return Vec::new();
        };
        let row = self
            .symbols()
            .iter()
            .find(|s| s.line == line)
            .map(Symbol::row);
        self.go_line(line as u64);
        row.map(Message::info).into_iter().collect()
    }

    fn go_line(&mut self, line: u64) {
        self.cursor = RowPos { line, part: 0 };
        self.point = 0;
        self.keep_cursor_visible();
    }

    /// The next symbol below the cursor, said as its row: "function main,
    /// line 47".
    pub fn next_symbol(&mut self) -> Vec<Message> {
        let here = self.cursor.line as usize;
        match self.symbols().iter().find(|s| s.line > here).cloned() {
            Some(symbol) => {
                self.go_line(symbol.line as u64);
                vec![Message::info(symbol.row())]
            }
            None => vec![Message::info(self.no_symbol("below"))],
        }
    }

    /// The previous symbol above the cursor.
    pub fn previous_symbol(&mut self) -> Vec<Message> {
        let here = self.cursor.line as usize;
        match self.symbols().iter().rev().find(|s| s.line < here).cloned() {
            Some(symbol) => {
                self.go_line(symbol.line as u64);
                vec![Message::info(symbol.row())]
            }
            None => vec![Message::info(self.no_symbol("above"))],
        }
    }

    fn no_symbol(&self, side: &str) -> String {
        if self.code.is_none() {
            self.needs_small_file()
        } else {
            format!("No symbol {side}")
        }
    }

    /// The words when a command needs the file read whole.
    fn needs_small_file(&self) -> String {
        let caps = self.caps();
        format!("Needs a file under {} KiB", grouped(caps.max_bytes / 1024))
    }

    /// Whether a byte of a line is inside a string or a comment.
    fn quoted(&self, line: usize, byte: usize) -> bool {
        let runs = self.line_runs(line as u64);
        matches!(
            class_at(runs, byte),
            TokenClass::String | TokenClass::Comment
        )
    }

    /// Jumps from the bracket at or after the point to its partner, not
    /// counting brackets in strings and comments: "Matches ) on line 52".
    pub fn bracket_jump(&mut self) -> Vec<Message> {
        let Some(code) = self.code.as_ref() else {
            return vec![Message::info(self.needs_small_file())];
        };
        let lines = Arc::clone(&code.lines);
        let line = self.cursor.line as usize;
        let Some(text) = lines.get(line) else {
            return Vec::new();
        };
        let found = text
            .char_indices()
            .filter(|(i, _)| *i >= self.point || self.point >= text.len())
            .find(|(i, c)| "()[]{}".contains(*c) && !self.quoted(line, *i));
        let Some((at, c)) = found else {
            return vec![Message::info("No bracket on this line")];
        };
        match self.partner(&lines, (line, at, c)) {
            Some((pl, pb, pc)) => {
                self.cursor = RowPos {
                    line: pl as u64,
                    part: 0,
                };
                self.point = pb;
                self.cursor_to_point();
                vec![Message::info(format!(
                    "Matches {pc} on line {}",
                    grouped(pl as u64 + 1)
                ))]
            }
            None => vec![Message::info(format!(
                "No partner for {c} on line {}",
                grouped(line as u64 + 1)
            ))],
        }
    }

    fn partner(&self, lines: &[String], (line, at, c): Bracket) -> Option<Bracket> {
        let (open, close, forward) = match c {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            _ => ('{', '}', false),
        };
        let mut depth = 0usize;
        if forward {
            for (l, text) in lines.iter().enumerate().skip(line) {
                for (i, ch) in text.char_indices() {
                    if (l == line && i <= at) || self.quoted(l, i) {
                        continue;
                    }
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        if depth == 0 {
                            return Some((l, i, ch));
                        }
                        depth -= 1;
                    }
                }
            }
        } else {
            for l in (0..=line).rev() {
                let text = &lines[l];
                for (i, ch) in text.char_indices().rev() {
                    if (l == line && i >= at) || self.quoted(l, i) {
                        continue;
                    }
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        if depth == 0 {
                            return Some((l, i, ch));
                        }
                        depth -= 1;
                    }
                }
            }
        }
        None
    }

    /// Whether a line holds only a comment, or nothing.
    fn comment_or_blank(&self, line: usize, text: &str) -> (bool, bool) {
        let blank = text.trim().is_empty();
        let runs = self.line_runs(line as u64);
        let comment = !blank
            && text
                .char_indices()
                .all(|(i, c)| c.is_whitespace() || class_at(runs, i) == TokenClass::Comment);
        (comment, blank)
    }

    /// Skips the comment at or after the cursor: "Next code, line 58".
    pub fn skip_comment(&mut self) -> Vec<Message> {
        let Some(code) = self.code.as_ref() else {
            return vec![Message::info(self.needs_small_file())];
        };
        if code.runs.is_empty() {
            return vec![Message::info("Comments are known only in highlighted code")];
        }
        let lines = Arc::clone(&code.lines);
        let mut l = self.cursor.line as usize;
        let (on_comment, _) = self.comment_or_blank(l, &lines[l.min(lines.len() - 1)]);
        if !on_comment {
            // To the start of the next comment block.
            loop {
                l += 1;
                let Some(text) = lines.get(l) else {
                    return vec![Message::info("No comment below")];
                };
                if self.comment_or_blank(l, text).0 {
                    break;
                }
            }
        }
        loop {
            l += 1;
            let Some(text) = lines.get(l) else {
                return vec![Message::info("No code after the comment")];
            };
            let (comment, blank) = self.comment_or_blank(l, text);
            if !comment && !blank {
                break;
            }
        }
        self.go_line(l as u64);
        vec![Message::info(format!(
            "Next code, line {}",
            grouped(l as u64 + 1)
        ))]
    }

    /// The columns of a line's indent, `None` for a blank line.
    fn indent_of(text: &str) -> Option<usize> {
        if text.trim().is_empty() {
            return None;
        }
        Some(text.len() - text.trim_start().len())
    }

    /// The indent unit: the smallest indent in the file, or four.
    fn indent_unit(lines: &[String]) -> usize {
        lines
            .iter()
            .filter_map(|l| Self::indent_of(l))
            .filter(|n| *n > 0)
            .min()
            .unwrap_or(4)
    }

    /// Say Indent: "Indent level 2", "No indent", "Blank line".
    pub fn say_indent(&mut self) -> Vec<Message> {
        let line = self.cursor.line;
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        let unit = self
            .code
            .as_ref()
            .map_or(4, |c| Self::indent_unit(&c.lines));
        let said = match Self::indent_of(&text) {
            None => "Blank line".to_owned(),
            Some(0) => "No indent".to_owned(),
            Some(n) => format!("Indent level {}", n.div_ceil(unit.max(1))),
        };
        vec![Message::info(said)]
    }

    /// IndentNav's moves: the next or previous line at the same indent,
    /// the parent (the line before with less), the first child (the next
    /// line with more, before the block ends). Each says its line.
    pub fn indent_move(&mut self, mv: IndentMove) -> Vec<Message> {
        let Some(code) = self.code.as_ref() else {
            return vec![Message::info(self.needs_small_file())];
        };
        let lines = Arc::clone(&code.lines);
        let here = self.cursor.line as usize;
        let Some(level) = lines.get(here).and_then(|t| Self::indent_of(t)) else {
            return vec![Message::info("Blank line")];
        };
        let indent = |l: usize| lines.get(l).and_then(|t| Self::indent_of(t));
        let found = match mv {
            IndentMove::NextSame => (here + 1..lines.len())
                .take_while(|&l| indent(l).is_none_or(|i| i >= level))
                .find(|&l| indent(l) == Some(level)),
            IndentMove::PreviousSame => (0..here)
                .rev()
                .take_while(|&l| indent(l).is_none_or(|i| i >= level))
                .find(|&l| indent(l) == Some(level)),
            IndentMove::Parent => (0..here)
                .rev()
                .find(|&l| indent(l).is_some_and(|i| i < level)),
            IndentMove::Child => (here + 1..lines.len())
                .take_while(|&l| indent(l).is_none_or(|i| i > level))
                .find(|&l| indent(l).is_some_and(|i| i > level)),
        };
        match found {
            Some(l) => {
                self.go_line(l as u64);
                vec![Message::info(format!("Line {}", grouped(l as u64 + 1)))]
            }
            None => vec![Message::info(match mv {
                IndentMove::NextSame | IndentMove::PreviousSame => "No other line at this indent",
                IndentMove::Parent => "No parent: at the top level",
                IndentMove::Child => "No line inside this one",
            })],
        }
    }

    /// Say Context: the symbols around the cursor, innermost first: "In
    /// function main, in impl Config"; "At the top level" outside them.
    pub fn context(&mut self) -> Vec<Message> {
        let Some(code) = self.code.as_ref() else {
            return vec![Message::info(self.needs_small_file())];
        };
        let lines = Arc::clone(&code.lines);
        let symbols = code.symbols.clone();
        let here = self.cursor.line as usize;
        let mut around: Vec<(usize, &Symbol)> = Vec::new();
        for (i, symbol) in symbols.iter().enumerate() {
            if symbol.line > here {
                break;
            }
            let end = self.extent(&lines, &symbols, i);
            if here <= end {
                around.push((symbol.line, symbol));
            }
        }
        if around.is_empty() {
            return vec![Message::info("At the top level")];
        }
        let words: Vec<String> = around
            .iter()
            .rev()
            .map(|(_, s)| match s.kind {
                SymbolKind::Heading(_) => format!("in heading {}", s.name),
                _ => format!("in {} {}", s.kind.word(), s.name),
            })
            .collect();
        let mut said = words.join(", ");
        if let Some(first) = said.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        vec![Message::info(said)]
    }

    /// The last line a symbol covers: to its block's closing brace, the
    /// end of its indented block, or the next heading of its level.
    fn extent(&self, lines: &[String], symbols: &[Symbol], i: usize) -> usize {
        let symbol = &symbols[i];
        let last = lines.len().saturating_sub(1);
        match symbol.kind {
            SymbolKind::Heading(level) => symbols[i + 1..]
                .iter()
                .find(|s| matches!(s.kind, SymbolKind::Heading(l) if l <= level))
                .map_or(last, |s| s.line.saturating_sub(1)),
            SymbolKind::Table => symbols[i + 1..]
                .iter()
                .find(|s| s.kind == SymbolKind::Table)
                .map_or(last, |s| s.line.saturating_sub(1)),
            _ if self.language() == Some(Language::Python)
                || self.language() == Some(Language::Yaml) =>
            {
                let level = lines
                    .get(symbol.line)
                    .and_then(|t| Self::indent_of(t))
                    .unwrap_or(0);
                (symbol.line + 1..lines.len())
                    .find(|&l| Self::indent_of(&lines[l]).is_some_and(|n| n <= level))
                    .map_or(last, |l| l.saturating_sub(1))
            }
            _ => {
                // The first opening brace from the symbol's line, then its
                // partner.
                for (l, text) in lines.iter().enumerate().skip(symbol.line).take(50) {
                    if let Some(at) = text
                        .char_indices()
                        .find(|(b, c)| *c == '{' && !self.quoted(l, *b))
                        .map(|(b, _)| b)
                    {
                        return self
                            .partner(lines, (l, at, '{'))
                            .map_or(last, |(pl, _, _)| pl);
                    }
                    if l > symbol.line && text.trim_end().ends_with(';') {
                        break;
                    }
                }
                symbol.line
            }
        }
    }
}

/// A move by indentation (IndentNav's four).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndentMove {
    /// The next line at the same indent, within the block.
    NextSame,
    /// The previous line at the same indent, within the block.
    PreviousSame,
    /// The line before with less indent.
    Parent,
    /// The next line with more indent, inside this one.
    Child,
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, texts};
    use super::*;
    use crate::viewer::ViewerOptions;
    use diroxide_vfs::Registry;

    fn code(name: &str) -> Viewer {
        let place = fixture(&format!("code/{name}"));
        let (mut viewer, _) = Viewer::open(
            &Registry::with_local(),
            &place,
            None,
            ViewerOptions::default(),
        )
        .unwrap();
        viewer.set_size(80, 20);
        let said = viewer.set_highlighter(Session::new());
        assert!(said.is_empty(), "silent: {said:?}");
        assert!(viewer.wait_highlight().is_empty());
        viewer
    }

    #[test]
    fn the_token_and_the_line_are_said_by_kind() {
        let mut viewer = code("main.rs");
        assert_eq!(viewer.language(), Some(Language::Rust));
        assert!(viewer.is_highlighted());
        viewer.jump("21");
        assert_eq!(texts(&viewer.token_kind()), ["Keyword: pub, line 21"]);
        assert_eq!(texts(&viewer.right()), ["fn"]);
        assert_eq!(texts(&viewer.right()), ["parse_args"]);
        assert_eq!(
            texts(&viewer.token_kind()),
            ["Function name: parse_args, line 21"]
        );
        assert_eq!(texts(&viewer.left()), ["fn"]);
        assert_eq!(
            texts(&viewer.line_kinds()),
            ["Line 21: keyword, function name, variable, operator and 1 more"]
        );
        viewer.jump("53");
        assert_eq!(
            texts(&viewer.line_kinds()),
            ["Line 53: keyword, operator, number"]
        );
        viewer.jump("3");
        assert_eq!(texts(&viewer.line_kinds()), ["Line 3: blank"]);
    }

    #[test]
    fn many_kinds_stop_at_four_and_say_how_many_more() {
        let mut viewer = code("main.rs");
        viewer.jump("21");
        let said = texts(&viewer.line_kinds()).remove(0);
        let kinds = said.trim_start_matches("Line 21: ");
        if kinds.contains(" more") {
            assert_eq!(kinds.matches(", ").count(), 3, "{said}");
        }
    }

    #[test]
    fn the_outline_lists_symbols_kind_first_and_jumps() {
        let mut viewer = code("main.rs");
        let outline = viewer.outline();
        assert_eq!(outline.title(), "Outline: 7 symbols in main.rs");
        let rows: Vec<String> = outline.rows().iter().map(|r| r.label.clone()).collect();
        assert_eq!(rows[0], "type Config, line 8");
        assert_eq!(rows[6], "function main, line 47");
        assert!(rows.iter().all(|r| r.chars().count() <= 40));
        let id = outline.rows()[3].id.clone();
        assert_eq!(
            texts(&viewer.outline_go(&id)),
            ["function parse_args, line 21"]
        );
        assert_eq!(viewer.cursor().line, 20);
        assert_eq!(viewer.outline().focus_index(), Some(3));
        assert_eq!(texts(&viewer.next_symbol()), ["module greeting, line 37"]);
        assert_eq!(
            texts(&viewer.previous_symbol()),
            ["function parse_args, line 21"]
        );
        viewer.jump("50");
        assert_eq!(texts(&viewer.next_symbol()), ["No symbol below"]);
    }

    #[test]
    fn a_bracket_jumps_to_its_partner_past_strings() {
        let mut viewer = code("main.rs");
        viewer.jump("19");
        assert_eq!(texts(&viewer.bracket_jump()), ["Matches } on line 35"]);
        assert_eq!(viewer.cursor().line, 34);
        assert_eq!(texts(&viewer.bracket_jump()), ["Matches { on line 19"]);
        viewer.jump("40");
        assert_eq!(
            texts(&viewer.bracket_jump()),
            ["Matches ) on line 40"],
            "the braces in the string are skipped"
        );
        viewer.jump("3");
        assert_eq!(texts(&viewer.bracket_jump()), ["No bracket on this line"]);
    }

    #[test]
    fn skip_comment_lands_on_the_next_code() {
        let mut viewer = code("main.rs");
        viewer.jump("44");
        assert_eq!(texts(&viewer.skip_comment()), ["Next code, line 47"]);
        viewer.jump("38");
        assert_eq!(texts(&viewer.skip_comment()), ["Next code, line 39"]);
    }

    #[test]
    fn indentation_is_said_and_walked() {
        let mut viewer = code("sample.py");
        viewer.jump("17");
        assert_eq!(texts(&viewer.say_indent()), ["Indent level 2"]);
        viewer.jump("9");
        assert_eq!(texts(&viewer.say_indent()), ["No indent"]);
        assert_eq!(texts(&viewer.indent_move(IndentMove::Child)), ["Line 10"]);
        assert_eq!(
            texts(&viewer.indent_move(IndentMove::NextSame)),
            ["Line 12"]
        );
        assert_eq!(texts(&viewer.indent_move(IndentMove::Parent)), ["Line 9"]);
        viewer.jump("8");
        assert_eq!(texts(&viewer.say_indent()), ["Blank line"]);
    }

    #[test]
    fn the_context_names_what_holds_the_cursor() {
        let mut viewer = code("main.rs");
        viewer.jump("25");
        assert_eq!(
            texts(&viewer.context()),
            ["In function parse_args, in impl Config"]
        );
        viewer.jump("4");
        assert_eq!(texts(&viewer.context()), ["At the top level"]);
        let mut python = code("sample.py");
        python.jump("17");
        assert_eq!(
            texts(&python.context()),
            ["In function add, in type Counter"]
        );
        let mut markdown = code("report.md");
        markdown.jump("20");
        assert_eq!(
            texts(&markdown.context()),
            ["In heading Options, in heading Usage, in heading The report"]
        );
    }

    #[test]
    fn structured_files_say_their_validity_when_they_open() {
        let place = fixture("code/bad.json");
        let (mut viewer, _) = Viewer::open(
            &Registry::with_local(),
            &place,
            None,
            ViewerOptions::default(),
        )
        .unwrap();
        let said = texts(&viewer.set_highlighter(Session::new()));
        assert_eq!(said, ["JSON invalid at line 4, column 1"]);
        let mut json = super::super::tests::open("code/data.json").0;
        let said = texts(&json.set_highlighter(Session::new()));
        assert_eq!(said.len(), 1);
        assert!(
            said[0].starts_with("JSON, ") && said[0].ends_with(", valid"),
            "{said:?}"
        );
    }

    #[test]
    fn a_large_file_says_once_why_it_is_plain() {
        let mut big = String::from("fn main() {}\n");
        while big.len() < 600 * 1024 {
            big.push_str("// filler line for the cap\n");
        }
        let place = diroxide_core::Place::parse(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .join("big.rs")
            .unwrap();
        let (mut viewer, _) = Viewer::from_source(
            place,
            super::super::ViewerSource::bytes(big.into_bytes()),
            ViewerOptions::default(),
        )
        .unwrap();
        assert_eq!(
            texts(&viewer.set_highlighter(Session::new())),
            ["Highlighting off: over 512 KiB"]
        );
        assert_eq!(
            texts(&viewer.bracket_jump()),
            ["Needs a file under 512 KiB"]
        );
        assert_eq!(texts(&viewer.token_kind()), ["Plain: fn, line 1"]);
    }
}
