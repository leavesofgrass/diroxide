//! The registry's lookup with fake tools (alpha.7, R30 commit 17): the
//! order of the owner's answer 124 said in words, ImageMagick's
//! `must_say`, the version read from standard output then standard
//! error, and every line at 40 cells with the tool's name first.
//!
//! A fake tool is a small script that prints what the real one prints
//! (spike T3's lines): a `.cmd` file on Windows, an `sh` script elsewhere.

use std::fs;
use std::path::{Path, PathBuf};

use diroxide_config::{ToolOverride, ToolsConfig};
use diroxide_tools::{Bases, ToolRegistry, ToolStatus};

/// Writes a fake tool `name` into `folder` that prints `out` on standard
/// output and `err` on standard error, and exits with `code`. Returns its
/// path.
fn fake(folder: &Path, name: &str, out: &[&str], err: &[&str], code: i32) -> PathBuf {
    fs::create_dir_all(folder).unwrap();
    if cfg!(windows) {
        let path = folder.join(format!("{name}.cmd"));
        let mut text = String::from("@echo off\r\n");
        // `echo.` prints an empty line; a bare `echo` would say "ECHO is
        // off."
        for line in out {
            text.push_str(&format!("echo.{line}\r\n"));
        }
        for line in err {
            text.push_str(&format!("echo.{line} 1>&2\r\n"));
        }
        text.push_str(&format!(
            "echo ran> \"%~dp0{name}-ran.txt\"\r\nexit /b {code}\r\n"
        ));
        fs::write(&path, text).unwrap();
        path
    } else {
        let path = folder.join(name);
        let mut text = String::from("#!/bin/sh\n");
        for line in out {
            text.push_str(&format!("printf '%s\\n' '{line}'\n"));
        }
        for line in err {
            text.push_str(&format!("printf '%s\\n' '{line}' >&2\n"));
        }
        text.push_str(&format!(
            "echo ran > \"$(dirname \"$0\")/{name}-ran.txt\"\nexit {code}\n"
        ));
        fs::write(&path, text).unwrap();
        make_executable(&path);
        path
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// A registry that looks only in `path` and in no install folder. Its
/// fake tools' version commands may run for a minute: they are scripts,
/// and on a machine loaded with other builds a `.cmd` once took 2.8
/// seconds to start, past the product's two (E's merge check, Saturday,
/// November 7, 2026). A passing run waits only for the script's end.
fn on_path(path: &Path) -> ToolRegistry {
    ToolRegistry::builtin()
        .with_path(Some(path.as_os_str().to_owned()))
        .with_version_timeout(std::time::Duration::from_secs(60))
}

/// The report's line for one tool, and the line after it.
fn line_of(tools: &ToolRegistry, name: &str) -> (String, Option<String>) {
    let lines = tools.report().lines();
    let at = lines
        .iter()
        .position(|l| l.starts_with(&format!("{name}:")))
        .unwrap_or_else(|| panic!("no line for {name}: {lines:?}"));
    (lines[at].clone(), lines.get(at + 1).cloned())
}

fn path_override(path: &Path) -> ToolsConfig {
    let mut config = ToolsConfig::default();
    config.tools.insert(
        "pandoc".to_owned(),
        ToolOverride {
            path: Some(path.to_string_lossy().into_owned()),
            disabled: false,
            needed: false,
        },
    );
    config
}

/// Answer 124's order: tools.toml, then a component, then PATH, then the
/// install folders; each found pandoc says where, in words, within 40
/// cells.
#[test]
fn the_lookup_order_is_said_in_words() {
    let root = tempfile::tempdir().unwrap();
    let own = fake(&root.path().join("own"), "pandoc", &["pandoc 3.13"], &[], 0);
    let component = fake(
        &root.path().join("component"),
        "pandoc",
        &["pandoc 3.12.1"],
        &[],
        0,
    );
    let path = root.path().join("path");
    fake(&path, "pandoc", &["pandoc 3.9.0.2"], &[], 0);
    let base = root.path().join("program-files");
    fake(&base.join("Pandoc"), "pandoc", &["pandoc 3.1.11.1"], &[], 0);
    let bases = Bases {
        program_files: Some(base.clone()),
        ..Bases::none()
    };

    let all = on_path(&path)
        .with_bases(bases.clone())
        .with_components(vec![component.clone()])
        .with_overrides(path_override(&own));
    let (line, _) = line_of(&all, "pandoc");
    assert_eq!(line, "pandoc: 3.13, from tools.toml");

    let no_own = on_path(&path)
        .with_bases(bases.clone())
        .with_components(vec![component]);
    assert_eq!(
        line_of(&no_own, "pandoc").0,
        "pandoc: 3.12.1, from components"
    );

    let no_component = on_path(&path).with_bases(bases.clone());
    assert_eq!(
        line_of(&no_component, "pandoc").0,
        "pandoc: 3.9.0.2, on PATH"
    );

    let empty = root.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let folders_only = on_path(&empty).with_bases(bases);
    if cfg!(windows) {
        let (line, _) = line_of(&folders_only, "pandoc");
        assert_eq!(line, "pandoc: 3.1.11.1, in Program Files");
        assert!(line.chars().count() <= 40, "{line}");
    } else {
        assert_eq!(line_of(&folders_only, "pandoc").0, "pandoc: not found");
    }

    // A tools.toml path with nothing at it is said, and nothing else is
    // looked in.
    let missing = on_path(&path).with_overrides(path_override(&root.path().join("gone")));
    let (line, next) = line_of(&missing, "pandoc");
    assert_eq!(line, "pandoc: not at its tools.toml path");
    assert_eq!(next.as_deref(), Some("Check its path in tools.toml."));
    assert_eq!(missing.find("pandoc"), Ok(ToolStatus::NotFound));
}

/// The runner says a tool's version as the Tools page does (CL's request
/// 4): read from the found program, carta's built in, and none for a tool
/// that is not found.
#[test]
fn the_runner_says_a_tools_version() {
    use diroxide_vfs::ToolRunner;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(&path, "pandoc", &["pandoc 3.9.0.2"], &[], 0);
    let tools = on_path(&path);
    assert_eq!(
        ToolRunner::version(&tools, "pandoc").as_deref(),
        Some("3.9.0.2")
    );
    assert_eq!(
        ToolRunner::version(&tools, "carta").as_deref(),
        Some(diroxide_tools::CARTA_VERSION)
    );
    assert_eq!(ToolRunner::version(&tools, "magick"), None);
    // A runner that reads no versions says none, and runs nothing.
    assert_eq!(diroxide_vfs::NoTools.version("pandoc"), None);
}

/// ImageMagick has to say "ImageMagick": a `convert` that does not (as
/// System32's disk tool would not) is passed over, and on Windows
/// `convert` is never looked for or run at all.
#[test]
fn must_say_rejects_a_convert_that_is_not_imagemagick() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(&path, "convert", &["Invalid Parameter - -version"], &[], 4);
    let tools = on_path(&path);
    assert_eq!(tools.find("magick"), Ok(ToolStatus::NotFound));
    assert_eq!(line_of(&tools, "ImageMagick").0, "ImageMagick: not found");
    let ran = path.join("convert-ran.txt").exists();
    if cfg!(windows) {
        assert!(!ran, "convert was run on Windows");
        assert_eq!(
            ToolRegistry::builtin().spec("magick").unwrap().programs,
            ["magick"]
        );
    } else {
        assert!(ran, "the version line was read to check it");
    }

    // The real one says so, and its version is read.
    fake(
        &path,
        "magick",
        &["Version: ImageMagick 7.1.2-35 Q16-HDRI x64 https://imagemagick.org"],
        &[],
        0,
    );
    let tools = on_path(&path);
    let (line, _) = line_of(&tools, "ImageMagick");
    assert_eq!(line, "ImageMagick: 7.1.2-35, on PATH");
    assert!(line.chars().count() <= 40);
    assert!(matches!(tools.find("magick"), Ok(ToolStatus::Found(_))));
}

/// Elsewhere ImageMagick 6's `convert` counts when it says ImageMagick.
#[cfg(not(windows))]
#[test]
fn imagemagick_sixs_convert_counts_where_it_says_so() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(
        &path,
        "convert",
        &["Version: ImageMagick 6.9.11-60 Q16 x86_64 2021-01-25"],
        &[],
        0,
    );
    assert_eq!(
        line_of(&on_path(&path), "ImageMagick").0,
        "ImageMagick: 6.9.11-60, on PATH"
    );
}

