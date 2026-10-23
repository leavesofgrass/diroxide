//! Optional components on the runtime's side (W4d, R22 commit 41): the
//! list, and the downloads, checks, removals and installs on workers.
//!
//! - The list is the embedded one with the owner's own source's list
//!   added (a source may add components, never change a known one's
//!   pins).
//! - A download runs only after the app's yes ([`Consent::yes`] is made
//!   here from that request and nowhere else); its progress goes through
//!   the components crate's [`ProgressVoice`] at the progress interval;
//!   Escape raises its cancel and the next try resumes from its `.part`
//!   files.
//! - Removal lists its paths first, and removes only the component's own
//!   files after the owner's yes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_components::download::project_releases;
use diroxide_components::messages::{self, ProgressVoice};
use diroxide_components::{
    Consent, Downloader, Fetcher, Manifest, Origin, Source, Sources, StandardFetcher, Status,
    host_target, install_from, manifest,
};
use diroxide_config::Paths;
use diroxide_core::JobId;
use diroxide_tools::ToolRegistry;
use diroxide_vfs::{CancelToken, OpContext};

use super::{Shared, Work, lock};
use crate::app::AppEvent;
use crate::components::{ComponentEvent, ComponentRequest, listing};

fn send(shared: &Shared, event: ComponentEvent) {
    shared.send(AppEvent::Component(Box::new(event)));
}

/// What the runtime keeps about components.
#[derive(Default)]
pub(crate) struct ComponentRuntime {
    /// No public addresses: the owner's source alone (tests never reach
    /// the network).
    offline: bool,
    /// How files are fetched, when not the standard way (tests).
    fetcher: Option<Arc<dyn Fetcher>>,
    /// The cancels of the downloads and installs that run.
    runs: Arc<Mutex<HashMap<JobId, CancelToken>>>,
}

impl std::fmt::Debug for ComponentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRuntime")
            .field("offline", &self.offline)
            .finish_non_exhaustive()
    }
}

/// The list this run knows: the embedded one, and the source's.
fn known(sources: &Sources) -> Manifest {
    let mut list = Manifest::embedded();
    if let Some(path) = sources.list_path()
        && path.is_file()
    {
        let read = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= manifest::MAX_BYTES as u64)
            .and_then(|_| std::fs::read_to_string(&path).ok());
        if let Some(Ok(own)) = read.map(|text| manifest::parse(&text, Origin::OwnSource)) {
            list.add_source(own);
        }
    }
    list
}

impl ComponentRuntime {
    /// No public addresses from now on.
    pub(crate) fn set_offline(&mut self) {
        self.offline = true;
    }

    /// Fetches through another fetcher (tests).
    pub(crate) fn set_fetcher(&mut self, fetcher: Arc<dyn Fetcher>) {
        self.fetcher = Some(fetcher);
    }

