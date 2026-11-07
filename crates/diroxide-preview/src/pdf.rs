//! PDF in words (alpha 4, R30 commit 39; answer 120): the first three
//! pages' text through the tools registry's `pdftotext` (`-f 1 -l 3`),
//! run as an argument list through the context's runner, ten seconds and
//! the first 64 KiB of its text; the properties (title, author, pages,
//! version, encryption) from `lopdf`'s metadata reader, under the guard,
//! from at most [`MAX_PROPERTIES_BYTES`] of the file. Without the tool:
//! "No preview: pdftotext not found", the properties still said. The
//! pure-Rust reading-order reader (textweaver's, about 6,900 lines) comes
//! in a later alpha.
//!
//! The text is read from a file on a local disk only: Poppler's and
//! Xpdf's `pdftotext` do not both read standard input, and a PDF is read
//! at random. A file elsewhere gets its properties alone.
//!
//! The sentence: "PDF, 12 pages, Example Report"; without a title, "PDF,
//! 12 pages, 1.2 MB". The text is a heading a page ("Page 1"), its
//! paragraphs below, then "and 9 more pages".

use std::ops::ControlFlow;
use std::time::Duration;

use diroxide_core::format::human_size;
use diroxide_core::{DetailGroup, DetailRow, EntryKind};
use diroxide_filetype::FileType;
use diroxide_meta::Field;
use diroxide_vfs::{
    OpContext, Registry, RunLimits, ToolRequest, ToolRunError, ToolStream, VfsError,
};

use crate::doc::{DocBuilder, DocKind, DocView, Guard};
use crate::words::{clean_text, count_words};
use crate::{Preview, PreviewError, PreviewKind, PreviewRequest, Previewer};

/// The tools registry's id for pdftotext.
pub const PDFTOTEXT: &str = "pdftotext";

/// What the preview says when pdftotext is not installed.
pub const NOT_FOUND: &str = "No preview: pdftotext not found";

/// The pages the preview reads.
pub const PAGES: u32 = 3;

/// The most bytes of text kept.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// The run's limits: ten seconds; the output is cut at
/// [`MAX_TEXT_BYTES`] by the reader, this cap only a backstop.
pub const LIMITS: RunLimits = RunLimits::new(Duration::from_secs(10), 1024 * 1024);

/// The most bytes of a file the properties are read from.
pub const MAX_PROPERTIES_BYTES: u64 = 64 * 1024 * 1024;

/// The most characters kept of a property.
const MAX_PROPERTY: usize = 200;

/// A PDF's properties, every string cleaned like a file name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PdfProperties {
    /// The title.
    pub title: Option<String>,
    /// The author.
    pub author: Option<String>,
    /// The subject.
    pub subject: Option<String>,
    /// The program that made it.
    pub creator: Option<String>,
    /// The program that wrote the PDF.
    pub producer: Option<String>,
    /// How many pages.
    pub pages: u32,
    /// The PDF version: "1.7".
    pub version: String,
    /// Whether it is encrypted.
    pub encrypted: bool,
}

/// Reads a PDF's properties from its bytes, through `lopdf`'s metadata
/// reader, which does not load the pages. Unguarded: the previewer runs
/// it under the guard.
pub fn properties(bytes: &[u8]) -> Option<PdfProperties> {
    let meta = lopdf::Document::load_metadata_mem(bytes).ok()?;
    let clean = |v: Option<String>| {
        v.map(|t| clean_text(&t, MAX_PROPERTY))
            .filter(|t| !t.is_empty())
    };
    Some(PdfProperties {
        title: clean(meta.title),
        author: clean(meta.author),
        subject: clean(meta.subject),
        creator: clean(meta.creator),
        producer: clean(meta.producer),
        pages: meta.page_count,
        version: clean_text(&meta.version, 16),
        encrypted: meta.encrypted,
    })
}

