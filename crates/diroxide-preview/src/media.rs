//! Audio and video in words through `ffprobe` (alpha 4, R30 commit 35;
//! R28 part 2.2): the tools registry's `ffprobe` run as an argument list
//! through the context's runner, never a shell string, with `-v error
//! -print_format json -show_format -show_streams`, ten seconds and 1 MiB
//! of output, and never `-count_frames`, which decodes every frame. A
//! local file is named with the `file:` protocol and only that protocol
//! allowed, so a playlist inside a file can reach no other place; a file
//! elsewhere goes in on standard input, its first [`MAX_PIPED`] bytes.
//! Nothing is played.
//!
//! The key names are spike FP1's, read from ffprobe 7.1.5's output on an
//! MP3, an MP4 and an MKV made with FFmpeg in the tools container
//! (`tests/fixtures/ffprobe/`): `streams[]` with `index`, `codec_name`,
//! `codec_type`, `profile`, `width`, `height`, `avg_frame_rate`,
//! `r_frame_rate`, `sample_rate` (a string), `channels`,
//! `channel_layout`, `duration` and `bit_rate` (strings, absent per
//! stream in Matroska), `disposition.attached_pic`, `tags.language`,
//! `tags.title`; `format` with `format_name`, `format_long_name`,
//! `duration`, `size`, `bit_rate` (strings) and `tags` (`title`,
//! `artist`, `album`; Matroska writes some keys in capitals).
//!
//! The sentence: "MP3, 3 min 41 sec, 320 kbps, stereo"; "MP4, 12 min 5
//! sec, 1920 by 1080". One row per stream: "Video: H.264, 30 frames a
//! second"; "Audio: AAC, stereo, 48 kHz, English"; the picture's size in
//! its own row, "Dimensions: 1920 by 1080". Without ffprobe: "No
//! preview: ffprobe not found".

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{DetailGroup, DetailRow, EntryKind, Place};
use diroxide_filetype::FileType;
use diroxide_meta::{Cost, Field, MetaError, ReadLimits, ReadRequest};
use diroxide_vfs::{
    CancelToken, OpContext, ReadSeek, Registry, RunLimits, ToolInput, ToolRequest, ToolRunError,
    ToolRunner, VfsError,
};
use serde::Deserialize;
use thiserror::Error;

use crate::words::{clean_text, count_words};
use crate::{Preview, PreviewError, PreviewKind, PreviewRequest, Previewer};

/// The tools registry's id for ffprobe.
pub const FFPROBE: &str = "ffprobe";

/// What the preview says when ffprobe is not installed.
pub const NOT_FOUND: &str = "No preview: ffprobe not found";

/// The most bytes of a file that is not on a local disk piped in.
pub const MAX_PIPED: u64 = 16 * 1024 * 1024;

/// The run's limits: ten seconds and 1 MiB of output.
pub const LIMITS: RunLimits = RunLimits::new(Duration::from_secs(10), 1024 * 1024);

/// The most characters kept of a tag.
const MAX_TAG: usize = 200;

/// Why a file could not be probed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MediaError {
    /// ffprobe is not installed, or is turned off.
    #[error("ffprobe not found")]
    NotFound,
    /// The run failed: ffprobe's own words, or the runner's.
    #[error("{0}")]
    Failed(String),
    /// ffprobe answered with something that is not its JSON.
    #[error("ffprobe's answer could not be read")]
    BadAnswer,
    /// The run was cancelled.
    #[error("it was cancelled")]
    Cancelled,
}

/// Where the file comes from for ffprobe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A file on a local disk, named.
    Path(String),
    /// A file's bytes, on standard input.
    Bytes(Vec<u8>),
}

/// ffprobe's argument list for an input: never a shell string, never
/// `-count_frames`.
pub fn arguments(input: &Input) -> Vec<OsString> {
    let (whitelist, name) = match input {
        Input::Path(path) => ("file", format!("file:{path}")),
        Input::Bytes(_) => ("pipe", "pipe:0".to_owned()),
    };
    [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-protocol_whitelist",
        whitelist,
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(OsString::from(name)))
    .collect()
}

