//! The custom columns in the app (alpha 4, lane MA, R30 commit 52; R28
//! 3.2; the model and its words are in [`crate::columns`]).
//!
//! - **The columns are resolved** when the app starts, when the
//!   `columns` or `regex_columns` setting changes (at once now, the panels
//!   included), and when a folder's own view brings its columns: each key
//!   to the metadata reader that names it, the owner's own field, a regex
//!   column or a plugin's column.
//! - **The readers follow the focus**, as the type check's do, and share
//!   its rules (four readers on a disk of this computer, one on a share,
//!   the rows on screen first, nothing read on a remote place): after
//!   each event, a panel whose entries changed, or whose focus moved more
//!   than half a page from where its read began, asks again from there;
//!   the readers of the old order stop. A fast column reads the whole
//!   folder; a background column the visible page and two pages either
//!   side.
//! - **Cells reach the rows quietly**: a batch that touches the page
//!   around the focus is put into the rows at once and the panel drawn
//!   again, never announced; the others wait until the focus comes near
//!   or the reading ends (spike M6).
//! - **The ask-first sort**: sorting by a column whose cells are not all
//!   read reads them first, asking when more than [`ASK_ABOVE`] files
//!   wait ("Read 12,000 files to sort by Artist?", then "About 40
//!   seconds.", the answer starting on No); the order changes only when
//!   the reading ends, and then it is said: "Sorted by artist, A to Z".
//! - **Fill columns** reads every column's cells of the folder, the
//!   on-demand ones too, asking first the same way.

use std::sync::Arc;

use diroxide_core::{Command, Effect, ListRow, Message, Side, SortKey};
use diroxide_meta::{Cost, Guard, Pipeline};

use super::App;
use super::organize::Wants;
use crate::columns::{
    ASK_ABOVE, ColumnCell, FieldColumn, fill_question, resolve, sort_estimate, sort_question,
};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::request::{Request, SharedPipeline};

/// The row id prefix of a column's row in the Sort list.
pub(super) const SORT_ROW: &str = "column:";

/// The most readers the columns' guard runs at once: four for each panel.
const COLUMN_READERS: usize = 8;

/// What the app asked of each panel's column readers.
#[derive(Clone, Debug, Default)]
pub(super) struct ColumnsState {
    asked: [Option<Asked>; 2],
    /// The pipeline the columns read through, with the runner a test gave
    /// when it was made (`None`: the tools registry).
    pipeline: Option<(Option<crate::request::SharedTools>, Arc<Pipeline>)>,
}

/// One panel's read as last asked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Asked {
    /// The panel's entries it was asked for.
    listing: u64,
    /// The focused row when it was asked.
    focus: usize,
    /// The files still to be answered.
    left: usize,
    /// A read the owner asked for, which the focus does not move.
    fill: Option<Fill>,
}

/// A read the owner asked for: Fill columns, or the reading before a
/// sort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Fill {
    /// The column to sort by when the reading ends, with its label; `None`
    /// for Fill columns.
    pub sort: Option<(String, String)>,
    /// How many files it reads.
    pub files: usize,
}

