//! Verify from a checksum list, the verdict first (R29 3.5; R28 3.2;
//! R30 commit 55's words, which the app's lane says): one line that sums
//! it up, then the failures, then the matches only when asked.
//!
//! - "All 12 files match SHA256SUMS"
//! - "2 of 12 files do not match", then "Does not match: setup.exe,
//!   SHA-256" for each
//! - "1 file named in the list is missing", then "Missing: notes.txt"
//! - "1 differs, 1 missing, 1 match" when they mix
//! - a line no form reads: "A line cannot be read: line 4"; a name that
//!   leaves the folder: "Skipped a name that leaves the folder: ../x"
//!
//! A list that names no algorithm and holds 64-digit values is tried as
//! SHA-256 and BLAKE3 in one read, and says which matched: "Match:
//! a.txt, BLAKE3, not SHA-256". A verify always reads the file again; no
//! stored value stands in (R12). Nothing here calls a file safe: a match
//! shows the file is not damaged ([`super::NOT_PROOF`]).

use std::io::Read;

use diroxide_core::Place;
use diroxide_core::names::display_name;
use diroxide_vfs::{OpContext, ProgressUpdate, Provider, VfsError};

use super::list::{ChecksumList, ListProblem, MAX_LIST_BYTES, parse_list};
use super::{Algorithm, compute};
use crate::reader::MetaError;

/// What one file's check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryVerdict {
    /// It matches, by this algorithm; `other` is the one it was also
    /// tried as and did not match, when the list named none.
    Match {
        /// The algorithm it matched by.
        algorithm: Algorithm,
        /// The other it was tried as.
        other: Option<Algorithm>,
    },
    /// It does not match by any algorithm it was tried as.
    Differs {
        /// The algorithm the list named or the first tried.
        algorithm: Algorithm,
    },
    /// No file has the name.
    Missing,
    /// It could not be read, with why.
    Unreadable(String),
    /// Its algorithm is not computed here (SHA-1).
    NotChecked(Algorithm),
}

/// One file of a list and what its check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The name as the list gives it, `/` between folders.
    pub name: String,
    /// What the check found.
    pub verdict: EntryVerdict,
}

/// The counts of a report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Files that match.
    pub matched: usize,
    /// Files that do not.
    pub differ: usize,
    /// Files named and not there.
    pub missing: usize,
    /// Files that could not be read.
    pub unreadable: usize,
    /// Files whose algorithm is not computed here.
    pub not_checked: usize,
    /// Lines passed over.
    pub skipped: usize,
}

impl Counts {
    /// The files the list named that were checked or looked for.
    pub fn files(&self) -> usize {
        self.matched + self.differ + self.missing + self.unreadable + self.not_checked
    }
}

/// A list's check: every file and every line passed over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// The list's own name: "SHA256SUMS".
    pub list_name: String,
    /// The files, in the list's order.
    pub entries: Vec<Verified>,
    /// The lines passed over.
    pub problems: Vec<ListProblem>,
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

