//! SVG drawings read the way SVG-AAM exposes them to a screen reader
//! (alpha 4, R30 commit 34; carried from textweaver's `svg.rs` under the
//! owner's relicensing, answers 3 and 41, its rules written here anew on
//! the row builder). A drawing reads as its name first (`<title>`, or
//! `aria-label`) as a level-1 heading, then its description (`<desc>`,
//! or `aria-description`), then each titled part (a group, a chart's bar
//! with its own `<title>`, with its own `<desc>` after a colon) as a list
//! item in document order, then the text it shows (`<text>` with its
//! `<tspan>`s), a row each. `role="img"` on the drawing makes it one
//! graphic whose parts are not read. A drawing with none of these reads
//! "Drawing with no description".
//!
//! What is never shown is never read: definitions, symbols, styles,
//! scripts, metadata, clip paths, masks, patterns, markers, gradients,
//! filters, foreign objects, and anything `aria-hidden`, `display: none`
//! or `visibility: hidden` by attribute. The file is parsed by
//! `roxmltree` with a document type allowed and no external entity read,
//! its nesting cut first ([`super::xmldepth`]), at most
//! [`MAX_ELEMENTS`] elements looked at, from at most [`MAX_BYTES`].

use super::xmldepth::{self, MAX_NESTING, NESTING_NOTE};
use super::{DocBuilder, DocError, DocKind, DocView};
use crate::encoding::{binary_kind, decode};
use crate::words::clean_text;

/// The largest drawing read, in bytes.
pub const MAX_BYTES: usize = 32 << 20;

/// The most elements of one drawing looked at.
pub const MAX_ELEMENTS: usize = 500_000;

/// What a drawing with nothing to read says.
pub const NO_DESCRIPTION: &str = "Drawing with no description";

/// The kind's name, as the announcement says it.
pub const KIND: &str = "SVG";

/// Elements whose content is never shown, or is never text.
const HIDDEN: &[&str] = &[
    "defs",
    "symbol",
    "style",
    "script",
    "metadata",
    "clippath",
    "mask",
    "pattern",
    "marker",
    "lineargradient",
    "radialgradient",
    "filter",
    "foreignobject",
];

/// An element, kept to what reading needs.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Child>,
}

#[derive(Debug)]
enum Child {
    Element(Node),
    Text(String),
}

impl Node {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn elements(&self) -> impl DoubleEndedIterator<Item = &Node> {
        self.children.iter().filter_map(|c| match c {
            Child::Element(e) => Some(e),
            Child::Text(_) => None,
        })
    }

    fn hidden(&self) -> bool {
        HIDDEN.contains(&self.name.to_ascii_lowercase().as_str())
            || self
                .attribute("aria-hidden")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
            || self
                .attribute("display")
                .is_some_and(|v| v.trim() == "none")
            || self
                .attribute("visibility")
                .is_some_and(|v| v.trim() == "hidden")
    }

    /// Every word inside, in order, joined by single spaces, what is
    /// hidden left out.
    fn words(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<&Child> = self.children.iter().rev().collect();
        while let Some(child) = stack.pop() {
            match child {
                Child::Text(text) => {
                    for word in text.split_whitespace() {
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(word);
                    }
                }
                Child::Element(e) if !e.hidden() => stack.extend(e.children.iter().rev()),
                Child::Element(_) => {}
            }
        }
        out
    }

    /// The words of the first child element of a name.
    fn child_words(&self, name: &str) -> Option<String> {
        self.elements()
            .find(|e| e.name == name)
            .map(Node::words)
            .filter(|t| !t.is_empty())
    }

    /// Its accessible name: `aria-label`, else its own `<title>`.
    fn accessible_name(&self) -> Option<String> {
        self.attribute("aria-label")
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .or_else(|| self.child_words("title"))
    }
}

/// What a drawing gives a reader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// Its name.
    pub name: Option<String>,
    /// Its description.
    pub description: Option<String>,
    /// Its titled parts, in order: "Quarter 1: 40 units".
    pub parts: Vec<String>,
    /// The text it shows, in order.
    pub texts: Vec<String>,
    /// Its width and height, from its attributes or its view box, when
    /// they are plain numbers.
    pub size: Option<(u32, u32)>,
}

impl Reading {
    /// Whether there is nothing at all to read.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.parts.is_empty()
            && self.texts.is_empty()
    }
}

