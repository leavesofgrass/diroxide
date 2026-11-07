//! Rich Text Format read into rows (alpha 4, R30 commit 40b): carta's
//! RTF reader drops every table (spike C2, answer 119), so RTF has a
//! reader of its own, carried from textweaver's `rtf.rs` under the
//! owner's relicensing (answers 3 and 41): its parser's rules, written
//! here anew onto the row builder, its document model left behind.
//!
//! The parser walks the bytes once with a stack of groups of its own, so
//! no nesting can overflow the thread's stack:
//!
//! - **Groups** nest at most [`MAX_NESTING`] deep; deeper groups are read
//!   in the enclosing group's state and the view says so once.
//! - **`\bin`** data is skipped, never read, never past the end.
//! - **`\uc`** skip counts are capped at [`MAX_UNICODE_SKIP`]; control
//!   words are cut at [`MAX_WORD`] letters.
//! - **Text encoding**: `\ansicpg`, `\mac` and each font's `\fcharset` or
//!   `\cpg` choose the code page for `\'hh` bytes and raw 8-bit text,
//!   decoded by `encoding_rs` (double-byte pages included); `\u`
//!   characters, surrogate pairs and their fallbacks are handled.
//! - **Destinations** this reader does not know (`\*`), headers, footers,
//!   color, list and file tables and picture data are skipped; the font
//!   table, the style sheet and the `\info` title and author are read.
//! - **Structure**: headings from styles the style sheet names `heading
//!   1` to `heading 9` or `Title`, or from `\outlinelevel`; lists from
//!   `\ls` and `\ilvl` with the label the writer rendered (`\listtext`);
//!   **tables** from `\trowd`, `\intbl`, `\cell` and `\row`, a `\trhdr`
//!   row the header row (else the first); footnotes as `[1]` and a
//!   "Footnotes" heading at the end, as the Markdown reader does;
//!   `HYPERLINK` fields as links; hidden text (`\v`) and deleted
//!   revisions left out; a picture's description (`wzDescription`) read.
//!
//! Every row's text goes through the row builder, which shows hidden
//! characters as question marks.

use std::collections::HashMap;

use encoding_rs::{Encoding, WINDOWS_1252};

use super::xmldepth::NESTING_NOTE;
use super::{DocBuilder, DocError, DocKind, DocView};
use crate::words::clean_text;

/// How deep groups nest before deeper ones are read in their parent's
/// state.
pub const MAX_NESTING: usize = super::xmldepth::MAX_NESTING;

/// The largest `\uc` skip count honored.
pub const MAX_UNICODE_SKIP: usize = 8;

/// The longest control word read; longer runs of letters are cut.
pub const MAX_WORD: usize = 32;

/// The kind's name, as the announcement says it.
pub const KIND: &str = "RTF";

/// Where a group's text goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    Text,
    Skip,
    FontTable,
    StyleSheet,
    Info,
    InfoTitle,
    InfoAuthor,
    Footnote,
    FieldInst,
    ListText,
    Pict,
    ShapeName,
    ShapeValue,
}

/// A group's state.
#[derive(Clone, Debug)]
struct Group {
    dest: Dest,
    /// `\*` seen: unless the next word names a known destination, the
    /// group is skipped.
    star: bool,
    uc: usize,
    font: Option<i32>,
    hidden: bool,
    deleted: bool,
    style: i32,
    in_table: bool,
    list: Option<i32>,
    level: u8,
    outline: Option<u8>,
    field: Option<usize>,
    link: Option<usize>,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            dest: Dest::Text,
            star: false,
            uc: 1,
            font: None,
            hidden: false,
            deleted: false,
            style: 0,
            in_table: false,
            list: None,
            level: 0,
            outline: None,
            field: None,
            link: None,
        }
    }
}

/// A piece of a paragraph or a cell.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text { text: String, link: Option<usize> },
    Break,
    Note(String),
    Picture(String),
}

#[derive(Clone, Debug)]
enum Block {
    Para {
        pieces: Vec<Piece>,
        heading: Option<u8>,
        /// The list's depth from 1 and the label the writer rendered.
        list: Option<(u8, Option<String>)>,
    },
    Row {
        cells: Vec<Vec<Piece>>,
        header: bool,
    },
}

