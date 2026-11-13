//! The details view's reading and its words (alpha 4, R30 commit 41; R28
//! 2.2 and 3.2; R12's order of facts): what the runtime reads on a worker
//! for the details list, Say Details and Show all, and the plain functions
//! the app builds the list's rows and the clipboard's text with.
//!
//! - **Every fact is a [`DetailRow`]**, listed in its group's order
//!   (core's [`DetailGroup`]): what it is, what is in it, its size, its
//!   dates, privacy and origin, the owner's own, its place, technical,
//!   system, and how it was read. A fact the file does not have is not
//!   listed, except the privacy row and the checksum row, whose "nothing
//!   private found" and "not computed" are the point (R12).
//! - **The fields are read on a worker** through the metadata pipeline
//!   (`diroxide-meta`, lane ML), with the preview crate's image and
//!   ffprobe readers added (lane VL), in all three cost classes: the
//!   details list is asked for, so its on-demand readers run too, the
//!   tool-backed ones among them (R28 2.2: tool-backed work runs in the
//!   quick view after its wait, in the details list, or on Preview).
//! - **Say Details** says the preview's summary alone ("PNG, 1920 by
//!   1080, 8-bit RGBA, 2.3 MB"); **Copy details** puts the rows on the
//!   clipboard, private rows left out unless all are asked for; **Show
//!   all** asks ExifTool for every tag it reads, one run through the tools
//!   registry's runner (ML's `tools::show_all`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::details::{DetailGroup, DetailRow, order_rows};
use diroxide_core::{Entry, Place};
use diroxide_filetype::FileType;
use diroxide_meta::pipeline::EXTENDED;
use diroxide_meta::{Cost, Field, MetaError, Outcome, Pipeline, ReadLimits, ReadRequest};
use diroxide_preview::{LatestOnly, PreviewCache, PreviewRequest, Previews};
use diroxide_vfs::{CancelToken, OpContext, Registry, VfsError};

use crate::request::SharedTools;

/// The id of the details list's last row, Show all.
pub const SHOW_ALL_ROW: &str = "show-all";

/// The Show all row before it is used (37 cells).
pub const SHOW_ALL: &str = "Show all: every tag, read by ExifTool";

/// The Show all row while ExifTool reads (31 cells).
pub const SHOW_ALL_READING: &str = "Show all: reading with ExifTool";

/// What Show all says on a file that is not on a local disk (33 cells).
pub const SHOW_ALL_NOT_LOCAL: &str = "More details: not on a local disk";

/// The privacy row's value while the file is read.
pub const READING: &str = "reading";

/// The privacy row's value when nothing private was found.
pub const NOTHING_PRIVATE: &str = "nothing private found";

/// The checksum row's value: the Checksum command computes it.
pub const NOT_COMPUTED: &str = "not computed";

/// How long a reader of the on-demand class may take: ffprobe's own ten
/// seconds, and the guard's margin.
const ON_DEMAND_DEADLINE: Duration = Duration::from_secs(12);

/// Shares a preview cache between the quick view, the details list and
/// the Preview command; two are equal when they are the same cache.
#[derive(Clone, Debug, Default)]
pub struct SharedCache(pub Arc<PreviewCache>);

impl PartialEq for SharedCache {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What a details request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// The file's fields, for the details list or Copy details.
    Fields,
    /// The preview's summary, for Say Details.
    Summary,
    /// Every tag ExifTool reads, for Show all.
    ShowAll,
}

/// A details request for the runtime: the file, what its type check
/// found, the runner its tools go through, and what is wanted.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailsAsk {
    /// The request's number, so a late answer to an older one is dropped.
    pub generation: u64,
    /// The file's place.
    pub place: Place,
    /// Its entry, as listed.
    pub entry: Entry,
    /// Its checked type, else the one its name says.
    pub file_type: Option<FileType>,
    /// The runner the tool-backed readers go through.
    pub tools: SharedTools,
    /// The previews' cache, for the summary.
    pub cache: Option<SharedCache>,
    /// What is wanted.
    pub wanted: Wanted,
}

