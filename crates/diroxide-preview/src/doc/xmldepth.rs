//! The nesting cut before an XML parser sees a part (carried from
//! textweaver's `xmldepth.rs` under the owner's relicensing, answer 41;
//! the rule and its two passes, written here anew). `roxmltree` recurses
//! once per nested element, so a part nested tens of thousands deep
//! would overflow the stack before any reader code runs. [`limit_depth`]
//! rewrites such a part so nothing is nested deeper than a limit: below
//! the limit, an element that holds other elements loses its tags (its
//! text stays), and the leaf elements (a DOCX run's text, a tab, a break)
//! are kept in order inside one [`FLAT_ELEMENT`], which readers take as
//! plain text. A part that is nested no deeper than the limit comes back
//! as `None`, so it is not copied.

/// The element that wraps flattened content.
pub const FLAT_ELEMENT: &str = "diroxide-flat";

/// How deep a part may nest before its deeper content is flattened, the
/// HTML reader's limit too.
pub const MAX_NESTING: usize = 256;

/// What a view says when part of a file was flattened.
pub const NESTING_NOTE: &str =
    "Part of this file is nested too deeply, so that part is read as plain text.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Text,
    Open { empty: bool },
    Close,
    Other,
}

/// The index of the byte after the first `pattern` at or after `from`.
fn after(bytes: &[u8], from: usize, pattern: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(pattern.len())
        .position(|w| w == pattern)
        .map(|at| from + at + pattern.len())
}

/// The part's tokens, one at a time: each a kind and its byte range.
/// Markup left open at the end is text, which the parser then refuses as
/// it would anyway.
struct Tokens<'a> {
    b: &'a [u8],
    i: usize,
}

impl Iterator for Tokens<'_> {
    type Item = (Token, usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let (b, i) = (self.b, self.i);
        if i >= b.len() {
            return None;
        }
        if b[i] != b'<' {
            let end = b[i..]
                .iter()
                .position(|&c| c == b'<')
                .map_or(b.len(), |p| i + p);
            self.i = end;
            return Some((Token::Text, i, end));
        }
        let rest = &b[i..];
        let found = if rest.starts_with(b"<!--") {
            after(b, i + 4, b"-->").map(|e| (Token::Other, e))
        } else if rest.starts_with(b"<![CDATA[") {
            after(b, i + 9, b"]]>").map(|e| (Token::Text, e))
        } else if rest.starts_with(b"<?") {
            after(b, i + 2, b"?>").map(|e| (Token::Other, e))
        } else if rest.starts_with(b"<!") {
            // A document type, perhaps with a bracketed internal subset.
            let mut depth = 0usize;
            let mut end = None;
            for (p, &c) in rest.iter().enumerate() {
                match c {
                    b'[' => depth += 1,
                    b']' => depth = depth.saturating_sub(1),
                    b'>' if depth == 0 => {
                        end = Some(i + p + 1);
                        break;
                    }
                    _ => {}
                }
            }
            end.map(|e| (Token::Other, e))
        } else if rest.starts_with(b"</") {
            after(b, i + 2, b">").map(|e| (Token::Close, e))
        } else {
            let mut quote = None;
            let mut end = None;
            for (p, &c) in rest.iter().enumerate().skip(1) {
                match (quote, c) {
                    (None, b'"' | b'\'') => quote = Some(c),
                    (Some(q), c) if c == q => quote = None,
                    (None, b'>') => {
                        end = Some((p > 0 && rest[p - 1] == b'/', i + p + 1));
                        break;
                    }
                    _ => {}
                }
            }
            end.map(|(empty, e)| (Token::Open { empty }, e))
        };
        match found {
            Some((kind, end)) => {
                self.i = end;
                Some((kind, i, end))
            }
            None => {
                self.i = b.len();
                Some((Token::Text, i, b.len()))
            }
        }
    }
}

fn tokens(text: &str) -> Tokens<'_> {
    Tokens {
        b: text.as_bytes(),
        i: 0,
    }
}

/// How deep the part's elements nest, read in one pass that keeps
/// nothing: what the container read path checks every XML part with
/// before it pays for a rewrite.
pub fn deepest(text: &str) -> usize {
    let mut open = 0usize;
    let mut deepest = 0usize;
    for (kind, ..) in tokens(text) {
        match kind {
            Token::Open { empty: false } => {
                open += 1;
                deepest = deepest.max(open);
            }
            Token::Close => open = open.saturating_sub(1),
            Token::Open { empty: true } | Token::Text | Token::Other => {}
        }
    }
    deepest
}

/// The part rewritten so no element nests deeper than `limit` (plus the
/// flat wrapper and its leaves), or `None` when it already does not. The
/// leaves go inside one [`FLAT_ELEMENT`].
pub fn limit_depth(text: &str, limit: usize) -> Option<String> {
    let open = format!("<{FLAT_ELEMENT}>");
    let close = format!("</{FLAT_ELEMENT}>");
    limit_depth_with(text, limit, &open, &close)
}