fn read_root(root: &Node) -> Reading {
    let mut reading = Reading {
        name: root.accessible_name(),
        description: root
            .attribute("aria-description")
            .map(str::to_owned)
            .or_else(|| root.child_words("desc")),
        size: size_of(root),
        ..Reading::default()
    };
    if root
        .attribute("role")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("img"))
    {
        return reading;
    }
    let mut stack: Vec<&Node> = root.elements().rev().collect();
    while let Some(e) = stack.pop() {
        if e.hidden() || matches!(e.name.as_str(), "title" | "desc") {
            continue;
        }
        if e.name == "text" {
            let words = e.words();
            if !words.is_empty() {
                reading.texts.push(words);
            }
            continue;
        }
        if let Some(name) = e.accessible_name() {
            reading.parts.push(match e.child_words("desc") {
                Some(d) => format!("{name}: {d}"),
                None => name,
            });
        }
        stack.extend(e.elements().rev());
    }
    reading
}

/// A size from `width` and `height` when both are plain numbers (pixels,
/// or `px`), else from the view box's last two numbers.
fn size_of(root: &Node) -> Option<(u32, u32)> {
    let plain = |v: &str| -> Option<f64> {
        let v = v.trim();
        let v = v.strip_suffix("px").unwrap_or(v);
        v.parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0)
    };
    let whole = |n: f64| n.round().min(f64::from(u32::MAX)) as u32;
    if let (Some(w), Some(h)) = (
        root.attribute("width").and_then(plain),
        root.attribute("height").and_then(plain),
    ) {
        return Some((whole(w), whole(h)));
    }
    let numbers: Vec<f64> = root
        .attribute("viewBox")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    match numbers.as_slice() {
        [_, _, w, h] if *w > 0.0 && *h > 0.0 && w.is_finite() && h.is_finite() => {
            Some((whole(*w), whole(*h)))
        }
        _ => None,
    }
}

/// The small tree of a parsed drawing, built without recursion, so no
/// depth can overflow the stack; at most [`MAX_ELEMENTS`] elements.
fn tree(root: roxmltree::Node<'_, '_>) -> Node {
    fn shallow(n: roxmltree::Node<'_, '_>) -> Node {
        Node {
            name: n.tag_name().name().to_owned(),
            attributes: n
                .attributes()
                .map(|a| (a.name().to_owned(), a.value().to_owned()))
                .collect(),
            children: Vec::new(),
        }
    }
    fn reversed<'a, 'i>(n: roxmltree::Node<'a, 'i>) -> Vec<roxmltree::Node<'a, 'i>> {
        let mut kids: Vec<_> = n.children().collect();
        kids.reverse();
        kids
    }
    let mut stack = vec![(shallow(root), reversed(root))];
    let mut seen = 0usize;
    loop {
        let Some(top) = stack.last_mut() else {
            return Node::default();
        };
        let Some(next) = top.1.pop() else {
            let Some((done, _)) = stack.pop() else {
                return Node::default();
            };
            match stack.last_mut() {
                Some(parent) => parent.0.children.push(Child::Element(done)),
                None => return done,
            }
            continue;
        };
        if next.is_text() {
            if let Some(text) = next.text() {
                top.0.children.push(Child::Text(text.to_owned()));
            }
        } else if next.is_element() {
            seen += 1;
            if seen <= MAX_ELEMENTS {
                stack.push((shallow(next), reversed(next)));
            }
        }
    }
}

/// Reads a drawing's bytes into what it gives a reader, every string
/// cleaned like a file name, and whether part of it was flattened.
pub fn reading(bytes: &[u8]) -> Result<(Reading, bool), DocError> {
    let not_valid = || DocError::NotValid(KIND.to_owned());
    if bytes.len() > MAX_BYTES || binary_kind(bytes).is_some() {
        return Err(not_valid());
    }
    let text = decode(bytes, None).text;
    let limited = xmldepth::limit_depth(&text, MAX_NESTING);
    let flattened = limited.is_some();
    let source = limited.as_deref().unwrap_or(&text);
    let document = xmldepth::parse(source).map_err(|_| not_valid())?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err(not_valid());
    }
    let mut reading = read_root(&tree(root));
    let clean = |t: &String| clean_text(t, 400);
    reading.name = reading.name.as_ref().map(clean).filter(|t| !t.is_empty());
    reading.description = reading
        .description
        .as_ref()
        .map(clean)
        .filter(|t| !t.is_empty());
    reading.parts = reading.parts.iter().map(clean).collect();
    reading.texts = reading.texts.iter().map(clean).collect();
    Ok((reading, flattened))
}

/// Reads a drawing into rows: its name as a level-1 heading, its
/// description, its parts as a list, then its text. Unguarded; the quick
/// view reads through [`super::read`], which runs it under the guard.
pub fn read(bytes: &[u8]) -> Result<DocView, DocError> {
    let (reading, flattened) = reading(bytes)?;
    Ok(view_of(&reading, flattened))
}

