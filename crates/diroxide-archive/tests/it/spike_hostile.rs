//! Spike 4.1 (research/alpha2-plan.md part four): hostile archives against
//! the limits. Every case of [`crate::hostile::cases`] runs under a
//! watchdog of five seconds, with its resident peak measured; none may
//! panic, hang or pass 512 MiB.
//!
//! The first half is the risk itself: the format crates alone, with no
//! guard of diroxide's around them, on the same inputs. It shows what the
//! provider has to add: the crates open the bomb, trust the claims and
//! follow nothing, so every refusal is the provider's to make.

use std::io::{Cursor, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::hostile::{Act, Case, Expect, cases};
use crate::memory::{Peak, mib};

/// The longest a case may take.
pub const WATCHDOG: Duration = Duration::from_secs(5);

/// What running a case gave: its outcome in words, its time and its peak.
pub struct Run {
    /// The outcome in words.
    pub outcome: String,
    /// How long it took.
    pub elapsed: Duration,
    /// The largest resident size seen, in bytes.
    pub peak: u64,
}

/// Runs `work` on a thread under the watchdog, catching a panic, watching
/// the resident size. Panics if the work hangs past the watchdog.
pub fn guarded(name: &str, work: impl FnOnce() -> String + Send + 'static) -> Run {
    let (send, receive) = mpsc::channel();
    let peak = Peak::start();
    let started = Instant::now();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(work));
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(WATCHDOG);
    let elapsed = started.elapsed();
    let peak = peak.stop();
    let outcome = match result {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => format!("PANIC in {name}"),
        Err(_) => panic!("{name}: hung past {WATCHDOG:?}"),
    };
    Run {
        outcome,
        elapsed,
        peak,
    }
}

/// Reads at most 128 MiB from a reader, saying how much came and how it
/// ended.
fn drain(mut reader: impl Read) -> String {
    let mut sink = Vec::new();
    match reader.by_ref().take(128 << 20).read_to_end(&mut sink) {
        Ok(n) => format!("read {n} bytes"),
        Err(e) => format!("read error: {e}"),
    }
}

/// The case run against the format crate alone.
fn raw(case: Case) -> String {
    let bytes = (case.bytes)();
    let file = case.file;
    if file.ends_with(".zip") {
        let mut archive = match zip::ZipArchive::new(Cursor::new(bytes)) {
            Ok(archive) => archive,
            Err(e) => return format!("zip refused it: {e}"),
        };
        let overlap = archive.has_overlapping_files().unwrap_or(false);
        let target = match case.act {
            Act::Read(member) | Act::Row(member) => member,
            Act::List | Act::ListFolder(_) => {
                return format!("zip listed {} names, overlap {overlap}", archive.len());
            }
        };
        match archive.by_name(target) {
            Ok(member) => format!("zip opened {target} (overlap {overlap}): {}", drain(member)),
            Err(e) => format!("zip could not open {target}: {e}"),
        }
    } else if file.ends_with(".7z") {
        match sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty())
        {
            Ok(reader) => format!("7z listed {} entries", reader.archive().files.len()),
            Err(e) => format!("7z refused it: {e}"),
        }
    } else if file.ends_with(".tar.gz") {
        let decoder = flate2::read::MultiGzDecoder::new(Cursor::new(bytes));
        walk_tar(decoder)
    } else if file.ends_with(".tar") {
        walk_tar(Cursor::new(bytes))
    } else if file.ends_with(".xz") {
        drain(lzma_rust2::XzReader::new(Cursor::new(bytes), true))
    } else {
        "no crate for this name".to_owned()
    }
}

/// Walks a tar's entries with the tar crate, reading each member.
fn walk_tar(reader: impl Read) -> String {
    let mut archive = tar::Archive::new(reader);
    let Ok(entries) = archive.entries() else {
        return "tar refused it".to_owned();
    };
    let mut seen = Vec::new();
    for entry in entries {
        match entry {
            Ok(mut entry) => {
                let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
                let mut data = Vec::new();
                let read = entry.read_to_end(&mut data).map_or(0, |n| n);
                seen.push(format!("{name} ({read} bytes)"));
            }
            Err(e) => {
                seen.push(format!("error: {e}"));
                break;
            }
        }
    }
    format!("tar walked: {}", seen.join(", "))
}

