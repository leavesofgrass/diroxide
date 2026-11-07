//! The pipeline: the readers in order, and the fields they give for one
//! file in one cost class, each reader run under the guard
//! ([`crate::guard`]): a thread, a catch, a deadline and a byte limit.
//! Every field that comes back is cleaned like a file name
//! ([`Field::cleaned`]).
//!
//! What a caller does with it (the details view, Say Details, the custom
//! columns, `diro info`):
//!
//! 1. [`Pipeline::builtin`] once, with the preview crate's readers added
//!    by [`Pipeline::with`].
//! 2. [`Pipeline::read`] per file and cost class: the fast class with the
//!    row, the background class after the list appears, the on-demand
//!    class when asked.
//! 3. [`Pipeline::fields`] for the details, in the groups' order, or
//!    [`Pipeline::cell`] for a column's cell, which is never blank.

use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{CancelToken, OpContext, Provider, ReadSeek, ToolRunner};

use crate::field::{Cell, Field};
use crate::guard::{Bounded, Guard};
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};

/// One reader's answer for one file: its id, and its fields or why it
/// gave none.
pub type Outcome = (&'static str, Result<Vec<Field>, MetaError>);

/// The time a guard waits past a read's own deadline, so the source's
/// own deadline trips first and its words come back.
const GRACE: Duration = Duration::from_millis(200);

/// The readers, in the order they are asked.
#[derive(Clone, Default)]
pub struct Pipeline {
    readers: Vec<Arc<dyn Reader>>,
    guard: Guard,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.readers.iter().map(|r| r.id()).collect();
        f.debug_struct("Pipeline")
            .field("readers", &ids)
            .field("guard", &self.guard)
            .finish()
    }
}

/// What one reader is asked, owned, so it can travel to the reader's
/// thread: the request's facts, the read's cancel token (the request's,
/// else the one the read runs under) and the runner, when given.
struct Asked {
    place: Place,
    entry: Entry,
    type_id: Option<String>,
    group: Option<String>,
    cancel: CancelToken,
    tools: Option<Arc<dyn ToolRunner>>,
}

impl Asked {
    fn of(request: &ReadRequest<'_>, read: &CancelToken) -> Self {
        Self {
            place: request.place.clone(),
            entry: request.entry.clone(),
            type_id: request.type_id.map(str::to_owned),
            group: request.group.map(str::to_owned),
            cancel: request.cancel.unwrap_or(read).clone(),
            tools: request.tools.cloned(),
        }
    }

    fn request(&self) -> ReadRequest<'_> {
        ReadRequest {
            place: &self.place,
            entry: &self.entry,
            type_id: self.type_id.as_deref(),
            group: self.group.as_deref(),
            cancel: Some(&self.cancel),
            tools: self.tools.as_ref(),
        }
    }
}

impl Pipeline {
    /// A pipeline with no readers and the default guard.
    pub fn new() -> Self {
        Self::default()
    }

    /// The readers this crate builds, in order: the fast ones first. The
    /// preview crate adds its image and ffprobe readers with
    /// [`Pipeline::with`].
    pub fn builtin() -> Self {
        crate::readers::builtin()
            .into_iter()
            .fold(Self::new(), Self::with)
    }

    /// The same pipeline with a reader added last.
    pub fn with(mut self, reader: Arc<dyn Reader>) -> Self {
        self.readers.push(reader);
        self
    }

    /// The same pipeline with another guard (another pool size; a clone
    /// shares its count).
    pub fn with_guard(mut self, guard: Guard) -> Self {
        self.guard = guard;
        self
    }

    /// Its guard.
    pub fn guard(&self) -> &Guard {
        &self.guard
    }

