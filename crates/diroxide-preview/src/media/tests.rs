// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ops::ControlFlow;
use std::sync::Mutex;

use diroxide_vfs::{ToolOutput, ToolStream};

use super::*;

/// Spike FP1's answers: ffprobe 7.1.5 on files made with FFmpeg in the
/// tools container (silence, a black frame), nothing fetched.
const MP3: &[u8] = include_bytes!("../../tests/fixtures/ffprobe/song-mp3.json");
const MP4: &[u8] = include_bytes!("../../tests/fixtures/ffprobe/clip-mp4.json");
const MKV: &[u8] = include_bytes!("../../tests/fixtures/ffprobe/clip-mkv.json");

/// A runner that has ffprobe and answers with one fixture, keeping the
/// requests it was given.
struct Fake {
    answer: Vec<u8>,
    status: i32,
    seen: Mutex<Vec<ToolRequest>>,
}

impl Fake {
    fn new(answer: &[u8]) -> Self {
        Self {
            answer: answer.to_vec(),
            status: 0,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl ToolRunner for Fake {
    fn has(&self, tool: &str) -> bool {
        tool == FFPROBE
    }

    fn run(
        &self,
        request: &ToolRequest,
        _cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolRunError> {
        self.seen.lock().unwrap().push(request.clone());
        if self.status == 0 {
            Ok(ToolOutput::new(Some(0), self.answer.clone(), Vec::new()))
        } else {
            Ok(ToolOutput::new(
                Some(self.status),
                Vec::new(),
                b"\nclip.mp4: Invalid data found when processing input\n".to_vec(),
            ))
        }
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        _record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        self.run(request, cancel)
    }
}

fn lines(facts: &MediaFacts) -> Vec<String> {
    facts.fields().iter().map(Field::line).collect()
}

#[test]
fn the_golden_key_names_read_into_words() {
    let mp3 = parse(MP3).unwrap();
    assert_eq!(mp3.sentence("MP3"), "MP3, 3 sec, 129 kbps, stereo");
    assert_eq!(
        lines(&mp3),
        [
            "Title: Example Song",
            "Artist: Ada Example",
            "Duration: 3 sec",
            "Audio: MP3, stereo, 44.1 kHz",
            "Bit rate: 129 kbps",
        ]
    );
    let mp4 = parse(MP4).unwrap();
    assert_eq!(mp4.sentence("MP4"), "MP4, 2 sec, 320 by 240");
    assert!(mp4.is_video());
    assert_eq!(
        lines(&mp4),
        [
            "Title: Example Clip",
            "Duration: 2 sec",
            "Dimensions: 320 by 240",
            "Video: H.264, 25 frames a second",
            "Audio: AAC, stereo, 48 kHz",
            "Bit rate: 21 kbps",
        ]
    );
    let mkv = parse(MKV).unwrap();
    assert_eq!(mkv.sentence("MKV"), "MKV, 2 sec, 640 by 360");
    let said = lines(&mkv);
    assert!(said.contains(&"Video: H.264, 30 frames a second".to_owned()));
    assert!(said.contains(&"Audio: Opus, mono, 48 kHz, English".to_owned()));
    for line in said.iter().chain(&lines(&mp3)).chain(&lines(&mp4)) {
        assert!(line.chars().count() <= 40, "{line}");
    }
}

#[test]
fn the_words_for_lengths_rates_channels_and_codecs() {
    assert_eq!(duration_words(221.0), "3 min 41 sec");
    assert_eq!(duration_words(725.4), "12 min 5 sec");
    assert_eq!(duration_words(3_725.0), "1 hr 2 min");
    assert_eq!(duration_words(0.4), "0 sec");
    assert_eq!(bit_rate_words(320_000), "320 kbps");
    assert_eq!(bit_rate_words(12_480_000), "12.5 Mbps");
    assert_eq!(
        frame_rate("30000/1001").as_deref(),
        Some("29.97 frames a second")
    );
    assert_eq!(frame_rate("0/0"), None);
    assert_eq!(frame_rate("1/0"), None);
    assert_eq!(
        channel_words(Some(6), Some("5.1(side)")).as_deref(),
        Some("5.1 surround")
    );
    assert_eq!(channel_words(Some(3), None).as_deref(), Some("3 channels"));
    assert_eq!(sample_rate_words("44100").as_deref(), Some("44.1 kHz"));
    assert_eq!(codec_words("pcm_s16le"), "PCM");
    assert_eq!(codec_words("weird\u{202E}codec"), "WEIRD?CODEC");
    assert_eq!(language_words("und"), None);
    // The lines R28 quotes, as these functions make them.
    let audio = MediaFacts {
        seconds: Some(221.0),
        bit_rate: Some(320_000),
        streams: vec![Stream {
            kind: StreamKind::Audio,
            words: "MP3, stereo, 44.1 kHz".to_owned(),
            size: None,
            channels: Some("stereo".to_owned()),
        }],
        ..MediaFacts::default()
    };
    let line = audio.sentence("MP3");
    assert_eq!(line, "MP3, 3 min 41 sec, 320 kbps, stereo");
    assert_eq!(line.chars().count(), 35);
    let video = MediaFacts {
        seconds: Some(725.0),
        streams: vec![Stream {
            kind: StreamKind::Video,
            words: "H.264, 30 frames a second".to_owned(),
            size: Some((1920, 1080)),
            channels: None,
        }],
        ..MediaFacts::default()
    };
    assert_eq!(video.sentence("MP4"), "MP4, 12 min 5 sec, 1920 by 1080");
    assert_eq!(NOT_FOUND.chars().count(), 29);
}

#[test]
fn the_arguments_name_the_file_by_its_protocol_and_never_count_frames() {
    let args: Vec<String> = arguments(&Input::Path("D:/music/-rf song.mp3".to_owned()))
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-protocol_whitelist",
            "file",
            "-i",
            "file:D:/music/-rf song.mp3"
        ]
    );
    assert!(!args.iter().any(|a| a.contains("count_frames")));
    let piped = arguments(&Input::Bytes(Vec::new()));
    assert_eq!(piped[7], "pipe");
    assert_eq!(piped[9], "pipe:0");
    let fake = Fake::new(MP3);
    let facts = probe(&fake, &CancelToken::new(), Input::Bytes(b"ID3".to_vec())).unwrap();
    assert_eq!(facts.title.as_deref(), Some("Example Song"));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen[0].tool, FFPROBE);
    assert_eq!(seen[0].limits, LIMITS);
    assert_eq!(seen[0].input, ToolInput::Bytes(b"ID3".to_vec()));
}

#[test]
fn a_failed_run_and_a_missing_tool_are_said_in_words() {
    let mut fake = Fake::new(b"");
    fake.status = 1;
    let error = probe(&fake, &CancelToken::new(), Input::Path("a.mp4".to_owned())).unwrap_err();
    assert_eq!(
        error.to_string(),
        "clip.mp4: Invalid data found when processing input"
    );
    let error = probe(
        &diroxide_vfs::NoTools,
        &CancelToken::new(),
        Input::Path("a.mp4".to_owned()),
    )
    .unwrap_err();
    assert_eq!(error, MediaError::NotFound);
    assert_eq!(parse(b"not json").unwrap_err(), MediaError::BadAnswer);
}

#[test]
fn the_previewer_says_no_preview_without_ffprobe_and_rows_with_it() {
    use crate::{LatestOnly, Previews};
    use diroxide_core::Entry;
    let folder = tempfile::Builder::new()
        .prefix("media-")
        .tempdir_in(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .unwrap();
    let path = folder.path().join("clip.mp4");
    std::fs::write(&path, b"\0\0\0\x18ftypisom").unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new("clip.mp4", EntryKind::File).with_size(5_156),
        generation: latest.next(),
        max_lines: 20,
    };
    let types = diroxide_filetype::TypeRegistry::builtin();
    let mp4 = types.get("mp4");
    let without = Previews::builtin()
        .render(
            &request,
            mp4,
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(without.kind, PreviewKind::Media);
    assert_eq!(without.summary, NOT_FOUND);
    let ctx = OpContext::new().with_tools(Arc::new(Fake::new(MP4)));
    let with = Previews::builtin()
        .render(&request, mp4, &latest, &Registry::with_local(), &ctx)
        .unwrap();
    assert_eq!(with.summary, "MP4, 2 sec, 320 by 240");
    assert!(
        with.lines
            .contains(&"Video: H.264, 25 frames a second".to_owned())
    );
}

#[test]
fn the_meta_reader_is_on_demand_and_never_takes_a_playlist() {
    use diroxide_core::Entry;
    use diroxide_meta::Reader;
    let reader = MediaFields::new(Arc::new(Fake::new(MKV)));
    assert_eq!((reader.id(), reader.cost()), ("media", Cost::OnDemand));
    let place = Place::parse("D:/videos/clip.mkv").unwrap();
    let entry = Entry::new("clip.mkv", EntryKind::File);
    let request = ReadRequest {
        place: &place,
        entry: &entry,
        type_id: Some("mkv"),
        group: Some("video"),
    };
    assert!(reader.accepts(&request));
    let playlist = ReadRequest {
        type_id: Some("m3u"),
        group: Some("music"),
        ..request
    };
    assert!(!reader.accepts(&playlist));
    let mut source = std::io::Cursor::new(Vec::new());
    let fields = reader
        .read(&mut source, &request, &ReadLimits::default())
        .unwrap();
    assert_eq!(fields[0].line(), "Title: Example Matroska");
}
