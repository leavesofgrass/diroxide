//! Duplicates in the app (alpha 5, lane DA; R34's commits 18 to 21): the
//! scope, the options, the search as a job, the groups list, the keep
//! rules, each copy's mark, and the trash through the engine, over the
//! engine half in `diroxide_ops::dups` and the words in
//! [`crate::dups_ui`].
//!
//! - **Find duplicates** (Tools menu) asks where to look ("This folder",
//!   the marked entries, both panels), then shows the options (the least
//!   size, the byte check, hidden files) under "Find now". The search is
//!   a job: "Hashing 120 of 450 files" at the progress interval, Cancel
//!   in the Jobs list. Above the size the settings name it stops and
//!   asks first, No the default, and Yes searches again without the
//!   limit.
//! - **The groups list** opens with the count line first: "3 groups, 7
//!   files, 44 KB to free", then what was left out ("2 empty files left
//!   out"). Each group is a row with its marks ("none marked", "2
//!   marked"); Enter lists its copies, each "Kept: a.txt, in D:\x" or
//!   "Marked: b.txt, in D:\y", where Enter changes the copy's mark and
//!   the last kept copy of a group can never be marked: "One copy must
//!   stay." The copies list's last row shows the group in the results
//!   lister (`App::open_list` with `ListKind::DuplicateGroup`).
//! - **Choose copies to keep** (a row of the groups list, and the
//!   command) offers the engine's rules, the oldest first, the default
//!   (question 54): it marks every copy but the keeper in each group and
//!   says what it did, "Marked 4 files, 44 KB. Kept the oldest in 3
//!   groups."
//! - **Trash the marked** has the engine read every keeper and every
//!   marked copy again; a group whose keeper changed is refused whole.
//!   What may go is shown as the trash's plan, with the bytes it frees,
//!   under a question that starts on No; the plan's check before each
//!   step compares each copy with the size and time the search hashed.
//!   It is one job, and Undo takes it back. Nothing here deletes.

use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, ConflictPolicy, Effect, Importance, JobId, JobKind, ListModel, ListRow,
    Message, MessageKind, Place, Redraw, Side,
};
use diroxide_ops::dups::{DupError, DupEstimate, KeepRule, Rechecked};
use diroxide_ops::{Plan, Step};
use diroxide_search::SearchHit;

use super::organize::Wants;
use super::searching::{ListKind, ListOpen};
use super::{App, Flow, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::dups_ui::{
    DupScope, DupSetup, DupView, DupsAsk, DupsNote, LEAST_SIZES, MARKS_UNCHANGED, NO_GROUPS_OPEN,
    NOTHING_MARKED, NOTHING_SEARCHED, NOTHING_TRASHED, OPTIONS_TITLE, RULES_TITLE, SCOPE_TITLE,
    progress_line, trash_question,
};
use crate::jobs::FinishedJob;
use crate::request::Request;

/// What a dialog of duplicates is for, hosted under `Wants::Dups`. Each
/// list carries what it shows, so the groups and their marks live in the
/// dialog that shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DupsWants {
    /// Where to look.
    Scope {
        /// The panel it was asked from.
        side: Side,
    },
    /// The options, under "Find now".
    Options(Box<DupSetup>),
    /// The question before a large read.
    Estimate(Box<DupSetup>),
    /// The groups.
    Groups(Box<DupView>),
    /// One group's copies.
    Members {
        /// The groups.
        view: Box<DupView>,
        /// The group shown.
        group: usize,
    },
    /// The keep rules.
    Rules(Box<DupView>),
    /// The question before the trash, with its plan.
    Trash {
        /// The groups, shown again on No.
        view: Box<DupView>,
        /// The trash's plan.
        plan: Box<Plan>,
    },
}

impl DupsWants {
    /// The purpose's name for the window's test of every purpose.
    pub(super) fn purpose(&self) -> &'static str {
        match self {
            DupsWants::Scope { .. } => "DupsScope",
            DupsWants::Options(_) => "DupsOptions",
            DupsWants::Estimate(_) => "DupsEstimate",
            DupsWants::Groups(_) => "DupsGroups",
            DupsWants::Members { .. } => "DupsMembers",
            DupsWants::Rules(_) => "DupsRules",
            DupsWants::Trash { .. } => "DupsTrash",
        }
    }
}

/// A duplicates command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Find,
    Keep,
}

const ROUTES: [(CommandId, Do); 2] = [(ids::FIND_DUPLICATES, Do::Find), (ids::DUPS_KEEP, Do::Keep)];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

