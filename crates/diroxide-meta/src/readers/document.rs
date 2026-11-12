//! Document properties (R12, "Document metadata"; the owner's answer
//! 120): PDF through `lopdf`'s metadata reader, Office Open XML through
//! `docProps/core.xml` and `docProps/app.xml`, OpenDocument through
//! `meta.xml`, and EPUB through its package document's `dc:` elements,
//! each XML part read through `zip` and `roxmltree` with a size cap and a
//! node cap, and never a DTD.
//!
//! Title and author are said. A word processor's page count is what the
//! program wrote when it last saved, often stale, so it is said as such:
//! "Pages: 12, as last saved, may be stale". The people a file names
//! (author, last saved by, company, manager) and its comments are
//! private: left out of Copy details unless asked, and named by Check
//! before sharing. A published book's author is not private.

use std::io::{Read, Seek, SeekFrom};

use diroxide_core::details::DetailGroup;
use diroxide_vfs::ReadSeek;
use roxmltree::{Document, Node, ParsingOptions};

use crate::field::Field;
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
use crate::words::{grouped, iso_date_words, pdf_date_words, yes_no};

/// The most bytes of one XML part read for its properties.
pub const PART_CAP: u64 = 4 * 1024 * 1024;

/// The most nodes of one XML part parsed.
pub const NODE_CAP: u32 = 100_000;

// ---------------------------------------------------------------- PDF

/// PDF properties through `lopdf`: `doc.title`, `doc.author` and the
/// rest of the Info dictionary, `doc.pages`, `doc.pdf_version`,
/// `doc.encrypted`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfProperties;

impl Reader for PdfProperties {
    fn id(&self) -> &'static str {
        "pdf"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "doc.title",
            "doc.author",
            "doc.subject",
            "doc.keywords",
            "doc.application",
            "doc.producer",
            "doc.created",
            "doc.modified",
            "doc.pages",
            "doc.pdf_version",
            "doc.encrypted",
            "doc.custom",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(request, &["pdf"], &[".pdf"])
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        // lopdf reads the whole file; a larger one is refused before a
        // byte is read, and `pdfinfo` is the fallback.
        let size = source
            .seek(SeekFrom::End(0))
            .map_err(|e| MetaError::from_io(&e))?;
        if size > limits.max_bytes {
            return Err(MetaError::TooLarge);
        }
        source
            .seek(SeekFrom::Start(0))
            .map_err(|e| MetaError::from_io(&e))?;
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        source
            .read_to_end(&mut bytes)
            .map_err(|e| MetaError::from_io(&e))?;
        pdf_fields(&bytes)
    }
}