    /// The readers' ids, in order.
    pub fn ids(&self) -> Vec<&'static str> {
        self.readers.iter().map(|r| r.id()).collect()
    }

    /// Every key a reader names, with its cost class and the reader's id,
    /// in the readers' order, then the extended items' keys
    /// ([`EXTENDED_KEYS`], background, id [`EXTENDED`]): what the column
    /// chooser lists.
    pub fn keys(&self) -> Vec<(&'static str, Cost, &'static str)> {
        self.readers
            .iter()
            .flat_map(|r| r.keys().iter().map(move |k| (*k, r.cost(), r.id())))
            .chain(
                EXTENDED_KEYS
                    .iter()
                    .map(|k| (*k, Cost::Background, EXTENDED)),
            )
            .collect()
    }

    /// The cost class of the first reader that names `key`.
    pub fn cost_of(&self, key: &str) -> Option<Cost> {
        self.readers
            .iter()
            .find(|r| r.keys().contains(&key))
            .map(|r| r.cost())
            .or_else(|| EXTENDED_KEYS.contains(&key).then_some(Cost::Background))
    }

    /// Every field of one file for the details view and Say Details: the
    /// readers of each class in `classes`, then the extended items and
    /// the origin row (an outcome with the id [`EXTENDED`]). The on-demand
    /// class belongs in `classes` only when the person asked.
    pub fn details(
        &self,
        provider: &dyn Provider,
        request: &ReadRequest<'_>,
        classes: &[Cost],
        limits: &ReadLimits,
        ctx: &OpContext,
    ) -> Vec<Outcome> {
        let mut outcomes = Vec::new();
        for class in classes {
            outcomes.extend(self.read(provider, request, *class, limits, ctx));
        }
        outcomes.push((
            EXTENDED,
            crate::origin::extended_fields(provider, request.place, ctx),
        ));
        outcomes
    }

    /// The fields of one file from every reader of `cost` that accepts
    /// it, each run under the guard on a source opened afresh through
    /// `provider` and held to `limits`; a reader that fails gives its
    /// error in place of fields, so one bad reader never hides the rest.
    /// A cancel stops between readers and inside a read.
    pub fn read(
        &self,
        provider: &dyn Provider,
        request: &ReadRequest<'_>,
        cost: Cost,
        limits: &ReadLimits,
        ctx: &OpContext,
    ) -> Vec<Outcome> {
        let mut outcomes = Vec::new();
        for reader in self
            .readers
            .iter()
            .filter(|r| r.cost() == cost && r.accepts(request))
        {
            if let Err(error) = ctx.check() {
                outcomes.push((reader.id(), Err(MetaError::Place(error))));
                break;
            }
            let fields = provider
                .open_seekable(request.place, ctx)
                .map_err(MetaError::from)
                .and_then(|source| {
                    self.run_guarded(
                        Arc::clone(reader),
                        source,
                        Asked::of(request, &ctx.cancel),
                        *limits,
                        ctx.cancel.clone(),
                    )
                });
            outcomes.push((reader.id(), fields));
        }
        outcomes
    }

    /// One reader, by its id, over a source the caller opened: what a
    /// plugin's bytes or a test's bytes go through. The reader runs under
    /// the guard whether or not it accepts the request.
    pub fn read_source(
        &self,
        id: &str,
        source: Box<dyn ReadSeek>,
        request: &ReadRequest<'_>,
        limits: &ReadLimits,
        cancel: &CancelToken,
    ) -> Option<Result<Vec<Field>, MetaError>> {
        let reader = self.readers.iter().find(|r| r.id() == id)?;
        Some(self.run_guarded(
            Arc::clone(reader),
            source,
            Asked::of(request, cancel),
            *limits,
            cancel.clone(),
        ))
    }

    fn run_guarded(
        &self,
        reader: Arc<dyn Reader>,
        source: Box<dyn ReadSeek>,
        asked: Asked,
        limits: ReadLimits,
        cancel: CancelToken,
    ) -> Result<Vec<Field>, MetaError> {
        let deadline = limits.deadline.saturating_add(GRACE);
        self.guard.run(deadline, move || {
            let mut bounded = Bounded::new(source, &limits, cancel);
            let trip = bounded.trip();
            reader
                .read(&mut bounded, &asked.request(), &limits)
                .map(|fields| fields.into_iter().map(Field::cleaned).collect())
                .map_err(|error| trip.explain(error))
        })
    }

    /// Every field of some outcomes, in the groups' order (each group
    /// keeping the readers' order), the first of any key kept.
    pub fn fields(outcomes: &[Outcome]) -> Vec<Field> {
        let mut seen = std::collections::HashSet::new();
        let mut fields: Vec<Field> = outcomes
            .iter()
            .filter_map(|(_, result)| result.as_ref().ok())
            .flatten()
            .filter(|field| seen.insert(field.key.clone()))
            .cloned()
            .collect();
        fields.sort_by_key(|field| field.group);
        fields
    }

    /// A column's cell for `key` from some outcomes: the value when a
    /// reader gave it; "unreadable: REASON" when the reader that names
    /// the key failed; otherwise "not available". Never blank.
    pub fn cell(&self, key: &str, outcomes: &[Outcome]) -> Cell {
        for (_, result) in outcomes {
            if let Ok(fields) = result
                && let Some(field) = fields.iter().find(|f| f.key == key)
            {
                return Cell::Value(field.value.clone());
            }
        }
        for (id, result) in outcomes {
            let names_key = if *id == EXTENDED {
                EXTENDED_KEYS.contains(&key)
            } else {
                self.readers
                    .iter()
                    .any(|r| r.id() == *id && r.keys().contains(&key))
            };
            if let Err(error) = result
                && names_key
            {
                return Cell::from_error(error);
            }
        }
        Cell::NotAvailable
    }
}