/// Runs ffprobe on an input through a runner and reads its answer.
pub fn probe(
    tools: &dyn ToolRunner,
    cancel: &CancelToken,
    input: Input,
) -> Result<MediaFacts, MediaError> {
    if !tools.has(FFPROBE) {
        return Err(MediaError::NotFound);
    }
    let args = arguments(&input);
    let mut request = ToolRequest::new(FFPROBE, args).with_limits(LIMITS);
    if let Input::Bytes(bytes) = input {
        request = request.with_input(ToolInput::Bytes(bytes));
    }
    let output = tools.run(&request, cancel).map_err(|e| match e {
        ToolRunError::NotInstalled { .. } => MediaError::NotFound,
        ToolRunError::Cancelled => MediaError::Cancelled,
        other => MediaError::Failed(other.to_string()),
    })?;
    if !output.succeeded() {
        let said = String::from_utf8_lossy(&output.stderr);
        let first = said.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let words = clean_text(first, 80);
        return Err(MediaError::Failed(if words.is_empty() {
            "ffprobe could not read it".to_owned()
        } else {
            words
        }));
    }
    parse(&output.stdout)
}

/// ffprobe's JSON, as far as the words need it.
#[derive(Debug, Default, Deserialize)]
struct Answer {
    #[serde(default)]
    streams: Vec<RawStream>,
    #[serde(default)]
    format: RawFormat,
}

#[derive(Debug, Default, Deserialize)]
struct RawStream {
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    channels: Option<u32>,
    #[serde(default)]
    channel_layout: Option<String>,
    #[serde(default)]
    disposition: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    tags: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawFormat {
    #[serde(default)]
    format_name: Option<String>,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
    #[serde(default)]
    tags: BTreeMap<String, serde_json::Value>,
}

/// A tag's value by its key, any case, cleaned; `None` when blank.
fn tag(tags: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<String> {
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.as_str())
        .map(|v| clean_text(v, MAX_TAG))
        .filter(|v| !v.is_empty())
}

/// What a stream is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StreamKind {
    /// Moving pictures.
    Video,
    /// Sound.
    Audio,
    /// Subtitles.
    Subtitles,
    /// A still picture carried with the sound: an album's cover.
    Cover,
    /// Anything else: data, attachments.
    Other,
}

impl StreamKind {
    /// The row's label: "Video", "Audio", "Subtitles", "Cover picture",
    /// "Data".
    pub const fn label(self) -> &'static str {
        match self {
            StreamKind::Video => "Video",
            StreamKind::Audio => "Audio",
            StreamKind::Subtitles => "Subtitles",
            StreamKind::Cover => "Cover picture",
            StreamKind::Other => "Data",
        }
    }
}

/// One stream in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    /// What it is.
    pub kind: StreamKind,
    /// Its row's value: "H.264, 30 frames a second".
    pub words: String,
    /// Its size, for a picture.
    pub size: Option<(u32, u32)>,
    /// Its channels in words, for sound: "stereo".
    pub channels: Option<String>,
}

/// What ffprobe says about a file.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MediaFacts {
    /// ffprobe's short format names: "mov,mp4,m4a,3gp,3g2,mj2".
    pub format_name: Option<String>,
    /// The length in seconds.
    pub seconds: Option<f64>,
    /// The bits a second, all streams.
    pub bit_rate: Option<u64>,
    /// The streams, in the file's order.
    pub streams: Vec<Stream>,
    /// The file's own title, artist and album tags, cleaned.
    pub title: Option<String>,
    /// The artist tag.
    pub artist: Option<String>,
    /// The album tag.
    pub album: Option<String>,
}

