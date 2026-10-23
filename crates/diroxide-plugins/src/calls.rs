//! The plugin's own requests: one place that checks each against the
//! method list, the host's offer and the manifest, then serves it through
//! the app's [`HostServices`].
//!
//! - A method the plugin may not send, or one this host does not serve in
//!   this version (a shape-only method such as `ui/form`), is answered
//!   -32601: the host never offered it in `initialize`.
//! - A call the manifest does not allow is answered -32030: `panel/*`
//!   need `permissions.panel`, `command/execute` a matching id pattern,
//!   `fs/*` `permissions.fs`, `tools/run` and `tools/launch` the tool's
//!   name in their lists. The first refusal of a session is said in two
//!   lines and every refusal is counted and logged.
//! - Everything else is served on the router's worker thread, so a dialog
//!   that waits for the owner never blocks the plugin's other traffic, and
//!   the supervisor pauses the waiting request's timer meanwhile.

use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use diroxide_core::Message;
use serde_json::{Value, json};

use crate::ask::{Answer, Completion, CompletionWanted, PluginCompleter, PromptHistory};
use crate::host::{HostServices, Voice};
use crate::lines;
use crate::live::LivePlugin;
use crate::log::{PluginLog, Source};
use crate::manifest::{Access, Manifest};
use crate::panels::{Denials, PanelChange, PanelChoice, read_execute};
use crate::protocol::methods::{self, Availability, Call, Sender};
use crate::protocol::{ErrorKind, Params, Request, RpcError};
use crate::router::{CancelFlag, Outcome};
use crate::session::PluginCapabilities;

/// What the host knows about one running plugin while it serves its
/// requests.
pub(crate) struct Calls {
    /// The plugin's id.
    pub(crate) plugin_id: String,
    /// The plugin's name, last in every line it causes.
    pub(crate) name: String,
    /// Its manifest, whose permissions every call is checked against.
    pub(crate) manifest: Manifest,
    /// What it confirmed in `initialize`.
    pub(crate) confirmed: PluginCapabilities,
    /// The app's services, when there are any.
    pub(crate) services: Option<Arc<dyn HostServices>>,
    /// The prompts' history, shared by every plugin of the host.
    pub(crate) history: Arc<Mutex<PromptHistory>>,
    /// The running plugin, once it is running.
    pub(crate) live: Arc<OnceLock<Weak<LivePlugin>>>,
    /// Its log.
    pub(crate) log: PluginLog,
    /// Where the host's words about it go.
    pub(crate) voice: Voice,
    /// The calls its manifest refused.
    pub(crate) denials: Denials,
}

fn not_served(method: &str) -> RpcError {
    RpcError::new(
        ErrorKind::MethodNotFound,
        format!("the host does not serve {method}"),
    )
}