/// [`limit_depth`] with the wrapper's own open and close tags, so a
/// format's reader takes the leaves as its own text: a Word part's leaves
/// inside a paragraph and a run (`<w:p><w:r>`), an OpenDocument part's
/// inside a paragraph (`<text:p>`).
pub fn limit_depth_with(
    text: &str,
    limit: usize,
    open_tag: &str,
    close_tag: &str,
) -> Option<String> {
    if deepest(text) <= limit {
        return None;
    }
    let tokens: Vec<(Token, usize, usize)> = tokens(text).collect();
    // First pass: which elements hold elements.
    let mut holds = vec![false; tokens.len()];
    let mut open: Vec<usize> = Vec::new();
    for (n, &(kind, ..)) in tokens.iter().enumerate() {
        match kind {
            Token::Open { empty } => {
                if let Some(&parent) = open.last() {
                    holds[parent] = true;
                }
                if !empty {
                    open.push(n);
                }
            }
            Token::Close => {
                open.pop();
            }
            Token::Text | Token::Other => {}
        }
    }
    // Second pass: copy, leaving out the tags of holders below the limit.
    let mut out = String::with_capacity(text.len());
    // For each open element, whether its tags were copied.
    let mut kept: Vec<bool> = Vec::new();
    let mut flat = false;
    for (n, &(kind, start, end)) in tokens.iter().enumerate() {
        let token = &text[start..end];
        match kind {
            Token::Open { empty } => {
                let keep = if kept.len() < limit {
                    true
                } else {
                    if !flat {
                        out.push_str(open_tag);
                        flat = true;
                    }
                    // Leaves only: they hold no elements, so what is kept
                    // never nests.
                    !holds[n]
                };
                if keep {
                    out.push_str(token);
                } else {
                    // Words in dropped elements stay apart.
                    out.push(' ');
                }
                if !empty {
                    kept.push(keep);
                }
            }
            Token::Close => {
                let level = kept.len();
                let was_kept = kept.pop().unwrap_or(true);
                if level == limit && flat {
                    out.push_str(close_tag);
                    flat = false;
                }
                if was_kept {
                    out.push_str(token);
                } else {
                    out.push(' ');
                }
            }
            Token::Text | Token::Other => out.push_str(token),
        }
    }
    if flat {
        out.push_str(close_tag);
    }
    Some(out)
}

/// Most nodes one XML part may have: about a gigabyte of parsed tree.
pub const MAX_XML_NODES: u32 = 16_000_000;

/// Parses XML with a document type allowed (no external entity is ever
/// read) and at most [`MAX_XML_NODES`] nodes.
pub fn parse(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: MAX_XML_NODES,
        ..roxmltree::ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(text, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(depth: usize, inner: &str) -> String {
        let mut s = String::from("<?xml version=\"1.0\"?><!-- note --><root>");
        for _ in 0..depth {
            s.push_str("<b x=\"a>b\">");
        }
        s.push_str(inner);
        for _ in 0..depth {
            s.push_str("</b>");
        }
        s.push_str("</root>");
        s
    }

    #[test]
    fn a_shallow_part_is_left_alone() {
        assert_eq!(limit_depth(&nested(5, "<t>x</t>"), 8), None);
        assert_eq!(deepest(&nested(5, "<t>x</t><br/>")), 7);
        assert_eq!(deepest("<a><!-- <b> --><![CDATA[<c>]]></a>"), 1);
    }

    #[test]
    fn a_word_part_keeps_its_deep_text_in_a_paragraph() {
        let mut part = String::from("<w:body>");
        part.push_str(&"<w:sdt>".repeat(300));
        part.push_str("<w:p><w:r><w:t>deep</w:t></w:r></w:p>");
        part.push_str(&"</w:sdt>".repeat(300));
        part.push_str("</w:body>");
        let out = limit_depth_with(&part, 10, "<w:p><w:r>", "</w:r></w:p>").unwrap();
        let at = out.find("<w:p><w:r>").unwrap();
        assert!(out[at..].contains("<w:t>deep</w:t>"), "{out}");
        assert!(out[at..].contains("</w:r></w:p>"), "{out}");
        assert!(deepest(&out) <= 13);
    }

    #[test]
    fn a_deep_part_is_flattened_and_keeps_its_leaves_in_order() {
        let source = nested(50_000, "<t>deep</t><tab/><t><![CDATA[a<b]]></t>");
        let out = limit_depth(&source, 10).unwrap();
        let doc = parse(&out).unwrap();
        let flat: Vec<_> = doc
            .descendants()
            .filter(|n| n.tag_name().name() == FLAT_ELEMENT)
            .collect();
        assert_eq!(flat.len(), 1);
        assert!(flat[0].ancestors().count() <= 12);
        let leaves: Vec<(String, String)> = flat[0]
            .children()
            .filter(|n| n.is_element())
            .map(|n| {
                (
                    n.tag_name().name().to_owned(),
                    n.text().unwrap_or("").to_owned(),
                )
            })
            .collect();
        assert_eq!(
            leaves,
            [
                ("t".to_owned(), "deep".to_owned()),
                ("tab".to_owned(), String::new()),
                ("t".to_owned(), "a<b".to_owned())
            ]
        );
    }

    #[test]
    fn markup_left_open_never_panics() {
        for s in [
            "<a",
            "<a b='>",
            "<!-- x",
            "<![CDATA[",
            "<!DOCTYPE [",
            "</",
            "<?x",
        ] {
            let _ = limit_depth(s, 0);
            let _ = limit_depth(s, 3);
        }
    }
}
