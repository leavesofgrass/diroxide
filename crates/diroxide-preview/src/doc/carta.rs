// SPDX-License-Identifier: Apache-2.0 OR MIT
//! carta's formats in the quick view: Org, reStructuredText, MediaWiki,
//! DokuWiki and Jira, read in process, in pure Rust, by carta 0.0.11, a
//! young reimplementation of pandoc (MIT or Apache-2.0), pinned exactly
//! (plan section 5.9, answer 27). The format tables, the include
//! neutralizer, the decode-first step and the writer list are carried
//! from textweaver's carta bridge (`carta.rs`) under the owner's
//! relicensing (answer 41); its loader shell, its priority and its
//! document building are not, since they belong to textweaver's model.
//!
//! carta writes an HTML fragment and the fragment reader
//! ([`super::html`]) reads it into rows: path A of spike C1 (plan
//! question 66), chosen because carta's CommonMark writer turns every
//! table into raw HTML.
//!
//! The safety rules, textweaver's: carta's reStructuredText reader
//! follows `.. include::` to any path on the disk, relative to the
//! working folder, with no sandbox, so every include is turned into a
//! comment before carta sees the text ([`without_includes`]) and the view
//! says so once ([`INCLUDE_WARNING`]); carta's Typst reader reads any file
//! a document names and is not in this build. carta reads UTF-8 only, so
//! the bytes are decoded first, as every other reader's are.

use std::borrow::Cow;

use super::{DocError, DocKind, DocView, html};
use crate::encoding::decode;

/// Extensions and the carta reader for each. Carried from textweaver.
pub const FORMATS: &[(&str, &str)] = &[
    ("org", "org"),
    ("rst", "rst"),
    ("rest", "rst"),
    ("mediawiki", "mediawiki"),
    ("wiki", "mediawiki"),
    ("dokuwiki", "dokuwiki"),
    ("jira", "jira"),
];

/// Extensions (and format names) the carta bridge claims. Carried from
/// textweaver.
pub const EXTENSIONS: &[&str] = &[
    "org",
    "rst",
    "rest",
    "mediawiki",
    "wiki",
    "dokuwiki",
    "jira",
];

/// The formats carta writes from Markdown for conversions (alpha.7): its
/// writer names, for the formats nothing else writes. Carried from
/// textweaver; not built into alpha.2, whose carta has the HTML writer
/// only.
pub const WRITERS: &[&str] = &["asciidoc", "typst", "latex", "mediawiki", "org"];

/// The note a view gets when an include was left out. Carried from
/// textweaver.
pub const INCLUDE_WARNING: &str =
    "An include directive was left out, so the file it names is not read.";

/// A format carta reads for the quick view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CartaFormat {
    /// Org mode.
    Org,
    /// reStructuredText.
    Rst,
    /// MediaWiki markup.
    MediaWiki,
    /// DokuWiki markup.
    DokuWiki,
    /// Jira markup.
    Jira,
}

impl CartaFormat {
    /// Every format, in [`FORMATS`] order.
    pub const ALL: [CartaFormat; 5] = [
        CartaFormat::Org,
        CartaFormat::Rst,
        CartaFormat::MediaWiki,
        CartaFormat::DokuWiki,
        CartaFormat::Jira,
    ];

    /// carta's reader name.
    pub const fn reader(self) -> &'static str {
        match self {
            CartaFormat::Org => "org",
            CartaFormat::Rst => "rst",
            CartaFormat::MediaWiki => "mediawiki",
            CartaFormat::DokuWiki => "dokuwiki",
            CartaFormat::Jira => "jira",
        }
    }

    /// The format's name as said: "Org", "reStructuredText".
    pub const fn name(self) -> &'static str {
        match self {
            CartaFormat::Org => "Org",
            CartaFormat::Rst => "reStructuredText",
            CartaFormat::MediaWiki => "MediaWiki",
            CartaFormat::DokuWiki => "DokuWiki",
            CartaFormat::Jira => "Jira",
        }
    }

    /// The format a file name's extension says, through [`FORMATS`].
    pub fn from_name(file_name: &str) -> Option<Self> {
        let (_, ext) = file_name.rsplit_once('.')?;
        let reader = FORMATS
            .iter()
            .find(|(e, _)| e.eq_ignore_ascii_case(ext))
            .map(|(_, r)| *r)?;
        Self::ALL.into_iter().find(|f| f.reader() == reader)
    }
}

/// The source with every reStructuredText `.. include::` directive turned
/// into a comment (its indented options become the comment's body), so
/// carta never reads another file. Carried from textweaver, with
/// diroxide's comment words.
///
/// Line based, so an include written inside a literal block (an example
/// in a code listing) is commented out too.
pub fn without_includes(text: &str) -> Cow<'_, str> {
    let is_include = |line: &str| {
        line.trim_start()
            .strip_prefix("..")
            .map(str::trim_start)
            .and_then(|rest| rest.split_once("::"))
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case("include"))
    };
    if !text.lines().any(is_include) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if is_include(line) {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(indent);
            out.push_str(".. diroxide left out an include\n");
        } else {
            out.push_str(line);
        }
    }
    Cow::Owned(out)
}

/// carta's HTML fragment for a text in a format, the document's title
/// from its metadata (Org's `#+TITLE`), and whether an include was left
/// out.
pub fn to_html(
    format: CartaFormat,
    text: &str,
) -> Result<(String, Option<String>, bool), DocError> {
    use carta::ast::MetaValue;
    let text = if format == CartaFormat::Rst {
        without_includes(text)
    } else {
        Cow::Borrowed(text)
    };
    let skipped = matches!(text, Cow::Owned(_));
    let not_valid = |_: carta::Error| DocError::NotValid(format.name().to_owned());
    let (doc, media) = carta::read_document(
        format.reader(),
        text.as_bytes(),
        &carta::ReaderOptions::default(),
    )
    .map_err(not_valid)?;
    let title = match doc.meta.get("title") {
        Some(MetaValue::MetaString(t)) => Some(t.to_string()),
        Some(MetaValue::MetaInlines(i)) => Some(carta::ast::to_plain_text(i)),
        _ => None,
    }
    .filter(|t| !t.trim().is_empty());
    match carta::render_document("html", doc, media, &carta::WriterOptions::default())
        .map_err(not_valid)?
    {
        carta::Output::Text(html) => Ok((html, title, skipped)),
        carta::Output::Bytes(_) => Err(DocError::NotValid(format.name().to_owned())),
    }
}

/// Reads a file's bytes in a carta format into a view: decoded first,
/// includes left out, carta's HTML read into rows. Unguarded: the quick
/// view reads through [`super::read`], which runs this under the guard.
pub fn read(format: CartaFormat, bytes: &[u8]) -> Result<DocView, DocError> {
    let text = decode(bytes, None).text;
    let (fragment, title, skipped) = to_html(format, &text)?;
    let mut view = html::read_fragment(&fragment, DocKind::Carta(format.name().to_owned()));
    if let Some(title) = title {
        view.title = Some(super::build::collapse(&title));
    }
    if skipped {
        view.add_note(INCLUDE_WARNING);
    }
    Ok(view)
}