/// What the runtime read for a details request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailsNote {
    /// The file's fields as rows, and what could not be read, in words
    /// ("picture facts, the file ends too soon").
    Fields {
        /// The request's number.
        generation: u64,
        /// The file.
        place: Place,
        /// The rows read, in the groups' order.
        rows: Vec<DetailRow>,
        /// What could not be read, each the reader in words, then why.
        unread: Vec<String>,
    },
    /// The preview's summary, for Say Details.
    Summary {
        /// The request's number.
        generation: u64,
        /// The file.
        place: Place,
        /// The summary: "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB".
        line: String,
    },
    /// Every tag ExifTool read, or why there are none ("More details:
    /// ExifTool not found").
    ShowAll {
        /// The request's number.
        generation: u64,
        /// The file.
        place: Place,
        /// The rows, or the line said instead.
        result: Result<Vec<DetailRow>, String>,
    },
}

/// Carries out a details request on the runtime's worker.
pub fn answer(registry: &Registry, ask: &DetailsAsk) -> DetailsNote {
    answer_with(registry, ask, &CancelToken::new())
}

/// Carries out a details request, its fields read under `cancel`: the
/// list closing or a newer request trips it, and the readers stop at
/// their next read (VA's request 6, the app's half).
pub fn answer_with(registry: &Registry, ask: &DetailsAsk, cancel: &CancelToken) -> DetailsNote {
    match ask.wanted {
        Wanted::Fields => {
            // A folder's own fields are not read, its access rows are
            // (alpha 4, lane AP: the focused entry's, under this token).
            let (mut rows, unread) = if crate::columns::file_like(&ask.entry) {
                read_fields_with(registry, ask, cancel)
            } else {
                (Vec::new(), Vec::new())
            };
            rows.extend(crate::access::rows_with(registry, &ask.place, cancel));
            DetailsNote::Fields {
                generation: ask.generation,
                place: ask.place.clone(),
                rows,
                unread,
            }
        }
        Wanted::Summary => DetailsNote::Summary {
            generation: ask.generation,
            place: ask.place.clone(),
            line: summary(registry, ask),
        },
        Wanted::ShowAll => DetailsNote::ShowAll {
            generation: ask.generation,
            place: ask.place.clone(),
            result: show_all(ask, &CancelToken::new()),
        },
    }
}

/// The metadata pipeline the details read through: the metadata crate's
/// readers, then the preview crate's image words and ffprobe's reader
/// over the runner.
pub fn pipeline(tools: &SharedTools) -> Pipeline {
    Pipeline::builtin()
        .with(diroxide_preview::images::ImageFields::shared())
        .with(Arc::new(diroxide_preview::media::MediaFields::new(
            Arc::clone(&tools.0),
        )))
}

/// Reads a file's fields in every cost class, and its extended items,
/// each reader under the pipeline's guard: the rows in the groups' order,
/// and what could not be read.
pub fn read_fields(registry: &Registry, ask: &DetailsAsk) -> (Vec<DetailRow>, Vec<String>) {
    read_fields_with(registry, ask, &CancelToken::new())
}

/// [`read_fields`] under a cancel token: a cancel stops between readers
/// and inside a read, and the rows read so far come back.
pub fn read_fields_with(
    registry: &Registry,
    ask: &DetailsAsk,
    cancel: &CancelToken,
) -> (Vec<DetailRow>, Vec<String>) {
    let provider = match registry.resolve(&ask.place) {
        Ok(provider) => provider,
        Err(error) => return (Vec::new(), vec![format!("the file, {error}")]),
    };
    let pipeline = pipeline(&ask.tools);
    // The readers get the cancel token and the runner too, so one that
    // starts a program (ffprobe) stops it on a cancel (MA's request 1).
    let request = ReadRequest {
        place: &ask.place,
        entry: &ask.entry,
        type_id: ask.file_type.as_ref().map(|t| t.id.as_str()),
        group: ask.file_type.as_ref().and_then(|t| t.group.as_deref()),
        cancel: Some(cancel),
        tools: Some(&ask.tools.0),
    };
    let ctx = OpContext::new()
        .with_tools(Arc::clone(&ask.tools.0))
        .with_cancel(cancel.clone());
    let quick = ReadLimits::default();
    let slow = ReadLimits {
        deadline: ON_DEMAND_DEADLINE,
        ..ReadLimits::default()
    };
    let mut outcomes: Vec<Outcome> = Vec::new();
    for (cost, limits) in [
        (Cost::Fast, &quick),
        (Cost::Background, &quick),
        (Cost::OnDemand, &slow),
    ] {
        outcomes.extend(pipeline.read(provider.as_ref(), &request, cost, limits, &ctx));
    }
    outcomes.push((
        EXTENDED,
        diroxide_meta::origin::extended_fields(provider.as_ref(), &ask.place, &ctx),
    ));
    let fields = without_working_keys(keep_audio_tags(Pipeline::fields(&outcomes)));
    let rows = fields.iter().map(Field::row).collect();
    (rows, unread_words(&outcomes))
}

