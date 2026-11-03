//! The quick view (R27's commit 23): Shift+F3 turns the other panel into
//! the preview of the file under the cursor, the same list box as a
//! panel; Tab goes into its region and back; the links and find are the
//! app's dialogs.

#![allow(clippy::unwrap_used)]

use diroxide_core::Side;
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture};

/// The list box named with `prefix`: its name and options.
fn list_named(rig: &Rig, prefix: &str) -> Option<(String, Vec<String>)> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    while let Some(node) = stack.pop() {
        if node.is_hidden() {
            continue;
        }
        let name = crate::rig::name_of(&node);
        if node.role() == Role::ListBox && name.starts_with(prefix) {
            let options = node
                .children()
                .filter(|o| o.role() == Role::ListBoxOption)
                .map(|o| crate::rig::name_of(&o))
                .collect();
            return Some((name, options));
        }
        stack.extend(node.children());
    }
    None
}

const NOTES: &str = "# Crows\n\nCrows remember faces.\n\n## Jimothy\n\nA raccoon of Ballard. See [the wiki](https://example.org/jimothy).\n\n## Portland\n\nKeep it weird.\n";

#[test]
fn shift_f3_on_a_markdown_file_previews_it_in_the_other_panel() {
    let dir = fixture(&["a.txt"], &[]);
    std::fs::write(dir.path().join("notes.md"), NOTES).unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.md"));
    rig.refresh();
    rig.press("shift+f3");
    assert!(rig.f.app().quick_view_on(), "{:?}", rig.spoken());
    // The other panel is the preview, a list box of the region's rows.
    assert!(
        rig.settle_until(|rig| list_named(rig, "Right panel, Preview, notes.md")
            .is_some_and(|(_, rows)| rows.iter().any(|r| r.contains("Jimothy")))),
        "{:?}",
        rig.walk()
    );
    // The focus stays on the file's row until Tab goes in.
    let (_, row) = rig.focus_node();
    assert!(row.starts_with("notes.md"), "{row}");
    rig.press("tab");
    assert!(rig.f.app().in_preview());
    let (role, first) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(first.contains("Crows"), "{first}");
    let (list, _) = rig.focus_list().unwrap();
    assert!(list.starts_with("Right panel, Preview"), "{list}");
    // The region's own keys: the next heading.
    rig.run("preview.next-heading");
    let (_, heading) = rig.focus_node();
    assert!(heading.contains("Jimothy"), "{heading}");
    // The links, a dialog; Escape back to the region.
    rig.run("preview.links");
    let links = rig.dialog_name().unwrap();
    assert!(links.contains("Links") || links.contains("link"), "{links}");
    assert!(
        rig.focus_node().1.contains("wiki"),
        "{:?}",
        rig.focus_node()
    );
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    assert!(rig.f.app().in_preview());
    // Find, a field.
    rig.run("preview.find");
    assert!(rig.dialog_name().is_some());
    rig.type_text("weird");
    rig.press("enter");
    assert!(
        rig.settle_until(|rig| rig.focus_node().1.contains("weird")),
        "{:?}",
        rig.focus_node()
    );
    // Tab: back to the panel's row; the preview stays.
    rig.press("tab");
    assert!(!rig.f.app().in_preview());
    let (_, row) = rig.focus_node();
    assert!(row.starts_with("notes.md"), "{row}");
    // A click on the preview goes into it; Escape comes back.
    let (_, rows) = list_named(&rig, "Right panel, Preview").unwrap();
    rig.click(Role::ListBoxOption, &rows[0], 1);
    assert!(rig.f.app().in_preview(), "{:?}", rig.spoken());
    rig.press("escape");
    assert!(!rig.f.app().in_preview());
    // Shift+F3 again: the panel is a folder's again.
    rig.press("shift+f3");
    assert!(!rig.f.app().quick_view_on());
    assert!(list_named(&rig, "Right panel, Preview").is_none());
}