/// Reads ffprobe's JSON answer.
pub fn parse(json: &[u8]) -> Result<MediaFacts, MediaError> {
    let answer: Answer = serde_json::from_slice(json).map_err(|_| MediaError::BadAnswer)?;
    let mut facts = MediaFacts {
        format_name: answer.format.format_name.clone(),
        seconds: answer
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d >= 0.0),
        bit_rate: answer
            .format
            .bit_rate
            .as_deref()
            .and_then(|b| b.parse().ok()),
        title: tag(&answer.format.tags, "title"),
        artist: tag(&answer.format.tags, "artist"),
        album: tag(&answer.format.tags, "album"),
        streams: Vec::new(),
    };
    for raw in &answer.streams {
        facts.streams.push(stream_of(raw));
    }
    Ok(facts)
}

fn stream_of(raw: &RawStream) -> Stream {
    let cover = raw
        .disposition
        .get("attached_pic")
        .and_then(serde_json::Value::as_i64)
        == Some(1);
    let kind = match (raw.codec_type.as_deref(), cover) {
        (Some("video"), true) => StreamKind::Cover,
        (Some("video"), false) => StreamKind::Video,
        (Some("audio"), _) => StreamKind::Audio,
        (Some("subtitle"), _) => StreamKind::Subtitles,
        _ => StreamKind::Other,
    };
    let codec = codec_words(raw.codec_name.as_deref().unwrap_or("unknown"));
    let mut parts = vec![codec];
    let size = match (raw.width, raw.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    };
    let mut channels = None;
    match kind {
        StreamKind::Video => {
            // The size is the Dimensions row's, so the stream's row keeps
            // its meaning in 40 cells.
            let rate = raw
                .avg_frame_rate
                .as_deref()
                .and_then(frame_rate)
                .or_else(|| raw.r_frame_rate.as_deref().and_then(frame_rate));
            if let Some(rate) = rate {
                parts.push(rate);
            }
        }
        StreamKind::Cover => {
            if let Some((w, h)) = size {
                parts = vec![format!("{w} by {h}")];
            }
        }
        StreamKind::Audio => {
            channels = channel_words(raw.channels, raw.channel_layout.as_deref());
            parts.extend(channels.clone());
            if let Some(rate) = raw.sample_rate.as_deref().and_then(sample_rate_words) {
                parts.push(rate);
            }
        }
        StreamKind::Subtitles | StreamKind::Other => {}
    }
    if let Some(language) = tag(&raw.tags, "language").and_then(|l| language_words(&l)) {
        parts.push(language);
    }
    if let Some(title) = tag(&raw.tags, "title") {
        parts.push(format!("titled {title}"));
    }
    Stream {
        kind,
        words: parts.join(", "),
        size,
        channels,
    }
}

/// A codec's name as said: "H.264", "AAC", "Opus"; an unknown one as
/// ffprobe names it, cleaned.
pub fn codec_words(codec: &str) -> String {
    let said = match codec {
        "h264" => "H.264",
        "hevc" => "H.265",
        "av1" => "AV1",
        "vp9" => "VP9",
        "vp8" => "VP8",
        "mpeg4" => "MPEG-4 video",
        "mpeg2video" => "MPEG-2 video",
        "mpeg1video" => "MPEG-1 video",
        "prores" => "ProRes",
        "theora" => "Theora",
        "mjpeg" => "Motion JPEG",
        "png" => "PNG",
        "aac" => "AAC",
        "mp3" => "MP3",
        "mp2" => "MP2",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "flac" => "FLAC",
        "alac" => "ALAC",
        "ac3" => "Dolby Digital",
        "eac3" => "Dolby Digital Plus",
        "truehd" => "Dolby TrueHD",
        "dts" => "DTS",
        "wmav2" | "wmav1" => "Windows Media Audio",
        "amr_nb" | "amr_wb" => "AMR",
        "subrip" => "SubRip",
        "ass" | "ssa" => "ASS",
        "webvtt" => "WebVTT",
        "mov_text" => "timed text",
        "dvd_subtitle" => "DVD subtitles",
        "hdmv_pgs_subtitle" => "Blu-ray subtitles",
        other if other.starts_with("pcm_") => "PCM",
        other => return clean_text(&other.to_ascii_uppercase(), 40),
    };
    said.to_owned()
}

