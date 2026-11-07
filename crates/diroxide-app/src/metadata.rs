//! The metadata commands' requests, answers and words (alpha 4, lane MA,
//! R30 commits 54 to 56): Checksum, Verify checksum, Strip metadata into a
//! copy, Check before sharing and Remove the internet mark, each run on a
//! worker over `diroxide-meta`'s engines as a job in the jobs line that
//! Cancel stops.
//!
//! **The job pattern** (for the lanes that add commands over an engine of
//! their own): the app takes a job number, puts its line in the jobs
//! table ([`crate::JobTable::begin`]), and sends a [`MetaAsk`] carrying
//! it; the runtime runs the engine on a worker with a context whose cancel
//! token Cancel trips and whose progress becomes the job's progress line,
//! and answers with a [`MetaNote`] carrying the same number; the app ends
//! the job in the table ([`crate::JobTable::finished`]) and says the
//! result, the verdict first. Nothing here writes but through a provider,
//! and nothing is ever overwritten.

use diroxide_core::Place;
use diroxide_core::names::display_name;
use diroxide_meta::checksum::verify::VerifyReport;
use diroxide_meta::checksum::{Algorithm, Digest};

/// What the app asks the runtime to do with metadata.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MetaAsk {
    /// Compute the files' checksums in one read each (Checksum).
    Checksums {
        /// The job.
        job: diroxide_core::JobId,
        /// The files.
        places: Vec<Place>,
        /// The algorithms, all in one pass.
        algorithms: Vec<Algorithm>,
    },
    /// Write a checksum list into a folder under its name, or the first
    /// numbered name no file has: never over a file.
    SaveList {
        /// The folder.
        folder: Place,
        /// The list's name: `SHA256SUMS`.
        name: String,
        /// The list's text.
        text: String,
    },
    /// Verify checksum: a file against the list beside it, found in R29's
    /// order, or a list itself, every file it names read again.
    Verify {
        /// The job.
        job: diroxide_core::JobId,
        /// The file the owner chose: a file, or a list.
        place: Place,
        /// The list chosen when several were found.
        list: Option<String>,
    },
    /// Strip metadata into a copy: each picture's copy beside it,
    /// `NAME-clean.EXT` or the first numbered name free, never over a
    /// file; the originals stay.
    Strip {
        /// The job.
        job: diroxide_core::JobId,
        /// The pictures.
        places: Vec<Place>,
    },
    /// Check before sharing: what each file carries that could say who
    /// made it, where or with what; read only.
    Check {
        /// The files.
        places: Vec<Place>,
        /// The runner the tool-backed readers go through.
        tools: crate::request::SharedTools,
    },
    /// Read a file's internet mark, before Remove the internet mark asks.
    ReadMark {
        /// The file.
        place: Place,
    },
    /// Remove the internet mark, keeping what it held for Undo.
    RemoveMark {
        /// The job.
        job: diroxide_core::JobId,
        /// The file.
        place: Place,
    },
    /// Undo of Remove the internet mark: write back what it held.
    RestoreMark {
        /// The file.
        place: Place,
        /// What the removal took.
        removal: diroxide_meta::origin::MarkRemoval,
    },
}