impl PdfProperties {
    /// The sentence: "PDF, 12 pages, Example Report", or the file's size
    /// when it has no title.
    pub fn sentence(&self, word: &str, file_size: Option<u64>) -> String {
        let mut parts = vec![word.to_owned()];
        if self.pages > 0 {
            parts.push(count_words(u64::from(self.pages), "page", "pages"));
        }
        if self.encrypted {
            parts.push("encrypted".to_owned());
        }
        match (&self.title, file_size) {
            (Some(title), _) => parts.push(title.clone()),
            (None, Some(size)) => parts.push(human_size(size)),
            (None, None) => {}
        }
        parts.join(", ")
    }

    /// The properties as metadata fields.
    pub fn fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        for (key, label, value) in [
            ("pdf.title", "Title", &self.title),
            ("pdf.author", "Author", &self.author),
            ("pdf.subject", "Subject", &self.subject),
        ] {
            if let Some(value) = value {
                out.push(Field::new(key, label, value, DetailGroup::Content));
            }
        }
        if self.pages > 0 {
            out.push(Field::new(
                "pdf.pages",
                "Pages",
                diroxide_core::format::grouped(u64::from(self.pages)),
                DetailGroup::Content,
            ));
        }
        if self.encrypted {
            out.push(Field::new(
                "pdf.encrypted",
                "Encrypted",
                "yes",
                DetailGroup::Flags,
            ));
        }
        for (key, label, value) in [
            ("pdf.creator", "Made with", &self.creator),
            ("pdf.producer", "Written by", &self.producer),
        ] {
            if let Some(value) = value {
                out.push(Field::new(key, label, value, DetailGroup::Source));
            }
        }
        if !self.version.is_empty() {
            out.push(Field::new(
                "pdf.version",
                "PDF version",
                &self.version,
                DetailGroup::Technical,
            ));
        }
        out
    }
}

/// pdftotext's argument list for a local file: the first [`PAGES`]
/// pages, UTF-8, no messages, the text on standard output.
pub fn arguments(path: &str) -> Vec<String> {
    vec![
        "-f".to_owned(),
        "1".to_owned(),
        "-l".to_owned(),
        PAGES.to_string(),
        "-enc".to_owned(),
        "UTF-8".to_owned(),
        "-q".to_owned(),
        path.to_owned(),
        "-".to_owned(),
    ]
}

/// Why the text could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextError {
    /// pdftotext is not installed.
    NotFound,
    /// The run failed, in words.
    Failed(String),
    /// The run was cancelled.
    Cancelled,
}

/// Runs pdftotext on a local file and keeps at most [`MAX_TEXT_BYTES`]
/// of its text, stopping it there.
pub fn text(ctx: &OpContext, path: &str) -> Result<String, TextError> {
    if !ctx.tools.has(PDFTOTEXT) {
        return Err(TextError::NotFound);
    }
    let request = ToolRequest::new(PDFTOTEXT, arguments(path)).with_limits(LIMITS);
    let mut kept = String::new();
    let output = ctx
        .tools
        .run_streaming(&request, &ctx.cancel, &mut |stream, line| {
            if stream != ToolStream::Stdout {
                return ControlFlow::Continue(());
            }
            if kept.len() + line.len() + 1 > MAX_TEXT_BYTES {
                return ControlFlow::Break(());
            }
            kept.push_str(line);
            kept.push('\n');
            ControlFlow::Continue(())
        })
        .map_err(|e| match e {
            ToolRunError::NotInstalled { .. } => TextError::NotFound,
            ToolRunError::Cancelled => TextError::Cancelled,
            other => TextError::Failed(other.to_string()),
        })?;
    if output.status.is_some_and(|s| s != 0) && kept.trim().is_empty() {
        return Err(TextError::Failed("pdftotext could not read it".to_owned()));
    }
    Ok(kept)
}