/// The keep rules' rows: each (id, words).
fn rule_rows(folder: &Place) -> Vec<(&'static str, String)> {
    vec![
        ("oldest", "Keep the oldest".to_owned()),
        ("newest", "Keep the newest".to_owned()),
        ("shortest", "Keep the shortest path".to_owned()),
        ("folder", format!("Keep the copies in {folder}")),
        ("name", "Keep the first by name".to_owned()),
    ]
}

fn rule_of(id: &str, folder: &Place) -> Option<KeepRule> {
    Some(match id {
        "oldest" => KeepRule::Oldest,
        "newest" => KeepRule::Newest,
        "shortest" => KeepRule::ShortestPath,
        "folder" => KeepRule::InFolder(folder.clone()),
        "name" => KeepRule::FirstByName,
        _ => return None,
    })
}

/// A row id with a number after its letter: "g2" is group 2.
fn numbered(row: &str, letter: char) -> Option<usize> {
    row.strip_prefix(letter)?.parse().ok()
}

fn with_focus(mut list: ListModel, focus: Option<&str>) -> ListModel {
    if let Some(id) = focus {
        list.focus_id(id);
    }
    list
}

impl App {
    /// The duplicates commands; `None` for any other.
    pub(super) fn dups_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::Find => self.find_duplicates(command),
            Do::Keep => self.keep_copies(command),
        })
    }

    /// Find duplicates: where to look, a list.
    fn find_duplicates(&mut self, _command: &Command) -> Vec<Effect> {
        let side = self.active;
        let here = self.panel(side).place().clone();
        let there = self.panel(side.other()).place().clone();
        let marked = self.panel(side).list().selected_count();
        let mut rows = Vec::new();
        if marked > 0 {
            rows.push(ListRow::new(
                DupScope::Marked.id(),
                format!("Marked: {}", count(marked as u64, "entry", "entries")),
            ));
        }
        rows.push(ListRow::new(
            DupScope::ThisFolder.id(),
            format!("This folder: {here}"),
        ));
        if there != here {
            rows.push(ListRow::new(
                DupScope::BothPanels.id(),
                format!(
                    "Both panels: {} and {}",
                    crate::panel::place_leaf(&here),
                    crate::panel::place_leaf(&there)
                ),
            ));
        }
        let mut list = ListModel::new(SCOPE_TITLE).with_rows(rows);
        list.set_help(
            "Where Find duplicates looks: every folder below it is searched too. Enter goes on to the options; Escape searches nothing.",
        );
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Dups(DupsWants::Scope { side }),
        )
    }

    /// The setup a scope starts from: the places it names, the settings'
    /// options.
    fn dups_setup(&self, side: Side, scope: DupScope) -> Option<DupSetup> {
        let here = self.panel(side).place().clone();
        let roots = match scope {
            DupScope::ThisFolder => vec![here],
            DupScope::BothPanels => vec![here, self.panel(side.other()).place().clone()],
            DupScope::Marked => {
                let names: Vec<String> = self
                    .panel(side)
                    .list()
                    .selected_ids()
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                names.iter().filter_map(|n| here.join(n).ok()).collect()
            }
        };
        if roots.is_empty() {
            return None;
        }
        use diroxide_config::settings::DupHidden;
        let hidden = match self.settings.duplicates.hidden {
            DupHidden::Panel => self.panel(side).shows_hidden(),
            DupHidden::Include => true,
            DupHidden::LeaveOut => false,
        };
        let interval_ms = if self.screen_reader {
            self.settings.progress_interval_seconds.saturating_mul(1000)
        } else {
            1000
        }
        .max(250);
        Some(DupSetup {
            side,
            scope,
            roots,
            least: 0,
            byte_compare: false,
            hidden,
            ask_above: Some(
                self.settings
                    .duplicates
                    .ask_above_gb
                    .saturating_mul(1_000_000_000),
            ),
            interval_ms,
        })
    }

    /// The options list, the focus on `focus`, saying `message` or the
    /// list's name.
    fn show_dups_options(
        &mut self,
        setup: DupSetup,
        focus: Option<&str>,
        message: Option<Message>,
    ) -> Vec<Effect> {
        let rows = setup
            .option_rows()
            .into_iter()
            .map(|(id, words)| ListRow::new(id, words))
            .collect();
        let mut list = ListModel::new(OPTIONS_TITLE).with_rows(rows);
        list.set_help(
            "Enter on Find now starts the search; Enter on an option changes it. Escape searches nothing.",
        );
        let list = with_focus(list, focus);
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::ACCEPT),
        };
        let wants = Wants::Dups(DupsWants::Options(Box::new(setup)));
        match message {
            Some(message) => {
                self.open_dialog_saying(dialog, Purpose::Organize(Box::new(wants)), message)
            }
            None => self.open_organize(dialog, wants),
        }
    }

    /// Enter on an option: Find now starts; another changes and is said.
    fn dups_option(&mut self, mut setup: DupSetup, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        match row {
            "find" => {
                effects.extend(self.start_dups(setup));
                return effects;
            }
            "least" => setup.least = (setup.least + 1) % LEAST_SIZES.len(),
            "bytes" => setup.byte_compare = !setup.byte_compare,
            "hidden" => setup.hidden = !setup.hidden,
            _ => {}
        }
        let said = setup
            .option_rows()
            .into_iter()
            .find(|(id, _)| *id == row)
            .map(|(_, words)| words)
            .unwrap_or_default();
        effects.extend(self.show_dups_options(setup, Some(row), Some(Message::status(said))));
        effects
    }

    /// The search, as a job.
    fn start_dups(&mut self, setup: DupSetup) -> Vec<Effect> {
        let job = self.next_job_id();
        let line = setup.start_line();
        let now = self.now();
        self.jobs
            .begin(job, JobKind::FindDuplicates, line.clone(), now);
        self.request(Request::Duplicates(Box::new(DupsAsk::Find {
            job,
            setup: Box::new(setup),
        })));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }

    /// The groups list, the focus on `focus`, saying `message` or the
    /// list's name and the focused row.
    fn show_groups(
        &mut self,
        view: DupView,
        focus: Option<&str>,
        message: Option<Message>,
    ) -> Vec<Effect> {
        let mut rows: Vec<ListRow> = (0..view.result.groups.len())
            .filter_map(|at| {
                let (row, marks) = view.group_row(at)?;
                Some(ListRow::new(format!("g{at}"), row).with_details([marks]))
            })
            .collect();
        rows.push(ListRow::new("keep", RULES_TITLE));
        rows.push(ListRow::new("trash", view.trash_row()));
        let mut list = ListModel::new(view.title()).with_rows(rows);
        list.set_help(
            "Enter on a group lists its copies, where Enter marks or keeps one. Choose copies to keep marks all but one in each group. Trash the marked asks first. One copy of each group always stays.",
        );
        let list = with_focus(list, focus);
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::ACCEPT),
        };
        let wants = Wants::Dups(DupsWants::Groups(Box::new(view)));
        match message {
            Some(message) => {
                self.open_dialog_saying(dialog, Purpose::Organize(Box::new(wants)), message)
            }
            None => self.open_organize(dialog, wants),
        }
    }

    /// One group's copies, the focus on `focus`, saying `message` or the
    /// list's name and the focused row.
    fn show_members(
        &mut self,
        view: DupView,
        group: usize,
        focus: Option<&str>,
        message: Option<Message>,
    ) -> Vec<Effect> {
        let files = view
            .result
            .groups
            .get(group)
            .map(|g| g.files.len())
            .unwrap_or_default();
        let mut rows: Vec<ListRow> = (0..files)
            .filter_map(|file| {
                Some(ListRow::new(
                    format!("m{file}"),
                    view.member_row(group, file)?,
                ))
            })
            .collect();
        rows.push(ListRow::new("show", "Show these in the results list"));
        let mut list = ListModel::new(view.members_title(group)).with_rows(rows);
        list.set_help(
            "Enter on a copy marks it for the trash or keeps it; one copy always stays. Escape goes back to the groups.",
        );
        let list = with_focus(list, focus);
        let dialog = Dialog::Choose {
            list,
            on_enter: Command::new(ids::ACCEPT),
        };
        let wants = Wants::Dups(DupsWants::Members {
            view: Box::new(view),
            group,
        });
        match message {
            Some(message) => {
                self.open_dialog_saying(dialog, Purpose::Organize(Box::new(wants)), message)
            }
            None => self.open_organize(dialog, wants),
        }
    }

    /// The keep rules over the groups, the oldest first.
    fn show_dup_rules(&mut self, view: DupView) -> Vec<Effect> {
        let folder = self.panel(view.setup.side).place().clone();
        let rows = rule_rows(&folder)
            .into_iter()
            .map(|(id, words)| ListRow::new(id, words))
            .collect();
        let mut list = ListModel::new(RULES_TITLE).with_rows(rows);
        list.set_help(
            "Each rule keeps one copy in every group and marks the rest; a tie goes to the shortest path. A group a rule cannot decide is left alone and said.",
        );
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Dups(DupsWants::Rules(Box::new(view))),
        )
    }

    /// Choose copies to keep: the keep rules over the open groups.
    fn keep_copies(&mut self, _command: &Command) -> Vec<Effect> {
        let view = match self.opens.last().map(|open| &open.purpose) {
            Some(Purpose::Organize(wants)) => match wants.as_ref() {
                Wants::Dups(DupsWants::Groups(view) | DupsWants::Members { view, .. }) => {
                    Some((**view).clone())
                }
                _ => None,
            },
            _ => None,
        };
        let Some(view) = view else {
            return vec![self.say(Message::info(NO_GROUPS_OPEN))];
        };
        let mut effects = self.close_top();
        effects.extend(self.show_dup_rules(view));
        effects
    }

    /// A rule chosen: it marks, says what it did, and the groups come
    /// back with their marks.
    fn rule_chosen(&mut self, mut view: DupView, row: &str) -> Vec<Effect> {
        let folder = self.panel(view.setup.side).place().clone();
        let Some(rule) = rule_of(row, &folder) else {
            return Vec::new();
        };
        let mut effects = self.close_top();
        let chosen = view.apply(&rule);
        let mut line = chosen.summary();
        if let Some(first) = chosen.left_alone_lines().first() {
            line.push(' ');
            line.push_str(first);
            line.push('.');
        }
        effects.extend(self.show_groups(view, Some("trash"), Some(Message::done(line))));
        effects
    }

    /// Enter on a copy: its mark changes, or the refusal is said.
    fn member_chosen(&mut self, mut view: DupView, group: usize, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        if row == "show" {
            effects.extend(self.show_members(view.clone(), group, Some("show"), None));
            effects.extend(self.show_group_list(&view, group));
            return effects;
        }
        let Some(file) = numbered(row, 'm') else {
            effects.extend(self.show_members(view, group, None, None));
            return effects;
        };
        let message = match view.toggle(group, file) {
            Ok(line) => Message::status(line),
            Err(refusal) => Message::warning(refusal),
        };
        effects.extend(self.show_members(view, group, Some(row), Some(message)));
        effects
    }

    /// A group's copies in the results lister, each with its mark after
    /// its name; the copies list waits under it.
    fn show_group_list(&mut self, view: &DupView, group: usize) -> Vec<Effect> {
        let Some(found) = view.result.groups.get(group) else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        let mut notes = Vec::new();
        for (file, member) in found.files.iter().enumerate() {
            let Some(entry) = view.seen(&member.place) else {
                continue;
            };
            hits.push(SearchHit::new(member.place.clone(), entry));
            let word = if view.is_marked(group, file) {
                "marked"
            } else {
                "kept"
            };
            notes.push((member.place.clone(), word.to_owned()));
        }
        let title = view.members_title(group);
        let (effects, _) = self.open_list(ListOpen {
            kind: ListKind::DuplicateGroup,
            side: view.setup.side,
            title: title.clone(),
            root: None,
            hits,
            gone: Vec::new(),
            notes,
            start: format!("{title}, duplicates"),
            streaming: false,
        });
        effects
    }

    /// Trash the marked: the engine reads every keeper and every marked
    /// copy again first, as a job.
    fn trash_marked(&mut self, view: DupView) -> Vec<Effect> {
        let (files, _) = view.marked_totals();
        if files == 0 {
            return self.show_groups(view, Some("keep"), Some(Message::info(NOTHING_MARKED)));
        }
        let job = self.next_job_id();
        let line = format!("Checking {} again", count(files as u64, "copy", "copies"));
        let now = self.now();
        self.jobs
            .begin(job, JobKind::FindDuplicates, line.clone(), now);
        self.request(Request::Duplicates(Box::new(DupsAsk::Recheck {
            job,
            view: Box::new(view),
        })));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }

    /// Enter or a typed answer in one of these dialogs.
    pub(super) fn dups_accept(
        &mut self,
        wants: DupsWants,
        row: Option<String>,
        _text: &str,
    ) -> Vec<Effect> {
        let Some(row) = row else {
            return Vec::new();
        };
        match wants {
            DupsWants::Scope { side } => {
                let Some(scope) = DupScope::from_id(&row) else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                match self.dups_setup(side, scope) {
                    Some(setup) => effects.extend(self.show_dups_options(setup, None, None)),
                    None => effects.push(self.say(Message::info(NOTHING_SEARCHED))),
                }
                effects
            }
            DupsWants::Options(setup) => self.dups_option(*setup, &row),
            DupsWants::Groups(view) => {
                let mut effects = self.close_top();
                match row.as_str() {
                    "keep" => effects.extend(self.show_dup_rules(*view)),
                    "trash" => effects.extend(self.trash_marked(*view)),
                    _ => match numbered(&row, 'g') {
                        Some(group) => effects.extend(self.show_members(*view, group, None, None)),
                        None => effects.extend(self.show_groups(*view, None, None)),
                    },
                }
                effects
            }
            DupsWants::Members { view, group } => self.member_chosen(*view, group, &row),
            DupsWants::Rules(view) => self.rule_chosen(*view, &row),
            DupsWants::Estimate(_) | DupsWants::Trash { .. } => self.dialog_escape(),
        }
    }

    /// Yes to one of these questions: the large read searches again
    /// without its limit; the trash's plan runs as one job.
    pub(super) fn dups_yes(&mut self, wants: DupsWants) -> Vec<Effect> {
        match wants {
            DupsWants::Estimate(mut setup) => {
                setup.ask_above = None;
                self.start_dups(*setup)
            }
            DupsWants::Trash { view, plan } => {
                self.flows.insert(
                    plan.job,
                    Flow {
                        side: view.setup.side,
                        note: None,
                        then: Then::Report,
                        confirm: false,
                        kind: JobKind::Trash,
                    },
                );
                self.run_plan(*plan)
            }
            _ => Vec::new(),
        }
    }

    /// Escape on one of these dialogs, after it closed: a list of the
    /// groups' own goes back to the groups; the first steps search
    /// nothing.
    pub(super) fn dups_escape(&mut self, wants: DupsWants) -> Vec<Effect> {
        match wants {
            DupsWants::Scope { .. } | DupsWants::Options(_) | DupsWants::Estimate(_) => {
                vec![self.say(Message::info(NOTHING_SEARCHED))]
            }
            DupsWants::Groups(_) => Vec::new(),
            DupsWants::Members { view, group } => {
                let focus = format!("g{group}");
                self.show_groups(*view, Some(&focus), None)
            }
            DupsWants::Rules(view) => {
                self.show_groups(*view, Some("keep"), Some(Message::info(MARKS_UNCHANGED)))
            }
            DupsWants::Trash { view, .. } => {
                self.show_groups(*view, Some("trash"), Some(Message::info(NOTHING_TRASHED)))
            }
        }
    }

    /// What the runtime answered a [`DupsAsk`].
    pub(super) fn dups_note(&mut self, note: DupsNote) -> Vec<Effect> {
        match note {
            DupsNote::Progress { job, progress } => {
                if self.jobs.kind_of(job).is_none() {
                    return Vec::new();
                }
                let line = progress_line(&progress);
                self.jobs.update(job, line.clone());
                let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
                if self.jobs.front() == Some(job) {
                    effects.push(Effect::Announce(Message::progress(line)));
                }
                effects
            }
            DupsNote::Found {
                job,
                setup,
                result,
                hidden_left_out,
            } => self.dups_found(job, *setup, result, hidden_left_out),
            DupsNote::Rechecked { job, view, result } => self.dups_rechecked(job, *view, result),
        }
    }

    fn finish_dups_job(&mut self, job: JobId, summary: &str, done: u64, cancelled: bool) {
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::FindDuplicates,
            summary: summary.to_owned(),
            done,
            cancelled,
            ..FinishedJob::default()
        });
    }

    /// A search ended: the groups, the question before a large read, or
    /// why it stopped.
    fn dups_found(
        &mut self,
        job: JobId,
        setup: DupSetup,
        result: Result<Box<diroxide_ops::dups::DupResult>, DupError>,
        hidden_left_out: u64,
    ) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        match result {
            Ok(result) => {
                let view = DupView::new(setup, *result, hidden_left_out);
                let line = view.start_line();
                self.finish_dups_job(job, &line, view.result.files_seen, false);
                if view.result.groups.is_empty() {
                    effects.push(self.say(Message::done(line)));
                } else {
                    effects.extend(self.show_groups(view, None, Some(Message::done(line))));
                }
            }
            Err(DupError::Declined(estimate)) => {
                let line = format!("Stopped to ask: {}", estimate_words(&estimate));
                self.finish_dups_job(job, &line, 0, false);
                let question =
                    Message::new(estimate.question(), Importance::High, MessageKind::Question);
                effects.extend(self.open_organize(
                    Dialog::Confirm {
                        question,
                        listing: vec![format!("Looks in {}", setup.where_words())],
                        on_yes: Command::new(ids::YES),
                    },
                    Wants::Dups(DupsWants::Estimate(Box::new(setup))),
                ));
            }
            Err(error) => {
                let line = error.to_string();
                let cancelled = error == DupError::Cancelled;
                self.finish_dups_job(job, &line, 0, cancelled);
                let message = if cancelled {
                    Message::info(format!("{line}."))
                } else {
                    Message::error(format!("{line}."))
                };
                effects.push(self.say(message));
            }
        }
        effects
    }

    /// The check before the trash ended: the trash's plan under its
    /// question, or the groups again with what was held back.
    fn dups_rechecked(
        &mut self,
        job: JobId,
        view: DupView,
        result: Result<Rechecked, DupError>,
    ) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        let checked = match result {
            Ok(checked) => checked,
            Err(error) => {
                let cancelled = error == DupError::Cancelled;
                let line = if cancelled {
                    "Check cancelled; nothing trashed.".to_owned()
                } else {
                    format!("Not trashed: {error}.")
                };
                self.finish_dups_job(job, &line, 0, cancelled);
                effects.extend(self.show_groups(view, Some("trash"), Some(Message::info(line))));
                return effects;
            }
        };
        let summary = checked.summary();
        self.finish_dups_job(job, &summary, checked.safe.len() as u64, false);
        let held: Vec<String> = checked.refused.iter().map(|r| r.line()).collect();
        if checked.safe.is_empty() {
            let mut line = format!("{summary}.");
            if let Some(first) = held.first() {
                line.push(' ');
                line.push_str(first);
                line.push('.');
            }
            effects.extend(self.show_groups(view, Some("trash"), Some(Message::warning(line))));
            return effects;
        }
        let bytes = view.bytes_of(&checked.safe);
        let plan_job = self.next_job_id();
        let steps = checked
            .safe
            .iter()
            .map(|place| Step::Trash {
                place: place.clone(),
            })
            .collect();
        let mut plan = Plan::new(plan_job, JobKind::Trash, steps, ConflictPolicy::Ask);
        plan.facts.items = checked.safe.len() as u64;
        plan.facts.files = checked.safe.len() as u64;
        plan.facts.bytes = bytes;
        plan.facts.counted = true;
        for place in &checked.safe {
            if let Some(entry) = view.seen(place) {
                plan.expected.insert(place.clone(), entry);
            }
        }
        let mut listing = vec![summary];
        listing.extend(held);
        listing.extend(plan.listing());
        let question = with_default_no(Message::new(
            trash_question(checked.safe.len(), bytes),
            Importance::High,
            MessageKind::Question,
        ));
        effects.extend(self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Wants::Dups(DupsWants::Trash {
                view: Box::new(view),
                plan: Box::new(plan),
            }),
        ));
        effects
    }
}