/// What the runtime found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetaNote {
    /// The checksums of each file, or why it could not be read.
    Checksums {
        /// The job.
        job: diroxide_core::JobId,
        /// Each file and its values in the order asked.
        results: Vec<(Place, Result<Vec<Digest>, String>)>,
        /// Whether Cancel stopped it.
        cancelled: bool,
    },
    /// A list was saved, under the name it got; or why not.
    Saved {
        /// The folder.
        folder: Place,
        /// The name it was saved under, or the cause in words.
        result: Result<String, String>,
    },
    /// What Verify checksum found.
    Verified {
        /// The job.
        job: diroxide_core::JobId,
        /// The file the owner chose.
        place: Place,
        /// What it found, or why it could not look, in words.
        outcome: Result<Verified, String>,
    },
    /// What Strip made: each picture's copy and its report, or why not.
    Stripped {
        /// The job.
        job: diroxide_core::JobId,
        /// Each picture, and its copy's place and report, or the cause.
        results: Vec<(Place, StripResult)>,
        /// Whether Cancel stopped it.
        cancelled: bool,
    },
    /// What Check before sharing found, each file's report.
    Checked {
        /// The reports, in the files' order.
        reports: Vec<diroxide_meta::privacy::PrivacyReport>,
    },
    /// A file's internet mark as the origin row says it, `None` when it
    /// has none, or why it could not be read.
    Mark {
        /// The file.
        place: Place,
        /// "Origin: downloaded from example.org", or none.
        result: Result<Option<String>, String>,
    },
    /// Remove the internet mark ended: what it took, `None` when there
    /// was none, or why not.
    MarkRemoved {
        /// The job.
        job: diroxide_core::JobId,
        /// The file.
        place: Place,
        /// What it took.
        result: Result<Option<diroxide_meta::origin::MarkRemoval>, String>,
    },
    /// Undo of Remove the internet mark ended.
    MarkRestored {
        /// The file.
        place: Place,
        /// Done, or why not.
        result: Result<(), String>,
    },
}

/// One picture's strip: its copy's place and what the strip did, or the
/// cause in words.
pub type StripResult = Result<(Place, diroxide_meta::strip::StripReport), String>;

/// The extensions Strip takes: JPEG, PNG and WebP (R28 3.2; ExifTool's
/// fallback for other kinds is a later alpha's).
pub const STRIP_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

/// Whether Strip takes a file, by its name.
pub fn strips(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| STRIP_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The line after a strip of several pictures: "Stripped 3 photos into
/// copies", "Stripped 2 of 3 photos into copies".
pub fn stripped_line(done: usize, all: usize) -> String {
    if done == all {
        format!(
            "Stripped {} into copies",
            diroxide_core::format::count(all as u64, "photo", "photos")
        )
    } else {
        format!("Stripped {done} of {all} photos into copies")
    }
}

// ---- The command line's twins: `diro info` and `diro sum` (the owner's
// answer 121), plain lines, the name first, for scripts and the Braille
// display. The command line reaches the metadata crate through here.

/// The algorithm a `--algorithm` value names, any case, with or without
/// the dash: `sha256`, `SHA-512`, `blake3`, `md5`, `crc32`; SHA-1 is not
/// computed.
pub fn algorithm_named(text: &str) -> Option<Algorithm> {
    Algorithm::from_tag(text).filter(|a| a.computed())
}

/// One file's value for `diro sum`, read through its provider.
pub fn sum_file(
    registry: &diroxide_vfs::Registry,
    place: &Place,
    algorithm: Algorithm,
) -> Result<Digest, String> {
    let provider = registry.resolve(place).map_err(|e| e.to_string())?;
    let ctx = diroxide_vfs::OpContext::new();
    diroxide_meta::checksum::compute(provider.as_ref(), place, &[algorithm], &ctx)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "no value was computed".to_owned())
}

/// `diro sum`'s line for one file, its name first: "example-tool.zip,
/// SHA-256, ba7816bf...".
pub fn sum_line(file: &str, digest: &Digest) -> String {
    format!(
        "{}, {}, {}",
        display_name(file),
        digest.algorithm.name(),
        digest.hex()
    )
}

/// `diro sum --list`'s line for one file, the GNU form a list holds:
/// "ba7816bf...  example-tool.zip".
pub fn sum_list_line(file: &str, digest: &Digest) -> String {
    diroxide_meta::checksum::list::write_list(
        &[(file.to_owned(), digest.clone())],
        diroxide_meta::checksum::list::ListForm::Gnu,
    )
    .unwrap_or_default()
    .trim_end_matches('\n')
    .to_owned()
}

/// `diro sum --check LIST`: every file the list names, read again; the
/// report's lines (the verdict first, then the failures) and the exit
/// code `sha256sum --check` gives: 0 all match, 1 any difference,
/// missing or unreadable file or line passed over.
pub fn check_list(
    registry: &diroxide_vfs::Registry,
    list: &Place,
) -> Result<(Vec<String>, i32), String> {
    let provider = registry.resolve(list).map_err(|e| e.to_string())?;
    let ctx = diroxide_vfs::OpContext::new();
    let report = diroxide_meta::checksum::verify::verify_list(provider.as_ref(), list, &ctx)
        .map_err(|e| e.to_string())?;
    Ok((report.lines(), report.exit_code()))
}