/// The rows of a drawing already read, with the nesting note when part
/// of it was flattened.
pub fn view_of(reading: &Reading, flattened: bool) -> DocView {
    let mut b = DocBuilder::new(DocKind::Format(KIND.to_owned()));
    write(&mut b, reading);
    if flattened {
        b.note(NESTING_NOTE);
    }
    b.finish()
}

fn write(b: &mut DocBuilder, reading: &Reading) {
    if reading.is_empty() {
        b.paragraph();
        b.text(NO_DESCRIPTION);
        b.end();
        return;
    }
    if let Some(name) = &reading.name {
        b.set_title(name.clone());
        b.heading(1);
        b.text(name);
        b.end();
    }
    if let Some(description) = &reading.description {
        b.paragraph();
        b.text(description);
        b.end();
    }
    for part in &reading.parts {
        b.list_item(1, None);
        b.text(part);
        b.end();
    }
    for text in &reading.texts {
        b.paragraph();
        b.text(text);
        b.end();
    }
}

/// The drawing's sentence for the details line: "SVG, 512 by 512, titled
/// Example Logo", "SVG, 512 by 512, 4 KB", "SVG drawing, 4 KB".
pub fn sentence(reading: &Reading, short: &str, file_size: Option<u64>) -> String {
    let mut parts = vec![short.to_owned()];
    if let Some((w, h)) = reading.size {
        parts.push(format!("{w} by {h}"));
    }
    match (&reading.name, file_size) {
        (Some(name), _) => parts.push(format!("titled {name}")),
        (None, Some(size)) => parts.push(diroxide_core::format::human_size(size)),
        (None, None) => {}
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r#"<?xml version="1.0"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 480">
  <title>Sales by quarter</title>
  <desc>Sales rose all year.</desc>
  <defs><text>hidden definition</text></defs>
  <g><title>Quarter 1</title><desc>40 units</desc><rect/></g>
  <rect aria-label="Quarter 2"/>
  <g aria-hidden="true"><title>Secret</title><text>no</text></g>
  <text x="1">Units <tspan>sold</tspan></text>
  <text>2026</text>
</svg>"#;

    fn shown(view: &DocView) -> Vec<String> {
        view.rows()
            .iter()
            .map(super::super::DocRow::shown)
            .collect()
    }

    #[test]
    fn a_chart_reads_its_name_description_parts_then_text() {
        let view = read(CHART.as_bytes()).unwrap();
        assert_eq!(
            shown(&view),
            [
                "Sales by quarter",
                "Sales rose all year.",
                "Quarter 1: 40 units",
                "Quarter 2",
                "Units sold",
                "2026"
            ]
        );
        assert_eq!(
            view.heading_words(0).as_deref(),
            Some("Heading 1: Sales by quarter, 1 of 1")
        );
        assert_eq!(view.title.as_deref(), Some("Sales by quarter"));
        assert_eq!(view.kind.word(), Some("SVG"));
        let (reading, _) = reading(CHART.as_bytes()).unwrap();
        assert_eq!(reading.size, Some((640, 480)));
        let line = sentence(&reading, "SVG", Some(4_000));
        assert_eq!(line, "SVG, 640 by 480, titled Sales by quarter");
        assert!(line.chars().count() <= 40);
    }

    #[test]
    fn nothing_to_read_one_graphic_and_refusals() {
        let empty = read(br#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512px"><circle r="4"/></svg>"#).unwrap();
        assert_eq!(shown(&empty), [NO_DESCRIPTION]);
        let (r, _) = reading(br#"<svg width="512" height="512px"/>"#).unwrap();
        assert_eq!(sentence(&r, "SVG", Some(4_000)), "SVG, 512 by 512, 4.0 KB");
        let logo = read(
            br#"<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Example Logo"><text>LOGO</text></svg>"#,
        )
        .unwrap();
        assert_eq!(shown(&logo), ["Example Logo"]);
        assert!(read(b"<html/>").is_err());
        assert!(read(b"<svg").is_err());
        assert!(read(b"\x00\x01\x02binary").is_err());
    }

    #[test]
    fn hostile_text_is_cleaned_and_deep_nesting_is_flattened() {
        let evil = "<svg><title>Bad\u{202E}name\n\twith breaks</title></svg>";
        let view = read(evil.as_bytes()).unwrap();
        assert_eq!(shown(&view), ["Bad?name with breaks"]);
        let mut deep = String::from("<svg><title>Deep</title>");
        for _ in 0..20_000 {
            deep.push_str("<g>");
        }
        deep.push_str("<text>bottom</text>");
        for _ in 0..20_000 {
            deep.push_str("</g>");
        }
        deep.push_str("</svg>");
        let view = read(deep.as_bytes()).unwrap();
        assert_eq!(view.notes(), [NESTING_NOTE]);
        assert_eq!(shown(&view)[0], "Deep");
    }
}
