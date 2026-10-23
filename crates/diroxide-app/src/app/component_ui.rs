//! Optional components in the app (W4d, R22 commit 41): Manage optional
//! components, the ask-first question, a download as a job in the jobs
//! line, Verify, Remove with its listing first, Install from a zip file or
//! a folder, Get example plugins, and the first-run list (built, shown
//! from stage 3).
//!
//! Nothing is fetched before a yes: the question names what needs the
//! component, its size and its license, and starts on No; the runtime
//! makes the download's consent from that yes alone. Progress is the
//! components crate's progress voice, a tenth at a time and never more
//! often than the progress interval; Escape stops a download, and the
//! next try resumes.

use std::collections::HashMap;

use diroxide_components::Status;
use diroxide_components::messages::{self, AskFirst};
use diroxide_core::{Command, Effect, JobId, JobKind, ListModel, ListRow, Message, Place, Redraw};

use super::{App, Purpose};
use crate::commands::ids;
use crate::components::{self as model, ComponentEvent, ComponentRequest, Listing};
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::FinishedJob;
use crate::plugins::PluginRequest;
use crate::request::Request;

/// What to do when the list arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    /// Open Manage optional components.
    Manage,
    /// Ask about the example plugins.
    Examples,
    /// Show the first-run list.
    FirstRun,
}

/// What the app keeps for components.
#[derive(Clone, Debug, Default)]
pub(super) struct ComponentState {
    /// The last list, with each component's state.
    listing: Option<Listing>,
    /// What waits for the next list.
    next: Option<Next>,
    /// The downloads and installs running, by job: the component's id.
    running: HashMap<JobId, String>,
}

/// What a components dialog is for.
#[derive(Clone, Debug)]
pub(super) enum Wants {
    /// Manage optional components.
    Manage,
    /// One component's actions.
    Actions { id: String },
    /// The ask-first question.
    AskFirst { id: String },
    /// The removal's question, after its listing.
    Remove { id: String },
    /// The place to install from.
    InstallFrom { id: String },
    /// Lines to read: a Verify's files.
    Read,
    /// The first-run list.
    FirstRun,
}