/// `diro info`'s lines for one file: its name and the summary first
/// ("photo.png: PNG, 4 by 3 pixels, 2.3 KB"), then each detail, label
/// first, in the order of facts, as the details list has them; private
/// details included, since the owner asked for this file.
pub fn info_lines(
    registry: &diroxide_vfs::Registry,
    place: &Place,
    tools: &crate::request::SharedTools,
) -> Result<Vec<String>, String> {
    use diroxide_core::details::{DetailGroup, DetailRow};
    let provider = registry.resolve(place).map_err(|e| e.to_string())?;
    let ctx = diroxide_vfs::OpContext::new();
    let mut entry = provider.stat(place, &ctx).map_err(|e| e.to_string())?;
    let name = place.name().unwrap_or_default().to_owned();
    if entry.name.is_empty() {
        entry.name = name.clone();
    }
    let types = diroxide_filetype::TypeRegistry::builtin();
    let head = if matches!(
        entry.kind,
        diroxide_core::EntryKind::File | diroxide_core::EntryKind::Archive
    ) {
        let mut head = Vec::new();
        if let Ok(reader) = provider.read(place, &ctx) {
            let _ = std::io::Read::read_to_end(
                &mut std::io::Read::take(reader, diroxide_filetype::HEAD_BYTES as u64),
                &mut head,
            );
        }
        head
    } else {
        Vec::new()
    };
    let file_type = types
        .detect(&entry.name, &head)
        .and_then(|d| types.get(&d.type_id))
        .cloned();
    let ask = crate::details::DetailsAsk {
        generation: 0,
        place: place.clone(),
        entry: entry.clone(),
        file_type: file_type.clone(),
        tools: tools.clone(),
        cache: None,
        wanted: crate::details::Wanted::Fields,
    };
    let mut rows = diroxide_preview::detail_rows(&entry, file_type.as_ref());
    rows.push(DetailRow::new(
        "name",
        "Name",
        entry.display_name().into_owned(),
        DetailGroup::Identity,
    ));
    let offset = crate::startup::local_offset();
    for (key, label, time) in [
        ("time.modified", "Modified", entry.modified),
        ("time.created", "Created", entry.created),
        ("time.accessed", "Accessed", entry.accessed),
    ] {
        if let Some(time) = time {
            let civil = time.to_civil(offset);
            rows.push(DetailRow::new(
                key,
                label,
                format!("{}, {}", civil.long_date(), civil.clock_time()),
                DetailGroup::Dates,
            ));
        }
    }
    rows.push(DetailRow::new(
        "path",
        "Path",
        diroxide_core::names::spell_out(&place.to_string()),
        DetailGroup::Place,
    ));
    let mut summary = diroxide_preview::details(&entry, file_type.as_ref());
    if matches!(
        entry.kind,
        diroxide_core::EntryKind::File | diroxide_core::EntryKind::Archive
    ) {
        let (read, unread) = crate::details::read_fields(registry, &ask);
        rows.extend(read);
        for (i, why) in unread.iter().enumerate() {
            rows.push(crate::details::unread_row(i, why));
        }
        summary = crate::details::summary(registry, &ask);
    }
    let mut lines = vec![format!("{}: {summary}", display_name(&name))];
    lines.extend(
        crate::details::ordered(rows)
            .iter()
            .map(crate::details::row_text),
    );
    Ok(lines)
}

/// The line after Undo put the internet mark back: "Put back the
/// internet mark on setup.exe".
pub fn restored_words(name: &str) -> String {
    format!("Put back the internet mark on {}", display_name(name))
}

