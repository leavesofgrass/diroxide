//! The Markdown reader: CommonMark with GitHub's tables, task lists,
//! strikethrough, footnotes and alerts, Obsidian's callouts and wiki
//! links, math and front matter, parsed by `pulldown-cmark` into a
//! [`DocView`].
//!
//! textweaver's reading rules, carried as rules and not as code (answer
//! 41; research/alpha2-plan.md part 1.7), and its test cases shared as
//! fixtures (`fixtures/docs/markdown/`):
//!
//! - front matter (YAML between `---` lines, TOML between `+++` lines) is
//!   not text: its `title` names the document;
//! - headings, list items and table rows are rows of their own; an
//!   ordered item's number and a task's state are its marker, said first
//!   ("2. Checked: Walk the dog");
//! - code blocks are rows as written, labeled with their fence's
//!   language; inline code is kept;
//! - footnote references read `[1]`, and their definitions come after the
//!   text under a "Footnotes" heading;
//! - math keeps its delimiters (`$x$`, `$$x$$`), so it reads as math;
//! - a GitHub alert or an Obsidian callout of any type is a quote that
//!   says its type first ("Tip: Remember", "Warning, collapsed: Hot
//!   stove");
//! - raw HTML's tags are dropped and its text kept, as textweaver reads a
//!   README's `<p align="center">` or `<details>`; comments, scripts and
//!   styles are not read; `<br>` and block tags break the line; an
//!   image's alt text is read;
//! - wiki links (`[[Page]]`, `[[Page|shown]]`) are links.
//!
//! What the quick view does not take from textweaver: Obsidian's embeds
//! (`![[note]]`). textweaver reads the other note in place; a preview
//! never reads a file other than the one previewed, so an embed reads
//! "note (embed not shown)" and the view notes it once. The reader takes
//! text, never a path, so it cannot open anything.

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, MetadataBlockKind, Options,
    Parser, Tag, TagEnd,
};

use super::callout::{self, Fold};
use super::{DocBuilder, DocKind, DocView, RowKind};

/// The note a view gets when an embed was left out.
pub const EMBED_NOTE: &str = "An embed was left out, so the note it names is not read.";

/// The extensions read as Markdown.
pub const EXTENSIONS: &[&str] = &[
    "md", "markdown", "mdown", "mkd", "mkdn", "mdwn", "mdtxt", "rmd",
];

/// Whether a file name is Markdown's, by its extension.
pub fn is_markdown(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// The tags whose start or end breaks the line in raw HTML.
const HTML_BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "details",
    "summary",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "tr",
    "table",
    "blockquote",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "figure",
    "figcaption",
];

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_WIKILINKS
}

/// Reads Markdown text into a view.
pub fn read(text: &str) -> DocView {
    let mut reader = Reader::new(text);
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        reader.event_at(event, range);
    }
    reader.finish()
}

/// Where raw HTML reading is.
#[derive(Debug, Default)]
struct HtmlState {
    in_comment: bool,
    skip_until: Option<&'static str>,
}

#[derive(Debug)]
struct ListState {
    depth: u8,
    next_number: Option<u64>,
}

/// One quote being read: its label, and the type words still to say
/// before its first text.
#[derive(Debug)]
struct QuoteState {
    label: Option<String>,
}

struct Reader<'a> {
    source: &'a str,
    b: DocBuilder,
    lists: Vec<ListState>,
    quotes: Vec<QuoteState>,
    /// A callout's or alert's type words, said before the first text.
    alert: Option<String>,
    /// The source offset where a callout's head line ends; its events
    /// are not read.
    head_end: Option<usize>,
    code: Option<(Option<String>, String)>,
    meta: Option<(MetadataBlockKind, String)>,
    /// Inside a footnote definition: its label and text.
    footnote: Option<(String, String)>,
    footnotes: Vec<(String, String)>,
    /// Events inside an embed, which are not read.
    skip_depth: usize,
    /// Inside an image: whether its alt text was written, and its title.
    images: Vec<(bool, String)>,
    html: HtmlState,
    /// The first level-1 heading's text, for the title.
    first_heading: Option<String>,
    in_heading: Option<String>,
}

