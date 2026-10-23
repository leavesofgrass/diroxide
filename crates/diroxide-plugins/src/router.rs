//! The router: which answer belongs to which request, in both directions.
//!
//! - **The host's requests** get ids counted from 1, never reused while
//!   the connection lives, so ids never collide. Each waits for its answer
//!   on a channel of its own. A cancelled request is still answered: the
//!   plugin answers -32800 or the result, and the caller takes whichever
//!   comes. An answer with an id nobody waits for (late, doubled, or made
//!   up) is logged and dropped.
//! - **The plugin's requests** are served on worker threads, at most
//!   [`MAX_INBOUND`] at once (more get -32031), each with a cancel flag the
//!   plugin can raise with `$/cancel_request`.
//!
//! [`Connection`] runs the one reading thread per plugin: it sorts every
//! frame, routes answers, hands requests and notifications to an
//! [`Inbound`] handler, logs stray output, answers lines that do not
//! parse, and fails every outstanding request when the plugin's output
//! ends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::PluginError;
use crate::framing::Frame;
use crate::log::{PluginLog, Source};
use crate::protocol::{
    ErrorKind, Id, Incoming, Notification, Params, Request, Response, RpcError, methods,
    parse_line, to_line,
};
use crate::session::Ending;
use crate::transport::PluginTransport;

/// How many of the plugin's requests the host serves at once.
pub const MAX_INBOUND: usize = 32;

/// How long a cancelled request may take to be answered before the host
/// stops waiting.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// How a request ended, as its caller sees it.
pub type Outcome = Result<Value, RpcError>;

/// A request the host sent and has not seen answered.
#[derive(Debug)]
struct Pending {
    method: String,
    reply: Sender<Outcome>,
    cancelled: bool,
}

/// What became of an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Routed {
    /// It went to the request that waits for it.
    Delivered,
    /// Nobody waits for this id: logged and dropped.
    Unknown,
}

/// The host's table of outstanding requests.
#[derive(Debug)]
pub struct Router {
    next: AtomicI64,
    pending: Mutex<HashMap<Id, Pending>>,
    unknown: AtomicUsize,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    /// An empty table; the first id is 1.
    pub fn new() -> Self {
        Self {
            next: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
            unknown: AtomicUsize::new(0),
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<Id, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a request: its new id and the channel its answer comes
    /// on.
    pub fn register(&self, method: &str) -> (Id, Receiver<Outcome>) {
        let id = Id::Number(self.next.fetch_add(1, Ordering::SeqCst));
        let (reply, answer) = mpsc::channel();
        self.table().insert(
            id.clone(),
            Pending {
                method: method.to_owned(),
                reply,
                cancelled: false,
            },
        );
        (id, answer)
    }

    /// Routes an answer to its request.
    pub fn route(&self, response: Response) -> Routed {
        let Some(id) = response.id.clone() else {
            self.unknown.fetch_add(1, Ordering::SeqCst);
            return Routed::Unknown;
        };
        let Some(pending) = self.table().remove(&id) else {
            self.unknown.fetch_add(1, Ordering::SeqCst);
            return Routed::Unknown;
        };
        let outcome = match (response.result, response.error) {
            (_, Some(error)) => Err(error),
            (Some(value), None) => Ok(value),
            (None, None) => Ok(Value::Null),
        };
        let _ = pending.reply.send(outcome);
        Routed::Delivered
    }

    /// Marks a request as cancelled; it is still answered. False when it
    /// is not outstanding.
    pub fn mark_cancelled(&self, id: &Id) -> bool {
        match self.table().get_mut(id) {
            Some(pending) => {
                pending.cancelled = true;
                true
            }
            None => false,
        }
    }

    /// Whether a request was cancelled and is still outstanding.
    pub fn is_cancelled(&self, id: &Id) -> bool {
        self.table().get(id).is_some_and(|p| p.cancelled)
    }

    /// Stops waiting for a request: its caller gets `error`, and a late
    /// answer is dropped as unknown. False when it was already answered.
    pub fn give_up(&self, id: &Id, error: RpcError) -> bool {
        match self.table().remove(id) {
            Some(pending) => {
                let _ = pending.reply.send(Err(error));
                true
            }
            None => false,
        }
    }

    /// Fails every outstanding request with `error` (the plugin stopped);
    /// how many there were.
    pub fn fail_all(&self, error: &RpcError) -> usize {
        let drained: Vec<Pending> = self.table().drain().map(|(_, p)| p).collect();
        for pending in &drained {
            let _ = pending.reply.send(Err(error.clone()));
        }
        drained.len()
    }

    /// How many requests are outstanding.
    pub fn outstanding(&self) -> usize {
        self.table().len()
    }

    /// The method of an outstanding request.
    pub fn method_of(&self, id: &Id) -> Option<String> {
        self.table().get(id).map(|p| p.method.clone())
    }

    /// How many answers were dropped because nobody waited for them.
    pub fn unknown_answers(&self) -> usize {
        self.unknown.load(Ordering::SeqCst)
    }
}

/// A flag the host raises when the plugin cancels one of its own requests.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// Whether the plugin cancelled the request.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Raises the flag.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Something the reading thread saw that is not a message for a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A line that was not a message, lossy, cut for the log.
    Stray(String),
    /// A message line that could not be read; it was answered with an
    /// error and a null id.
    ParseFailure(String),
    /// A line over the limit; refused.
    TooLong(u64),
    /// An answer nobody waited for; dropped.
    UnknownAnswer(Option<Id>),
    /// The plugin's output ended: it exited or was killed.
    Ended,
}

/// What the host does with the plugin's requests and notifications. The
/// app's services implement it (host services); tests use small fakes.
pub trait Inbound: Send + Sync {
    /// Serves one of the plugin's requests, on a worker thread. `cancel`
    /// rises when the plugin cancels it.
    fn request(&self, request: &Request, cancel: &CancelFlag) -> Outcome;