/// The text as rows: a heading a page ("Page 1"), the page's paragraphs
/// (lines joined until a blank line), then the pages not read.
pub fn view_of(text: &str, pages: u32) -> DocView {
    let mut b = DocBuilder::new(DocKind::Format("PDF".to_owned()));
    let shown: Vec<&str> = text.split('\u{c}').collect();
    let mut written = 0u32;
    for (n, page) in shown.iter().enumerate() {
        if page.trim().is_empty() && n + 1 == shown.len() {
            break;
        }
        written += 1;
        b.heading(2);
        b.text(&format!("Page {}", n + 1));
        b.end();
        let mut open = false;
        for line in page.lines() {
            if line.trim().is_empty() {
                if open {
                    b.end();
                    open = false;
                }
                continue;
            }
            if !open {
                b.paragraph();
                open = true;
            } else {
                b.space();
            }
            b.text(line);
        }
        if open {
            b.end();
        }
    }
    if pages > written {
        let left = pages - written;
        b.paragraph();
        b.text(&if left == 1 {
            "and 1 more page".to_owned()
        } else {
            format!(
                "and {} more pages",
                diroxide_core::format::grouped(u64::from(left))
            )
        });
        b.end();
    }
    b.finish()
}

/// The PDF previewer: the properties, and the first pages' text when
/// pdftotext is installed. Tool-backed, like ffprobe: the app runs it
/// after the quick view's wait, in the details list or on Preview.
#[derive(Clone, Debug, Default)]
pub struct PdfPreviewer {
    /// The guard the properties are read under.
    pub guard: Guard,
}

impl Previewer for PdfPreviewer {
    fn name(&self) -> &str {
        "pdf"
    }

    fn version(&self) -> u32 {
        1
    }

    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
        request.entry.kind == EntryKind::File
            && match file_type {
                Some(t) => t.id == "pdf",
                None => request
                    .entry
                    .name
                    .rsplit_once('.')
                    .is_some_and(|(_, e)| e.eq_ignore_ascii_case("pdf")),
            }
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let word = file_type.map_or_else(|| "PDF".to_owned(), |t| t.short.clone());
        let base = crate::detail_rows(&request.entry, file_type);
        let props = match crate::open_source(request, registry, ctx, MAX_PROPERTIES_BYTES) {
            Ok(mut source) => {
                let bytes = crate::doc::package::read_all(
                    source.as_mut(),
                    &crate::doc::package::ContainerCaps {
                        max_container: MAX_PROPERTIES_BYTES,
                        ..Default::default()
                    },
                )
                .ok();
                bytes.and_then(|bytes| self.guard.run(move || properties(&bytes)).ok().flatten())
            }
            Err(PreviewError::TooLarge) => None,
            Err(error) => return Err(error),
        };
        ctx.check()?;
        let mut rows: Vec<DetailRow> = props
            .as_ref()
            .map(|p| p.fields().iter().map(Field::row).collect())
            .unwrap_or_default();
        let pages = props.as_ref().map_or(0, |p| p.pages);
        let text = match request.place.local_path() {
            Some(path) => text(ctx, path),
            None => {
                rows.push(DetailRow::new(
                    "pdf.text",
                    "Text",
                    "not read here, the file is not on a local disk",
                    DetailGroup::Technical,
                ));
                Err(TextError::Failed(String::new()))
            }
        };
        let summary = match (&text, &props) {
            (Err(TextError::NotFound), _) => NOT_FOUND.to_owned(),
            (_, Some(p)) => p.sentence(&word, request.entry.size),
            (_, None) => format!("{word}, unreadable"),
        };
        match text {
            Ok(text) => Ok(crate::documents::document_preview(
                summary,
                view_of(&text, pages),
                [base, rows].concat(),
                None,
                request.max_lines,
            )),
            Err(TextError::Cancelled) => Err(PreviewError::Vfs(VfsError::Cancelled)),
            Err(error) => {
                if let TextError::Failed(words) = &error
                    && !words.is_empty()
                {
                    rows.push(DetailRow::new(
                        "pdf.text",
                        "Text",
                        format!("not read, {words}"),
                        DetailGroup::Technical,
                    ));
                }
                Ok(crate::details_preview(
                    PreviewKind::Document,
                    summary,
                    base,
                    rows,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
