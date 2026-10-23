//! Hooks wired in the app (R25 commit 81; R3, section 9): the runtime is
//! the engine's [`HookRunner`], through the plugin hosts.
//!
//! - **Who wakes:** each enabled plugin's `[[hook]]` tables, by their
//!   filters (the event, the schemes, a glob, a kind), in the order the
//!   plugins and their tables come.
//! - **`before_*`** is `hook/before` on the engine's thread for it, after
//!   the plan and before the confirmation. A veto stops the job with the
//!   plugin's reason, its name last (answer 90): "Delete stopped: folder
//!   is protected. By Folder guard." Silence or an error is the hook's
//!   `on_timeout`, said either way: "Delete goes on: no answer. By Folder
//!   guard." The engine waits the hook's limit and a grace, so the line is
//!   the app's own words, never a quiet veto.
//! - **`after_*`** is `hook/after`, sent once with the report, never
//!   waited for.
//! - **Navigation:** `enter_folder` and `select` are notifications only,
//!   debounced: the runtime notes the active panel's place, marks and
//!   focus after each batch of effects, and a thread of its own sends the
//!   last of each kind once things have been still for a moment.

use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::thread;
use std::time::Duration;

use diroxide_core::Place;
use diroxide_ops::{
    HookCall, HookEvent, HookItem, HookReport, HookRunner, HookVerdict, OnSilence, WakingHook,
};
use diroxide_plugins::PluginError;
use diroxide_plugins::hooks::{HookAfter, HookAnswer, HookBefore, HookItem as WireItem, wakes};
use diroxide_plugins::host::Host;
use diroxide_plugins::manifest::{HookSpec, OnTimeout};
use diroxide_plugins::protocol::methods;
use diroxide_plugins::services::{MAX_ANNOUNCE_CHARS, clean_text};

/// How much longer the engine waits than a hook's own limit, so the
/// plugin host's time-out reaches the app's words first.
const GRACE: Duration = Duration::from_secs(3);

/// How long the panels must be still before a navigation hook is told.
pub(crate) const NAV_DEBOUNCE: Duration = Duration::from_millis(300);

/// A plugin with hooks, as the runner knows it.
#[derive(Clone)]
pub(crate) struct Hooked {
    /// Its id.
    pub id: String,
    /// Its name, last in every line it causes.
    pub name: String,
    /// Its host.
    pub host: Arc<Host>,
    /// Its `[[hook]]` tables.
    pub hooks: Vec<HookSpec>,
    /// Its `[timeouts] hook`.
    pub limit: Duration,
}

/// What the panels did, for the navigation hooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NavEvent {
    /// The active panel shows a new folder.
    EnterFolder {
        /// The folder.
        place: Place,
    },
    /// The marks or the focus changed in the active panel.
    Select {
        /// The folder.
        place: Place,
        /// How many rows are marked.
        count: u64,
        /// The focused row's name.
        cursor: Option<String>,
    },
}

impl NavEvent {
    fn event(&self) -> &'static str {
        match self {
            NavEvent::EnterFolder { .. } => "enter_folder",
            NavEvent::Select { .. } => "select",
        }
    }
}

/// The hook runner the engine asks. See the module documentation.
#[derive(Default)]
pub(crate) struct AppHooks {
    plugins: RwLock<Vec<Hooked>>,
    nav: Mutex<Option<Sender<NavEvent>>>,
}

impl fmt::Debug for AppHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<String> = self.plugins().iter().map(|p| p.id.clone()).collect();
        f.debug_struct("AppHooks").field("plugins", &ids).finish()
    }
}

/// The event's name as manifests write it: `before_mkdir` for a new
/// folder, the engine's own name otherwise.
fn wire_event(event: HookEvent, before: bool) -> &'static str {
    match (event, before) {
        (HookEvent::MakeFolder, true) => "before_mkdir",
        (HookEvent::MakeFolder, false) => "after_mkdir",
        (event, true) => event.before(),
        (event, false) => event.after(),
    }
}

