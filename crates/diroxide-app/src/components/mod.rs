//! Optional components in the app (W4d, R22 commit 41; plan section 5.14):
//! what the app and its runtime say to each other about them, and the
//! models the screens show.
//!
//! - **Manage optional components**, in the Tools menu and the palette:
//!   one row a component, meaning first ("Example plugins: not installed,
//!   74.1 KB, license Apache-2.0 OR MIT"); Enter offers Download, Verify
//!   the files, Remove, and Install from a zip file or a folder.
//! - **Ask first.** Nothing is fetched before a yes: the question says
//!   what needs the component, its size and its license, and starts on
//!   No. A download is a job (`JobKind::Download`) in the jobs line, its
//!   progress said at most every tenth and never more often than the
//!   progress interval; Escape stops it and the next try resumes.
//! - **Get example plugins**, in the Plugins menu: the examples pack's
//!   question, its download, then the plugins found again, so Manage
//!   plugins lists the examples, each not enabled.
//! - **The first-run list** ([`first_run`]) opens once at a first start
//!   from stage 3 of alpha 2 (answer 93), nothing chosen, when it holds
//!   something besides the small example pack; Space marks, Enter asks
//!   once and downloads the marked ones, Escape skips. The "Ask about
//!   components again" setting opens it at the next start, and the Tools
//!   menu's "Ask first-run choices again" opens it now.

use std::path::PathBuf;

use diroxide_components::messages::{self, AskFirst};
use diroxide_components::{Component, Kind, Manifest, Status};
use diroxide_core::{JobId, ListModel, ListRow};

/// Whether the first-run components list is shown at a first start: on
/// from stage 3 of alpha 2, when the SFTP plugin is a component worth a
/// question (answer 93).
pub const FIRST_RUN_SHOWN: bool = true;

/// One component as the screens show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The component's id.
    pub id: String,
    /// Its title, what it is for.
    pub title: String,
    /// Its row in the Manage list, meaning first.
    pub line: String,
    /// Its state.
    pub status: Status,
    /// What it is.
    pub kind: Kind,
    /// The features that need it, by key (`examples`, `sftp`).
    pub features: Vec<String>,
    /// The question asked before it is fetched, with no feature named.
    pub ask: AskFirst,
    /// The question asked when a feature needs it (Get example plugins).
    pub ask_for_feature: Option<AskFirst>,
    /// The lines when it has no build for this computer.
    pub no_build: Vec<String>,
}

/// What the runtime found: every component known, from the embedded list
/// and the owner's own source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The rows, in the list's order.
    pub rows: Vec<Row>,
    /// The ids of the first-run choices (every component but the
    /// examples pack).
    pub first_run: Vec<String>,
    /// The source list's refusals, each a sentence.
    pub refused: Vec<String>,
}

impl Listing {
    /// A row by id.
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// The row of the component a feature needs (`examples`).
    pub fn for_feature(&self, feature: &str) -> Option<&Row> {
        self.rows
            .iter()
            .find(|r| r.features.iter().any(|f| f == feature))
    }
}

/// The rows of a list for this computer, from the components folder.
pub fn listing(manifest: &Manifest, components_dir: &std::path::Path, target: &str) -> Listing {
    let rows = manifest
        .components
        .iter()
        .map(|c| row(c, components_dir, target))
        .collect();
    Listing {
        rows,
        first_run: messages::first_run_choices(manifest)
            .iter()
            .map(|c| c.id.clone())
            .collect(),
        refused: manifest.refused.iter().map(|r| r.line()).collect(),
    }
}

fn row(component: &Component, components_dir: &std::path::Path, target: &str) -> Row {
    let status = component.status_in(components_dir, target);
    let feature = component.features.first().map(String::as_str);
    Row {
        id: component.id.clone(),
        title: component.title.clone(),
        line: messages::manage_row(component, &status, target),
        status,
        kind: component.kind,
        features: component.features.clone(),
        ask: messages::ask_first(component, None, target),
        ask_for_feature: feature.map(|f| messages::ask_first(component, Some(f), target)),
        no_build: messages::no_build(component),
    }
}

/// The Manage list: one row a component, its line meaning first; each
/// row's id is the component's id.
pub fn manage_list(listing: &Listing) -> ListModel {
    let rows = listing
        .rows
        .iter()
        .map(|r| ListRow::new(r.id.clone(), r.line.clone()))
        .collect();
    let mut list = ListModel::new("Optional components").with_rows(rows);
    list.set_empty_text("No optional components are known");
    list.set_help("Up and Down choose, Enter offers what to do, Escape closes.");
    list
}

/// What can be done with a component, as a list: Download (or Download
/// again when damaged), Verify the files, Remove, Install from a zip
/// file, Install from a folder.
pub fn actions_list(row: &Row) -> ListModel {
    let mut rows = Vec::new();
    match row.status {
        Status::Installed => {}
        Status::NoBuild => {}
        Status::NotInstalled => rows.push(ListRow::new("download", "Download")),
        Status::Partial(_) | Status::Damaged(_) => {
            rows.push(ListRow::new("download", "Download again"));
        }
    }
    if row.status != Status::NotInstalled && row.status != Status::NoBuild {
        rows.push(ListRow::new("verify", "Verify the files"));
        rows.push(ListRow::new("remove", "Remove"));
    }
    rows.push(ListRow::new("install-zip", "Install from a zip file"));
    rows.push(ListRow::new("install-folder", "Install from a folder"));
    let mut list = ListModel::new(row.title.clone()).with_rows(rows);
    list.set_help("Up and Down choose, Enter does it, Escape goes back.");
    list
}

