//! The readers of the custom columns (alpha 4, lane MA, R30 commit 52):
//! each file's cells read through the metadata pipeline in the order the
//! app gave, the rows on screen first, and sent in batches: a small first
//! batch for the rows on screen, then up to [`BATCH`] at a time or every
//! [`BATCH_WAIT`].
//!
//! They mirror the type check's readers (`runtime::typing`) and share its
//! rules: four readers on a disk of this computer and one on a share
//! (the type check's own count of the folder's disk), each panel's reads
//! counted so a newer read of the panel, or a stop, sends the readers of
//! the older one away at their next file with their unsent cells
//! dropped. Each reader runs the pipeline's readers of the cost classes
//! the file's keys need, every one under the pipeline's guard.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use diroxide_core::{Place, Side};
use diroxide_meta::pipeline::{EXTENDED, EXTENDED_KEYS};
use diroxide_meta::{Cell, Cost, MetaError, Outcome, Pipeline, ReadLimits, ReadRequest};
use diroxide_vfs::{OpContext, Registry};

use super::Shared;
use super::typing::{BATCH, BATCH_WAIT, readers_for};
use crate::app::AppEvent;
use crate::columns::{ColumnCell, ColumnItem};

/// Each panel's count of column reads.
#[derive(Debug, Default)]
pub(super) struct ColumnReaders {
    counts: [Arc<AtomicU64>; 2],
}

impl ColumnReaders {
    /// A new read of a panel: its number, which stops the readers of the
    /// read before.
    pub(super) fn next(&self, side: Side) -> u64 {
        self.counts[side.index()].fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The count a reader compares its read's number with.
    pub(super) fn current(&self, side: Side) -> Arc<AtomicU64> {
        Arc::clone(&self.counts[side.index()])
    }
}

/// One read: what its readers share.
pub(super) struct ColumnJob {
    /// The panel.
    pub side: Side,
    /// The panel's entries the items belong to.
    pub listing: u64,
    /// This read's number.
    pub generation: u64,
    /// The panel's count: while it equals `generation`, the read runs.
    pub current: Arc<AtomicU64>,
    /// The folder.
    pub folder: Place,
    /// The files, in the order to read them.
    pub items: Vec<ColumnItem>,
    /// The next item to take.
    pub next: AtomicUsize,
    /// How many first items are on screen.
    pub urgent: usize,
    /// Each key's cost class.
    pub costs: Vec<(String, Cost)>,
    /// The pipeline.
    pub pipeline: Arc<Pipeline>,
    /// Where the files are read from.
    pub registry: Arc<Registry>,
    /// The app's number for this read.
    pub ask: u64,
}

impl ColumnJob {
    fn running(&self) -> bool {
        self.current.load(Ordering::SeqCst) == self.generation
    }
}

/// Runs a read: as many readers as the folder's disk takes, to the end or
/// until a newer read stops them.
pub(super) fn run(shared: &Arc<Shared>, job: &ColumnJob) {
    if !job.running() {
        return;
    }
    let readers = readers_for(&job.registry, &job.folder).min(job.items.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..readers {
            scope.spawn(|| read(shared, job, readers));
        }
    });
}

/// One reader: takes the next file until none is left or the read is
/// stopped.
fn read(shared: &Arc<Shared>, job: &ColumnJob, readers: usize) {
    let ctx = OpContext::new();
    let mut batch = Vec::new();
    let mut since = Instant::now();
    let mut limit = job.urgent.div_ceil(readers).max(1);
    loop {
        if !job.running() {
            return;
        }
        let i = job.next.fetch_add(1, Ordering::SeqCst);
        let Some(item) = job.items.get(i) else {
            break;
        };
        batch.push(cells_of(job, item, &ctx));
        if batch.len() >= limit || since.elapsed() >= BATCH_WAIT {
            if !job.running() {
                return;
            }
            send(shared, job, std::mem::take(&mut batch));
            since = Instant::now();
            limit = BATCH;
        }
    }
    if !batch.is_empty() && job.running() {
        send(shared, job, batch);
    }
}

fn send(shared: &Arc<Shared>, job: &ColumnJob, cells: Vec<ColumnCell>) {
    shared.send(AppEvent::ColumnsRead {
        side: job.side,
        listing: job.listing,
        cells,
        plugin: false,
        ask: job.ask,
    });
}

/// One file's cells: the pipeline's readers of each cost class its keys
/// need, then each key's words. A reader the guard was too busy to run is
/// asked again once it has room; one still refused after
/// [`BUSY_TRIES`] reads says so in words.
fn cells_of(job: &ColumnJob, item: &ColumnItem, ctx: &OpContext) -> ColumnCell {
    let values = match job.folder.join(&item.name) {
        Ok(place) => read_cells(job, item, &place, ctx),
        Err(_) => item
            .keys
            .iter()
            .map(|key| (key.clone(), String::new()))
            .collect(),
    };
    ColumnCell {
        index: item.index,
        name: item.name.clone(),
        values,
    }
}