/// Poppler writes its version on standard error, Xpdf exits with 99, and
/// 7-Zip's banner starts with an empty line: each is read (spike T3).
#[test]
fn the_version_comes_from_standard_error_when_standard_output_is_empty() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(
        &path,
        "pdfinfo",
        &[],
        &[
            "pdfinfo version 26.09.0",
            "Copyright 2005-2026 The Poppler Developers",
        ],
        0,
    );
    fake(&path, "pdftotext", &[], &["pdftotext version 4.00"], 99);
    fake(
        &path,
        "7z",
        &[
            "",
            "7-Zip 26.05 (x64) : Copyright (c) 1999-2026 Igor Pavlov",
        ],
        &[],
        0,
    );
    let tools = on_path(&path);
    let lines = tools.report().lines();
    for wanted in [
        "pdfinfo: 26.09.0, on PATH",
        "pdftotext: 4.00, on PATH",
        "7-Zip: 26.05, on PATH",
    ] {
        assert!(lines.iter().any(|l| l == wanted), "{wanted}: {lines:?}");
        assert!(wanted.chars().count() <= 40);
    }
    assert!(lines[0].starts_with("Tools: 4 of "), "{}", lines[0]);
}

/// A program whose version command says nothing is found, and its line
/// says the version was not read.
#[test]
fn a_silent_version_command_is_said_in_words() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(&path, "exiftool", &[], &[], 0);
    assert_eq!(
        line_of(&on_path(&path), "ExifTool").0,
        "ExifTool: version not read, on PATH"
    );
}