impl VerifyReport {
    /// The counts.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            skipped: self.problems.len(),
            ..Counts::default()
        };
        for entry in &self.entries {
            match entry.verdict {
                EntryVerdict::Match { .. } => counts.matched += 1,
                EntryVerdict::Differs { .. } => counts.differ += 1,
                EntryVerdict::Missing => counts.missing += 1,
                EntryVerdict::Unreadable(_) => counts.unreadable += 1,
                EntryVerdict::NotChecked(_) => counts.not_checked += 1,
            }
        }
        counts
    }

    /// Whether every file named matches and no line was passed over.
    pub fn all_match(&self) -> bool {
        let c = self.counts();
        c.matched > 0 && c.matched == c.files() && c.skipped == 0
    }

    /// The verdict, one line, first.
    pub fn verdict(&self) -> String {
        let c = self.counts();
        let list = display_name(&self.list_name);
        let files = c.files();
        if files == 0 {
            return match c.skipped {
                0 => format!("No checksums in {list}"),
                n => format!(
                    "No checksums read: {n} {} skipped",
                    plural(n, "line", "lines")
                ),
            };
        }
        if self.all_match() {
            return if files == 1 {
                format!("1 file matches {list}")
            } else {
                format!("All {files} files match {list}")
            };
        }
        let only = |n: usize| n > 0 && n + c.matched == files && c.skipped == 0;
        if only(c.differ) {
            return format!(
                "{} of {files} files {} not match",
                c.differ,
                plural(c.differ, "does", "do")
            );
        }
        if only(c.missing) {
            return format!(
                "{} named in the list {} missing",
                crate::words::count(c.missing as u64, "file", "files"),
                plural(c.missing, "is", "are")
            );
        }
        let mut parts = Vec::new();
        if c.differ > 0 {
            parts.push(format!(
                "{} {}",
                c.differ,
                plural(c.differ, "differs", "differ")
            ));
        }
        if c.missing > 0 {
            parts.push(format!("{} missing", c.missing));
        }
        if c.unreadable > 0 {
            parts.push(format!("{} unreadable", c.unreadable));
        }
        if c.not_checked > 0 {
            parts.push(format!("{} not checked", c.not_checked));
        }
        parts.push(format!(
            "{} {}",
            c.matched,
            plural(c.matched, "match", "match")
        ));
        if c.skipped > 0 {
            parts.push(format!(
                "{} {} skipped",
                c.skipped,
                plural(c.skipped, "line", "lines")
            ));
        }
        parts.join(", ")
    }

    /// The failures, after the verdict, in the list's order, then the
    /// lines passed over.
    pub fn failures(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let name = display_name(&entry.name);
                match &entry.verdict {
                    EntryVerdict::Match { .. } => None,
                    EntryVerdict::Differs { algorithm } => {
                        Some(format!("Does not match: {name}, {}", algorithm.name()))
                    }
                    EntryVerdict::Missing => Some(format!("Missing: {name}")),
                    EntryVerdict::Unreadable(why) => Some(format!("Unreadable: {name}, {why}")),
                    EntryVerdict::NotChecked(algorithm) => {
                        Some(format!("Not checked: {name}, {}", algorithm.name()))
                    }
                }
            })
            .collect();
        lines.extend(self.problems.iter().map(ListProblem::words));
        lines
    }

    /// The matches, for the command that lists them: "Match: a.txt,
    /// SHA-256", or "Match: a.txt, BLAKE3, not SHA-256".
    pub fn matches(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match &entry.verdict {
                EntryVerdict::Match { algorithm, other } => Some(match other {
                    Some(other) => format!(
                        "Match: {}, {}, not {}",
                        display_name(&entry.name),
                        algorithm.name(),
                        other.name()
                    ),
                    None => format!("Match: {}, {}", display_name(&entry.name), algorithm.name()),
                }),
                _ => None,
            })
            .collect()
    }

    /// The verdict, then the failures.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.verdict()];
        lines.extend(self.failures());
        lines
    }

    /// The exit code of `diro sum --check`, as `sha256sum --check` has
    /// it: 0 when every file matches, 1 for any difference, missing or
    /// unreadable file or skipped line (2, wrong use, is the command's).
    pub fn exit_code(&self) -> i32 {
        if self.all_match() { 0 } else { 1 }
    }

    /// The one line on the list's signature, after the verdict, once the
    /// signature is checked (lane E's gpg or minisign): "List is signed:
    /// 12 of 12 files match", or "List not signed: 12 of 12 match".
    pub fn signature_line(&self, signed: bool) -> String {
        let c = self.counts();
        if signed {
            format!("List is signed: {} of {} files match", c.matched, c.files())
        } else {
            format!("List not signed: {} of {} match", c.matched, c.files())
        }
    }
}

/// Verifies a list already read, its names resolved against `folder`.
pub fn verify_parsed(
    provider: &dyn Provider,
    folder: &Place,
    list_name: &str,
    list: ChecksumList,
    ctx: &OpContext,
) -> Result<VerifyReport, MetaError> {
    let total = list.entries.len() as u64;
    let mut report = VerifyReport {
        list_name: list_name.to_owned(),
        entries: Vec::with_capacity(list.entries.len()),
        problems: list.problems,
    };
    for (done, entry) in list.entries.into_iter().enumerate() {
        ctx.check()?;
        ctx.progress.report(ProgressUpdate {
            items_done: done as u64,
            items_total: Some(total),
            current: Some(entry.name.clone()),
            ..ProgressUpdate::default()
        });
        let place = entry
            .parts()
            .into_iter()
            .try_fold(folder.clone(), |place, part| place.join(part));
        let verdict = match place {
            Err(_) => EntryVerdict::Unreadable("the name cannot be used here".to_owned()),
            Ok(place) => check_one(provider, &place, &entry.hex, &entry.candidates(), ctx)?,
        };
        report.entries.push(Verified {
            name: entry.name,
            verdict,
        });
    }
    Ok(report)
}