impl App {
    /// Fill columns; `None` for commands that are not the columns'.
    pub(super) fn columns_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::FILL_COLUMNS {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.fill_columns())
    }

    /// The pipeline the columns read through: the details' readers, the
    /// tools through the app's runner, under a guard of their own.
    pub(super) fn column_pipeline(&mut self) -> Arc<Pipeline> {
        let given = self.preview.runner.clone();
        if let Some((made_for, pipeline)) = &self.columns.pipeline
            && *made_for == given
        {
            return Arc::clone(pipeline);
        }
        let tools = self.tool_runner();
        let pipeline =
            Arc::new(crate::details::pipeline(&tools).with_guard(Guard::new(COLUMN_READERS)));
        self.columns.pipeline = Some((given, Arc::clone(&pipeline)));
        pipeline
    }

    /// The plugins' declared columns, for the keys `PLUGIN.COLUMN`.
    pub(super) fn plugin_columns(&self) -> Vec<diroxide_plugins::listers::PluginColumn> {
        Vec::new()
    }

    /// Resolves both panels' columns of fields' keys; their reads start
    /// again.
    pub(super) fn resolve_columns(&mut self) {
        let pipeline = self.column_pipeline();
        let keys = pipeline.keys();
        let plugins = self.plugin_columns();
        let clock = Arc::clone(&self.clock);
        for side in Side::BOTH {
            let specs = self.panel(side).columns().to_vec();
            let fields = resolve(&specs, &keys, &self.settings.regex_columns, &plugins);
            self.panels[side.index()].set_field_columns(fields, clock.as_ref());
            self.columns.asked[side.index()] = None;
        }
    }

    /// The `columns` or `regex_columns` setting changed: both panels take
    /// the new columns at once.
    pub(super) fn columns_setting_changed(&mut self) {
        let specs = self.settings.column_specs();
        let clock = Arc::clone(&self.clock);
        for side in Side::BOTH {
            self.panels[side.index()].set_columns(specs.clone(), clock.as_ref());
        }
        self.resolve_columns();
    }

    /// After each event: rows waiting for cells near the focus take them,
    /// and a panel whose entries changed, or whose focus moved more than
    /// half a page from where its read began, asks its readers again.
    pub(super) fn follow_columns(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for side in Side::BOTH {
            let panel = self.panel(side);
            if panel.is_loading() || !panel.is_listed() {
                continue;
            }
            if panel.has_stale_rows() && panel.stale_near_focus() {
                let clock = Arc::clone(&self.clock);
                self.panels[side.index()].put_stale_rows(clock.as_ref());
                if side == self.active || !self.screen_reader {
                    effects.push(self.redraw_panel(side));
                }
            }
            let panel = self.panel(side);
            let reads = panel
                .field_columns()
                .iter()
                .any(|f| matches!(f.cost(), Some(Cost::Fast | Cost::Background)));
            let listing = panel.entries_generation();
            let focus = panel.list().focus_index().unwrap_or(0);
            let half_page = (panel.list().page_size() / 2).max(1);
            if let Some(asked) = &self.columns.asked[side.index()]
                && asked.listing == listing
                && ((asked.fill.is_some() && asked.left > 0)
                    || focus.abs_diff(asked.focus) <= half_page)
            {
                continue;
            }
            if !reads {
                continue;
            }
            self.ask_columns(side, None);
        }
        effects
    }

    /// Asks the runtime to read a panel's cells: without a fill, the
    /// window around the focus for fast and background columns, then the
    /// rest of the folder for fast ones; with one, every file the fill's
    /// columns wait for. With nothing left, stops a read still running.
    /// Returns how many files it reads.
    fn ask_columns(&mut self, side: Side, fill: Option<Fill>) -> usize {
        let types = Arc::clone(&self.types);
        let panel = self.panel(side);
        let (items, urgent) = match &fill {
            None => panel.column_queue(
                &|f: &FieldColumn| matches!(f.cost(), Some(Cost::Fast | Cost::Background)),
                &|f: &FieldColumn| f.cost() == Some(Cost::Fast),
                &types,
            ),
            Some(Fill {
                sort: Some((key, _)),
                ..
            }) => {
                let key = key.clone();
                panel.column_queue(&|_| false, &move |f: &FieldColumn| f.key == key, &types)
            }
            Some(Fill { sort: None, .. }) => panel.column_queue(&|_| false, &|_| true, &types),
        };
        let listing = panel.entries_generation();
        let focus = panel.list().focus_index().unwrap_or(0);
        let folder = panel.place().clone();
        let costs: Vec<(String, Cost)> = panel
            .field_columns()
            .iter()
            .filter_map(|f| f.cost().map(|c| (f.key.clone(), c)))
            .collect();
        let running = self.columns.asked[side.index()]
            .as_ref()
            .is_some_and(|a| a.left > 0);
        let files = items.len();
        self.columns.asked[side.index()] = Some(Asked {
            listing,
            focus,
            left: files,
            fill: fill.map(|f| Fill { files, ..f }),
        });
        if items.is_empty() {
            if running {
                self.request(Request::StopColumns(side));
            }
            return 0;
        }
        let pipeline = SharedPipeline(self.column_pipeline());
        self.request(Request::ReadColumns {
            side,
            listing,
            folder,
            items,
            urgent,
            costs,
            pipeline,
        });
        files
    }

    /// A batch of cells: the entries take them, and the rows near the
    /// focus are drawn again; answers for entries replaced since are
    /// dropped. When the read ends, the rows still waiting take their
    /// cells, and a sort waiting for it sorts.
    pub(super) fn columns_read(
        &mut self,
        side: Side,
        listing: u64,
        cells: Vec<ColumnCell>,
    ) -> Vec<Effect> {
        if self.panel(side).entries_generation() != listing {
            return Vec::new();
        }
        let count = cells.len();
        let clock = Arc::clone(&self.clock);
        let near = self.panels[side.index()].apply_column_cells(cells, clock.as_ref());
        let shown = side == self.active || !self.screen_reader;
        let mut effects = Vec::new();
        let mut ended = None;
        if let Some(asked) = self.columns.asked[side.index()].as_mut()
            && asked.listing == listing
            && asked.left > 0
        {
            asked.left = asked.left.saturating_sub(count);
            if asked.left == 0 {
                ended = Some(asked.fill.clone());
            }
        }
        if let Some(fill) = ended {
            self.panels[side.index()].put_stale_rows(clock.as_ref());
            if let Some(fill) = fill {
                effects.extend(self.fill_ended(side, fill));
            }
            if shown {
                effects.insert(0, self.redraw_panel(side));
            }
        } else if near && shown {
            effects.push(self.redraw_panel(side));
        }
        effects
    }

    /// A read the owner asked for ended: the sort it waited for, or Fill
    /// columns' line.
    fn fill_ended(&mut self, side: Side, fill: Fill) -> Vec<Effect> {
        match fill.sort {
            Some((key, label)) => self.sort_by_column_now(side, &key, &label),
            None => {
                let line = format!(
                    "Columns read: {}",
                    diroxide_core::format::count(fill.files as u64, "file", "files")
                );
                vec![self.say(Message::info(line))]
            }
        }
    }

    /// The Sort list's rows for the columns of fields' keys: "Artist",
    /// the current one marked as the built-in rows are.
    pub(super) fn column_sort_rows(&self, side: Side) -> Vec<ListRow> {
        let panel = self.panel(side);
        let sort = panel.sort();
        panel
            .field_columns()
            .iter()
            .map(|f| {
                let row = ListRow::new(format!("{SORT_ROW}{}", f.key), f.label.clone());
                match &sort.key {
                    SortKey::Extra(key) if *key == f.key => {
                        let direction = sort.key.direction_words(sort.direction);
                        row.with_details(["current".to_owned(), direction.to_owned()])
                    }
                    _ => row,
                }
            })
            .collect()
    }

    /// Sort by a column: at once when its cells are read; otherwise they
    /// are read first, asking when more than [`ASK_ABOVE`] files wait,
    /// and the order changes when the reading ends.
    pub(super) fn sort_by_column(&mut self, key: &str) -> Vec<Effect> {
        let side = self.active;
        let Some(field) = self
            .panel(side)
            .field_columns()
            .iter()
            .find(|f| f.key == key)
            .cloned()
        else {
            return vec![self.say(Message::info("That column is not shown here."))];
        };
        let waiting = self.panel(side).waiting_for(key);
        if waiting == 0 {
            return self.sort_by_column_now(side, key, &field.label);
        }
        let fill = Fill {
            sort: Some((key.to_owned(), field.label.clone())),
            files: waiting,
        };
        if waiting <= ASK_ABOVE {
            return self.start_fill(side, fill);
        }
        self.ask_fill(side, fill)
    }

    /// Fill columns: every column's cells of the folder, the on-demand
    /// ones too, asking first when more than [`ASK_ABOVE`] files wait.
    pub(super) fn fill_columns(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        if !panel.field_columns().iter().any(|f| f.cost().is_some()) {
            return vec![self.say(Message::info(
                "No column here is read from the files. The columns setting adds one.",
            ))];
        }
        if !panel.reads_contents() {
            return vec![self.say(Message::info(
                "Columns are not read here: only on a disk of this computer.",
            ))];
        }
        let waiting = panel
            .all_entries()
            .iter()
            .filter(|e| {
                panel
                    .field_columns()
                    .iter()
                    .any(|f| f.cost().is_some() && f.waits(e))
            })
            .count();
        if waiting == 0 {
            return vec![self.say(Message::info("Every column here is read."))];
        }
        let fill = Fill {
            sort: None,
            files: waiting,
        };
        if waiting <= ASK_ABOVE {
            return self.start_fill(side, fill);
        }
        self.ask_fill(side, fill)
    }

    /// The question before a big read, starting on No: "Read 12,000 files
    /// to sort by Artist? About 40 seconds."
    fn ask_fill(&mut self, side: Side, fill: Fill) -> Vec<Effect> {
        let share = self.panel(side).place().to_string().starts_with(r"\\");
        let question = match &fill.sort {
            Some((_, label)) => sort_question(fill.files, label),
            None => fill_question(fill.files),
        };
        // The estimate is said with the question, after its meaning.
        let mut question = with_default_no(Message::question(&question));
        question.text = question.text.replacen(
            "? No is the default.",
            &format!("? {} No is the default.", sort_estimate(fill.files, share)),
            1,
        );
        let listing = vec![format!(
            "In {}",
            crate::panel::place_title(self.panel(side).place())
        )];
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::FILL_COLUMNS),
            },
            Wants::ReadColumns { side, fill },
        )
    }

    /// Yes to the question: the read starts; its end sorts or says so.
    pub(super) fn start_fill(&mut self, side: Side, fill: Fill) -> Vec<Effect> {
        let label = fill.sort.as_ref().map(|(_, label)| label.clone());
        let files = self.ask_columns(side, Some(fill.clone()));
        if files == 0 {
            return self.fill_ended(side, fill);
        }
        let what = diroxide_core::format::count(files as u64, "file", "files");
        let line = match label {
            Some(label) => format!("Reading {what} to sort by {label}"),
            None => format!("Reading {what} for the columns"),
        };
        vec![self.say(Message::status(line))]
    }

    /// The sort by a column whose cells are read: "Sorted by artist, A to
    /// Z".
    fn sort_by_column_now(&mut self, side: Side, key: &str, label: &str) -> Vec<Effect> {
        let spec = self
            .panel(side)
            .sort()
            .choose(SortKey::Extra(key.to_owned()));
        let clock = Arc::clone(&self.clock);
        self.panels[side.index()].set_sort(spec.clone(), clock.as_ref());
        let text = format!(
            "Sorted by {}, {}",
            crate::labels::in_sentence(label),
            spec.key.direction_words(spec.direction)
        );
        vec![self.redraw_panel(side), self.say(Message::info(text))]
    }
}
