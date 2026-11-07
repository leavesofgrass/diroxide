//! Check before sharing (R28 3.2; R30 commit 50): what a file carries
//! that could say who made it, where, or with what, read only. It runs
//! the pipeline's fast and background readers and the extended items and
//! keeps the private fields: a place, a device's serial number, the
//! people a file names, comments, a stored thumbnail, data that can hold
//! more (XMP, IPTC, pictures riding along), and where it was downloaded
//! from. Each file is said by name first, then what it holds, then each
//! detail, then what a strip cannot reach ("crop first, then strip").
//!
//! A place is said with two decimals by default (about a kilometer, R28's
//! location risk); nothing read here is sent anywhere.

use diroxide_core::names::display_name;
use diroxide_vfs::{OpContext, Provider};

use crate::field::Field;
use crate::pipeline::Pipeline;
use crate::reader::{Cost, ReadLimits, ReadRequest};

/// The kinds of private detail, in the order they are said.
pub const KINDS: [&str; 8] = [
    "location",
    "device",
    "people",
    "comments",
    "thumbnail",
    "hidden data",
    "origin",
    "other",
];

/// The kind of private detail a field's key holds.
pub fn kind_of(key: &str) -> &'static str {
    match key {
        "image.gps" => "location",
        "image.serial" | "image.lens_serial" => "device",
        "image.owner" | "image.artist" | "doc.author" | "doc.last_saved_by" | "doc.company"
        | "doc.manager" => "people",
        "image.comment"
        | "doc.comments"
        | "doc.review_comments"
        | "audio.comment"
        | "file.comment" => "comments",
        "image.thumbnail" => "thumbnail",
        "image.xmp" | "image.iptc" | "image.more_pictures" => "hidden data",
        "file.origin" => "origin",
        _ => "other",
    }
}

/// What one file carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacyReport {
    /// The file's name.
    pub name: String,
    /// The private fields, in the order they were read.
    pub details: Vec<Field>,
    /// What a strip cannot reach, for this kind of file.
    pub cautions: Vec<&'static str>,
    /// The readers that could not read it, with why.
    pub unread: Vec<String>,
}

impl PrivacyReport {
    /// Whether it holds any private detail.
    pub fn holds_private(&self) -> bool {
        !self.details.is_empty()
    }

    /// The kinds it holds, each once, in [`KINDS`] order.
    pub fn kinds(&self) -> Vec<&'static str> {
        KINDS
            .into_iter()
            .filter(|kind| self.details.iter().any(|f| kind_of(&f.key) == *kind))
            .collect()
    }

    /// The file's line, its name first: "IMG_2041.jpg: location,
    /// device, people", or "notes.txt: nothing private found".
    pub fn headline(&self) -> String {
        let name = display_name(&self.name);
        if self.holds_private() {
            format!("{name}: {}", self.kinds().join(", "))
        } else {
            format!("{name}: nothing private found")
        }
    }

    /// The headline, each detail ("Location: 45.50 north, 122.69
    /// west"), what could not be read, then the cautions.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.headline()];
        lines.extend(self.details.iter().map(Field::line));
        lines.extend(self.unread.iter().map(|why| format!("Not read: {why}")));
        if self.holds_private() {
            lines.extend(self.cautions.iter().map(|c| (*c).to_owned()));
        }
        lines
    }
}

/// The summary over several files, first: "3 of 5 files hold private
/// details", "1 file holds private details", "No private details in 5
/// files".
pub fn summary(reports: &[PrivacyReport]) -> String {
    let holding = reports.iter().filter(|r| r.holds_private()).count();
    let total = reports.len();
    match (holding, total) {
        (0, 1) => "No private details in 1 file".to_owned(),
        (0, n) => format!("No private details in {n} files"),
        (1, 1) => "1 file holds private details".to_owned(),
        (h, n) => format!("{h} of {n} files hold private details"),
    }
}

/// What a strip cannot reach, by what the fields say the file is.
fn cautions(details: &[Field], all: &[Field], name: &str) -> Vec<&'static str> {
    let lower = name.to_ascii_lowercase();
    let picture = all.iter().any(|f| f.key.starts_with("image."))
        || [".jpg", ".jpeg", ".png", ".webp"]
            .iter()
            .any(|e| lower.ends_with(e));
    let mut out = Vec::new();
    if details.iter().any(|f| f.key == "image.thumbnail") {
        out.push("Its stored thumbnail may show the uncropped picture: crop first, then strip.");
    }
    if picture {
        out.push("A strip leaves the picture itself and the file's name.");
    }
    if lower.ends_with(".pdf") {
        out.push("A PDF can keep earlier versions; a strip here does not reach them.");
    }
    if all.iter().any(|f| f.key == "doc.review_comments")
        || [".docx", ".xlsx", ".pptx", ".odt", ".ods", ".odp"]
            .iter()
            .any(|e| lower.ends_with(e))
    {
        out.push("Comments and tracked changes inside a document stay in it.");
    }
    out
}

/// Reports what a list of fields holds: the private ones, with the
/// cautions their kind of file needs.
pub fn report_of(name: &str, fields: &[Field], unread: Vec<String>) -> PrivacyReport {
    let details: Vec<Field> = fields.iter().filter(|f| f.private).cloned().collect();
    PrivacyReport {
        name: name.to_owned(),
        cautions: cautions(&details, fields, name),
        details,
        unread,
    }
}

