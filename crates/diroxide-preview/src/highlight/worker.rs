//! The highlight worker: one thread per file, parsing in chunks with the
//! cancel and the deadline checked between them, the threads run one at
//! a time, and the session paused for good when a parse overruns its
//! deadline (a hang inside one line cannot be cancelled in Rust, so the
//! thread is left and no other is started after it).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::{Caps, Highlighted, Language, Refusal, highlight_with};

/// How long loading the syntax set may take before the parse starts.
const LOAD_ALLOWANCE: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct Inner {
    paused: AtomicBool,
    runner: Mutex<()>,
}

/// Highlighting for one run of the program: the app keeps one and hands
/// it to each viewer, so a parse that overran its deadline pauses
/// highlighting everywhere until the next start.
#[derive(Clone, Debug, Default)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// A session with highlighting on.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a parse overran its deadline, so highlighting waits for the
    /// next session.
    pub fn is_paused(&self) -> bool {
        self.inner.paused.load(Ordering::Relaxed)
    }

    fn pause(&self) {
        self.inner.paused.store(true, Ordering::Relaxed);
    }

    /// Starts highlighting a file's lines on the worker, within the caps;
    /// `bytes` is the file's size. Refused, in words, when the file is over
    /// a cap, the session is paused, or the build has no syntax set.
    pub fn start(
        &self,
        language: Language,
        lines: Arc<Vec<String>>,
        bytes: u64,
        caps: Caps,
    ) -> Result<Job, Refusal> {
        if !super::is_built() {
            return Err(Refusal::NotInBuild);
        }
        if self.is_paused() {
            return Err(Refusal::Paused);
        }
        caps.check(bytes, &lines)?;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let inner = Arc::clone(&self.inner);
        let stop = Arc::clone(&cancel);
        let deadline = caps.deadline;
        thread::Builder::new()
            .name("diroxide-highlight".to_owned())
            .spawn(move || {
                // One parse at a time: a thread waits for the one before.
                let _turn = inner.runner.lock().unwrap_or_else(PoisonError::into_inner);
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // The first use loads the syntax set; the deadline counts
                // from the parse, after it.
                let _ = super::load_syntax_set();
                let since = Instant::now();
                let _ = tx.send(Event::Parsing(since));
                let result = highlight_with(language, &lines, Some(since + deadline), &|| {
                    stop.load(Ordering::Relaxed)
                });
                let _ = tx.send(Event::Done(result));
            })
            .map_err(|_| Refusal::NoGrammar)?;
        Ok(Job {
            rx,
            cancel,
            session: self.clone(),
            deadline,
            asked: Instant::now(),
            parsing_since: None,
            finished: false,
        })
    }
}

enum Event {
    Parsing(Instant),
    Done(Result<Highlighted, Refusal>),
}

/// A highlighting in progress. Dropping it cancels it.
#[derive(Debug)]
pub struct Job {
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    session: Session,
    deadline: Duration,
    asked: Instant,
    parsing_since: Option<Instant>,
    finished: bool,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Parsing(_) => f.write_str("Parsing"),
            Event::Done(_) => f.write_str("Done"),
        }
    }
}

impl Job {
    /// The result once the parse is done, exactly once: the classes, or
    /// why not ("Highlighting stopped: took too long", which also pauses
    /// the session). `None` while it runs.
    pub fn poll(&mut self) -> Option<Result<Highlighted, Refusal>> {
        if self.finished {
            return None;
        }
        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    if let Some(result) = self.take(event) {
                        return Some(result);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Some(self.lost()),
            }
        }
        self.overdue()
    }

    /// Waits for the result: at most the deadline after the parse starts.
    pub fn wait(&mut self) -> Result<Highlighted, Refusal> {
        loop {
            if let Some(result) = self.poll() {
                return result;
            }
            if self.finished {
                return Err(Refusal::TookTooLong);
            }
            match self.rx.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => {
                    if let Some(result) = self.take(event) {
                        return result;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return self.lost(),
            }
        }
    }

    /// Stops the parse at its next chunk.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn take(&mut self, event: Event) -> Option<Result<Highlighted, Refusal>> {
        match event {
            Event::Parsing(at) => {
                self.parsing_since = Some(at);
                None
            }
            Event::Done(result) => {
                self.finished = true;
                if matches!(result, Err(Refusal::TookTooLong)) {
                    self.session.pause();
                }
                Some(result)
            }
        }
    }

    /// The thread ended without a result: cancelled, or it failed.
    fn lost(&mut self) -> Result<Highlighted, Refusal> {
        self.finished = true;
        Err(Refusal::NoGrammar)
    }

    /// The deadline passed with no result: the thread is left, and the
    /// session is paused.
    fn overdue(&mut self) -> Option<Result<Highlighted, Refusal>> {
        let late = match self.parsing_since {
            Some(since) => since.elapsed() > self.deadline,
            None => self.asked.elapsed() > LOAD_ALLOWANCE,
        };
        if !late {
            return None;
        }
        self.cancel();
        self.session.pause();
        self.finished = true;
        Some(Err(Refusal::TookTooLong))
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}
