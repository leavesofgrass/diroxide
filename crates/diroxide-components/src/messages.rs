//! The lines the components feature says, meaning first, each carrying its
//! point within the 40 cells of the owner's Braille display (R22 section
//! 9), and the small models the app and `diro components` share: the
//! ask-first question, the Manage list's rows, the progress voice, the
//! "no" remembered until the next start, and the first-run list.
//!
//! Words before symbols; nothing here depends on color. Names a list or an
//! archive supplies are plain names, checked before they get here.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use diroxide_core::format::human_size;

use crate::component::{Checked, Component, FileState, Kind, Origin, Status};
use crate::download::{Event, Progress};
use crate::error::ComponentError;
use crate::manifest::Manifest;

/// "Not downloaded.": said after a no.
pub const NOT_DOWNLOADED: &str = "Not downloaded.";

/// Said when Escape stops a download.
pub const STOPPED: &str = "Download stopped; it resumes later.";

/// Said when every file arrived and is being checked.
pub const CHECKING: &str = "Checking the files.";

/// Said when an archive is being unpacked.
pub const UNPACKING: &str = "Unpacking the files.";

/// The first line when a component has no build for this computer.
pub const NO_BUILD: &str = "No build for this computer.";

/// The feature a key names, in words, for "X needs ...": `examples` is
/// "Getting example plugins", `sftp` "Opening SFTP places", `rar`
/// "Reading a RAR file". An unknown key is "This feature".
pub fn feature_words(feature: &str) -> &'static str {
    match feature {
        "examples" => "Getting example plugins",
        "sftp" => "Opening SFTP places",
        "rar" => "Reading a RAR file",
        "iso" => "Reading an ISO image",
        "previews" => "This preview",
        "conversions" => "This conversion",
        _ => "This feature",
    }
}

/// The ask-first question: the lines said first, then the question, which
/// ends "y or n" and starts on No. Nothing is fetched before a yes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskFirst {
    /// What is missing and why, then the size, the license and, for a
    /// component from the owner's own list, where it comes from.
    pub lines: Vec<String>,
    /// The question.
    pub question: String,
}

impl AskFirst {
    /// Every row in order, the question last: what a dialog shows.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = self.lines.clone();
        rows.push(self.question.clone());
        rows
    }
}

/// The license in words: "license MIT", "license MIT, unconfirmed".
fn license(c: &Component) -> String {
    if c.license_note.is_empty() {
        c.license.clone()
    } else {
        format!("{}, {}", c.license, c.license_note)
    }
}

/// Which list it came from, in words.
pub fn origin_line(origin: Origin) -> &'static str {
    match origin {
        Origin::Public => "From the public manifest.",
        Origin::OwnSource => "From your own components source.",
    }
}

/// The question asked before `component` is fetched for `target`, for
/// `feature` when a feature asked for it (the app), or with no feature
/// (`diro components download`). A plugin says so first; the size and the
/// license come before the question, so a terminal's prompt ends on it.
///
/// - "Reading a RAR file needs 7-Zip 25.01." / "Size 3.1 MB, license
///   LGPL-2.1-or-later." / "Download it now? y or n"
/// - "Opening SFTP places needs a plugin." / "License MIT. From the
///   public manifest." / "Download diro-sftp 0.1.0, 4.2 MB? y or n"
pub fn ask_first(component: &Component, feature: Option<&str>, target: &str) -> AskFirst {
    let size = human_size(component.size_for(target));
    let words = feature.map_or("", feature_words);
    if component.kind == Kind::Plugin {
        let needs = match feature {
            Some(_) => format!("{words} needs a plugin."),
            None => format!("{}: a plugin.", component.title),
        };
        return AskFirst {
            lines: vec![
                needs,
                format!(
                    "License {}. {}",
                    license(component),
                    origin_line(component.origin)
                ),
            ],
            question: format!(
                "Download {} {}, {size}? y or n",
                component.id, component.version
            ),
        };
    }
    let needs = match feature {
        Some(_) => format!("{words} needs {} {}.", component.title, component.version),
        None => format!(
            "{} {}, {}.",
            component.title, component.version, component.id
        ),
    };
    let mut lines = vec![
        needs,
        format!("Size {size}, license {}.", license(component)),
    ];
    if component.origin == Origin::OwnSource {
        lines.push(origin_line(Origin::OwnSource).to_owned());
    }
    AskFirst {
        lines,
        question: "Download it now? y or n".to_owned(),
    }
}

