//! The flat view in the app (alpha 5, lane LA): every file below a
//! folder in one list, streamed into the general results lister
//! (`App::open_list` with `ListKind::Flat`), over
//! `diroxide_search::flat_view`.
//!
//! Flat view (View menu, palette; no key, question 131) opens a short
//! form of its settings as rows, Start first: the depth, files only, and
//! hidden files as the panel shows them. Start opens the lister ("Flat
//! view of work: listing files only"), the rows arrive as a search's do,
//! and the end is said: "Flat view on, 14 files in 4 folders." Escape
//! while it lists stops it: "Stopped at 4,200 entries." Flat view again
//! while the list is open turns it off: "Flat view off. work."

use diroxide_core::{Command, JobKind, JobRequest, ListModel, ListRow, Message, Place, Side};
use diroxide_search::flat::root_name;
use diroxide_search::{FlatOptions, FlatSummary, flat_off_words};

use super::collections::ListsWants;
use super::organize::Wants;
use super::searching::{ListKind, ListOpen};
use super::{App, Effect, Flow, Purpose, Then};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::lists::ListsAsk;
use crate::request::Request;

/// The question before copy or move out of a flat view: 35 cells.
pub(super) const KEEP_OR_ONE: &str = "Keep folders, or all in one folder?";

/// Said when copy is asked to keep folders: 39 cells.
pub(super) const KEEP_COPY_NOT_BUILT: &str = "Keep folders is for Move only, for now.";

/// Said when a kept folder would be two levels or more down: 40 cells.
pub(super) const KEEP_ONE_LEVEL: &str = "Keep folders: one folder level, for now.";

/// Its rows, the safer answer first.
const KEEP: &str = "keep";
const ONE: &str = "one";

/// Copy or move out of a flat view, waiting on the question (lane AS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlatCopy {
    /// The panel that asked.
    pub side: Side,
    /// Copy or move.
    pub kind: JobKind,
    /// The flat view's folder: the folders kept start below it.
    pub root: Place,
    /// The other panel's folder.
    pub destination: Place,
    /// The marked results, or the focused one.
    pub sources: Vec<Place>,
    /// Whether the plan is shown before it runs, as the settings say.
    pub confirm: bool,
}

impl FlatCopy {
    /// "Nothing copied." or "Nothing moved."
    pub(super) fn nothing_done(&self) -> &'static str {
        if self.kind == JobKind::Move {
            "Nothing moved."
        } else {
            "Nothing copied."
        }
    }

    /// Where a source goes when its folders are kept: the destination
    /// with the folders between the root and the source; `None` when the
    /// source is not below the root.
    fn kept_folder(&self, source: &Place) -> Option<(Place, usize)> {
        let mut names = Vec::new();
        let mut folder = source.parent()?;
        while folder != self.root {
            names.push(folder.name()?.to_owned());
            folder = folder.parent()?;
        }
        let mut kept = self.destination.clone();
        for name in names.iter().rev() {
            kept = kept.join(name).ok()?;
        }
        Some((kept, names.len()))
    }
}

/// The form's rows, by id.
const START: &str = "start";
const DEPTH: &str = "depth";
const FILES_ONLY: &str = "files-only";
const HIDDEN: &str = "hidden";

/// The depth in words for its row: "every level", "1 level", "3
/// levels".
fn depth_words(options: &FlatOptions) -> String {
    match options.depth {
        None => "every level".to_owned(),
        Some(1) => "1 level".to_owned(),
        Some(n) => format!("{n} levels"),
    }
}

/// The flat view's settings as a list, Start first: "Flat view of work".
pub(super) fn form_model(root: &Place, options: &FlatOptions) -> ListModel {
    let files_only = if options.folders {
        "Files only: no, folders too"
    } else {
        "Files only: yes"
    };
    let hidden = if options.hidden {
        "Hidden files: listed"
    } else {
        "Hidden files: left out"
    };
    let rows = vec![
        ListRow::new(START, "Start the flat view"),
        ListRow::new(DEPTH, format!("Depth: {}", depth_words(options))),
        ListRow::new(FILES_ONLY, files_only),
        ListRow::new(HIDDEN, hidden),
    ];
    let mut list = ListModel::new(format!("Flat view of {}", root_name(root))).with_rows(rows);
    list.set_help("Enter starts, or changes the setting on its row. Escape closes.");
    list
}

