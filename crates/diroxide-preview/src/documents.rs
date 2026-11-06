//! Documents as rows (alpha 4, R30 commits 34 and 37): the previewer for
//! every format the quick view's readers ([`crate::doc`]) read, chosen by
//! the file's checked type before its name ([`Reader::for_type`]). It
//! reads at most [`DocPreviewer::max_bytes`] of the file, runs the reader
//! under the guard, and when the reader cannot read it shows the text,
//! with why. The rows travel in [`Preview::document`], so the quick view
//! moves by heading, table and link.
//!
//! The summary says the format first, then the document's own title,
//! then what is in it: "SVG, 640 by 480, titled Sales by quarter",
//! "Markdown, Installation guide, 12 headings".

use std::io::Read;

use diroxide_core::{DetailGroup, DetailRow, EntryKind};
use diroxide_filetype::FileType;
use diroxide_vfs::{OpContext, Registry, VfsError};

use crate::doc::{self, DocError, DocView, ReadOptions, Reader, svg};
use crate::{Preview, PreviewError, PreviewKind, PreviewRequest, Previewer};

/// The quick view's read cap by default (`quick_view.max_kib`, 1 MiB).
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;

/// The documents previewer. See the module documentation.
#[derive(Clone, Debug)]
pub struct DocPreviewer {
    /// How documents are read: the guard and its deadline.
    pub options: ReadOptions,
    /// The most bytes of a file read.
    pub max_bytes: u64,
}

impl Default for DocPreviewer {
    fn default() -> Self {
        Self {
            options: ReadOptions::default(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// The format's word for the summary: the type's short name, else the
/// reader's own.
fn format_word(file_type: Option<&FileType>, view: &DocView) -> String {
    file_type
        .map(|t| t.short.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| view.kind.word().map(str::to_owned))
        .unwrap_or_else(|| "Document".to_owned())
}

/// The summary of a document read into rows: "DOCX, Example Report, 14
/// headings".
pub fn summary(word: &str, view: &DocView) -> String {
    let mut parts = vec![word.to_owned()];
    if let Some(title) = &view.title {
        parts.push(title.clone());
    }
    parts.push(view.summary_words());
    parts.join(", ")
}

/// A document preview: the summary, the rows as lines (at most
/// `max_lines`), the entry's rows and a "Contents" row, and the view.
pub fn document_preview(
    summary: String,
    view: DocView,
    base: Vec<DetailRow>,
    error: Option<&DocError>,
    max_lines: usize,
) -> Preview {
    let mut more = vec![DetailRow::new(
        "doc.contents",
        "Contents",
        view.summary_words(),
        DetailGroup::Content,
    )];
    if let Some(error) = error {
        more.push(DetailRow::new(
            "doc.read",
            "Read as",
            format!("text, {error}"),
            DetailGroup::Technical,
        ));
    }
    let mut preview = crate::details_preview(PreviewKind::Document, summary, base, more);
    preview.lines = view
        .rows()
        .iter()
        .take(max_lines)
        .map(doc::DocRow::shown)
        .collect();
    preview.document = Some(view);
    preview
}

impl Previewer for DocPreviewer {
    fn name(&self) -> &str {
        "document"
    }

    fn version(&self) -> u32 {
        1
    }

    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
        request.entry.kind == EntryKind::File
            && Reader::for_type(file_type, &request.entry.name) != Reader::Text
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let provider = registry.resolve(&request.place)?;
        let mut bytes = Vec::new();
        provider
            .read(&request.place, ctx)?
            .take(self.max_bytes)
            .read_to_end(&mut bytes)
            .map_err(VfsError::from)?;
        ctx.check()?;
        let name = &request.entry.name;
        let base = crate::detail_rows(&request.entry, file_type);
        let reader = Reader::for_type(file_type, name);
        if reader == Reader::Svg {
            let read = self.options.guard.run({
                let bytes = bytes.clone();
                move || svg::reading(&bytes)
            });
            if let Ok(Ok((reading, flattened))) = read {
                let word = file_type.map_or(svg::KIND, |t| t.short.as_str());
                let line = svg::sentence(&reading, word, request.entry.size);
                let view = svg::view_of(&reading, flattened);
                return Ok(document_preview(line, view, base, None, request.max_lines));
            }
        }
        let (view, error) = doc::read_as_or_text(reader, name, bytes, &self.options);
        let line = summary(&format_word(file_type, &view), &view);
        Ok(document_preview(
            line,
            view,
            base,
            error.as_ref(),
            request.max_lines,
        ))
    }
}