/// The lines when a component has no build for this computer: the fact,
/// then what to do.
pub fn no_build(component: &Component) -> Vec<String> {
    let next = if component.kind == Kind::Helper {
        format!("Install {} from your system's packages.", component.title)
    } else {
        "Install it from a file, if you have one.".to_owned()
    };
    vec![NO_BUILD.to_owned(), next]
}

/// "Downloading diro-sftp: 40 percent."
pub fn downloading(id: &str, percent: u64) -> String {
    format!("Downloading {id}: {percent} percent.")
}

/// The first line of a download: "Downloading diro-sftp, 4.2 MB."
pub fn starting(id: &str, bytes: u64) -> String {
    format!("Downloading {id}, {}.", human_size(bytes))
}

/// "Ready: diro-sftp 0.1.0."
pub fn ready(component: &Component) -> String {
    format!("Ready: {} {}.", component.id, component.version)
}

/// "Already installed: diro-sftp 0.1.0."
pub fn already_installed(component: &Component) -> String {
    format!("Already installed: {} {}.", component.id, component.version)
}

/// "Left out ../x: name leaves the archive."
pub fn left_out(name: &str, reason: &str) -> String {
    if name.is_empty() {
        format!("Left out: {reason}.")
    } else {
        format!("Left out {name}: {reason}.")
    }
}

/// "Not downloaded: needs 300 MB, 120 free." The free amount drops its
/// unit when it shares the needed one.
pub fn no_room(needed: u64, free: u64) -> String {
    let needed = human_size(needed);
    let free = human_size(free);
    let free = match (needed.rsplit_once(' '), free.rsplit_once(' ')) {
        (Some((_, a)), Some((number, b))) if a == b => number.to_owned(),
        _ => free,
    };
    format!("Not downloaded: needs {needed}, {free} free.")
}

/// The lines for a failed download or install, the standard form first,
/// then the next step.
pub fn failure(error: &ComponentError) -> Vec<String> {
    let two = |a: String, b: &str| vec![a, b.to_owned()];
    match error {
        ComponentError::Cancelled => vec![STOPPED.to_owned()],
        ComponentError::Hash { .. } | ComponentError::Size { .. } => two(
            "Not installed: a file did not match.".to_owned(),
            "Get it again, or check the source.",
        ),
        ComponentError::NoTransport => two(
            "Not downloaded: curl was not found.".to_owned(),
            "Install curl, or install from a file.",
        ),
        ComponentError::NoRoom { needed, free } => {
            two(no_room(*needed, *free), "Make room, then try again.")
        }
        ComponentError::NoBuild(_) => vec![NO_BUILD.to_owned()],
        ComponentError::Busy(id) => two(
            format!("Already downloading: {id}."),
            "Wait for it to finish.",
        ),
        ComponentError::Fetch { reason, .. } => two(
            format!("Not downloaded: {reason}."),
            "Try again later, or install from a file.",
        ),
        ComponentError::Unpack(reason) => vec![format!("Not installed: {reason}.")],
        ComponentError::Io { path, reason } => vec![
            "Not installed: a folder cannot be written.".to_owned(),
            format!("{path}: {reason}."),
        ],
        other => vec![format!("Not downloaded: {other}.")],
    }
}

/// The Verify lines: "Files check out: SFTP places plugin." or the count
/// that do not, then one line a file, the state first.
pub fn verified(component: &Component, checked: &[Checked]) -> Vec<String> {
    let bad: Vec<&Checked> = checked
        .iter()
        .filter(|c| c.state != FileState::Good)
        .collect();
    if bad.is_empty() {
        return vec![format!("Files check out: {}.", component.title)];
    }
    let first = if bad.len() == 1 {
        format!("1 file does not check out: {}.", component.title)
    } else {
        format!("{} files do not check out: {}.", bad.len(), component.title)
    };
    let mut lines = vec![first];
    for file in bad {
        let state = file.state.words();
        let mut state_words = state.to_owned();
        if let Some(first) = state_words.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        lines.push(format!("{state_words}: {}", file.path));
    }
    lines
}

/// One row of Manage optional components, meaning first: "SFTP places
/// plugin: not installed, 4.2 MB, license MIT".
pub fn manage_row(component: &Component, status: &Status, target: &str) -> String {
    format!(
        "{}: {}, {}, license {}",
        component.title,
        status.words(),
        human_size(component.size_for(target)),
        license(component)
    )
}

/// The Remove question and its count: "Remove SFTP places plugin? y or n"
/// after "Removes 3 files. Yours stay."
pub fn remove_question(component: &Component) -> String {
    format!("Remove {}? y or n", component.title)
}