/// Keys the readers give for the app's own use, not as details: the date
/// taken to the second (lane FX), which bursts and columns read, while
/// the details say the date taken once, as "Taken".
const WORKING_KEYS: [&str; 1] = ["image.taken_second"];

/// The details without the working keys, so a fact is not said twice.
pub fn without_working_keys(fields: Vec<Field>) -> Vec<Field> {
    fields
        .into_iter()
        .filter(|f| !WORKING_KEYS.contains(&f.key.as_str()))
        .collect()
}

/// The pairs of an ffprobe row and the audio reader's row that says the
/// same of a sound file; the audio reader's stands.
const AUDIO_OVER_MEDIA: [(&str, &str); 6] = [
    ("media.title", "audio.title"),
    ("media.artist", "audio.artist"),
    ("media.album", "audio.album"),
    ("media.duration", "audio.length"),
    ("media.bit_rate", "audio.bitrate"),
    ("media.cover_picture.", "audio.cover"),
];

/// A sound file's tags are read twice, by the metadata crate's audio
/// reader (`audio.*`, lane ML) and by ffprobe's (`media.*`, lane VL):
/// ffprobe's row is left out where the audio reader gave the same fact
/// (VL's request 5), and its stream rows stay ("Audio: MP3, stereo, 44.1
/// kHz").
pub fn keep_audio_tags(fields: Vec<Field>) -> Vec<Field> {
    let has = |key: &str| fields.iter().any(|f| f.key == key);
    let dropped: Vec<&str> = AUDIO_OVER_MEDIA
        .iter()
        .filter(|(_, audio)| has(audio))
        .map(|(media, _)| *media)
        .collect();
    if dropped.is_empty() {
        return fields;
    }
    fields
        .into_iter()
        .filter(|f| {
            !dropped
                .iter()
                .any(|media| f.key == *media || (media.ends_with('.') && f.key.starts_with(media)))
        })
        .collect()
}

/// What each reader that failed could not read, its reader in words
/// first: "picture facts, the file ends too soon". A reader refused while
/// the guard was busy, a cancel and the extended items' absence are left
/// out.
fn unread_words(outcomes: &[Outcome]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (id, result) in outcomes {
        let Err(error) = result else { continue };
        if *id == EXTENDED
            || matches!(
                error,
                MetaError::Busy | MetaError::Place(VfsError::Cancelled)
            )
        {
            continue;
        }
        let line = format!("{}, {error}", reader_words(id));
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out
}

/// A reader's id in words, for the "Not read" rows.
pub fn reader_words(id: &str) -> &str {
    match id {
        "archive" => "archive facts",
        "audio" => "sound tags",
        "audio-properties" => "sound properties",
        "pdf" => "PDF properties",
        "office" | "opendocument" => "document properties",
        "epub" => "book properties",
        "photo" => "photo facts",
        "text" => "text facts",
        "image" => "picture facts",
        "media" => "sound and video facts",
        other => other,
    }
}

/// The preview's summary for Say Details: the first previewer that takes
/// the file makes it, with the tools, through the shared cache; when none
/// can, the type and the size in words.
pub fn summary(registry: &Registry, ask: &DetailsAsk) -> String {
    let previews = match &ask.cache {
        Some(cache) => Previews::builtin().with_cache(Arc::clone(&cache.0)),
        None => Previews::builtin().without_cache(),
    };
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: ask.place.clone(),
        entry: ask.entry.clone(),
        generation: latest.next(),
        max_lines: 1,
    };
    let ctx = OpContext::new().with_tools(Arc::clone(&ask.tools.0));
    match previews.render(&request, ask.file_type.as_ref(), &latest, registry, &ctx) {
        Ok(preview) if !preview.summary.trim().is_empty() => preview.summary,
        _ => diroxide_preview::details(&ask.entry, ask.file_type.as_ref()),
    }
}