impl App {
    /// Flat view: the settings form for this panel's folder, or off when
    /// the flat view's list is open.
    pub(super) fn flat_view_toggle(&mut self, _command: &Command) -> Vec<Effect> {
        let lister_open = self
            .opens
            .iter()
            .any(|open| matches!(open.purpose, Purpose::Results));
        if lister_open && self.list_kind() == Some(&ListKind::Flat) {
            let side = self.list_owner().map_or(self.active, |(side, _)| side);
            let root = self.panel(side).place().clone();
            self.request(Request::StopSearch);
            let mut effects = self.close_all();
            effects.push(self.say(Message::info(flat_off_words(&root))));
            return effects;
        }
        let side = self.active;
        let root = self.panel(side).place().clone();
        let options = FlatOptions::for_panel(self.panel(side).shows_hidden());
        self.flat_form(side, root, options, None)
    }

    /// Opens the settings form, its focus on a row when one is named.
    fn flat_form(
        &mut self,
        side: Side,
        root: Place,
        options: FlatOptions,
        focus: Option<&str>,
    ) -> Vec<Effect> {
        let mut list = form_model(&root, &options);
        if let Some(id) = focus {
            list.focus_id(id);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::FLAT_VIEW),
            },
            Wants::Lists(ListsWants::FlatForm {
                side,
                root,
                options,
            }),
        )
    }

    /// Enter on a row of the form: Start, the depth field, or a setting
    /// changed and said as the form opens again on it.
    pub(super) fn flat_form_row(
        &mut self,
        side: Side,
        root: Place,
        options: FlatOptions,
        row: &str,
    ) -> Vec<Effect> {
        let mut effects = self.close_all();
        match row {
            START => effects.extend(self.start_flat(side, root, options)),
            DEPTH => {
                let text = options.depth.map(|d| d.to_string()).unwrap_or_default();
                let field =
                    EntryField::new("Depth, empty or 0 for every level", CompletionSource::None)
                        .with_text(text);
                effects.extend(self.open_organize(
                    Dialog::edit(field),
                    Wants::Lists(ListsWants::FlatDepth {
                        side,
                        root,
                        options,
                    }),
                ));
            }
            FILES_ONLY => {
                let folders = !options.folders;
                let options = options.with_folders(folders);
                effects.extend(self.flat_form(side, root, options, Some(FILES_ONLY)));
            }
            HIDDEN => {
                let hidden = !options.hidden;
                let options = options.with_hidden(hidden);
                effects.extend(self.flat_form(side, root, options, Some(HIDDEN)));
            }
            _ => {}
        }
        effects
    }

    /// The depth typed: a number of levels, or every level; the form
    /// opens again on its row.
    pub(super) fn flat_depth_typed(
        &mut self,
        side: Side,
        root: Place,
        options: FlatOptions,
        text: &str,
    ) -> Vec<Effect> {
        let text = text.trim();
        let depth = if text.is_empty() {
            Some(0)
        } else {
            text.parse::<u32>().ok()
        };
        let Some(depth) = depth else {
            return vec![self.say(Message::error(
                "Depth is a number of levels: 1 for this folder's files.",
            ))];
        };
        let mut effects = self.close_all();
        let options = options.with_depth(Some(depth));
        effects.extend(self.flat_form(side, root, options, Some(DEPTH)));
        effects
    }

    /// The form again, on the depth row, after its field closed.
    pub(super) fn flat_form_again(
        &mut self,
        side: Side,
        root: Place,
        options: FlatOptions,
    ) -> Vec<Effect> {
        self.flat_form(side, root, options, Some(DEPTH))
    }

    /// Starts the flat view: the lister opens empty and streams.
    fn start_flat(&mut self, side: Side, root: Place, options: FlatOptions) -> Vec<Effect> {
        let name = root_name(&root);
        let open = ListOpen {
            kind: ListKind::Flat,
            side,
            title: format!("Flat view of {name}"),
            root: Some(root.clone()),
            hits: Vec::new(),
            gone: Vec::new(),
            notes: Vec::new(),
            start: format!("Flat view of {name}: listing {}", options.mode_words()),
            streaming: true,
        };
        let (effects, list) = self.open_list(open);
        self.lists_ask(ListsAsk::Flat {
            list,
            root,
            options,
        });
        effects
    }

    /// The walk ended: "Flat view on, 14 files in 4 folders.", or
    /// "Stopped at 4,200 entries.", then what was left out.
    pub(super) fn flat_done(
        &mut self,
        list: u64,
        result: Result<Box<FlatSummary>, Message>,
    ) -> Vec<Effect> {
        match result {
            Ok(summary) => {
                let mut line = format!("{}.", summary.on_words());
                for sentence in summary.skip_sentences() {
                    line.push(' ');
                    line.push_str(&sentence);
                }
                // The lister's own end: the list stops running, and the
                // flat view's words are said instead of a search's.
                self.finish_list(list, Message::done(line))
            }
            Err(message) => self.finish_results_failed(list, message),
        }
    }

    /// Copy or move out of a flat view asks first: "Keep folders, or all
    /// in one folder?", on Keep folders. A name already taken is asked
    /// about by the job, never overwritten in silence.
    pub(super) fn flat_copy_question(&mut self, copy: FlatCopy) -> Vec<Effect> {
        let rows = vec![
            ListRow::new(KEEP, "Keep folders, as below the flat view"),
            ListRow::new(ONE, "All in one folder; taken names asked"),
        ];
        let mut list = ListModel::new(KEEP_OR_ONE).with_rows(rows);
        list.set_help(
            "Enter chooses. A name already taken is asked about, never overwritten. Escape does nothing.",
        );
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Lists(ListsWants::FlatCopy(Box::new(copy))),
        )
    }

    /// The answer: all in one folder plans the job as the results would;
    /// keep folders moves each file into its own folder below the
    /// destination, made when missing (the engine's planner for moves to
    /// several folders). Copy that keeps folders has no planner yet and
    /// says so.
    pub(super) fn flat_copy_chosen(&mut self, copy: FlatCopy, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let job = self.next_job_id();
        let mut request =
            JobRequest::new(job, copy.kind, copy.sources.clone()).to(copy.destination.clone());
        let mut deepest = 0;
        let kept: Vec<(Place, Place)> = if row == KEEP {
            copy.sources
                .iter()
                .filter_map(|source| {
                    let (folder, depth) = copy.kept_folder(source)?;
                    deepest = deepest.max(depth);
                    Some((source.clone(), folder))
                })
                .collect()
        } else {
            Vec::new()
        };
        if row != KEEP || deepest == 0 {
            effects.extend(self.plan_job(copy.side, request, None, Then::Report, copy.confirm));
            return effects;
        }
        // The engine's planner makes one missing folder level and moves
        // only; the rest waits on a planner in the ops crate.
        let refusal = if copy.kind != JobKind::Move {
            Some(KEEP_COPY_NOT_BUILT)
        } else if deepest > 1 {
            Some(KEEP_ONE_LEVEL)
        } else {
            None
        };
        if let Some(line) = refusal {
            effects.push(self.say(Message::status(line)));
            effects.extend(self.flat_copy_question(copy));
            return effects;
        }
        if request.errors.is_ask() {
            request.errors = self.settings.jobs.on_errors.policy();
        }
        self.flows.insert(
            request.id,
            Flow {
                side: copy.side,
                note: None,
                then: Then::Report,
                confirm: copy.confirm,
                kind: JobKind::Move,
            },
        );
        self.request(Request::PlanMoves {
            request: Box::new(request),
            moves: kept,
        });
        effects
    }
}
