//! Audio facts through `lofty` (R12, "Audio metadata"; R28 3.2).
//!
//! Two readers, because the length of an MP3 with no Xing or VBRI header
//! can mean reading frames:
//!
//! - [`AudioTags`], **fast**: the tags only, with `lofty`'s properties
//!   and cover art skipped. Spike M4 (a 300 MB MP3 of silent frames with
//!   no Xing header, made by the spike itself) read its title and artist
//!   in 0.4 ms cold and 0.07 ms warm, touching 82 KB in 13 reads: no
//!   frame scan.
//! - [`AudioProperties`], **background**: the length, bit rate, sample
//!   rate, channels, bit depth and the cover art. On the same file it
//!   read 91 KB in 15 reads; `lofty` estimates a header-less file's
//!   length from its size and first frame (5 hr 12 min 30 sec, right
//!   for a constant bit rate) rather than scanning.
//!
//! Nothing is ever played or decoded.

use std::borrow::Cow;
use std::io::BufReader;

use diroxide_core::details::DetailGroup;
use diroxide_vfs::ReadSeek;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagType};

use crate::field::Field;
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
use crate::words::{bitrate_words, channels_words, count, duration_words, sample_rate_words};

/// The types `lofty` reads.
const TYPES: [&str; 10] = [
    "mp3", "flac", "ogg", "opus", "wav", "aiff", "m4a", "aac", "ape", "wavpack",
];

/// Names read as audio when the type is not checked yet.
const NAMES: [&str; 15] = [
    ".mp3", ".flac", ".ogg", ".oga", ".opus", ".wav", ".aif", ".aiff", ".m4a", ".m4b", ".aac",
    ".ape", ".wv", ".mpc", ".spx",
];

fn takes(request: &ReadRequest<'_>) -> bool {
    super::takes(request, &TYPES, &NAMES)
}

/// The fast audio reader: the tags.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioTags;

impl Reader for AudioTags {
    fn id(&self) -> &'static str {
        "audio"
    }

    fn cost(&self) -> Cost {
        Cost::Fast
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "audio.title",
            "audio.artist",
            "audio.album_artist",
            "audio.album",
            "audio.track",
            "audio.disc",
            "audio.date",
            "audio.genre",
            "audio.composer",
            "audio.comment",
            "audio.tags",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        takes(request)
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let file = open(
            source,
            ParseOptions::new()
                .read_properties(false)
                .read_cover_art(false),
        )?;
        Ok(file
            .primary_tag()
            .or_else(|| file.first_tag())
            .map(tag_fields)
            .unwrap_or_default())
    }
}

/// The background audio reader: length, rates, channels and cover art.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioProperties;

impl Reader for AudioProperties {
    fn id(&self) -> &'static str {
        "audio-properties"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "audio.length",
            "audio.bitrate",
            "audio.sample_rate",
            "audio.channels",
            "audio.bit_depth",
            "audio.cover",
            "audio.format",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        takes(request)
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let file = open(
            source,
            ParseOptions::new()
                .read_properties(true)
                .read_cover_art(true),
        )?;
        Ok(property_fields(&file))
    }
}

/// Reads a file with `lofty`, its kind guessed from its first bytes.
fn open(source: &mut dyn ReadSeek, options: ParseOptions) -> Result<TaggedFile, MetaError> {
    let probe = Probe::new(BufReader::new(source))
        .options(options)
        .guess_file_type()
        .map_err(|e| MetaError::from_io(&e))?;
    if probe.file_type().is_none() {
        return Err(MetaError::unreadable("not a kind of audio read here"));
    }
    probe
        .read()
        .map_err(|_| MetaError::unreadable("its audio tags are damaged"))
}

fn text(value: Option<Cow<'_, str>>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn of_total(number: Option<u32>, total: Option<u32>) -> Option<String> {
    match (number, total) {
        (Some(n), Some(t)) if t > 0 => Some(format!("{n} of {t}")),
        (Some(n), _) => Some(n.to_string()),
        _ => None,
    }
}

fn tag_kind(kind: TagType) -> &'static str {
    match kind {
        TagType::Id3v1 => "ID3 version 1",
        TagType::Id3v2 => "ID3 version 2",
        TagType::Ape => "APE",
        TagType::Mp4Ilst => "MP4",
        TagType::VorbisComments => "Vorbis comments",
        TagType::RiffInfo => "RIFF INFO",
        TagType::AiffText => "AIFF text",
        _ => "other",
    }
}