    /// Cancels a download or an install; whether one was running.
    pub(crate) fn cancel(&self, job: JobId) -> bool {
        match lock(&self.runs).get(&job) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    fn sources(&self, setting: &str) -> Sources {
        Sources {
            source: Source::from_setting(setting),
            releases: if self.offline {
                None
            } else {
                project_releases()
            },
        }
    }

    fn fetcher(&self, paths: Option<&Paths>) -> Arc<dyn Fetcher> {
        match &self.fetcher {
            Some(fetcher) => Arc::clone(fetcher),
            None => {
                let tools = match paths {
                    Some(paths) => {
                        ToolRegistry::builtin().with_overrides(diroxide_config::load(paths).tools)
                    }
                    None => ToolRegistry::builtin(),
                };
                Arc::new(StandardFetcher::from_tools(&tools))
            }
        }
    }

    /// Carries out one request on a worker.
    pub(crate) fn request(
        &self,
        request: ComponentRequest,
        paths: Option<&Paths>,
        source_setting: &str,
        worker: &dyn Fn(Work),
    ) {
        let Some(paths) = paths.cloned() else {
            worker(Box::new(|shared| {
                send(shared, ComponentEvent::Listed(Box::default()));
            }));
            return;
        };
        let sources = self.sources(source_setting);
        let dir = paths.components_dir();
        match request {
            ComponentRequest::List => worker(Box::new(move |shared| {
                let list = known(&sources);
                send(
                    shared,
                    ComponentEvent::Listed(Box::new(listing(&list, &dir, host_target()))),
                );
            })),
            ComponentRequest::Download { job, id, interval } => {
                let token = CancelToken::new();
                lock(&self.runs).insert(job, token.clone());
                let runs = Arc::clone(&self.runs);
                let fetcher = self.fetcher(Some(&paths));
                worker(Box::new(move |shared| {
                    let list = known(&sources);
                    let (ready, lines) = match list.get(&id) {
                        None => (false, vec![format!("Could not find the component {id}.")]),
                        Some(component) => download(
                            component, &dir, sources, fetcher, &token, interval, job, shared,
                        ),
                    };
                    lock(&runs).remove(&job);
                    send(
                        shared,
                        ComponentEvent::Finished {
                            job,
                            id,
                            ready,
                            lines,
                        },
                    );
                }));
            }
            ComponentRequest::Verify { id } => worker(Box::new(move |shared| {
                let list = known(&sources);
                let lines = match list.get(&id) {
                    None => vec![format!("Could not find the component {id}.")],
                    Some(c) => match c.status_in(&dir, host_target()) {
                        Status::NotInstalled | Status::NoBuild => {
                            vec![format!("Not installed: {}.", c.title)]
                        }
                        _ => messages::verified(c, &c.verify_in(&dir, host_target())),
                    },
                };
                send(shared, ComponentEvent::Verified { id, lines });
            })),
            ComponentRequest::PlanRemove { id } => worker(Box::new(move |shared| {
                let list = known(&sources);
                let Some(c) = list.get(&id) else {
                    send(
                        shared,
                        ComponentEvent::Removed {
                            id: id.clone(),
                            line: format!("Could not find the component {id}."),
                        },
                    );
                    return;
                };
                let removal = c.removal_in(&dir);
                if removal.is_empty() {
                    send(
                        shared,
                        ComponentEvent::Removed {
                            id,
                            line: format!("Nothing to remove: {} is not installed.", c.title),
                        },
                    );
                    return;
                }
                send(
                    shared,
                    ComponentEvent::RemovalPlanned {
                        id,
                        lines: removal.lines(),
                        summary: removal.summary(),
                        question: messages::remove_question(c),
                    },
                );
            })),
            ComponentRequest::Remove { id } => worker(Box::new(move |shared| {
                let list = known(&sources);
                let line = match list.get(&id) {
                    None => format!("Could not find the component {id}."),
                    Some(c) => match c.removal_in(&dir).run() {
                        Ok(_) => format!("Removed: {}.", c.title),
                        Err(error) => format!("Could not remove {}: {error}.", c.title),
                    },
                };
                send(shared, ComponentEvent::Removed { id, line });
            })),
            ComponentRequest::Install { job, id, from } => {
                let token = CancelToken::new();
                lock(&self.runs).insert(job, token.clone());
                let runs = Arc::clone(&self.runs);
                worker(Box::new(move |shared| {
                    let list = known(&sources);
                    let (ready, lines) = match list.get(&id) {
                        None => (false, vec![format!("Could not find the component {id}.")]),
                        Some(c) => {
                            let ctx = OpContext::new().with_cancel(token);
                            match install_from(c, &from, &dir, host_target(), &ctx) {
                                Ok(outcome) => (true, ready_lines(c, &outcome)),
                                Err(error) => (false, messages::failure(&error)),
                            }
                        }
                    };
                    lock(&runs).remove(&job);
                    send(
                        shared,
                        ComponentEvent::Finished {
                            job,
                            id,
                            ready,
                            lines,
                        },
                    );
                }));
            }
        }
    }
}

/// "Ready: diro-examples 0.1.0.", then each member left out with why.
fn ready_lines(
    component: &diroxide_components::Component,
    outcome: &diroxide_components::Outcome,
) -> Vec<String> {
    let mut lines = vec![messages::ready(component)];
    lines.extend(
        outcome
            .left_out
            .iter()
            .filter(|(_, reason)| reason != diroxide_components::unpack::NOT_ITS_FILE)
            .map(|(name, reason)| messages::left_out(name, reason)),
    );
    lines
}

#[allow(clippy::too_many_arguments)]
fn download(
    component: &diroxide_components::Component,
    dir: &Path,
    sources: Sources,
    fetcher: Arc<dyn Fetcher>,
    token: &CancelToken,
    interval: u64,
    job: JobId,
    shared: &Arc<Shared>,
) -> (bool, Vec<String>) {
    match component.status_in(dir, host_target()) {
        Status::Installed => return (true, vec![messages::already_installed(component)]),
        Status::NoBuild => return (false, messages::no_build(component)),
        _ => {}
    }
    let downloader =
        Downloader::new(dir.to_path_buf(), sources, fetcher).with_target(host_target());
    let mut voice = ProgressVoice::new(&component.id, Duration::from_secs(interval.max(1)));
    let started = Instant::now();
    let ctx = OpContext::new().with_cancel(token.clone());
    let result = downloader.download(component, &Consent::yes(component), &ctx, &mut |event| {
        if let Some(line) = voice.hear(event, started.elapsed()) {
            send(shared, ComponentEvent::Progress { job, line });
        }
    });
    match result {
        Ok(outcome) => (true, ready_lines(component, &outcome)),
        Err(error) => (false, messages::failure(&error)),
    }
}
