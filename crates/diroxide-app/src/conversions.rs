//! Conversions in the app and the command line (alpha 4, lane AW, R30
//! commits 62 and 63; R28 5.3): the recipe book as this computer has it,
//! the type registry as the Convert engine's [`Typing`], the recipes a
//! selection can take with whether each can run here, and the messages
//! between the app's Convert command and its runtime.
//!
//! The book is the built-in recipes, then the owner's `conversions.toml`
//! (a recipe there with a built-in id replaces it in its place). It is
//! read again for every Convert, so a changed file applies at once. The
//! engine is CL's ([`diroxide_ops::convert`]), called as it is.

use std::sync::Arc;

use diroxide_config::Paths;
use diroxide_core::{Entry, EntryKind, JobId, Message, Place, SystemClock};
use diroxide_filetype::TypeRegistry;
use diroxide_ops::JobReport;
use diroxide_ops::convert::{
    Availability, ConvertPlan, FileKind, KeptReport, Recipe, RecipeBook, RecipeSource, Typing,
};
use diroxide_vfs::{CancelToken, ToolRunner};

use crate::request::{SharedClock, SharedTools};

/// The folder in the state folder where the last 20 conversion reports
/// are kept.
pub const REPORTS_FOLDER: &str = "conversions";

/// The recipe book of this computer: the built-in recipes, then the
/// owner's `conversions.toml` from the config folder when there is one.
/// Every problem met is kept in the book, the recipe's id first.
pub fn recipe_book(paths: Option<&Paths>) -> RecipeBook {
    let mut book = RecipeBook::built_in();
    if let Some(paths) = paths {
        let config = diroxide_config::load(paths);
        book.add_serialized(&config.conversions.recipes, &RecipeSource::User);
    }
    book
}

/// The kept reports' shelf in the state folder.
pub fn report_shelf(paths: &Paths) -> diroxide_ops::convert::ReportShelf {
    diroxide_ops::convert::ReportShelf::new(paths.state_file(REPORTS_FOLDER))
}

/// The type registry as the Convert engine asks it: a file's type from
/// its name and first bytes, and the groups it is in, so `{typed_input}`
/// says what the file is whatever its name says.
#[derive(Clone, Debug)]
pub struct RegistryTyping(pub Arc<TypeRegistry>);

impl Typing for RegistryTyping {
    fn kind_of(&self, name: &str, head: &[u8]) -> FileKind {
        let type_id = self.0.detect(name, head).map(|found| found.type_id);
        let mut entry = Entry::new(name, EntryKind::File);
        if let Some(id) = &type_id
            && let Some(at) = self.0.types().iter().position(|t| &t.id == id)
        {
            entry.type_index = u16::try_from(at + 1).unwrap_or(0);
        }
        let groups = self
            .0
            .groups_of(&entry, &SystemClock::utc())
            .map(|groups| groups.iter().map(|g| g.id.clone()).collect())
            .unwrap_or_default();
        FileKind { type_id, groups }
    }
}

/// The type registry handed to a worker; two are equal when they are the
/// same registry.
#[derive(Clone, Debug)]
pub struct SharedTypes(pub Arc<TypeRegistry>);

impl PartialEq for SharedTypes {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One recipe in the Convert list: its id, its name and sentence, and its
/// line when it cannot run here ("Video to MP4, needs libx264").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeRow {
    /// The recipe's id.
    pub id: String,
    /// Its name: "Image to JPEG".
    pub name: String,
    /// Its sentence: "Makes a JPEG copy of each picture, quality 90."
    pub about: String,
    /// Why it cannot run here, as its row says it, the name first:
    /// "Video to MP4, needs libx264"; `None` when it is ready.
    pub needs: Option<String>,
}

/// The recipes a selection can take, the ready ones first in the book's
/// order, then those whose tool or encoder is missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipeChoice {
    /// The rows.
    pub rows: Vec<RecipeRow>,
    /// Whether some selected files have no recipe here, so the list
    /// offers the recipes that take most of them.
    pub partial: bool,
    /// The book's problems, each a sentence, the recipe's id first.
    pub problems: Vec<String>,
}

/// The extension of a name, lowercase, without its dot.
fn extension(name: &str) -> Option<String> {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(ext.to_ascii_lowercase()),
        _ => None,
    }
}