/// "30 frames a second", "29.97 frames a second"; `None` for 0/0.
pub fn frame_rate(ratio: &str) -> Option<String> {
    let (n, d) = ratio.split_once('/')?;
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    if n <= 0.0 || d <= 0.0 {
        return None;
    }
    let rate = n / d;
    if !rate.is_finite() || rate > 1_000.0 {
        return None;
    }
    let rounded = (rate * 100.0).round() / 100.0;
    let shown = if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    };
    Some(format!("{shown} frames a second"))
}

/// "mono", "stereo", "5.1 surround", "7.1 surround", "3 channels".
pub fn channel_words(channels: Option<u32>, layout: Option<&str>) -> Option<String> {
    let layout = layout.unwrap_or("");
    Some(match channels? {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        6 if layout.starts_with("5.1") => "5.1 surround".to_owned(),
        8 if layout.starts_with("7.1") => "7.1 surround".to_owned(),
        n => count_words(u64::from(n), "channel", "channels"),
    })
}

/// "48 kHz", "44.1 kHz".
pub fn sample_rate_words(rate: &str) -> Option<String> {
    let hz: u64 = rate.trim().parse().ok().filter(|r| *r > 0)?;
    let khz = hz as f64 / 1000.0;
    Some(if khz.fract() == 0.0 {
        format!("{khz:.0} kHz")
    } else {
        format!("{} kHz", (khz * 10.0).round() / 10.0)
    })
}

/// "320 kbps", "12.5 Mbps".
pub fn bit_rate_words(bits: u64) -> String {
    let kbps = (bits as f64 / 1000.0).round() as u64;
    if kbps >= 10_000 {
        format!("{:.1} Mbps", bits as f64 / 1_000_000.0)
    } else {
        format!("{kbps} kbps")
    }
}

/// A length in words: "3 min 41 sec", "1 hr 2 min", "8 sec".
pub fn duration_words(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h} hr {m} min")
    } else if m > 0 {
        format!("{m} min {s} sec")
    } else {
        format!("{s} sec")
    }
}

/// A language code as said, for the common ones: "English"; `None` for
/// "und" (undetermined); the code itself otherwise.
pub fn language_words(code: &str) -> Option<String> {
    let said = match code.to_ascii_lowercase().as_str() {
        "und" | "" => return None,
        "eng" | "en" => "English",
        "spa" | "es" => "Spanish",
        "fra" | "fre" | "fr" => "French",
        "deu" | "ger" | "de" => "German",
        "ita" | "it" => "Italian",
        "por" | "pt" => "Portuguese",
        "jpn" | "ja" => "Japanese",
        "zho" | "chi" | "zh" => "Chinese",
        "kor" | "ko" => "Korean",
        "rus" | "ru" => "Russian",
        "ara" | "ar" => "Arabic",
        "hin" | "hi" => "Hindi",
        "nld" | "dut" | "nl" => "Dutch",
        "swe" | "sv" => "Swedish",
        "pol" | "pl" => "Polish",
        "vie" | "vi" => "Vietnamese",
        other => return Some(other.to_owned()),
    };
    Some(said.to_owned())
}

impl MediaFacts {
    /// Whether it holds moving pictures.
    pub fn is_video(&self) -> bool {
        self.streams.iter().any(|s| s.kind == StreamKind::Video)
    }

