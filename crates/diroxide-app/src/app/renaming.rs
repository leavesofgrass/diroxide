//! Batch rename in the app (alpha 5, lane RA): the rules list, the
//! preview list, apply, the editor door, presets and passes, over the
//! engine half in `diroxide_ops::rename_rules`.
//!
//! Reserved by the pre-wiring step (lane P5): the command ids, their
//! catalog rows and menu items, this dispatch, the dialogs' [`RenameWants`]
//! hosted under `Wants::Rename`, [`crate::rename_batch::RenameNote`] (an
//! [`AppEvent`](crate::AppEvent) variant) and
//! [`crate::rename_batch::RenameAsk`] (a [`Request`](crate::Request)
//! variant).
//!
//! The flow, every list and field a dialog under the contract (named,
//! modal, Escape goes back one step):
//!
//! - **Batch rename** opens the rules list over the marked entries (or
//!   the focused one): its first row is the count line ("2 problems. 46
//!   of 50 files will change."), Enter on it opens the preview; then
//!   "Rename now", each rule as a sentence ("Rule 1: Replace IMG_ with
//!   trip-, in the name"), "Add a rule", the counter, the preview's view,
//!   Make names unique, Skip problems, Edit names in editor, Next pass and
//!   the presets. A rule is made or changed by prompts over the list; a
//!   template's placeholders are a list in words, Enter puts one in. After
//!   every change the count line is said once, never the rows.
//! - **The preview** is named by the count line, problems first by
//!   default (the `rename.preview_order` setting). Enter on a row lists
//!   what can be done to it: edit its new name ("Edited by you"), skip it
//!   or take it back, say both names with hidden characters spelled out,
//!   the next and previous problem. Its views: old and new names, new
//!   names only, changes only.
//! - **Rename now** with problems says "Cannot rename: 2 problems. First:
//!   ..." and puts the focus on the first problem in the preview. Else the
//!   question starts on No ("Rename 50 files in D:\photos? No is the
//!   default. y or n"); on yes the folders are read again and the plan is
//!   made again against them, a change refused in words, then one rename
//!   job with one undo record, kept in the journal.
//! - **Edit names in editor** writes `rename-NUMBER.txt` in the state
//!   folder with keyed lines ("trip-01.jpg (was IMG_0001.jpg)", or plain
//!   names by the `rename.editor_lines` setting), opens the editor on it
//!   and asks "Done editing? No is the default. y or n": the question is
//!   there whether or not the editor's program waits (spike R1, Notepad).
//!   The names read back are the rows' own names; a refused buffer is
//!   said with its line, and kept for another try.

use diroxide_config::{RenameEditorLines, RenamePreviewOrder};
use diroxide_core::names::spell_out;
use diroxide_core::{
    Command, CommandId, Effect, Key, KeyCode, Layer, ListModel, ListRow, Message, Move, Place,
    Side, UtcOffset,
};
use diroxide_ops::rename_rules::{
    CaseForm, CharacterClass, Choices, Counter, EditorFormat, End, FindMode, FolderFacts,
    MAX_COUNTER_WIDTH, PLACEHOLDERS, Part, Pass, Position, Presets, Recipe, RecipeError,
    RenameContext, RenamePlan, RenameRow, RenameSource, RowState, Rule, Tidy, editor_text,
    plan_rename_with, read_editor_text,
};

use super::organize::Wants;
use super::{App, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::keymap::Lookup;
use crate::rename_batch::{RenameAsk, RenameNote};
use crate::request::{Request, SpawnKind};

/// The rules list's help.
const RULES_HELP: &str = "Enter on the first row shows the preview. Enter on a rule changes it. Escape closes; nothing is renamed until you say yes.";

/// The preview's help.
const PREVIEW_HELP: &str =
    "Enter on a row: edit its name, skip it, or hear both names. Escape goes back to the rules.";

/// The rules list's first row while the folders are read (24 cells).
pub(super) const READING: &str = "Reading the folder first";

/// Said when Rename now is asked before the folders are read.
const STILL_READING: &str = "Still reading the folder. Try again.";

/// A rename command with no batch open.
pub(super) const NO_BATCH: &str = "No batch rename is open.";

/// The editor question (20 cells before the default's words).
pub(super) const DONE_EDITING: &str = "Done editing";

/// Next problem with none left.
const NO_PROBLEMS: &str = "No problems.";

/// The preview's view: what each row says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum View {
    /// Old and new names, the state first.
    #[default]
    OldAndNew,
    /// New names only, as the panel will show them; rows with a state
    /// keep their words.
    NewOnly,
    /// Only rows that change or have a problem.
    ChangesOnly,
}

impl View {
    fn words(self) -> &'static str {
        match self {
            View::OldAndNew => "old and new names",
            View::NewOnly => "new names only",
            View::ChangesOnly => "changes only",
        }
    }

    fn next(self) -> Self {
        match self {
            View::OldAndNew => View::NewOnly,
            View::NewOnly => View::ChangesOnly,
            View::ChangesOnly => View::OldAndNew,
        }
    }
}

/// One batch rename: what it renames, the rules, the owner's choices,
/// the folders' facts once read, and the plan they make. It travels in
/// its dialogs' purposes, so every open dialog of a batch holds it, and
/// every change is stored back into each of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Batch {
    /// Its number: the job number it took when it opened, which names the
    /// editor buffer.
    id: u64,
    /// The panel it started from.
    side: Side,
    /// The entries, in the panel's order.
    sources: Vec<RenameSource>,
    /// Each folder's facts, once the runtime read them.
    facts: Option<Vec<(Place, FolderFacts)>>,
    /// The rules: the last pass is the one edited.
    recipe: Recipe,
    /// The owner's own names and skips.
    choices: Choices,
    /// The plan from all of the above, or the rule that cannot run.
    plan: Option<Result<RenamePlan, RecipeError>>,
    /// The preview's view.
    view: View,
    /// The saved rule sets.
    presets: Presets,
    /// The editor buffer, while it is kept for another try.
    buffer: Option<Place>,
    /// Whether the folders are being read again before the job.
    rechecking: bool,
}

impl Batch {
    /// The plan, when it can run.
    fn ready_plan(&self) -> Option<&RenamePlan> {
        self.plan.as_ref()?.as_ref().ok()
    }

    /// The rules of the pass being edited.
    fn rules(&self) -> &[Rule] {
        self.recipe
            .passes
            .last()
            .map_or(&[][..], |pass| pass.rules.as_slice())
    }

    /// The pass being edited, made when there is none.
    fn pass_mut(&mut self) -> &mut Pass {
        if self.recipe.passes.is_empty() {
            self.recipe.passes.push(Pass::default());
        }
        let last = self.recipe.passes.len() - 1;
        &mut self.recipe.passes[last]
    }

    /// The count line, or what stands in its place.
    fn count_line(&self) -> String {
        match &self.plan {
            None => READING.to_owned(),
            Some(Err(error)) => error.line(),
            Some(Ok(plan)) => plan.count_line(),
        }
    }

    /// The plan made again from the rules, the choices and the facts.
    fn replan(&mut self, offset: UtcOffset, portable: bool) {
        let Some(facts) = &self.facts else {
            self.plan = None;
            return;
        };
        let mut context = RenameContext::new().with_offset(offset).portable(portable);
        for (folder, folder_facts) in facts {
            context = context.with_folder(folder.clone(), folder_facts.clone());
        }
        self.plan = Some(plan_rename_with(
            &self.sources,
            &self.recipe,
            &context,
            &self.choices,
        ));
    }
}

/// Which prompt of a rule is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// The text or pattern to find.
    Find,
    /// What it becomes.
    With,
    /// The text to insert.
    InsertText,
    /// Where it goes.
    InsertAt,
    /// How many characters to remove.
    RemoveCount,
    /// Where the removal starts.
    RemoveAt,
    /// A template.
    Template,
}

/// A rule being made or changed: the batch, the rule's place in its pass
/// (`None` for a new one), and the rule so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Draft {
    batch: Box<Batch>,
    slot: Option<usize>,
    rule: Rule,
}

/// What a dialog of batch rename is for, hosted under `Wants::Rename`:
/// every one carries its batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RenameWants {
    /// The rules list.
    Rules(Box<Batch>),
    /// The preview list.
    Preview(Box<Batch>),
    /// What can be done to one row of the preview.
    Row { batch: Box<Batch>, index: usize },
    /// A row's new name, typed.
    RowName { batch: Box<Batch>, index: usize },
    /// The kinds of rule, to add one.
    Kinds(Box<Batch>),
    /// What can be done to one rule.
    RuleActions { batch: Box<Batch>, index: usize },
    /// One prompt of a rule.
    RuleField { draft: Draft, step: Step },
    /// The placeholders, Enter puts one into the template.
    Placeholders { draft: Draft, text: String },
    /// The case forms.
    Cases(Draft),
    /// The tidying rules.
    Tidies(Draft),
    /// The counter, typed.
    Counter(Box<Batch>),
    /// The question before the job.
    Confirm(Box<Batch>),
    /// The question after the editor opened.
    EditorDone(Box<Batch>),
    /// The presets list.
    Presets(Box<Batch>),
    /// A preset's name, typed, to save the rules under.
    PresetName(Box<Batch>),
    /// What can be done to one preset.
    PresetActions { batch: Box<Batch>, index: usize },
    /// The question before a preset is deleted.
    DeletePreset { batch: Box<Batch>, index: usize },
}

