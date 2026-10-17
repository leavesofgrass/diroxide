//! `diro split FILE`: split a file into numbered parts (`report.pdf.001`,
//! `report.pdf.002`, ...) with a `.crc` sidecar, as the app's Split file
//! does, with the same plan and the same sentences (W2g, plan commit 56).
//!
//! The part size comes from one of three: `--preset NAME`, a preset from
//! `config.toml` by name, case-insensitive, or the start of one when only
//! one starts that way (`--preset email`); `--size N[unit]`, a size with
//! its unit (`20MB`, `100MiB`, `1.5GB`, `4096`); or `--parts N`, that many
//! equal parts. `diro split --presets` lists the presets, use first, with
//! their exact bytes. The parts go to the current folder, or `--into
//! FOLDER`.
//!
//! The outcome is said before anything is written ("4 parts of 18 MB,
//! the last 6 MB", "60 MB in all", "Enough space: 12 GB free on D:"),
//! then the exact bytes, then the engine's summary. A question the plan
//! asks ("This makes 1,500 parts. Go on? y or n") starts on No; with no
//! terminal it is said and answered No, and nothing is split.

use std::io::Write;

use diroxide_config::SplitPreset;
use diroxide_core::job::PartSize;
use diroxide_core::names::display_name;
use diroxide_core::{ConflictPolicy, JobId, JobKind, JobRequest, Message, Place};
use diroxide_ops::{JobControl, OpsError, Preflight};

use crate::CliError;
use crate::cmd::archive::{
    Relay, Session, Unanswered, failure_lines, line, lines, report_code, with_terminal,
};

/// How the parts are sized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitSize {
    /// A preset, by its name or the start of it.
    Preset(String),
    /// Parts of this many bytes.
    Bytes(u64),
    /// This many equal parts.
    Parts(u32),
}

/// What `diro split` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitArgs {
    /// Split a file.
    Split {
        /// The file.
        file: String,
        /// How the parts are sized.
        size: SplitSize,
        /// Where the parts go; the current folder by default.
        into: Option<String>,
        /// Replace parts of the same names (the old ones to the trash).
        replace: bool,
    },
    /// List the presets.
    Presets,
    /// Print the help.
    Help,
}

/// The help for `diro split`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro split: split a file into numbered parts with a .crc checksum file.",
        "",
        "Usage:",
        "diro split FILE --preset NAME   parts of a preset's size, such as email",
        "diro split FILE --size N        parts of N bytes, or with a unit: 20MB, 100MiB",
        "diro split FILE --parts N       N equal parts",
        "diro split --presets            list the presets, use first, exact bytes after",
        "",
        "Options:",
        "--into FOLDER      write the parts there instead of the current folder",
        "--replace          replace parts of the same names; the old go to the trash",
        "",
        "Units: KB, MB, GB and TB are powers of 1000; K, M, G, KiB, MiB and GiB",
        "are powers of 1024. The parts are FILE.001, FILE.002 and on, and",
        "FILE.crc holds the name, the size and the CRC-32; diro join puts them",
        "back, and so do cat, copy /b and 7-Zip.",
        "",
        "Exit codes:",
        "0  split",
        "1  nothing split: the file could not be read, or a question was answered no",
        "2  wrong use, such as no size or an unknown preset",
        "3  partly split: some parts failed; the output says which",
    ]
    .join("\n")
}

/// A size typed with its unit: `20MB` (powers of 1000), `100MiB` or
/// `100M` (powers of 1024), `1.5GB`, `4096` or `4096B` (bytes); case does
/// not matter, and a space before the unit is allowed. `None` for zero,
/// a fraction of a byte, or anything else.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let multiplier: u128 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        "tb" => 1_000_000_000_000,
        "k" | "kib" => 1 << 10,
        "m" | "mib" => 1 << 20,
        "g" | "gib" => 1 << 30,
        "t" | "tib" => 1 << 40,
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty()
        || fraction.contains('.')
        || fraction.len() > 9
        || whole.len() > 20
    {
        return None;
    }
    let whole: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let scale = 10u128.pow(fraction.len() as u32);
    let fraction: u128 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    let bytes = whole
        .checked_mul(multiplier)?
        .checked_add(fraction * multiplier / scale)?;
    if !(fraction * multiplier).is_multiple_of(scale) {
        return None;
    }
    u64::try_from(bytes).ok().filter(|&bytes| bytes > 0)
}