fn read_cells(
    job: &ColumnJob,
    item: &ColumnItem,
    place: &Place,
    ctx: &OpContext,
) -> Vec<(String, String)> {
    let provider = match job.registry.resolve(place) {
        Ok(provider) => provider,
        // Every cell says why, rather than "reading" for good.
        Err(error) => {
            let words = Cell::Unavailable(error.to_string()).words();
            return item
                .keys
                .iter()
                .map(|key| (key.clone(), words.clone()))
                .collect();
        }
    };
    let entry = item.entry();
    let request = ReadRequest {
        place,
        entry: &entry,
        type_id: item.type_id.as_deref(),
        group: item.group.as_deref(),
        cancel: Some(&ctx.cancel),
        tools: Some(&ctx.tools),
    };
    let mut costs: Vec<Cost> = item
        .keys
        .iter()
        .filter_map(|key| job.costs.iter().find(|(k, _)| k == key).map(|(_, c)| *c))
        .collect();
    costs.sort();
    costs.dedup();
    let limits = ReadLimits::default();
    let read_all = || {
        let mut outcomes: Vec<Outcome> = Vec::new();
        for cost in &costs {
            outcomes.extend(
                job.pipeline
                    .read(provider.as_ref(), &request, *cost, &limits, ctx),
            );
        }
        outcomes
    };
    let mut outcomes = read_all();
    // A reader the guard refused as busy is asked again once the guard
    // has room, as meta's guard expects of its callers: no later read
    // comes for a file whose cells came back unread, so its row would say
    // "reading" for good (CI run 43373763659). A slot frees within a
    // read's deadline, since a reader past it stops at its next read.
    let mut tries = 1;
    while refused(&outcomes)
        && tries < BUSY_TRIES
        && job.running()
        && job
            .pipeline
            .guard()
            .wait_for_room(limits.deadline.saturating_mul(2))
    {
        outcomes = read_all();
        tries += 1;
    }
    // Still refused: the cells say so in words.
    let gave_up = refused(&outcomes) && job.running();
    // The extended items (the origin row, the streams) are read beside the
    // readers, as the details read them.
    if item
        .keys
        .iter()
        .any(|key| EXTENDED_KEYS.contains(&key.as_str()))
    {
        outcomes.push((
            EXTENDED,
            diroxide_meta::origin::extended_fields(provider.as_ref(), place, ctx),
        ));
    }
    item.keys
        .iter()
        .filter_map(|key| {
            // A photo column's cell in the photo helpers' words (lane AP).
            let words = match crate::photos::PhotoField::of(key) {
                Some(field) => crate::photos::cell(field, &job.pipeline, &outcomes),
                None => cell_words(job.pipeline.cell(key, &outcomes)),
            };
            let words = match words {
                None if gave_up => Some(busy_words()),
                words => words,
            };
            words.map(|w| (key.clone(), w))
        })
        .collect()
}

/// How many times a file is read while the guard refuses a reader as
/// busy: the first read, and each after the guard had room.
const BUSY_TRIES: usize = 3;

/// Whether the guard refused a reader as busy.
fn refused(outcomes: &[Outcome]) -> bool {
    outcomes
        .iter()
        .any(|(_, result)| matches!(result, Err(MetaError::Busy)))
}

/// The words of a cell whose reader the guard refused at every try:
/// "unavailable: busy with an earlier file", the state first.
fn busy_words() -> String {
    Cell::Unavailable(MetaError::Busy.to_string()).words()
}