/// Cases the crates alone must not be given: each allocates what the
/// archive claims. Measured on the owner's machine on Friday, October 16,
/// 2026: `lzma-rust2`'s `XzReader` without a memory limit took 4,012 MiB
/// resident for the xz block that asks for a 4 GiB dictionary, and
/// `sevenz-rust2` would do the same for the 7z content block. Running them
/// here would make the test suite need 4 GiB of free memory.
const CLAIMS_ITS_MEMORY: [&str; 2] = [
    "xz block over the dictionary cap",
    "7z content asking for a 4 GiB dictionary",
];

#[test]
fn the_crates_alone_neither_panic_nor_refuse() {
    let mut findings = Vec::new();
    for case in cases() {
        if CLAIMS_ITS_MEMORY.contains(&case.name) {
            findings.push(format!(
                "raw | {} | not run: allocates its claim",
                case.name
            ));
            continue;
        }
        let run = guarded(case.name, move || raw(case));
        findings.push(format!(
            "raw | {} | {} | {:?} | peak {:.0} MiB",
            case.name,
            run.outcome,
            run.elapsed,
            mib(run.peak)
        ));
        assert!(!run.outcome.starts_with("PANIC"), "{}", run.outcome);
    }
    for line in &findings {
        eprintln!("{line}");
    }
    // The risks the provider has to cover, as the crates show them.
    let said = |name: &str| {
        findings
            .iter()
            .find(|line| line.starts_with(&format!("raw | {name} |")))
            .cloned()
            .unwrap_or_default()
    };
    assert!(said("quoted-overlap zip").contains("read 1048918 bytes"));
    assert!(said("4 GB claimed over 1 KB").contains("read 1024 bytes"));
    assert!(said("local and central names disagree").contains("read 7 bytes"));
    assert!(said("gzip of zeros at over 1,000 to 1").contains("67108864 bytes"));
}

/// The lying-size cases of the spike through the limiter chain: the
/// format crate's reader inside `Guarded`, with the member's own claims.
#[test]
fn the_limiter_chain_stops_a_member_that_lies_about_its_size() {
    use diroxide_archive::guard::{Budget, Claims, Guarded, from_io};
    use diroxide_archive::{Allowances, Limits};
    use diroxide_vfs::CancelToken;
    let cases = [
        (
            crate::hostile::claims_four_gb(),
            "big.bin",
            "shorter than its stated size",
        ),
        // The zip crate's own check (9.0.1) stops this one first, in its
        // words here (the provider says it in its own); the chain's is the
        // net behind it for every other codec.
        (
            crate::hostile::claims_too_little(),
            "small.bin",
            "larger than its declared",
        ),
    ];
    for (bytes, name, words) in cases {
        let run = guarded(name, move || {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let member = archive.by_name(name).unwrap();
            let claims = Claims {
                name: name.to_owned(),
                size: Some(member.size()),
                packed: Some(member.compressed_size()),
                crc32: Some(member.crc32()),
            };
            let limits = Limits::strict();
            let mut reader = Guarded::new(
                member,
                claims,
                limits,
                Budget::new(&limits),
                Allowances::all(),
                CancelToken::new(),
            );
            let mut sink = Vec::new();
            match reader.read_to_end(&mut sink) {
                Ok(n) => format!("read {n} bytes"),
                Err(e) => from_io(e).to_string(),
            }
        });
        assert!(run.outcome.contains(words), "{name}: {}", run.outcome);
        assert!(run.peak < 512 << 20, "{name}: {:.0} MiB", mib(run.peak));
    }
}

/// What a case ended in, through the provider.
#[derive(Debug)]
enum Ended {
    /// Refused, with the sentence the app would say.
    Refused(String),
    /// A question first.
    Asked(String),
    /// A listing: its rows and its skip lines.
    Listed {
        rows: Vec<String>,
        skipped: Vec<String>,
    },
    /// Read to the end.
    Read(usize),
}

/// The formats the provider reads so far; a case of another waits.
const READ_SO_FAR: [&str; 4] = [".zip", ".tar", ".tar.gz", ".xz"];

