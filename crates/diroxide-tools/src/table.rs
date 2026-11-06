//! The table of tools diroxide knows (alpha.7, R28 1.3 and 1.4, R29 2.2
//! and 8): each tool's id, its name as said, the program names on this
//! system, its version command, what its version line must say, the
//! folders its installers use, and its words for Settings.
//!
//! The version commands and their streams are spike T3's (Friday,
//! November 6, 2026): run in the development image's tools stage and on
//! the owner's Windows computer; the golden lines are in `version.rs`.

use crate::lookup::{Base, InstallFolder};
use crate::{ToolKind, ToolSpec};

/// carta's version, the converter compiled into diro: the root manifest's
/// pin (`carta = { version = "=0.0.13", ... }`); a test keeps the two in
/// step.
pub const CARTA_VERSION: &str = "0.0.13";

const PROGRAM_FILES_7ZIP: InstallFolder =
    InstallFolder::new(Base::ProgramFiles, "7-Zip", "Program Files");
const PROGRAM_FILES_X86_7ZIP: InstallFolder =
    InstallFolder::new(Base::ProgramFilesX86, "7-Zip", "Program Files");
const GIT_FOR_WINDOWS: InstallFolder =
    InstallFolder::new(Base::ProgramFiles, "Git/cmd", "Git for Windows");
const GIT_FOR_WINDOWS_TOOLS: InstallFolder =
    InstallFolder::new(Base::ProgramFiles, "Git/usr/bin", "Git for Windows");
const GPG4WIN: InstallFolder = InstallFolder::new(Base::ProgramFiles, "GnuPG/bin", "Gpg4win");
const GPG4WIN_X86: InstallFolder =
    InstallFolder::new(Base::ProgramFilesX86, "GnuPG/bin", "Gpg4win");

/// The folders each tool's installers use on this system.
const fn folders(windows: &'static [InstallFolder]) -> &'static [InstallFolder] {
    if cfg!(windows) { windows } else { &[] }
}

/// The program names on this system: `windows` there, `elsewhere` on the
/// others.
const fn programs(
    windows: &'static [&'static str],
    elsewhere: &'static [&'static str],
) -> &'static [&'static str] {
    if cfg!(windows) { windows } else { elsewhere }
}