impl RenameWants {
    /// The purpose's name for the window's test of every purpose: one per
    /// variant, as `organize::Wants::purpose` gives the others.
    pub(super) fn purpose(&self) -> &'static str {
        match self {
            RenameWants::Rules(_) => "RenameRules",
            RenameWants::Preview(_) => "RenamePreview",
            RenameWants::Row { .. } => "RenameRow",
            RenameWants::RowName { .. } => "RenameRowName",
            RenameWants::Kinds(_) => "RenameKinds",
            RenameWants::RuleActions { .. } => "RenameRuleActions",
            RenameWants::RuleField { .. } => "RenameRuleField",
            RenameWants::Placeholders { .. } => "RenamePlaceholders",
            RenameWants::Cases(_) => "RenameCases",
            RenameWants::Tidies(_) => "RenameTidies",
            RenameWants::Counter(_) => "RenameCounter",
            RenameWants::Confirm(_) => "RenameConfirm",
            RenameWants::EditorDone(_) => "RenameEditorDone",
            RenameWants::Presets(_) => "RenamePresets",
            RenameWants::PresetName(_) => "RenamePresetName",
            RenameWants::PresetActions { .. } => "RenamePresetActions",
            RenameWants::DeletePreset { .. } => "RenameDeletePreset",
        }
    }

    /// The batch the dialog belongs to.
    fn batch(&self) -> &Batch {
        match self {
            RenameWants::Rules(batch)
            | RenameWants::Preview(batch)
            | RenameWants::Row { batch, .. }
            | RenameWants::RowName { batch, .. }
            | RenameWants::Kinds(batch)
            | RenameWants::RuleActions { batch, .. }
            | RenameWants::Counter(batch)
            | RenameWants::Confirm(batch)
            | RenameWants::EditorDone(batch)
            | RenameWants::Presets(batch)
            | RenameWants::PresetName(batch)
            | RenameWants::PresetActions { batch, .. }
            | RenameWants::DeletePreset { batch, .. } => batch,
            RenameWants::RuleField { draft, .. }
            | RenameWants::Placeholders { draft, .. }
            | RenameWants::Cases(draft)
            | RenameWants::Tidies(draft) => &draft.batch,
        }
    }

    fn batch_mut(&mut self) -> &mut Batch {
        match self {
            RenameWants::Rules(batch)
            | RenameWants::Preview(batch)
            | RenameWants::Row { batch, .. }
            | RenameWants::RowName { batch, .. }
            | RenameWants::Kinds(batch)
            | RenameWants::RuleActions { batch, .. }
            | RenameWants::Counter(batch)
            | RenameWants::Confirm(batch)
            | RenameWants::EditorDone(batch)
            | RenameWants::Presets(batch)
            | RenameWants::PresetName(batch)
            | RenameWants::PresetActions { batch, .. }
            | RenameWants::DeletePreset { batch, .. } => batch,
            RenameWants::RuleField { draft, .. }
            | RenameWants::Placeholders { draft, .. }
            | RenameWants::Cases(draft)
            | RenameWants::Tidies(draft) => &mut draft.batch,
        }
    }
}

/// A batch rename command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Batch,
    OneByOne,
    InEditor,
    MakeUnique,
    SkipProblems,
    NextProblem,
    PreviousProblem,
    SayBoth,
    NextPass,
    Presets,
}