fn through_provider(case: Case) -> Ended {
    use diroxide_archive::{Allowances, ArchiveError, member_row};
    use diroxide_vfs::OpContext;
    let world = crate::world::World::new(diroxide_archive::Limits::strict());
    let archive = world.put(case.file, &(case.bytes)());
    let allow = if case.allow {
        Allowances::all()
    } else {
        Allowances::default()
    };
    let say = |error: ArchiveError, verb: &str, object: &str| match error {
        ArchiveError::Ask(question) => Ended::Asked(question.message().text),
        other => Ended::Refused(other.message(verb, object).text),
    };
    let ctx = OpContext::new();
    let list = |folder: &diroxide_core::Place| match world.provider.list_folder(folder, &ctx, allow)
    {
        Ok(listing) => {
            let skipped = world
                .provider
                .open_archive(&archive, &ctx, allow)
                .map(|o| o.index().summary().refused_lines.clone())
                .unwrap_or_default();
            Ended::Listed {
                rows: listing.entries.iter().map(member_row).collect(),
                skipped,
            }
        }
        Err(error) => say(error, "open", case.file),
    };
    match case.act {
        Act::List => list(&archive.clone().into_archive()),
        Act::ListFolder(path) => list(&world.inside(&archive, path)),
        Act::Read(member) => {
            let place = world.inside(&archive, member);
            match world.read_with(&place, allow) {
                Ok(bytes) => Ended::Read(bytes.len()),
                Err(error) => say(error, "read", member),
            }
        }
        Act::Row(member) => {
            let folder = match member.rsplit_once('/') {
                Some((folder, _)) => world.inside(&archive, folder),
                None => archive.clone().into_archive(),
            };
            let leaf = member.rsplit('/').next().unwrap_or(member);
            match world.provider.list_folder(&folder, &ctx, allow) {
                Ok(listing) => Ended::Listed {
                    rows: listing
                        .entries
                        .iter()
                        .filter(|e| e.name == leaf)
                        .map(member_row)
                        .collect(),
                    skipped: Vec::new(),
                },
                Err(error) => say(error, "open", case.file),
            }
        }
    }
}

/// Whether a case ended as it must.
fn ends_as_it_must(expect: Expect, ended: &Ended) -> bool {
    match (expect, ended) {
        (Expect::Refused(words), Ended::Refused(said)) => said.contains(words),
        (Expect::Asked(words), Ended::Asked(said)) => {
            said.contains(words) && said.ends_with("? y or n")
        }
        (Expect::Flagged(words), Ended::Listed { rows, .. }) => {
            !rows.is_empty() && rows.iter().any(|row| row.contains(words))
        }
        (Expect::Skipped(words), Ended::Listed { skipped, .. }) => {
            skipped.iter().any(|line| line.contains(words))
        }
        (Expect::Fine, Ended::Read(bytes)) => *bytes > 0,
        (Expect::Fine, Ended::Listed { .. }) => true,
        _ => false,
    }
}

#[test]
fn every_hostile_case_ends_in_its_sentence() {
    let mut report = Vec::new();
    for case in cases() {
        if !READ_SO_FAR.iter().any(|ext| case.file.ends_with(ext)) {
            report.push(format!("waits | {}", case.name));
            continue;
        }
        let (send, receive) = mpsc::channel();
        let peak = Peak::start();
        let started = Instant::now();
        thread::spawn(move || {
            let _ = send.send(through_provider(case));
        });
        let ended = receive
            .recv_timeout(WATCHDOG)
            .unwrap_or_else(|_| panic!("{}: hung past {WATCHDOG:?}", case.name));
        let elapsed = started.elapsed();
        let peak = peak.stop();
        report.push(format!(
            "provider | {} | {ended:?} | {elapsed:?} | peak {:.0} MiB",
            case.name,
            mib(peak)
        ));
        assert!(
            ends_as_it_must(case.expect, &ended),
            "{}: expected {:?}, ended {ended:?}",
            case.name,
            case.expect
        );
        assert!(peak < 512 << 20, "{}: {:.0} MiB", case.name, mib(peak));
    }
    for line in report {
        eprintln!("{line}");
    }
}