/// The tools diroxide knows about, in the order Settings and `diro
/// doctor` list them. A slice, so a later alpha adds a tool without
/// breaking callers.
pub const BUILTIN: &[ToolSpec] = &[
    ToolSpec {
        id: "magick",
        name: "ImageMagick",
        // On Windows `convert.exe` in System32 is a disk tool, so only
        // `magick` is looked for there; elsewhere ImageMagick 6's
        // `convert` counts when its version line says ImageMagick.
        programs: programs(&["magick"], &["magick", "convert"]),
        version_args: &["-version"],
        must_say: Some("ImageMagick"),
        install_folders: &[],
        purpose: "image details and conversions",
        install_hint: "Install ImageMagick from imagemagick.org or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "ffmpeg",
        name: "FFmpeg",
        programs: &["ffmpeg"],
        version_args: &["-version"],
        must_say: None,
        install_folders: &[],
        purpose: "audio and video conversions",
        install_hint: "Install FFmpeg from ffmpeg.org or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "ffprobe",
        name: "FFprobe",
        programs: &["ffprobe"],
        version_args: &["-version"],
        must_say: None,
        install_folders: &[],
        purpose: "audio and video details",
        install_hint: "FFprobe comes with FFmpeg.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "pandoc",
        name: "pandoc",
        programs: &["pandoc"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[InstallFolder::new(
            Base::ProgramFiles,
            "Pandoc",
            "Program Files",
        )]),
        purpose: "document previews and conversions",
        install_hint: "Install pandoc from pandoc.org or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "7z",
        name: "7-Zip",
        // The full programs first: 7za and 7zr read fewer formats (no
        // RAR).
        programs: &["7z", "7zz", "7za", "7zr"],
        // `i` lists the formats and reads no file; the banner before the
        // list is the version (spike T3: 7z, 7za, 7zr and 7-Zip for
        // Windows alike).
        version_args: &["i"],
        must_say: None,
        install_folders: folders(&[PROGRAM_FILES_7ZIP, PROGRAM_FILES_X86_7ZIP]),
        purpose: "reading RAR, CAB and other archives, and cross-checking",
        install_hint: "Install 7-Zip from 7-zip.org, or 7zip from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "pdftotext",
        name: "pdftotext",
        programs: &["pdftotext"],
        // Poppler and Xpdf write the version on standard error.
        version_args: &["-v"],
        must_say: None,
        install_folders: &[],
        purpose: "PDF text previews",
        install_hint: "Install Poppler's utilities from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "pdfinfo",
        name: "pdfinfo",
        programs: &["pdfinfo"],
        version_args: &["-v"],
        must_say: None,
        install_folders: &[],
        purpose: "PDF details",
        install_hint: "Install Poppler's utilities from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "exiftool",
        name: "ExifTool",
        programs: &["exiftool"],
        version_args: &["-ver"],
        must_say: None,
        install_folders: &[],
        purpose: "photo, audio and document details",
        install_hint: "Install ExifTool from exiftool.org or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "tesseract",
        name: "Tesseract",
        programs: &["tesseract"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[
            InstallFolder::new(Base::ProgramFiles, "Tesseract-OCR", "Program Files"),
            InstallFolder::new(Base::LocalPrograms, "Tesseract-OCR", "your programs folder"),
        ]),
        purpose: "reading text in pictures",
        install_hint: "Install Tesseract from its project page or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "qemu-img",
        name: "qemu-img",
        programs: &["qemu-img"],
        version_args: &["--version"],
        must_say: None,
        install_folders: &[],
        purpose: "converting disk images",
        install_hint: "Install QEMU's qemu-img from qemu.org or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "xorriso",
        name: "xorriso",
        programs: &["xorriso"],
        version_args: &["-version"],
        must_say: None,
        install_folders: &[],
        purpose: "making ISO images",
        install_hint: "Install xorriso from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "gpg",
        name: "GnuPG",
        programs: &["gpg", "gpg2"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[GPG4WIN, GPG4WIN_X86, GIT_FOR_WINDOWS_TOOLS]),
        purpose: "OpenPGP encryption and signatures",
        install_hint: "Install GnuPG from gnupg.org, Gpg4win on Windows, or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "gpgv",
        name: "gpgv",
        programs: &["gpgv"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[GPG4WIN, GPG4WIN_X86, GIT_FOR_WINDOWS_TOOLS]),
        purpose: "checking OpenPGP signatures",
        install_hint: "gpgv comes with GnuPG.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "age",
        name: "age",
        programs: &["age"],
        version_args: &["--version"],
        must_say: None,
        install_folders: &[],
        purpose: "age encryption",
        install_hint: "Install age from its releases page or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "minisign",
        name: "minisign",
        programs: &["minisign"],
        version_args: &["-v"],
        must_say: None,
        install_folders: &[],
        purpose: "checking minisign signatures",
        install_hint: "Install minisign from its releases page or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "git",
        name: "git",
        programs: &["git"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[GIT_FOR_WINDOWS]),
        purpose: "git status and commands",
        install_hint: "Install git from git-scm.com or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "gh",
        name: "GitHub CLI",
        programs: &["gh"],
        version_args: &["--version"],
        must_say: None,
        install_folders: folders(&[InstallFolder::new(
            Base::ProgramFiles,
            "GitHub CLI",
            "Program Files",
        )]),
        purpose: "GitHub from git places, and private component sources",
        install_hint: "Install the GitHub CLI from cli.github.com or your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "open",
        name: "the system opener",
        programs: if cfg!(windows) {
            &["explorer"]
        } else if cfg!(target_os = "macos") {
            &["open"]
        } else {
            &["xdg-open"]
        },
        // Explorer and macOS's open have no version command.
        version_args: if cfg!(any(windows, target_os = "macos")) {
            &[]
        } else {
            &["--version"]
        },
        must_say: None,
        install_folders: &[],
        purpose: "opening files in their default programs",
        install_hint: "Install xdg-utils from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "tw",
        name: "textweaver",
        programs: &["tw"],
        version_args: &["--version"],
        must_say: None,
        install_folders: &[],
        purpose: "reading a document aloud, from the quick view",
        install_hint: "Install textweaver from its releases page or with cargo.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "curl",
        name: "curl",
        programs: &["curl"],
        version_args: &["--version"],
        must_say: None,
        install_folders: &[],
        purpose: "downloading components when the built-in client is off",
        install_hint: "curl comes with Windows 10 and later; install it from your package manager.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "wget",
        name: "wget",
        programs: &["wget"],
        version_args: &["--version"],
        must_say: None,
        install_folders: &[],
        purpose: "downloading components when curl is missing",
        install_hint: "Install wget from your package manager.",
        kind: ToolKind::Program,
    },
    #[cfg(target_os = "macos")]
    ToolSpec {
        id: "mdls",
        name: "mdls",
        programs: &["mdls"],
        version_args: &[],
        must_say: None,
        install_folders: &[],
        purpose: "Spotlight's details of a file",
        install_hint: "mdls comes with macOS.",
        kind: ToolKind::Program,
    },
    ToolSpec {
        id: "carta",
        name: "carta",
        programs: &[],
        version_args: &[],
        must_say: None,
        install_folders: &[],
        purpose: "document conversions in diro itself",
        install_hint: "carta is built into diro.",
        kind: ToolKind::BuiltIn,
    },
];
