//! One running plugin: its connection, its supervisor and its queue.
//!
//! [`LivePlugin::call`] runs on the caller's worker thread, never the
//! input thread: it waits for a free slot (one request at a time unless the
//! manifest's `concurrency` says more, so a stuck plugin stops its own
//! queue and nobody else's), sends the request with a progress token, and
//! waits for the answer. [`LivePlugin::tick`] looks at the clock: it sends
//! the cancel when a request's inactivity limit runs out, answers -32034
//! two seconds later, and reports the waiting notice, a restart and the
//! idle stop to the host.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use diroxide_vfs::CancelToken;

use serde_json::{Value, json};

use crate::lines;
use crate::manifest::{Limit, Manifest, Timeouts};
use crate::protocol::{ErrorKind, Id, Params, RpcError};
use crate::router::{Connection, Outcome};
use crate::session::Ending;
use crate::supervisor::{Action, Supervisor, Ticker};

/// The most requests that may wait for a plugin's free slot.
pub const QUEUE_LIMIT: usize = 256;

/// Which inactivity limit of `[timeouts]` a request has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// Provider calls and the like: 30 seconds by default.
    Request,
    /// A preview: 5 seconds.
    Preview,
    /// A `before_*` hook: 2 seconds.
    Hook,
    /// A command: none.
    Command,
    /// A conversion: none.
    Convert,
}

impl RequestKind {
    /// The kind's limit in a manifest's timeouts.
    pub fn limit(self, timeouts: &Timeouts) -> Limit {
        match self {
            RequestKind::Request => timeouts.request,
            RequestKind::Preview => timeouts.preview,
            RequestKind::Hook => timeouts.hook,
            RequestKind::Command => timeouts.command,
            RequestKind::Convert => timeouts.convert,
        }
    }
}

/// A counting gate: at most `size` callers inside at once, at most
/// [`QUEUE_LIMIT`] waiting.
#[derive(Debug)]
struct Slots {
    state: Mutex<(usize, usize)>,
    free: Condvar,
    size: usize,
}

impl Slots {
    fn new(size: usize) -> Self {
        Self {
            state: Mutex::new((0, 0)),
            free: Condvar::new(),
            size: size.max(1),
        }
    }

    /// Waits for a slot; false when the queue is full.
    fn enter(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.0 >= self.size && state.1 >= QUEUE_LIMIT {
            return false;
        }
        state.1 += 1;
        while state.0 >= self.size {
            state = self
                .free
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.1 -= 1;
        state.0 += 1;
        true
    }

    fn leave(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.0 = state.0.saturating_sub(1);
        self.free.notify_one();
    }
}

/// Takes each partial batch of entries as it arrives; `Break` stops the
/// listing early (the request is cancelled and still answered).
pub type Batches<'a> = &'a mut dyn FnMut(Vec<Value>) -> ControlFlow<()>;

/// What a call may carry besides its parameters.
#[derive(Default)]
pub struct CallOptions<'a> {
    /// Raised by the caller to cancel the request, which is still
    /// answered: a provider call whose operation was cancelled.
    pub cancel: Option<&'a CancelToken>,
    /// Asks for a streamed answer (a `partial_token`) and takes its
    /// batches.
    pub batches: Option<Batches<'a>>,
}

impl std::fmt::Debug for CallOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallOptions")
            .field("cancel", &self.cancel.is_some())
            .field("batches", &self.batches.is_some())
            .finish()
    }
}

/// What a request's caller hears besides the answer; the label is what
/// the sentence names (a command's name, or the plugin's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The request has waited two seconds for the plugin.
    Slow {
        /// The request.
        id: Id,
        /// What the sentence names.
        label: String,
    },
    /// The request timed out after its limit.
    TimedOut {
        /// The request.
        id: Id,
        /// What the sentence names.
        label: String,
        /// Its limit.
        after: Duration,
    },
}