/// A cell's words as an entry keeps them: the value; empty for a file
/// that has none; the reason for one that could not be read; `None` for a
/// cell still to be read (the guard was busy), so it is asked again.
pub(crate) fn cell_words(cell: Cell) -> Option<String> {
    match cell {
        Cell::Value(value) => Some(value),
        Cell::NotAvailable => Some(String::new()),
        Cell::Reading | Cell::NotComputed => None,
        other => Some(other.words()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use diroxide_core::{EntryKind, Place};
    use diroxide_meta::Cell;
    use diroxide_ops::{FakeProvider, Latency};

    use super::*;
    use crate::request::{Request, SharedPipeline};
    use crate::runtime::Runtime;

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    fn item(index: usize, name: &str) -> ColumnItem {
        ColumnItem {
            index: index as u32,
            name: name.to_owned(),
            kind: EntryKind::File,
            size: None,
            modified: None,
            type_id: None,
            group: None,
            keys: vec!["text.lines".into()],
        }
    }

    #[test]
    fn a_cells_words_keep_what_the_row_says() {
        assert_eq!(cell_words(Cell::Value("12".into())).as_deref(), Some("12"));
        assert_eq!(cell_words(Cell::NotAvailable).as_deref(), Some(""));
        assert_eq!(cell_words(Cell::Reading), None);
        assert_eq!(
            cell_words(Cell::Unreadable("the file ends too soon".into())).as_deref(),
            Some("unreadable: the file ends too soon")
        );
    }

    /// Lane MA, commit 52: the readers read each file's cells through the
    /// pipeline, and a newer read of the panel sends the readers of the
    /// one before away.
    #[test]
    fn the_readers_send_each_files_cells_and_a_newer_read_stops_them() {
        let fake = FakeProvider::new();
        let folder = p("/work");
        fake.add_folder(&folder);
        let mut items = Vec::new();
        for i in 0..200 {
            let name = format!("f{i:03}.txt");
            fake.add_file(&folder.join(&name).unwrap(), b"one\ntwo\nthree\n");
            items.push(item(i, &name));
        }
        fake.add_file(&folder.join("data.bin").unwrap(), b"\0\x01");
        items.push(item(200, "data.bin"));
        fake.switches_mut().latency = Latency {
            per_call: Duration::from_millis(5),
            ..Latency::default()
        };
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        let mut runtime = Runtime::new(Arc::new(registry));
        let pipeline = SharedPipeline(Arc::new(Pipeline::builtin()));
        let ask = |listing: u64, items: Vec<ColumnItem>| Request::ReadColumns {
            side: Side::Left,
            listing,
            folder: folder.clone(),
            items,
            urgent: 4,
            costs: vec![("text.lines".into(), Cost::OnDemand)],
            pipeline: pipeline.clone(),
            ask: listing,
        };
        runtime.request(ask(1, items.clone()));
        runtime.request(ask(2, items[198..].to_vec()));
        let (mut first, mut second) = (0, Vec::new());
        loop {
            let events = runtime.settle();
            if events.is_empty() {
                break;
            }
            for event in events {
                if let AppEvent::ColumnsRead { listing, cells, .. } = event {
                    if listing == 1 {
                        first += cells.len();
                    } else {
                        second.extend(cells);
                    }
                }
            }
        }
        assert!(first < 201, "the first read took {first} of 201");
        assert_eq!(second.len(), 3, "{second:?}");
        let of = |name: &str| {
            second
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.values.clone())
                .unwrap()
        };
        assert_eq!(of("f199.txt"), [("text.lines".to_owned(), "3".to_owned())]);
        // No reader takes it: its cell is empty, and the row leaves it out.
        assert_eq!(of("data.bin"), [("text.lines".to_owned(), String::new())]);
    }

    /// One file read through a pipeline whose guard has a single slot,
    /// held by another reader until `release` is called: the cells the
    /// read sends.
    fn read_with_the_slot_held(release_first: bool) -> Vec<(String, String)> {
        let fake = FakeProvider::new();
        let folder = p("/work");
        fake.add_folder(&folder);
        fake.add_file(&folder.join("f.txt").unwrap(), b"one\ntwo\nthree\n");
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        let mut runtime = Runtime::new(Arc::new(registry));
        let pipeline = Arc::new(Pipeline::builtin().with_guard(diroxide_meta::Guard::new(1)));
        // Another reader holds the only slot.
        let (started, has_started) = std::sync::mpsc::channel::<()>();
        let (hold, wait) = std::sync::mpsc::channel::<()>();
        let guard = pipeline.guard().clone();
        let holder = std::thread::spawn(move || {
            guard.run(Duration::from_secs(60), move || {
                let _ = started.send(());
                let _ = wait.recv_timeout(Duration::from_secs(60));
                Ok(())
            })
        });
        has_started.recv().unwrap();
        runtime.request(Request::ReadColumns {
            side: Side::Left,
            listing: 1,
            folder: folder.clone(),
            items: vec![item(0, "f.txt")],
            urgent: 1,
            costs: vec![("text.lines".into(), Cost::OnDemand)],
            pipeline: SharedPipeline(pipeline),
            ask: 1,
        });
        if release_first {
            hold.send(()).unwrap();
        }
        let mut cells = Vec::new();
        loop {
            let events = runtime.settle();
            if events.is_empty() {
                break;
            }
            for event in events {
                if let AppEvent::ColumnsRead { cells: got, .. } = event {
                    cells.extend(got);
                }
            }
        }
        let _ = hold.send(());
        assert_eq!(holder.join().unwrap(), Ok(()));
        assert_eq!(cells.len(), 1, "{cells:?}");
        cells.remove(0).values
    }

    /// Lane PC (CI run 43373763659): a file whose reader the guard
    /// refused as busy is read again when the guard has room, so its cell
    /// fills; it is never sent back unread, which left the row saying
    /// "reading" for good, since no later read comes for it.
    #[test]
    fn a_reader_refused_as_busy_is_asked_again_when_there_is_room() {
        assert_eq!(
            read_with_the_slot_held(true),
            [("text.lines".to_owned(), "3".to_owned())]
        );
    }

    /// Lane PC: a slot held past every try leaves the cell in words, the
    /// state first ("lines unavailable", 17 cells, in the row), never
    /// "reading".
    #[test]
    fn a_reader_refused_at_every_try_says_so() {
        let values = read_with_the_slot_held(false);
        assert_eq!(
            values,
            [(
                "text.lines".to_owned(),
                "unavailable: busy with an earlier file".to_owned()
            )]
        );
        assert_eq!(busy_words(), "unavailable: busy with an earlier file");
        assert!(diroxide_core::format::within_cells(
            &format!("lines {}", busy_words()),
            "lines unavailable"
        ));
    }
}
