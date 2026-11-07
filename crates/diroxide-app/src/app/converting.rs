//! The Convert command and the conversion reports (alpha 4, lane AW, R30
//! commit 62; R28 5.3): a list of the recipes that take the selection,
//! name first and the sentence second, those whose tool is missing last
//! ("Video to MP4, needs libx264"); the other panel's folder the target;
//! the plan's question starting on No ("Convert 13 files to JPEG? y or
//! n") with the plan's rows (refusals, skips, the work, the tool, the
//! input's size, a low-space warning); the job through CL's engine with
//! the tools registry; its report said at the end ("Report: 1 failed, 12
//! converted") and kept with the last 20 in the Conversion reports list,
//! each report's rows failures first. Undo trashes what the job made.
//! carta runs in the `diro` child the frontend gives the runtime.
//!
//! No default key: Shift+F5 is Duplicate. The lists are hosted
//! under the Organize purpose so the window's test of every purpose keeps
//! its count; a purpose of their own is for the fold lane (CR5).

use diroxide_core::clock::describe_time;
use diroxide_core::{Command, Effect, JobKind, ListModel, ListRow, Message, Place, Redraw, Side};
use diroxide_ops::convert::KeptReport;

use super::{App, Flow, Purpose, Then};
use crate::commands::ids;
use crate::conversions::{ConvertAsk, ConvertNote, PlanToRun, RecipeChoice, SharedTypes};
use crate::dialog::{Dialog, with_default_no};
use crate::request::{Request, SharedClock};

/// The row of the Convert list that says some files have no recipe.
const PARTIAL_ROW: &str = "partial";

/// What a dialog of the Convert command is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConvertWants {
    /// The recipes for a selection: Enter plans with the chosen one.
    Recipes {
        /// The panel the selection is in.
        side: Side,
        /// The selection.
        sources: Vec<Place>,
        /// The folder the outputs go to.
        target: Place,
    },
    /// The plan's question: Yes runs it.
    Run {
        /// The panel it was asked from.
        side: Side,
        /// The plan.
        plan: PlanToRun,
    },
    /// The kept reports: Enter reads one.
    Reports(Vec<KeptReport>),
}

/// The line said when a conversion starts, the verb first: "Convert
/// started: 13 files to JPEG".
fn start_line(question: &str) -> String {
    let what = question.trim_end_matches("? y or n");
    match what.strip_prefix("Convert ") {
        Some(rest) => format!("Convert started: {rest}"),
        None => format!("{what} started"),
    }
}

impl App {
    /// Convert and Conversion reports; `None` for any other command.
    pub(super) fn convert_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::CONVERT && command.id != ids::CONVERT_REPORTS {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        // A file whose type has a convert action runs it (R30 commit 64).
        if command.id == ids::CONVERT
            && let Some(effects) = self.typed_slot(crate::typing::Slot::Convert)
        {
            return Some(effects);
        }
        Some(if command.id == ids::CONVERT {
            self.convert_selection()
        } else {
            self.request(Request::Convert(Box::new(ConvertAsk::Reports)));
            Vec::new()
        })
    }