/// A plugin that is running.
pub struct LivePlugin {
    id: String,
    name: String,
    connection: Connection,
    supervisor: Mutex<Supervisor>,
    ticker: Arc<dyn Ticker>,
    timeouts: Timeouts,
    shutdown_grace: Duration,
    slots: Slots,
    tokens: Mutex<HashMap<String, Id>>,
    limits: Mutex<HashMap<Id, Duration>>,
    labels: Mutex<HashMap<Id, String>>,
    notices: Mutex<Vec<Notice>>,
    next_token: AtomicU64,
    partials: Mutex<HashMap<String, Sender<Vec<Value>>>>,
}

impl std::fmt::Debug for LivePlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LivePlugin")
            .field("id", &self.id)
            .field("connection", &self.connection)
            .finish()
    }
}

impl LivePlugin {
    /// A running plugin over a connection whose handshake is done.
    pub fn new(manifest: &Manifest, connection: Connection, ticker: Arc<dyn Ticker>) -> Self {
        let run = manifest.run.clone().unwrap_or_default();
        let now = ticker.now();
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            connection,
            supervisor: Mutex::new(Supervisor::new(run.idle_timeout, now)),
            ticker,
            timeouts: manifest.timeouts,
            shutdown_grace: run.shutdown_grace,
            slots: Slots::new(run.concurrency as usize),
            tokens: Mutex::new(HashMap::new()),
            limits: Mutex::new(HashMap::new()),
            labels: Mutex::new(HashMap::new()),
            notices: Mutex::new(Vec::new()),
            next_token: AtomicU64::new(1),
            partials: Mutex::new(HashMap::new()),
        }
    }

    /// The plugin's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The plugin's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The connection.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    fn supervisor(&self) -> std::sync::MutexGuard<'_, Supervisor> {
        self.supervisor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends a request and waits for its answer, with the kind's
    /// inactivity limit. `params` gains a `progress_token` unless it has
    /// one.
    pub fn call(&self, method: &str, params: Params, kind: RequestKind) -> Outcome {
        let label = self.name.clone();
        self.call_labeled(method, params, kind, &label)
    }

    /// The same, with the label the notices name: a command's name, so the
    /// jobs line says "Count lines: waiting for the plugin".
    pub fn call_labeled(
        &self,
        method: &str,
        params: Params,
        kind: RequestKind,
        label: &str,
    ) -> Outcome {
        self.call_with(method, params, kind, label, CallOptions::default())
    }

    /// The same, with a cancel token and a taker of partial batches.
    pub fn call_with(
        &self,
        method: &str,
        mut params: Params,
        kind: RequestKind,
        label: &str,
        options: CallOptions<'_>,
    ) -> Outcome {
        if !self.slots.enter() {
            return Err(RpcError::new(
                ErrorKind::LimitExceeded,
                "too many requests are waiting for this plugin",
            ));
        }
        let outcome = self.call_inside(method, &mut params, kind, label, options);
        self.slots.leave();
        outcome
    }

    fn lock_partials(&self) -> std::sync::MutexGuard<'_, HashMap<String, Sender<Vec<Value>>>> {
        self.partials.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A partial batch for a token: handed to the call that waits for it.
    pub fn partial(&self, token: &str, entries: Vec<Value>) {
        if let Some(sender) = self.lock_partials().get(token) {
            let _ = sender.send(entries);
        }
    }

    fn call_inside(
        &self,
        method: &str,
        params: &mut Params,
        kind: RequestKind,
        label: &str,
        mut options: CallOptions<'_>,
    ) -> Outcome {
        let number = self.next_token.fetch_add(1, Ordering::SeqCst);
        let token = format!("t{number}");
        let batches: Option<(String, Receiver<Vec<Value>>)> = options.batches.as_ref().map(|_| {
            let partial = format!("p{number}");
            let (sender, receiver) = mpsc::channel();
            self.lock_partials().insert(partial.clone(), sender);
            params.insert("partial_token".to_owned(), json!(partial));
            (partial, receiver)
        });
        params
            .entry("progress_token".to_owned())
            .or_insert_with(|| json!(token));
        let token = params
            .get("progress_token")
            .and_then(Value::as_str)
            .unwrap_or(&token)
            .to_owned();
        let limit = kind.limit(&self.timeouts);
        let mut registered = None;
        let sent = self
            .connection
            .request_then(method, std::mem::take(params), |id| {
                registered = Some(id.clone());
                self.lock_labels().insert(id.clone(), label.to_owned());
                self.lock_tokens().insert(token.clone(), id.clone());
                if let Some((partial, _)) = &batches {
                    self.lock_tokens().insert(partial.clone(), id.clone());
                }
                if let Some(limit) = limit {
                    self.lock_limits().insert(id.clone(), limit);
                }
                self.supervisor()
                    .track(id.clone(), limit, self.ticker.now());
            });
        let call = match sent {
            Ok(call) => call,
            Err(e) => {
                self.lock_tokens().remove(&token);
                if let Some((partial, _)) = &batches {
                    self.lock_tokens().remove(partial);
                    self.lock_partials().remove(partial);
                }
                if let Some(id) = registered {
                    self.lock_limits().remove(&id);
                    self.lock_labels().remove(&id);
                    self.supervisor().finish(&id, self.ticker.now());
                }
                return Err(RpcError::new(ErrorKind::PluginUnavailable, e.to_string()));
            }
        };
        let mut cancel_sent = false;
        let mut stopped_early = false;
        let deliver = |options: &mut CallOptions<'_>, stopped_early: &mut bool| {
            if let (Some((_, receiver)), Some(take)) = (&batches, options.batches.as_mut()) {
                while let Ok(batch) = receiver.try_recv() {
                    if !*stopped_early && take(batch).is_break() {
                        *stopped_early = true;
                    }
                }
            }
        };
        let outcome = loop {
            let wait = if batches.is_some() { 20 } else { 100 };
            if let Some(outcome) = call.wait(Duration::from_millis(wait)) {
                deliver(&mut options, &mut stopped_early);
                break outcome;
            }
            deliver(&mut options, &mut stopped_early);
            let asked = options.cancel.is_some_and(CancelToken::is_cancelled) || stopped_early;
            if asked && !cancel_sent {
                cancel_sent = true;
                let _ = self.connection.cancel(&call.id);
            }
            if self.connection.is_ended() {
                // The reading thread fails every request when the output
                // ends; one more look takes that answer.
                break call.wait(Duration::from_millis(100)).unwrap_or_else(|| {
                    Err(RpcError::new(
                        ErrorKind::PluginUnavailable,
                        format!("{} stopped", self.name),
                    ))
                });
            }
        };
        self.lock_tokens().remove(&token);
        if let Some((partial, _)) = &batches {
            self.lock_tokens().remove(partial);
            self.lock_partials().remove(partial);
        }
        self.lock_limits().remove(&call.id);
        self.lock_labels().remove(&call.id);
        self.supervisor().finish(&call.id, self.ticker.now());
        if stopped_early
            && outcome
                .as_ref()
                .is_err_and(|e| e.code == ErrorKind::RequestCancelled.code())
        {
            return Ok(json!({"complete": false}));
        }
        outcome
    }

    fn lock_labels(&self) -> std::sync::MutexGuard<'_, HashMap<Id, String>> {
        self.labels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn label_of(&self, id: &Id) -> String {
        self.lock_labels()
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }

    fn lock_tokens(&self) -> std::sync::MutexGuard<'_, HashMap<String, Id>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_limits(&self) -> std::sync::MutexGuard<'_, HashMap<Id, Duration>> {
        self.limits.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The request a progress token belongs to.
    pub fn request_of(&self, token: &str) -> Option<Id> {
        self.lock_tokens().get(token).cloned()
    }

    /// Progress for a token: its request's timer restarts; `waiting` is
    /// the waiting-for-the-owner state when the progress carries one.
    pub fn progress(&self, token: &str, waiting: Option<bool>) {
        let Some(id) = self.request_of(token) else {
            return;
        };
        let now = self.ticker.now();
        let mut supervisor = self.supervisor();
        supervisor.activity(&id, now);
        if let Some(waiting) = waiting {
            supervisor.waiting(&id, waiting, now);
        }
    }

    /// A log line: every request's timer restarts.
    pub fn logged(&self) {
        let now = self.ticker.now();
        self.supervisor().activity_all(now);
    }

    /// Takes a lease: the plugin stays alive while it is held.
    pub fn lease(&self) {
        self.supervisor().lease();
    }

    /// Gives a lease back.
    pub fn release(&self) {
        let now = self.ticker.now();
        self.supervisor().release(now);
    }

    /// How many leases are held, counting requests in flight.
    pub fn leases(&self) -> usize {
        self.supervisor().leases()
    }

    /// Looks at the clock and acts: the cancel when a limit runs out,
    /// -32034 to the caller two seconds later. Returns what the host must
    /// do or say: [`Action::Restart`], [`Action::IdleStop`], and the
    /// notices.
    pub fn tick(&self) -> Vec<Action> {
        let now = self.ticker.now();
        let host_calls = self.connection.serving() > 0;
        let actions = self.supervisor().tick(now, host_calls);
        for action in &actions {
            match action {
                Action::Slow(id) => self.notice(Notice::Slow {
                    id: id.clone(),
                    label: self.label_of(id),
                }),
                Action::Cancel(id) => {
                    let _ = self.connection.cancel(id);
                }
                Action::TimedOut(id) => {
                    let limit = self.lock_limits().remove(id).unwrap_or_default();
                    // The notice goes first: giving the request up wakes its
                    // caller, who may look for the event at once.
                    self.notice(Notice::TimedOut {
                        id: id.clone(),
                        label: self.label_of(id),
                        after: limit,
                    });
                    self.connection.router().give_up(
                        id,
                        RpcError::new(
                            ErrorKind::TimedOut,
                            format!("no answer in {}", lines::duration_words(limit)),
                        ),
                    );
                }
                Action::Restart => {
                    let _ = self.connection.kill();
                }
                Action::IdleStop => {}
            }
        }
        actions
    }

    fn notice(&self, notice: Notice) {
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notice);
    }

    /// The notices since the last look.
    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *self.notices.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether the plugin's output has ended.
    pub fn is_ended(&self) -> bool {
        self.connection.is_ended()
    }

    /// The polite ending with the manifest's grace.
    pub fn stop(&self) -> Ending {
        self.connection.stop(self.shutdown_grace)
    }

    /// The polite ending within a budget: half for the answer to
    /// `shutdown`, half for the output to end, then a kill.
    pub fn stop_within(&self, budget: Duration) -> Ending {
        let half = budget / 2;
        self.connection
            .stop_within(half.min(self.shutdown_grace), budget - half)
    }

    /// Kills the plugin at once.
    pub fn kill(&self) {
        let _ = self.connection.kill();
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::log::PluginLog;
    use crate::router::{CancelFlag, Inbound};
    use crate::supervisor::ManualTicker;
    use crate::transport::{MemoryPeer, MemoryTransport};

    struct Quiet;

    impl Inbound for Quiet {
        fn request(&self, _: &crate::protocol::Request, _: &CancelFlag) -> Outcome {
            Ok(Value::Null)
        }
        fn notification(&self, _: &crate::protocol::Notification) {}
    }

    fn manifest(extra: &str) -> Manifest {
        let text = format!(
            "[plugin]\nid = \"line-counter\"\nname = \"Line counter\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = [\"x\"]\n{extra}"
        );
        Manifest::check(&text).manifest.unwrap()
    }

    fn live(extra: &str) -> (Arc<LivePlugin>, MemoryPeer, Arc<ManualTicker>) {
        let (host, peer) = MemoryTransport::pair("Line counter").unwrap();
        let connection = Connection::open(
            "Line counter",
            Arc::new(host),
            Arc::new(Quiet),
            PluginLog::in_memory(),
            Vec::new(),
        )
        .unwrap();
        let ticker = Arc::new(ManualTicker::new());
        let plugin = Arc::new(LivePlugin::new(
            &manifest(extra),
            connection,
            Arc::clone(&ticker) as Arc<dyn Ticker>,
        ));
        (plugin, peer, ticker)
    }

    fn sent(peer: &MemoryPeer) -> Value {
        serde_json::from_str(&peer.from_host.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap()
    }

    /// The `silent` case with the fake clock: no real waiting for the
    /// thirty seconds.
    #[test]
    fn a_silent_plugin_times_out_on_the_fake_clock() {
        let (plugin, peer, ticker) = live("");
        let caller = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || plugin.call("provider/stat", Params::new(), RequestKind::Request))
        };
        let request = sent(&peer);
        assert_eq!(request["params"]["progress_token"], "t1");
        ticker.advance(Duration::from_secs(2));
        plugin.tick();
        assert_eq!(
            plugin.take_notices(),
            [Notice::Slow {
                id: Id::Number(1),
                label: "Line counter".into()
            }]
        );
        ticker.advance(Duration::from_secs(28));
        plugin.tick();
        let cancel = sent(&peer);
        assert_eq!(cancel["method"], "$/cancel_request");
        ticker.advance(Duration::from_secs(2));
        plugin.tick();
        let outcome = caller.join().unwrap().unwrap_err();
        assert_eq!(outcome.code, -32034);
        assert_eq!(outcome.message, "no answer in 30 seconds");
        assert_eq!(
            plugin.take_notices(),
            [Notice::TimedOut {
                id: Id::Number(1),
                label: "Line counter".into(),
                after: Duration::from_secs(30)
            }]
        );
    }

    /// The `slow` case: progress keeps a long request alive.
    #[test]
    fn progress_keeps_a_slow_request_alive() {
        let (plugin, peer, ticker) = live("");
        let caller = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || plugin.call("provider/list", Params::new(), RequestKind::Request))
        };
        let request = sent(&peer);
        let token = request["params"]["progress_token"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..10 {
            ticker.advance(Duration::from_secs(20));
            plugin.progress(&token, None);
            plugin.tick();
        }
        assert!(peer.write_line(r#"{"jsonrpc":"2.0","id":1,"result":{"entries":[]}}"#));
        let outcome = caller.join().unwrap().unwrap();
        assert_eq!(outcome["entries"], json!([]));
    }

    /// The `crash` case: the output ends while a request waits.
    #[test]
    fn a_crash_fails_the_waiting_request() {
        let (plugin, mut peer, _ticker) = live("");
        let caller = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || plugin.call("command/run", Params::new(), RequestKind::Command))
        };
        let _ = sent(&peer);
        peer.end_output();
        let outcome = caller.join().unwrap().unwrap_err();
        assert_eq!(outcome.code, -32033);
        assert!(plugin.is_ended());
    }

    #[test]
    fn one_request_at_a_time_by_default() {
        let (plugin, peer, _ticker) = live("");
        let first = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || plugin.call("command/run", Params::new(), RequestKind::Command))
        };
        let second = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || plugin.call("command/run", Params::new(), RequestKind::Command))
        };
        let one = sent(&peer);
        // The second waits for the first's answer before it is sent.
        assert!(
            peer.from_host
                .recv_timeout(Duration::from_millis(200))
                .is_err()
        );
        assert!(peer.write_line(&format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#,
            one["id"]
        )));
        let two = sent(&peer);
        assert!(peer.write_line(&format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#,
            two["id"]
        )));
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
    }

    #[test]
    fn the_idle_stop_comes_after_the_last_lease() {
        let (plugin, _peer, ticker) = live("idle_timeout = \"5m\"\n");
        plugin.lease();
        ticker.advance(Duration::from_secs(600));
        assert!(!plugin.tick().contains(&Action::IdleStop));
        plugin.release();
        ticker.advance(Duration::from_secs(299));
        assert!(!plugin.tick().contains(&Action::IdleStop));
        ticker.advance(Duration::from_secs(1));
        assert!(plugin.tick().contains(&Action::IdleStop));
    }
}
