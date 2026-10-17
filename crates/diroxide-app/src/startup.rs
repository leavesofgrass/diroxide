//! What the app is given when it starts, read once from the config and
//! state folders before [`App::new`](crate::App::new): the problems found
//! in the config files, the menus and banks, the session to restore, the
//! interrupted jobs of the last session and the undo records of the
//! finished ones (from the undo journal), the prompts' histories, the home
//! folder, the labels, bookmarks and folder views (W3l), and the clock
//! with the local offset.
//!
//! [`load`] does the reading (I/O), so the app's core stays free of it;
//! tests build a [`Startup`] by hand or load one from a temporary state
//! folder, never the real one.

use std::collections::BTreeMap;
use std::fs;

use diroxide_config::{Config, ConfigProblem, Layout, Paths, SessionState};
use diroxide_core::{CommandId, Place, SystemClock, UtcOffset};
use diroxide_ops::{Interrupted, Journal, UndoRecord};

use crate::keymap::{self, Keymap, Platform};
use crate::menu::{self, Bank, Menus};
use crate::state;

/// What the app starts with besides its places, settings, keys and clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Startup {
    /// Every problem in the config files, file by file: the startup line
    /// says how many, and Config problems lists them.
    pub problems: Vec<ConfigProblem>,
    /// The menus (the owner's `menus.toml`, or the built-in ones).
    pub menus: Menus,
    /// The button banks; the first is the key bar.
    pub banks: Vec<Bank>,
    /// The session to restore, when one was saved.
    pub session: Option<SessionState>,
    /// The layout to use instead of the setting: the session's, unless
    /// the setting changed since the session was saved.
    pub layout: Option<Layout>,
    /// The jobs the last session did not finish, oldest first.
    pub interrupted: Vec<Interrupted>,
    /// The undo records of the finished jobs, oldest first, with the jobs
    /// an undo already reversed taken out.
    pub undo: Vec<UndoRecord>,
    /// The first job id: above every job in the journal, so a new job
    /// never writes over an old one's journal.
    pub first_job: u64,
    /// The prompts' histories, by key, newest first.
    pub histories: BTreeMap<String, Vec<String>>,
    /// The home folder.
    pub home: Option<Place>,
    /// Whether this is the first start (no session was ever saved).
    pub first_run: bool,
    /// The labels and the labeled places (`labels.toml`; the starter set
    /// on a first start).
    pub labels: crate::labels::LabelStore,
    /// The bookmarks (`bookmarks.toml`).
    pub bookmarks: crate::bookmarks::Bookmarks,
    /// The folders' own views (`folder-views.toml`).
    pub views: crate::views::FolderViews,
}

impl Default for Startup {
    fn default() -> Self {
        Self {
            problems: Vec::new(),
            menus: Menus::builtin(),
            banks: vec![menu::default_bank()],
            session: None,
            layout: None,
            interrupted: Vec::new(),
            undo: Vec::new(),
            first_job: 1,
            histories: BTreeMap::new(),
            home: None,
            first_run: false,
            labels: crate::labels::LabelStore::starter(),
            bookmarks: crate::bookmarks::Bookmarks::new(),
            views: crate::views::FolderViews::new(),
        }
    }
}