    /// Takes one of the plugin's notifications, on the reading thread; it
    /// must not block.
    fn notification(&self, notification: &Notification);

    /// Hears what the reading thread saw, on the reading thread.
    fn event(&self, event: &Event) {
        let _ = event;
    }
}

struct Shared {
    name: String,
    transport: Arc<dyn PluginTransport>,
    router: Router,
    inbound: Arc<dyn Inbound>,
    log: PluginLog,
    serving: AtomicUsize,
    cancels: Mutex<HashMap<Id, CancelFlag>>,
    ended: AtomicBool,
    stopping: AtomicBool,
}

impl Shared {
    fn send<T: serde::Serialize>(&self, message: &T) -> Result<(), PluginError> {
        let line = to_line(message).map_err(|e| PluginError::Protocol(e.to_string()))?;
        self.transport.send(line)
    }

    fn answer(&self, id: Option<Id>, outcome: Outcome) {
        let response = match (id, outcome) {
            (Some(id), Ok(value)) => Response::success(id, value),
            (id, Err(error)) => Response::failure(id, error),
            (None, Ok(_)) => return,
        };
        if self.send(&response).is_err() {
            self.log.write(Source::Host, "an answer could not be sent");
        }
    }

    fn take(&self, incoming: Incoming, me: &Arc<Shared>) {
        match incoming {
            Incoming::Response(response) => {
                let id = response.id.clone();
                if self.router.route(response) == Routed::Unknown {
                    self.log.write(
                        Source::Host,
                        &format!("dropped an answer nobody waited for, id {id:?}"),
                    );
                    self.inbound.event(&Event::UnknownAnswer(id));
                }
            }
            Incoming::Notification(notification) => {
                if notification.method == methods::CANCEL_REQUEST {
                    if let Some(id) = notification
                        .params
                        .get("id")
                        .and_then(|v| serde_json::from_value::<Id>(v.clone()).ok())
                        && let Some(flag) = self.cancel_flags().get(&id)
                    {
                        flag.cancel();
                    }
                    return;
                }
                self.inbound.notification(&notification);
            }
            Incoming::Request(request) => {
                if self.serving.fetch_add(1, Ordering::SeqCst) >= MAX_INBOUND {
                    self.serving.fetch_sub(1, Ordering::SeqCst);
                    self.answer(
                        Some(request.id),
                        Err(RpcError::new(
                            ErrorKind::LimitExceeded,
                            "too many requests at once",
                        )),
                    );
                    return;
                }
                let flag = CancelFlag::default();
                self.cancel_flags().insert(request.id.clone(), flag.clone());
                let shared = Arc::clone(me);
                let spawned = thread::Builder::new()
                    .name("plugin-service".to_owned())
                    .spawn(move || {
                        let outcome = shared.inbound.request(&request, &flag);
                        shared.cancel_flags().remove(&request.id);
                        shared.answer(Some(request.id.clone()), outcome);
                        shared.serving.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    self.serving.fetch_sub(1, Ordering::SeqCst);
                    self.log
                        .write(Source::Host, "a service thread could not start");
                }
            }
        }
    }

    fn cancel_flags(&self) -> std::sync::MutexGuard<'_, HashMap<Id, CancelFlag>> {
        self.cancels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_loop(me: Arc<Shared>, early: Vec<Incoming>) {
        for incoming in early {
            me.take(incoming, &me);
        }
        loop {
            let frame = match me.transport.receive(Duration::from_millis(200)) {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(_) => break,
            };
            match frame {
                Frame::Message(text) => match parse_line(&text) {
                    Ok(incoming) => me.take(incoming, &me),
                    Err(error) => {
                        me.log.write(
                            Source::Host,
                            &format!("a line could not be read: {}", error.message),
                        );
                        me.inbound
                            .event(&Event::ParseFailure(error.message.clone()));
                        me.answer(None, Err(error));
                    }
                },
                Frame::NotUtf8(preview) => {
                    me.log
                        .write(Source::Host, &format!("a line is not UTF-8: {preview}"));
                    me.inbound
                        .event(&Event::ParseFailure("the line is not UTF-8".to_owned()));
                    me.answer(
                        None,
                        Err(RpcError::new(
                            ErrorKind::ParseError,
                            "the line is not UTF-8",
                        )),
                    );
                }
                Frame::TooLong { length, message } => {
                    me.log.write(
                        Source::Host,
                        &format!("refused a line of {length} bytes, over the limit"),
                    );
                    me.inbound.event(&Event::TooLong(length));
                    if message {
                        me.answer(
                            None,
                            Err(RpcError::new(
                                ErrorKind::LimitExceeded,
                                "the line is longer than the limit",
                            )),
                        );
                    }
                }
                Frame::Stray(text) => {
                    me.log.write(Source::Stray, &text);
                    me.inbound.event(&Event::Stray(text));
                }
                Frame::Blank => {}
            }
        }
        me.ended.store(true, Ordering::SeqCst);
        let error = RpcError::new(ErrorKind::PluginUnavailable, format!("{} stopped", me.name));
        me.router.fail_all(&error);
        if !me.stopping.load(Ordering::SeqCst) {
            me.log.write(Source::Host, "the plugin's output ended");
        }
        me.inbound.event(&Event::Ended);
    }
}

/// A request in flight: its id, and where its answer comes.
#[derive(Debug)]
pub struct Call {
    /// The request's id.
    pub id: Id,
    answer: Receiver<Outcome>,
}

impl Call {
    /// Waits up to `timeout` for the answer; `None` when none came.
    pub fn wait(&self, timeout: Duration) -> Option<Outcome> {
        self.answer.recv_timeout(timeout).ok()
    }
}

/// One live plugin's connection: a transport, the router and the reading
/// thread.
pub struct Connection {
    shared: Arc<Shared>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("name", &self.shared.name)
            .field("outstanding", &self.shared.router.outstanding())
            .finish()
    }
}

impl Connection {
    /// Starts reading a plugin's output after its handshake; `early` holds
    /// the messages that came with the `initialize` answer.
    pub fn open(
        name: &str,
        transport: Arc<dyn PluginTransport>,
        inbound: Arc<dyn Inbound>,
        log: PluginLog,
        early: Vec<Incoming>,
    ) -> Result<Self, PluginError> {
        let shared = Arc::new(Shared {
            name: name.to_owned(),
            transport,
            router: Router::new(),
            inbound,
            log,
            serving: AtomicUsize::new(0),
            cancels: Mutex::new(HashMap::new()),
            ended: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        let reader = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("plugin-router".to_owned())
                .spawn(move || Shared::read_loop(shared, early))
                .map_err(|e| PluginError::Io(e.to_string()))?
        };
        Ok(Self {
            shared,
            reader: Mutex::new(Some(reader)),
        })
    }

    /// The plugin's name.
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// The router.
    pub fn router(&self) -> &Router {
        &self.shared.router
    }

    /// The plugin's log.
    pub fn log(&self) -> &PluginLog {
        &self.shared.log
    }

    /// Whether the plugin's output has ended.
    pub fn is_ended(&self) -> bool {
        self.shared.ended.load(Ordering::SeqCst)
    }

    /// How many of the plugin's requests the host is serving now.
    pub fn serving(&self) -> usize {
        self.shared.serving.load(Ordering::SeqCst)
    }

    /// Sends a request; the answer comes on the returned [`Call`].
    pub fn request(&self, method: &str, params: Params) -> Result<Call, PluginError> {
        self.request_then(method, params, |_| {})
    }

    /// Sends a request, after `registered` has seen its id, so whatever
    /// keeps track of the request (its timer, its token) is in place before
    /// the plugin can answer.
    pub fn request_then(
        &self,
        method: &str,
        params: Params,
        registered: impl FnOnce(&Id),
    ) -> Result<Call, PluginError> {
        if self.is_ended() {
            return Err(PluginError::Stopped(self.shared.name.clone()));
        }
        let (id, answer) = self.shared.router.register(method);
        registered(&id);
        let request = Request::new(id.clone(), method, params);
        if let Err(e) = self.shared.send(&request) {
            self.shared.router.give_up(
                &id,
                RpcError::new(ErrorKind::PluginUnavailable, e.to_string()),
            );
            return Err(e);
        }
        if self.is_ended() {
            // The output ended while this request was being sent, perhaps
            // after the reading thread failed the others: nothing will
            // answer it.
            self.shared.router.give_up(
                &id,
                RpcError::new(
                    ErrorKind::PluginUnavailable,
                    format!("{} stopped", self.shared.name),
                ),
            );
        }
        Ok(Call { id, answer })
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Params) -> Result<(), PluginError> {
        self.shared.send(&Notification::new(method, params))
    }

    /// Cancels a request: sends `$/cancel_request` and keeps waiting for
    /// its answer, which still comes.
    pub fn cancel(&self, id: &Id) -> Result<(), PluginError> {
        if !self.shared.router.mark_cancelled(id) {
            return Ok(());
        }
        let mut params = Params::new();
        params.insert("id".to_owned(), json!(id));
        self.notify(methods::CANCEL_REQUEST, params)
    }

    /// Sends a request and waits up to `timeout` for its answer. When none
    /// comes, the request is cancelled and given [`CANCEL_GRACE`] to be
    /// answered; after that its caller gets -32034.
    pub fn call(&self, method: &str, params: Params, timeout: Duration) -> Outcome {
        let call = self
            .request(method, params)
            .map_err(|e| RpcError::new(ErrorKind::PluginUnavailable, e.to_string()))?;
        if let Some(outcome) = call.wait(timeout) {
            return outcome;
        }
        let _ = self.cancel(&call.id);
        if let Some(outcome) = call.wait(CANCEL_GRACE) {
            return outcome;
        }
        let error = RpcError::new(ErrorKind::TimedOut, "no answer in time");
        self.shared.router.give_up(&call.id, error.clone());
        call.wait(Duration::ZERO).unwrap_or(Err(error))
    }

    /// The polite ending through the router: `shutdown` with up to `grace`
    /// for its answer, `exit`, the input closed, two seconds for the
    /// output to end, then a kill.
    pub fn stop(&self, grace: Duration) -> Ending {
        self.stop_within(grace, Duration::from_secs(2))
    }

    /// The polite ending with both waits chosen: up to `grace` for the
    /// answer to `shutdown` and up to `end_wait` for the output to end
    /// after `exit`; then a kill. Never waits longer than the two.
    pub fn stop_within(&self, grace: Duration, end_wait: Duration) -> Ending {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let answered = matches!(
            self.request(methods::SHUTDOWN, Params::new())
                .ok()
                .and_then(|call| call.wait(grace)),
            Some(Ok(_))
        );
        let _ = self.notify(methods::EXIT, Params::new());
        self.shared.transport.close_input();
        let deadline = Instant::now() + end_wait;
        while !self.is_ended() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let ended = self.is_ended();
        if !ended {
            let _ = self.shared.transport.kill();
        }
        let reader = self
            .reader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(reader) = reader {
            let _ = reader.join();
        }
        if answered && ended {
            Ending::Polite
        } else {
            Ending::Killed
        }
    }

    /// Whether the host ended the plugin (a stop or a kill), so an end of
    /// its output is not a crash.
    pub fn stopped_by_host(&self) -> bool {
        self.shared.stopping.load(Ordering::SeqCst)
    }

    /// Kills the plugin as a crash would, without telling the router: for
    /// tests of the crash policy.
    #[cfg(test)]
    pub(crate) fn kill_unexpectedly(&self) {
        let _ = self.shared.transport.kill();
    }

    /// Kills the plugin at once, without the polite ending.
    pub fn kill(&self) -> Result<(), PluginError> {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.transport.kill()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.is_ended() {
            let _ = self.shared.transport.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;
    use crate::testing::{FakePlugin, Mode};
    use crate::transport::{MemoryPeer, MemoryTransport};

    const WAIT: Duration = Duration::from_secs(5);

    /// Records what the plugin sent the host.
    #[derive(Default)]
    struct Recorder {
        notes: Mutex<Vec<String>>,
        events: Mutex<Vec<Event>>,
    }

    impl Inbound for Recorder {
        fn request(&self, request: &Request, cancel: &CancelFlag) -> Outcome {
            match request.method.as_str() {
                "test/slow" => {
                    let start = Instant::now();
                    while !cancel.is_cancelled() && start.elapsed() < WAIT {
                        thread::sleep(Duration::from_millis(5));
                    }
                    if cancel.is_cancelled() {
                        Err(RpcError::new(ErrorKind::RequestCancelled, "cancelled"))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Ok(json!({"echo": request.method})),
            }
        }

        fn notification(&self, notification: &Notification) {
            self.notes.lock().unwrap().push(notification.method.clone());
        }

        fn event(&self, event: &Event) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    fn open(recorder: &Arc<Recorder>) -> (Connection, MemoryPeer) {
        let (host, peer) = MemoryTransport::pair("Line counter").unwrap();
        let connection = Connection::open(
            "Line counter",
            Arc::new(host),
            Arc::clone(recorder) as Arc<dyn Inbound>,
            PluginLog::in_memory(),
            Vec::new(),
        )
        .unwrap();
        (connection, peer)
    }

    fn next_line(peer: &MemoryPeer) -> Value {
        serde_json::from_str(&peer.from_host.recv_timeout(WAIT).unwrap()).unwrap()
    }

    #[test]
    fn ids_count_from_one_and_never_collide() {
        let router = Router::new();
        let ids: Vec<Id> = (0..100).map(|_| router.register("x").0).collect();
        assert_eq!(ids[0], Id::Number(1));
        let unique: BTreeSet<String> = ids.iter().map(|id| format!("{id:?}")).collect();
        assert_eq!(unique.len(), 100);
    }

    #[test]
    fn a_cancelled_request_is_still_answered_and_unknown_answers_are_dropped() {
        let recorder = Arc::new(Recorder::default());
        let (connection, peer) = open(&recorder);
        let call = connection
            .request(methods::COMMAND_RUN, Params::new())
            .unwrap();
        let sent = next_line(&peer);
        assert_eq!(sent["id"], json!(1));
        connection.cancel(&call.id).unwrap();
        let cancel = next_line(&peer);
        assert_eq!(cancel["method"], "$/cancel_request");
        assert_eq!(cancel["params"]["id"], json!(1));
        // An answer for an id nobody waits for comes first, then the real
        // answer to the cancelled request.
        assert!(peer.write_line(r#"{"jsonrpc":"2.0","id":99,"result":true}"#));
        assert!(peer.write_line(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32800,"message":"cancelled"}}"#
        ));
        let outcome = call.wait(WAIT).unwrap();
        assert_eq!(outcome.unwrap_err().code, -32800);
        assert_eq!(connection.router().unknown_answers(), 1);
        assert!(
            recorder
                .events
                .lock()
                .unwrap()
                .contains(&Event::UnknownAnswer(Some(Id::Number(99))))
        );
    }

    #[test]
    fn the_plugins_requests_are_served_and_can_be_cancelled() {
        let recorder = Arc::new(Recorder::default());
        let (_connection, peer) = open(&recorder);
        assert!(peer.write_line(r#"{"jsonrpc":"2.0","id":"a","method":"panel/get"}"#));
        let answer = next_line(&peer);
        assert_eq!(answer["id"], "a");
        assert_eq!(answer["result"]["echo"], "panel/get");
        assert!(peer.write_line(r#"{"jsonrpc":"2.0","id":"slow","method":"test/slow"}"#));
        thread::sleep(Duration::from_millis(50));
        assert!(
            peer.write_line(
                r#"{"jsonrpc":"2.0","method":"$/cancel_request","params":{"id":"slow"}}"#
            )
        );
        let cancelled = next_line(&peer);
        assert_eq!(cancelled["id"], "slow");
        assert_eq!(cancelled["error"]["code"], -32800);
        assert!(peer.write_line(r#"{"jsonrpc":"2.0","method":"log","params":{}}"#));
        thread::sleep(Duration::from_millis(50));
        assert_eq!(*recorder.notes.lock().unwrap(), ["log"]);
    }

    #[test]
    fn bad_lines_are_answered_with_a_null_id_and_stray_text_is_logged() {
        let recorder = Arc::new(Recorder::default());
        let (connection, peer) = open(&recorder);
        assert!(peer.write_line("debug: hello"));
        assert!(peer.write_line("{not json"));
        let answer = next_line(&peer);
        assert_eq!(answer["id"], Value::Null);
        assert_eq!(answer["error"]["code"], -32700);
        assert!(peer.write_line("[1,2]"));
        assert_eq!(next_line(&peer)["error"]["code"], -32600);
        thread::sleep(Duration::from_millis(50));
        let events = recorder.events.lock().unwrap().clone();
        assert!(
            events.contains(&Event::Stray("debug: hello".into())),
            "{events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::ParseFailure(_)))
                .count(),
            2
        );
        drop(connection);
    }

    #[test]
    fn when_the_output_ends_every_request_fails_with_plugin_unavailable() {
        let recorder = Arc::new(Recorder::default());
        let (connection, mut peer) = open(&recorder);
        let call = connection
            .request(methods::COMMAND_RUN, Params::new())
            .unwrap();
        peer.end_output();
        let outcome = call.wait(WAIT).unwrap().unwrap_err();
        assert_eq!(outcome.code, -32033);
        assert_eq!(outcome.message, "Line counter stopped");
        for _ in 0..200 {
            if recorder.events.lock().unwrap().contains(&Event::Ended) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(connection.is_ended());
        assert!(recorder.events.lock().unwrap().contains(&Event::Ended));
        assert!(connection.request("x", Params::new()).is_err());
    }

    #[test]
    fn a_call_that_is_never_answered_times_out_after_its_cancel() {
        let recorder = Arc::new(Recorder::default());
        let (connection, peer) = open(&recorder);
        let outcome = connection.call("test/never", Params::new(), Duration::from_millis(50));
        assert_eq!(outcome.unwrap_err().code, -32034);
        let first = next_line(&peer);
        assert_eq!(first["method"], "test/never");
        let cancel = next_line(&peer);
        assert_eq!(cancel["method"], "$/cancel_request");
        assert_eq!(connection.router().outstanding(), 0);
    }

    #[test]
    fn a_connection_stops_a_fake_plugin_politely() {
        let (host, peer) = MemoryTransport::pair("Fake").unwrap();
        let plugin = thread::spawn(move || FakePlugin::new(Mode::Echo).serve(peer));
        let host: Arc<dyn PluginTransport> = Arc::new(host);
        let started = crate::session::handshake(
            host.as_ref(),
            "Fake",
            &crate::session::InitializeParams::for_plugin("fake"),
            WAIT,
        )
        .unwrap();
        let connection = Connection::open(
            "Fake",
            host,
            Arc::new(Recorder::default()),
            PluginLog::in_memory(),
            started.early,
        )
        .unwrap();
        let mut params = Params::new();
        params.insert("command".into(), json!("echo"));
        let outcome = connection.call(methods::COMMAND_RUN, params, WAIT).unwrap();
        assert_eq!(outcome["done"], 1);
        assert_eq!(connection.stop(WAIT), Ending::Polite);
        let seen = plugin.join().unwrap();
        assert_eq!(
            seen.methods(),
            [
                methods::INITIALIZE,
                methods::INITIALIZED,
                methods::COMMAND_RUN,
                methods::SHUTDOWN,
                methods::EXIT
            ]
        );
    }

    /// One step of an interleaving.
    #[derive(Clone, Debug)]
    enum Step {
        Start,
        Answer(usize),
        Cancel(usize),
        Unknown(i64),
        Again(usize),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            Just(Step::Start),
            (0usize..16).prop_map(Step::Answer),
            (0usize..16).prop_map(Step::Cancel),
            (1000i64..2000).prop_map(Step::Unknown),
            (0usize..16).prop_map(Step::Again),
        ]
    }

    proptest! {
        /// Any interleaving of requests, answers, cancels, made-up answers
        /// and doubled answers: every request gets exactly one outcome, a
        /// cancelled one too; ids never repeat; and every answer nobody
        /// waited for is counted as dropped.
        #[test]
        fn interleavings_keep_one_outcome_per_request(steps in proptest::collection::vec(step(), 1..80)) {
            let router = Router::new();
            let mut calls: Vec<(Id, Receiver<Outcome>, bool)> = Vec::new();
            let mut answered = BTreeSet::new();
            let mut dropped = 0;
            for s in steps {
                match s {
                    Step::Start => {
                        let (id, rx) = router.register("x");
                        prop_assert!(calls.iter().all(|(other, _, _)| *other != id));
                        calls.push((id, rx, false));
                    }
                    Step::Answer(k) | Step::Again(k) => {
                        if let Some((id, _, _)) = calls.get(k) {
                            let routed = router.route(Response::success(id.clone(), json!(k)));
                            if answered.insert(k) {
                                prop_assert_eq!(routed, Routed::Delivered);
                            } else {
                                prop_assert_eq!(routed, Routed::Unknown);
                                dropped += 1;
                            }
                        }
                    }
                    Step::Cancel(k) => {
                        if let Some((id, _, cancelled)) = calls.get_mut(k) {
                            let outstanding = router.mark_cancelled(id);
                            prop_assert_eq!(outstanding, !answered.contains(&k));
                            *cancelled |= outstanding;
                        }
                    }
                    Step::Unknown(n) => {
                        prop_assert_eq!(router.route(Response::success(Id::Number(n), json!(0))), Routed::Unknown);
                        dropped += 1;
                    }
                }
            }
            // Every cancelled request is still answered.
            for (k, (id, _, cancelled)) in calls.iter().enumerate() {
                if *cancelled && !answered.contains(&k) {
                    prop_assert!(router.is_cancelled(id));
                    prop_assert_eq!(router.route(Response::failure(Some(id.clone()), RpcError::new(ErrorKind::RequestCancelled, "cancelled"))), Routed::Delivered);
                    answered.insert(k);
                }
            }
            let left = router.fail_all(&RpcError::new(ErrorKind::PluginUnavailable, "stopped"));
            prop_assert_eq!(left + answered.len(), calls.len());
            for (_, rx, _) in &calls {
                prop_assert!(rx.try_recv().is_ok());
                prop_assert!(rx.try_recv().is_err());
            }
            prop_assert_eq!(router.unknown_answers(), dropped);
        }
    }
}
