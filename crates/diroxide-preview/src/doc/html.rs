// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The HTML-fragment reader, for carta's output (path A of spike C1, plan
//! question 66). Its rules, its skip and block lists, its hidden-element
//! test and its row gathering are carried from textweaver's HTML loader
//! (`html.rs`) under the owner's relicensing (answer 41): the fragment
//! conversion only (headings, paragraphs, lists, tables, links, code),
//! not the file-level reader alpha.7 brings, written onto the document
//! view's builder.
//!
//! The rules: what the skip list names, and what is `hidden` or
//! `aria-hidden="true"`, is not read; headings, paragraphs, list items,
//! table rows and code are rows; an ordered item's number is its marker;
//! a table's first row is its header when it is in `<thead>` or made of
//! `<th>` cells; a `<pre>` keeps its lines and names its language from a
//! `language-` or `lang-` class; an image reads its `alt`, then `title`,
//! then `aria-label`, and `role="presentation"` is silent; content
//! nested past [`MAX_NESTING`] is read as text, without its structure,
//! by a loop rather than recursion, so no depth can overflow the stack.

use scraper::{ElementRef, Html, Node};

use super::{DocBuilder, DocKind, DocView, RowKind};

/// How deep elements nest before what is inside is read as text.
pub const MAX_NESTING: usize = 256;

/// Elements that are not read. Carried from textweaver's HTML loader.
const SKIP: &[&str] = &[
    "script", "style", "nav", "footer", "aside", "noscript", "svg", "canvas", "meta", "link",
    "base", "iframe", "template", "button", "form",
];

/// Elements that start and end a paragraph-like block. Carried from
/// textweaver's HTML loader.
const BLOCKS: &[&str] = &[
    "div", "section", "article", "main", "header", "address", "figure", "details", "summary",
    "center", "hgroup", "search", "fieldset", "body",
];

/// Inline elements that do not separate words, for flattened content.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "dfn", "em", "i", "kbd", "mark", "q",
    "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var",
];

/// Reads an HTML fragment into a view of a kind.
pub fn read_fragment(html: &str, kind: DocKind) -> DocView {
    let fragment = Html::parse_fragment(html);
    let mut walker = Walker {
        b: DocView::builder(kind),
        lists: Vec::new(),
        depth: 0,
        in_cell: 0,
        quotes: 0,
    };
    walker.children(fragment.root_element());
    walker.b.finish()
}

/// Whether an element is hidden. Carried from textweaver's HTML loader.
fn is_hidden(el: &ElementRef<'_>) -> bool {
    let v = el.value();
    v.attr("hidden").is_some()
        || v.attr("aria-hidden")
            .is_some_and(|a| a.eq_ignore_ascii_case("true"))
}

/// The rows of a table in document order, each with whether it is in the
/// table head. Carried from textweaver's HTML loader.
fn rows(table: ElementRef<'_>) -> Vec<(ElementRef<'_>, bool)> {
    let mut out = Vec::new();
    for child in table.child_elements() {
        match child.value().name() {
            "tr" => out.push((child, false)),
            "thead" | "tbody" | "tfoot" => {
                let head = child.value().name() == "thead";
                out.extend(
                    child
                        .child_elements()
                        .filter(|r| r.value().name() == "tr")
                        .map(|r| (r, head)),
                );
            }
            _ => {}
        }
    }
    out
}

struct Walker {
    b: DocBuilder,
    /// The open lists: the next number of an ordered one.
    lists: Vec<Option<i64>>,
    depth: usize,
    in_cell: usize,
    quotes: usize,
}

