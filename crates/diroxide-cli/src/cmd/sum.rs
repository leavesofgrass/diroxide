//! `diro sum FILE...` and `diro sum --check LIST`: checksums computed and
//! checked as plain lines, for scripts and the Braille display (alpha.7,
//! the owner's answer 121).
//!
//! - `diro sum FILE...` prints one line a file, the name first:
//!   "example-tool.zip, SHA-256, ba7816bf...". `--algorithm NAME` picks
//!   SHA-512, BLAKE3, MD5 or CRC-32 instead of SHA-256 (the Checksum
//!   command's default, what publishers post); `--list` prints the lines a
//!   list holds instead (`HASH  NAME`, as `sha256sum` writes them), to keep
//!   in `SHA256SUMS`.
//! - `diro sum --check LIST` reads every file the list names again,
//!   prints the verdict first ("All 12 files match SHA256SUMS"), then the
//!   failures, and exits as `sha256sum --check` does: 0 when every file
//!   matches, 1 for any difference, missing or unreadable file or line it
//!   could not read, 2 for wrong use.
//!
//! A match shows the files are not damaged; it never says who made them.

use std::io::Write;

use diroxide_core::Place;
use diroxide_core::names::display_name;
use diroxide_vfs::Registry;

use crate::CliError;
use crate::open::current_place;

/// `diro sum`'s arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SumArgs {
    /// The files, as typed; or with `--check`, the one list.
    pub args: Vec<String>,
    /// `--check`: check the list named.
    pub check: bool,
    /// `--list`: print the lines a list holds.
    pub list: bool,
    /// `--algorithm NAME`, as typed.
    pub algorithm: Option<String>,
    /// Print the help for `diro sum` and nothing else.
    pub help: bool,
}

/// The help for `diro sum`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro sum: checksums as plain lines, the name first.",
        "",
        "Usage:",
        "diro sum FILE...           each file's SHA-256: name, algorithm, value",
        "diro sum --check LIST      check every file LIST names, the verdict first",
        "",
        "Options:",
        "--algorithm NAME           sha256 (the default), sha512, blake3, md5 or crc32",
        "--list                     print the lines a list holds, HASH  NAME",
        "",
        "Exit codes:",
        "0  done; with --check, every file matches",
        "1  a file could not be read; with --check, any difference, missing or unreadable file",
        "2  wrong use",
    ]
    .join("\n")
}