/// The PDF properties of some bytes.
pub(crate) fn pdf_fields(bytes: &[u8]) -> Result<Vec<Field>, MetaError> {
    if !bytes.windows(5).take(1024).any(|w| w == b"%PDF-") {
        return Err(MetaError::unreadable("not a PDF"));
    }
    let meta = lopdf::Document::load_metadata_mem(bytes)
        .map_err(|_| MetaError::unreadable("the PDF is damaged"))?;
    let mut fields = Fields::default();
    fields.text(
        "doc.title",
        "Title",
        meta.title.as_deref(),
        DetailGroup::Content,
    );
    fields.private(
        "doc.author",
        "Author",
        meta.author.as_deref(),
        DetailGroup::Source,
    );
    fields.text(
        "doc.subject",
        "Subject",
        meta.subject.as_deref(),
        DetailGroup::Content,
    );
    fields.text(
        "doc.keywords",
        "Keywords",
        meta.keywords.as_deref(),
        DetailGroup::Content,
    );
    fields.text(
        "doc.application",
        "Made with",
        meta.creator.as_deref(),
        DetailGroup::Source,
    );
    fields.text(
        "doc.producer",
        "PDF made by",
        meta.producer.as_deref(),
        DetailGroup::Source,
    );
    fields.date(
        "doc.created",
        "Document created",
        meta.creation_date.as_deref().and_then(pdf_date_words),
    );
    fields.date(
        "doc.modified",
        "Document modified",
        meta.modification_date.as_deref().and_then(pdf_date_words),
    );
    if meta.page_count > 0 {
        fields.push(
            Field::new(
                "doc.pages",
                "Pages",
                grouped(u64::from(meta.page_count)),
                DetailGroup::Content,
            )
            .sorted_by_count(u64::from(meta.page_count)),
        );
    }
    fields.text(
        "doc.pdf_version",
        "PDF version",
        Some(meta.version.as_str()),
        DetailGroup::Technical,
    );
    let encrypted = if meta.encrypted && meta.page_count == 0 {
        "yes, properties locked"
    } else {
        yes_no(meta.encrypted)
    };
    fields.push(Field::new(
        "doc.encrypted",
        "Encrypted",
        encrypted,
        DetailGroup::Flags,
    ));
    if !meta.custom.is_empty() {
        fields.push(
            Field::new(
                "doc.custom",
                "Custom properties",
                grouped(meta.custom.len() as u64),
                DetailGroup::Technical,
            )
            .sorted_by_count(meta.custom.len() as u64),
        );
    }
    Ok(fields.0)
}

// ---------------------------------------------------------------- zip parts

/// One member of a zip as text, at most [`PART_CAP`] bytes, or `None`
/// when the zip has no such member. A UTF-16 part is decoded; anything
/// else is read as UTF-8, a broken sequence shown as a stand-in.
fn member_text<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>, MetaError> {
    let mut member = match archive.by_name(name) {
        Ok(member) => member,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err(MetaError::unreadable("a part of the file is damaged")),
    };
    if member.size() > PART_CAP {
        return Err(MetaError::TooLarge);
    }
    let mut bytes = Vec::new();
    (&mut member)
        .take(PART_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| MetaError::unreadable("a part of the file is damaged"))?;
    if bytes.len() as u64 > PART_CAP {
        return Err(MetaError::TooLarge);
    }
    Ok(Some(decode(&bytes)))
}

/// XML bytes as text, by their byte-order mark.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |little: bool, rest: &[u8]| -> String {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| {
                if little {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(true, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(false, rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Parses XML with no DTD and at most [`NODE_CAP`] nodes.
fn parse(text: &str) -> Result<Document<'_>, MetaError> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: NODE_CAP,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options)
        .map_err(|_| MetaError::unreadable("its properties are damaged"))
}

/// The text of the first element child of `parent` with this local name.
fn child_text(parent: Node<'_, '_>, local: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == local)
        .map(element_text)
        .filter(|t| !t.is_empty())
}

/// Every element child of `parent` with this local name, as text.
fn children_text(parent: Node<'_, '_>, local: &str) -> Vec<String> {
    parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == local)
        .map(element_text)
        .filter(|t| !t.is_empty())
        .collect()
}

/// An element's text, its descendants' text joined.
fn element_text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The zip, refused in words when it is not one.
fn open_zip(source: &mut dyn ReadSeek) -> Result<zip::ZipArchive<&mut dyn ReadSeek>, MetaError> {
    zip::ZipArchive::new(source).map_err(|error| match error {
        zip::result::ZipError::Io(io) => MetaError::from_io(&io),
        _ => MetaError::unreadable("not a zip package, or a damaged one"),
    })
}

/// Fields as they are found, skipping the empty ones.
#[derive(Default)]
struct Fields(Vec<Field>);

impl Fields {
    fn push(&mut self, field: Field) {
        self.0.push(field);
    }

