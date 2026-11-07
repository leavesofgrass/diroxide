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