/// Reads the arguments after `sum`.
pub fn parse(args: &[String]) -> Result<SumArgs, CliError> {
    let mut parsed = SumArgs::default();
    let mut only_files = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_files {
            parsed.args.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--" => only_files = true,
            "--help" | "-h" => parsed.help = true,
            "--check" | "-c" => parsed.check = true,
            "--list" => parsed.list = true,
            "--algorithm" | "-a" => {
                let value = inline
                    .or_else(|| rest.next().cloned())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| CliError::Missing("--algorithm".to_owned()))?;
                if diroxide_app::metadata::algorithm_named(&value).is_none() {
                    return Err(CliError::UnknownOption(format!("--algorithm {value}")));
                }
                parsed.algorithm = Some(value);
            }
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            file => parsed.args.push(file.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    if parsed.args.is_empty() {
        return Err(CliError::Missing("sum".to_owned()));
    }
    if parsed.check && parsed.args.len() > 1 {
        return Err(CliError::TooMany(parsed.args[0].clone()));
    }
    if parsed.check && (parsed.list || parsed.algorithm.is_some()) {
        return Err(CliError::Failed(
            "--check reads the algorithm from the list; leave out --list and --algorithm"
                .to_owned(),
        ));
    }
    Ok(parsed)
}

/// Runs `diro sum` on this computer's files.
pub fn run(args: &SumArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    run_with(args, &Registry::with_local(), out)
}

/// Runs `diro sum` over a registry.
pub fn run_with(args: &SumArgs, registry: &Registry, out: &mut dyn Write) -> Result<i32, CliError> {
    let write = |out: &mut dyn Write, line: &str| {
        writeln!(out, "{line}").map_err(|e| CliError::Failed(e.to_string()))
    };
    if args.help {
        write(out, &help_text())?;
        return Ok(0);
    }
    let here = current_place()?;
    let place_of = |typed: &str| {
        Place::resolve(typed, &here)
            .map_err(|e| CliError::Failed(format!("{typed} cannot be read as a path: {e}")))
    };
    if args.check {
        let list = place_of(&args.args[0])?;
        let (lines, code) = diroxide_app::metadata::check_list(registry, &list).map_err(|why| {
            CliError::Failed(format!(
                "Could not check {}: {why}.",
                list.name().map(display_name).unwrap_or_default()
            ))
        })?;
        for line in lines {
            write(out, &line)?;
        }
        return Ok(code);
    }
    let algorithm = args
        .algorithm
        .as_deref()
        .and_then(diroxide_app::metadata::algorithm_named)
        .or_else(|| diroxide_app::metadata::algorithm_named("sha256"))
        .ok_or_else(|| CliError::Failed("no algorithm".to_owned()))?;
    let mut code = 0;
    for typed in &args.args {
        let place = place_of(typed)?;
        let name = place.name().unwrap_or(typed).to_owned();
        match diroxide_app::metadata::sum_file(registry, &place, algorithm) {
            Ok(digest) if args.list => {
                write(out, &diroxide_app::metadata::sum_list_line(&name, &digest))?;
            }
            Ok(digest) => write(out, &diroxide_app::metadata::sum_line(&name, &digest))?,
            Err(why) => {
                write(
                    out,
                    &format!("{}: could not be read: {why}", display_name(&name)),
                )?;
                code = 1;
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_are_read_and_wrong_use_is_refused() {
        let args = parse(&strings(&["--algorithm", "BLAKE3", "a.zip"])).unwrap();
        assert_eq!(args.algorithm.as_deref(), Some("BLAKE3"));
        assert_eq!(args.args, ["a.zip"]);
        assert!(parse(&strings(&["--check", "SHA256SUMS"])).unwrap().check);
        assert!(matches!(parse(&[]), Err(CliError::Missing(_))));
        assert!(matches!(
            parse(&strings(&["--algorithm", "sha1", "a"])),
            Err(CliError::UnknownOption(_))
        ));
        assert!(matches!(
            parse(&strings(&["--check", "a", "b"])),
            Err(CliError::TooMany(_))
        ));
        // Wrong use exits 2, as sha256sum --check does.
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = crate::main_with(
            vec!["sum".into(), "--check".into()],
            &mut std::io::empty(),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn sums_are_lines_name_first_and_a_list_checks_with_its_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("example-tool.zip");
        std::fs::write(&file, b"abc").unwrap();
        let registry = Registry::with_local();
        let mut out = Vec::new();
        let args = SumArgs {
            args: vec![file.display().to_string()],
            ..SumArgs::default()
        };
        assert_eq!(run_with(&args, &registry, &mut out).unwrap(), 0);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("example-tool.zip, SHA-256, {ABC}\n")
        );
        let mut out = Vec::new();
        let listed = SumArgs {
            list: true,
            ..args.clone()
        };
        run_with(&listed, &registry, &mut out).unwrap();
        let list_text = String::from_utf8(out).unwrap();
        assert_eq!(list_text, format!("{ABC}  example-tool.zip\n"));
        // The list checks: 0 when all match.
        let sums = dir.path().join("SHA256SUMS");
        std::fs::write(&sums, &list_text).unwrap();
        let check = SumArgs {
            args: vec![sums.display().to_string()],
            check: true,
            ..SumArgs::default()
        };
        let mut out = Vec::new();
        assert_eq!(run_with(&check, &registry, &mut out).unwrap(), 0);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "1 file matches SHA256SUMS\n"
        );
        // 1 for a difference or a missing file, the verdict first.
        std::fs::write(
            &sums,
            format!("{}  example-tool.zip\n{ABC}  gone.txt\n", "0".repeat(64)),
        )
        .unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&check, &registry, &mut out).unwrap(), 1);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "1 differs, 1 missing, 0 match",
                "Does not match: example-tool.zip, SHA-256",
                "Missing: gone.txt"
            ]
        );
    }
}