fn wire_item(item: &HookItem) -> WireItem {
    WireItem {
        from: item.from.to_string(),
        to: item.to.as_ref().map(Place::to_string),
        kind: if item.folder { "folder" } else { "file" }.to_owned(),
        result: item.result.map(|result| result.word().to_owned()),
    }
}

/// The line of a veto, the reason first and the plugin's name last:
/// "Delete stopped: folder is protected. By Folder guard."
pub(crate) fn vetoed_line(verb: &str, reason: &str, plugin: &str) -> String {
    let reason = clean_text(reason, MAX_ANNOUNCE_CHARS);
    let reason = reason.trim().trim_end_matches('.');
    let reason = if reason.is_empty() {
        "no reason given"
    } else {
        reason
    };
    format!("{verb} stopped: {reason}. By {plugin}.")
}

/// The line of a hook that did not answer: "Copy goes on: no answer. By
/// Folder guard.", or with `stops`, "Copy stopped: no answer. By Folder
/// guard."
pub(crate) fn silent_line(verb: &str, stops: bool, plugin: &str) -> String {
    let what = if stops { "stopped" } else { "goes on" };
    format!("{verb} {what}: no answer. By {plugin}.")
}

impl AppHooks {
    fn plugins(&self) -> Vec<Hooked> {
        self.plugins
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The plugins with hooks now: called whenever the plugins change.
    pub(crate) fn set(&self, plugins: Vec<Hooked>) {
        *self.plugins.write().unwrap_or_else(PoisonError::into_inner) = plugins;
    }

    /// Whether any plugin hooks the panels' navigation.
    pub(crate) fn wants_nav(&self) -> bool {
        self.plugins().iter().any(|p| {
            p.hooks
                .iter()
                .any(|h| h.event == "enter_folder" || h.event == "select")
        })
    }

    /// A move in the panels: sent to its hooks once things are still.
    pub(crate) fn navigated(self: &Arc<Self>, event: NavEvent) {
        let mut nav = self.nav.lock().unwrap_or_else(PoisonError::into_inner);
        if nav.is_none() {
            let (send, receive) = mpsc::channel::<NavEvent>();
            let runner: Weak<Self> = Arc::downgrade(self);
            let spawned = thread::Builder::new()
                .name("diroxide-nav-hooks".to_owned())
                .spawn(move || {
                    while let Ok(first) = receive.recv() {
                        let mut latest: HashMap<&'static str, NavEvent> = HashMap::new();
                        latest.insert(first.event(), first);
                        let ended = loop {
                            match receive.recv_timeout(NAV_DEBOUNCE) {
                                Ok(event) => {
                                    latest.insert(event.event(), event);
                                }
                                Err(RecvTimeoutError::Timeout) => break false,
                                Err(RecvTimeoutError::Disconnected) => break true,
                            }
                        };
                        let Some(runner) = runner.upgrade() else {
                            return;
                        };
                        for event in ["enter_folder", "select"] {
                            if let Some(event) = latest.remove(event) {
                                runner.notify(&event);
                            }
                        }
                        if ended {
                            return;
                        }
                    }
                });
            if spawned.is_ok() {
                *nav = Some(send);
            }
        }
        if let Some(send) = nav.as_ref() {
            let _ = send.send(event);
        }
    }

    /// Tells each plugin whose hook wakes for a navigation event, once.
    fn notify(&self, event: &NavEvent) {
        let (place, after) = match event {
            NavEvent::EnterFolder { place } => (
                place,
                HookAfter {
                    event: "enter_folder".to_owned(),
                    job: None,
                    status: None,
                    items: Vec::new(),
                    place: Some(place.to_string()),
                    selection_count: None,
                    cursor: None,
                },
            ),
            NavEvent::Select {
                place,
                count,
                cursor,
            } => (
                place,
                HookAfter {
                    event: "select".to_owned(),
                    job: None,
                    status: None,
                    items: Vec::new(),
                    place: Some(place.to_string()),
                    selection_count: Some(*count),
                    cursor: cursor.clone(),
                },
            ),
        };
        for plugin in self.plugins() {
            if plugin
                .hooks
                .iter()
                .any(|spec| wakes(spec, event.event(), place, true))
            {
                let _ = plugin.host.hook_after(&plugin.id, &after);
            }
        }
    }
}

impl HookRunner for AppHooks {
    fn waking(&self, call: &HookCall) -> Vec<WakingHook> {
        let event = wire_event(call.event, true);
        let mut waking = Vec::new();
        for plugin in self.plugins() {
            for (index, spec) in plugin.hooks.iter().enumerate() {
                let wakes_here = if call.items.is_empty() {
                    spec.event == event
                } else {
                    call.items
                        .iter()
                        .any(|item| wakes(spec, event, &item.from, item.folder))
                };
                if !wakes_here {
                    continue;
                }
                waking.push(WakingHook {
                    plugin: plugin.id.clone(),
                    name: plugin.name.clone(),
                    index,
                    timeout: spec.timeout.flatten().unwrap_or(plugin.limit) + GRACE,
                    on_timeout: match spec.on_timeout {
                        OnTimeout::Cancel => OnSilence::Cancel,
                        OnTimeout::Continue => OnSilence::Continue,
                    },
                });
            }
        }
        waking
    }