    /// The sentence: the format, the length, then the picture's size for
    /// a video or the bit rate and channels for sound: "MP4, 12 min 5
    /// sec, 1920 by 1080"; "MP3, 3 min 41 sec, 320 kbps, stereo".
    pub fn sentence(&self, word: &str) -> String {
        let mut parts = vec![word.to_owned()];
        if let Some(seconds) = self.seconds {
            parts.push(duration_words(seconds));
        }
        if let Some((w, h)) = self
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .and_then(|s| s.size)
        {
            parts.push(format!("{w} by {h}"));
        } else {
            if let Some(bits) = self.bit_rate {
                parts.push(bit_rate_words(bits));
            }
            if let Some(channels) = self
                .streams
                .iter()
                .find(|s| s.kind == StreamKind::Audio)
                .and_then(|s| s.channels.clone())
            {
                parts.push(channels);
            }
        }
        parts.join(", ")
    }

    /// The facts as metadata fields: the tags, the length, one row per
    /// stream, then the bit rate.
    pub fn fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        for (key, label, value) in [
            ("media.title", "Title", &self.title),
            ("media.artist", "Artist", &self.artist),
            ("media.album", "Album", &self.album),
        ] {
            if let Some(value) = value {
                out.push(Field::new(key, label, value, DetailGroup::Content));
            }
        }
        if let Some(seconds) = self.seconds {
            out.push(Field::new(
                "media.duration",
                "Duration",
                duration_words(seconds),
                DetailGroup::Content,
            ));
        }
        if let Some((w, h)) = self
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .and_then(|s| s.size)
        {
            out.push(Field::new(
                "media.dimensions",
                "Dimensions",
                format!("{w} by {h}"),
                DetailGroup::Content,
            ));
        }
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for stream in &self.streams {
            let label = stream.kind.label();
            let n = counts.entry(label).or_insert(0);
            *n += 1;
            let key = format!(
                "media.{}.{}",
                label.to_ascii_lowercase().replace(' ', "_"),
                *n
            );
            out.push(Field::new(
                key,
                label,
                &stream.words,
                if stream.kind == StreamKind::Other {
                    DetailGroup::Technical
                } else {
                    DetailGroup::Content
                },
            ));
        }
        if let Some(bits) = self.bit_rate {
            out.push(Field::new(
                "media.bit_rate",
                "Bit rate",
                bit_rate_words(bits),
                DetailGroup::Technical,
            ));
        }
        out
    }
}

/// The type ids the media words read: the music and video groups.
pub fn takes(file_type: Option<&FileType>, name: &str) -> bool {
    match file_type {
        // A playlist names other files and is never probed; MIDI is
        // notes, not sound.
        Some(t) => {
            matches!(t.group.as_deref(), Some("music" | "video"))
                && !matches!(t.id.as_str(), "m3u" | "midi")
        }
        None => {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            ext.is_some_and(|e| {
                [
                    "mp3", "flac", "ogg", "oga", "opus", "wav", "aif", "aiff", "m4a", "aac", "wma",
                    "mka", "mp4", "m4v", "mov", "avi", "mkv", "webm", "mpg", "mpeg", "mts", "m2ts",
                    "flv", "wmv", "ogv", "3gp",
                ]
                .contains(&e.as_str())
            })
        }
    }
}

/// The input for a place: the path of a local file, else its first
/// [`MAX_PIPED`] bytes from `source`.
fn input_for(
    place: &Place,
    source: impl FnOnce() -> Result<Vec<u8>, VfsError>,
) -> Result<Input, VfsError> {
    match place.local_path() {
        Some(path) => Ok(Input::Path(path.to_owned())),
        None => source().map(Input::Bytes),
    }
}

/// The ffprobe previewer: a details preview whose summary is the
/// sentence and whose lines are the rows. Tool-backed: the app runs it in
/// the quick view after its wait, in the details list or on Preview,
/// never on a plain cursor move (R28 2.2).
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaPreviewer;

impl Previewer for MediaPreviewer {
    fn name(&self) -> &str {
        "media"
    }