/// What Verify checksum found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verified {
    /// The list's report, verdict first, and a signature file found
    /// beside the list, not checked here.
    Report {
        /// The report.
        report: Box<VerifyReport>,
        /// A signature file beside the list (`SHA256SUMS.asc`), when one
        /// is there.
        signature: Option<String>,
    },
    /// Several lists sit beside the file: each one's name and its row,
    /// "SHA256SUMS, 12 files", for the owner to choose.
    Choose(Vec<(String, String)>),
    /// No list sits beside the file.
    NoList,
    /// The list found names no line for the file: its name.
    NoLine(String),
    /// Cancel stopped it.
    Cancelled,
}

/// Whether a name is a checksum list's: `SHA256SUMS`, `files.md5`,
/// `report.pdf.sha256`, `checksums.sfv`.
pub fn is_list_name(name: &str) -> bool {
    Algorithm::from_list_name(name).is_some()
}

/// The signature files a list may have beside it, in the order looked
/// for: `.asc`, `.sig`, `.minisig` (lane E checks them).
pub fn signature_names(list: &str) -> [String; 3] {
    ["asc", "sig", "minisig"].map(|ext| format!("{list}.{ext}"))
}

/// What Verify checksum's list says, the verdict first (R29 3.5; R30
/// commit 55): the verdict, the failures, "Matches BLAKE3, not SHA-256"
/// where a value named no algorithm, then the one line on the signature
/// and what a match shows. Never "safe" or "genuine".
pub fn verify_lines(report: &VerifyReport, signature: Option<&str>) -> Vec<String> {
    let mut lines = report.lines();
    for entry in &report.entries {
        if let diroxide_meta::checksum::verify::EntryVerdict::Match {
            algorithm,
            other: Some(other),
        } = &entry.verdict
        {
            lines.push(format!(
                "Matches {}, not {}",
                algorithm.name(),
                other.name()
            ));
        }
    }
    lines.push(match signature {
        // Lane E's commit 73 checks it; until then it is named, unchecked.
        Some(file) => format!("Signature not checked: {}", display_name(file)),
        None => "List not signed.".to_owned(),
    });
    if report.counts().matched > 0 {
        lines.push(diroxide_meta::checksum::NOT_PROOF.to_owned());
    }
    lines
}

/// The line when no list sits beside a file: "No checksum list beside
/// setup.exe".
pub fn no_list_line(file: &str) -> String {
    format!("No checksum list beside {}", display_name(file))
}

/// The line when the list names no line for the file: "SHA256SUMS has no
/// line for setup.exe".
pub fn no_line_line(list: &str, file: &str) -> String {
    format!(
        "{} has no line for {}",
        display_name(list),
        display_name(file)
    )
}

/// The line said when a checksum is computed, meaning first: "SHA-256 of
/// example-tool.zip, ab12 cd34" (R30's 38 cells), the value's first two
/// groups; the whole value is in the list below it.
pub fn computed_line(digest: &Digest, file: &str) -> String {
    let groups = digest.groups();
    let start = groups.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    format!(
        "{} of {}, {start}",
        digest.algorithm.name(),
        display_name(file)
    )
}

/// The algorithm a pasted value's length names when it is not the
/// computed one's: 128 digits SHA-512, 32 MD5, 8 CRC-32. 64 digits are
/// SHA-256 or BLAKE3, both computed at once.
pub fn algorithm_for_pasted(digits: usize) -> Option<Algorithm> {
    match digits {
        128 => Some(Algorithm::Sha512),
        32 => Some(Algorithm::Md5),
        8 => Some(Algorithm::Crc32),
        _ => None,
    }
}

/// A pasted value's digits: spaces, dashes and colons out.
pub fn pasted_digits(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | ':'))
        .collect()
}