/// Checks one file before it is shared: the pipeline's fast and
/// background readers that take it, and its extended items (the origin
/// row), each under the pipeline's guard and `limits`.
pub fn check(
    pipeline: &Pipeline,
    provider: &dyn Provider,
    request: &ReadRequest<'_>,
    limits: &ReadLimits,
    ctx: &OpContext,
) -> PrivacyReport {
    let mut outcomes = pipeline.read(provider, request, Cost::Fast, limits, ctx);
    outcomes.extend(pipeline.read(provider, request, Cost::Background, limits, ctx));
    let mut fields = Pipeline::fields(&outcomes);
    let mut unread: Vec<String> = outcomes
        .iter()
        .filter_map(|(_, result)| result.as_ref().err().map(ToString::to_string))
        .collect();
    match crate::origin::extended_fields(provider, request.place, ctx) {
        Ok(extended) => fields.extend(extended),
        Err(error) => unread.push(error.to_string()),
    }
    unread.dedup();
    report_of(&request.entry.name, &fields, unread)
}

#[cfg(test)]
mod tests {
    use diroxide_core::details::DetailGroup;
    use diroxide_core::{Entry, EntryKind, Place};
    use diroxide_vfs::LocalProvider;

    use super::*;

    #[test]
    fn a_file_is_said_by_name_then_what_it_holds() {
        let fields = vec![
            Field::new(
                "image.taken",
                "Taken",
                "2001-01-01 08:30",
                DetailGroup::Dates,
            ),
            Field::new(
                "image.gps",
                "Location",
                "45.50 north, 122.69 west",
                DetailGroup::Source,
            )
            .private(),
            Field::new(
                "image.serial",
                "Camera serial",
                "SN12345",
                DetailGroup::Source,
            )
            .private(),
            Field::new("image.artist", "Author", "Ada Example", DetailGroup::Source).private(),
            Field::new(
                "image.thumbnail",
                "Stored thumbnail",
                "yes, maybe uncropped",
                DetailGroup::Technical,
            )
            .private(),
        ];
        let report = report_of("IMG_2041.jpg", &fields, Vec::new());
        assert_eq!(
            report.lines(),
            [
                "IMG_2041.jpg: location, device, people, thumbnail",
                "Location: 45.50 north, 122.69 west",
                "Camera serial: SN12345",
                "Author: Ada Example",
                "Stored thumbnail: yes, maybe uncropped",
                "Its stored thumbnail may show the uncropped picture: crop first, then strip.",
                "A strip leaves the picture itself and the file's name.",
            ]
        );
        let clean = report_of("notes.txt", &[], vec![]);
        assert_eq!(clean.lines(), ["notes.txt: nothing private found"]);
        assert_eq!(
            summary(&[
                report.clone(),
                clean.clone(),
                report.clone(),
                clean.clone(),
                report
            ]),
            "3 of 5 files hold private details"
        );
        assert_eq!(
            summary(std::slice::from_ref(&clean)),
            "No private details in 1 file"
        );
        for line in [
            "3 of 5 files hold private details",
            "No private details in 5 files",
        ] {
            assert!(line.chars().count() <= 40);
        }
    }

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap()
    }

    /// The whole check on files of the test's own: a photo with a place
    /// and a camera serial, a document with an author, a plain text file.
    #[test]
    fn the_check_reads_photos_and_documents() {
        let dir = scratch();
        let mut exif = crate::exif::EXIF_PREFIX.to_vec();
        exif.extend_from_slice(&crate::exif::tests::private_block(true));
        let photo = crate::image::tests::jpeg(&[(0xE1, exif)]);
        std::fs::write(dir.path().join("IMG_2041.jpg"), photo).unwrap();
        std::fs::write(
            dir.path().join("notes.pdf"),
            crate::readers::document::tests::pdf("/Author (Ada Example) /Title (Notes)"),
        )
        .unwrap();
        std::fs::write(dir.path().join("plain.txt"), b"nothing here").unwrap();
        let pipeline = Pipeline::builtin();
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let limits = ReadLimits::default();
        let mut reports = Vec::new();
        for name in ["IMG_2041.jpg", "notes.pdf", "plain.txt"] {
            let place = Place::parse(&dir.path().join(name).display().to_string()).unwrap();
            let entry = Entry::new(name, EntryKind::File);
            let request = ReadRequest {
                place: &place,
                entry: &entry,
                type_id: None,
                group: None,
                cancel: None,
                tools: None,
            };
            reports.push(check(&pipeline, &provider, &request, &limits, &ctx));
        }
        assert_eq!(
            reports[0].headline(),
            "IMG_2041.jpg: location, device, people, comments, thumbnail"
        );
        assert_eq!(reports[1].headline(), "notes.pdf: people");
        assert!(reports[1].lines().contains(
            &"A PDF can keep earlier versions; a strip here does not reach them.".to_owned()
        ));
        assert_eq!(reports[2].headline(), "plain.txt: nothing private found");
        assert_eq!(summary(&reports), "2 of 3 files hold private details");
    }
}