const ROUTES: [(CommandId, Do); 10] = [
    (ids::BATCH_RENAME, Do::Batch),
    (ids::RENAME_ONE_BY_ONE, Do::OneByOne),
    (ids::RENAME_IN_EDITOR, Do::InEditor),
    (ids::RENAME_MAKE_UNIQUE, Do::MakeUnique),
    (ids::RENAME_SKIP_PROBLEMS, Do::SkipProblems),
    (ids::RENAME_NEXT_PROBLEM, Do::NextProblem),
    (ids::RENAME_PREVIOUS_PROBLEM, Do::PreviousProblem),
    (ids::RENAME_SAY_BOTH, Do::SayBoth),
    (ids::RENAME_NEXT_PASS, Do::NextPass),
    (ids::RENAME_PRESETS, Do::Presets),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

/// "1 file" or "50 files".
fn files(n: usize) -> String {
    diroxide_core::format::count(n as u64, "file", "files")
}

/// The counter in words: "Counter: from 1, step 1, 4 digits".
pub(super) fn counter_words(counter: &Counter) -> String {
    let digits = if counter.width == 1 {
        "digit"
    } else {
        "digits"
    };
    format!(
        "Counter: from {}, step {}, {} {digits}",
        counter.start, counter.step, counter.width
    )
}

/// A typed counter: "1, 1, 4" or "1 1 4", start, step and width.
fn parse_counter(text: &str, per_folder: bool) -> Result<Counter, String> {
    let numbers: Vec<&str> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    let refused = || "Counter not read: type 3 numbers, as 1, 1, 4".to_owned();
    let [start, step, width] = numbers.as_slice() else {
        return Err(refused());
    };
    let start = start.parse::<i64>().map_err(|_| refused())?;
    let step = step.parse::<i64>().map_err(|_| refused())?;
    let width = width.parse::<u8>().map_err(|_| refused())?;
    if width == 0 || width > MAX_COUNTER_WIDTH {
        return Err(format!(
            "Counter not read: digits from 1 to {MAX_COUNTER_WIDTH}"
        ));
    }
    Ok(Counter {
        start,
        step,
        width,
        per_folder,
    })
}

/// A typed position: "start", "end", "3" (after the third character) or
/// "-3" (before the last three).
fn parse_position(text: &str) -> Result<Position, String> {
    let text = text.trim().to_lowercase();
    match text.as_str() {
        "" | "start" => Ok(Position::from_start(0)),
        "end" => Ok(Position::from_end(0)),
        _ => {
            let refused = || "Place not read: type start, end, 3 or -3".to_owned();
            match text.strip_prefix('-') {
                Some(back) => back
                    .parse::<usize>()
                    .map(Position::from_end)
                    .map_err(|_| refused()),
                None => text
                    .parse::<usize>()
                    .map(Position::from_start)
                    .map_err(|_| refused()),
            }
        }
    }
}

/// A position as it is typed back into its field.
fn position_text(at: Position) -> String {
    match (at.from, at.offset) {
        (End::Start, 0) => "start".to_owned(),
        (End::End, 0) => "end".to_owned(),
        (End::Start, n) => n.to_string(),
        (End::End, n) => format!("-{n}"),
    }
}

/// The next part a rule edits: the name, the extension, the whole name.
fn next_part(part: Part) -> Part {
    match part {
        Part::Stem => Part::Extension,
        Part::Extension => Part::Name,
        Part::Name => Part::Stem,
    }
}

/// The tidying rules offered, with their row ids.
fn tidies() -> [(&'static str, Tidy); 8] {
    [
        ("trim", Tidy::Trim),
        ("collapse", Tidy::CollapseSpaces),
        (
            "spaces-underscore",
            Tidy::SpacesTo {
                with: "_".to_owned(),
            },
        ),
        (
            "spaces-hyphen",
            Tidy::SpacesTo {
                with: "-".to_owned(),
            },
        ),
        (
            "underscore-spaces",
            Tidy::ToSpaces {
                from: "_".to_owned(),
            },
        ),
        (
            "digits",
            Tidy::Remove {
                class: CharacterClass::Digits,
            },
        ),
        (
            "punctuation",
            Tidy::Remove {
                class: CharacterClass::Punctuation,
            },
        ),
        (
            "non-ascii",
            Tidy::Remove {
                class: CharacterClass::NonAscii,
            },
        ),
    ]
}

/// The case forms offered, with their row ids.
const CASES: [(&str, CaseForm); 4] = [
    ("lower", CaseForm::Lower),
    ("upper", CaseForm::Upper),
    ("title", CaseForm::Title),
    ("sentence", CaseForm::Sentence),
];

/// The kinds of rule, with their row ids and words.
const KINDS: [(&str, &str); 7] = [
    ("replace", "Replace text"),
    ("pattern", "Replace a pattern"),
    ("insert", "Insert text"),
    ("remove", "Remove characters"),
    ("template", "Make the name from a template"),
    ("case", "Change case"),
    ("tidy", "Tidy spaces and characters"),
];

/// A row's words in a view.
fn row_words(row: &RenameRow, view: View) -> String {
    match (view, &row.state) {
        (View::NewOnly, RowState::Changes) if !row.edited && row.note.is_none() => {
            spell_out(&row.new).into_owned()
        }
        (View::NewOnly, RowState::Unchanged) => spell_out(&row.old).into_owned(),
        _ => row.line(),
    }
}

/// Whether a row is shown in a view.
fn shown_in(row: &RenameRow, view: View) -> bool {
    view != View::ChangesOnly || !matches!(row.state, RowState::Unchanged | RowState::Skipped)
}

impl App {
    /// The batch rename commands; `None` for any other.
    pub(super) fn rename_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        Some(match action {
            Do::Batch => self.batch_rename(command),
            Do::OneByOne => self.rename_one_by_one(command),
            Do::InEditor
            | Do::MakeUnique
            | Do::SkipProblems
            | Do::NextProblem
            | Do::PreviousProblem
            | Do::SayBoth
            | Do::NextPass
            | Do::Presets => self.rename_list_action(command),
        })
    }

    /// The entries a batch acts on: the marked ones in the panel's order,
    /// or the focused one.
    fn batch_sources(&self, side: Side) -> Vec<RenameSource> {
        let panel = self.panel(side);
        let mut entries: Vec<&diroxide_core::Entry> = panel.selected_entries();
        if entries.is_empty() {
            entries.extend(panel.focused_entry());
        }
        entries
            .into_iter()
            .filter_map(|entry| {
                let place = panel.place().join(&entry.name).ok()?;
                Some(RenameSource::new(place, entry.clone()))
            })
            .collect()
    }

    /// Batch rename: the rules list over the marked entries, the folders'
    /// facts and the presets asked of the runtime.
    pub(super) fn batch_rename(&mut self, _command: &Command) -> Vec<Effect> {
        let side = self.active;
        let sources = self.batch_sources(side);
        if sources.is_empty() {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        }
        let id = self.next_job_id().get();
        let batch = Batch {
            id,
            side,
            sources: sources.clone(),
            facts: None,
            recipe: Recipe::of(Vec::new()),
            choices: Choices::new(),
            plan: None,
            view: View::default(),
            presets: Presets::default(),
            buffer: None,
            rechecking: false,
        };
        self.request(Request::RenameBatch(Box::new(RenameAsk::Open {
            batch: id,
            sources,
        })));
        self.show_rules(batch, None)
    }

    /// Rename one by one: today's stepping through the marked entries
    /// (`app/files.rs`), as its own command (question 132).
    fn rename_one_by_one(&mut self, _command: &Command) -> Vec<Effect> {
        let side = self.active;
        let targets = self.panel(side).targets();
        let Some((first, rest)) = targets.split_first() else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        self.rename_prompt(side, first.clone(), rest.to_vec(), Vec::new())
    }

    /// The rename dialog nearest the top, with its place in the stack.
    fn top_rename(&self) -> Option<(usize, &RenameWants)> {
        self.opens
            .iter()
            .enumerate()
            .rev()
            .find_map(|(at, open)| match &open.purpose {
                Purpose::Organize(wants) => match wants.as_ref() {
                    Wants::Rename(wants) => Some((at, wants)),
                    _ => None,
                },
                _ => None,
            })
    }

    /// The open batch, the newest one's latest state.
    fn open_batch(&self) -> Option<Batch> {
        self.top_rename().map(|(_, wants)| wants.batch().clone())
    }

    /// A command of the rules or preview list: Edit names in editor, Make
    /// names unique, Skip problems, Next and Previous problem, Say both
    /// names, Next pass, Rename presets.
    fn rename_list_action(&mut self, command: &Command) -> Vec<Effect> {
        let Some(batch) = self.open_batch() else {
            return vec![self.say(Message::info(NO_BATCH))];
        };
        match route(&command.id) {
            Some(Do::InEditor) => self.edit_in_editor(batch),
            Some(Do::MakeUnique) => self.make_unique(batch),
            Some(Do::SkipProblems) => self.skip_problems(batch),
            Some(Do::NextProblem) => self.step_problem(batch, true),
            Some(Do::PreviousProblem) => self.step_problem(batch, false),
            Some(Do::SayBoth) => self.say_both(&batch),
            Some(Do::NextPass) => self.next_pass(batch),
            Some(Do::Presets) => self.show_presets(batch, None),
            _ => Vec::new(),
        }
    }

    // ----- The lists, built and stored -----

    /// The rules list's model for a batch.
    fn rules_list(&self, batch: &Batch) -> ListModel {
        let mut rows = vec![ListRow::new("count", batch.count_line())];
        rows.push(ListRow::new("apply", "Rename now"));
        let passes = batch.recipe.passes.len();
        for (at, pass) in batch
            .recipe
            .passes
            .iter()
            .enumerate()
            .take(passes.saturating_sub(1))
        {
            let rules = diroxide_core::format::count(pass.rules.len() as u64, "rule", "rules");
            rows.push(ListRow::new(
                format!("pass-{at}"),
                format!("Pass {}, done first: {rules}", at + 1),
            ));
        }
        // A rule's row is its sentence, the action first: the list's own
        // "3 of 10" says where it is, so no number takes the first cells.
        // In a later pass the pass leads.
        for (at, rule) in batch.rules().iter().enumerate() {
            let words = if passes > 1 {
                format!("Pass {passes}: {}", rule.describe())
            } else {
                rule.describe()
            };
            rows.push(ListRow::new(format!("rule-{at}"), words));
        }
        rows.push(ListRow::new("add", "Add a rule"));
        let counter = batch
            .recipe
            .passes
            .last()
            .map(|pass| pass.counter)
            .unwrap_or_default();
        rows.push(ListRow::new("counter", counter_words(&counter)));
        rows.push(ListRow::new(
            "view",
            format!("Preview shows: {}", batch.view.words()),
        ));
        rows.push(ListRow::new("unique", "Make names unique"));
        rows.push(ListRow::new("skip", "Skip problems"));
        rows.push(ListRow::new("editor", "Edit names in editor"));
        rows.push(ListRow::new("pass", "Next pass: more rules after these"));
        rows.push(ListRow::new("presets", "Presets: load, save or delete"));
        let mut list =
            ListModel::new(format!("Batch rename, {}", files(batch.sources.len()))).with_rows(rows);
        list.set_help(RULES_HELP);
        list
    }

    /// The preview rows in the batch's order and view.
    fn preview_rows<'p>(&self, batch: &Batch, plan: &'p RenamePlan) -> Vec<&'p RenameRow> {
        let rows: Vec<&RenameRow> = match self.settings.rename.preview_order {
            RenamePreviewOrder::ProblemsFirst => plan.problems_first(),
            RenamePreviewOrder::Panel => plan.rows().iter().collect(),
        };
        rows.into_iter()
            .filter(|row| shown_in(row, batch.view))
            .collect()
    }

    /// The preview's model for a batch: named by the count line.
    fn preview_list(&self, batch: &Batch) -> ListModel {
        let rows = match batch.ready_plan() {
            Some(plan) => self
                .preview_rows(batch, plan)
                .into_iter()
                .map(|row| ListRow::new(row.index.to_string(), row_words(row, batch.view)))
                .collect(),
            None => Vec::new(),
        };
        let mut list = ListModel::new(batch.count_line()).with_rows(rows);
        list.set_help(PREVIEW_HELP);
        list.set_empty_text("No row in this view.");
        list
    }

    /// Opens the rules list, the focus on a row.
    fn show_rules(&mut self, batch: Batch, focus: Option<&str>) -> Vec<Effect> {
        let mut list = self.rules_list(&batch);
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Rules(Box::new(batch))),
        )
    }

    /// Opens the preview, the focus on a source's row or the first.
    fn show_preview(&mut self, batch: Batch, focus: Option<usize>) -> Vec<Effect> {
        let mut list = self.preview_list(&batch);
        if let Some(index) = focus {
            list.focus_id(&index.to_string());
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Preview(Box::new(batch))),
        )
    }

    /// Stores a batch's new state into every open dialog of it, the
    /// rules list and the preview built again with their focus kept.
    fn store_batch(&mut self, batch: &Batch) {
        for at in 0..self.opens.len() {
            let Purpose::Organize(wants) = &mut self.opens[at].purpose else {
                continue;
            };
            let Wants::Rename(rename) = wants.as_mut() else {
                continue;
            };
            if rename.batch().id != batch.id {
                continue;
            }
            *rename.batch_mut() = batch.clone();
            let kind = match rename {
                RenameWants::Rules(_) => Some(false),
                RenameWants::Preview(_) => Some(true),
                _ => None,
            };
            let mut list = match kind {
                Some(false) => self.rules_list(batch),
                Some(true) => self.preview_list(batch),
                None => continue,
            };
            if let Some((_, Dialog::Choose { list: open, .. })) = self.dialogs.get_mut(at) {
                let focus = open.focused().map(|row| row.id.clone());
                let position = open.focus_index();
                let kept = focus.is_some_and(|id| list.focus_id(&id));
                if !kept && let Some(position) = position {
                    let last = list.len().saturating_sub(1);
                    list.move_focus(Move::To(position.min(last)));
                }
                *open = list;
            }
        }
    }

    /// Closes the dialogs above the batch's dialog of this kind, so it is
    /// on top; whether it was open.
    fn uncover(&mut self, id: u64, preview: bool) -> (bool, Vec<Effect>) {
        let mut effects = Vec::new();
        let found = self.opens.iter().rposition(|open| match &open.purpose {
            Purpose::Organize(wants) => match wants.as_ref() {
                Wants::Rename(RenameWants::Preview(batch)) if preview => batch.id == id,
                Wants::Rename(RenameWants::Rules(batch)) if !preview => batch.id == id,
                _ => false,
            },
            _ => false,
        });
        let Some(at) = found else {
            return (false, effects);
        };
        while self.dialogs.len() > at + 1 {
            effects.extend(self.close_top());
        }
        (true, effects)
    }

    /// Back to the rules list after a change: the dialogs above it closed,
    /// the batch planned again and stored, the focus on a row, and one
    /// line said: `lead` (when given) and the count line.
    fn back_to_rules(&mut self, mut batch: Batch, focus: &str, lead: Option<&str>) -> Vec<Effect> {
        batch.replan(self.clock.offset(), self.settings.portable_names);
        let (open, mut effects) = self.uncover(batch.id, false);
        if !open {
            effects.extend(self.show_rules(batch, Some(focus)));
            return effects;
        }
        self.store_batch(&batch);
        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() {
            list.focus_id(focus);
        }
        let line = match lead {
            Some(lead) => format!("{lead} {}", batch.count_line()),
            None => batch.count_line(),
        };
        effects.push(Effect::Redraw(diroxide_core::Redraw::All));
        effects.push(self.say(Message::status(line)));
        effects
    }

    /// Back to the preview after a change to its rows: the dialogs above
    /// it closed (or it opened over the rules), stored, the focus on a
    /// source's row, and one line said.
    fn back_to_preview(
        &mut self,
        mut batch: Batch,
        focus: Option<usize>,
        line: String,
    ) -> Vec<Effect> {
        batch.replan(self.clock.offset(), self.settings.portable_names);
        let (open, mut effects) = self.uncover(batch.id, true);
        if !open {
            // Opened over the rules list, saying the one line in place of
            // its name: its name is the count line.
            let (_, closed) = self.uncover(batch.id, false);
            effects.extend(closed);
            self.store_batch(&batch);
            let mut list = self.preview_list(&batch);
            if let Some(index) = focus {
                list.focus_id(&index.to_string());
            }
            effects.extend(self.open_dialog_saying(
                Dialog::Choose {
                    list,
                    on_enter: Command::new(ids::ACCEPT),
                },
                Purpose::Organize(Box::new(Wants::Rename(RenameWants::Preview(Box::new(
                    batch,
                ))))),
                Message::status(line),
            ));
            return effects;
        }
        self.store_batch(&batch);
        if let Some(index) = focus
            && let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut()
        {
            list.focus_id(&index.to_string());
        }
        effects.push(Effect::Redraw(diroxide_core::Redraw::All));
        effects.push(self.say(Message::status(line)));
        effects
    }

    // ----- The rules list's rows -----

    /// Enter on a row of the rules list.
    fn rules_row(&mut self, mut batch: Batch, row: &str) -> Vec<Effect> {
        match row {
            "count" => {
                let mut effects = Vec::new();
                if batch.plan.is_none() {
                    effects.push(self.say(Message::status(STILL_READING)));
                    return effects;
                }
                if let Some(Err(error)) = &batch.plan {
                    effects.push(self.say(Message::error(error.line())));
                    return effects;
                }
                let focus = self.first_shown(&batch);
                effects.extend(self.show_preview(batch, focus));
                effects
            }
            "apply" => self.apply(batch),
            "add" => self.show_kinds(batch),
            "counter" => {
                let counter = batch
                    .recipe
                    .passes
                    .last()
                    .map(|pass| pass.counter)
                    .unwrap_or_default();
                let text = format!("{}, {}, {}", counter.start, counter.step, counter.width);
                let field =
                    EntryField::new("Counter, as start, step, digits", CompletionSource::None)
                        .with_text(text);
                self.open_organize(
                    Dialog::edit(field),
                    Wants::Rename(RenameWants::Counter(Box::new(batch))),
                )
            }
            "view" => {
                batch.view = batch.view.next();
                let lead = format!("Preview shows: {}.", batch.view.words());
                self.back_to_rules(batch, "view", Some(&lead))
            }
            "unique" => self.make_unique(batch),
            "skip" => self.skip_problems(batch),
            "editor" => self.edit_in_editor(batch),
            "pass" => self.next_pass(batch),
            "presets" => self.show_presets(batch, None),
            other => {
                if let Some(index) = other
                    .strip_prefix("rule-")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    return self.show_rule_actions(batch, index);
                }
                if let Some(at) = other
                    .strip_prefix("pass-")
                    .and_then(|n| n.parse::<usize>().ok())
                    && let Some(pass) = batch.recipe.passes.get(at)
                {
                    let rules: Vec<String> = pass.rules.iter().map(Rule::describe).collect();
                    let line = if rules.is_empty() {
                        format!("Pass {}: no rules.", at + 1)
                    } else {
                        format!("Pass {}: {}.", at + 1, rules.join("; "))
                    };
                    return vec![self.say(Message::info(line))];
                }
                Vec::new()
            }
        }
    }

    /// The first row the preview shows, as a source position.
    fn first_shown(&self, batch: &Batch) -> Option<usize> {
        let plan = batch.ready_plan()?;
        self.preview_rows(batch, plan).first().map(|row| row.index)
    }

    /// Next pass: the rules so far apply first, and a new pass with its
    /// own counter starts from their names.
    fn next_pass(&mut self, mut batch: Batch) -> Vec<Effect> {
        if batch.rules().is_empty() {
            let line = "Add a rule to this pass first.";
            return vec![self.say(Message::info(line))];
        }
        batch.recipe.passes.push(Pass::default());
        let lead = format!(
            "Pass {}: its rules start from the names so far.",
            batch.recipe.passes.len()
        );
        self.back_to_rules(batch, "add", Some(&lead))
    }

    /// Make names unique: every clash numbered, for review in the preview.
    fn make_unique(&mut self, mut batch: Batch) -> Vec<Effect> {
        let Some(plan) = batch.ready_plan().cloned() else {
            return vec![self.say(Message::status(batch.count_line()))];
        };
        let clashes = plan
            .rows()
            .iter()
            .filter(|row| row.state.is_conflict())
            .count();
        if clashes == 0 {
            return vec![self.say(Message::info("No names clash."))];
        }
        batch.choices = plan.make_unique();
        let focus = plan
            .rows()
            .iter()
            .find(|row| row.state.is_conflict())
            .map(|row| row.index);
        batch.replan(self.clock.offset(), self.settings.portable_names);
        let line = format!("Numbered the clashes. {}", batch.count_line());
        self.back_to_preview(batch, focus, line)
    }

    /// Skip problems: every problem row skipped; the others still rename.
    fn skip_problems(&mut self, mut batch: Batch) -> Vec<Effect> {
        let Some(plan) = batch.ready_plan() else {
            return vec![self.say(Message::status(batch.count_line()))];
        };
        let problems: Vec<usize> = plan
            .rows()
            .iter()
            .filter(|row| row.state.is_problem())
            .map(|row| row.index)
            .collect();
        if problems.is_empty() {
            return vec![self.say(Message::info(NO_PROBLEMS))];
        }
        for index in &problems {
            batch.choices.skipped.insert(*index);
        }
        let lead = format!(
            "Skipped {}.",
            diroxide_core::format::count(problems.len() as u64, "problem", "problems")
        );
        self.back_to_rules(batch, "count", Some(&lead))
    }

    // ----- Apply -----

    /// Rename now: refused in words with the focus on the first problem,
    /// or the question, which starts on No.
    fn apply(&mut self, batch: Batch) -> Vec<Effect> {
        let plan = match &batch.plan {
            None => return vec![self.say(Message::status(STILL_READING))],
            Some(Err(error)) => {
                let line = format!("Cannot rename: {}", error.line());
                return vec![self.say(Message::error(line))];
            }
            Some(Ok(plan)) => plan.clone(),
        };
        if plan.problems() > 0 {
            let line = plan.refusal_line().unwrap_or_default();
            let first = plan.problems_first().first().map(|row| row.index);
            return self.back_to_preview(batch, first, line);
        }
        if let Some(line) = plan.refusal_line() {
            return vec![self.say(Message::info(line))];
        }
        let question = with_default_no(Message::question(&plan.question()));
        let listing = plan
            .rows()
            .iter()
            .filter(|row| row.state.renames())
            .map(RenameRow::line)
            .collect();
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Wants::Rename(RenameWants::Confirm(Box::new(batch))),
        )
    }

    /// Yes: the folders read again before anything is touched.
    fn confirmed(&mut self, mut batch: Batch) -> Vec<Effect> {
        batch.rechecking = true;
        self.store_batch(&batch);
        self.request(Request::RenameBatch(Box::new(RenameAsk::Recheck {
            batch: batch.id,
            sources: batch.sources.clone(),
        })));
        vec![self.say(Message::status("Checking the folder again."))]
    }

    /// The folders as they are now: the plan made again against them;
    /// any change refused in words; else one rename job.
    fn rechecked(
        &mut self,
        mut batch: Batch,
        sources: Vec<RenameSource>,
        facts: Vec<(Place, FolderFacts)>,
    ) -> Vec<Effect> {
        batch.rechecking = false;
        let before = batch.ready_plan().map(RenamePlan::pairs);
        let mut fresh = batch.clone();
        fresh.sources = sources;
        fresh.facts = Some(facts);
        fresh.replan(self.clock.offset(), self.settings.portable_names);
        let plan = match &fresh.plan {
            Some(Ok(plan)) => plan.clone(),
            Some(Err(error)) => {
                let line = format!("Cannot rename: {}", error.line());
                return vec![self.say(Message::error(line))];
            }
            None => return Vec::new(),
        };
        if plan.problems() > 0 {
            let first = plan.problems_first().first().map(|row| row.index);
            let detail = plan
                .problems_first()
                .first()
                .map(|row| row.detail())
                .unwrap_or_default();
            let line = format!("The folder changed since the preview: {detail}.");
            return self.back_to_preview(fresh, first, line);
        }
        if before.as_ref() != Some(&plan.pairs()) {
            let line = "The names changed since the preview. Preview again.".to_owned();
            return self.back_to_preview(fresh, None, line);
        }
        let job = self.next_job_id();
        let request = match plan.request(job) {
            Ok(request) => request,
            Err(refused) => return vec![self.say(Message::info(refused.line()))],
        };
        let pairs: Vec<(String, String)> = plan
            .rows()
            .iter()
            .filter(|row| row.state.renames())
            .map(|row| (row.old.clone(), row.new.clone()))
            .collect();
        let mut effects = self.close_batch(batch.id);
        if let Some(buffer) = batch.buffer.take() {
            self.request(Request::RenameBatch(Box::new(RenameAsk::DropBuffer {
                place: buffer,
            })));
        }
        self.request(Request::RenameBatch(Box::new(RenameAsk::SaveLast(
            batch.recipe.clone(),
        ))));
        effects.extend(self.plan_job(batch.side, request, None, Then::Renamed(pairs), false));
        effects
    }

    /// Closes every dialog of a batch, from the top.
    fn close_batch(&mut self, id: u64) -> Vec<Effect> {
        let mut effects = Vec::new();
        let lowest = self.opens.iter().position(|open| match &open.purpose {
            Purpose::Organize(wants) => match wants.as_ref() {
                Wants::Rename(wants) => wants.batch().id == id,
                _ => false,
            },
            _ => false,
        });
        if let Some(lowest) = lowest {
            while self.dialogs.len() > lowest {
                effects.extend(self.close_top());
            }
        }
        effects
    }

    // ----- The preview's rows -----

    /// The source position of the preview's focused row, when the preview
    /// is on top.
    fn focused_preview_row(&self) -> Option<usize> {
        match (self.opens.last(), self.dialogs.last()) {
            (Some(open), Some((_, Dialog::Choose { list, .. })))
                if matches!(
                    &open.purpose,
                    Purpose::Organize(w) if matches!(w.as_ref(), Wants::Rename(RenameWants::Preview(_)))
                ) =>
            {
                list.focused()?.id.parse().ok()
            }
            _ => None,
        }
    }

    /// Enter on a row of the preview: what can be done to it.
    fn show_row(&mut self, batch: Batch, index: usize) -> Vec<Effect> {
        let Some(row) = batch.ready_plan().and_then(|plan| plan.row(index)) else {
            return Vec::new();
        };
        let name = spell_out(&row.old).into_owned();
        let skip = if row.state == RowState::Skipped {
            "Take this row back"
        } else if row.edited {
            "Back to the rules' name"
        } else {
            "Skip this row"
        };
        let rows = vec![
            ListRow::new("edit", "Edit the new name"),
            ListRow::new("skip", skip),
            ListRow::new("both", "Say both names"),
            ListRow::new("next", "Next problem"),
            ListRow::new("previous", "Previous problem"),
        ];
        let list = ListModel::new(format!("{name}:")).with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Row {
                batch: Box::new(batch),
                index,
            }),
        )
    }

    /// Enter on one of a row's actions.
    fn row_action(&mut self, batch: Batch, index: usize, action: &str) -> Vec<Effect> {
        match action {
            "edit" => {
                let mut effects = self.close_top();
                effects.extend(self.edit_row(batch, index));
                effects
            }
            "skip" => self.toggle_row(batch, index),
            "both" => {
                let mut effects = self.close_top();
                effects.extend(self.say_row_both(&batch, index));
                effects
            }
            "next" | "previous" => {
                let mut effects = self.close_top();
                effects.extend(self.step_problem(batch, action == "next"));
                effects
            }
            _ => Vec::new(),
        }
    }

    /// The field a row's new name is typed in, the old extension's dot
    /// left before the cursor as in the one-entry prompt.
    fn edit_row(&mut self, batch: Batch, index: usize) -> Vec<Effect> {
        let Some(row) = batch.ready_plan().and_then(|plan| plan.row(index)) else {
            return Vec::new();
        };
        let shown = diroxide_core::names::display_name(&row.old).into_owned();
        let new = row.new.clone();
        let mut field = EntryField::new(format!("New name for {shown}"), CompletionSource::None)
            .with_text(new.clone());
        let (_, extension) = diroxide_core::names::split_extension(&new);
        if !extension.is_empty() && extension.len() < new.len() {
            for _ in 0..extension.chars().count() {
                field.left();
            }
        }
        self.open_organize(
            Dialog::edit(field),
            Wants::Rename(RenameWants::RowName {
                batch: Box::new(batch),
                index,
            }),
        )
    }

    /// A row's name typed: the owner's own; the old name back takes the
    /// row back to the rules' name... or leaves it unchanged.
    fn row_named(&mut self, mut batch: Batch, index: usize, text: &str) -> Vec<Effect> {
        let name = text.trim_matches(['\r', '\n']).to_owned();
        if name.is_empty() {
            return vec![self.say(Message::error("Type a name, or Escape to keep this one."))];
        }
        batch.choices.skipped.remove(&index);
        batch.choices.edit(index, name);
        batch.replan(self.clock.offset(), self.settings.portable_names);
        let line = batch
            .ready_plan()
            .and_then(|plan| plan.row(index))
            .map(RenameRow::line)
            .unwrap_or_default();
        self.back_to_preview(batch, Some(index), line)
    }

    /// Space, or Skip this row: skipped, or taken back; the row's new
    /// words said once.
    fn toggle_row(&mut self, mut batch: Batch, index: usize) -> Vec<Effect> {
        let edited = batch.choices.edited.contains_key(&index);
        if edited {
            batch.choices.reset(index);
        } else {
            batch.choices.toggle_skip(index);
        }
        batch.replan(self.clock.offset(), self.settings.portable_names);
        let line = batch
            .ready_plan()
            .and_then(|plan| plan.row(index))
            .map(RenameRow::line)
            .unwrap_or_default();
        self.back_to_preview(batch, Some(index), line)
    }

    /// Next or Previous problem in the preview, opening it when the rules
    /// list is on top.
    fn step_problem(&mut self, batch: Batch, forward: bool) -> Vec<Effect> {
        let Some(plan) = batch.ready_plan().cloned() else {
            return vec![self.say(Message::status(batch.count_line()))];
        };
        let rows = self.preview_rows(&batch, &plan);
        let at = self
            .focused_preview_row()
            .and_then(|index| rows.iter().position(|row| row.index == index));
        let found = if forward {
            RenamePlan::next_problem_in(&rows, at)
        } else {
            RenamePlan::previous_problem_in(&rows, at)
        };
        let Some(found) = found else {
            return vec![self.say(Message::info(NO_PROBLEMS))];
        };
        let index = rows[found].index;
        let line = rows[found].line();
        self.back_to_preview(batch, Some(index), line)
    }

    /// Say both names, of the preview's focused row.
    fn say_both(&mut self, batch: &Batch) -> Vec<Effect> {
        match self.focused_preview_row() {
            Some(index) => self.say_row_both(batch, index),
            None => vec![self.say(Message::info("Open the preview, then a row."))],
        }
    }

    /// One row's old and new names in full, hidden characters spelled out.
    fn say_row_both(&mut self, batch: &Batch, index: usize) -> Vec<Effect> {
        let Some(row) = batch.ready_plan().and_then(|plan| plan.row(index)) else {
            return Vec::new();
        };
        let line = if row.old == row.new {
            format!("Name stays: {}", spell_out(&row.old))
        } else {
            format!("Old: {}. New: {}", spell_out(&row.old), spell_out(&row.new))
        };
        vec![self.say(Message::info(line))]
    }

    /// A key in the preview before the dialog's own keys: Space skips the
    /// focused row or takes it back, and the key that renames a file
    /// edits its new name. `None` when the key is not the preview's.
    pub(super) fn rename_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let index = self.focused_preview_row()?;
        let batch = self.open_batch()?;
        if key.code == KeyCode::Char(' ') && key.mods.is_empty() {
            return Some(self.toggle_row(batch, index));
        }
        let renames = matches!(
            self.keymap.lookup(&[Layer::Panel], key),
            Lookup::Command(id) if id == ids::RENAME
        );
        renames.then(|| self.edit_row(batch, index))
    }

    // ----- Rules: kinds, actions, prompts -----

    fn show_kinds(&mut self, batch: Batch) -> Vec<Effect> {
        let rows = KINDS
            .iter()
            .map(|(id, words)| ListRow::new(*id, *words))
            .collect();
        let list = ListModel::new("Add a rule:").with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Kinds(Box::new(batch))),
        )
    }

    /// A kind chosen: its first prompt or list, for a new rule.
    fn kind_chosen(&mut self, batch: Batch, kind: &str) -> Vec<Effect> {
        let rule = match kind {
            "replace" => Rule::replace("", ""),
            "pattern" => Rule::pattern("", ""),
            "insert" => Rule::Insert {
                text: String::new(),
                at: Position::from_start(0),
                part: Part::Stem,
            },
            "remove" => Rule::Remove {
                at: Position::from_start(0),
                count: 1,
                part: Part::Stem,
            },
            "template" => Rule::template(""),
            "case" => Rule::case(CaseForm::Lower),
            "tidy" => Rule::Tidy {
                tidy: Tidy::Trim,
                part: Part::Stem,
            },
            _ => return Vec::new(),
        };
        let draft = Draft {
            batch: Box::new(batch),
            slot: None,
            rule,
        };
        self.first_prompt(draft)
    }

    /// A rule's first prompt or list.
    fn first_prompt(&mut self, draft: Draft) -> Vec<Effect> {
        match &draft.rule {
            Rule::Replace { .. } => self.rule_field(draft, Step::Find),
            Rule::Insert { .. } => self.rule_field(draft, Step::InsertText),
            Rule::Remove { .. } => self.rule_field(draft, Step::RemoveCount),
            Rule::Template { text, .. } => {
                let text = text.clone();
                self.show_placeholders(draft, text)
            }
            Rule::Case { .. } => self.show_cases(draft),
            Rule::Tidy { .. } => self.show_tidies(draft),
        }
    }

    /// One prompt of a rule, filled with the rule's value so far.
    fn rule_field(&mut self, draft: Draft, step: Step) -> Vec<Effect> {
        let pattern = matches!(
            draft.rule,
            Rule::Replace {
                mode: FindMode::Pattern,
                ..
            }
        );
        let (label, text) = match (&draft.rule, step) {
            (Rule::Replace { find, .. }, Step::Find) => {
                let label = if pattern { "Find pattern" } else { "Find" };
                (label, find.clone())
            }
            (Rule::Replace { with, .. }, Step::With) => {
                let label = if pattern {
                    "Replace with, $1 for a group"
                } else {
                    "Replace with"
                };
                (label, with.clone())
            }
            (Rule::Insert { text, .. }, Step::InsertText) => ("Insert text", text.clone()),
            (Rule::Insert { at, .. }, Step::InsertAt) => {
                ("Insert at, as start, end, 3 or -3", position_text(*at))
            }
            (Rule::Remove { count, .. }, Step::RemoveCount) => {
                ("Remove how many, 0 for all", count.to_string())
            }
            (Rule::Remove { at, .. }, Step::RemoveAt) => {
                ("Remove from, as start, end, 3 or -3", position_text(*at))
            }
            (Rule::Template { text, .. }, Step::Template) => ("Template", text.clone()),
            _ => return Vec::new(),
        };
        let field = EntryField::new(label, CompletionSource::None).with_text(text);
        self.open_organize(
            Dialog::edit(field),
            Wants::Rename(RenameWants::RuleField { draft, step }),
        )
    }

    /// A prompt answered: the next prompt, or the rule kept.
    fn rule_typed(&mut self, mut draft: Draft, step: Step, text: &str) -> Vec<Effect> {
        let mut next = None;
        let refused = match (&mut draft.rule, step) {
            (Rule::Replace { find, .. }, Step::Find) => {
                if text.is_empty() {
                    Some("Type the text to find.".to_owned())
                } else {
                    *find = text.to_owned();
                    next = Some(Step::With);
                    None
                }
            }
            (Rule::Replace { with, .. }, Step::With) => {
                *with = text.to_owned();
                None
            }
            (Rule::Insert { text: insert, .. }, Step::InsertText) => {
                if text.is_empty() {
                    Some("Type the text to insert.".to_owned())
                } else {
                    *insert = text.to_owned();
                    next = Some(Step::InsertAt);
                    None
                }
            }
            (Rule::Insert { at, .. }, Step::InsertAt)
            | (Rule::Remove { at, .. }, Step::RemoveAt) => match parse_position(text) {
                Ok(position) => {
                    *at = position;
                    None
                }
                Err(line) => Some(line),
            },
            (Rule::Remove { count, .. }, Step::RemoveCount) => match text.trim().parse::<usize>() {
                Ok(n) => {
                    *count = n;
                    next = Some(Step::RemoveAt);
                    None
                }
                Err(_) => Some("Count not read: type a number, 0 for all".to_owned()),
            },
            (Rule::Template { text: template, .. }, Step::Template) => {
                if text.is_empty() {
                    Some("Type a template, as {stem}-{counter}".to_owned())
                } else {
                    *template = text.to_owned();
                    None
                }
            }
            _ => None,
        };
        if let Some(line) = refused {
            return vec![self.say(Message::error(line))];
        }
        let mut effects = self.close_top();
        match next {
            Some(step) => effects.extend(self.rule_field(draft, step)),
            None => effects.extend(self.keep_rule(draft)),
        }
        effects
    }

    /// A rule made or changed, back in its pass; the rules list says the
    /// count line once.
    fn keep_rule(&mut self, draft: Draft) -> Vec<Effect> {
        let Draft {
            mut batch,
            slot,
            rule,
        } = draft;
        let pass = batch.pass_mut();
        let at = match slot {
            Some(at) if at < pass.rules.len() => {
                pass.rules[at] = rule;
                at
            }
            _ => {
                pass.rules.push(rule);
                pass.rules.len() - 1
            }
        };
        let focus = format!("rule-{at}");
        self.back_to_rules(*batch, &focus, None)
    }

    /// The placeholders as a list in words; Enter puts one in the template.
    fn show_placeholders(&mut self, draft: Draft, text: String) -> Vec<Effect> {
        let mut rows = vec![ListRow::new("type", "Type the template")];
        rows.extend(
            PLACEHOLDERS
                .iter()
                .map(|(name, words)| ListRow::new(*name, format!("{name}: {words}"))),
        );
        let title = if text.is_empty() {
            "Template, put in:".to_owned()
        } else {
            format!("Template {text}, put in:")
        };
        let list = ListModel::new(title).with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Placeholders { draft, text }),
        )
    }

    /// A placeholder chosen: the template's field with it at the end.
    fn placeholder_chosen(&mut self, mut draft: Draft, text: String, row: &str) -> Vec<Effect> {
        let text = if row == "type" {
            text
        } else {
            format!("{text}{row}")
        };
        if let Rule::Template { text: template, .. } = &mut draft.rule {
            template.clone_from(&text);
        }
        let mut effects = self.close_top();
        effects.extend(self.rule_field(draft, Step::Template));
        effects
    }

    fn show_cases(&mut self, draft: Draft) -> Vec<Effect> {
        let rows = CASES
            .iter()
            .map(|(id, form)| ListRow::new(*id, form.words()))
            .collect();
        let mut list = ListModel::new("Change case to:").with_rows(rows);
        if let Rule::Case { form, .. } = &draft.rule
            && let Some((id, _)) = CASES.iter().find(|(_, f)| f == form)
        {
            list.focus_id(id);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Cases(draft)),
        )
    }

    fn show_tidies(&mut self, draft: Draft) -> Vec<Effect> {
        let rows = tidies()
            .into_iter()
            .map(|(id, tidy)| {
                let words = Rule::Tidy {
                    tidy,
                    part: Part::Stem,
                }
                .describe();
                ListRow::new(
                    id,
                    words.replace(", in the name", "").replace(" the name", ""),
                )
            })
            .collect();
        let list = ListModel::new("Tidy:").with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Tidies(draft)),
        )
    }

    /// What can be done to one rule.
    fn show_rule_actions(&mut self, batch: Batch, index: usize) -> Vec<Effect> {
        let Some(rule) = batch.rules().get(index) else {
            return Vec::new();
        };
        let rows = vec![
            ListRow::new("change", "Change this rule"),
            ListRow::new("part", format!("Edits: {}", rule.part().words())),
            ListRow::new("up", "Move up"),
            ListRow::new("down", "Move down"),
            ListRow::new("remove", "Remove this rule"),
        ];
        let list = ListModel::new(format!("Rule {}:", index + 1)).with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::RuleActions {
                batch: Box::new(batch),
                index,
            }),
        )
    }

    /// Enter on one of a rule's actions.
    fn rule_action(&mut self, mut batch: Batch, index: usize, action: &str) -> Vec<Effect> {
        let count = batch.rules().len();
        if index >= count {
            return Vec::new();
        }
        match action {
            "change" => {
                let rule = batch.rules()[index].clone();
                let mut effects = self.close_top();
                effects.extend(self.first_prompt(Draft {
                    batch: Box::new(batch),
                    slot: Some(index),
                    rule,
                }));
                effects
            }
            "part" => {
                let pass = batch.pass_mut();
                let rule = pass.rules[index].clone();
                let part = next_part(rule.part());
                pass.rules[index] = rule.on(part);
                let lead = format!("Rule {} edits {}.", index + 1, part.words());
                self.back_to_rules(batch, &format!("rule-{index}"), Some(&lead))
            }
            "up" | "down" => {
                let to = if action == "up" {
                    index.checked_sub(1)
                } else {
                    (index + 1 < count).then_some(index + 1)
                };
                let Some(to) = to else {
                    let edge = if action == "up" {
                        "Already the first rule."
                    } else {
                        "Already the last rule."
                    };
                    return vec![self.say(Message::status(edge))];
                };
                batch.pass_mut().rules.swap(index, to);
                let lead = format!("Now rule {}.", to + 1);
                self.back_to_rules(batch, &format!("rule-{to}"), Some(&lead))
            }
            "remove" => {
                batch.pass_mut().rules.remove(index);
                let lead = format!("Removed rule {}.", index + 1);
                let focus = if batch.rules().is_empty() {
                    "add".to_owned()
                } else {
                    format!("rule-{}", index.min(batch.rules().len() - 1))
                };
                self.back_to_rules(batch, &focus, Some(&lead))
            }
            _ => Vec::new(),
        }
    }

    /// The counter typed.
    fn counter_typed(&mut self, mut batch: Batch, text: &str) -> Vec<Effect> {
        let per_folder = batch
            .recipe
            .passes
            .last()
            .is_some_and(|pass| pass.counter.per_folder);
        match parse_counter(text, per_folder) {
            Ok(counter) => {
                batch.pass_mut().counter = counter;
                self.back_to_rules(batch, "counter", None)
            }
            Err(line) => vec![self.say(Message::error(line))],
        }
    }

    // ----- The editor door -----

    fn editor_format(&self) -> EditorFormat {
        match self.settings.rename.editor_lines {
            RenameEditorLines::Keyed => EditorFormat::Keyed,
            RenameEditorLines::Plain => EditorFormat::Plain,
        }
    }

    /// Edit names in editor: the buffer written (or the kept one used
    /// again), then the editor and the question.
    fn edit_in_editor(&mut self, batch: Batch) -> Vec<Effect> {
        if batch.ready_plan().is_some()
            && let Some(buffer) = batch.buffer.clone()
        {
            return self.open_editor(batch, buffer);
        }
        let Some(plan) = batch.ready_plan() else {
            return vec![self.say(Message::status(batch.count_line()))];
        };
        match editor_text(plan, self.editor_format()) {
            Ok(text) => {
                self.request(Request::RenameBatch(Box::new(RenameAsk::WriteBuffer {
                    batch: batch.id,
                    text,
                })));
                Vec::new()
            }
            Err(refused) => vec![
                self.say(Message::error(refused.line())),
                self.say(Message::info(refused.next_step())),
            ],
        }
    }

    /// The editor on the buffer, and the question that waits for it.
    fn open_editor(&mut self, mut batch: Batch, buffer: Place) -> Vec<Effect> {
        batch.buffer = Some(buffer.clone());
        self.store_batch(&batch);
        let mut spawn = self.spawn_for(batch.side, SpawnKind::Editor(buffer.clone()));
        spawn.cursor = Some(buffer.clone());
        spawn.selection = vec![buffer];
        self.request(Request::Spawn(Box::new(spawn)));
        let question = with_default_no(Message::question(DONE_EDITING));
        self.open_organize(
            Dialog::Confirm {
                question,
                listing: Vec::new(),
                on_yes: Command::new(ids::YES),
            },
            Wants::Rename(RenameWants::EditorDone(Box::new(batch))),
        )
    }

    /// The buffer read back: the names become the rows' own, or the
    /// refusal is said and the buffer kept for another try.
    fn buffer_read(&mut self, mut batch: Batch, result: Result<String, String>) -> Vec<Effect> {
        let text = match result {
            Ok(text) => text,
            Err(cause) => {
                let line = format!("Names not read: {cause}. Nothing renamed.");
                return vec![self.say(Message::error(line))];
            }
        };
        let Some(plan) = batch.ready_plan() else {
            return Vec::new();
        };
        match read_editor_text(plan, &text, self.editor_format()) {
            Ok(choices) => {
                batch.choices = choices;
                if let Some(buffer) = batch.buffer.take() {
                    self.request(Request::RenameBatch(Box::new(RenameAsk::DropBuffer {
                        place: buffer,
                    })));
                }
                self.back_to_rules(batch, "count", Some("Names read."))
            }
            Err(refused) => vec![
                self.say(Message::error(format!(
                    "{}. Nothing renamed.",
                    refused.line()
                ))),
                self.say(Message::info(format!(
                    "{} Your edits are kept.",
                    refused.next_step()
                ))),
            ],
        }
    }

    // ----- Presets -----

    fn show_presets(&mut self, batch: Batch, focus: Option<&str>) -> Vec<Effect> {
        let mut rows = vec![ListRow::new("save", "Save these rules as a preset")];
        for (at, preset) in batch.presets.presets.iter().enumerate() {
            let rules =
                diroxide_core::format::count(preset.recipe.rule_count() as u64, "rule", "rules");
            rows.push(ListRow::new(
                format!("preset-{at}"),
                format!("{}: {rules}", preset.name),
            ));
        }
        let saved = batch.presets.presets.len();
        let mut list = ListModel::new(format!("Rename presets, {saved} saved")).with_rows(rows);
        if let Some(focus) = focus {
            list.focus_id(focus);
        }
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::Presets(Box::new(batch))),
        )
    }

    fn presets_row(&mut self, batch: Batch, row: &str) -> Vec<Effect> {
        if row == "save" {
            if batch.recipe.rule_count() == 0 {
                return vec![self.say(Message::info("Add a rule first: no rules to save."))];
            }
            let field = EntryField::new("Preset name", CompletionSource::None);
            return self.open_organize(
                Dialog::edit(field),
                Wants::Rename(RenameWants::PresetName(Box::new(batch))),
            );
        }
        let Some(index) = row
            .strip_prefix("preset-")
            .and_then(|n| n.parse::<usize>().ok())
        else {
            return Vec::new();
        };
        let Some(preset) = batch.presets.presets.get(index) else {
            return Vec::new();
        };
        let name = preset.name.clone();
        let rows = vec![
            ListRow::new("load", format!("Load {name}")),
            ListRow::new("delete", format!("Delete {name}")),
        ];
        let list = ListModel::new(format!("{name}:")).with_rows(rows);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Wants::Rename(RenameWants::PresetActions {
                batch: Box::new(batch),
                index,
            }),
        )
    }

    fn preset_named(&mut self, mut batch: Batch, text: &str) -> Vec<Effect> {
        let name = text.trim();
        if name.is_empty() {
            return vec![self.say(Message::error("Type a name for the preset."))];
        }
        let recipe = batch.recipe.clone();
        batch.presets.save(name, recipe);
        self.request(Request::RenameBatch(Box::new(RenameAsk::SavePresets(
            batch.presets.clone(),
        ))));
        let mut effects = self.close_top();
        self.store_batch(&batch);
        effects.push(self.say(Message::info(format!("Saved preset {name}."))));
        effects
    }

    fn preset_action(&mut self, mut batch: Batch, index: usize, action: &str) -> Vec<Effect> {
        let Some(preset) = batch.presets.presets.get(index).cloned() else {
            return Vec::new();
        };
        match action {
            "load" => {
                batch.recipe = preset.recipe;
                let lead = format!("Loaded {}:", preset.name);
                self.back_to_rules(batch, "count", Some(&lead))
            }
            "delete" => {
                let question =
                    with_default_no(Message::question(&format!("Delete preset {}", preset.name)));
                self.open_organize(
                    Dialog::Confirm {
                        question,
                        listing: Vec::new(),
                        on_yes: Command::new(ids::YES),
                    },
                    Wants::Rename(RenameWants::DeletePreset {
                        batch: Box::new(batch),
                        index,
                    }),
                )
            }
            _ => Vec::new(),
        }
    }

    fn preset_deleted(&mut self, mut batch: Batch, index: usize) -> Vec<Effect> {
        if index >= batch.presets.presets.len() {
            return Vec::new();
        }
        let preset = batch.presets.presets.remove(index);
        self.request(Request::RenameBatch(Box::new(RenameAsk::SavePresets(
            batch.presets.clone(),
        ))));
        let (_, mut effects) = self.uncover(batch.id, false);
        self.store_batch(&batch);
        effects.extend(self.show_presets(batch, Some("save")));
        effects.push(self.say(Message::info(format!("Deleted preset {}.", preset.name))));
        effects
    }

    // ----- The dialogs' answers -----

    /// Enter or a typed answer in one of the batch rename's dialogs.
    pub(super) fn rename_accept(
        &mut self,
        wants: RenameWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            RenameWants::Rules(batch) => match row {
                Some(row) => self.rules_row(*batch, &row),
                None => Vec::new(),
            },
            RenameWants::Preview(batch) => match row.and_then(|r| r.parse::<usize>().ok()) {
                Some(index) => self.show_row(*batch, index),
                None => Vec::new(),
            },
            RenameWants::Row { batch, index } => match row {
                Some(action) => self.row_action(*batch, index, &action),
                None => Vec::new(),
            },
            RenameWants::RowName { batch, index } => self.row_named(*batch, index, text),
            RenameWants::Kinds(batch) => match row {
                Some(kind) => {
                    let mut effects = self.close_top();
                    effects.extend(self.kind_chosen(*batch, &kind));
                    effects
                }
                None => Vec::new(),
            },
            RenameWants::RuleActions { batch, index } => match row {
                Some(action) => self.rule_action(*batch, index, &action),
                None => Vec::new(),
            },
            RenameWants::RuleField { draft, step } => self.rule_typed(draft, step, text),
            RenameWants::Placeholders { draft, text } => match row {
                Some(row) => self.placeholder_chosen(draft, text, &row),
                None => Vec::new(),
            },
            RenameWants::Cases(mut draft) => {
                let Some(form) =
                    row.and_then(|row| CASES.iter().find(|(id, _)| *id == row).map(|(_, f)| *f))
                else {
                    return Vec::new();
                };
                let part = draft.rule.part();
                draft.rule = Rule::Case { form, part };
                self.keep_rule(draft)
            }
            RenameWants::Tidies(mut draft) => {
                let Some(tidy) = row.and_then(|row| {
                    tidies()
                        .into_iter()
                        .find(|(id, _)| *id == row)
                        .map(|(_, tidy)| tidy)
                }) else {
                    return Vec::new();
                };
                let part = draft.rule.part();
                draft.rule = Rule::Tidy { tidy, part };
                self.keep_rule(draft)
            }
            RenameWants::Counter(batch) => self.counter_typed(*batch, text),
            RenameWants::Presets(batch) => match row {
                Some(row) => self.presets_row(*batch, &row),
                None => Vec::new(),
            },
            RenameWants::PresetName(batch) => self.preset_named(*batch, text),
            RenameWants::PresetActions { batch, index } => match row {
                Some(action) => self.preset_action(*batch, index, &action),
                None => Vec::new(),
            },
            RenameWants::Confirm(_)
            | RenameWants::EditorDone(_)
            | RenameWants::DeletePreset { .. } => self.dialog_escape(),
        }
    }

    /// Yes to one of the batch rename's questions.
    pub(super) fn rename_yes(&mut self, wants: RenameWants) -> Vec<Effect> {
        match wants {
            RenameWants::Confirm(batch) => self.confirmed(*batch),
            RenameWants::EditorDone(batch) => match batch.buffer.clone() {
                Some(place) => {
                    self.request(Request::RenameBatch(Box::new(RenameAsk::ReadBuffer {
                        batch: batch.id,
                        place,
                    })));
                    Vec::new()
                }
                None => Vec::new(),
            },
            RenameWants::DeletePreset { batch, index } => self.preset_deleted(*batch, index),
            _ => Vec::new(),
        }
    }

    /// Escape on one of the batch rename's dialogs, after it closed: the
    /// rules list closing ends the batch and says nothing was renamed.
    pub(super) fn rename_escape(&mut self, wants: RenameWants) -> Vec<Effect> {
        match wants {
            RenameWants::Rules(batch) => {
                if let Some(buffer) = batch.buffer {
                    self.request(Request::RenameBatch(Box::new(RenameAsk::DropBuffer {
                        place: buffer,
                    })));
                }
                vec![self.say(Message::info("Nothing renamed."))]
            }
            RenameWants::Confirm(_) => vec![self.say(Message::info("Nothing renamed yet."))],
            RenameWants::EditorDone(_) => vec![self.say(Message::info(
                "Names not read. Edit names in editor again when done.",
            ))],
            RenameWants::DeletePreset { .. } => vec![self.say(Message::info("Nothing deleted."))],
            _ => Vec::new(),
        }
    }

    /// What the runtime answered a [`crate::rename_batch::RenameAsk`]; a
    /// note for a batch whose dialogs are closed is dropped.
    pub(super) fn rename_note(&mut self, note: RenameNote) -> Vec<Effect> {
        let id = match &note {
            RenameNote::Opened { batch, .. }
            | RenameNote::Rechecked { batch, .. }
            | RenameNote::BufferWritten { batch, .. }
            | RenameNote::BufferRead { batch, .. } => *batch,
            RenameNote::PresetsNotSaved(cause) => {
                let line = format!("Presets not saved: {cause}");
                return vec![self.say(Message::error(line))];
            }
        };
        let Some(mut batch) = self
            .opens
            .iter()
            .rev()
            .find_map(|open| match &open.purpose {
                Purpose::Organize(wants) => match wants.as_ref() {
                    Wants::Rename(wants) if wants.batch().id == id => Some(wants.batch().clone()),
                    _ => None,
                },
                _ => None,
            })
        else {
            return Vec::new();
        };
        match note {
            RenameNote::Opened {
                facts,
                presets,
                last,
                ..
            } => {
                batch.facts = Some(facts);
                let mut effects = Vec::new();
                match presets {
                    Ok(presets) => batch.presets = presets,
                    Err(cause) => effects.push(self.say(Message::warning(cause))),
                }
                let untouched = batch.recipe.rule_count() == 0 && batch.recipe.passes.len() == 1;
                let mut lead = None;
                if untouched && let Some(last) = last.filter(|r| r.rule_count() > 0) {
                    batch.recipe = last;
                    lead = Some("The last rules used:");
                }
                batch.replan(self.clock.offset(), self.settings.portable_names);
                self.store_batch(&batch);
                let line = match lead {
                    Some(lead) => format!("{lead} {}", batch.count_line()),
                    None => batch.count_line(),
                };
                effects.push(Effect::Redraw(diroxide_core::Redraw::All));
                effects.push(self.say(Message::status(line)));
                effects
            }
            RenameNote::Rechecked { sources, facts, .. } => self.rechecked(batch, sources, facts),
            RenameNote::BufferWritten { result, .. } => match result {
                Ok(place) => self.open_editor(batch, place),
                Err(cause) => {
                    let line = format!("Names not written for the editor: {cause}.");
                    vec![self.say(Message::error(line))]
                }
            },
            RenameNote::BufferRead { result, .. } => self.buffer_read(batch, result),
            RenameNote::PresetsNotSaved(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