/// Reads the arguments after `split`.
pub fn parse(args: &[String]) -> Result<SplitArgs, CliError> {
    let mut file = None;
    let mut size: Option<SplitSize> = None;
    let mut into = None;
    let mut replace = false;
    let mut presets = false;
    let mut only_positionals = false;
    let mut rest = args.iter();
    let mut set_size = |size_now: SplitSize| -> Result<(), CliError> {
        if size.replace(size_now).is_some() {
            return Err(CliError::Failed(
                "choose one of --preset, --size and --parts".to_owned(),
            ));
        }
        Ok(())
    };
    while let Some(arg) = rest.next() {
        if only_positionals || arg == "-" || !arg.starts_with('-') {
            if file.replace(arg.clone()).is_some() {
                return Err(CliError::TooMany("split".to_owned()));
            }
            continue;
        }
        let (name, given) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, CliError> {
            match given.clone().or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_positionals = true,
            "--help" | "-h" => return Ok(SplitArgs::Help),
            "--presets" => presets = true,
            "--replace" => replace = true,
            "--into" => into = Some(value("--into")?),
            "--preset" => set_size(SplitSize::Preset(value("--preset")?))?,
            "--size" => {
                let text = value("--size")?;
                let bytes = parse_size(&text).ok_or_else(|| {
                    CliError::Failed(format!(
                        "{text} is not a size; write a number of bytes, or one with a unit such as 20MB or 100MiB"
                    ))
                })?;
                set_size(SplitSize::Bytes(bytes))?;
            }
            "--parts" => {
                let text = value("--parts")?;
                let parts = text.parse::<u32>().ok().filter(|&n| n > 0).ok_or_else(|| {
                    CliError::Failed(format!("{text} is not a number of parts; use 1 or more"))
                })?;
                set_size(SplitSize::Parts(parts))?;
            }
            other => return Err(CliError::UnknownOption(other.to_owned())),
        }
    }
    if presets {
        return if file.is_none() && size.is_none() {
            Ok(SplitArgs::Presets)
        } else {
            Err(CliError::Failed(
                "--presets lists the presets, and takes nothing else".to_owned(),
            ))
        };
    }
    let file = file.ok_or_else(|| CliError::Missing("split".to_owned()))?;
    let size = size.ok_or_else(|| {
        CliError::Failed("split needs a size: --preset NAME, --size N or --parts N".to_owned())
    })?;
    Ok(SplitArgs::Split {
        file,
        size,
        into,
        replace,
    })
}

/// A preset by name, case-insensitive, or the one preset whose name starts
/// with the text; the error names them all.
pub fn find_preset<'a>(presets: &'a [SplitPreset], name: &str) -> Result<&'a SplitPreset, String> {
    let wanted = name.trim().to_lowercase();
    if let Some(exact) = presets.iter().find(|p| p.name.to_lowercase() == wanted) {
        return Ok(exact);
    }
    let starting: Vec<&SplitPreset> = presets
        .iter()
        .filter(|p| !wanted.is_empty() && p.name.to_lowercase().starts_with(&wanted))
        .collect();
    if let [one] = starting.as_slice() {
        return Ok(one);
    }
    let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    let said = display_name(name);
    Err(if starting.is_empty() {
        format!(
            "There is no preset {said}. The presets: {}.",
            names.join(", ")
        )
    } else {
        let some: Vec<&str> = starting.iter().map(|p| p.name.as_str()).collect();
        format!(
            "More than one preset starts with {said}: {}. Type more of the name.",
            some.join(", ")
        )
    })
}

/// The preset's line in the list: "Email attachment, 18 MB. Parts of
/// 18,000,000 bytes."
pub fn preset_line(preset: &SplitPreset) -> String {
    format!("{}. {}.", preset.row_text(), preset.details_line())
}