/// What the parser found.
#[derive(Debug, Default)]
struct Found {
    blocks: Vec<Block>,
    links: Vec<String>,
    title: String,
    author: String,
    flattened: bool,
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
    stack: Vec<Group>,
    /// Groups opened past the nesting limit and not yet closed.
    over: usize,
    ansi: &'static Encoding,
    default_font: Option<i32>,
    fonts: HashMap<i32, &'static Encoding>,
    font_def: Option<i32>,
    styles: HashMap<i32, String>,
    style_def: Option<i32>,
    entry: String,
    pending: Vec<u8>,
    pending_encoding: &'static Encoding,
    high: Option<u16>,
    skip: usize,
    pieces: Vec<Piece>,
    label: Option<String>,
    cells: Vec<Vec<Piece>>,
    header_row: bool,
    footnote: String,
    list_text: String,
    fields: Vec<String>,
    shape_name: String,
    shape_value: String,
    picture: Option<String>,
    found: Found,
}

/// Whether bytes start as an RTF file does, after white space and a
/// byte-order mark.
pub fn is_rtf(bytes: &[u8]) -> bool {
    body(bytes).is_some()
}

fn body(bytes: &[u8]) -> Option<&[u8]> {
    let mut rest = bytes;
    loop {
        let start = rest.iter().position(|b| !b.is_ascii_whitespace())?;
        rest = &rest[start..];
        match rest.strip_prefix(b"\xef\xbb\xbf") {
            Some(after) => rest = after,
            None => break,
        }
    }
    rest.starts_with(b"{\\rtf").then_some(rest)
}

/// Reads an RTF file's bytes into rows. Unguarded: the quick view reads
/// through [`super::read`], which runs it under the guard.
pub fn read(bytes: &[u8]) -> Result<DocView, DocError> {
    let src = body(bytes).ok_or_else(|| DocError::NotValid(KIND.to_owned()))?;
    let found = parse(src);
    Ok(write(found))
}

fn parse(src: &[u8]) -> Found {
    let mut p = Parser {
        src,
        at: 0,
        stack: vec![Group::default()],
        over: 0,
        ansi: WINDOWS_1252,
        default_font: None,
        fonts: HashMap::new(),
        font_def: None,
        styles: HashMap::new(),
        style_def: None,
        entry: String::new(),
        pending: Vec::new(),
        pending_encoding: WINDOWS_1252,
        high: None,
        skip: 0,
        pieces: Vec::new(),
        label: None,
        cells: Vec::new(),
        header_row: false,
        footnote: String::new(),
        list_text: String::new(),
        fields: Vec::new(),
        shape_name: String::new(),
        shape_value: String::new(),
        picture: None,
        found: Found::default(),
    };
    p.walk();
    p.flush();
    p.end_paragraph();
    p.end_row();
    p.found
}