    fn before(&self, hook: &WakingHook, call: &HookCall) -> HookVerdict {
        let verb = call.event.verb();
        let silence = || {
            let stops = hook.on_timeout == OnSilence::Cancel;
            let line = silent_line(verb, stops, &hook.name);
            if stops {
                HookVerdict::Cancel(line)
            } else {
                HookVerdict::ContinueWithWarning(line)
            }
        };
        let Some(plugin) = self.plugins().into_iter().find(|p| p.id == hook.plugin) else {
            return silence();
        };
        let before = HookBefore {
            event: wire_event(call.event, true).to_owned(),
            job: call.job.get(),
            items: call.items.iter().map(wire_item).collect(),
            count: call.count,
            summary: call.summary.clone(),
            destination: call.destination.as_ref().map(Place::to_string),
        };
        let outcome = serde_json::to_value(&before)
            .map_err(|e| PluginError::Protocol(e.to_string()))
            .and_then(|params| plugin.host.call(&plugin.id, methods::HOOK_BEFORE, params));
        let answer = outcome
            .ok()
            .and_then(|value| serde_json::from_value::<HookAnswer>(value).ok());
        match answer {
            Some(HookAnswer::Continue) => HookVerdict::Continue,
            Some(HookAnswer::Cancel { reason }) => {
                HookVerdict::Cancel(vetoed_line(verb, &reason, &hook.name))
            }
            None => silence(),
        }
    }

    fn after(&self, report: &HookReport) {
        let event = wire_event(report.event, false);
        let after = HookAfter {
            event: event.to_owned(),
            job: Some(report.job.get()),
            status: Some(report.status.word().to_owned()),
            items: report.items.iter().map(wire_item).collect(),
            place: None,
            selection_count: None,
            cursor: None,
        };
        for plugin in self.plugins() {
            let wakes_here = plugin.hooks.iter().any(|spec| {
                spec.event == event
                    && (report.items.is_empty()
                        || report
                            .items
                            .iter()
                            .any(|item| wakes(spec, event, &item.from, item.folder)))
            });
            if wakes_here {
                let _ = plugin.host.hook_after(&plugin.id, &after);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn a_veto_says_the_reason_first_and_the_plugin_last() {
        let line = vetoed_line("Delete", "folder is protected.", "Folder guard");
        assert_eq!(
            line,
            "Delete stopped: folder is protected. By Folder guard."
        );
        assert!(within_cells(&line, "Delete stopped: folder is protected."));
        assert_eq!(
            vetoed_line("Copy", "  ", "Folder guard"),
            "Copy stopped: no reason given. By Folder guard."
        );
        assert_eq!(
            silent_line("Delete", false, "Folder guard"),
            "Delete goes on: no answer. By Folder guard."
        );
        assert_eq!(wire_event(HookEvent::MakeFolder, true), "before_mkdir");
        assert_eq!(wire_event(HookEvent::Delete, false), "after_delete");
    }
}
