//! The names an arrival never is: files that are plainly unfinished, and
//! the scratch files programs leave beside real ones (R16 section 2.1; the
//! alpha.3 plan's list, with the editors' swap files).
//!
//! Careful producers write under a temporary name and rename the finished
//! file in, so leaving these names alone, and waiting for the real name to
//! appear, is most of what "fully arrived" takes.

use diroxide_vfs::names::is_unfinished;

/// Name starts that mark a scratch file: a leading dot (rsync's and
/// Syncthing's temporary copies on Unix, editors' lock and swap files,
/// LibreOffice's `.~lock.` files, diroxide's own `.diro-part-` files),
/// Office's lock files (`~$report.docx`) and Syncthing's temporary copies
/// on Windows (`~syncthing~report.pdf.tmp`).
pub const IGNORED_PREFIXES: [&str; 3] = [".", "~$", "~syncthing~"];

/// Name endings that mark an unfinished or scratch file, in any case:
/// downloads in progress (Firefox's `.part`, Chrome's `.crdownload`,
/// Safari's `.download`, the `.partial` and `.partial~` of other tools),
/// temporary files (`.tmp`, `.temp`), vim's swap files (`.swp`, `.swo`,
/// `.swx`) and editors' backup copies (a final `~`).
pub const IGNORED_ENDINGS: [&str; 11] = [
    ".part",
    ".partial",
    ".partial~",
    ".crdownload",
    ".download",
    ".tmp",
    ".temp",
    ".swp",
    ".swo",
    ".swx",
    "~",
];

/// Whole names, in any case, that programs write beside real files:
/// vim's write test (`4913`), and the folder settings Windows keeps
/// (`desktop.ini`, `Thumbs.db`).
pub const IGNORED_NAMES: [&str; 3] = ["4913", "desktop.ini", "thumbs.db"];

/// Whether a name is one the tracker never follows: an unfinished file, a
/// lock, swap or backup file, a sync client's temporary copy, or a
/// folder's settings. Emacs's auto-save files (`#notes.txt#`) count too.
pub fn is_ignored(name: &str) -> bool {
    if name.is_empty() || is_unfinished(name) {
        return true;
    }
    if IGNORED_PREFIXES.iter().any(|start| name.starts_with(start)) {
        return true;
    }
    let lower = name.to_lowercase();
    if IGNORED_ENDINGS.iter().any(|end| lower.ends_with(end)) {
        return true;
    }
    if IGNORED_NAMES.contains(&lower.as_str()) {
        return true;
    }
    name.len() > 2 && name.starts_with('#') && name.ends_with('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_producers_temporary_name_is_ignored() {
        for name in [
            ".diro-part-0badf00d",
            ".syncthing.report.pdf.tmp",
            "~syncthing~report.pdf.tmp",
            "report.pdf.part",
            "report.pdf.crdownload",
            "Report.PDF.CRDOWNLOAD",
            "report.pdf.download",
            "scan.partial~",
            "scan.partial",
            "copy.tmp",
            "copy.TEMP",
            "~$report.docx",
            ".~lock.report.odt#",
            ".report.txt.swp",
            "report.txt.swp",
            "report.txt.swo",
            "notes.txt~",
            "#notes.txt#",
            ".#notes.txt",
            "4913",
            "Thumbs.db",
            "desktop.ini",
            ".DS_Store",
            "",
        ] {
            assert!(is_ignored(name), "{name}");
        }
    }

    #[test]
    fn real_names_are_followed() {
        for name in [
            "report.pdf",
            "scan-004.pdf",
            "partition.img",
            "tmp-notes.txt",
            "a#b.txt",
            "#",
            "##",
            "notes.txt.bak",
            "4914",
            "export.temperature.csv",
        ] {
            assert!(!is_ignored(name), "{name}");
        }
    }
}