/// "8.0 GB in 1,204 files".
fn estimate_words(estimate: &DupEstimate) -> String {
    format!(
        "{} in {}",
        diroxide_core::format::human_size(estimate.bytes),
        count(estimate.files, "file", "files")
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diroxide_core::{FixedClock, Key, Timestamp, UtcOffset};

    use super::*;
    use crate::app::{AppEvent, AppOptions};
    use crate::dups_ui::ONE_COPY_MUST_STAY;
    use crate::runtime::Runtime;

    /// A folder of its own under the test's temporary folder: two copies
    /// of one text, one other text, and an empty file. Nothing a test does
    /// here reaches the trash.
    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        std::fs::write(sub.join("b.txt"), "hello world").unwrap();
        std::fs::write(dir.path().join("c.txt"), "something else").unwrap();
        std::fs::write(dir.path().join("e.txt"), "").unwrap();
        dir
    }

    fn started(dir: &std::path::Path) -> (App, Runtime) {
        let here = Place::parse(dir.to_str().unwrap()).unwrap();
        let mut options = AppOptions::new(here.clone(), here);
        options.clock = Arc::new(FixedClock::new(
            Timestamp::from_civil(2026, 11, 12, 16, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        ));
        options.settings.dialog_settle_ms = 0;
        let mut app = App::new(options);
        let mut runtime = Runtime::with_local();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        (app, runtime)
    }

    fn said(effects: Vec<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Announce(message) => Some(message.text),
                _ => None,
            })
            .collect()
    }

    fn key(app: &mut App, runtime: &mut Runtime, name: &str) -> Vec<String> {
        let effects = app.handle(AppEvent::Key(Key::parse(name).unwrap()));
        said(runtime.run_until_idle(app, effects))
    }

    fn command(app: &mut App, runtime: &mut Runtime, id: CommandId) -> Vec<String> {
        let effects = app.dispatch(&Command::new(id));
        said(runtime.run_until_idle(app, effects))
    }

    fn rows(app: &App) -> Vec<String> {
        match app.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => {
                list.rows().iter().map(|r| r.label.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    fn focus(app: &mut App, label_start: &str) {
        let at = rows(app)
            .iter()
            .position(|r| r.starts_with(label_start))
            .unwrap_or_else(|| panic!("no row starts {label_start}: {:?}", rows(app)));
        assert!(app.focus_dialog_row(at));
    }

    /// From the command to the groups: where, the options, the search as
    /// a job, and the groups opening with the count line first.
    fn groups_open(app: &mut App, runtime: &mut Runtime) -> Vec<String> {
        let opened = command(app, runtime, ids::FIND_DUPLICATES);
        assert_eq!(app.dialog_purpose(), Some("DupsScope"));
        assert!(opened[0].starts_with("Find duplicates where, This folder: "));
        assert!(rows(app).iter().any(|r| r.starts_with("This folder: ")));
        let options = key(app, runtime, "enter");
        assert_eq!(app.dialog_purpose(), Some("DupsOptions"));
        assert!(options[0].starts_with("Find duplicates options, Find now: "));
        // An option changes and is said; the focus stays on it.
        focus(app, "Byte check");
        assert_eq!(key(app, runtime, "enter"), ["Byte check: on, slower"]);
        focus(app, "Find now");
        key(app, runtime, "enter")
    }

    #[test]
    fn the_search_finds_the_copies_and_the_groups_say_their_count_first() {
        let dir = folder();
        let (mut app, mut runtime) = started(dir.path());
        let said = groups_open(&mut app, &mut runtime);
        assert!(said[0].starts_with("Finding duplicates in "), "{said:?}");
        let start = "1 group, 2 files, 11 bytes to free. 1 empty file left out.";
        assert!(said.iter().any(|s| s == start), "{said:?}");
        assert_eq!(start.split(". ").next().unwrap().chars().count(), 34);
        assert_eq!(app.dialog_purpose(), Some("DupsGroups"));
        assert_eq!(
            rows(&app),
            [
                "Group 1 of 1, 2 copies, 11 bytes each, 11 bytes to free",
                "Choose copies to keep",
                "Trash the marked: none marked yet",
            ]
        );
        // Trash with nothing marked says so and touches nothing.
        focus(&mut app, "Trash the marked");
        assert_eq!(
            key(&mut app, &mut runtime, "enter"),
            ["Nothing marked: choose copies to keep."]
        );
    }

    #[test]
    fn a_rule_marks_a_copy_the_last_copy_stays_and_the_trash_asks_first() {
        let dir = folder();
        let (mut app, mut runtime) = started(dir.path());
        groups_open(&mut app, &mut runtime);
        // Choose copies to keep, the command: the rules, the oldest first.
        let rules = command(&mut app, &mut runtime, ids::DUPS_KEEP);
        assert_eq!(app.dialog_purpose(), Some("DupsRules"));
        assert_eq!(rules, ["Choose copies to keep, Keep the oldest, 1 of 5"]);
        let marked = key(&mut app, &mut runtime, "enter");
        assert!(
            marked[0].starts_with("Marked 1 file, 11 bytes. Kept the "),
            "{marked:?}"
        );
        assert_eq!(app.dialog_purpose(), Some("DupsGroups"));
        assert_eq!(rows(&app)[2], "Trash the marked: 1 file, 11 bytes");

        // The group's copies: one kept, one marked; the kept one stays.
        focus(&mut app, "Group 1");
        key(&mut app, &mut runtime, "enter");
        assert_eq!(app.dialog_purpose(), Some("DupsMembers"));
        let copies = rows(&app);
        assert_eq!(copies.iter().filter(|r| r.starts_with("Kept: ")).count(), 1);
        assert_eq!(
            copies.iter().filter(|r| r.starts_with("Marked: ")).count(),
            1
        );
        assert_eq!(copies[2], "Show these in the results list");
        focus(&mut app, "Kept: ");
        assert_eq!(key(&mut app, &mut runtime, "enter"), [ONE_COPY_MUST_STAY]);
        // Unmarked and marked again, each said.
        focus(&mut app, "Marked: ");
        let kept = key(&mut app, &mut runtime, "enter");
        assert!(kept[0].starts_with("Kept "), "{kept:?}");
        let again = key(&mut app, &mut runtime, "enter");
        assert!(again[0].starts_with("Marked "), "{again:?}");
        // The copies in the results lister, each with its mark.
        focus(&mut app, "Show these");
        let shown = key(&mut app, &mut runtime, "enter");
        assert!(
            shown
                .iter()
                .any(|s| s == "Group 1 of 1, 2 copies, duplicates")
        );
        assert_eq!(app.list_kind(), Some(&ListKind::DuplicateGroup));
        key(&mut app, &mut runtime, "escape");
        assert_eq!(app.dialog_purpose(), Some("DupsMembers"));
        key(&mut app, &mut runtime, "escape");
        assert_eq!(app.dialog_purpose(), Some("DupsGroups"));

        // Trash the marked: checked again, then the question on No.
        focus(&mut app, "Trash the marked");
        let asked = key(&mut app, &mut runtime, "enter");
        assert_eq!(asked[0], "Checking 1 copy again");
        assert_eq!(app.dialog_purpose(), Some("DupsTrash"));
        let question = "Trash 1 copy, 11 bytes? No is the default. y or n";
        assert!(asked.iter().any(|s| s == question), "{asked:?}");
        let Some((_, Dialog::Confirm { listing, .. })) = app.dialogs.last() else {
            panic!("no question");
        };
        assert_eq!(listing[0], "Checked again: 1 file can go to the trash");
        let plan = match app.opens.last().map(|o| &o.purpose) {
            Some(Purpose::Organize(wants)) => match wants.as_ref() {
                Wants::Dups(DupsWants::Trash { plan, .. }) => plan.clone(),
                _ => panic!("not the trash question"),
            },
            _ => panic!("not the trash question"),
        };
        assert_eq!(plan.kind, JobKind::Trash);
        assert_eq!(plan.steps.len(), 1);
        let Step::Trash { place } = &plan.steps[0] else {
            panic!("not a trash step");
        };
        assert_eq!(plan.expected[place].size, Some(11));
        // No: nothing moves, and the groups come back.
        let no = key(&mut app, &mut runtime, "n");
        assert!(no.iter().any(|s| s == NOTHING_TRASHED), "{no:?}");
        assert_eq!(app.dialog_purpose(), Some("DupsGroups"));
        assert!(dir.path().join("a.txt").exists());
        assert!(dir.path().join("sub").join("b.txt").exists());
    }

    #[test]
    fn a_large_read_asks_first_on_no_and_yes_searches_again() {
        // Files longer than two quick-check blocks go to the full hash.
        let dir = tempfile::tempdir().unwrap();
        let big = vec![7u8; 200_000];
        std::fs::write(dir.path().join("one.bin"), &big).unwrap();
        std::fs::write(dir.path().join("two.bin"), &big).unwrap();
        let (mut app, mut runtime) = started(dir.path());
        let mut setup = app.dups_setup(Side::Left, DupScope::ThisFolder).unwrap();
        setup.ask_above = Some(1);
        let effects = app.start_dups(setup);
        let heard = said(runtime.run_until_idle(&mut app, effects));
        assert_eq!(app.dialog_purpose(), Some("DupsEstimate"));
        let question = "Read 400 KB to find duplicates? 2 files. No is the default. y or n";
        assert!(heard.iter().any(|s| s == question), "{heard:?}");
        let found = key(&mut app, &mut runtime, "y");
        assert_eq!(app.dialog_purpose(), Some("DupsGroups"), "{found:?}");
        // Escape on the question searches nothing.
        let effects = app.dups_escape(DupsWants::Estimate(Box::new(
            app.dups_setup(Side::Left, DupScope::ThisFolder).unwrap(),
        )));
        assert_eq!(said(effects), [NOTHING_SEARCHED]);
    }

    #[test]
    fn a_cancelled_search_says_so_and_keep_needs_open_groups() {
        let dir = folder();
        let (mut app, mut runtime) = started(dir.path());
        let setup = app.dups_setup(Side::Left, DupScope::ThisFolder).unwrap();
        let effects = app.dups_note(DupsNote::Found {
            job: JobId::new(9_000),
            setup: Box::new(setup),
            result: Err(DupError::Cancelled),
            hidden_left_out: 0,
        });
        assert_eq!(said(effects), ["Finding duplicates was cancelled."]);
        assert_eq!(
            command(&mut app, &mut runtime, ids::DUPS_KEEP),
            [NO_GROUPS_OPEN]
        );
        // Escape on where to look searches nothing.
        command(&mut app, &mut runtime, ids::FIND_DUPLICATES);
        assert_eq!(key(&mut app, &mut runtime, "escape"), [NOTHING_SEARCHED]);
    }
}