impl App {
    /// The components' commands; `None` for commands that are not theirs.
    pub(super) fn components_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let next = if command.id == ids::COMPONENTS_MANAGE {
            Next::Manage
        } else if command.id == ids::PLUGINS_EXAMPLES {
            Next::Examples
        } else if command.id == ids::COMPONENTS_FIRST_RUN {
            Next::FirstRun
        } else {
            return None;
        };
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        self.components.next = Some(next);
        self.request(Request::Components(Box::new(ComponentRequest::List)));
        let line = match next {
            Next::Manage => "Reading the optional components",
            Next::Examples => "Looking for the example plugins",
            Next::FirstRun => {
                self.settings.components_first_run_shown = false;
                self.request(Request::SaveSetting {
                    key: "components_first_run_shown".to_owned(),
                    value: Some(toml::Value::Boolean(false)),
                });
                "Reading the first-run choices"
            }
        };
        Some(vec![self.say(Message::status(line))])
    }

    /// An event of the runtime's about components.
    pub(super) fn component_event(&mut self, event: ComponentEvent) -> Vec<Effect> {
        match event {
            ComponentEvent::Listed(listing) => self.components_listed(*listing),
            ComponentEvent::Progress { job, line } => {
                self.jobs.update(job, line.clone());
                vec![
                    Effect::Redraw(Redraw::JobsLine),
                    Effect::Announce(Message::progress(line)),
                ]
            }
            ComponentEvent::Finished {
                job,
                id,
                ready,
                lines,
            } => self.component_finished(job, &id, ready, lines),
            ComponentEvent::Verified { lines, .. } => self.say_lines(lines, "Verify"),
            ComponentEvent::RemovalPlanned {
                id,
                lines,
                summary,
                question,
            } => {
                let mut listing = lines;
                listing.push(summary);
                self.open_dialog(
                    Dialog::Confirm {
                        question: with_default_no(Message::question(
                            question.trim_end_matches(" y or n"),
                        )),
                        listing,
                        on_yes: Command::new(ids::YES),
                    },
                    Purpose::Component(Box::new(Wants::Remove { id })),
                )
            }
            ComponentEvent::Removed { line, .. } => {
                self.request(Request::Components(Box::new(ComponentRequest::List)));
                self.request(Request::Plugins(Box::new(PluginRequest::Scan)));
                vec![self.say(Message::info(line))]
            }
        }
    }

    /// Says the first line; more lines open as a list to read.
    fn say_lines(&mut self, lines: Vec<String>, title: &str) -> Vec<Effect> {
        let Some(first) = lines.first().cloned() else {
            return Vec::new();
        };
        if lines.len() == 1 || !self.dialogs.is_empty() {
            return vec![self.say(Message::info(first))];
        }
        let rows = lines
            .iter()
            .enumerate()
            .map(|(i, line)| ListRow::new(i.to_string(), line.clone()))
            .collect();
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help("Up and Down read a line, Escape closes.");
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Component(Box::new(Wants::Read)),
            Message::info(first),
        )
    }

    fn components_listed(&mut self, listing: Listing) -> Vec<Effect> {
        let next = self.components.next.take();
        self.components.listing = Some(listing.clone());
        let mut effects = Vec::new();
        // An open Manage list shows the new states.
        let index = self.opens.iter().rposition(
            |o| matches!(&o.purpose, Purpose::Component(w) if matches!(**w, Wants::Manage)),
        );
        if let Some(index) = index
            && let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index)
        {
            list.set_rows(model::manage_list(&listing).rows().to_vec());
            effects.push(Effect::Redraw(Redraw::All));
        }
        match next {
            None => {}
            Some(Next::Manage) => {
                let list = model::manage_list(&listing);
                effects.extend(self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Purpose::Component(Box::new(Wants::Manage)),
                ));
            }
            Some(Next::Examples) => effects.extend(self.ask_examples(&listing)),
            Some(Next::FirstRun) => {
                let list = model::first_run(&listing);
                effects.extend(self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Purpose::Component(Box::new(Wants::FirstRun)),
                ));
            }
        }
        effects
    }

    /// Get example plugins: already here, no build, or the question.
    fn ask_examples(&mut self, listing: &Listing) -> Vec<Effect> {
        let Some(row) = listing.for_feature("examples").cloned() else {
            return vec![self.say(Message::info(
                "No example plugins are in the components list.",
            ))];
        };
        match row.status {
            Status::Installed => {
                self.request(Request::Plugins(Box::new(PluginRequest::Scan)));
                vec![self.say(Message::info(format!(
                    "Already installed: {}. Manage plugins lists them.",
                    row.title
                )))]
            }
            Status::NoBuild => {
                let text = row.no_build.join(" ");
                vec![self.say(Message::info(text))]
            }
            _ => {
                let ask = row.ask_for_feature.clone().unwrap_or(row.ask.clone());
                self.ask_first(&row.id, &ask)
            }
        }
    }

    /// The ask-first question: its first line and the question as the
    /// question, starting on No; the size and the license under it.
    fn ask_first(&mut self, id: &str, ask: &AskFirst) -> Vec<Effect> {
        let (first, rest) = match ask.lines.split_first() {
            Some((first, rest)) => (format!("{first} "), rest.to_vec()),
            None => (String::new(), Vec::new()),
        };
        let question = with_default_no(Message::new(
            format!("{first}{}", ask.question),
            diroxide_core::Importance::High,
            diroxide_core::MessageKind::Question,
        ));
        let spoken = Message::new(
            {
                let mut words = vec![question.text.clone()];
                words.extend(rest.iter().cloned());
                words.join(" ")
            },
            diroxide_core::Importance::High,
            diroxide_core::MessageKind::Question,
        );
        self.open_dialog_saying(
            Dialog::Confirm {
                question,
                listing: rest,
                on_yes: Command::new(ids::YES),
            },
            Purpose::Component(Box::new(Wants::AskFirst { id: id.to_owned() })),
            spoken,
        )
    }

    fn component_row(&self, id: &str) -> Option<model::Row> {
        self.components.listing.as_ref()?.row(id).cloned()
    }

    /// Starts a download or an install as a job in the jobs line.
    fn component_job(&mut self, id: &str, from: Option<std::path::PathBuf>) -> Vec<Effect> {
        let title = self
            .component_row(id)
            .map_or_else(|| id.to_owned(), |r| r.title);
        let job = self.next_job_id();
        let line = match &from {
            Some(_) => format!("Installing {title}"),
            None => format!("Downloading {title}"),
        };
        let now = self.now();
        self.jobs.begin(job, JobKind::Download, line.clone(), now);
        self.components.running.insert(job, id.to_owned());
        let request = match from {
            Some(from) => ComponentRequest::Install {
                job,
                id: id.to_owned(),
                from,
            },
            None => ComponentRequest::Download {
                job,
                id: id.to_owned(),
                interval: self.settings.progress_interval_seconds,
            },
        };
        self.request(Request::Components(Box::new(request)));
        vec![
            Effect::Redraw(Redraw::JobsLine),
            self.say(Message::info(line)),
        ]
    }

    fn component_finished(
        &mut self,
        job: JobId,
        id: &str,
        ready: bool,
        lines: Vec<String>,
    ) -> Vec<Effect> {
        self.components.running.remove(&job);
        let summary = lines.first().cloned().unwrap_or_default();
        self.jobs.finished(FinishedJob {
            job,
            kind: JobKind::Download,
            summary: summary.clone(),
            failed: if ready { Vec::new() } else { lines.clone() },
            ..FinishedJob::default()
        });
        self.request(Request::Components(Box::new(ComponentRequest::List)));
        if ready {
            // Example plugins or a plugin component: the plugins are found
            // again, each not enabled.
            self.request(Request::Plugins(Box::new(PluginRequest::Scan)));
        }
        let _ = id;
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        for (i, line) in lines.into_iter().enumerate() {
            let message = match (i, ready) {
                (0, true) => Message::done(line),
                (0, false) => Message::warning(line),
                _ => Message::info(line),
            };
            effects.push(self.say(message));
        }
        effects
    }

    /// Cancel's words for a download: "Download: cancelling".
    pub(super) fn component_cancel_words(&self, job: JobId) -> Option<String> {
        self.components
            .running
            .contains_key(&job)
            .then(|| "Download: cancelling".to_owned())
    }

    // ---- The hooks of the dialogs ----

    /// Enter in a components dialog.
    pub(super) fn component_accept(
        &mut self,
        wants: Wants,
        row: Option<String>,
        text: String,
    ) -> Vec<Effect> {
        match wants {
            Wants::Manage => {
                let Some(row) = row.and_then(|id| self.component_row(&id)) else {
                    return Vec::new();
                };
                let list = model::actions_list(&row);
                self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::ACCEPT),
                    },
                    Purpose::Component(Box::new(Wants::Actions { id: row.id.clone() })),
                )
            }
            Wants::Actions { id } => {
                let Some(action) = row else {
                    return Vec::new();
                };
                let Some(component) = self.component_row(&id) else {
                    return Vec::new();
                };
                match action.as_str() {
                    "download" => self.ask_first(&id, &component.ask),
                    "verify" => {
                        self.request(Request::Components(Box::new(ComponentRequest::Verify {
                            id,
                        })));
                        vec![self.say(Message::status(messages::CHECKING))]
                    }
                    "remove" => {
                        self.request(Request::Components(Box::new(
                            ComponentRequest::PlanRemove { id },
                        )));
                        Vec::new()
                    }
                    "install-zip" | "install-folder" => {
                        let what = if action == "install-zip" {
                            "a zip file"
                        } else {
                            "a folder"
                        };
                        let field = EntryField::new(
                            format!("Install {} from {what}", component.title),
                            CompletionSource::Paths,
                        );
                        self.open_dialog(
                            Dialog::edit(field),
                            Purpose::Component(Box::new(Wants::InstallFrom { id })),
                        )
                    }
                    _ => Vec::new(),
                }
            }
            Wants::InstallFrom { id } => {
                let here = self.panel(self.active).place().clone();
                let place = match Place::resolve(text.trim(), &here) {
                    Ok(place) => place,
                    Err(error) => {
                        return vec![self.say(Message::from_error("use", text.trim(), &error))];
                    }
                };
                let Some(local) = place.local_path().map(std::path::PathBuf::from) else {
                    return vec![self.say(Message::info(
                        "Type a zip file or a folder on this computer.",
                    ))];
                };
                let mut effects = self.close_all();
                effects.extend(self.component_job(&id, Some(local)));
                effects
            }
            Wants::Read | Wants::FirstRun => Vec::new(),
            Wants::AskFirst { .. } | Wants::Remove { .. } => Vec::new(),
        }
    }

    /// Yes in a components question.
    pub(super) fn component_yes(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::AskFirst { id } => {
                let mut effects = self.close_all();
                effects.extend(self.component_job(&id, None));
                effects
            }
            Wants::Remove { id } => {
                self.request(Request::Components(Box::new(ComponentRequest::Remove {
                    id,
                })));
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Escape or No in a components dialog.
    pub(super) fn component_escape(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::AskFirst { .. } => vec![self.say(Message::info(messages::NOT_DOWNLOADED))],
            Wants::Remove { .. } => vec![self.say(Message::info("Nothing was removed."))],
            Wants::FirstRun => vec![self.say(Message::info("Nothing downloaded."))],
            _ => Vec::new(),
        }
    }
}