/// The id of the extended items' outcome in [`Pipeline::details`].
pub const EXTENDED: &str = "extended";

/// The keys of the extended items ([`crate::origin::extended_fields`]):
/// the streams or attributes, the origin row, and the tags and comment
/// other programs keep in the file system.
pub const EXTENDED_KEYS: [&str; 4] = ["file.extended", "file.origin", "file.tags", "file.comment"];

#[cfg(test)]
mod tests {

    use std::time::Duration;

    use diroxide_core::details::DetailGroup;
    use diroxide_core::{Entry, EntryKind, Place};
    use diroxide_vfs::{LocalProvider, ReadSeek};

    use super::*;

    /// A reader that counts the bytes of a text file.
    struct Bytes;

    impl Reader for Bytes {
        fn id(&self) -> &'static str {
            "bytes"
        }
        fn cost(&self) -> Cost {
            Cost::Background
        }
        fn keys(&self) -> &'static [&'static str] {
            &["text.bytes"]
        }
        fn accepts(&self, request: &ReadRequest<'_>) -> bool {
            request.entry.name.ends_with(".txt")
        }
        fn read(
            &self,
            source: &mut dyn ReadSeek,
            _request: &ReadRequest<'_>,
            _limits: &ReadLimits,
        ) -> Result<Vec<Field>, MetaError> {
            let mut bytes = Vec::new();
            source
                .read_to_end(&mut bytes)
                .map_err(|e| MetaError::from_io(&e))?;
            Ok(vec![Field::new(
                "text.bytes",
                "Bytes\n",
                format!("{}\u{1b}", bytes.len()),
                DetailGroup::Size,
            )])
        }
    }

    /// A reader that panics, one that hangs and one that reads forever.
    struct Hostile(&'static str);

    impl Reader for Hostile {
        fn id(&self) -> &'static str {
            self.0
        }
        fn cost(&self) -> Cost {
            Cost::Background
        }
        fn keys(&self) -> &'static [&'static str] {
            &["hostile.value"]
        }
        fn accepts(&self, _request: &ReadRequest<'_>) -> bool {
            true
        }
        fn read(
            &self,
            source: &mut dyn ReadSeek,
            _request: &ReadRequest<'_>,
            _limits: &ReadLimits,
        ) -> Result<Vec<Field>, MetaError> {
            match self.0 {
                "panics" => panic!("a hostile file"),
                "hangs" => {
                    // Reads one byte at a time, slowly, past its deadline.
                    let mut one = [0u8; 1];
                    loop {
                        std::thread::sleep(Duration::from_millis(5));
                        if source.read(&mut one).is_err() {
                            return Err(MetaError::unreadable("stopped"));
                        }
                        let _ = std::io::Seek::seek(source, std::io::SeekFrom::Start(0));
                    }
                }
                _ => {
                    let mut all = Vec::new();
                    source
                        .read_to_end(&mut all)
                        .map_err(|e| MetaError::from_io(&e))?;
                    Ok(Vec::new())
                }
            }
        }
    }

    /// A reader that says what its request carried: a runner, and whether
    /// its cancel token is set.
    struct Sees;

    impl Reader for Sees {
        fn id(&self) -> &'static str {
            "sees"
        }
        fn cost(&self) -> Cost {
            Cost::OnDemand
        }
        fn accepts(&self, _request: &ReadRequest<'_>) -> bool {
            true
        }
        fn read(
            &self,
            _source: &mut dyn ReadSeek,
            request: &ReadRequest<'_>,
            _limits: &ReadLimits,
        ) -> Result<Vec<Field>, MetaError> {
            let tools = request.tools.is_some_and(|t| t.has("ffprobe"));
            let cancel = request.cancel.map(CancelToken::is_cancelled);
            Ok(vec![Field::new(
                "sees.request",
                "Request",
                format!("tools {tools}, cancel {cancel:?}"),
                DetailGroup::Technical,
            )])
        }
    }

    /// A runner that has every tool and runs none.
    struct Every;

    impl ToolRunner for Every {
        fn has(&self, _tool: &str) -> bool {
            true
        }
        fn run(
            &self,
            _request: &diroxide_vfs::ToolRequest,
            _cancel: &CancelToken,
        ) -> Result<diroxide_vfs::ToolOutput, diroxide_vfs::ToolRunError> {
            Err(diroxide_vfs::ToolRunError::Cancelled)
        }
        fn run_streaming(
            &self,
            _request: &diroxide_vfs::ToolRequest,
            _cancel: &CancelToken,
            _record: &mut dyn FnMut(diroxide_vfs::ToolStream, &str) -> std::ops::ControlFlow<()>,
        ) -> Result<diroxide_vfs::ToolOutput, diroxide_vfs::ToolRunError> {
            Err(diroxide_vfs::ToolRunError::Cancelled)
        }
    }

    /// The reader on its thread gets the request's runner, and a cancel
    /// token always: the request's, else the read's own (MA's request 1).
    #[test]
    fn a_reader_gets_the_requests_runner_and_a_cancel_token() {
        let pipeline = Pipeline::new().with(Arc::new(Sees));
        let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let entry = Entry::new("clip.mkv", EntryKind::File);
        let runner: Arc<dyn ToolRunner> = Arc::new(Every);
        let asked = CancelToken::new();
        let read = CancelToken::new();
        read.cancel();
        let said = |request: &ReadRequest<'_>| -> String {
            let source: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(Vec::new()));
            let fields = pipeline
                .read_source("sees", source, request, &ReadLimits::default(), &read)
                .unwrap()
                .unwrap();
            fields[0].value.clone()
        };
        let mut request = ReadRequest::new(&place, &entry);
        // Nothing given: no runner, the read's own token (cancelled here).
        assert_eq!(said(&request), "tools false, cancel Some(true)");
        request.tools = Some(&runner);
        request.cancel = Some(&asked);
        assert_eq!(said(&request), "tools true, cancel Some(false)");
    }

    /// The scratch folder sits under the build's target folder, never the
    /// system's temporary folder.
    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap()
    }

    #[test]
    fn the_pipeline_asks_the_readers_of_one_class_that_take_the_file() {
        let dir = scratch();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"twelve bytes").unwrap();
        let place = Place::parse(&path.display().to_string()).unwrap();
        let entry = Entry::new("notes.txt", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: Some("text"),
            group: Some("text"),
            cancel: None,
            tools: None,
        };
        let pipeline = Pipeline::new().with(Arc::new(Bytes));
        assert_eq!(pipeline.ids(), ["bytes"]);
        assert_eq!(pipeline.cost_of("text.bytes"), Some(Cost::Background));
        let keys = pipeline.keys();
        assert_eq!(keys[0], ("text.bytes", Cost::Background, "bytes"));
        assert_eq!(keys[2], ("file.origin", Cost::Background, EXTENDED));
        assert_eq!(pipeline.cost_of("file.origin"), Some(Cost::Background));
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let limits = ReadLimits::default();
        let got = pipeline.read(&provider, &request, Cost::Background, &limits, &ctx);
        assert_eq!(got.len(), 1);
        // Cleaned: the escape is a question mark, the label one line.
        let field = &got[0].1.as_ref().unwrap()[0];
        assert_eq!(field.line(), "Bytes: 12?");
        assert_eq!(
            pipeline.cell("text.bytes", &got),
            Cell::Value("12?".to_owned())
        );
        assert_eq!(pipeline.cell("audio.artist", &got), Cell::NotAvailable);
        assert!(
            pipeline
                .read(&provider, &request, Cost::Fast, &limits, &ctx)
                .is_empty()
        );
        // A byte limit below the file's size.
        let small = ReadLimits {
            max_bytes: 4,
            deadline: Duration::from_secs(2),
        };
        let got = pipeline.read(&provider, &request, Cost::Background, &small, &ctx);
        assert_eq!(got[0].1, Err(MetaError::TooLarge));
        assert_eq!(
            pipeline.cell("text.bytes", &got).words(),
            "unreadable: it is too large to read here"
        );
        // A cancelled context reads nothing.
        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        let got = pipeline.read(&provider, &request, Cost::Background, &limits, &cancelled);
        assert!(got[0].1.is_err());
        // The details: the classes asked for, then the extended items.
        let got = pipeline.details(
            &provider,
            &request,
            &[Cost::Fast, Cost::Background],
            &limits,
            &ctx,
        );
        let ids: Vec<&str> = got.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["bytes", EXTENDED]);
        assert!(got[1].1.is_ok());
        let failed: Vec<Outcome> = vec![(EXTENDED, Err(MetaError::TooSlow))];
        assert_eq!(
            pipeline.cell("file.origin", &failed).words(),
            "unreadable: it took too long to read"
        );
    }

    #[test]
    fn a_hostile_reader_fails_one_file_and_never_the_program() {
        let pipeline = Pipeline::new()
            .with(Arc::new(Hostile("panics")))
            .with(Arc::new(Hostile("hangs")))
            .with(Arc::new(Hostile("reads")))
            .with(Arc::new(Bytes));
        let place = Place::parse(if cfg!(windows) { "C:\\a.txt" } else { "/a.txt" }).unwrap();
        let entry = Entry::new("a.txt", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: None,
            group: None,
            cancel: None,
            tools: None,
        };
        let limits = ReadLimits {
            max_bytes: 1024,
            deadline: Duration::from_millis(100),
        };
        let cancel = CancelToken::new();
        let source = || -> Box<dyn ReadSeek> { Box::new(std::io::Cursor::new(vec![b'x'; 4096])) };
        let panics = pipeline.read_source("panics", source(), &request, &limits, &cancel);
        assert_eq!(panics, Some(Err(MetaError::ReaderFailed)));
        let hangs = pipeline.read_source("hangs", source(), &request, &limits, &cancel);
        assert_eq!(hangs, Some(Err(MetaError::TooSlow)));
        let reads = pipeline.read_source("reads", source(), &request, &limits, &cancel);
        assert_eq!(reads, Some(Err(MetaError::TooLarge)));
        assert!(
            pipeline
                .read_source("nobody", source(), &request, &limits, &cancel)
                .is_none()
        );
        let outcomes: Vec<Outcome> = vec![
            ("panics", Err(MetaError::ReaderFailed)),
            (
                "bytes",
                Ok(vec![Field::new(
                    "text.bytes",
                    "Bytes",
                    "3",
                    DetailGroup::Size,
                )]),
            ),
        ];
        assert_eq!(
            pipeline.cell("hostile.value", &outcomes).words(),
            "unreadable: the reader failed"
        );
        assert_eq!(Pipeline::fields(&outcomes).len(), 1);
    }

    #[test]
    fn fields_come_in_the_groups_order_once_each() {
        let outcomes: Vec<Outcome> = vec![
            (
                "one",
                Ok(vec![
                    Field::new("doc.author", "Author", "Ada Example", DetailGroup::Source),
                    Field::new("doc.title", "Title", "Notes", DetailGroup::Content),
                ]),
            ),
            ("two", Err(MetaError::TooSlow)),
            (
                "three",
                Ok(vec![Field::new(
                    "doc.title",
                    "Title",
                    "Other",
                    DetailGroup::Content,
                )]),
            ),
        ];
        let fields = Pipeline::fields(&outcomes);
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["doc.title", "doc.author"]);
        assert_eq!(fields[0].value, "Notes");
    }
}