/// Show all: every tag ExifTool reads of one local file, one run through
/// the runner, as rows in ExifTool's order; or the line said instead
/// ("More details: ExifTool not found").
pub fn show_all(ask: &DetailsAsk, cancel: &CancelToken) -> Result<Vec<DetailRow>, String> {
    let Some(path) = ask.place.local_path() else {
        return Err(SHOW_ALL_NOT_LOCAL.to_owned());
    };
    if !ask.tools.0.has(diroxide_meta::tools::EXIFTOOL) {
        return Err(diroxide_meta::tools::EXIFTOOL_MISSING.to_owned());
    }
    match diroxide_meta::tools::show_all(ask.tools.0.as_ref(), &[PathBuf::from(path)], cancel) {
        Ok(found) => Ok(found
            .into_iter()
            .flat_map(|(_, fields)| fields)
            .map(|field| field.row())
            .collect()),
        Err(error) => Err(format!("More details: {error}")),
    }
}

/// The privacy row's words from the rows read: the kinds of private
/// detail in the metadata crate's order ("location, device"), or
/// "nothing private found".
pub fn privacy_words(rows: &[DetailRow]) -> String {
    let kinds: Vec<&str> = diroxide_meta::privacy::KINDS
        .into_iter()
        .filter(|kind| {
            rows.iter()
                .any(|r| r.private && diroxide_meta::privacy::kind_of(&r.key) == *kind)
        })
        .collect();
    if kinds.is_empty() {
        NOTHING_PRIVATE.to_owned()
    } else {
        kinds.join(", ")
    }
}

/// A row as listed and said: "Dimensions: 4032 by 3024"; a row with no
/// label is its value alone ("Name says PNG; contents say JPEG").
pub fn row_text(row: &DetailRow) -> String {
    if row.label.is_empty() {
        row.value.clone()
    } else {
        row.spoken()
    }
}

/// A "Not read" row, in the last group: "Not read: picture facts, the
/// file ends too soon".
pub fn unread_row(index: usize, words: &str) -> DetailRow {
    DetailRow::new(
        format!("read.not.{index}"),
        "Not read",
        words,
        DetailGroup::Source,
    )
}

/// Every row in the groups' order, each group keeping its own order, a
/// key kept once (the first).
pub fn ordered(mut rows: Vec<DetailRow>) -> Vec<DetailRow> {
    let mut seen = std::collections::HashSet::new();
    rows.retain(|row| seen.insert(row.key.clone()));
    order_rows(&mut rows);
    rows
}

/// What Copy details puts on the clipboard: the title, then each row's
/// words, one a line; private rows left out unless `all`. Returns the
/// text, the rows copied and the private rows left out.
pub fn copy_text(title: &str, rows: &[DetailRow], all: bool) -> (String, usize, usize) {
    let mut lines = vec![title.to_owned()];
    let mut copied = 0;
    let mut left = 0;
    for row in rows {
        if row.private && !all {
            left += 1;
            continue;
        }
        lines.push(row_text(row));
        copied += 1;
    }
    let mut text = lines.join("\n");
    text.push('\n');
    (text, copied, left)
}

