//! The palette's recently used ranking, driven with the fake clock: the
//! last command used is first for an empty query, and each use counts a
//! fifth less every day after it (Worker's aging, 0.8 a day).

use diroxide_app::Dialog;
use diroxide_config::Paths;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, fixture, place, temp};

const DAY: i64 = 86_400;

fn run_from_palette(s: &mut Session, name: &str) {
    s.key("ctrl+p");
    s.type_text(name);
    s.key("enter");
}

fn first_rows(s: &Session, how_many: usize) -> Vec<String> {
    let Some(Dialog::Edit {
        candidates: Some(list),
        ..
    }) = s.dialog()
    else {
        panic!("{:?}", s.dialog())
    };
    list.rows()[..how_many]
        .iter()
        .map(|r| r.label.clone())
        .collect()
}

#[test]
fn the_last_command_used_is_first_and_older_uses_count_less() {
    let dir = temp();
    let work = fixture(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let mut s = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup {
            paths: Some(paths.clone()),
            ..Setup::default()
        },
    );
    // Sort by size three times today, Next theme twice in four days, then
    // Swap panels once a day after that.
    for _ in 0..3 {
        run_from_palette(&mut s, "sort by size");
    }
    s.clock.advance(4 * DAY);
    for _ in 0..2 {
        run_from_palette(&mut s, "next theme");
    }
    s.clock.advance(DAY);
    run_from_palette(&mut s, "swap panels");
    s.clock.advance(DAY);
    s.key("ctrl+p");
    // Swap panels was last. Next theme's two uses, a day old (1.6), beat
    // Sort by size's three, five days old (0.98).
    assert_eq!(
        first_rows(&s, 3),
        ["Swap panels", "Next theme", "Sort by size"]
    );
    s.key("escape");
    // The uses are kept between runs, with their times.
    let kept = diroxide_app::state::load_history(&paths, diroxide_app::state::keys::PALETTE);
    assert_eq!(kept.len(), 3);
    assert!(kept[0].starts_with("panel.swap 1.0000 "), "{kept:?}");
    assert!(kept[1].starts_with("app.next-theme 2.0000 "), "{kept:?}");
    let again = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup {
            paths: Some(paths),
            ..Setup::default()
        },
    );
    let mut again = again;
    again.key("ctrl+p");
    assert_eq!(
        first_rows(&again, 3),
        ["Swap panels", "Next theme", "Sort by size"]
    );
}