impl Startup {
    /// The palette's recent commands, newest first.
    pub fn palette_recent(&self) -> Vec<CommandId> {
        self.histories
            .get(state::keys::PALETTE)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| CommandId::new(id.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The system clock at the local offset from UTC, read once with `jiff`
/// (the zone database bundled, the Windows zone read from the system), so
/// every date in the panels, the details, the questions and `diro ls`
/// reads in local time. Falls back to UTC when the zone cannot be read.
pub fn local_clock() -> SystemClock {
    SystemClock::new(local_offset())
}

/// The local offset from UTC now, in seconds east.
pub fn local_offset() -> UtcOffset {
    let seconds = jiff::Zoned::now().offset().seconds();
    UtcOffset::from_seconds(seconds)
}

/// The terminal column's keymap with the owner's `keymap.toml` applied.
pub fn terminal_keymap(config: &Config, platform: Platform) -> Keymap {
    keymap::keymaps(&config.keymap, platform).0
}

/// The undo records to start with: the journal's finished records, oldest
/// first, where each undo job takes away the record it reversed (the
/// newest one before it), and undo jobs are not themselves undone. The
/// journal returns jobs that left nothing to undo too: an undo among them
/// still takes its record away, and a job with nothing to undo is dropped,
/// as the undo log drops it in the session.
pub fn pair_undo_records(records: Vec<UndoRecord>) -> Vec<UndoRecord> {
    let mut log: Vec<UndoRecord> = Vec::new();
    for record in records {
        if record.origin == diroxide_core::JobOrigin::Undo {
            log.pop();
        } else if !record.is_empty() {
            log.push(record);
        }
    }
    log
}

/// Reads everything the app starts with: the config's problems with the
/// keymap's and the menus' and banks' own, the references written beside
/// the owner's files, the session, the journal and the histories.
pub fn load(paths: &Paths, config: &Config, platform: Platform) -> Startup {
    let mut problems = config.problems.clone();
    problems.extend(keymap::app_keymap_problems(&config.keymap, platform));
    let (menus, menu_problems) = Menus::load(config.menus.as_ref());
    problems.extend(menu_problems);
    let (banks, bank_problems) = menu::banks(&config.banks);
    problems.extend(bank_problems);
    if let Err(error) = keymap::write_references(paths) {
        problems.push(ConfigProblem::new(
            Paths::KEYMAP_DEFAULTS,
            format!("The references could not be written: {error}."),
        ));
    }
    let session_path = paths.state_file(Paths::SESSION);
    let first_run = !session_path.exists();
    let loaded = diroxide_config::load_session(paths);
    problems.extend(loaded.problems);
    let session = (!first_run).then_some(loaded.value);
    let layout = session.as_ref().and_then(|session| {
        let setting_newer = match (
            fs::metadata(paths.config_file(Paths::SETTINGS)).and_then(|m| m.modified()),
            fs::metadata(&session_path).and_then(|m| m.modified()),
        ) {
            (Ok(setting), Ok(saved)) => setting > saved,
            _ => false,
        };
        (!setting_newer && session.layout != config.settings.layout).then_some(session.layout)
    });
    let journal = Journal::new(paths.state_dir.clone());
    let interrupted = journal.interrupted();
    let records = journal.records();
    let highest = records
        .iter()
        .filter_map(|r| r.job)
        .chain(interrupted.iter().map(|i| i.job))
        .map(diroxide_core::JobId::get)
        .max()
        .unwrap_or(0);
    let home = dirs::home_dir()
        .and_then(|home| home.into_os_string().into_string().ok())
        .and_then(|home| Place::parse(&home).ok());
    let labels = crate::labels::load(paths);
    problems.extend(labels.problems);
    let bookmarks = crate::bookmarks::load(paths);
    problems.extend(bookmarks.problems);
    let views = crate::views::load(paths);
    problems.extend(views.problems);
    Startup {
        problems,
        menus,
        banks,
        session,
        layout,
        interrupted,
        undo: pair_undo_records(records),
        first_job: highest + 1,
        histories: state::load_histories(paths),
        home,
        first_run,
        labels: labels.value,
        bookmarks: bookmarks.value,
        views: views.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{JobId, JobKind, JobOrigin};

    fn record(job: u64, origin: JobOrigin) -> UndoRecord {
        let mut record = UndoRecord::for_job(JobId::new(job), JobKind::Rename);
        if origin != JobOrigin::Undo {
            let place = Place::parse(if cfg!(windows) { r"D:\a" } else { "/a" }).unwrap();
            record.moves.push((place.clone(), place));
        }
        record.origin = origin;
        record
    }

    #[test]
    fn an_empty_undo_still_takes_its_record_and_an_empty_job_is_dropped() {
        let empty_job = UndoRecord::for_job(JobId::new(2), JobKind::DeletePermanently);
        let paired = pair_undo_records(vec![
            record(1, JobOrigin::User),
            empty_job,
            record(3, JobOrigin::User),
            UndoRecord {
                origin: JobOrigin::Undo,
                ..UndoRecord::for_job(JobId::new(4), JobKind::Undo)
            },
        ]);
        let jobs: Vec<u64> = paired
            .iter()
            .filter_map(|r| r.job)
            .map(JobId::get)
            .collect();
        assert_eq!(
            jobs,
            [1],
            "the undo reversed job 3; job 2 had nothing to undo"
        );
    }

    #[test]
    fn an_undo_in_the_journal_takes_away_what_it_reversed() {
        let paired = pair_undo_records(vec![
            record(1, JobOrigin::User),
            record(2, JobOrigin::User),
            record(3, JobOrigin::Undo),
            record(4, JobOrigin::User),
        ]);
        let jobs: Vec<u64> = paired
            .iter()
            .filter_map(|r| r.job)
            .map(JobId::get)
            .collect();
        assert_eq!(jobs, [1, 4]);
    }

    #[test]
    fn a_fresh_state_folder_is_a_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let config = diroxide_config::load(&paths);
        let startup = load(&paths, &config, Platform::current());
        assert!(startup.first_run);
        assert!(startup.problems.is_empty(), "{:?}", startup.problems);
        assert_eq!(startup.first_job, 1);
        assert!(paths.config_file(Paths::KEYMAP_DEFAULTS).exists());
        let offset = local_offset().seconds();
        assert!(offset.abs() <= 14 * 3600);
    }
}