/// The verdict of a pasted value against what was computed, first, and
/// the line after it: "Match: example-tool.zip, SHA-256" (32), or "Does
/// not match: setup.exe, SHA-256" then "The first difference is in group
/// 6"; a 64-digit value that matches BLAKE3 says "Matches BLAKE3, not
/// SHA-256". `None` when the value's length names an algorithm not
/// computed yet ([`algorithm_for_pasted`]).
pub fn compare_lines(digests: &[Digest], file: &str, pasted: &str) -> Option<Vec<String>> {
    let digits = pasted_digits(pasted).len();
    let main = digests.first()?;
    if digits != main.algorithm.hex_len()
        && let Some(other) = digests.iter().find(|d| d.algorithm.hex_len() == digits)
    {
        let comparison = other.compare(pasted);
        return Some(verdict_lines(&comparison, file, other.algorithm));
    }
    if digits != main.algorithm.hex_len() && algorithm_for_pasted(digits).is_some() {
        return None;
    }
    let comparison = main.compare(pasted);
    if comparison != diroxide_meta::checksum::Comparison::Match
        && digits == main.algorithm.hex_len()
        && let Some(line) = diroxide_meta::checksum::which_matches(digests, pasted)
    {
        let matched = digests
            .iter()
            .find(|d| d.compare(pasted) == diroxide_meta::checksum::Comparison::Match)
            .map_or(main.algorithm, |d| d.algorithm);
        return Some(vec![
            format!("Match: {}, {}", display_name(file), matched.name()),
            line,
        ]);
    }
    Some(verdict_lines(&comparison, file, main.algorithm))
}

fn verdict_lines(
    comparison: &diroxide_meta::checksum::Comparison,
    file: &str,
    algorithm: Algorithm,
) -> Vec<String> {
    let mut lines = vec![comparison.verdict(file, algorithm)];
    lines.extend(comparison.detail());
    if let Some(caution) = algorithm.caution()
        && *comparison == diroxide_meta::checksum::Comparison::Match
    {
        lines.push(format!("{} is {caution}.", algorithm.name()));
    }
    lines
}

/// The line when a list is saved: "Saved SHA256SUMS in work".
pub fn saved_line(name: &str, folder: &Place) -> String {
    format!(
        "Saved {} in {}",
        display_name(name),
        crate::panel::place_leaf(folder)
    )
}

#[cfg(test)]
mod tests {
    use diroxide_core::format::within_cells;
    use diroxide_meta::checksum::compute_reader;
    use diroxide_vfs::CancelToken;

    use super::*;

    fn digests(bytes: &[u8]) -> Vec<Digest> {
        compute_reader(
            &mut std::io::Cursor::new(bytes.to_vec()),
            &[Algorithm::Sha256, Algorithm::Blake3],
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap()
    }

    #[test]
    fn the_computed_line_is_the_value_s_start() {
        let d = digests(b"abc");
        let line = computed_line(&d[0], "example-tool.zip");
        assert_eq!(line, "SHA-256 of example-tool.zip, ba78 16bf");
        assert_eq!(line.chars().count(), 38);
        assert!(within_cells(&line, &line));
    }

    #[test]
    fn a_pasted_value_is_judged_verdict_first() {
        let d = digests(b"abc");
        let sha = d[0].hex();
        assert_eq!(
            compare_lines(&d, "example-tool.zip", &sha.to_uppercase()).unwrap(),
            ["Match: example-tool.zip, SHA-256"]
        );
        let mut wrong = sha.clone();
        wrong.replace_range(21..22, if &sha[21..22] == "0" { "1" } else { "0" });
        assert_eq!(
            compare_lines(&d, "setup.exe", &wrong).unwrap(),
            [
                "Does not match: setup.exe, SHA-256",
                "The first difference is in group 6"
            ]
        );
        // 64 digits that are BLAKE3's.
        let lines = compare_lines(&d, "a.txt", &d[1].hex()).unwrap();
        assert_eq!(
            lines,
            ["Match: a.txt, BLAKE3", "Matches BLAKE3, not SHA-256"]
        );
        // 128 digits: SHA-512, computed on request.
        assert_eq!(compare_lines(&d, "a.txt", &"a".repeat(128)), None);
        assert_eq!(algorithm_for_pasted(128), Some(Algorithm::Sha512));
        let odd = compare_lines(&d, "a.txt", &"a".repeat(63)).unwrap();
        assert_eq!(odd, ["Not a SHA-256 value: 63 digits, needs 64"]);
        for line in lines.iter().chain(&odd) {
            assert!(within_cells(line, line), "{line}");
        }
    }
}
