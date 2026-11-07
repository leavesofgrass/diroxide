//! What the engine adds to a tool's run, whatever the recipe says, and
//! what it reads back (spikes V1, V2 and V4, Friday, November 6, 2026):
//!
//! - **FFmpeg** gets `-hide_banner -nostdin -nostats -loglevel error
//!   -progress pipe:1` first, so its progress arrives as `key=value` lines
//!   on standard output, and `-protocol_whitelist file` before every `-i`,
//!   so an input can only be a file (`concat:` was refused with it, read
//!   without it, by FFmpeg 7.1.5 and 9.0.2). It never gets `-y`: the
//!   staging name is new, and FFmpeg with `-nostdin` refuses a name that
//!   exists.
//! - **ImageMagick** gets resource limits first (`-limit` before the
//!   input took effect in 7.1.1-43: a width limit below the picture's
//!   stopped it with "Image width exceeds user limit").
//! - The tools' own lists are read once a plan: the encoders of
//!   `ffmpeg -encoders` for a recipe's `needs-encoder`, and the formats
//!   ImageMagick reads (`magick -list format`) for each picture's typed
//!   format.

use std::collections::BTreeSet;
use std::ffi::OsString;

/// FFmpeg's first arguments in every recipe run.
pub const FFMPEG_FIRST: [&str; 8] = [
    "-hide_banner",
    "-nostdin",
    "-nostats",
    "-loglevel",
    "error",
    "-progress",
    "pipe:1",
    "-protocol_whitelist",
];

/// ImageMagick's resource limits in every recipe run: memory, the memory
/// map, the disk cache, the picture's area, width and height.
pub const MAGICK_LIMITS: [&str; 12] = [
    "-limit", "memory", "512MiB", "-limit", "map", "1GiB", "-limit", "disk", "4GiB", "-limit",
    "area", "256MP",
];

/// ImageMagick's width and height limits.
pub const MAGICK_SIZE_LIMITS: [&str; 6] = ["-limit", "width", "32KP", "-limit", "height", "32KP"];

/// The arguments a tool runs with: the recipe's, filled, with the
/// engine's additions for FFmpeg and ImageMagick.
pub fn hardened(tool: &str, args: Vec<OsString>) -> Vec<OsString> {
    match tool {
        "ffmpeg" => {
            let mut out: Vec<OsString> = FFMPEG_FIRST[..7].iter().map(OsString::from).collect();
            let mut previous: Option<&OsString> = None;
            for argument in &args {
                let guarded = previous.is_some_and(|p| p == "file" || p == "-protocol_whitelist");
                if argument == "-i" && !guarded {
                    out.push("-protocol_whitelist".into());
                    out.push("file".into());
                }
                out.push(argument.clone());
                previous = Some(argument);
            }
            out
        }
        "magick" => MAGICK_LIMITS
            .iter()
            .chain(MAGICK_SIZE_LIMITS.iter())
            .map(OsString::from)
            .chain(args)
            .collect(),
        _ => args,
    }
}

/// The encoders `ffmpeg -hide_banner -encoders` lists: the second word of
/// each line after the dashed rule (" A....D libmp3lame  libmp3lame MP3").
pub fn ffmpeg_encoders(listing: &str) -> BTreeSet<String> {
    let mut past_rule = false;
    let mut out = BTreeSet::new();
    for line in listing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("------") {
            past_rule = true;
            continue;
        }
        if !past_rule {
            continue;
        }
        let mut words = trimmed.split_whitespace();
        if let (Some(flags), Some(name)) = (words.next(), words.next())
            && flags.len() == 6
        {
            out.insert(name.to_owned());
        }
    }
    out
}

/// The formats ImageMagick reads, from `magick -list format`: each line's
/// format name (without the `*` of native blob support) whose mode starts
/// with `r`, in capitals.
pub fn magick_readable(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let _module = words.next()?;
            let mode = words.next()?;
            let is_mode = mode.len() == 3
                && mode.as_bytes()[0] == b'r'
                && matches!(mode.as_bytes()[1], b'w' | b'-')
                && matches!(mode.as_bytes()[2], b'+' | b'-');
            is_mode.then(|| name.trim_end_matches('*').to_ascii_uppercase())
        })
        .collect()
}

/// One record of FFmpeg's `-progress pipe:1`: how far it is, in
/// microseconds of the output, and whether it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfmpegProgress {
    /// `out_time_us=N`.
    At(u64),
    /// `progress=end`.
    End,
}

/// Reads one line of FFmpeg's progress; other keys, `N/A` and anything
/// else are `None`.
pub fn ffmpeg_progress(line: &str) -> Option<FfmpegProgress> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        "out_time_us" => value.trim().parse().ok().map(FfmpegProgress::At),
        "progress" if value.trim() == "end" => Some(FfmpegProgress::End),
        _ => None,
    }
}