    /// Convert: asks the runtime for the recipes that take the marked
    /// entries, or the focused one; the target is the other panel's
    /// folder.
    fn convert_selection(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, _)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let panel = self.panel(side);
        let with_kind: Vec<(Place, bool)> = sources
            .iter()
            .map(|place| {
                let folder = place
                    .name()
                    .and_then(|name| panel.entry(name))
                    .is_some_and(|entry| entry.is_folder_like());
                (place.clone(), folder)
            })
            .collect();
        let target = self.panel(side.other()).place().clone();
        let tools = self.tool_runner();
        let types = SharedTypes(std::sync::Arc::clone(&self.types));
        self.request(Request::Convert(Box::new(ConvertAsk::Recipes {
            sources: with_kind,
            types,
            tools,
            target,
        })));
        Vec::new()
    }

    /// A type's convert action, or any action naming a recipe: the
    /// focused file planned with that recipe into the other panel, the
    /// plan's question next (R30 commit 64).
    pub(super) fn convert_with_recipe(&mut self, recipe: &str, place: Place) -> Vec<Effect> {
        let side = self.active;
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side,
                note: None,
                then: Then::Report,
                confirm: true,
                kind: JobKind::Convert,
            },
        );
        let tools = self.tool_runner();
        let types = SharedTypes(std::sync::Arc::clone(&self.types));
        let destination = self.panel(side.other()).place().clone();
        self.request(Request::Convert(Box::new(ConvertAsk::Plan {
            job,
            sources: vec![place],
            destination,
            recipe: recipe.to_owned(),
            types,
            tools,
        })));
        vec![self.say(Message::status("Planning the conversion"))]
    }

    /// What the runtime answered the Convert command.
    pub(super) fn convert_note(&mut self, note: ConvertNote) -> Vec<Effect> {
        match note {
            ConvertNote::Recipes {
                sources,
                target,
                result,
            } => match result {
                Ok(choice) => self.recipes_list(sources, target, &choice),
                Err(message) => vec![self.say(message)],
            },
            ConvertNote::Planned { job, result } => match result {
                Ok(plan) => self.convert_question(job, plan),
                Err(message) => {
                    self.flows.remove(&job);
                    vec![self.say(message)]
                }
            },
            ConvertNote::Finished {
                report,
                summary,
                lines,
            } => {
                // The summary, and the first failure when there is one: the
                // rest is in Conversion reports.
                let mut said = vec![summary];
                said.extend(lines.iter().find(|l| l.starts_with("Failed: ")).cloned());
                self.finish_lines = Some(said);
                let effects = self.job_finished(*report);
                self.finish_lines = None;
                effects
            }
            ConvertNote::Reports(result) => match result {
                Ok(reports) => self.reports_list(reports),
                Err(message) => vec![self.say(message)],
            },
        }
    }

    /// The recipes list: "Convert 13 files", each recipe by name with its
    /// sentence, those that cannot run here last and unavailable.
    fn recipes_list(
        &mut self,
        sources: Vec<Place>,
        target: Place,
        choice: &RecipeChoice,
    ) -> Vec<Effect> {
        let what = match sources.as_slice() {
            [one] => one
                .name()
                .map(|n| diroxide_core::names::display_name(n).into_owned())
                .unwrap_or_else(|| crate::panel::place_leaf(one)),
            many => diroxide_core::format::count(many.len() as u64, "item", "items"),
        };
        let mut rows = Vec::new();
        if choice.partial {
            let mut row = ListRow::new(PARTIAL_ROW, "Some files have no recipe")
                .with_details(["these take the most"]);
            row.enabled = false;
            rows.push(row);
        }
        for recipe in &choice.rows {
            let mut row = match &recipe.needs {
                None => ListRow::new(recipe.id.clone(), recipe.name.clone()),
                Some(needs) => {
                    let mut row = ListRow::new(recipe.id.clone(), needs.clone());
                    row.enabled = false;
                    row
                }
            };
            if !recipe.about.is_empty() {
                row.details.push(recipe.about.clone());
            }
            rows.push(row);
        }
        let mut list = ListModel::new(format!("Convert {what}")).with_rows(rows);
        list.set_help("Up and Down choose a recipe, Enter plans it, Escape closes.");
        let side = self.active;
        let mut effects = self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Convert(
                ConvertWants::Recipes {
                    side,
                    sources,
                    target,
                },
            ))),
        );
        if let Some(problem) = choice.problems.first() {
            effects.push(self.say(Message::warning(problem.clone())));
        }
        effects
    }

    /// The plan's question, starting on No, with the plan's rows; or, when
    /// nothing can be converted, the plan's summary and why.
    fn convert_question(
        &mut self,
        job: diroxide_core::JobId,
        plan: Box<diroxide_ops::convert::ConvertPlan>,
    ) -> Vec<Effect> {
        let side = self.flows.remove(&job).map_or(self.active, |f| f.side);
        if plan.is_empty() {
            let mut text = format!("{}. Nothing to convert.", plan.summary());
            if let Some(first) = plan.lines().first()
                && (first.starts_with("Refused: ") || first.starts_with("Skipped "))
            {
                text = format!("{}. {first}.", plan.summary());
            }
            return vec![self.say(Message::info(text))];
        }
        let question = with_default_no(plan.question());
        let listing = plan.lines();
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Convert(
                ConvertWants::Run {
                    side,
                    plan: PlanToRun(plan),
                },
            ))),
        )
    }

    /// The kept reports, newest first: each by its summary, with when it
    /// ended.
    fn reports_list(&mut self, reports: Vec<KeptReport>) -> Vec<Effect> {
        if reports.is_empty() {
            return vec![self.say(Message::info("No conversion reports yet"))];
        }
        let rows: Vec<ListRow> = reports
            .iter()
            .enumerate()
            .map(|(i, report)| {
                let at = diroxide_core::Timestamp::from_unix(report.finished, 0);
                let when = describe_time(at, self.clock.as_ref());
                ListRow::new(i.to_string(), report.summary.clone()).with_details([when])
            })
            .collect();
        let title = format!(
            "Conversion reports: {}",
            diroxide_core::format::count(reports.len() as u64, "kept", "kept")
        );
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help("Up and Down choose a report, Enter reads it, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Convert(
                ConvertWants::Reports(reports),
            ))),
        )
    }

    /// Enter in one of the Convert command's lists.
    pub(super) fn convert_accept(
        &mut self,
        wants: ConvertWants,
        row: Option<String>,
    ) -> Vec<Effect> {
        let Some(row) = row else {
            return Vec::new();
        };
        match wants {
            ConvertWants::Recipes {
                side,
                sources,
                target,
            } => {
                if row == PARTIAL_ROW {
                    return Vec::new();
                }
                let mut effects = self.close_top();
                let job = self.next_job_id();
                self.flows.insert(
                    job,
                    Flow {
                        side,
                        note: None,
                        then: Then::Report,
                        confirm: true,
                        kind: JobKind::Convert,
                    },
                );
                let tools = self.tool_runner();
                let types = SharedTypes(std::sync::Arc::clone(&self.types));
                self.request(Request::Convert(Box::new(ConvertAsk::Plan {
                    job,
                    sources,
                    destination: target,
                    recipe: row,
                    types,
                    tools,
                })));
                effects.push(self.say(Message::status("Planning the conversion")));
                effects
            }
            ConvertWants::Reports(reports) => {
                let Some(report) = row.parse::<usize>().ok().and_then(|i| reports.get(i)) else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.open_dialog(
                    Dialog::Read {
                        title: report.summary.clone(),
                        lines: report.lines.clone(),
                    },
                    Purpose::Read,
                ));
                effects
            }
            ConvertWants::Run { .. } => Vec::new(),
        }
    }

    /// Yes to the plan's question: the job starts, in the jobs line.
    pub(super) fn convert_yes(&mut self, wants: ConvertWants) -> Vec<Effect> {
        let ConvertWants::Run { side, plan } = wants else {
            return Vec::new();
        };
        let job = plan.0.job;
        let line = start_line(&plan.0.question().text);
        let now = self.now();
        self.jobs.begin(job, JobKind::Convert, line.clone(), now);
        self.flows.insert(
            job,
            Flow {
                side,
                note: None,
                then: Then::Report,
                confirm: false,
                kind: JobKind::Convert,
            },
        );
        let tools = self.tool_runner();
        let clock = SharedClock(std::sync::Arc::clone(&self.clock));
        self.request(Request::Convert(Box::new(ConvertAsk::Run {
            plan,
            tools,
            clock,
        })));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }

    /// No or Escape on the plan's question: nothing changed.
    pub(super) fn convert_escape(&mut self, wants: ConvertWants) -> Vec<Effect> {
        match wants {
            ConvertWants::Run { .. } => vec![self.say(Message::info("Nothing changed."))],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_line_puts_the_verb_first() {
        assert_eq!(
            start_line("Convert 13 files to JPEG? y or n"),
            "Convert started: 13 files to JPEG"
        );
        assert!(
            start_line("Convert 13 files to JPEG? y or n")
                .chars()
                .count()
                <= 40
        );
    }
}