impl Walker {
    fn children(&mut self, el: ElementRef<'_>) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => self.text(t),
                Node::Element(_) => {
                    if let Some(c) = ElementRef::wrap(child) {
                        self.element(c);
                    }
                }
                _ => {}
            }
        }
    }

    fn text(&mut self, t: &str) {
        if self.quotes > 0 && self.b.open_kind().is_none() && !t.trim().is_empty() {
            self.b.start(RowKind::Quote { label: None });
        }
        self.b.text(t);
    }

    fn element(&mut self, el: ElementRef<'_>) {
        if self.depth >= MAX_NESTING {
            self.flatten(el);
            return;
        }
        self.depth += 1;
        self.element_inner(el);
        self.depth -= 1;
    }

    /// Content nested past the limit: its text without structure, with an
    /// explicit stack, children in document order.
    fn flatten(&mut self, el: ElementRef<'_>) {
        self.b.space();
        let mut stack: Vec<_> = el.children().collect();
        stack.reverse();
        while let Some(n) = stack.pop() {
            match n.value() {
                Node::Text(t) => self.b.text(t),
                Node::Element(e) => {
                    let hidden = ElementRef::wrap(n).is_some_and(|r| is_hidden(&r));
                    if SKIP.contains(&e.name()) || hidden {
                        continue;
                    }
                    if !INLINE.contains(&e.name()) {
                        self.b.space();
                    }
                    let from = stack.len();
                    stack.extend(n.children());
                    stack[from..].reverse();
                }
                _ => {}
            }
        }
        self.b.space();
    }

    /// Whether a list item's row is being written, so a paragraph in it
    /// continues it.
    fn in_item(&self) -> bool {
        !self.lists.is_empty() && matches!(self.b.open_kind(), Some(RowKind::ListItem { .. }))
    }

    fn element_inner(&mut self, el: ElementRef<'_>) {
        let name = el.value().name();
        if SKIP.contains(&name) || is_hidden(&el) {
            return;
        }
        match name {
            "head" | "title" => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.in_cell > 0 {
                    self.b.space();
                    self.children(el);
                    return;
                }
                let level = name[1..].parse::<u8>().unwrap_or(1);
                self.b.heading(level);
                self.children(el);
                self.b.end();
            }
            "p" => {
                if self.in_cell > 0 {
                    self.b.space();
                    self.children(el);
                } else if self.in_item() {
                    if self.b.row_has_text() {
                        self.b.line_break();
                    }
                    self.children(el);
                } else {
                    if self.quotes > 0 {
                        self.b.start(RowKind::Quote { label: None });
                    } else {
                        self.b.paragraph();
                    }
                    self.children(el);
                    self.b.end();
                }
            }
            "br" => self.b.line_break(),
            "hr" => self.b.separator(),
            "ul" | "ol" | "menu" => self.list(el, name == "ol"),
            "li" => self.list_item(el),
            "dl" => {
                self.b.end();
                self.children(el);
                self.b.end();
            }
            "dt" | "dd" => {
                if self.in_cell > 0 {
                    self.b.space();
                } else {
                    self.b.paragraph();
                }
                self.children(el);
                if self.in_cell == 0 {
                    self.b.end();
                }
            }
            "blockquote" => {
                if self.in_cell > 0 {
                    self.children(el);
                    return;
                }
                self.b.end();
                self.quotes += 1;
                self.children(el);
                self.quotes -= 1;
                self.b.end();
            }
            "pre" => self.pre(el),
            "a" => match el.attr("href") {
                Some(href) => {
                    self.b.start_link(href);
                    self.children(el);
                    self.b.end_link();
                }
                None => self.children(el),
            },
            "img" => self.image(el),
            "table" => self.table(el),
            "figcaption" => {
                self.b.end();
                self.children(el);
                self.b.end();
            }
            _ if BLOCKS.contains(&name) => {
                if self.in_cell > 0 {
                    self.b.space();
                    self.children(el);
                } else {
                    self.b.end();
                    self.children(el);
                    self.b.end();
                }
            }
            _ => self.children(el),
        }
    }

    fn list(&mut self, el: ElementRef<'_>, ordered: bool) {
        if self.in_cell > 0 {
            self.children(el);
            return;
        }
        let start = el
            .attr("start")
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(1);
        self.lists.push(ordered.then_some(start));
        self.children(el);
        self.lists.pop();
        self.b.end();
    }

    fn list_item(&mut self, el: ElementRef<'_>) {
        if self.in_cell > 0 {
            self.b.space();
            self.children(el);
            return;
        }
        let depth = u8::try_from(self.lists.len().max(1)).unwrap_or(u8::MAX);
        let mut marker = None;
        if let Some(Some(n)) = self.lists.last_mut() {
            if let Some(v) = el.attr("value").and_then(|v| v.trim().parse::<i64>().ok()) {
                *n = v;
            }
            marker = Some(format!("{n}."));
            *n += 1;
        }
        self.b.list_item(depth, marker);
        self.children(el);
        self.b.end();
    }

    fn pre(&mut self, el: ElementRef<'_>) {
        let text: String = el.text().collect();
        if self.in_cell > 0 {
            self.b.text(&text);
            return;
        }
        let language = std::iter::once(el)
            .chain(el.child_elements().filter(|c| c.value().name() == "code"))
            .flat_map(|e| e.value().classes().collect::<Vec<_>>())
            .find_map(|c| {
                c.strip_prefix("language-")
                    .or_else(|| c.strip_prefix("lang-"))
                    .map(str::to_owned)
            });
        self.b.code_block(language.as_deref(), &text);
    }

    fn image(&mut self, el: ElementRef<'_>) {
        if el.attr("role").is_some_and(|r| {
            r.eq_ignore_ascii_case("presentation") || r.eq_ignore_ascii_case("none")
        }) {
            return;
        }
        let alt = ["alt", "title", "aria-label"]
            .iter()
            .filter_map(|a| el.attr(a))
            .map(super::build::collapse)
            .find(|s| !s.is_empty());
        if let Some(alt) = alt {
            self.b.space();
            self.b.text(&alt);
            self.b.space();
        }
    }

    fn table(&mut self, el: ElementRef<'_>) {
        if self.in_cell > 0 {
            // A nested table reads as running text inside its cell.
            self.children(el);
            return;
        }
        if let Some(caption) = el
            .child_elements()
            .find(|c| c.value().name() == "caption")
            .map(|c| super::build::collapse(&c.text().collect::<String>()))
            .filter(|c| !c.is_empty())
        {
            self.b.paragraph();
            self.b.text(&caption);
            self.b.end();
        }
        self.b.start_table();
        let mut first = true;
        for (row, in_head) in rows(el) {
            let cells: Vec<ElementRef<'_>> = row
                .child_elements()
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .collect();
            if cells.is_empty() {
                continue;
            }
            let header = first && (in_head || cells.iter().all(|c| c.value().name() == "th"));
            first = false;
            self.b.start_table_row(header);
            for cell in cells {
                self.b.start_cell();
                self.in_cell += 1;
                self.children(cell);
                self.in_cell -= 1;
                self.b.end_cell();
            }
            self.b.end_table_row();
        }
        self.b.end_table();
    }
}