/// The first-run list: every first-run choice, nothing chosen; empty
/// while the list holds only the examples pack.
pub fn first_run(listing: &Listing) -> ListModel {
    let rows = listing
        .rows
        .iter()
        .filter(|r| listing.first_run.contains(&r.id))
        .map(|r| ListRow::new(r.id.clone(), r.line.clone()))
        .collect();
    let mut list = ListModel::new("Optional components to get now").with_rows(rows);
    list.set_empty_text("No optional components to choose from yet");
    list.set_help(
        "Space marks a component to download, Enter downloads the marked ones, Escape skips.",
    );
    list
}

/// Whether the first-run list is shown at this start: once, when it
/// holds something ([`FIRST_RUN_SHOWN`] is the stage switch).
pub fn show_first_run(listing: &Listing, shown_before: bool) -> bool {
    FIRST_RUN_SHOWN && !shown_before && !listing.first_run.is_empty()
}

/// What the app asks the runtime about components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentRequest {
    /// Read the list and each component's state; answered with
    /// [`ComponentEvent::Listed`].
    List,
    /// Download a component the owner said yes to, as a job.
    Download {
        /// The job.
        job: JobId,
        /// The component.
        id: String,
        /// The progress interval in seconds.
        interval: u64,
    },
    /// Check every file of a component.
    Verify {
        /// The component.
        id: String,
    },
    /// List what Remove would take away, before the question.
    PlanRemove {
        /// The component.
        id: String,
    },
    /// Remove a component's own files, after the owner's yes.
    Remove {
        /// The component.
        id: String,
    },
    /// Install a component from a zip file or a folder, checked against
    /// the same pins.
    Install {
        /// The job.
        job: JobId,
        /// The component.
        id: String,
        /// The zip file or folder.
        from: PathBuf,
    },
}

/// What the runtime tells the app about components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentEvent {
    /// The list and each component's state.
    Listed(Box<Listing>),
    /// A download's or an install's line, as the progress voice says it.
    Progress {
        /// The job.
        job: JobId,
        /// The line.
        line: String,
    },
    /// A download or an install ended: the lines to say, the first the
    /// verdict ("Ready: diro-examples 0.1.0."), and whether it worked.
    Finished {
        /// The job.
        job: JobId,
        /// The component.
        id: String,
        /// Whether it is ready now.
        ready: bool,
        /// The lines, meaning first.
        lines: Vec<String>,
    },
    /// A Verify's lines: "Files check out: Example plugins." or each file.
    Verified {
        /// The component.
        id: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// What Remove would take away: each path, then the count.
    RemovalPlanned {
        /// The component.
        id: String,
        /// Each path, hidden characters spelled out.
        lines: Vec<String>,
        /// "Removes 2 files. Yours stay."
        summary: String,
        /// "Remove Example plugins? y or n"
        question: String,
    },
    /// A Remove ended.
    Removed {
        /// The component.
        id: String,
        /// The sentence.
        line: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn the_embedded_list_reads_meaning_first() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::embedded();
        let listing = listing(&manifest, dir.path(), diroxide_components::host_target());
        let examples = listing.row("diro-examples").unwrap();
        assert!(
            examples
                .line
                .starts_with("Example plugins: not installed, ")
        );
        assert!(within_cells(
            &examples.line,
            "Example plugins: not installed"
        ));
        let list = manage_list(&listing);
        assert!(
            list.announce_focus()
                .starts_with("Example plugins: not installed")
        );
        let actions = actions_list(examples);
        let labels: Vec<&str> = actions.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Download",
                "Install from a zip file",
                "Install from a folder"
            ]
        );
        let ask = examples.ask_for_feature.as_ref().unwrap();
        assert_eq!(ask.question, "Download it now? y or n");
        assert!(ask.lines[0].starts_with("Getting example plugins needs Example plugins"));
        for line in ask.rows() {
            assert!(line.chars().count() <= 60, "{line}");
        }
    }

    #[test]
    fn the_first_run_list_is_shown_once_when_it_holds_something() {
        let dir = tempfile::tempdir().unwrap();
        let mut listing = listing(&Manifest::embedded(), dir.path(), "any");
        // Only the examples pack, as before the SFTP plugin joined the
        // list: nothing to ask.
        listing.rows.retain(|r| r.id == "diro-examples");
        assert!(
            !listing.first_run.iter().any(|id| id == "diro-examples"),
            "the examples are not offered"
        );
        listing.first_run.clear();
        assert_eq!(
            first_run(&listing).announce_focus(),
            "No optional components to choose from yet"
        );
        assert!(!show_first_run(&listing, false), "nothing to ask");
        let mut more = listing.clone();
        more.first_run.push("diro-examples".into());
        assert!(show_first_run(&more, false));
        assert!(!show_first_run(&more, true), "once");
        let list = first_run(&more);
        assert_eq!(list.len(), 1);
        assert!(list.selected_ids().is_empty(), "nothing chosen");
    }
}