/// An engine error in words: the sentences the engine already makes
/// whole, as they are, else "Could not VERB NAME: CAUSE. NEXT STEP."
pub fn ops_failed(verb: &str, name: &str, error: &OpsError) -> String {
    match error {
        OpsError::TooManyParts(sentence) | OpsError::CannotJoin(sentence) => sentence.clone(),
        other => Message::from_error(verb, name, other).text,
    }
}

/// A place's name as the lines say it.
pub fn name_of(place: &Place) -> String {
    place.name().map_or_else(
        || display_name(&place.to_string()).into_owned(),
        |name| display_name(name).into_owned(),
    )
}

/// Asks the plan's questions before the start: each one's facts said,
/// then the question. Returns false at the first No.
pub fn ask_preflight(
    questions: &[Preflight],
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<bool, CliError> {
    for question in questions {
        let said = question.lines();
        let ask = said.iter().position(|l| l.ends_with("? y or n"));
        for (n, text) in said.iter().enumerate() {
            if Some(n) != ask {
                line(out, text)?;
            }
        }
        out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
        if let Some(n) = ask
            && !session.confirm(&said[n], out)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Runs `diro split`, in the terminal, over the real places.
pub fn run(args: &SplitArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if *args == SplitArgs::Help {
        line(out, &help_text())?;
        return Ok(0);
    }
    with_terminal(|session| run_with(args, out, session))
}

/// [`run`] in a session the caller gives.
pub fn run_with(
    args: &SplitArgs,
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<i32, CliError> {
    let (file, size, into, replace) = match args {
        SplitArgs::Help => {
            line(out, &help_text())?;
            return Ok(0);
        }
        SplitArgs::Presets => {
            let said: Vec<String> = session.presets.iter().map(preset_line).collect();
            lines(out, &said)?;
            return Ok(0);
        }
        SplitArgs::Split {
            file,
            size,
            into,
            replace,
        } => (file, size, into, *replace),
    };
    let source = session.resolve("split", file)?;
    let name = name_of(&source);
    let destination = match into {
        Some(text) => session.resolve("split into", text)?,
        None => session.here.clone(),
    };
    let rule = match size {
        SplitSize::Preset(wanted) => {
            let preset = find_preset(&session.presets, wanted).map_err(CliError::Failed)?;
            line(out, &preset.row_text())?;
            PartSize::Bytes(preset.part_bytes())
        }
        SplitSize::Bytes(bytes) => PartSize::Bytes(*bytes),
        SplitSize::Parts(parts) => PartSize::EqualParts(*parts),
    };
    let mut request = JobRequest::new(JobId::new(1), JobKind::Split, vec![source])
        .to(destination)
        .split_into(rule);
    request.conflicts = if replace {
        ConflictPolicy::Overwrite
    } else {
        ConflictPolicy::KeepBoth
    };
    let engine = session.engine();
    let plan = engine
        .plan(&request)
        .map_err(|e| CliError::Failed(ops_failed("split", &name, &e)))?;
    if let Some(facts) = &plan.facts.split {
        let mut said = facts.rows();
        said.extend(facts.files.iter().map(|file| file.details_line()));
        lines(out, &said)?;
    }
    if !ask_preflight(&plan.facts.questions, out, session)? {
        line(out, "Nothing was split.")?;
        return Ok(1);
    }
    let control = JobControl::new();
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let report = session.drive(&relay, &mut unanswered, || {
        engine.run(&plan, &control, relay.as_ref())
    });
    unanswered.write(out)?;
    line(out, &report.summary())?;
    lines(out, &failure_lines(&report))?;
    Ok(report_code(&report))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cmd::archive::tests::{Fixture, args, fixture, now, session_in};
    use crate::cmd::archive::{NoAsker, Script};
    use diroxide_core::{FixedClock, UtcOffset};
    use std::fs;

    #[test]
    fn sizes_are_read_with_their_units() {
        for (text, bytes) in [
            ("4096", Some(4096)),
            ("4096B", Some(4096)),
            ("20MB", Some(20_000_000)),
            ("20 mb", Some(20_000_000)),
            ("1.5GB", Some(1_500_000_000)),
            ("100MiB", Some(104_857_600)),
            ("100M", Some(104_857_600)),
            ("1.44MB", Some(1_440_000)),
            ("2k", Some(2048)),
            ("0", None),
            ("0.5", None),
            ("1.5", None),
            ("ten", None),
            ("10 parsecs", None),
            ("1..5MB", None),
            ("", None),
        ] {
            assert_eq!(parse_size(text), bytes, "{text}");
        }
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(
            parse(&args(&["big.bin", "--preset", "email"])),
            Ok(SplitArgs::Split {
                file: "big.bin".into(),
                size: SplitSize::Preset("email".into()),
                into: None,
                replace: false,
            })
        );
        assert_eq!(
            parse(&args(&["--size=20MB", "big.bin", "--into", "out"])),
            Ok(SplitArgs::Split {
                file: "big.bin".into(),
                size: SplitSize::Bytes(20_000_000),
                into: Some("out".into()),
                replace: false,
            })
        );
        assert_eq!(parse(&args(&["--presets"])), Ok(SplitArgs::Presets));
        assert_eq!(parse(&args(&["-h"])), Ok(SplitArgs::Help));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("split".into())));
        assert_eq!(
            parse(&args(&["a", "b", "--parts", "2"])),
            Err(CliError::TooMany("split".into()))
        );
        for wrong in [
            &["big.bin"][..],
            &["big.bin", "--parts", "0"],
            &["big.bin", "--size", "huge"],
            &["big.bin", "--parts", "2", "--size", "1MB"],
            &["big.bin", "--presets"],
        ] {
            assert!(
                matches!(parse(&args(wrong)), Err(CliError::Failed(_))),
                "{wrong:?}"
            );
        }
        assert_eq!(
            parse(&args(&["big.bin", "--fast"])),
            Err(CliError::UnknownOption("--fast".into()))
        );
    }

    #[test]
    fn presets_are_found_by_name_or_its_start_in_any_case() {
        let presets = diroxide_config::builtin_split_presets();
        assert_eq!(
            find_preset(&presets, "EMAIL ATTACHMENT").unwrap().name,
            "Email attachment"
        );
        assert_eq!(
            find_preset(&presets, "email").unwrap().name,
            "Email attachment"
        );
        assert_eq!(find_preset(&presets, "cd").unwrap().name, "CD");
        assert_eq!(
            find_preset(&presets, "fat32").unwrap().name,
            "FAT32's largest file"
        );
        let missing = find_preset(&presets, "zip disk").unwrap_err();
        assert!(
            missing.starts_with("There is no preset zip disk. The presets: Email attachment, "),
            "{missing}"
        );
        let mut more = presets.clone();
        more.push(SplitPreset::new("Email server", 10_000_000));
        let both = find_preset(&more, "email").unwrap_err();
        assert!(
            both.starts_with("More than one preset starts with email"),
            "{both}"
        );
        assert_eq!(
            preset_line(&presets[0]),
            "Email attachment, 18 MB. Parts of 18,000,000 bytes."
        );
    }

    /// A file of `len` bytes that are not all alike, in the fixture's
    /// current folder.
    pub(crate) fn data_file(fixture: &Fixture, name: &str, len: usize) -> Vec<u8> {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(fixture.path("src").join(name), &bytes).unwrap();
        bytes
    }

    pub(crate) fn split_in(
        fixture: &Fixture,
        asker: &dyn crate::cmd::archive::Asker,
        presets: Vec<SplitPreset>,
        list: &[&str],
    ) -> (i32, String) {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let mut session = session_in(&clock, &fixture.here, asker);
        session.presets = presets;
        let mut out = Vec::new();
        let code = run_with(&parse(&args(list)).unwrap(), &mut out, &session);
        let text = String::from_utf8(out).unwrap();
        match code {
            Ok(code) => (code, text),
            Err(error) => (-1, format!("{text}{error}")),
        }
    }

    #[test]
    fn the_plan_is_said_before_the_parts_are_written() {
        let fixture = fixture();
        data_file(&fixture, "data.bin", 50_000);
        fs::create_dir(fixture.path("parts")).unwrap();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["data.bin", "--size", "20KB", "--into", "../parts"],
        );
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "3 parts of 20 KB, the last 10 KB");
        assert_eq!(lines[1], "50 KB in all");
        assert!(lines[2].starts_with("Enough space: "), "{text}");
        assert_eq!(
            &lines[3..],
            [
                "Parts of 20,000 bytes, the last 10,000; 50,000 bytes in all",
                "Split finished: 4 done.",
            ]
        );
        for name in [
            "data.bin.001",
            "data.bin.002",
            "data.bin.003",
            "data.bin.crc",
        ] {
            assert!(fixture.path("parts").join(name).is_file(), "{name}");
        }
        let sidecar = fs::read_to_string(fixture.path("parts").join("data.bin.crc")).unwrap();
        assert!(sidecar.starts_with("filename=data.bin\r\nsize=50000\r\ncrc32="));
    }

    #[test]
    fn a_preset_by_name_and_equal_parts() {
        let fixture = fixture();
        data_file(&fixture, "data.bin", 50_000);
        let mut presets = diroxide_config::builtin_split_presets();
        presets.push(SplitPreset::new("Tiny disk", 30_000));
        fs::create_dir(fixture.path("tiny")).unwrap();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            presets.clone(),
            &["data.bin", "--preset", "TINY", "--into", "../tiny"],
        );
        assert_eq!(code, 0, "{text}");
        assert!(
            text.starts_with("Tiny disk, 30.0 KB\n2 parts of 30 KB, the last 20 KB\n"),
            "{text}"
        );
        fs::create_dir(fixture.path("equal")).unwrap();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            presets.clone(),
            &["data.bin", "--parts", "4", "--into", "../equal"],
        );
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("4 parts of 12.5 KB each\n"), "{text}");
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            presets,
            &["data.bin", "--preset", "zip"],
        );
        assert_eq!(code, -1);
        assert!(text.starts_with("There is no preset zip."), "{text}");
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["--presets"],
        );
        assert_eq!(code, 0);
        assert_eq!(text.lines().count(), 7);
        assert!(
            text.lines()
                .all(|l| l.contains(". Parts of ") || l.contains(". Capacity "))
        );
    }

    #[test]
    fn many_parts_ask_first_and_no_splits_nothing() {
        let fixture = fixture();
        data_file(&fixture, "data.bin", 1_200);
        fs::create_dir(fixture.path("parts")).unwrap();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["data.bin", "--size", "1", "--into", "../parts"],
        );
        assert_eq!(code, 1, "{text}");
        assert!(
            text.contains("This makes 1,200 parts. Go on? y or n\nAnswered no: there is no terminal to ask.\nNothing was split.\n"),
            "{text}"
        );
        assert_eq!(fs::read_dir(fixture.path("parts")).unwrap().count(), 0);
        let yes = Script::answering(&[true], &[]);
        let (code, text) = split_in(
            &fixture,
            &yes,
            diroxide_config::builtin_split_presets(),
            &["data.bin", "--size", "1", "--into", "../parts"],
        );
        assert_eq!(code, 0, "{text}");
        assert_eq!(yes.asked(), ["This makes 1,200 parts. Go on? y or n"]);
        assert!(fixture.path("parts").join("data.bin.1200").is_file());
    }

    #[test]
    fn what_cannot_be_split_says_so() {
        let fixture = fixture();
        let (code, text) = split_in(
            &fixture,
            &NoAsker,
            diroxide_config::builtin_split_presets(),
            &["missing.bin", "--size", "1MB"],
        );
        assert_eq!(code, -1);
        assert!(text.starts_with("Could not split missing.bin: "), "{text}");
    }

    #[test]
    fn the_help_says_its_meaning_first() {
        for line in help_text().lines() {
            assert!(line.chars().count() <= 80, "{line}");
        }
        assert!(diroxide_core::format::within_cells(
            help_text().lines().next().unwrap(),
            "diro split: split a file into numbered"
        ));
    }
}