/// One file against one value; a cancel stops the whole check.
fn check_one(
    provider: &dyn Provider,
    place: &Place,
    hex: &str,
    candidates: &[Algorithm],
    ctx: &OpContext,
) -> Result<EntryVerdict, MetaError> {
    let computed: Vec<Algorithm> = candidates
        .iter()
        .copied()
        .filter(|a| a.computed())
        .collect();
    let Some(first) = computed.first().copied() else {
        return Ok(match candidates.first() {
            Some(algorithm) => EntryVerdict::NotChecked(*algorithm),
            None => EntryVerdict::Unreadable("its value has no known length".to_owned()),
        });
    };
    match provider.stat(place, ctx) {
        Err(VfsError::NotFound) => return Ok(EntryVerdict::Missing),
        Err(VfsError::Cancelled) => return Err(MetaError::Place(VfsError::Cancelled)),
        Err(error) => return Ok(EntryVerdict::Unreadable(error.to_string())),
        Ok(_) => {}
    }
    let digests = match compute(provider, place, &computed, ctx) {
        Ok(digests) => digests,
        Err(MetaError::Place(VfsError::Cancelled)) => {
            return Err(MetaError::Place(VfsError::Cancelled));
        }
        Err(error) => return Ok(EntryVerdict::Unreadable(error.to_string())),
    };
    let matched = digests.iter().find(|d| d.hex().eq_ignore_ascii_case(hex));
    Ok(match matched {
        Some(digest) => EntryVerdict::Match {
            algorithm: digest.algorithm,
            other: (computed.len() > 1)
                .then(|| computed.iter().copied().find(|a| *a != digest.algorithm))
                .flatten(),
        },
        None => EntryVerdict::Differs { algorithm: first },
    })
}

/// Verifies the list at `list`: read through its provider (at most 16
/// MiB), its names resolved against its own folder, each file read again
/// in full.
pub fn verify_list(
    provider: &dyn Provider,
    list: &Place,
    ctx: &OpContext,
) -> Result<VerifyReport, MetaError> {
    ctx.check()?;
    let name = list.name().unwrap_or_default().to_owned();
    let folder = list
        .parent()
        .ok_or_else(|| MetaError::unreadable("the list has no folder"))?;
    let mut bytes = Vec::new();
    provider
        .read(list, ctx)?
        .take(MAX_LIST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| MetaError::Place(e.into()))?;
    if bytes.len() > MAX_LIST_BYTES {
        return Err(MetaError::TooLarge);
    }
    let parsed = parse_list(&bytes, &name);
    verify_parsed(provider, &folder, &name, parsed, ctx)
}

#[cfg(test)]
mod tests {
    use diroxide_vfs::LocalProvider;

    use super::*;
    use crate::checksum::list::{ListForm, write_list};
    use crate::checksum::{Digest, compute_reader};

    fn report(entries: Vec<(&str, EntryVerdict)>, problems: Vec<ListProblem>) -> VerifyReport {
        VerifyReport {
            list_name: "SHA256SUMS".to_owned(),
            entries: entries
                .into_iter()
                .map(|(name, verdict)| Verified {
                    name: name.to_owned(),
                    verdict,
                })
                .collect(),
            problems,
        }
    }

    const MATCH: EntryVerdict = EntryVerdict::Match {
        algorithm: Algorithm::Sha256,
        other: None,
    };
    const DIFFERS: EntryVerdict = EntryVerdict::Differs {
        algorithm: Algorithm::Sha256,
    };

