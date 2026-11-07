//! A task running on its own thread over a real watch and a real clock
//! (R16 3.8): the app's watched folders and `diro watch --task NAME`
//! without `--once`. A closed diroxide runs nothing; nothing here installs
//! a service.
//!
//! The task's `in` folder is watched, never recursively; a place that
//! cannot be watched is listed every 30 seconds instead and says so once
//! ("Scans: checking every 30 seconds"). Each note goes to the sink as it
//! comes. [`TaskRun::stop`] ends the run after the file in hand; dropping
//! the [`TaskRun`] interrupts the file in hand (it stays in `in`, and its
//! partial output is removed by the job), as Quit does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use diroxide_core::Clock;
use diroxide_vfs::{WatchEvent, WatchHandle};

use super::runner::{HotNote, Interrupter, StartError, TaskRunner};

/// How often a place that cannot be watched is listed.
pub const POLL_EVERY: Duration = Duration::from_secs(30);

/// What the run's thread is asked to do.
enum Ask {
    Event(WatchEvent),
    Resume,
    Pause(String),
    Rescan,
    Stop,
}

/// A task running on its own thread. See the module documentation.
#[derive(Debug)]
pub struct TaskRun {
    asks: Sender<Ask>,
    ended: Arc<AtomicBool>,
    interrupter: Interrupter,
    _watch: Option<WatchHandle>,
}

impl TaskRun {
    /// Ends the run after the file in hand.
    pub fn stop(&self) {
        let _ = self.asks.send(Ask::Stop);
    }

    /// Goes on after a pause.
    pub fn resume(&self) {
        let _ = self.asks.send(Ask::Resume);
    }

    /// Pauses the task with the owner's reason; the file in hand ends
    /// first.
    pub fn pause(&self, reason: &str) {
        let _ = self.asks.send(Ask::Pause(reason.to_owned()));
    }

    /// Lists `in` again ("Reload").
    pub fn rescan(&self) {
        let _ = self.asks.send(Ask::Rescan);
    }

    /// Whether the run's thread has ended.
    pub fn has_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }
}

impl Drop for TaskRun {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
        self.interrupter.interrupt();
        let _ = self.asks.send(Ask::Stop);
    }
}

/// Starts a task on its own thread: the start's checks here, at once, so
/// a refusal comes back to the caller; then the loop on the thread, each
/// note sent to `sink`. The provider's watch of `in` wakes it; a place
/// that cannot be watched is polled every [`POLL_EVERY`]. An error means
/// the task did not start, or its thread could not.
pub fn spawn_task(
    mut runner: TaskRunner,
    clock: Arc<dyn Clock>,
    sink: Box<dyn Fn(HotNote) + Send>,
) -> Result<TaskRun, StartError> {
    let (sender, receiver) = mpsc::channel::<Ask>();
    let input = runner.task().folders.input.clone();
    let events = sender.clone();
    let watch = runner
        .engine()
        .registry()
        .resolve(&input)
        .ok()
        .and_then(|provider| {
            provider
                .watch(
                    &input,
                    Box::new(move |event| {
                        let _ = events.send(Ask::Event(event));
                    }),
                )
                .ok()
                .flatten()
        });
    if watch.is_none() {
        runner = runner.with_polling(POLL_EVERY);
    }
    for note in runner.start(clock.now())? {
        sink(note);
    }
    let ended = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&ended);
    let interrupter = runner.interrupter();
    std::thread::Builder::new()
        .name("diroxide-task".to_owned())
        .spawn(move || {
            let finish = || done.store(true, Ordering::SeqCst);
            loop {
                let wait = runner
                    .next_wake(clock.now())
                    .unwrap_or(Duration::from_secs(60));
                let ask = match receiver.recv_timeout(wait) {
                    Ok(Ask::Stop) | Err(RecvTimeoutError::Disconnected) => return finish(),
                    Ok(ask) => Some(ask),
                    Err(RecvTimeoutError::Timeout) => None,
                };
                if done.load(Ordering::SeqCst) {
                    return finish();
                }
                let now = clock.now();
                let mut notes = match ask {
                    Some(Ask::Event(event)) => runner.event(&event, now),
                    Some(Ask::Resume) => runner.resume(now),
                    Some(Ask::Pause(reason)) => runner.pause(&reason, now),
                    Some(Ask::Rescan) => runner.rescan(now),
                    Some(Ask::Stop) | None => Vec::new(),
                };
                notes.extend(runner.wake(clock.now()));
                for note in notes {
                    if done.load(Ordering::SeqCst) {
                        return finish();
                    }
                    sink(note);
                }
                if runner.is_stopped() {
                    return finish();
                }
            }
        })
        .map_err(|e| StartError::Cannot(format!("Could not run the task: {e}.")))?;
    Ok(TaskRun {
        asks: sender,
        ended,
        interrupter,
        _watch: watch,
    })
}