    fn text(&mut self, key: &str, label: &str, value: Option<&str>, group: DetailGroup) {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            self.0.push(Field::new(key, label, value, group));
        }
    }

    fn private(&mut self, key: &str, label: &str, value: Option<&str>, group: DetailGroup) {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            self.0.push(Field::new(key, label, value, group).private());
        }
    }

    fn date(&mut self, key: &str, label: &str, value: Option<String>) {
        if let Some(value) = value {
            self.0
                .push(Field::new(key, label, value, DetailGroup::Dates));
        }
    }

    fn number(&mut self, key: &str, label: &str, value: Option<&str>, suffix: &str) {
        if let Some(n) = value.and_then(|v| v.trim().parse::<u64>().ok()) {
            self.0.push(
                Field::new(
                    key,
                    label,
                    format!("{}{suffix}", grouped(n)),
                    DetailGroup::Content,
                )
                .sorted_by_count(n),
            );
        }
    }
}

// ---------------------------------------------------------------- Office

/// Office Open XML properties: `docProps/core.xml` and `docProps/app.xml`
/// of a DOCX, XLSX or PPTX, and whether a word processing file holds
/// review comments.
#[derive(Clone, Copy, Debug, Default)]
pub struct OfficeProperties;

impl Reader for OfficeProperties {
    fn id(&self) -> &'static str {
        "office"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "doc.title",
            "doc.subject",
            "doc.author",
            "doc.keywords",
            "doc.comments",
            "doc.category",
            "doc.last_saved_by",
            "doc.revision",
            "doc.created",
            "doc.modified",
            "doc.application",
            "doc.company",
            "doc.manager",
            "doc.template",
            "doc.pages",
            "doc.words",
            "doc.slides",
            "doc.review_comments",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(
            request,
            &["docx", "xlsx", "pptx", "ooxml"],
            &[
                ".docx", ".docm", ".dotx", ".xlsx", ".xlsm", ".xltx", ".pptx", ".pptm", ".potx",
            ],
        )
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let mut archive = open_zip(source)?;
        let mut fields = Fields::default();
        let core = member_text(&mut archive, "docProps/core.xml")?;
        let app = member_text(&mut archive, "docProps/app.xml")?;
        if core.is_none()
            && app.is_none()
            && archive.index_for_name("[Content_Types].xml").is_none()
        {
            return Err(MetaError::unreadable("not an Office document"));
        }
        if let Some(core) = core {
            let doc = parse(&core)?;
            let root = doc.root_element();
            fields.text(
                "doc.title",
                "Title",
                child_text(root, "title").as_deref(),
                DetailGroup::Content,
            );
            fields.text(
                "doc.subject",
                "Subject",
                child_text(root, "subject").as_deref(),
                DetailGroup::Content,
            );
            fields.private(
                "doc.author",
                "Author",
                child_text(root, "creator").as_deref(),
                DetailGroup::Source,
            );
            fields.text(
                "doc.keywords",
                "Keywords",
                child_text(root, "keywords").as_deref(),
                DetailGroup::Content,
            );
            fields.private(
                "doc.comments",
                "Comments",
                child_text(root, "description").as_deref(),
                DetailGroup::Content,
            );
            fields.text(
                "doc.category",
                "Category",
                child_text(root, "category").as_deref(),
                DetailGroup::Content,
            );
            fields.private(
                "doc.last_saved_by",
                "Last saved by",
                child_text(root, "lastModifiedBy").as_deref(),
                DetailGroup::Source,
            );
            fields.text(
                "doc.revision",
                "Revision",
                child_text(root, "revision").as_deref(),
                DetailGroup::Technical,
            );
            fields.date(
                "doc.created",
                "Document created",
                child_text(root, "created")
                    .as_deref()
                    .and_then(iso_date_words),
            );
            fields.date(
                "doc.modified",
                "Document modified",
                child_text(root, "modified")
                    .as_deref()
                    .and_then(iso_date_words),
            );
        }
        if let Some(app) = app {
            let doc = parse(&app)?;
            let root = doc.root_element();
            fields.text(
                "doc.application",
                "Made with",
                child_text(root, "Application").as_deref(),
                DetailGroup::Source,
            );
            fields.private(
                "doc.company",
                "Company",
                child_text(root, "Company").as_deref(),
                DetailGroup::Source,
            );
            fields.private(
                "doc.manager",
                "Manager",
                child_text(root, "Manager").as_deref(),
                DetailGroup::Source,
            );
            fields.text(
                "doc.template",
                "Template",
                child_text(root, "Template").as_deref(),
                DetailGroup::Source,
            );
            // Only a word processor's page count is a guess at the pages;
            // it is the count when the file was last saved.
            fields.number(
                "doc.pages",
                "Pages",
                child_text(root, "Pages").as_deref(),
                ", as last saved, may be stale",
            );
            fields.number(
                "doc.words",
                "Words",
                child_text(root, "Words").as_deref(),
                "",
            );
            fields.number(
                "doc.slides",
                "Slides",
                child_text(root, "Slides").as_deref(),
                "",
            );
        }
        let review = archive.file_names().any(|name| {
            name.as_deref().is_ok_and(|n| {
                n == "word/comments.xml"
                    || n.starts_with("xl/comments")
                    || n.starts_with("ppt/comments/")
            })
        });
        if review {
            fields.push(
                Field::new(
                    "doc.review_comments",
                    "Review comments",
                    "yes",
                    DetailGroup::Content,
                )
                .private(),
            );
        }
        Ok(fields.0)
    }
}