/// A row of the Manage list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManageRow {
    /// The component's id.
    pub id: String,
    /// The row, meaning first.
    pub line: String,
    /// Its state.
    pub status: Status,
}

/// The Manage list's rows for every component in `manifest`, in its order.
pub fn manage_rows(manifest: &Manifest, components_dir: &Path, target: &str) -> Vec<ManageRow> {
    manifest
        .components
        .iter()
        .map(|c| {
            let status = c.status_in(components_dir, target);
            ManageRow {
                id: c.id.clone(),
                line: manage_row(c, &status, target),
                status,
            }
        })
        .collect()
}

/// The components a "no" was said to in this session: the feature that
/// asked says what it cannot do instead, and asks again at the next start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declined(BTreeSet<String>);

impl Declined {
    /// Remembers a no to `id` until the program ends.
    pub fn remember(&mut self, id: &str) {
        self.0.insert(id.to_owned());
    }

    /// Whether a no was said to `id` in this session.
    pub fn is_declined(&self, id: &str) -> bool {
        self.0.contains(id)
    }
}

/// The first-run choices: every component but the examples pack, nothing
/// chosen. Shown once, at the first start, and only when it holds
/// something (from alpha 5, when the SFTP plugin joins the list).
pub fn first_run_choices(manifest: &Manifest) -> Vec<&Component> {
    manifest
        .components
        .iter()
        .filter(|c| !c.features.iter().any(|f| f == "examples"))
        .collect()
}

/// Whether to show the first-run list now: not shown before, and not empty.
pub fn show_first_run(manifest: &Manifest, shown_before: bool) -> bool {
    !shown_before && !first_run_choices(manifest).is_empty()
}

/// Says a download's progress the way the jobs line does (R22 section
/// 6.2): the start, a stop, a failure and the end always; a step only when
/// the percentage crosses a tenth and at least the progress interval has
/// passed since the last line, so a short download says little more than
/// its start and "Ready". Never a spinner.
#[derive(Clone, Debug)]
pub struct ProgressVoice {
    id: String,
    interval: Duration,
    tenth: u64,
    last: Option<Duration>,
}

impl ProgressVoice {
    /// A voice for `id`, saying a step at most every `interval` (the
    /// progress setting, ten seconds by default).
    pub fn new(id: &str, interval: Duration) -> Self {
        Self {
            id: id.to_owned(),
            interval,
            tenth: 0,
            last: None,
        }
    }