/// The recipes that take the files named (by their names and the type
/// registry, nothing read), in the book's order with whether each can
/// run: those that take every file, or with none such, those that take
/// the most. A folder among the sources takes every recipe, since the
/// plan says what it leaves out. Asking whether a tool is there may run
/// `ffmpeg -encoders` through `tools` once, never more.
pub fn recipes_for(
    book: &RecipeBook,
    names: &[(String, bool)],
    typing: &dyn Typing,
    tools: &dyn ToolRunner,
    cancel: &CancelToken,
) -> RecipeChoice {
    let files: Vec<(Option<String>, FileKind)> = names
        .iter()
        .filter(|(_, folder)| !folder)
        .map(|(name, _)| (extension(name), typing.kind_of(name, &[])))
        .collect();
    let any_folder = names.iter().any(|(_, folder)| *folder);
    let taken = |recipe: &Recipe| {
        files
            .iter()
            .filter(|(ext, kind)| {
                recipe.takes(ext.as_deref(), kind.type_id.as_deref(), &kind.groups)
            })
            .count()
    };
    let counts: Vec<usize> = book.recipes().iter().map(taken).collect();
    let most = counts.iter().copied().max().unwrap_or(0);
    let wanted = if any_folder && files.is_empty() {
        0
    } else if counts.contains(&files.len()) {
        files.len()
    } else {
        most
    };
    let partial = !files.is_empty() && wanted < files.len();
    let mut ready = Vec::new();
    let mut missing = Vec::new();
    for ((recipe, state), count) in book.availability(tools, cancel).into_iter().zip(&counts) {
        let keep = if any_folder && files.is_empty() {
            true
        } else {
            wanted > 0 && *count == wanted
        };
        if !keep {
            continue;
        }
        let row = RecipeRow {
            id: recipe.id.clone(),
            name: recipe.name.clone(),
            about: recipe.about.clone(),
            needs: (state != Availability::Ready).then(|| state.line(recipe)),
        };
        if row.needs.is_none() {
            ready.push(row);
        } else {
            missing.push(row);
        }
    }
    ready.extend(missing);
    RecipeChoice {
        rows: ready,
        partial,
        problems: book.problems().to_vec(),
    }
}

/// A plan the owner confirmed, handed to the runtime to run; two are
/// equal when they are the same job's.
#[derive(Clone, Debug)]
pub struct PlanToRun(pub Box<ConvertPlan>);

impl PartialEq for PlanToRun {
    fn eq(&self, other: &Self) -> bool {
        self.0.job == other.0.job
    }
}

impl Eq for PlanToRun {}

/// What the Convert command asks of the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum ConvertAsk {
    /// The recipes that take these places, with whether each can run.
    Recipes {
        /// The selection, each with whether it is a folder.
        sources: Vec<(Place, bool)>,
        /// The type registry.
        types: SharedTypes,
        /// The tools runner.
        tools: SharedTools,
        /// What the list is answered for: the selection and the target,
        /// kept for the plan.
        target: Place,
    },
    /// Plan a conversion with a recipe, touching nothing.
    Plan {
        /// The job.
        job: JobId,
        /// The files and folders.
        sources: Vec<Place>,
        /// The folder the outputs go to: the other panel's.
        destination: Place,
        /// The recipe's id.
        recipe: String,
        /// The type registry.
        types: SharedTypes,
        /// The tools runner.
        tools: SharedTools,
    },
    /// Run a plan the owner confirmed.
    Run {
        /// The plan.
        plan: PlanToRun,
        /// The tools runner.
        tools: SharedTools,
        /// The app's clock, for the report's time.
        clock: SharedClock,
    },
    /// The kept reports, newest first.
    Reports,
}

/// What the runtime answers the Convert command.
#[derive(Debug)]
pub enum ConvertNote {
    /// The recipes for a selection, or why there are none.
    Recipes {
        /// The selection.
        sources: Vec<Place>,
        /// The target folder.
        target: Place,
        /// The list's rows, or the sentence instead.
        result: Result<RecipeChoice, Message>,
    },
    /// A plan, or why there is none.
    Planned {
        /// The job.
        job: JobId,
        /// The plan.
        result: Result<Box<ConvertPlan>, Message>,
    },
    /// A conversion ended: its report's rows, failures first, and the
    /// engine's report for the jobs list and Undo.
    Finished {
        /// The engine's report: the outputs made, the failures, the undo
        /// record that names every output.
        report: Box<JobReport>,
        /// "Report: 1 failed, 12 converted".
        summary: String,
        /// The report's rows.
        lines: Vec<String>,
    },
    /// The kept reports, newest first.
    Reports(Result<Vec<KeptReport>, Message>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CL's request 6: the placeholders the config checks a recipe by
    /// are the engine's own.
    #[test]
    fn the_config_and_the_engine_share_the_placeholders() {
        assert_eq!(
            diroxide_config::recipes::RECIPE_PLACEHOLDERS.to_vec(),
            diroxide_ops::convert::placeholders::PLACEHOLDERS.to_vec()
        );
    }

    /// The registry types a file by its name, with its group.
    #[test]
    fn the_registry_types_a_picture_by_its_name() {
        let typing = RegistryTyping(Arc::new(TypeRegistry::builtin()));
        let kind = typing.kind_of("photo.png", &[]);
        assert_eq!(kind.type_id.as_deref(), Some("png"));
        assert!(kind.groups.contains(&"images".to_owned()), "{kind:?}");
    }
}