fn words_of(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Parser<'_> {
    fn group(&self) -> &Group {
        &self.stack[self.stack.len() - 1]
    }

    fn group_mut(&mut self) -> &mut Group {
        let last = self.stack.len() - 1;
        &mut self.stack[last]
    }

    fn walk(&mut self) {
        while self.at < self.src.len() {
            let b = self.src[self.at];
            match b {
                b'{' => {
                    self.at += 1;
                    self.flush();
                    self.skip = 0;
                    if self.stack.len() > MAX_NESTING {
                        self.over += 1;
                        self.found.flattened = true;
                    } else {
                        let mut g = self.group().clone();
                        g.star = false;
                        self.stack.push(g);
                    }
                }
                b'}' => {
                    self.at += 1;
                    self.flush();
                    self.skip = 0;
                    if self.over > 0 {
                        self.over -= 1;
                    } else if self.stack.len() > 1 {
                        self.close_group();
                        if self.stack.len() == 1 {
                            // The document's own group ended.
                            return;
                        }
                    }
                }
                b'\\' => self.control(),
                b'\r' | b'\n' => self.at += 1,
                b'\t' => {
                    self.at += 1;
                    self.flush();
                    self.text(" ");
                }
                _ => {
                    self.at += 1;
                    if self.skip > 0 {
                        self.skip -= 1;
                    } else {
                        self.byte(b);
                    }
                }
            }
        }
    }

    /// A text byte, decoded later in the current font's code page.
    fn byte(&mut self, b: u8) {
        let encoding = self.encoding();
        if encoding != self.pending_encoding {
            self.flush();
            self.pending_encoding = encoding;
        }
        self.pending.push(b);
    }

    fn encoding(&self) -> &'static Encoding {
        self.group()
            .font
            .or(self.default_font)
            .and_then(|f| self.fonts.get(&f).copied())
            .unwrap_or(self.ansi)
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.pending);
        let (decoded, _) = self.pending_encoding.decode_without_bom_handling(&bytes);
        let decoded = decoded.into_owned();
        self.text(&decoded);
    }

    /// Text for the current destination.
    fn text(&mut self, s: &str) {
        let g = self.group().clone();
        match g.dest {
            Dest::Text => {
                if g.hidden || g.deleted {
                    return;
                }
                match self.pieces.last_mut() {
                    Some(Piece::Text { text, link }) if *link == g.link => text.push_str(s),
                    _ => self.pieces.push(Piece::Text {
                        text: s.to_owned(),
                        link: g.link,
                    }),
                }
            }
            Dest::Footnote => {
                if !g.hidden && !g.deleted {
                    self.footnote.push_str(s);
                }
            }
            Dest::FontTable | Dest::StyleSheet => {
                for c in s.chars() {
                    if c == ';' {
                        self.end_entry(g.dest);
                    } else if self.entry.len() < 1_024 {
                        self.entry.push(c);
                    }
                }
            }
            Dest::InfoTitle if self.found.title.len() < 4_096 => self.found.title.push_str(s),
            Dest::InfoAuthor if self.found.author.len() < 4_096 => self.found.author.push_str(s),
            Dest::FieldInst => {
                if let Some(f) = g.field.and_then(|n| self.fields.get_mut(n))
                    && f.len() < 4_096
                {
                    f.push_str(s);
                }
            }
            Dest::ListText if self.list_text.len() < 256 => self.list_text.push_str(s),
            Dest::ShapeName if self.shape_name.len() < 256 => self.shape_name.push_str(s),
            Dest::ShapeValue if self.shape_value.len() < 4_096 => self.shape_value.push_str(s),
            _ => {}
        }
    }

    /// The end of a style sheet entry, at its `;`.
    fn end_entry(&mut self, dest: Dest) {
        let name = words_of(&std::mem::take(&mut self.entry));
        if dest == Dest::StyleSheet
            && let Some(style) = self.style_def.take()
            && self.styles.len() < 10_000
        {
            self.styles.insert(style, name.to_lowercase());
        }
    }

    fn close_group(&mut self) {
        let Some(g) = self.stack.pop() else {
            return;
        };
        let parent = self.group().dest;
        match g.dest {
            Dest::Footnote if parent != Dest::Footnote => {
                let note = words_of(&std::mem::take(&mut self.footnote));
                if !note.is_empty() && parent == Dest::Text {
                    self.pieces.push(Piece::Note(note));
                }
            }
            Dest::ListText if parent != Dest::ListText => {
                let label = words_of(&std::mem::take(&mut self.list_text));
                if !label.is_empty() {
                    self.label = Some(clean_text(&label, 16));
                }
            }
            Dest::StyleSheet if !self.entry.trim().is_empty() => self.end_entry(g.dest),
            Dest::ShapeValue if parent != Dest::ShapeValue => {
                if self.shape_name.trim() == "wzDescription" {
                    let alt = words_of(&self.shape_value);
                    if !alt.is_empty() {
                        self.picture = Some(alt);
                    }
                }
            }
            Dest::Pict if parent != Dest::Pict => {
                if let Some(alt) = self.picture.take()
                    && parent == Dest::Text
                {
                    self.pieces.push(Piece::Picture(alt));
                }
            }
            _ => {}
        }
        if g.dest == Dest::FontTable && parent != Dest::FontTable {
            self.entry.clear();
            self.font_def = None;
        }
        if g.dest == Dest::ShapeValue && parent == Dest::Pict {
            self.shape_name.clear();
            self.shape_value.clear();
        }
    }

    fn control(&mut self) {
        self.at += 1;
        let Some(&c) = self.src.get(self.at) else {
            return;
        };
        if !c.is_ascii_alphabetic() {
            self.at += 1;
            self.symbol(c);
            return;
        }
        let start = self.at;
        while self.at < self.src.len() && self.src[self.at].is_ascii_alphabetic() {
            self.at += 1;
        }
        let end = self.at.min(start + MAX_WORD);
        let word = std::str::from_utf8(&self.src[start..end])
            .unwrap_or("")
            .to_owned();
        let negative = self.src.get(self.at) == Some(&b'-')
            && self.src.get(self.at + 1).is_some_and(u8::is_ascii_digit);
        if negative {
            self.at += 1;
        }
        let mut digits = 0;
        let mut n: i64 = 0;
        while self.at < self.src.len() && self.src[self.at].is_ascii_digit() {
            if digits < 10 {
                n = n * 10 + i64::from(self.src[self.at] - b'0');
            }
            digits += 1;
            self.at += 1;
        }
        let param = (digits > 0).then(|| {
            let n = if negative { -n } else { n };
            i32::try_from(n.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
        });
        if self.src.get(self.at) == Some(&b' ') {
            self.at += 1;
        }
        if word == "bin" {
            self.flush();
            let n = usize::try_from(param.unwrap_or(0).max(0)).unwrap_or(0);
            self.at = self.at.saturating_add(n).min(self.src.len());
            return;
        }
        if word == "u" {
            self.flush();
            self.unicode(param.unwrap_or(0));
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        self.flush();
        self.word(&word, param);
    }

    fn unicode(&mut self, n: i32) {
        let v = if n < 0 { n + 65_536 } else { n };
        let v = u32::try_from(v).unwrap_or(0xFFFD);
        self.skip = self.group().uc;
        if (0xD800..0xDC00).contains(&v) {
            self.high = u16::try_from(v).ok();
            return;
        }
        let c = if (0xDC00..0xE000).contains(&v) {
            self.high.take().and_then(|h| {
                char::from_u32(0x10000 + ((u32::from(h) - 0xD800) << 10) + (v - 0xDC00))
            })
        } else {
            self.high = None;
            char::from_u32(v)
        };
        if let Some(c) = c.filter(|c| !c.is_control()) {
            let mut buf = [0u8; 4];
            self.text(c.encode_utf8(&mut buf));
        }
    }

    fn symbol(&mut self, c: u8) {
        if c == b'\'' {
            let hex = self
                .src
                .get(self.at..self.at + 2)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                self.at += 2;
                if self.skip > 0 {
                    self.skip -= 1;
                } else {
                    self.byte(b);
                }
            }
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        match c {
            b'\\' | b'{' | b'}' => self.byte(c),
            b'*' => self.group_mut().star = true,
            b'~' => {
                self.flush();
                self.text("\u{a0}");
            }
            b'_' => {
                self.flush();
                self.text("-");
            }
            b'\r' | b'\n' => {
                self.flush();
                self.word("par", None);
            }
            b'\t' => {
                self.flush();
                self.text(" ");
            }
            _ => {}
        }
    }

    fn set_dest(&mut self, dest: Dest) {
        self.group_mut().dest = dest;
    }

    fn word(&mut self, word: &str, param: Option<i32>) {
        let star = std::mem::replace(&mut self.group_mut().star, false);
        let on = param != Some(0);
        let dest = self.group().dest;
        match word {
            "fonttbl" => return self.set_dest(Dest::FontTable),
            "stylesheet" => {
                self.style_def = Some(0);
                return self.set_dest(Dest::StyleSheet);
            }
            "info" => return self.set_dest(Dest::Info),
            "title" if dest == Dest::Info => return self.set_dest(Dest::InfoTitle),
            "author" if dest == Dest::Info => return self.set_dest(Dest::InfoAuthor),
            "footnote" if matches!(dest, Dest::Text | Dest::Footnote) => {
                return self.set_dest(Dest::Footnote);
            }
            "field" => {
                if self.fields.len() < 100_000 {
                    self.fields.push(String::new());
                    self.group_mut().field = Some(self.fields.len() - 1);
                }
                return;
            }
            "fldinst" => return self.set_dest(Dest::FieldInst),
            "fldrslt" => {
                let link = self
                    .group()
                    .field
                    .and_then(|f| self.fields.get(f))
                    .and_then(|inst| hyperlink(inst));
                if let Some(target) = link
                    && self.found.links.len() < 100_000
                {
                    self.found.links.push(target);
                    self.group_mut().link = Some(self.found.links.len() - 1);
                }
                return self.set_dest(if dest == Dest::FieldInst {
                    Dest::Skip
                } else {
                    dest
                });
            }
            "listtext" | "pntext" if dest == Dest::Text => return self.set_dest(Dest::ListText),
            "pict" | "shpinst" => return self.set_dest(Dest::Pict),
            "shppict" | "shp" => return,
            "sn" if dest == Dest::Pict => return self.set_dest(Dest::ShapeName),
            "sv" if dest == Dest::Pict => return self.set_dest(Dest::ShapeValue),
            "sp" | "picprop" if dest == Dest::Pict => return,
            "colortbl" | "header" | "headerl" | "headerr" | "headerf" | "footer" | "footerl"
            | "footerr" | "footerf" | "filetbl" | "listtable" | "listoverridetable" | "rsidtbl"
            | "generator" | "xmlnstbl" | "themedata" | "colorschememapping" | "datastore"
            | "latentstyles" | "pgdsctbl" | "objdata" | "objclass" | "objname" | "nonshppict"
            | "shprslt" | "pn" | "pntxta" | "pntxtb" | "annotation" | "atnid" | "atnauthor"
            | "atndate" | "atnref" | "atrfstart" | "atrfend" | "txe" | "xe" | "tc" | "tcn"
            | "bkmkstart" | "bkmkend" | "docvar" | "private" | "userprops" | "fontemb"
            | "fontfile" | "falt" | "panose" | "template" | "mmathPr" | "wgrffmtfilter"
            | "nesttableprops" | "ftnsep" | "ftnsepc" | "ftncn" | "aftnsep" | "aftnsepc"
            | "aftncn" | "comment" | "doccomm" | "subject" | "keywords" | "operator"
            | "company" | "manager" | "category" | "hlinkbase" | "revtbl" => {
                return self.set_dest(Dest::Skip);
            }
            _ if star => return self.set_dest(Dest::Skip),
            _ => {}
        }
        match dest {
            Dest::FontTable => match word {
                "f" => {
                    self.font_def = param;
                    self.entry.clear();
                }
                "fcharset" => {
                    if let (Some(f), Some(set)) = (self.font_def, param)
                        && let Some(page) = charset_page(set)
                        && self.fonts.len() < 10_000
                    {
                        self.fonts.insert(f, code_page(page));
                    }
                }
                "cpg" => {
                    if let (Some(f), Some(page)) = (self.font_def, param)
                        && self.fonts.len() < 10_000
                    {
                        self.fonts.insert(f, code_page(page));
                    }
                }
                _ => {}
            },
            Dest::StyleSheet => match word {
                "s" => {
                    self.style_def = param;
                    self.entry.clear();
                }
                "cs" | "ds" | "ts" => {
                    self.style_def = None;
                    self.entry.clear();
                }
                _ => {}
            },
            _ => self.state_word(word, param, on),
        }
    }

    /// Control words that set state or write text.
    fn state_word(&mut self, word: &str, param: Option<i32>, on: bool) {
        match word {
            "ansi" | "pc" | "pca" => self.ansi = WINDOWS_1252,
            "mac" => self.ansi = code_page(10_000),
            "ansicpg" => {
                if let Some(page) = param {
                    self.ansi = code_page(page);
                }
            }
            "deff" => self.default_font = param,
            "f" => self.group_mut().font = param,
            "uc" => {
                self.group_mut().uc = usize::try_from(param.unwrap_or(1).max(0))
                    .map_or(1, |n| n.min(MAX_UNICODE_SKIP));
            }
            "plain" => {
                let font = self.default_font;
                let g = self.group_mut();
                g.hidden = false;
                g.deleted = false;
                g.font = font;
            }
            "v" => self.group_mut().hidden = on,
            "deleted" => self.group_mut().deleted = on,
            "pard" => {
                let g = self.group_mut();
                g.style = 0;
                g.in_table = false;
                g.list = None;
                g.level = 0;
                g.outline = None;
            }
            "s" => self.group_mut().style = param.unwrap_or(0),
            "intbl" => self.group_mut().in_table = on,
            "ls" => self.group_mut().list = param.filter(|&n| n > 0),
            "ilvl" => {
                self.group_mut().level = u8::try_from(param.unwrap_or(0).clamp(0, 8)).unwrap_or(0);
            }
            "outlinelevel" => {
                self.group_mut().outline = param
                    .filter(|n| (0..9).contains(n))
                    .and_then(|n| u8::try_from(n + 1).ok())
                    .map(|n| n.min(6));
            }
            "par" | "sect" => match self.group().dest {
                Dest::Text => self.end_paragraph(),
                Dest::Footnote => self.footnote.push(' '),
                _ => {}
            },
            "line" => match self.group().dest {
                Dest::Text => self.pieces.push(Piece::Break),
                _ => self.text(" "),
            },
            "cell" => {
                if self.group().dest == Dest::Text {
                    self.end_cell();
                } else {
                    self.text(" ");
                }
            }
            "row" => {
                if self.group().dest == Dest::Text {
                    self.end_row();
                }
            }
            "nestcell" | "nestrow" | "tab" => self.text(" "),
            "trowd" => self.header_row = false,
            "trhdr" => self.header_row = true,
            "emdash" => self.text("\u{2014}"),
            "endash" => self.text("\u{2013}"),
            "emspace" | "enspace" | "qmspace" => self.text(" "),
            "bullet" => self.text("\u{2022}"),
            "lquote" => self.text("\u{2018}"),
            "rquote" => self.text("\u{2019}"),
            "ldblquote" => self.text("\u{201c}"),
            "rdblquote" => self.text("\u{201d}"),
            _ => {}
        }
    }

    /// The heading level or list place of the paragraph that just ended.
    fn paragraph_kind(&mut self) -> (Option<u8>, Option<(u8, Option<String>)>) {
        let g = self.group().clone();
        let heading = g.outline.or_else(|| {
            let name = self.styles.get(&g.style)?;
            if let Some(n) = name.strip_prefix("heading ")
                && let Ok(n) = n.trim().parse::<u8>()
            {
                return Some(n.clamp(1, 6));
            }
            (name == "title").then_some(1)
        });
        let label = self.label.take();
        let list = (heading.is_none() && (g.list.is_some() || label.is_some()))
            .then(|| (g.level + 1, label));
        (heading, list)
    }

    fn end_paragraph(&mut self) {
        if self.group().in_table {
            // A paragraph inside a cell: the cell goes on after a space.
            self.label = None;
            self.pieces.push(Piece::Text {
                text: " ".to_owned(),
                link: None,
            });
            return;
        }
        if !self.cells.is_empty() {
            self.end_row();
        }
        let (heading, list) = self.paragraph_kind();
        let pieces = std::mem::take(&mut self.pieces);
        if pieces.is_empty() {
            return;
        }
        self.found.blocks.push(Block::Para {
            pieces,
            heading,
            list,
        });
    }

    fn end_cell(&mut self) {
        self.label = None;
        let pieces = std::mem::take(&mut self.pieces);
        if self.cells.len() < 1_000 {
            self.cells.push(pieces);
        }
    }

    fn end_row(&mut self) {
        if !self.pieces.is_empty() && !self.cells.is_empty() {
            // Text after a row's last `\cell` belongs to that cell.
            let pieces = std::mem::take(&mut self.pieces);
            if let Some(last) = self.cells.last_mut() {
                last.push(Piece::Text {
                    text: " ".to_owned(),
                    link: None,
                });
                last.extend(pieces);
            }
        }
        let cells = std::mem::take(&mut self.cells);
        if !cells.is_empty() {
            self.found.blocks.push(Block::Row {
                cells,
                header: self.header_row,
            });
        }
    }
}

/// The target of a `HYPERLINK "address" \l "anchor"` field instruction.
fn hyperlink(instruction: &str) -> Option<String> {
    let rest = instruction.trim_start().strip_prefix("HYPERLINK")?;
    let mut address = None;
    let mut anchor = None;
    let mut next_is_anchor = false;
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => next_is_anchor = chars.next() == Some('l'),
            '"' => {
                let quoted: String = chars.by_ref().take_while(|&c| c != '"').collect();
                if next_is_anchor {
                    anchor = Some(quoted);
                } else if address.is_none() {
                    address = Some(quoted);
                }
                next_is_anchor = false;
            }
            c if !c.is_whitespace() && address.is_none() && !next_is_anchor => {
                let mut bare = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() {
                        break;
                    }
                    bare.push(n);
                    chars.next();
                }
                address = Some(bare);
            }
            _ => {}
        }
    }
    let address = address.filter(|u| !u.trim().is_empty());
    let anchor = anchor.filter(|a| !a.trim().is_empty());
    match (address, anchor) {
        (Some(u), Some(a)) => Some(format!("{u}#{a}")),
        (Some(u), None) => Some(u),
        (None, Some(a)) => Some(format!("#{a}")),
        (None, None) => None,
    }
}