// ---------------------------------------------------------------- ODF

/// OpenDocument properties from `meta.xml`.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenDocumentProperties;

impl Reader for OpenDocumentProperties {
    fn id(&self) -> &'static str {
        "opendocument"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "doc.title",
            "doc.subject",
            "doc.author",
            "doc.keywords",
            "doc.comments",
            "doc.last_saved_by",
            "doc.revision",
            "doc.created",
            "doc.modified",
            "doc.application",
            "doc.pages",
            "doc.words",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(
            request,
            &["odt", "ods", "odp", "odg", "opendocument"],
            &[".odt", ".ods", ".odp", ".odg", ".ott", ".ots", ".otp"],
        )
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let mut archive = open_zip(source)?;
        let Some(meta) = member_text(&mut archive, "meta.xml")? else {
            return if archive.index_for_name("mimetype").is_some() {
                Ok(Vec::new())
            } else {
                Err(MetaError::unreadable("not an OpenDocument file"))
            };
        };
        let doc = parse(&meta)?;
        let Some(office_meta) = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "meta")
        else {
            return Ok(Vec::new());
        };
        let mut fields = Fields::default();
        fields.text(
            "doc.title",
            "Title",
            child_text(office_meta, "title").as_deref(),
            DetailGroup::Content,
        );
        fields.text(
            "doc.subject",
            "Subject",
            child_text(office_meta, "subject").as_deref(),
            DetailGroup::Content,
        );
        fields.private(
            "doc.author",
            "Author",
            child_text(office_meta, "initial-creator").as_deref(),
            DetailGroup::Source,
        );
        let keywords = children_text(office_meta, "keyword").join(", ");
        fields.text(
            "doc.keywords",
            "Keywords",
            Some(&keywords),
            DetailGroup::Content,
        );
        fields.private(
            "doc.comments",
            "Comments",
            child_text(office_meta, "description").as_deref(),
            DetailGroup::Content,
        );
        fields.private(
            "doc.last_saved_by",
            "Last saved by",
            child_text(office_meta, "creator").as_deref(),
            DetailGroup::Source,
        );
        fields.text(
            "doc.revision",
            "Revision",
            child_text(office_meta, "editing-cycles").as_deref(),
            DetailGroup::Technical,
        );
        fields.date(
            "doc.created",
            "Document created",
            child_text(office_meta, "creation-date")
                .as_deref()
                .and_then(iso_date_words),
        );
        fields.date(
            "doc.modified",
            "Document modified",
            child_text(office_meta, "date")
                .as_deref()
                .and_then(iso_date_words),
        );
        fields.text(
            "doc.application",
            "Made with",
            child_text(office_meta, "generator").as_deref(),
            DetailGroup::Source,
        );
        if let Some(stats) = office_meta
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "document-statistic")
        {
            let attr = |local: &str| {
                stats
                    .attributes()
                    .find(|a| a.name() == local)
                    .map(|a| a.value().to_owned())
            };
            fields.number(
                "doc.pages",
                "Pages",
                attr("page-count").as_deref(),
                ", as last saved, may be stale",
            );
            fields.number("doc.words", "Words", attr("word-count").as_deref(), "");
        }
        Ok(fields.0)
    }
}

