//! The `layout` setting's three arrangements (R27's commit 10): side by
//! side and stacked show both panels; single shows the active one alone,
//! the other out of the tree until Tab brings it.

#![allow(clippy::unwrap_used)]

use diroxide_app::config::Layout;
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{Place, Side};
use masonry::accesskit::Role;

use crate::rig::{Rig, fixture};

fn rig_with(layout: Layout, dir: &std::path::Path) -> Rig {
    let place = Place::parse(dir.to_str().unwrap()).unwrap();
    let mut options = AppOptions::new(place.clone(), place);
    options.settings.layout = layout;
    options.settings.dialog_settle_ms = 0;
    Rig::new(App::new(options), Runtime::with_local())
}

fn lists(rig: &Rig) -> Vec<String> {
    rig.walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBox)
        .map(|(_, name)| name)
        .collect()
}

#[test]
fn side_by_side_and_stacked_show_both_panels() {
    let dir = fixture(&["a.txt"], &[]);
    for layout in [Layout::SideBySide, Layout::Stacked] {
        let rig = rig_with(layout, dir.path());
        assert_eq!(lists(&rig).len(), 2, "{layout:?}");
        // Each list keeps its rows: a page is more than one row.
        assert!(rig.f.app().panel(Side::Left).list().page_size() > 1);
    }
}

#[test]
fn single_shows_the_active_panel_alone_and_tab_brings_the_other() {
    let dir = fixture(&["a.txt"], &["Backup"]);
    let mut rig = rig_with(Layout::Single, dir.path());
    let shown = lists(&rig);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].starts_with("Left panel"), "{shown:?}");
    rig.press("tab");
    let shown = lists(&rig);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].starts_with("Right panel"), "{shown:?}");
    assert_eq!(rig.h.focused_widget_id(), Some(rig.panel_id(Side::Right)));
}