    #[test]
    fn the_verdict_first_in_every_case() {
        let all = report((0..12).map(|_| ("a.txt", MATCH)).collect(), vec![]);
        assert_eq!(all.verdict(), "All 12 files match SHA256SUMS");
        assert_eq!(all.exit_code(), 0);
        assert_eq!(all.signature_line(false), "List not signed: 12 of 12 match");
        assert_eq!(
            all.signature_line(true),
            "List is signed: 12 of 12 files match"
        );
        let one = report(vec![("a.txt", MATCH)], vec![]);
        assert_eq!(one.verdict(), "1 file matches SHA256SUMS");
        let mut two_bad: Vec<_> = (0..10).map(|_| ("a.txt", MATCH)).collect();
        two_bad.push(("setup.exe", DIFFERS));
        two_bad.push(("tool.exe", DIFFERS));
        let two_bad = report(two_bad, vec![]);
        assert_eq!(two_bad.verdict(), "2 of 12 files do not match");
        assert_eq!(
            two_bad.failures(),
            [
                "Does not match: setup.exe, SHA-256",
                "Does not match: tool.exe, SHA-256"
            ]
        );
        assert_eq!(two_bad.exit_code(), 1);
        let missing = report(
            vec![("a.txt", MATCH), ("notes.txt", EntryVerdict::Missing)],
            vec![],
        );
        assert_eq!(missing.verdict(), "1 file named in the list is missing");
        assert_eq!(missing.failures(), ["Missing: notes.txt"]);
        let mixed = report(
            vec![("a", MATCH), ("b", DIFFERS), ("c", EntryVerdict::Missing)],
            vec![ListProblem::Unreadable { line: 4 }],
        );
        assert_eq!(
            mixed.verdict(),
            "1 differs, 1 missing, 1 match, 1 line skipped"
        );
        assert_eq!(
            mixed.failures(),
            [
                "Does not match: b, SHA-256",
                "Missing: c",
                "A line cannot be read: line 4"
            ]
        );
        let none = report(vec![], vec![ListProblem::Unreadable { line: 1 }]);
        assert_eq!(none.verdict(), "No checksums read: 1 line skipped");
        assert_eq!(
            report(vec![], vec![]).verdict(),
            "No checksums in SHA256SUMS"
        );
        let sha1 = report(
            vec![
                ("old.iso", EntryVerdict::NotChecked(Algorithm::Sha1)),
                ("a", MATCH),
            ],
            vec![],
        );
        assert_eq!(sha1.verdict(), "1 not checked, 1 match");
        assert_eq!(sha1.failures(), ["Not checked: old.iso, SHA-1"]);
        for line in [
            all.verdict(),
            two_bad.verdict(),
            missing.verdict(),
            mixed.verdict(),
            all.signature_line(false),
            all.signature_line(true),
            two_bad.failures()[0].clone(),
        ] {
            assert!(line.chars().count() <= 48, "{line}");
        }
        for line in [
            all.verdict(),
            two_bad.verdict(),
            missing.verdict(),
            all.signature_line(true),
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap()
    }

    fn digest_of(algorithm: Algorithm, bytes: &[u8]) -> Digest {
        compute_reader(
            &mut std::io::Cursor::new(bytes.to_vec()),
            &[algorithm],
            &diroxide_vfs::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap()
        .remove(0)
    }

    /// The whole road on files of the test's own: a list written, read
    /// back and checked, with a changed file, a missing one, a name that
    /// leaves the folder, and a BLAKE3 value in a list that names none.
    #[test]
    fn a_list_written_here_is_checked_here() {
        let dir = scratch();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("alpha.txt"), b"alpha\n").unwrap();
        std::fs::write(dir.path().join("sub").join("gamma.txt"), b"gamma\r\n").unwrap();
        std::fs::write(dir.path().join("changed.txt"), b"after").unwrap();
        let entries = vec![
            (
                "alpha.txt".to_owned(),
                digest_of(Algorithm::Sha256, b"alpha\n"),
            ),
            (
                "sub/gamma.txt".to_owned(),
                digest_of(Algorithm::Sha256, b"gamma\r\n"),
            ),
            (
                "changed.txt".to_owned(),
                digest_of(Algorithm::Sha256, b"before"),
            ),
            ("missing.txt".to_owned(), digest_of(Algorithm::Sha256, b"x")),
        ];
        let mut text = write_list(&entries, ListForm::Gnu).unwrap();
        text.push_str(&format!("{}  ../outside.txt\n", "0".repeat(64)));
        std::fs::write(dir.path().join("SHA256SUMS"), &text).unwrap();
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let list = Place::parse(&dir.path().join("SHA256SUMS").display().to_string()).unwrap();
        let report = verify_list(&provider, &list, &ctx).unwrap();
        assert_eq!(
            report.verdict(),
            "1 differs, 1 missing, 2 match, 1 line skipped"
        );
        assert_eq!(
            report.failures(),
            [
                "Does not match: changed.txt, SHA-256",
                "Missing: missing.txt",
                "Skipped a name that leaves the folder: ../outside.txt",
            ]
        );
        assert_eq!(report.exit_code(), 1);

        let blake = digest_of(Algorithm::Blake3, b"alpha\n");
        std::fs::write(
            dir.path().join("checksums.txt"),
            format!("{}  alpha.txt\n", blake.hex()),
        )
        .unwrap();
        let list = Place::parse(&dir.path().join("checksums.txt").display().to_string()).unwrap();
        let report = verify_list(&provider, &list, &ctx).unwrap();
        assert_eq!(report.verdict(), "1 file matches checksums.txt");
        assert_eq!(report.matches(), ["Match: alpha.txt, BLAKE3, not SHA-256"]);

        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        assert_eq!(
            verify_list(&provider, &list, &cancelled),
            Err(MetaError::Place(VfsError::Cancelled))
        );
    }
}