// ---------------------------------------------------------------- EPUB

/// EPUB properties: the package document's `dc:` elements, found through
/// `META-INF/container.xml`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EpubProperties;

impl Reader for EpubProperties {
    fn id(&self) -> &'static str {
        "epub"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "doc.title",
            "doc.author",
            "doc.language",
            "doc.publisher",
            "doc.published",
            "doc.subject",
            "doc.identifier",
            "doc.rights",
            "doc.comments",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(request, &["epub"], &[".epub"])
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let mut archive = open_zip(source)?;
        let container = member_text(&mut archive, "META-INF/container.xml")?
            .ok_or_else(|| MetaError::unreadable("not an EPUB: it has no container"))?;
        let path = {
            let doc = parse(&container)?;
            doc.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "rootfile")
                .and_then(|n| n.attribute("full-path"))
                .map(str::to_owned)
                .ok_or_else(|| MetaError::unreadable("not an EPUB: it names no book"))?
        };
        let package = member_text(&mut archive, path.trim_start_matches('/'))?
            .ok_or_else(|| MetaError::unreadable("the EPUB's book file is missing"))?;
        let doc = parse(&package)?;
        let Some(metadata) = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "metadata")
        else {
            return Ok(Vec::new());
        };
        let mut fields = Fields::default();
        fields.text(
            "doc.title",
            "Title",
            child_text(metadata, "title").as_deref(),
            DetailGroup::Content,
        );
        let authors = children_text(metadata, "creator").join(", ");
        fields.text("doc.author", "Author", Some(&authors), DetailGroup::Source);
        fields.text(
            "doc.language",
            "Language",
            child_text(metadata, "language").as_deref(),
            DetailGroup::Content,
        );
        fields.text(
            "doc.publisher",
            "Publisher",
            child_text(metadata, "publisher").as_deref(),
            DetailGroup::Source,
        );
        fields.date(
            "doc.published",
            "Published",
            child_text(metadata, "date")
                .as_deref()
                .and_then(iso_date_words),
        );
        let subjects = children_text(metadata, "subject").join(", ");
        fields.text(
            "doc.subject",
            "Subject",
            Some(&subjects),
            DetailGroup::Content,
        );
        fields.text(
            "doc.identifier",
            "Identifier",
            child_text(metadata, "identifier").as_deref(),
            DetailGroup::Technical,
        );
        fields.text(
            "doc.rights",
            "Rights",
            child_text(metadata, "rights").as_deref(),
            DetailGroup::Source,
        );
        fields.text(
            "doc.comments",
            "Description",
            child_text(metadata, "description").as_deref(),
            DetailGroup::Content,
        );
        Ok(fields.0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use diroxide_core::{Entry, EntryKind, Place};

    use super::*;
    use crate::readers::archive::tests::zip_of;

    /// A one-page PDF made in memory with an Info dictionary.
    pub(crate) fn pdf(info: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
            format!("<< {info} >>"),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    pub(crate) fn lines(
        reader: &dyn Reader,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<Vec<String>, MetaError> {
        let place = Place::parse(if cfg!(windows) { "C:\\doc" } else { "/doc" }).unwrap();
        let entry = Entry::new(name, EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: None,
            group: None,
            cancel: None,
            tools: None,
        };
        assert!(reader.accepts(&request), "{name}");
        reader
            .read(&mut Cursor::new(bytes), &request, &ReadLimits::default())
            .map(|fields| {
                fields
                    .into_iter()
                    .map(|f| {
                        let private = if f.private { " (private)" } else { "" };
                        format!("{}{private}", f.cleaned().line())
                    })
                    .collect()
            })
    }

    #[test]
    fn a_pdf_says_its_title_author_pages_and_dates() {
        let bytes = pdf(
            "/Title (Field Notes) /Author (Ada Example) /Creator (Writer 7) \
             /Producer (diroxide tests) /CreationDate (D:20010101083000Z) /ModDate (D:2002)",
        );
        let got = lines(&PdfProperties, "notes.pdf", bytes).unwrap();
        assert_eq!(
            got,
            [
                "Title: Field Notes",
                "Author: Ada Example (private)",
                "Made with: Writer 7",
                "PDF made by: diroxide tests",
                "Document created: 2001-01-01 08:30",
                "Document modified: 2002",
                "Pages: 1",
                "PDF version: 1.7",
                "Encrypted: no",
            ]
        );
        for line in &got {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert_eq!(
            lines(&PdfProperties, "a.pdf", b"not a pdf at all".to_vec()),
            Err(MetaError::unreadable("not a PDF"))
        );
        assert_eq!(
            lines(&PdfProperties, "a.pdf", b"%PDF-1.7\nbroken".to_vec()),
            Err(MetaError::unreadable("the PDF is damaged"))
        );
    }

    #[test]
    fn a_pdf_larger_than_the_limit_is_refused_unread() {
        let place = Place::parse(if cfg!(windows) { "C:\\doc" } else { "/doc" }).unwrap();
        let entry = Entry::new("big.pdf", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: Some("pdf"),
            group: None,
            cancel: None,
            tools: None,
        };
        let limits = ReadLimits {
            max_bytes: 10,
            ..ReadLimits::default()
        };
        assert_eq!(
            PdfProperties.read(&mut Cursor::new(pdf("")), &request, &limits),
            Err(MetaError::TooLarge)
        );
    }

    const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>Budget</dc:title><dc:creator>Ada Example</dc:creator><cp:keywords>plan, money</cp:keywords>
<dc:description>Draft for the team</dc:description><cp:lastModifiedBy>Bea Example</cp:lastModifiedBy>
<cp:revision>7</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">2001-01-01T08:00:00Z</dcterms:created>
<dcterms:modified xsi:type="dcterms:W3CDTF">2001-02-03T04:05:00Z</dcterms:modified></cp:coreProperties>"#;

    const APP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
<Template>Normal.dotm</Template><Pages>12</Pages><Words>3400</Words><Application>Writer 7</Application>
<Company>Example Company</Company></Properties>"#;

    #[test]
    fn a_docx_says_its_people_privately_and_its_pages_as_stale() {
        let bytes = zip_of(
            &[
                ("[Content_Types].xml", b"<Types/>"),
                ("docProps/core.xml", CORE.as_bytes()),
                ("docProps/app.xml", APP.as_bytes()),
                ("word/document.xml", b"<w:document/>"),
                ("word/comments.xml", b"<w:comments/>"),
            ],
            "",
        );
        let got = lines(&OfficeProperties, "budget.docx", bytes).unwrap();
        assert_eq!(
            got,
            [
                "Title: Budget",
                "Author: Ada Example (private)",
                "Keywords: plan, money",
                "Comments: Draft for the team (private)",
                "Last saved by: Bea Example (private)",
                "Revision: 7",
                "Document created: 2001-01-01 08:00",
                "Document modified: 2001-02-03 04:05",
                "Made with: Writer 7",
                "Company: Example Company (private)",
                "Template: Normal.dotm",
                "Pages: 12, as last saved, may be stale",
                "Words: 3,400",
                "Review comments: yes (private)",
            ]
        );
        for line in &got {
            let line = line.trim_end_matches(" (private)");
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert_eq!(
            lines(&OfficeProperties, "a.docx", zip_of(&[("x.txt", b"x")], "")),
            Err(MetaError::unreadable("not an Office document"))
        );
        assert_eq!(
            lines(&OfficeProperties, "a.docx", b"PK not really".to_vec()),
            Err(MetaError::unreadable("not a zip package, or a damaged one"))
        );
    }

    #[test]
    fn a_dtd_or_a_damaged_part_is_refused_in_words() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]><x>&a;</x>"#;
        let bytes = zip_of(&[("docProps/core.xml", bomb.as_bytes())], "");
        assert_eq!(
            lines(&OfficeProperties, "a.docx", bytes),
            Err(MetaError::unreadable("its properties are damaged"))
        );
        // UTF-16 parts are decoded.
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<Properties><Pages>3</Pages></Properties>".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let bytes = zip_of(&[("docProps/app.xml", &utf16)], "");
        assert_eq!(
            lines(&OfficeProperties, "a.xlsx", bytes).unwrap(),
            ["Pages: 3, as last saved, may be stale"]
        );
    }

    #[test]
    fn an_odt_and_an_epub_say_their_properties() {
        let meta = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/">
<office:meta><meta:generator>Writer 7</meta:generator><dc:title>Minutes</dc:title>
<meta:initial-creator>Ada Example</meta:initial-creator><dc:creator>Bea Example</dc:creator>
<meta:keyword>meeting</meta:keyword><meta:keyword>notes</meta:keyword>
<meta:creation-date>2001-01-01T08:00:00</meta:creation-date><dc:date>2001-01-02T09:30:00.12</dc:date>
<meta:editing-cycles>3</meta:editing-cycles>
<meta:document-statistic meta:page-count="2" meta:word-count="512"/></office:meta></office:document-meta>"#;
        let bytes = zip_of(
            &[
                ("mimetype", b"application/vnd.oasis.opendocument.text"),
                ("meta.xml", meta.as_bytes()),
            ],
            "",
        );
        assert_eq!(
            lines(&OpenDocumentProperties, "minutes.odt", bytes).unwrap(),
            [
                "Title: Minutes",
                "Author: Ada Example (private)",
                "Keywords: meeting, notes",
                "Last saved by: Bea Example (private)",
                "Revision: 3",
                "Document created: 2001-01-01 08:00",
                "Document modified: 2001-01-02 09:30",
                "Made with: Writer 7",
                "Pages: 2, as last saved, may be stale",
                "Words: 512",
            ]
        );
        let container = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>A Quiet Book</dc:title><dc:creator>Ada Example</dc:creator><dc:creator>Bea Example</dc:creator>
<dc:language>en</dc:language><dc:publisher>Example Press</dc:publisher><dc:date>2001-05-06</dc:date>
<dc:identifier id="uid">urn:isbn:0000000000</dc:identifier></metadata></package>"#;
        let bytes = zip_of(
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", container.as_bytes()),
                ("OEBPS/book.opf", opf.as_bytes()),
            ],
            "",
        );
        let got = lines(&EpubProperties, "book.epub", bytes).unwrap();
        assert_eq!(
            got,
            [
                "Title: A Quiet Book",
                "Author: Ada Example, Bea Example",
                "Language: en",
                "Publisher: Example Press",
                "Published: 2001-05-06",
                "Identifier: urn:isbn:0000000000",
            ]
        );
        assert_eq!(
            lines(&EpubProperties, "b.epub", zip_of(&[("mimetype", b"x")], "")),
            Err(MetaError::unreadable("not an EPUB: it has no container"))
        );
    }
}
