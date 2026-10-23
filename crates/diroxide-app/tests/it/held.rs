//! A fake place whose reads wait at a gate, and whose listings can arrive
//! in batches: the queue's and the streamed listing's tests hold jobs
//! running, and folders half listed, for as long as they need, with no
//! timer and no network. Every other call is the fake provider's own.

use std::io::Read;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use diroxide_app::AppEvent;
use diroxide_core::{Effect, Entry, Place};
use diroxide_ops::FakeProvider;
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, LinkRequest, OpContext, Provider,
    ReadSeek, SpaceInfo, StreamingList, TrashCheck, TrashGate, VfsError, VolumeInfo, WatchHandle,
    WatchSink, WriteMode,
};

use crate::support::Session;

/// The fake provider behind a gate: reads wait while it is closed, and a
/// listing comes in batches when `batch` is set.
#[derive(Clone, Debug)]
pub struct Held {
    /// The fake that holds the files.
    pub fake: FakeProvider,
    gate: Arc<(Mutex<bool>, Condvar)>,
    batch: Arc<AtomicUsize>,
    /// Batches handed out so far.
    pub batches: Arc<AtomicUsize>,
}

impl Held {
    /// A held fake with its gate open and listings whole.
    pub fn new(fake: FakeProvider) -> Self {
        Self {
            fake,
            gate: Arc::new((Mutex::new(true), Condvar::new())),
            batch: Arc::new(AtomicUsize::new(0)),
            batches: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Closes the gate: reads wait.
    pub fn close(&self) {
        *self.gate.0.lock().unwrap() = false;
    }

    /// Opens the gate: waiting reads go on.
    pub fn open(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }

    /// Lists in batches of this many entries (0: whole).
    #[allow(dead_code)]
    pub fn set_batch(&self, entries: usize) {
        self.batch.store(entries, Ordering::SeqCst);
    }

    fn wait(&self, ctx: &OpContext) -> Result<(), VfsError> {
        let (open, changed) = &*self.gate;
        let mut guard = open.lock().unwrap();
        while !*guard {
            if ctx.cancel.is_cancelled() {
                return Err(VfsError::Cancelled);
            }
            guard = changed
                .wait_timeout(guard, Duration::from_millis(10))
                .unwrap()
                .0;
        }
        Ok(())
    }
}

impl Provider for Held {
    fn scheme(&self) -> &str {
        self.fake.scheme()
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        self.fake.capabilities(place)
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.fake.list(place, ctx)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.fake.stat(place, ctx)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.wait(ctx)?;
        self.fake.read(place, ctx)
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.wait(ctx)?;
        self.fake.open_seekable(place, ctx)
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.fake.write(place, mode, ctx)
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.create_dir(place, ctx)
    }

    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        self.fake.create_link(place, request, ctx)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.rename(from, to, ctx)
    }

    fn rename_replacing(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.rename_replacing(from, to, ctx)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.remove(place, ctx)
    }

    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        self.fake.trash_gate(place, ctx)
    }

    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        self.fake.trash(place, ctx)
    }

    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.restore(original, ctx)
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.fake.set_attributes(place, change, ctx)
    }

    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        self.fake.copy_hint(from, to)
    }

    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.copy_within(from, to, ctx)
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        self.fake.volume(place, ctx)
    }

    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        self.fake.space(place, ctx)
    }

    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        self.fake.watch(place, sink)
    }

    fn as_streaming(&self) -> Option<&dyn StreamingList> {
        (self.batch.load(Ordering::SeqCst) > 0).then_some(self as &dyn StreamingList)
    }
}

impl StreamingList for Held {
    fn list_batches(
        &self,
        place: &Place,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<Entry>) -> ControlFlow<()>,
    ) -> Result<(), VfsError> {
        let entries = self.fake.list(place, ctx)?;
        let size = self.batch.load(Ordering::SeqCst).max(1);
        for (n, chunk) in entries.chunks(size).enumerate() {
            // After the first batch the gate holds the rest, so a test can
            // look at a half-listed folder.
            if n > 0 {
                self.wait(ctx)?;
            }
            ctx.check()?;
            self.batches.fetch_add(1, Ordering::SeqCst);
            if batch(chunk.to_vec()).is_break() {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Carries out effects and feeds the runtime's events back to the app
/// until `done` holds and the runtime has been quiet a moment, or ten
/// seconds pass; returns what was said. Unlike `Session::run`, it does
/// not wait for jobs held at a gate.
pub fn drive(
    s: &mut Session,
    effects: Vec<Effect>,
    done: impl Fn(&Session, &[String]) -> bool,
) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut said = Vec::new();
    let mut pending = effects;
    let mut quiet = Instant::now();
    loop {
        let screen = s
            .runtime
            .carry_out(&mut s.app, std::mem::take(&mut pending));
        for effect in screen {
            if let Effect::Announce(message) = &effect {
                said.push(message.text.clone());
                s.said.push(message.clone());
            }
            s.effects.push(effect);
        }
        let events = s.runtime.wait(Duration::from_millis(10));
        if events.is_empty() {
            let settled = quiet.elapsed() > Duration::from_millis(60);
            if (settled && done(s, &said)) || Instant::now() > deadline {
                return said;
            }
            continue;
        }
        quiet = Instant::now();
        pending = events
            .into_iter()
            .flat_map(|event| s.app.handle(event))
            .collect();
    }
}

/// Presses a key and drives until `done`.
pub fn key_until(
    s: &mut Session,
    key: &str,
    done: impl Fn(&Session, &[String]) -> bool,
) -> Vec<String> {
    let effects = s
        .app
        .handle(AppEvent::Key(diroxide_core::Key::parse(key).unwrap()));
    drive(s, effects, done)
}

/// Presses a key and drives until the runtime is quiet.
pub fn key_held(s: &mut Session, key: &str) -> Vec<String> {
    key_until(s, key, |_, _| true)
}

/// Runs a command and drives until the runtime is quiet.
pub fn command_held(s: &mut Session, id: diroxide_core::CommandId) -> Vec<String> {
    let effects = s
        .app
        .handle(AppEvent::Command(diroxide_core::Command::new(id)));
    drive(s, effects, |_, _| true)
}

/// Whether anything said starts with `start`.
pub fn said_start(said: &[String], start: &str) -> bool {
    said.iter().any(|line| line.starts_with(start))
}