/// ffprobe's arguments for an input's length in seconds.
pub fn ffprobe_length_args(input: &std::path::Path) -> Vec<OsString> {
    let mut file = OsString::from("file:");
    file.push(input.as_os_str());
    [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-show_entries",
        "format=duration",
        "-of",
        "csv=p=0",
    ]
    .iter()
    .map(OsString::from)
    .chain(std::iter::once(file))
    .collect()
}

/// The length ffprobe printed, in microseconds.
pub fn ffprobe_length(stdout: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(stdout);
    let seconds: f64 = text.trim().parse().ok()?;
    (seconds.is_finite() && seconds > 0.0).then_some((seconds * 1e6) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn ffmpeg_gets_its_quiet_progress_and_a_whitelist_before_each_input() {
        let run = hardened(
            "ffmpeg",
            os(&[
                "-i",
                "file:/in/a.flac",
                "-c:a",
                "libopus",
                "/out/.diro-part-0000002a.opus",
            ]),
        );
        assert_eq!(
            run,
            os(&[
                "-hide_banner",
                "-nostdin",
                "-nostats",
                "-loglevel",
                "error",
                "-progress",
                "pipe:1",
                "-protocol_whitelist",
                "file",
                "-i",
                "file:/in/a.flac",
                "-c:a",
                "libopus",
                "/out/.diro-part-0000002a.opus",
            ])
        );
        assert!(!run.contains(&OsString::from("-y")));
        // A recipe that already whitelists is not given a second one.
        let own = hardened("ffmpeg", os(&["-protocol_whitelist", "file", "-i", "x"]));
        assert_eq!(
            own.iter().filter(|a| *a == "-protocol_whitelist").count(),
            1
        );
    }

    #[test]
    fn imagemagick_gets_its_limits_first_and_others_nothing() {
        let run = hardened("magick", os(&["PNG:/in/a.png", "/out/b.jpg"]));
        assert_eq!(run[0], "-limit");
        assert_eq!(run[run.len() - 2], "PNG:/in/a.png");
        assert_eq!(run.len(), 20);
        assert_eq!(hardened("pandoc", os(&["a"])), os(&["a"]));
    }

    /// The lines of spike V2: Debian's FFmpeg 7.1.5 in the tools image.
    #[test]
    fn the_encoder_list_is_read_past_its_rule() {
        let listing = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264 / AVC\n V....D libvpx-vp9           libvpx VP9 (codec vp9)\n A....D flac                 FLAC (Free Lossless Audio Codec)\n A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)\n A....D libopus              libopus Opus (codec opus)\n";
        let encoders = ffmpeg_encoders(listing);
        assert!(encoders.contains("libx264"));
        assert!(encoders.contains("libmp3lame"));
        assert!(!encoders.contains("="));
        assert_eq!(encoders.len(), 5);
    }

    /// The lines of spike V1: ImageMagick 7.1.1-43 in the tools image,
    /// which reads HEIC and AVIF and writes neither.
    #[test]
    fn the_format_list_says_what_imagemagick_reads() {
        let listing = "   Format  Module    Mode  Description\n-------------------------------------------------------------------------------\n     AVIF  HEIC      r--   AV1 Image File Format (1.19.8)\n      BMP* BMP       rw-   Microsoft Windows bitmap image\n     HEIC  HEIC      r--   High Efficiency Image Format (1.19.8)\n     JPEG* JPEG      rw-   Joint Photographic Experts Group JFIF format (libjpeg-turbo 2.1.5)\n      PNG* PNG       rw-   Portable Network Graphics (libpng 1.6.48)\n     WEBP* WEBP      rw+   WebP Image Format (libwebp 1.5.0 [0210])\n     XC*   XC        -w+   Constant image uniform color\n\n* native blob support\nr read support\n";
        let readable = magick_readable(listing);
        assert!(readable.contains("HEIC"));
        assert!(readable.contains("WEBP"));
        assert!(readable.contains("PNG"));
        assert!(!readable.contains("XC"));
        assert!(!readable.contains("FORMAT"));
        assert_eq!(readable.len(), 6);
    }

    /// The keys spike V2 saw on `-progress pipe:1`, in both FFmpegs.
    #[test]
    fn progress_lines_give_the_time_and_the_end() {
        assert_eq!(
            ffmpeg_progress("out_time_us=3000000"),
            Some(FfmpegProgress::At(3_000_000))
        );
        assert_eq!(ffmpeg_progress("progress=end\r"), Some(FfmpegProgress::End));
        assert_eq!(ffmpeg_progress("progress=continue"), None);
        assert_eq!(ffmpeg_progress("out_time_us=N/A"), None);
        assert_eq!(ffmpeg_progress("speed= 244x"), None);
        assert_eq!(ffprobe_length(b"3.000000\n"), Some(3_000_000));
        assert_eq!(ffprobe_length(b"N/A\n"), None);
        let args = ffprobe_length_args(std::path::Path::new("/in/a.flac"));
        assert_eq!(args.last().unwrap(), "file:/in/a.flac");
    }
}