/// The Windows code page an RTF `\fcharset` names; `None` for the
/// document's own.
fn charset_page(set: i32) -> Option<i32> {
    Some(match set {
        2 => 1252,
        77 => 10_000,
        78 | 128 => 932,
        129 | 130 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return None,
    })
}

/// The encoding of a Windows code page; Windows-1252 for one
/// `encoding_rs` does not have.
fn code_page(page: i32) -> &'static Encoding {
    let label = match page {
        866 => "ibm866".to_owned(),
        874 => "windows-874".to_owned(),
        932 => "shift_jis".to_owned(),
        936 => "gbk".to_owned(),
        949 => "euc-kr".to_owned(),
        950 => "big5".to_owned(),
        1250..=1258 => format!("windows-{page}"),
        10_000 => "macintosh".to_owned(),
        10_007 => "x-mac-cyrillic".to_owned(),
        20_866 => "koi8-r".to_owned(),
        21_866 => "koi8-u".to_owned(),
        28_592..=28_606 => format!("iso-8859-{}", page - 28_590),
        51_932 => "euc-jp".to_owned(),
        54_936 => "gb18030".to_owned(),
        65_001 => "utf-8".to_owned(),
        _ => return WINDOWS_1252,
    };
    Encoding::for_label(label.as_bytes()).unwrap_or(WINDOWS_1252)
}