    /// The line to say for `event` at `now` (time since the download
    /// began), if any.
    pub fn hear(&mut self, event: &Event, now: Duration) -> Option<String> {
        let line = match event {
            Event::Started(Progress { total, .. }) => Some(starting(&self.id, *total)),
            Event::Progress(progress) => {
                let tenth = progress.percent() / 10 * 10;
                let due = self
                    .last
                    .is_none_or(|last| now.saturating_sub(last) >= self.interval);
                (tenth > self.tenth && tenth < 100 && due).then(|| {
                    self.tenth = tenth;
                    downloading(&self.id, tenth)
                })
            }
            Event::Retrying { file, .. } => Some(format!("Getting {file} again from the start.")),
            Event::Checking => Some(CHECKING.to_owned()),
            Event::Unpacking => Some(UNPACKING.to_owned()),
        };
        if line.is_some() {
            self.last = Some(now);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Origin;
    use crate::pin::{ANY_TARGET, FilePin};
    use diroxide_core::format::{BRAILLE_CELLS, within_cells};

    fn sized(
        id: &str,
        title: &str,
        kind: Kind,
        version: &str,
        license: &str,
        bytes: u64,
    ) -> Component {
        let mut c = Component::new(
            id,
            title,
            vec![FilePin::new("a.bin", bytes, "0".repeat(64))],
        );
        c.kind = kind;
        c.version = version.into();
        c.license = license.into();
        c
    }

    fn sftp() -> Component {
        sized(
            "diro-sftp",
            "SFTP places plugin",
            Kind::Plugin,
            "0.1.0",
            "MIT",
            4_200_000,
        )
    }

    fn seven_zip() -> Component {
        sized(
            "7zip",
            "7-Zip",
            Kind::Helper,
            "25.01",
            "LGPL-2.1-or-later",
            3_100_000,
        )
    }

    /// Each golden line, and the words that must be whole in its first 40
    /// cells.
    fn golden() -> Vec<(String, &'static str, &'static str)> {
        let rar = ask_first(&seven_zip(), Some("rar"), ANY_TARGET);
        let plugin = ask_first(&sftp(), Some("sftp"), ANY_TARGET);
        let mut own = sftp();
        own.origin = Origin::OwnSource;
        let own = ask_first(&own, Some("sftp"), ANY_TARGET);
        let removal = crate::component::Removal {
            files: vec!["a".into(), "b".into(), "c".into()],
            folders: Vec::new(),
            kept: vec!["d".into()],
        };
        vec![
            (
                rar.lines[0].clone(),
                "Reading a RAR file needs 7-Zip 25.01.",
                "needs 7-Zip",
            ),
            (
                rar.lines[1].clone(),
                "Size 3.1 MB, license LGPL-2.1-or-later.",
                "3.1 MB",
            ),
            (rar.question.clone(), "Download it now? y or n", "y or n"),
            (
                plugin.lines[0].clone(),
                "Opening SFTP places needs a plugin.",
                "needs a plugin",
            ),
            (
                plugin.lines[1].clone(),
                "License MIT. From the public manifest.",
                "public manifest",
            ),
            (
                plugin.question.clone(),
                "Download diro-sftp 0.1.0, 4.2 MB? y or n",
                "y or n",
            ),
            (
                own.lines[1].clone(),
                "License MIT. From your own components source.",
                "your own",
            ),
            (NOT_DOWNLOADED.into(), "Not downloaded.", "Not downloaded"),
            (NO_BUILD.into(), "No build for this computer.", "No build"),
            (
                downloading("diro-sftp", 40),
                "Downloading diro-sftp: 40 percent.",
                "40 percent",
            ),
            (
                STOPPED.into(),
                "Download stopped; it resumes later.",
                "resumes later",
            ),
            (CHECKING.into(), "Checking the files.", "Checking"),
            (ready(&sftp()), "Ready: diro-sftp 0.1.0.", "Ready"),
            (
                verified(&sftp(), &[])[0].clone(),
                "Files check out: SFTP places plugin.",
                "check out",
            ),
            (
                failure(&ComponentError::Hash { file: "a".into() })[0].clone(),
                "Not installed: a file did not match.",
                "did not match",
            ),
            (
                failure(&ComponentError::Hash { file: "a".into() })[1].clone(),
                "Get it again, or check the source.",
                "Get it again",
            ),
            (
                failure(&ComponentError::NoTransport)[0].clone(),
                "Not downloaded: curl was not found.",
                "curl was not found",
            ),
            (
                failure(&ComponentError::NoTransport)[1].clone(),
                "Install curl, or install from a file.",
                "Install curl",
            ),
            (
                no_room(300_000_000, 120_000_000),
                "Not downloaded: needs 300 MB, 120 free.",
                "needs 300 MB, 120 free",
            ),
            (
                manage_row(&sftp(), &Status::NotInstalled, ANY_TARGET),
                "SFTP places plugin: not installed, 4.2 MB, license MIT",
                "not installed",
            ),
            (
                remove_question(&sftp()),
                "Remove SFTP places plugin? y or n",
                "y or n",
            ),
            (
                removal.summary(),
                "Removes 3 files. Yours stay.",
                "Yours stay",
            ),
            (
                failure(&ComponentError::Cancelled)[0].clone(),
                "Download stopped; it resumes later.",
                "stopped",
            ),
        ]
    }

    #[test]
    fn every_line_is_its_golden_text_with_its_point_in_40_cells() {
        for (line, expected, point) in golden() {
            assert_eq!(line, expected);
            assert!(
                within_cells(&line, point),
                "\"{point}\" is not within the first {BRAILLE_CELLS} cells of \"{line}\""
            );
        }
    }

    #[test]
    fn the_question_without_a_feature_names_the_component() {
        let mut pack = sized(
            "diro-examples",
            "Example plugins",
            Kind::Data,
            "0.1.0",
            "Apache-2.0 OR MIT",
            31_000,
        );
        let ask = ask_first(&pack, None, ANY_TARGET);
        assert_eq!(
            ask.rows(),
            [
                "Example plugins 0.1.0, diro-examples.",
                "Size 31.0 KB, license Apache-2.0 OR MIT.",
                "Download it now? y or n",
            ]
        );
        pack.origin = Origin::OwnSource;
        pack.license_note = "unconfirmed".into();
        let ask = ask_first(&pack, Some("examples"), ANY_TARGET);
        assert_eq!(
            ask.rows(),
            [
                "Getting example plugins needs Example plugins 0.1.0.",
                "Size 31.0 KB, license Apache-2.0 OR MIT, unconfirmed.",
                "From your own components source.",
                "Download it now? y or n",
            ]
        );
        assert!(within_cells(&ask.lines[0], "needs Example"));
    }

    #[test]
    fn verify_names_each_file_that_does_not_check_out() {
        let checked = [
            Checked {
                path: "a.toml".into(),
                state: FileState::Good,
            },
            Checked {
                path: "b.toml".into(),
                state: FileState::WrongHash,
            },
        ];
        assert_eq!(
            verified(&sftp(), &checked),
            [
                "1 file does not check out: SFTP places plugin.",
                "Does not match: b.toml"
            ]
        );
        let two = [
            Checked {
                path: "a".into(),
                state: FileState::Missing,
            },
            Checked {
                path: "b".into(),
                state: FileState::WrongSize(3),
            },
        ];
        assert_eq!(
            verified(&sftp(), &two),
            [
                "2 files do not check out: SFTP places plugin.",
                "Missing: a",
                "Wrong size: b"
            ]
        );
    }

    #[test]
    fn progress_is_said_by_tenths_no_more_often_than_the_interval() {
        let mut voice = ProgressVoice::new("diro-sftp", Duration::from_secs(10));
        let step = |done| Event::Progress(Progress { done, total: 100 });
        let s = Duration::from_secs;
        assert_eq!(
            voice.hear(
                &Event::Started(Progress {
                    done: 0,
                    total: 4_200_000
                }),
                s(0)
            ),
            Some("Downloading diro-sftp, 4.2 MB.".into())
        );
        assert_eq!(voice.hear(&step(15), s(3)), None, "within the interval");
        assert_eq!(
            voice.hear(&step(42), s(11)),
            Some("Downloading diro-sftp: 40 percent.".into())
        );
        assert_eq!(voice.hear(&step(45), s(30)), None, "the same tenth");
        assert_eq!(
            voice.hear(&step(100), s(40)),
            None,
            "the end is said by Ready"
        );
        assert_eq!(voice.hear(&Event::Checking, s(40)), Some(CHECKING.into()));
        // A short download: the start, then nothing until Checking.
        let mut quick = ProgressVoice::new("diro-examples", Duration::from_secs(10));
        let lines: Vec<String> = [0, 30, 60, 90, 100]
            .iter()
            .filter_map(|d| quick.hear(&step(*d), Duration::from_millis(*d)))
            .collect();
        assert_eq!(lines, ["Downloading diro-examples: 30 percent."]);
    }

    #[test]
    fn a_no_is_remembered_and_the_first_run_list_waits_for_more_than_examples() {
        let mut declined = Declined::default();
        assert!(!declined.is_declined("diro-sftp"));
        declined.remember("diro-sftp");
        assert!(declined.is_declined("diro-sftp"));
        let mut manifest = Manifest::new("0.1.0", "test");
        let mut examples = sized(
            "diro-examples",
            "Example plugins",
            Kind::Data,
            "0.1.0",
            "MIT",
            10,
        );
        examples.features = vec!["examples".into()];
        manifest.components.push(examples);
        assert!(!show_first_run(&manifest, false));
        manifest.components.push(sftp());
        assert!(show_first_run(&manifest, false));
        assert!(!show_first_run(&manifest, true));
        assert_eq!(first_run_choices(&manifest).len(), 1);
        let tmp = tempfile::tempdir().unwrap();
        let rows = manage_rows(&manifest, tmp.path(), ANY_TARGET);
        assert_eq!(
            rows[1].line,
            "SFTP places plugin: not installed, 4.2 MB, license MIT"
        );
        assert_eq!(rows[1].status, Status::NotInstalled);
    }

    #[test]
    fn failures_say_the_fact_first() {
        let lines = failure(&ComponentError::Fetch {
            file: "a".into(),
            reason: "the connection was lost".into(),
        });
        assert_eq!(
            lines,
            [
                "Not downloaded: the connection was lost.",
                "Try again later, or install from a file."
            ]
        );
        assert_eq!(
            no_room(300_000_000, 900_000),
            "Not downloaded: needs 300 MB, 900 KB free."
        );
        assert_eq!(
            left_out("../x", "name leaves the archive"),
            "Left out ../x: name leaves the archive."
        );
        assert_eq!(
            no_build(&seven_zip())[1],
            "Install 7-Zip from your system's packages."
        );
    }
}