/// The fields of one tag.
fn tag_fields(tag: &Tag) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut add = |key: &str, label: &str, value: Option<String>, group: DetailGroup| {
        if let Some(value) = value {
            fields.push(Field::new(key, label, value, group));
        }
    };
    add(
        "audio.title",
        "Title",
        text(tag.title()),
        DetailGroup::Content,
    );
    add(
        "audio.artist",
        "Artist",
        text(tag.artist()),
        DetailGroup::Content,
    );
    add(
        "audio.album_artist",
        "Album artist",
        tag.get_string(ItemKey::AlbumArtist).map(str::to_owned),
        DetailGroup::Content,
    );
    add(
        "audio.album",
        "Album",
        text(tag.album()),
        DetailGroup::Content,
    );
    add(
        "audio.track",
        "Track",
        of_total(tag.track(), tag.track_total()),
        DetailGroup::Content,
    );
    add(
        "audio.disc",
        "Disc",
        of_total(tag.disk(), tag.disk_total()),
        DetailGroup::Content,
    );
    let date = tag.date().and_then(|ts| {
        let month = ts.month.map(u32::from);
        let day = ts.day.map(u32::from);
        let time = ts
            .hour
            .map(|h| (u32::from(h), u32::from(ts.minute.unwrap_or(0))));
        match (month, day) {
            (Some(m), Some(d)) => crate::words::date_words(u32::from(ts.year), m, d, time),
            _ if ts.year > 0 => Some(ts.year.to_string()),
            _ => None,
        }
    });
    add("audio.date", "Date", date, DetailGroup::Content);
    add(
        "audio.genre",
        "Genre",
        text(tag.genre()),
        DetailGroup::Content,
    );
    add(
        "audio.composer",
        "Composer",
        tag.get_string(ItemKey::Composer).map(str::to_owned),
        DetailGroup::Content,
    );
    if let Some(comment) = text(tag.comment()) {
        fields.push(Field::new("audio.comment", "Comment", comment, DetailGroup::Yours).private());
    }
    fields.push(Field::new(
        "audio.tags",
        "Tags",
        tag_kind(tag.tag_type()),
        DetailGroup::Technical,
    ));
    fields
}

/// The kind of audio in words: "MP3", "FLAC".
fn kind_words(kind: FileType) -> &'static str {
    match kind {
        FileType::Aac => "AAC",
        FileType::Aiff => "AIFF",
        FileType::Ape => "Monkey's Audio",
        FileType::Flac => "FLAC",
        FileType::Mpeg => "MPEG audio",
        FileType::Mp4 => "MPEG-4 audio",
        FileType::Mpc => "Musepack",
        FileType::Opus => "Opus",
        FileType::Vorbis => "Ogg Vorbis",
        FileType::Speex => "Speex",
        FileType::Wav => "WAV",
        FileType::WavPack => "WavPack",
        _ => "audio",
    }
}

