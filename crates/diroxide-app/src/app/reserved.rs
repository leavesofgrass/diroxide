//! The commands reserved for alpha 5's app lanes by the pre-wiring step
//! (lane P5), and the one line their stubs said until a lane filled them.
//!
//! Each lane's dispatch (`renaming.rs`, `syncing.rs`, `duplicates.rs`,
//! `collections.rs`) routes its ids to functions in its own files. Each
//! such function started as a stub that said "Not built yet: NAME." with
//! the command's catalog name. Every lane has filled its arms, so the
//! stub is gone, and a test holds that no reserved command says the
//! line again.

use diroxide_core::CommandId;

use crate::commands::ids;

/// The start of the line a reserved command said until its lane filled it.
#[cfg(test)]
const NOT_BUILT: &str = "Not built yet: ";

/// Every reserved command, with the lane that fills it: the tests walk it.
#[cfg_attr(not(test), allow(dead_code, reason = "read by the tests"))]
pub(super) const RESERVED: [(CommandId, &str); 24] = [
    (ids::BATCH_RENAME, "RA"),
    (ids::RENAME_ONE_BY_ONE, "RA"),
    (ids::RENAME_IN_EDITOR, "RA"),
    (ids::RENAME_MAKE_UNIQUE, "RA"),
    (ids::RENAME_SKIP_PROBLEMS, "RA"),
    (ids::RENAME_NEXT_PROBLEM, "RA"),
    (ids::RENAME_PREVIOUS_PROBLEM, "RA"),
    (ids::RENAME_SAY_BOTH, "RA"),
    (ids::RENAME_NEXT_PASS, "RA"),
    (ids::RENAME_PRESETS, "RA"),
    (ids::COMPARE_FOLDERS, "SA"),
    (ids::SYNCHRONIZE, "SA"),
    (ids::SYNC_TOWARD_HERE, "SA"),
    (ids::SYNC_TOWARD_THERE, "SA"),
    (ids::COMPARE_AGAIN, "SA"),
    (ids::FIND_DUPLICATES, "DA"),
    (ids::DUPS_KEEP, "DA"),
    (ids::FLAT_VIEW, "LA"),
    (ids::COLLECTIONS, "LA"),
    (ids::RECENT_FILES, "LA"),
    (ids::SAVE_AS_COLLECTION, "LA"),
    (ids::COLLECT, "LA"),
    (ids::COLLECTION_REMOVE_MISSING, "LA"),
    (ids::RECENT_CLEAR, "LA"),
];

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diroxide_core::{Command, FixedClock, Place, Timestamp, UtcOffset};

    use super::*;
    use crate::app::{App, AppOptions, Effect};
    use crate::commands;
    use crate::runtime::Runtime;

    fn here() -> Place {
        Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
    }

    fn started() -> (App, Runtime) {
        let mut options = AppOptions::new(here(), here().join("src").unwrap());
        options.clock = Arc::new(FixedClock::new(
            Timestamp::from_civil(2026, 11, 12, 10, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        ));
        options.settings.dialog_settle_ms = 0;
        let mut app = App::new(options);
        let mut runtime = Runtime::with_local();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        (app, runtime)
    }

    /// What a command says from the panels, every dialog closed first.
    fn said(app: &mut App, runtime: &mut Runtime, id: &CommandId) -> Vec<String> {
        while !app.dialogs.is_empty() {
            let effects = app.dialog_escape();
            runtime.run_until_idle(app, effects);
        }
        let effects = app.dispatch(&Command::new(id.clone()));
        runtime
            .run_until_idle(app, effects)
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Announce(message) => Some(message.text),
                _ => None,
            })
            .collect()
    }

    fn announced(effects: Vec<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Announce(message) => Some(message.text),
                _ => None,
            })
            .collect()
    }

    /// The general results lister: any list opens in it, says it replaces
    /// a search's results once a session, shows the list's own words after
    /// a name and "missing" for a gone item, and in a collection the trash
    /// key goes to lane LA's removal, never to the trash.
    #[test]
    fn the_general_lister_opens_any_list_in_the_results_lister() {
        use diroxide_core::{Entry, EntryKind, Key, Side};
        use diroxide_search::SearchHit;

        use crate::app::searching::{ListKind, ListOpen, REPLACES_RESULTS};

        let (mut app, mut runtime) = started();
        let file = here().join("Cargo.toml").unwrap();
        let gone = here().join("gone.txt").unwrap();
        let open = |kind: ListKind, start: &str| ListOpen {
            kind,
            side: Side::Left,
            title: "Trip photos, collection".to_owned(),
            root: Some(here()),
            hits: vec![SearchHit::new(
                file.clone(),
                Entry::new("Cargo.toml", EntryKind::File),
            )],
            gone: vec![gone.clone()],
            notes: vec![(file.clone(), "oldest".to_owned())],
            start: start.to_owned(),
            streaming: false,
        };
        let (effects, _) = app.open_list(open(ListKind::Search, "2 found"));
        assert_eq!(
            announced(runtime.run_until_idle(&mut app, effects)),
            ["2 found"]
        );
        let effects = app.dialog_escape();
        runtime.run_until_idle(&mut app, effects);

        let start = "Trip photos, collection, 2 items";
        assert_eq!(start.chars().count(), 32);
        let kind = ListKind::Collection("Trip photos".to_owned());
        let (effects, _) = app.open_list(open(kind.clone(), start));
        let said = announced(runtime.run_until_idle(&mut app, effects));
        assert_eq!(said, [start, REPLACES_RESULTS]);
        assert_eq!(REPLACES_RESULTS.chars().count(), 28);
        assert_eq!(app.list_kind(), Some(&kind));
        assert_eq!(app.list_owner().map(|(side, _)| side), Some(Side::Left));
        let rows = match app.dialogs.last() {
            Some((_, crate::dialog::Dialog::Choose { list, .. })) => list.rows().to_vec(),
            _ => panic!("the results lister is not open"),
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].details.first().map(String::as_str), Some("oldest"));
        assert_eq!(rows[1].details.first().map(String::as_str), Some("missing"));
        // The trash key in a collection: lane LA's removal, not the trash.
        let effects = app.handle(crate::app::AppEvent::Key(Key::parse("delete").unwrap()));
        let said = announced(runtime.run_until_idle(&mut app, effects));
        assert_eq!(said, ["Lists need a state folder, and this run has none."]);
        let effects = app.dialog_escape();
        runtime.run_until_idle(&mut app, effects);

        // Said once a session only.
        let (effects, _) = app.open_list(open(ListKind::Search, "2 found"));
        runtime.run_until_idle(&mut app, effects);
        let effects = app.dialog_escape();
        runtime.run_until_idle(&mut app, effects);
        let (effects, _) = app.open_list(open(ListKind::Flat, "Flat view on, 2 files"));
        let said = announced(runtime.run_until_idle(&mut app, effects));
        assert_eq!(said, ["Flat view on, 2 files"]);
        let effects = app.list_remove_places(std::slice::from_ref(&file));
        runtime.run_until_idle(&mut app, effects);
        assert!(ListKind::Collection(String::new()).trash_removes_item());
        assert!(!ListKind::Flat.trash_removes_item());
        assert_eq!(kind.words(), "the collection Trip photos");
    }

    #[test]
    fn every_reserved_command_is_in_the_catalog_and_answers() {
        let (mut app, mut runtime) = started();
        for (id, lane) in &RESERVED {
            let info = commands::info(id).unwrap_or_else(|| panic!("{id} ({lane}) not in CATALOG"));
            assert!(
                info.terminal.is_empty() && info.gui.is_empty(),
                "{id} has a key"
            );
            let said = said(&mut app, &mut runtime, id);
            assert!(!said.is_empty(), "{id} ({lane}) said nothing");
            for line in &said {
                if line.starts_with(NOT_BUILT) {
                    // "Not built yet: Copy toward other panel." is the
                    // longest, 39 cells.
                    assert_eq!(*line, format!("{NOT_BUILT}{}.", info.name));
                    assert!(line.chars().count() <= 40, "{line}: over 40 cells");
                }
            }
        }
    }

    /// On since the four app lanes of alpha 5 merged: every lane has
    /// filled its arms.
    #[test]
    fn no_reserved_command_is_left_unbuilt() {
        let (mut app, mut runtime) = started();
        let left: Vec<String> = RESERVED
            .iter()
            .filter(|(id, _)| {
                said(&mut app, &mut runtime, id)
                    .iter()
                    .any(|line| line.starts_with(NOT_BUILT))
            })
            .map(|(id, lane)| format!("{id} ({lane})"))
            .collect();
        assert!(left.is_empty(), "not built yet: {left:?}");
    }
}