    fn version(&self) -> u32 {
        1
    }

    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool {
        request.entry.kind == EntryKind::File && takes(file_type, &request.entry.name)
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let base = crate::detail_rows(&request.entry, file_type);
        if !ctx.tools.has(FFPROBE) {
            return Ok(crate::details_preview(
                PreviewKind::Media,
                NOT_FOUND.to_owned(),
                base,
                [DetailRow::new(
                    "media.tool",
                    "Details",
                    "ffprobe not found",
                    DetailGroup::Technical,
                )],
            ));
        }
        let input = input_for(&request.place, || {
            let provider = registry.resolve(&request.place)?;
            let mut bytes = Vec::new();
            provider
                .read(&request.place, ctx)?
                .take(MAX_PIPED)
                .read_to_end(&mut bytes)
                .map_err(VfsError::from)?;
            Ok(bytes)
        })?;
        ctx.check()?;
        let word =
            file_type.map_or_else(|| crate::details(&request.entry, None), |t| t.short.clone());
        match probe(ctx.tools.as_ref(), &ctx.cancel, input) {
            Ok(facts) => Ok(crate::details_preview(
                PreviewKind::Media,
                facts.sentence(&word),
                base,
                facts.fields().iter().map(Field::row),
            )),
            Err(MediaError::Cancelled) => Err(PreviewError::Vfs(VfsError::Cancelled)),
            Err(MediaError::NotFound) => Ok(crate::details_preview(
                PreviewKind::Media,
                NOT_FOUND.to_owned(),
                base,
                std::iter::empty(),
            )),
            Err(error) => Ok(crate::details_preview(
                PreviewKind::Media,
                format!("{word}, unreadable: {error}"),
                base,
                std::iter::empty(),
            )),
        }
    }
}

/// The keys of the fields ffprobe's reader gives, each stream's first
/// of its kind among them, for the column chooser and a failed read's
/// cells (VL's request 2, built in lane VA).
pub const FIELD_KEYS: [&str; 11] = [
    "media.title",
    "media.artist",
    "media.album",
    "media.duration",
    "media.dimensions",
    "media.video.1",
    "media.audio.1",
    "media.subtitles.1",
    "media.cover_picture.1",
    "media.data.1",
    "media.bit_rate",
];

/// The ffprobe reader as a metadata reader (R30 commit 12's joint), on
/// demand, since it starts a program. The reader holds its runner: the
/// trait's `read` carries no context.
#[derive(Clone)]
pub struct MediaFields {
    /// The runner ffprobe goes through.
    pub tools: Arc<dyn ToolRunner>,
    /// The token a run checks; a fresh one by default.
    pub cancel: CancelToken,
}

impl std::fmt::Debug for MediaFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaFields").finish_non_exhaustive()
    }
}

impl MediaFields {
    /// The reader over a runner.
    pub fn new(tools: Arc<dyn ToolRunner>) -> Self {
        Self {
            tools,
            cancel: CancelToken::new(),
        }
    }
}

impl diroxide_meta::Reader for MediaFields {
    fn id(&self) -> &'static str {
        "media"
    }

    fn cost(&self) -> Cost {
        Cost::OnDemand
    }

    fn keys(&self) -> &'static [&'static str] {
        &FIELD_KEYS
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        match (request.group, request.type_id) {
            (_, Some("m3u" | "midi")) => false,
            (Some(group), _) => matches!(group, "music" | "video"),
            (None, _) => takes(None, &request.entry.name),
        }
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        request: &ReadRequest<'_>,
        limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let cap = limits.max_bytes.min(MAX_PIPED);
        let input = input_for(request.place, || {
            let mut bytes = Vec::new();
            source
                .take(cap)
                .read_to_end(&mut bytes)
                .map_err(VfsError::from)?;
            Ok(bytes)
        })?;
        match probe(self.tools.as_ref(), &self.cancel, input) {
            Ok(facts) => Ok(facts.fields()),
            Err(MediaError::Cancelled) => Err(MetaError::Place(VfsError::Cancelled)),
            Err(error) => Err(MetaError::Unreadable(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;