/// What Copy details says: "Copied 14 details, 2 private left out",
/// "Copied 14 details", "Copied all 16 details".
pub fn copied_words(copied: usize, left_out: usize, all: bool) -> String {
    let details = diroxide_core::format::count(copied as u64, "detail", "details");
    match (all, left_out) {
        (true, _) => format!("Copied all {details}"),
        (false, 0) => format!("Copied {details}"),
        (false, n) => format!("Copied {details}, {n} private left out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn row(key: &str, label: &str, value: &str, group: DetailGroup) -> DetailRow {
        DetailRow::new(key, label, value, group)
    }

    /// VA's request 6, the app's half: a cancelled field read stops
    /// before its readers, and says nothing it could not read.
    #[test]
    fn a_cancelled_field_read_stops_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let place = Place::parse(path.to_str().unwrap()).unwrap();
        let ask = DetailsAsk {
            generation: 1,
            place: place.clone(),
            entry: Entry::new("notes.txt", diroxide_core::EntryKind::File),
            file_type: None,
            tools: SharedTools(Arc::new(diroxide_vfs::NoTools)),
            cache: None,
            wanted: Wanted::Fields,
        };
        let registry = Registry::with_local();
        let (rows, _) = read_fields(&registry, &ask);
        assert!(rows.iter().any(|r| r.key == "text.lines"), "{rows:?}");
        let cancel = CancelToken::new();
        cancel.cancel();
        let (rows, unread) = read_fields_with(&registry, &ask, &cancel);
        assert!(!rows.iter().any(|r| r.key == "text.lines"), "{rows:?}");
        assert!(unread.is_empty(), "{unread:?}");
    }

    #[test]
    fn rows_go_in_the_order_of_facts_each_key_once() {
        let rows = ordered(vec![
            row(
                "file.checksum",
                "Checksum",
                NOT_COMPUTED,
                DetailGroup::System,
            ),
            row("time.modified", "Modified", "today", DetailGroup::Dates),
            row("type", "Type", "JPEG image", DetailGroup::Identity),
            row("image.gps", "Location", "45.50 north", DetailGroup::Source).private(),
            row("file.privacy", "Privacy", "location", DetailGroup::Flags),
            row("size", "Size", "3.1 MB", DetailGroup::Size),
            row(
                "image.dimensions",
                "Dimensions",
                "3024 by 4032",
                DetailGroup::Content,
            ),
            row("type", "Type", "a second type", DetailGroup::Identity),
        ]);
        let said: Vec<String> = rows.iter().map(row_text).collect();
        assert_eq!(
            said,
            [
                "Type: JPEG image",
                "Dimensions: 3024 by 4032",
                "Size: 3.1 MB",
                "Modified: today",
                "Privacy: location",
                "Checksum: not computed",
                "Location: 45.50 north",
            ]
        );
    }

    #[test]
    fn privacy_names_the_kinds_or_says_nothing_was_found() {
        let rows = vec![
            row("image.serial", "Camera serial", "123", DetailGroup::Source).private(),
            row("image.gps", "Location", "45.50 north", DetailGroup::Source).private(),
            row(
                "image.dimensions",
                "Dimensions",
                "1 by 1",
                DetailGroup::Content,
            ),
        ];
        assert_eq!(privacy_words(&rows), "location, device");
        assert_eq!(privacy_words(&rows[2..]), "nothing private found");
        for line in [
            format!("Privacy: {}", privacy_words(&rows)),
            format!("Privacy: {NOTHING_PRIVATE}"),
            format!("Checksum: {NOT_COMPUTED}"),
            SHOW_ALL.to_owned(),
            SHOW_ALL_READING.to_owned(),
            SHOW_ALL_NOT_LOCAL.to_owned(),
            diroxide_meta::tools::EXIFTOOL_MISSING.to_owned(),
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn copy_details_leaves_private_rows_out_unless_all() {
        let rows = vec![
            row("type", "Type", "JPEG image", DetailGroup::Identity),
            row("image.gps", "Location", "45.50 north", DetailGroup::Source).private(),
            row(
                "file.type-check",
                "",
                "Name says PNG; contents say JPEG",
                DetailGroup::Flags,
            ),
        ];
        let (text, copied, left) = copy_text("Details of photo.jpg", &rows, false);
        assert_eq!(
            text,
            "Details of photo.jpg\nType: JPEG image\nName says PNG; contents say JPEG\n"
        );
        assert_eq!((copied, left), (2, 1));
        let (all, copied, left) = copy_text("Details of photo.jpg", &rows, true);
        assert!(all.contains("Location: 45.50 north"));
        assert_eq!((copied, left), (3, 0));
        let lines = [
            copied_words(14, 2, false),
            copied_words(14, 0, false),
            copied_words(16, 0, true),
            copied_words(1, 0, false),
        ];
        assert_eq!(
            lines,
            [
                "Copied 14 details, 2 private left out",
                "Copied 14 details",
                "Copied all 16 details",
                "Copied 1 detail",
            ]
        );
        for line in &lines {
            assert!(within_cells(line, "Copied"), "{line}");
        }
    }

    #[test]
    fn a_sound_files_tags_are_the_audio_readers() {
        let field = |key: &str| Field::new(key, key, "x", DetailGroup::Content);
        let kept = keep_audio_tags(vec![
            field("audio.title"),
            field("audio.artist"),
            field("audio.length"),
            field("media.title"),
            field("media.artist"),
            field("media.album"),
            field("media.duration"),
            field("media.audio.1"),
            field("media.cover_picture.1"),
        ]);
        let keys: Vec<&str> = kept.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "audio.title",
                "audio.artist",
                "audio.length",
                "media.album",
                "media.audio.1",
                "media.cover_picture.1",
            ],
            "ffprobe's album and cover stay while the audio reader has none"
        );
        let video = keep_audio_tags(vec![field("media.title"), field("media.video.1")]);
        assert_eq!(video.len(), 2, "no audio reader, ffprobe's rows stay");
        let covered = keep_audio_tags(vec![field("audio.cover"), field("media.cover_picture.1")]);
        assert_eq!(covered.len(), 1);
    }

    #[test]
    fn the_date_taken_is_said_once_without_its_seconds_form() {
        let field = |key: &str| Field::new(key, key, "x", DetailGroup::Dates);
        let kept = without_working_keys(vec![
            field("image.taken"),
            field("image.taken_second"),
            field("image.camera"),
        ]);
        let keys: Vec<&str> = kept.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["image.taken", "image.camera"]);
    }

    /// VL's request 2: the image words and ffprobe's reader name their
    /// keys, so a column finds its cost class and a failed read's cells
    /// say why.
    #[test]
    fn the_preview_readers_name_their_keys() {
        let pipeline = pipeline(&SharedTools(Arc::new(diroxide_vfs::NoTools)));
        assert_eq!(pipeline.cost_of("image.dimensions"), Some(Cost::Background));
        assert_eq!(
            pipeline.cost_of("image.orientation"),
            Some(Cost::Background)
        );
        assert_eq!(pipeline.cost_of("media.audio.1"), Some(Cost::OnDemand));
        assert_eq!(pipeline.cost_of("media.bit_rate"), Some(Cost::OnDemand));
        let keys = pipeline.keys();
        assert!(keys.contains(&("image.shown", Cost::Background, "image")));
        assert!(keys.contains(&("media.duration", Cost::OnDemand, "media")));
        let outcomes: Vec<Outcome> = vec![(
            "image",
            Err(MetaError::unreadable("the file ends too soon")),
        )];
        assert_eq!(
            pipeline.cell("image.dimensions", &outcomes).words(),
            "unreadable: the file ends too soon"
        );
        assert_eq!(
            pipeline.cell("media.duration", &outcomes).words(),
            "not available"
        );
    }

    #[test]
    fn an_unread_reader_is_said_by_what_it_reads() {
        let outcomes: Vec<Outcome> = vec![
            (
                "image",
                Err(MetaError::unreadable("the file ends too soon")),
            ),
            ("photo", Err(MetaError::Busy)),
            (EXTENDED, Err(MetaError::unreadable("not here"))),
            ("audio", Ok(Vec::new())),
        ];
        let words = unread_words(&outcomes);
        assert_eq!(words, ["picture facts, the file ends too soon"]);
        assert_eq!(
            row_text(&unread_row(0, &words[0])),
            "Not read: picture facts, the file ends too soon"
        );
    }
}