/// A list label that is only a bullet: said as nothing, as the other
/// readers say a plain bullet.
fn is_bullet(label: &str) -> bool {
    label.chars().count() == 1 && !label.chars().any(char::is_alphanumeric)
}

/// Writes what the parser found onto the row builder.
fn write(found: Found) -> DocView {
    let mut b = DocBuilder::new(DocKind::Format(KIND.to_owned()));
    let title = words_of(&found.title);
    if !title.is_empty() {
        b.set_title(clean_text(&title, 200));
    }
    let mut notes: Vec<String> = Vec::new();
    let mut in_table = false;
    let mut first_row = true;
    for block in &found.blocks {
        match block {
            Block::Para {
                pieces,
                heading,
                list,
            } => {
                if in_table {
                    b.end_table();
                    in_table = false;
                }
                match (heading, list) {
                    (Some(level), _) => b.heading(*level),
                    (None, Some((depth, label))) => b.list_item(
                        (*depth).clamp(1, 9),
                        label.clone().filter(|l| !is_bullet(l)),
                    ),
                    (None, None) => b.paragraph(),
                }
                pieces_onto(&mut b, pieces, &found.links, &mut notes, false);
                b.end();
            }
            Block::Row { cells, header } => {
                if !in_table {
                    b.start_table();
                    in_table = true;
                    first_row = true;
                }
                // A row the writer marked as the header, else the first.
                b.start_table_row(*header || first_row);
                first_row = false;
                for cell in cells {
                    b.start_cell();
                    pieces_onto(&mut b, cell, &found.links, &mut notes, true);
                    b.end_cell();
                }
                b.end_table_row();
            }
        }
    }
    if in_table {
        b.end_table();
    }
    if !notes.is_empty() {
        b.heading(2);
        b.text("Footnotes");
        b.end();
        for (n, note) in notes.iter().enumerate() {
            b.paragraph();
            b.text(&format!("[{}] {note}", n + 1));
            b.end();
        }
    }
    if found.flattened {
        b.note(NESTING_NOTE);
    }
    let mut view = b.finish();
    if view.title.is_none() {
        view.title = view.outline().first().map(|heading| heading.name.clone());
    }
    view
}

fn pieces_onto(
    b: &mut DocBuilder,
    pieces: &[Piece],
    links: &[String],
    notes: &mut Vec<String>,
    in_cell: bool,
) {
    for piece in pieces {
        match piece {
            Piece::Text { text, link } => match link.and_then(|l| links.get(l)) {
                Some(target) => {
                    b.start_link(target.clone());
                    b.text(text);
                    b.end_link();
                }
                None => b.text(text),
            },
            Piece::Break if in_cell => b.space(),
            Piece::Break => b.line_break(),
            Piece::Note(text) => {
                notes.push(text.clone());
                b.text(&format!("[{}]", notes.len()));
            }
            Piece::Picture(alt) => {
                b.space();
                b.text(alt);
                b.space();
            }
        }
    }
}

#[cfg(test)]
mod tests;