impl Calls {
    /// Serves one of the plugin's requests.
    pub(crate) fn serve(&self, request: &Request, cancel: &CancelFlag) -> Outcome {
        let method = request.method.as_str();
        let known = methods::find(method)
            .filter(|m| m.sender != Sender::Host && m.call == Call::Request)
            .ok_or_else(|| not_served(method))?;
        if known.availability == Availability::ShapeOnly {
            return Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("the host does not serve {method} yet"),
            ));
        }
        let _ = cancel;
        self.permit(method, &request.params)?;
        match method {
            methods::UI_ASK => self.ask(&request.params),
            methods::UI_SHOW_TEXT => self.show_text(&request.params),
            methods::PANEL_GET => self.panel_get(&request.params),
            methods::PANEL_SET => self.panel_set(&request.params),
            methods::COMMAND_EXECUTE => self.execute(&request.params),
            _ => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("the host does not serve {method} yet"),
            )),
        }
    }

    /// Whether the manifest allows the call; the refusal is -32030, said
    /// once a session and logged every time.
    fn permit(&self, method: &str, params: &Params) -> Result<(), RpcError> {
        let permissions = &self.manifest.permissions;
        let named = |key: &str| params.get(key).and_then(Value::as_str).unwrap_or("");
        let allowed = match method {
            methods::PANEL_GET => permissions.panel >= Access::Read,
            methods::PANEL_SET => permissions.panel == Access::Write,
            methods::COMMAND_EXECUTE => permissions.allows_command(named("command")),
            methods::FS_LIST | methods::FS_STAT | methods::FS_READ => {
                permissions.fs >= Access::Read
            }
            methods::FS_WRITE
            | methods::FS_MKDIR
            | methods::FS_RENAME
            | methods::FS_COPY
            | methods::FS_MOVE
            | methods::FS_DELETE
            | methods::FS_STAGE
            | methods::FS_COMMIT => permissions.fs == Access::Write,
            methods::TOOLS_RUN => permissions.tools.iter().any(|t| t == named("tool")),
            methods::TOOLS_LAUNCH => permissions.launch.iter().any(|t| t == named("program")),
            _ => true,
        };
        if allowed {
            return Ok(());
        }
        self.log.write(
            Source::Host,
            &format!("refused {method}: its manifest does not allow it"),
        );
        if self.denials.record() {
            let [first, second] = lines::refused_by_manifest(&self.name);
            self.voice
                .say(Message::warning(format!("{first} {second}")));
        }
        Err(RpcError::new(
            ErrorKind::DeniedByHost,
            format!("the manifest does not allow {method} here"),
        ))
    }

    fn services(&self) -> Result<&Arc<dyn HostServices>, RpcError> {
        self.services
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host has no frontend"))
    }

    /// `panel/get`: a panel's place, cursor, selection, sort and filter.
    fn panel_get(&self, params: &Params) -> Outcome {
        let which = PanelChoice::read(params)?;
        let fields: Option<Vec<String>> = params.get("fields").and_then(Value::as_array).map(|f| {
            f.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });
        let panel = self
            .services()?
            .panel(which)
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host has no panels"))?;
        Ok(panel.to_value(fields.as_deref()))
    }

    /// `panel/set`: a change made as ordinary commands.
    fn panel_set(&self, params: &Params) -> Outcome {
        let change = PanelChange::read(params)?;
        self.services()?.set_panel(&change)?;
        Ok(json!({"applied": true}))
    }

    /// `command/execute`: a diroxide command by id.
    fn execute(&self, params: &Params) -> Outcome {
        let (command, args) = read_execute(params)?;
        self.services()?.execute(&command, &args)
    }

    fn history(&self) -> std::sync::MutexGuard<'_, PromptHistory> {
        self.history.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn live_weak(&self) -> Weak<LivePlugin> {
        self.live.get().cloned().unwrap_or_default()
    }

    /// `ui/ask`: one dialog, the owner's answer or `cancelled`.
    fn ask(&self, params: &Params) -> Outcome {
        let (mut question, wanted) =
            crate::ask::read_question(params, &self.plugin_id, &self.name)?;
        if let Some(prompt) = &question.prompt_id
            && question.kind.is_typed()
        {
            question.history = self.history().recall(&self.plugin_id, prompt);
        }
        question.completion = match wanted {
            Some(CompletionWanted::Host(source)) => Some(Completion::Host(source)),
            Some(CompletionWanted::List(list)) => Some(Completion::List(list)),
            Some(CompletionWanted::Plugin) if self.confirmed.completion => {
                question.prompt_id.as_deref().map(|prompt| {
                    Completion::Plugin(PluginCompleter::new(self.live_weak(), &self.name, prompt))
                })
            }
            Some(CompletionWanted::Plugin) => {
                self.log.write(
                    Source::Host,
                    "asked for completion from itself without confirming completion in initialize",
                );
                None
            }
            None => None,
        };
        let answer = match &self.services {
            Some(services) => services.ask(&question),
            None => Answer::Cancelled,
        };
        let value = answer.to_value(&question)?;
        if let (Some(prompt), Some(text)) = (&question.prompt_id, answer.for_history()) {
            self.history().remember(&self.plugin_id, prompt, text);
        }
        Ok(value)
    }

    /// `ui/show_text`: the viewer, waiting for its close when asked to.
    fn show_text(&self, params: &Params) -> Outcome {
        let document = crate::ask::read_document(params, &self.name)?;
        let services = self
            .services
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host has no viewer"))?;
        services.show_text(&document)?;
        Ok(json!({"shown": true}))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use diroxide_config::PluginSettings;
    use diroxide_core::Secret;
    use serde_json::{Value, json};

    use super::*;
    use crate::ask::{Candidate, QuestionKind};
    use crate::host::{Host, HostEvent, QUIT_BUDGET};
    use crate::supervisor::{ManualTicker, Ticker};
    use crate::testing::{FakeLauncher, Mode, RecordingServices, plugin_info};

    pub(crate) fn host_with(
        services: &Arc<RecordingServices>,
        extra: &str,
    ) -> (Host, Arc<ManualTicker>) {
        let ticker = Arc::new(ManualTicker::new());
        let mut host = Host::new(&PluginSettings::default())
            .with_launcher(FakeLauncher::new(Mode::Call))
            .with_ticker(Arc::clone(&ticker) as Arc<dyn Ticker>)
            .with_services(Arc::clone(services) as Arc<dyn HostServices>);
        host.add(plugin_info(extra));
        (host, ticker)
    }

    /// Runs the fake's command, which makes one host call, and returns
    /// what the host answered it, or `{"error": ...}`.
    pub(crate) fn call(host: &Host, method: &str, params: Value) -> Value {
        let answer = host
            .call(
                "line-counter",
                methods::COMMAND_RUN,
                json!({"command": "echo", "args": {"call": {"method": method, "params": params}}}),
            )
            .unwrap();
        match answer.get("host") {
            Some(value) => value.clone(),
            None => json!({"error": answer["host_error"].clone()}),
        }
    }

    #[test]
    fn each_kind_round_trips_with_a_scripted_dialog() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        services.answer(Answer::Confirmed(true));
        assert_eq!(
            call(
                &host,
                methods::UI_ASK,
                json!({"kind": "confirm", "title": "Go on?"})
            ),
            json!({"answer": true})
        );
        services.answer(Answer::Chosen(vec!["dev".into()]));
        assert_eq!(
            call(
                &host,
                methods::UI_ASK,
                json!({"kind": "choose", "choices": [{"id": "main", "label": "Main"}, {"id": "dev", "label": "Dev"}]})
            ),
            json!({"ids": ["dev"]})
        );
        services.answer(Answer::Text("hello".into()));
        assert_eq!(
            call(&host, methods::UI_ASK, json!({"kind": "text"})),
            json!({"text": "hello"})
        );
        services.answer(Answer::Place("/tmp/x".into()));
        assert_eq!(
            call(
                &host,
                methods::UI_ASK,
                json!({"kind": "path", "mode": "folder"})
            ),
            json!({"place": "/tmp/x"})
        );
        services.answer(Answer::Secret(Secret::new("pw".into())));
        assert_eq!(
            call(&host, methods::UI_ASK, json!({"kind": "secret"})),
            json!({"text": "pw"})
        );
        let questions = services.questions();
        assert_eq!(questions[0].title, "Go on? (Line counter)");
        assert_eq!(questions[2].title, "Line counter asks");
        assert_eq!(questions[4].kind, QuestionKind::Secret);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn escape_answers_cancelled_and_a_wrong_answer_is_a_host_error() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        assert_eq!(
            call(&host, methods::UI_ASK, json!({"kind": "text"})),
            json!({"cancelled": true})
        );
        services.answer(Answer::Confirmed(true));
        let wrong = call(&host, methods::UI_ASK, json!({"kind": "text"}));
        assert_eq!(wrong["error"]["code"], -32603);
        let bad = call(&host, methods::UI_ASK, json!({"kind": "riddle"}));
        assert_eq!(bad["error"]["code"], -32602);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn the_owners_thinking_time_does_not_count_against_the_plugin() {
        let services = RecordingServices::new();
        let (release, wait) = mpsc::channel::<()>();
        let (asked, question_came) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        services.respond(move |_| {
            let _ = asked.send(());
            let _ = wait.lock().unwrap().recv();
            Answer::Confirmed(false)
        });
        let (host, ticker) = host_with(&services, "\n[timeouts]\ncommand = \"30s\"\n");
        let host = Arc::new(host);
        let caller = {
            let host = Arc::clone(&host);
            thread::spawn(move || call(&host, methods::UI_ASK, json!({"title": "Delete?"})))
        };
        question_came.recv_timeout(Duration::from_secs(5)).unwrap();
        // Two minutes of thinking, well past the command's 30 seconds.
        for _ in 0..12 {
            ticker.advance(Duration::from_secs(10));
            host.tick();
        }
        release.send(()).unwrap();
        assert_eq!(caller.join().unwrap(), json!({"answer": false}));
        assert!(
            !host
                .take_events()
                .iter()
                .any(|e| matches!(e, HostEvent::TimedOut { .. } | HostEvent::Waiting { .. })),
            "the owner's time counted against the plugin"
        );
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn a_fixed_list_and_the_plugin_complete_a_prompt() {
        let services = RecordingServices::new();
        let (found, got) = mpsc::channel();
        services.respond(move |question| {
            let candidates = match &question.completion {
                Some(Completion::List(list)) => Completion::from_list(list, "ma"),
                Some(Completion::Plugin(completer)) => completer.query("ma", 2).unwrap(),
                other => panic!("no completion: {other:?}"),
            };
            let _ = found.send(candidates);
            Answer::Text("main".into())
        });
        let (host, _) = host_with(&services, "");
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch", "complete": {"source": "list",
                "candidates": [{"text": "main", "kind": "branch"}, {"text": "dev", "kind": "branch"}]}}),
        );
        assert_eq!(
            got.recv().unwrap(),
            [Candidate {
                text: "main".into(),
                kind: "branch".into()
            }]
        );
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch", "complete": {"source": "plugin"}}),
        );
        let from_plugin = got.recv().unwrap();
        assert_eq!(from_plugin[0].text, "ma-one");
        assert_eq!(from_plugin.len(), 2);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn the_next_keystroke_cancels_the_query_before_it() {
        let services = RecordingServices::new();
        let (found, got) = mpsc::channel();
        services.respond(move |question| {
            let Some(Completion::Plugin(completer)) = question.completion.clone() else {
                panic!("no plugin completion");
            };
            let first = {
                let completer = completer.clone();
                thread::spawn(move || completer.query("ma-slow", 7))
            };
            while !completer.is_waiting() {
                thread::sleep(Duration::from_millis(2));
            }
            let second = completer.query("mai", 3);
            let _ = found.send((first.join().unwrap(), second));
            Answer::Cancelled
        });
        let (host, _) = host_with(&services, "");
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch", "complete": {"source": "plugin"}}),
        );
        let (first, second) = got.recv().unwrap();
        assert_eq!(first.unwrap_err().code, -32800);
        assert_eq!(second.unwrap()[0].text, "mai-one");
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn history_is_kept_per_prompt_and_never_for_a_secret() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        services.answer(Answer::Text("main".into()));
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch"}),
        );
        services.answer(Answer::Text("dev".into()));
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch"}),
        );
        services.answer(Answer::Secret(Secret::new("hunter2".into())));
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "secret", "prompt_id": "branch"}),
        );
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "branch"}),
        );
        call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "other"}),
        );
        let questions = services.questions();
        assert_eq!(questions[3].history, ["dev", "main"]);
        assert!(
            questions[2].history.is_empty(),
            "a secret prompt shows history"
        );
        assert!(questions[4].history.is_empty());
        host.shutdown_all(QUIT_BUDGET);
    }

    /// The canary: a secret typed into a plugin's prompt reaches the plugin
    /// and nothing else, not its log, not the history.
    #[test]
    fn no_secret_reaches_a_log_or_the_history() {
        let folder = tempfile::tempdir().unwrap();
        let services = RecordingServices::new();
        let ticker = Arc::new(ManualTicker::new());
        let mut host = Host::new(&PluginSettings::default())
            .with_state_dir(folder.path())
            .with_launcher(FakeLauncher::new(Mode::Call))
            .with_ticker(Arc::clone(&ticker) as Arc<dyn Ticker>)
            .with_services(Arc::clone(&services) as Arc<dyn HostServices>);
        host.add(plugin_info(""));
        services.answer(Answer::Secret(Secret::new("canary-5f3a".into())));
        let answer = call(
            &host,
            methods::UI_ASK,
            json!({"kind": "secret", "prompt_id": "password", "title": "Password"}),
        );
        assert_eq!(answer, json!({"text": "canary-5f3a"}));
        services.answer(Answer::Text("x".into()));
        let later = call(
            &host,
            methods::UI_ASK,
            json!({"kind": "text", "prompt_id": "password"}),
        );
        assert_eq!(later, json!({"text": "x"}));
        host.shutdown_all(QUIT_BUDGET);
        let log = host.log("line-counter").unwrap();
        let written = log
            .path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        assert!(!written.contains("canary"), "{written}");
        assert!(
            services.questions()[1]
                .history
                .iter()
                .all(|h| !h.contains("canary"))
        );
        assert!(!format!("{:?}", services.questions()).contains("canary"));
    }

    #[test]
    fn show_text_reaches_the_viewer_with_its_source_last() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        assert_eq!(
            call(
                &host,
                methods::UI_SHOW_TEXT,
                json!({"title": "Log of main", "text": "one\ntwo", "kind": "plain"})
            ),
            json!({"shown": true})
        );
        let documents = services.documents();
        assert_eq!(documents[0].title, "Log of main (Line counter)");
        assert_eq!(documents[0].lines, ["one", "two"]);
        host.shutdown_all(QUIT_BUDGET);
    }

    fn root(name: &str) -> diroxide_core::Place {
        diroxide_core::Place::parse(&if cfg!(windows) {
            format!("D:\\{name}")
        } else {
            format!("/{name}")
        })
        .unwrap()
    }

    #[test]
    fn a_call_the_manifest_does_not_allow_is_refused_said_once_and_counted() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        for _ in 0..2 {
            let refused = call(&host, methods::PANEL_GET, json!({}));
            assert_eq!(refused["error"]["code"], -32030);
        }
        let refused = call(
            &host,
            methods::COMMAND_EXECUTE,
            json!({"command": "panel.refresh"}),
        );
        assert_eq!(refused["error"]["code"], -32030);
        assert_eq!(
            services.said(),
            ["Line counter may not do that. Its manifest does not allow it."]
        );
        assert_eq!(host.refusals("line-counter"), 3);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn a_panel_is_read_with_read_and_changed_only_with_write() {
        let services = RecordingServices::new();
        let mut left = crate::panels::PanelSnapshot::at(diroxide_core::Side::Left, root("docs"));
        left.cursor = Some(diroxide_core::Entry::new(
            "a.txt",
            diroxide_core::EntryKind::File,
        ));
        let mut right = crate::panels::PanelSnapshot::at(diroxide_core::Side::Right, root("out"));
        right.active = false;
        services.set_panels(vec![left, right]);
        let (host, _) = host_with(&services, "\n[permissions]\npanel = \"read\"\n");
        let active = call(&host, methods::PANEL_GET, json!({}));
        assert_eq!(active["panel"], "left");
        assert_eq!(active["cursor"]["name"], "a.txt");
        let other = call(
            &host,
            methods::PANEL_GET,
            json!({"panel": "other", "fields": ["place"]}),
        );
        assert_eq!(other["place"], root("out").to_string());
        assert!(other.get("cursor").is_none());
        let refused = call(&host, methods::PANEL_SET, json!({"refresh": true}));
        assert_eq!(refused["error"]["code"], -32030);
        host.shutdown_all(QUIT_BUDGET);

        let (writer, _) = host_with(&services, "\n[permissions]\npanel = \"write\"\n");
        let applied = call(
            &writer,
            methods::PANEL_SET,
            json!({"panel": "other", "place": root("photos").to_string(), "refresh": true}),
        );
        assert_eq!(applied, json!({"applied": true}));
        let hostile = call(&writer, methods::PANEL_SET, json!({"place": "C:"}));
        assert_eq!(hostile["error"]["code"], -32602);
        let changes = services.changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].place, Some(root("photos")));
        writer.shutdown_all(QUIT_BUDGET);
    }

    /// The pattern table of `permissions.commands`: a word, a word star,
    /// and a final star.
    #[test]
    fn commands_run_only_when_a_pattern_matches() {
        let services = RecordingServices::new();
        let (host, _) = host_with(
            &services,
            "\n[permissions]\ncommands = [\"panel.*\", \"file.copy\", \"view.*.next\"]\n",
        );
        for (id, allowed) in [
            ("panel.refresh", true),
            ("panel.go.up", true),
            ("file.copy", true),
            ("view.search.next", true),
            ("file.delete", false),
            ("app.quit", false),
            ("view.search.previous", false),
        ] {
            let answer = call(
                &host,
                methods::COMMAND_EXECUTE,
                json!({"command": id, "args": {"to": "x"}}),
            );
            assert_eq!(answer.get("error").is_none(), allowed, "{id}: {answer}");
        }
        let executed = services.executed();
        assert_eq!(executed.len(), 4);
        assert_eq!(executed[0].0, "panel.refresh");
        assert_eq!(executed[0].1["to"], "x");
        assert_eq!(host.refusals("line-counter"), 3);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn shape_only_and_host_methods_are_not_served() {
        let services = RecordingServices::new();
        let (host, _) = host_with(&services, "");
        let form = call(&host, methods::UI_FORM, json!({"title": "x", "fields": []}));
        assert_eq!(form["error"]["code"], -32601);
        assert_eq!(
            form["error"]["message"],
            "the host does not serve ui/form yet"
        );
        let backwards = call(&host, methods::COMMAND_RUN, json!({}));
        assert_eq!(backwards["error"]["code"], -32601);
        host.shutdown_all(QUIT_BUDGET);
    }
}