/// The fields of the properties and the cover art.
fn property_fields(file: &TaggedFile) -> Vec<Field> {
    let properties = file.properties();
    let mut fields = Vec::new();
    let length = properties.duration();
    if !length.is_zero() {
        fields.push(
            Field::new(
                "audio.length",
                "Length",
                duration_words(length),
                DetailGroup::Content,
            )
            .sorted_by_length(length),
        );
    }
    if let Some(rate) = properties.audio_bitrate().or(properties.overall_bitrate())
        && rate > 0
    {
        fields.push(
            Field::new(
                "audio.bitrate",
                "Bit rate",
                bitrate_words(rate),
                DetailGroup::Technical,
            )
            .sorted_by(i64::from(rate)),
        );
    }
    if let Some(hz) = properties.sample_rate().filter(|hz| *hz > 0) {
        fields.push(
            Field::new(
                "audio.sample_rate",
                "Sample rate",
                sample_rate_words(hz),
                DetailGroup::Technical,
            )
            .sorted_by(i64::from(hz)),
        );
    }
    if let Some(channels) = properties.channels().filter(|c| *c > 0) {
        fields.push(
            Field::new(
                "audio.channels",
                "Channels",
                channels_words(channels),
                DetailGroup::Technical,
            )
            .sorted_by(i64::from(channels)),
        );
    }
    if let Some(depth) = properties.bit_depth().filter(|d| *d > 0) {
        fields.push(
            Field::new(
                "audio.bit_depth",
                "Bit depth",
                format!("{depth}-bit"),
                DetailGroup::Technical,
            )
            .sorted_by(i64::from(depth)),
        );
    }
    let pictures: Vec<_> = file.tags().iter().flat_map(Tag::pictures).collect();
    if !pictures.is_empty() {
        let mut words = count(pictures.len() as u64, "picture", "pictures");
        if let [only] = pictures.as_slice()
            && let Some(mime) = only.mime_type()
        {
            let kind = match mime.as_str() {
                "image/jpeg" => "JPEG",
                "image/png" => "PNG",
                "image/gif" => "GIF",
                "image/bmp" => "BMP",
                "image/tiff" => "TIFF",
                _ => "",
            };
            if !kind.is_empty() {
                words.push_str(", ");
                words.push_str(kind);
            }
        }
        fields.push(Field::new(
            "audio.cover",
            "Cover art",
            words,
            DetailGroup::Content,
        ));
    }
    fields.push(Field::new(
        "audio.format",
        "Audio",
        kind_words(file.file_type()),
        DetailGroup::Technical,
    ));
    fields
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use diroxide_core::{Entry, EntryKind, Place};

    use super::*;

    /// A small MP3 made in memory: an ID3 version 2.3 tag with the given
    /// frames, then `frames` silent MPEG-1 Layer III frames at 128 kbps
    /// and 44.1 kHz with no Xing header (side information all zeros is
    /// silence), as spike M4's 300 MB file was made.
    pub(crate) fn mp3(tags: &[(&[u8; 4], &str)], frames: usize) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, text) in tags {
            let mut content = vec![0u8];
            content.extend_from_slice(text.as_bytes());
            body.extend_from_slice(*id);
            body.extend_from_slice(&u32::try_from(content.len()).unwrap().to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&content);
        }
        let size = u32::try_from(body.len()).unwrap();
        let mut file = b"ID3\x03\x00\x00".to_vec();
        file.extend_from_slice(&[
            ((size >> 21) & 0x7F) as u8,
            ((size >> 14) & 0x7F) as u8,
            ((size >> 7) & 0x7F) as u8,
            (size & 0x7F) as u8,
        ]);
        file.extend_from_slice(&body);
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        for _ in 0..frames {
            file.extend_from_slice(&frame);
        }
        file
    }

    fn read(reader: &dyn Reader, bytes: Vec<u8>) -> Result<Vec<String>, MetaError> {
        let place = Place::parse(if cfg!(windows) {
            "C:\\song.mp3"
        } else {
            "/song.mp3"
        })
        .unwrap();
        let entry = Entry::new("song.mp3", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: Some("mp3"),
            group: Some("music"),
            cancel: None,
            tools: None,
        };
        assert!(reader.accepts(&request));
        let mut source = Cursor::new(bytes);
        reader
            .read(&mut source, &request, &ReadLimits::default())
            .map(|fields| fields.iter().map(|f| f.clone().cleaned().line()).collect())
    }

    #[test]
    fn the_fast_class_reads_the_tags_and_the_background_the_length() {
        let song = mp3(
            &[
                (b"TIT2", "Silence, long"),
                (b"TPE1", "Ada Example"),
                (b"TALB", "Quiet Hours"),
                (b"TRCK", "3/12"),
                (b"TCON", "Ambient"),
                (b"TPE2", "Various"),
            ],
            1000,
        );
        let tags = read(&AudioTags, song.clone()).unwrap();
        assert_eq!(
            tags,
            [
                "Title: Silence, long",
                "Artist: Ada Example",
                "Album artist: Various",
                "Album: Quiet Hours",
                "Track: 3 of 12",
                "Genre: Ambient",
                "Tags: ID3 version 2",
            ]
        );
        let properties = read(&AudioProperties, song).unwrap();
        assert_eq!(
            properties,
            [
                "Length: 26 sec",
                "Bit rate: 128 kbps",
                "Sample rate: 44.1 kHz",
                "Channels: stereo",
                "Audio: MPEG audio",
            ]
        );
        for line in tags.iter().chain(&properties) {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn not_audio_is_said_in_words() {
        assert_eq!(
            read(&AudioTags, b"just some text, not a song".to_vec()),
            Err(MetaError::unreadable("not a kind of audio read here"))
        );
        assert!(read(&AudioProperties, Vec::new()).is_err());
    }
}
