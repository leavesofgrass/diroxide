//! The window frontend: Masonry, `masonry_winit`, Vello, Parley and
//! AccessKit, vendored from textweaver at the same revision and patches.
//! Each panel is an AccessKit grid with real row and cell nodes, named
//! columns, a selection state and a focus the screen reader follows; menus
//! are native; announcements go through a live region; dialogs are modal,
//! named, close on Escape and return the focus.
//!
//! Phase 0: [`panel_grid`] computes the grid the accessibility tree will
//! carry, without AccessKit; [`run`] says the window is not built yet.
//! Wave 4 (W4a and W4b) owns this crate.

use diroxide_app::{App, Panel, Runtime};
use diroxide_core::Explain;
use thiserror::Error;

/// Options of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiOptions {
    /// The window's title.
    pub title: String,
}

impl Default for GuiOptions {
    fn default() -> Self {
        Self {
            title: "diroxide".to_owned(),
        }
    }
}

/// Why the window could not run.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GuiError {
    /// The window frontend is not built yet.
    #[error("the window is not built yet")]
    NotBuilt,
}

impl Explain for GuiError {
    fn next_step(&self) -> String {
        "Use the terminal: run diro with a folder.".to_owned()
    }
}

/// A row of a panel's grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRow {
    /// The cells: the name first, then the details.
    pub cells: Vec<String>,
    /// Whether the row is selected.
    pub selected: bool,
}

/// A panel as the accessibility tree will carry it: a named grid with
/// named columns, rows of cells, a selection and a focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridModel {
    /// The grid's name: the panel's title.
    pub name: String,
    /// The column headers.
    pub columns: Vec<String>,
    /// The rows.
    pub rows: Vec<GridRow>,
    /// The focused row.
    pub focus: Option<usize>,
}

/// The grid for a panel. Phase 0 names the columns "Name" and "Details";
/// W4a names one column per configured detail.
pub fn panel_grid(panel: &Panel) -> GridModel {
    let list = panel.list();
    GridModel {
        name: list.title().to_owned(),
        columns: vec!["Name".to_owned(), "Details".to_owned()],
        rows: list
            .rows()
            .iter()
            .map(|row| GridRow {
                cells: vec![row.label.clone(), row.details.join(", ")],
                selected: row.selected,
            })
            .collect(),
        focus: list.focus_index(),
    }
}

/// Runs the window. Phase 0: not built.
pub fn run(app: &mut App, runtime: &mut Runtime, options: &GuiOptions) -> Result<(), GuiError> {
    let _ = (app, runtime, options);
    Err(GuiError::NotBuilt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_app::AppOptions;
    use diroxide_core::{Place, Side};

    #[test]
    fn a_panel_becomes_a_named_grid() {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let mut app = App::new(AppOptions::new(here.clone(), here));
        let mut runtime = Runtime::with_local();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        let grid = panel_grid(app.panel(Side::Left));
        assert!(grid.name.starts_with("diroxide-gui, in "));
        assert_eq!(grid.rows[0].cells[0], "src");
        assert_eq!(grid.focus, Some(0));
        let err = run(&mut app, &mut runtime, &GuiOptions::default()).unwrap_err();
        assert_eq!(err.to_string(), "the window is not built yet");
    }
}