/// On Windows the extensions of PATHEXT come before the bare name: a bare
/// `pandoc` beside `pandoc.cmd` is not the one found.
#[cfg(windows)]
#[test]
fn pathext_comes_before_the_bare_name() {
    use diroxide_tools::Location;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("path");
    fake(&path, "pandoc", &["pandoc 3.13"], &[], 0);
    fs::write(path.join("pandoc"), b"#!/bin/sh\n").unwrap();
    match on_path(&path).locate("pandoc") {
        Ok(Location::Found { path: found, .. }) => {
            let name = found.file_name().unwrap().to_string_lossy().to_lowercase();
            assert_eq!(name, "pandoc.cmd");
        }
        other => panic!("{other:?}"),
    }
}

/// The install folders find 7-Zip where its installer put it, off PATH
/// (Windows), or Homebrew's folder (macOS); Linux has none.
#[test]
fn an_install_folder_finds_a_program_off_path() {
    let root = tempfile::tempdir().unwrap();
    let empty = root.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let base = root.path().join("base");
    let (bases, folder, words) = if cfg!(windows) {
        (
            Bases {
                program_files: Some(base.clone()),
                ..Bases::none()
            },
            base.join("7-Zip"),
            "in Program Files",
        )
    } else {
        (
            Bases {
                homebrew: vec![base.clone()],
                ..Bases::none()
            },
            base.clone(),
            "in Homebrew",
        )
    };
    fake(
        &folder,
        "7z",
        &[
            "",
            "7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov",
        ],
        &[],
        0,
    );
    let tools = on_path(&empty).with_bases(bases);
    let (line, _) = line_of(&tools, "7-Zip");
    if cfg!(any(windows, target_os = "macos")) {
        assert_eq!(line, format!("7-Zip: 24.08, {words}"));
        assert!(line.chars().count() <= 40);
    } else {
        assert_eq!(line, "7-Zip: not found");
    }
}
