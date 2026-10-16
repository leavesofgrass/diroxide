//! The runtime: carries out the effects that are I/O (listing, jobs,
//! plugins) and turns their results into [`AppEvent`]s. Frontends handle
//! the screen and speech effects themselves and hand the rest here.
//!
//! Phase 0 runs everything synchronously on the caller's thread. Wave 1
//! (W1d) owns this module and moves the work to worker threads with a
//! channel back, so the input thread never blocks.

use std::collections::VecDeque;
use std::sync::Arc;

use diroxide_core::{ConflictAnswer, ErrorAction};
use diroxide_core::{Effect, Message};
use diroxide_ops::{Conflict, Engine, ItemError, JobControl, JobProgress, JobSink};
use diroxide_vfs::{OpContext, Registry};

use crate::app::AppEvent;

/// Carries out I/O effects and queues their results.
#[derive(Debug)]
pub struct Runtime {
    registry: Arc<Registry>,
    engine: Engine,
    queue: VecDeque<AppEvent>,
}

struct Quiet;

impl JobSink for Quiet {
    fn progress(&self, _progress: &JobProgress) {}

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

impl Runtime {
    /// A runtime over a registry.
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            engine: Engine::new(Arc::clone(&registry)),
            registry,
            queue: VecDeque::new(),
        }
    }

    /// A runtime with only the local provider.
    pub fn with_local() -> Self {
        Self::new(Arc::new(Registry::with_local()))
    }

    /// The registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Carries out an effect if it is I/O; returns whether it was. Screen
    /// and speech effects return `false` and are left to the frontend.
    pub fn execute(&mut self, effect: &Effect) -> bool {
        match effect {
            Effect::List { request, place } => {
                let result = self
                    .registry
                    .resolve(place)
                    .and_then(|provider| provider.list(place, &OpContext::new()));
                self.queue.push_back(AppEvent::Listed {
                    request: *request,
                    place: place.clone(),
                    result,
                });
                true
            }
            Effect::StartJob(request) => {
                let event = match self.engine.plan(request) {
                    Ok(plan) => {
                        AppEvent::JobFinished(self.engine.run(&plan, &JobControl::new(), &Quiet))
                    }
                    Err(error) => {
                        AppEvent::Problem(Message::from_error(request.kind.verb(), "", &error))
                    }
                };
                self.queue.push_back(event);
                true
            }
            Effect::CancelJob(_) => true,
            Effect::RunPlugin(call) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "run",
                    &call.plugin,
                    "plugins are not built yet",
                    "",
                )));
                true
            }
            Effect::OpenExternally(place) => {
                let name = place.name().unwrap_or("it").to_owned();
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "open",
                    &name,
                    "opening files in other programs is not built yet",
                    "",
                )));
                true
            }
            Effect::SetClipboard(_) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "copy",
                    "to the clipboard",
                    "the clipboard is not built yet",
                    "",
                )));
                true
            }
            _ => false,
        }
    }

    /// Takes the results that are ready.
    pub fn poll(&mut self) -> Vec<AppEvent> {
        self.queue.drain(..).collect()
    }

    /// Carries out effects, feeds their results back to the app, and
    /// repeats until nothing is left; returns the screen and speech effects
    /// in order. For tests, `diro` one-shot commands and the Phase 0
    /// frontends; an interactive frontend polls instead.
    pub fn run_until_idle(&mut self, app: &mut crate::App, effects: Vec<Effect>) -> Vec<Effect> {
        let mut for_frontend = Vec::new();
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in pending {
                if !self.execute(&effect) {
                    for_frontend.push(effect);
                }
            }
            pending = self
                .poll()
                .into_iter()
                .flat_map(|event| app.handle(event))
                .collect();
        }
        for_frontend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{Place, Redraw, RequestId};

    #[test]
    fn listing_is_carried_out_and_screen_effects_are_not() {
        let mut runtime = Runtime::with_local();
        let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(runtime.execute(&Effect::List {
            request: RequestId::new(1),
            place,
        }));
        assert!(!runtime.execute(&Effect::Redraw(Redraw::All)));
        let events = runtime.poll();
        assert!(
            matches!(&events[..], [AppEvent::Listed { result: Ok(entries), .. }] if !entries.is_empty())
        );
        assert!(runtime.poll().is_empty());
    }
}
