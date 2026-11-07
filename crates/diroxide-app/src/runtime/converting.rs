//! Conversions on workers (alpha 4, lane AW): the recipes a selection
//! takes, the plan through CL's engine with the app's tools runner and
//! type registry, and the run, whose report is kept on the state folder's
//! shelf of the last 20 and sent to the app. An attended conversion runs
//! carta in process (CL's default); the watched folders give theirs a
//! `diro` child.

use std::sync::Arc;

use diroxide_core::Message;
use diroxide_ops::JobControl;
use diroxide_ops::convert::ConvertRequest;
use diroxide_vfs::CancelToken;

use super::{Runtime, forward_sink, lock};
use crate::app::AppEvent;
use crate::conversions::{
    ConvertAsk, ConvertNote, RegistryTyping, recipe_book, recipes_for, report_shelf,
};

/// What the runtime sends for a conversion's message.
fn note(note: ConvertNote) -> AppEvent {
    AppEvent::Convert(Box::new(note))
}

impl Runtime {
    /// Carries out one of the Convert command's asks.
    pub(super) fn convert(&mut self, ask: ConvertAsk) {
        match ask {
            ConvertAsk::Recipes {
                sources,
                types,
                tools,
                target,
            } => {
                let paths = self.paths.clone();
                self.worker(move |shared| {
                    let book = recipe_book(paths.as_ref());
                    let typing = RegistryTyping(types.0);
                    let names: Vec<(String, bool)> = sources
                        .iter()
                        .map(|(place, folder)| {
                            (place.name().unwrap_or_default().to_owned(), *folder)
                        })
                        .collect();
                    let choice = recipes_for(
                        &book,
                        &names,
                        &typing,
                        tools.0.as_ref(),
                        &CancelToken::new(),
                    );
                    let result = if choice.rows.is_empty() {
                        Err(Message::info("No recipe takes these files"))
                    } else {
                        Ok(choice)
                    };
                    shared.send(note(ConvertNote::Recipes {
                        sources: sources.into_iter().map(|(p, _)| p).collect(),
                        target,
                        result,
                    }));
                });
            }
            ConvertAsk::Plan {
                job,
                sources,
                destination,
                recipe,
                types,
                tools,
            } => {
                let engine = self.engine.clone().with_tools(tools.0);
                let paths = self.paths.clone();
                self.worker(move |shared| {
                    let book = recipe_book(paths.as_ref());
                    let result = match book.get(&recipe).cloned() {
                        None => Err(Message::error(format!(
                            "Could not convert: no recipe is named {recipe}."
                        ))),
                        Some(recipe) => {
                            let mut request =
                                ConvertRequest::new(job, sources, destination, recipe);
                            if let Some(paths) = &paths {
                                request = request.spool(paths.spool_dir());
                            }
                            engine
                                .plan_convert(
                                    &request,
                                    &RegistryTyping(types.0),
                                    &JobControl::new(),
                                )
                                .map(Box::new)
                                .map_err(|e| Message::error(format!("Could not convert: {e}.")))
                        }
                    };
                    shared.send(note(ConvertNote::Planned { job, result }));
                });
            }
            ConvertAsk::Run { plan, tools, clock } => {
                let engine = self.engine.clone().with_tools(tools.0);
                let job = plan.0.job;
                let control = self.register(job);
                let jobs = Arc::clone(&self.jobs);
                let shelf = self.paths.as_ref().map(report_shelf);
                self.worker(move |shared| {
                    let sink = forward_sink(job, shared, &engine, &control);
                    let mut report = engine.run_convert(&plan.0, &control, &sink);
                    report.stamp(clock.0.as_ref());
                    // A shelf that cannot be written never fails the job.
                    if let Some(shelf) = &shelf {
                        let _ = shelf.keep(&report.kept(clock.0.as_ref()));
                    }
                    let lines = report.lines(clock.0.as_ref());
                    shared.send(note(ConvertNote::Finished {
                        summary: report.summary(),
                        report: Box::new(report.job),
                        lines,
                    }));
                    lock(&jobs).remove(&job);
                });
            }
            ConvertAsk::Reports => {
                let result = match &self.paths {
                    Some(paths) => Ok(report_shelf(paths).list()),
                    None => Err(Message::info(
                        "No conversion reports: no state folder to keep them",
                    )),
                };
                self.queue.push_back(note(ConvertNote::Reports(result)));
            }
        }
    }
}