impl<'a> Reader<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            b: DocView::builder(DocKind::Markdown),
            lists: Vec::new(),
            quotes: Vec::new(),
            alert: None,
            head_end: None,
            code: None,
            meta: None,
            footnote: None,
            footnotes: Vec::new(),
            skip_depth: 0,
            images: Vec::new(),
            html: HtmlState::default(),
            first_heading: None,
            in_heading: None,
        }
    }

    fn finish(mut self) -> DocView {
        if !self.footnotes.is_empty() {
            self.b.heading(2);
            self.b.text("Footnotes");
            for (label, text) in std::mem::take(&mut self.footnotes) {
                self.b.paragraph();
                self.b.text(&format!("[{label}]"));
                self.b.space();
                self.b.text(&text);
            }
        }
        if !self.b.has_title()
            && let Some(title) = self.first_heading.take()
        {
            self.b.set_title(title);
        }
        self.b.finish()
    }

    /// An event with where it is: a block quote whose first line is a
    /// callout's head (`> [!tip]- Title`) becomes a callout, and the
    /// head's own events are not read.
    fn event_at(&mut self, event: Event<'_>, range: std::ops::Range<usize>) {
        if let Some(end) = self.head_end {
            if range.start < end {
                if !matches!(event, Event::Start(_) | Event::End(_)) {
                    return;
                }
            } else {
                self.head_end = None;
            }
        }
        if let Event::Start(Tag::BlockQuote(_)) = &event
            && self.skip_depth == 0
            && self.code.is_none()
            && self.meta.is_none()
            && self.footnote.is_none()
        {
            let rest = self.source.get(range.start..).unwrap_or("");
            let line = rest.split('\n').next().unwrap_or(rest);
            if let Some(head) = callout::head(line, true) {
                self.callout(&head);
                self.head_end = Some(range.start + line.len());
                return;
            }
        }
        self.event(event);
    }

    /// A callout: a quote labeled with its type as written, its title on
    /// a row of its own after the type words ("Tip: Remember"), or the
    /// type words before its body when it has no title ("Note: ...").
    fn callout(&mut self, head: &callout::CalloutHead) {
        let fold = match head.fold {
            Some(Fold::Collapsed) => ", collapsed",
            Some(Fold::Expanded) => ", expanded",
            None => "",
        };
        let words = format!("{}{fold}:", head.type_word());
        self.quotes.push(QuoteState {
            label: Some(head.kind.clone()),
        });
        let title = plain_title(&head.custom_title);
        self.b.start(self.quote_kind());
        if title.is_empty() {
            self.alert = Some(words);
        } else {
            self.alert = None;
            self.b.text(&format!("{words} {title}"));
            self.b.end();
        }
    }

    fn quote_kind(&self) -> RowKind {
        RowKind::Quote {
            label: self.quotes.last().and_then(|q| q.label.clone()),
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if self.skip_depth > 0 {
            match event {
                Event::Start(_) => self.skip_depth += 1,
                Event::End(_) => self.skip_depth -= 1,
                _ => {}
            }
            return;
        }
        if let Some((_, text)) = self.meta.as_mut() {
            match event {
                Event::Text(t) => text.push_str(&t),
                Event::End(TagEnd::MetadataBlock(_)) => {
                    if let Some((kind, text)) = self.meta.take() {
                        self.front_matter(kind, &text);
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some((_, code)) = self.code.as_mut() {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((language, code)) = self.code.take() {
                        self.b.code_block(language.as_deref(), &code);
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some((_, text)) = self.footnote.as_mut() {
            match event {
                Event::Text(t) | Event::Code(t) => text.push_str(&t),
                Event::SoftBreak | Event::HardBreak => text.push(' '),
                Event::InlineMath(t) => text.push_str(&format!("${t}$")),
                Event::End(TagEnd::FootnoteDefinition) => {
                    if let Some((label, text)) = self.footnote.take() {
                        let text = super::build::collapse(&text);
                        self.footnotes.push((label, text));
                    }
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) | Event::Code(t) => self.text(&t),
            Event::SoftBreak => self.b.space(),
            Event::HardBreak => self.b.line_break(),
            Event::Rule => self.b.separator(),
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h),
            Event::FootnoteReference(label) => self.text(&format!("[{label}]")),
            Event::TaskListMarker(done) => {
                self.b
                    .set_item_marker(if done { "checked" } else { "not checked" });
            }
            Event::InlineMath(t) => self.text(&format!("${t}$")),
            Event::DisplayMath(t) => self.text(&format!("$${t}$$")),
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(words) = self.alert.take() {
            if self.b.open_kind().is_none() {
                self.b.start(self.quote_kind());
            }
            self.b.text(&words);
            self.b.space();
        }
        if let Some(h) = self.in_heading.as_mut() {
            h.push_str(t);
        }
        if let Some((written, _)) = self.images.last_mut() {
            *written |= !t.trim().is_empty();
        }
        self.b.text(t);
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.in_item() {
                    // An item's first paragraph is its row; a later one
                    // continues it.
                    if self.b.row_has_text() {
                        self.b.line_break();
                    }
                } else if !self.quotes.is_empty() {
                    // The type words still to say go before this text.
                    if self.alert.is_none() {
                        self.b.start(self.quote_kind());
                    }
                } else {
                    self.b.paragraph();
                }
            }
            Tag::Heading { level, .. } => {
                let level = heading_level(level);
                if level == 1 && self.first_heading.is_none() {
                    self.in_heading = Some(String::new());
                }
                self.b.heading(level);
            }
            Tag::BlockQuote(kind) => {
                let label = kind.map(|k| alert_word(k).to_lowercase());
                if let Some(kind) = kind {
                    self.alert = Some(format!("{}:", alert_word(kind)));
                }
                self.quotes.push(QuoteState { label });
                self.b.start(self.quote_kind());
            }
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((language, String::new()));
            }
            Tag::HtmlBlock => self.b.end(),
            Tag::List(start) => {
                let depth = u8::try_from(self.lists.len() + 1).unwrap_or(u8::MAX);
                self.lists.push(ListState {
                    depth,
                    next_number: start,
                });
            }
            Tag::Item => {
                let (depth, marker) = match self.lists.last_mut() {
                    Some(list) => {
                        let marker = list.next_number.map(|n| format!("{n}."));
                        if let Some(n) = list.next_number.as_mut() {
                            *n += 1;
                        }
                        (list.depth, marker)
                    }
                    None => (1, None),
                };
                self.b.list_item(depth, marker);
            }
            Tag::FootnoteDefinition(label) => {
                self.footnote = Some((label.to_string(), String::new()));
            }
            Tag::Table(_) => self.b.start_table(),
            Tag::TableHead => self.b.start_table_row(true),
            Tag::TableRow => self.b.start_table_row(false),
            Tag::TableCell => self.b.start_cell(),
            Tag::Link { dest_url, .. } => self.b.start_link(dest_url.to_string()),
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => {
                if matches!(link_type, LinkType::WikiLink { .. }) {
                    // `![[note]]`: an Obsidian embed, never read here.
                    self.text(&format!("{dest_url} (embed not shown)"));
                    self.b.note(EMBED_NOTE);
                    self.skip_depth = 1;
                    return;
                }
                self.images.push((false, title.to_string()));
            }
            Tag::MetadataBlock(kind) => self.meta = Some((kind, String::new())),
            Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if !self.in_item() {
                    self.b.end();
                }
            }
            TagEnd::Heading(_) => {
                self.b.end();
                if let Some(h) = self.in_heading.take() {
                    let h = super::build::collapse(&h);
                    if !h.is_empty() {
                        self.first_heading = Some(h);
                    }
                }
            }
            TagEnd::BlockQuote(_) => {
                // A callout with no body still says its type.
                if let Some(words) = self.alert.take() {
                    self.b.start(self.quote_kind());
                    self.b.text(words.trim_end_matches(':'));
                }
                self.quotes.pop();
                self.b.end();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.b.end();
            }
            TagEnd::Item => self.b.end(),
            TagEnd::Table => self.b.end_table(),
            TagEnd::TableHead | TagEnd::TableRow => self.b.end_table_row(),
            TagEnd::TableCell => self.b.end_cell(),
            TagEnd::Link => self.b.end_link(),
            TagEnd::Image => {
                if let Some((written, title)) = self.images.pop()
                    && !written
                    && !title.trim().is_empty()
                {
                    self.b.text(&title);
                }
            }
            TagEnd::HtmlBlock => self.b.end(),
            TagEnd::CodeBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    fn in_item(&self) -> bool {
        matches!(self.b.open_kind(), Some(RowKind::ListItem { .. })) && !self.lists.is_empty()
    }

    /// Raw HTML: its text read (entities decoded), its tags dropped, a
    /// line break for `<br>` and block tags, an image's alt text; nothing
    /// inside a comment, a script or a style.
    fn html(&mut self, html: &str) {
        let mut rest = html;
        loop {
            if self.html.in_comment {
                match rest.find("-->") {
                    Some(i) => {
                        rest = &rest[i + 3..];
                        self.html.in_comment = false;
                    }
                    None => return,
                }
            }
            if let Some(end) = self.html.skip_until {
                match rest.to_ascii_lowercase().find(end) {
                    Some(i) => {
                        rest = &rest[i..];
                        self.html.skip_until = None;
                    }
                    None => return,
                }
            }
            let Some(i) = rest.find('<') else {
                self.html_text(rest);
                return;
            };
            self.html_text(&rest[..i]);
            rest = &rest[i..];
            if let Some(after) = rest.strip_prefix("<!--") {
                self.html.in_comment = true;
                rest = after;
                continue;
            }
            let Some(close) = rest.find('>') else {
                // Not a tag after all: "a < b".
                self.html_text(rest);
                return;
            };
            let tag = &rest[1..close];
            rest = &rest[close + 1..];
            self.html_tag(tag);
        }
    }

    fn html_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.text(&decode_entities(text));
        }
    }

    fn html_tag(&mut self, tag: &str) {
        let inner = tag.trim();
        let closing = inner.starts_with('/');
        let name = inner
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        match name.as_str() {
            "br" => self.b.line_break(),
            "img" if !closing => {
                if let Some(alt) = html_attr(inner, "alt").filter(|a| !a.trim().is_empty()) {
                    self.text(&alt);
                }
            }
            "script" if !closing => self.html.skip_until = Some("</script"),
            "style" if !closing => self.html.skip_until = Some("</style"),
            "td" | "th" => self.b.space(),
            n if HTML_BLOCK_TAGS.contains(&n) => self.b.line_break(),
            _ => {}
        }
    }

    /// Front matter: its `title` names the document; the rest is not read.
    fn front_matter(&mut self, kind: MetadataBlockKind, text: &str) {
        let title = text.lines().find_map(|line| {
            let line = line.trim();
            let (key, value) = match kind {
                MetadataBlockKind::YamlStyle => line.split_once(':')?,
                MetadataBlockKind::PlusesStyle => line.split_once('=')?,
            };
            key.trim()
                .eq_ignore_ascii_case("title")
                .then(|| value.trim().trim_matches(['"', '\'']).to_owned())
        });
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            self.b.set_title(title);
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// What a GitHub alert says first.
fn alert_word(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "Note",
        BlockQuoteKind::Tip => "Tip",
        BlockQuoteKind::Important => "Important",
        BlockQuoteKind::Warning => "Warning",
        BlockQuoteKind::Caution => "Caution",
    }
}

/// A callout's title as words: the emphasis, code, highlight and wiki
/// link marks a title may carry are not read.
fn plain_title(title: &str) -> String {
    let mut t = title.to_owned();
    for mark in ["**", "__", "==", "~~", "[[", "]]", "`", "*"] {
        t = t.replace(mark, "");
    }
    super::build::collapse(&t)
}

/// The named and numbered character references HTML text uses most;
/// anything else stays as written.
pub(crate) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[1..].find(';').map(|e| e + 1).filter(|&e| e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// An attribute's value in a tag's text, quoted or not.
fn html_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        let before = lower[..start].chars().next_back();
        let after = &tag[start + name.len()..];
        from = start + name.len();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let found = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next(),
        };
        return found.map(decode_entities);
    }
    None
}
