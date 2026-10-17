//! The provider for places inside archives: it opens a container through
//! the registry, decides its format by its bytes, builds its index once
//! (cached by the container's place, size and time), and serves listings
//! and member reads in place, through the limiter chain. Nothing is
//! extracted to browse.
//!
//! The `Provider` methods give plain errors; [`ArchiveProvider::open_archive`],
//! [`ArchiveProvider::list_folder`] and [`ArchiveProvider::read_member`]
//! keep a limit's question so the caller can ask it and call again with
//! the answer.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Weak};

use diroxide_core::format::{count, human_size};
use diroxide_core::{Entry, EntryKind, Place, SecretKind, SecretRequest};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, LinkRequest, OpContext, Provider, ReadSeek,
    Registry, TrashCheck, VfsError, WriteMode,
};

use diroxide_ops::part_names::{Discovery, SetProblem, find_set};
use diroxide_ops::{PartInfo, PartsError, PartsReader};

use crate::error::ArchiveError;
use crate::format::ArchiveFormat;
use crate::guard::{Allowances, Budget, Claims, Guarded, check_cost, check_member, refuse};
use crate::index::{
    ContainerKey, Index, IndexCache, Locator, MAX_LISTED, ROOT, RecordFlags, RecordKind,
};
use crate::limits::Limits;
use crate::names::nesting_depth;
use crate::read_7z::{self, SevenZBackend};
use crate::read_tar::{self, SingleBackend, TarBackend, Wrapper};
use crate::read_xz;
use crate::read_zip::{self, ZipBackend};
use crate::sniff::{SNIFF_LEN, Sniffed, decide};
use crate::source::Source;
use crate::spool::{self, SpoolBudget};

/// How many indexes the provider keeps (research/archives.md 5.2).
pub const CACHED_INDEXES: usize = 8;

/// An archive opened: its index and its format's reader.
#[derive(Debug)]
pub struct Opened {
    key: ContainerKey,
    container: Place,
    name: String,
    sniffed: Sniffed,
    index: Index,
    backend: Backend,
    parts: Option<usize>,
}

#[derive(Debug)]
enum Backend {
    Zip(ZipBackend),
    Tar(TarBackend),
    Single(SingleBackend),
    SevenZ(SevenZBackend),
}

impl Opened {
    /// The archive's index.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The format it was opened as.
    pub fn format(&self) -> ArchiveFormat {
        self.sniffed.format
    }

    /// What the name and the bytes said.
    pub fn sniffed(&self) -> Sniffed {
        self.sniffed
    }

    /// The container's place.
    pub fn container(&self) -> &Place {
        &self.container
    }

    /// The container's file name, as the words say it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What identifies the container in the cache.
    pub fn key(&self) -> &ContainerKey {
        &self.key
    }

    /// The status line on opening: "Opened course.zip: 10 items, 48 KB."
    pub fn opened_line(&self) -> String {
        let summary = self.index.summary();
        let items = count(summary.members + summary.folders, "item", "items");
        match summary.total_size {
            Some(total) => format!("Opened {}: {items}, {}.", self.name, human_size(total)),
            None => format!("Opened {}: {items}, size unknown.", self.name),
        }
    }

    /// Whether members can be read with random access: a zip, a 7z that
    /// is not solid, an uncompressed tar.
    pub fn random_read(&self) -> bool {
        match &self.backend {
            Backend::Zip(_) => true,
            Backend::Tar(tar) => tar.random_read(),
            Backend::Single(_) => false,
            Backend::SevenZ(sevenz) => sevenz.random_read(),
        }
    }

    /// How many parts the archive was opened from, for a split archive
    /// (`backup.7z.001` and its siblings); `None` for one file.
    pub fn parts(&self) -> Option<usize> {
        self.parts
    }

    /// How many times a compressed tar's decoder started again from the
    /// archive's start to read members, after the listing: a whole
    /// unpack in archive order is one. Zero for the other formats.
    pub fn passes(&self) -> u64 {
        match &self.backend {
            Backend::Tar(tar) => tar.passes(),
            _ => 0,
        }
    }

    /// The record numbers in the order their data lies in the archive: a
    /// tar's by their offset (a duplicate's last copy lies later than its
    /// record's place in the index), every other format's as listed.
    pub fn archive_order(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = (0..self.index.len() as u32).collect();
        if let Backend::Tar(_) = &self.backend {
            ids.sort_by_key(|&id| match self.index.record(id).map(|r| r.locator) {
                Some(Locator::Tar(offset)) => offset,
                _ => 0,
            });
        }
        ids
    }

    /// Reads one record's data through the limiter chain: its claims
    /// checked first (a member over the size limit asks), the job's budget
    /// shared when given, a password asked of the context when the member
    /// is locked.
    pub(crate) fn read_record(
        &self,
        id: u32,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
        limits: &Limits,
        cost_ask_bytes: u64,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let record = *self.index.record(id).ok_or(VfsError::NotAFolder)?;
        match record.kind {
            RecordKind::Folder => return Err(VfsError::NotAFolder.into()),
            RecordKind::Special => {
                return Err(VfsError::UnsupportedFeature {
                    what: "a special entry, which holds no data".to_owned(),
                }
                .into());
            }
            RecordKind::File | RecordKind::Link => {}
        }
        let name = self.index.name(id).to_owned();
        check_member(&name, record.size, limits, allow)?;
        let password = if record.flags.contains(RecordFlags::ENCRYPTED) {
            match &self.backend {
                Backend::SevenZ(sevenz) if sevenz.password().is_some() => {
                    sevenz.password().cloned()
                }
                _ => {
                    let request =
                        SecretRequest::new(SecretKind::ArchivePassword, self.name.clone());
                    Some(ctx.ask_secret(&request).ok_or(VfsError::PasswordRequired)?)
                }
            }
        } else {
            None
        };
        let inner: Box<dyn Read + Send> = match &self.backend {
            Backend::Zip(zip) => {
                Box::new(zip.read(&self.index, id, password, ctx.cancel.clone())?)
            }
            Backend::Tar(tar) => tar.read(&self.index, id, &ctx.cancel, allow, cost_ask_bytes)?,
            Backend::Single(single) => single.read(),
            Backend::SevenZ(sevenz) => {
                let Locator::SevenZ(file) = record.locator else {
                    return Err(VfsError::NotFound.into());
                };
                let file = file as usize;
                if !sevenz.random_read() {
                    check_cost(&name, sevenz.ahead(file), cost_ask_bytes, allow)?;
                }
                Box::new(sevenz.read(file, password, ctx.cancel.clone())?)
            }
        };
        let size_known = !record.flags.contains(RecordFlags::SIZE_UNKNOWN);
        let claims = Claims {
            name,
            size: size_known.then_some(record.size),
            packed: record.packed(),
            crc32: record
                .flags
                .contains(RecordFlags::HAS_CRC)
                .then_some(record.crc32),
        };
        let budget = budget.unwrap_or_else(|| Budget::new(limits));
        Ok(Box::new(Guarded::new(
            inner,
            claims,
            *limits,
            budget,
            allow,
            ctx.cancel.clone(),
        )))
    }

    /// Visits every record in archive order; see [`ArchiveProvider::visit`].
    pub(crate) fn visit_records(
        &self,
        ctx: &OpContext,
        allow: Allowances,
        budget: Budget,
        limits: &Limits,
        wanted: &dyn Fn(u32) -> bool,
        each: &mut Visitor<'_>,
    ) -> Result<(), ArchiveError> {
        let index = &self.index;
        let claims_of = |id: u32| {
            let record = index.record(id).copied();
            Claims {
                name: index.name(id).to_owned(),
                size: record
                    .filter(|r| !r.flags.contains(RecordFlags::SIZE_UNKNOWN))
                    .map(|r| r.size),
                packed: record.and_then(|r| r.packed()),
                crc32: record
                    .filter(|r| r.flags.contains(RecordFlags::HAS_CRC))
                    .map(|r| r.crc32),
            }
        };
        if let Backend::SevenZ(sevenz) = &self.backend {
            let mut record_of = vec![u32::MAX; index.len()];
            for id in 0..index.len() as u32 {
                if let Some(Locator::SevenZ(file)) = index.record(id).map(|r| r.locator) {
                    let file = file as usize;
                    if file >= record_of.len() {
                        record_of.resize(file + 1, u32::MAX);
                    }
                    record_of[file] = id;
                }
            }
            let encrypted = (0..index.len() as u32).any(|id| {
                wanted(id)
                    && index
                        .record(id)
                        .is_some_and(|r| r.flags.contains(RecordFlags::ENCRYPTED))
            });
            let password = match (sevenz.password(), encrypted) {
                (Some(password), _) => Some(password.clone()),
                (None, true) => {
                    let request =
                        SecretRequest::new(SecretKind::ArchivePassword, self.name.clone());
                    Some(ctx.ask_secret(&request).ok_or(VfsError::PasswordRequired)?)
                }
                (None, false) => None,
            };
            let mut visit_one = |file: usize, reader: &mut dyn Read| -> Result<bool, VfsError> {
                match record_of.get(file).copied() {
                    Some(id) if id != u32::MAX && wanted(id) => {
                        let mut guarded = Guarded::new(
                            reader,
                            claims_of(id),
                            *limits,
                            budget.clone(),
                            allow,
                            ctx.cancel.clone(),
                        );
                        each(index, id, &mut guarded)
                    }
                    _ => Ok(true),
                }
            };
            let wanted_file = |file: usize| {
                record_of
                    .get(file)
                    .is_some_and(|&id| id != u32::MAX && wanted(id))
            };
            sevenz.visit(password.as_ref(), &ctx.cancel, &wanted_file, &mut visit_one)?;
            return Ok(());
        }
        let in_order = Allowances {
            cost: true,
            ..allow
        };
        for id in self.archive_order() {
            ctx.check()?;
            if !wanted(id) {
                continue;
            }
            let Some(record) = index.record(id).copied() else {
                continue;
            };
            let go_on = if record.kind == RecordKind::File && record.locator != Locator::None {
                let mut reader =
                    self.read_record(id, ctx, in_order, Some(budget.clone()), limits, u64::MAX)?;
                each(index, id, &mut reader)?
            } else {
                each(index, id, &mut std::io::empty())?
            };
            if !go_on {
                break;
            }
        }
        Ok(())
    }
}

/// Builds an archive's index from its bytes: the format by the name and
/// the first bytes, then the format's reader.
#[allow(clippy::too_many_arguments)]
fn open_source(
    source: Source,
    key: ContainerKey,
    container: Place,
    name: String,
    parts: Option<usize>,
    limits: &Limits,
    ctx: &OpContext,
    allow: Allowances,
) -> Result<Opened, ArchiveError> {
    let mut head = vec![0u8; SNIFF_LEN];
    let got = source.read_at(0, &mut head).map_err(VfsError::from)?;
    head.truncate(got);
    let sniffed = decide(&name, &head)?;
    let (index, backend) = match sniffed.format {
        ArchiveFormat::Zip => {
            let (index, zip) = read_zip::open(source, &name, limits, &ctx.cancel)?;
            (index, Backend::Zip(zip))
        }
        ArchiveFormat::Tar => {
            let index = read_tar::list(&source, Wrapper::Plain, &name, limits, ctx, allow)?;
            (index, Backend::Tar(TarBackend::new(source, Wrapper::Plain)))
        }
        ArchiveFormat::TarGz if read_tar::holds_tar(&source, Wrapper::Gzip) => {
            let index = read_tar::list(&source, Wrapper::Gzip, &name, limits, ctx, allow)?;
            (index, Backend::Tar(TarBackend::new(source, Wrapper::Gzip)))
        }
        ArchiveFormat::TarGz => {
            let index = read_tar::single_index(&name, None, limits)?;
            (
                index,
                Backend::Single(SingleBackend::new(source, Wrapper::Gzip)),
            )
        }
        ArchiveFormat::SevenZ => {
            let ask = || {
                ctx.ask_secret(&SecretRequest::new(
                    SecretKind::ArchivePassword,
                    name.clone(),
                ))
            };
            let (index, sevenz) = read_7z::open(source, &name, limits, allow, &ask)?;
            (index, Backend::SevenZ(sevenz))
        }
        ArchiveFormat::TarXz | ArchiveFormat::Xz => {
            read_xz::check(&source, &name, limits, allow)?;
            let wrapper = Wrapper::Xz {
                mem_limit_kb: read_xz::mem_limit_kb(limits),
            };
            if sniffed.format == ArchiveFormat::TarXz || read_tar::holds_tar(&source, wrapper) {
                let index = read_tar::list(&source, wrapper, &name, limits, ctx, allow)?;
                (index, Backend::Tar(TarBackend::new(source, wrapper)))
            } else {
                let size = read_xz::uncompressed_size(&source, limits);
                let index = read_tar::single_index(&name, size, limits)?;
                (index, Backend::Single(SingleBackend::new(source, wrapper)))
            }
        }
    };
    Ok(Opened {
        key,
        container,
        name,
        sniffed,
        index,
        backend,
        parts,
    })
}

/// A split archive found from one of its parts: the provider of its
/// folder and the parts, in order.
struct SplitSet {
    provider: Arc<dyn Provider>,
    parts: Vec<SetPart>,
}

/// One part of a split archive, as its folder lists it.
struct SetPart {
    place: Place,
    size: u64,
    modified: Option<(i64, u32)>,
}

impl SplitSet {
    fn total(&self) -> u64 {
        self.parts.iter().map(|part| part.size).sum()
    }

    fn newest(&self) -> Option<(i64, u32)> {
        self.parts.iter().filter_map(|part| part.modified).max()
    }
}

/// A folder's listing, with whether it was cut at [`MAX_LISTED`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// The entries, in archive order.
    pub entries: Vec<Entry>,
    /// The children the folder holds in all.
    pub total: usize,
}

impl Listing {
    /// Whether the listing stopped at the cap, so the panel says
    /// [`crate::index::capped_line`].
    pub fn capped(&self) -> bool {
        self.total > self.entries.len()
    }
}

/// The provider for places inside archives.
#[derive(Debug)]
pub struct ArchiveProvider {
    limits: Limits,
    registry: RwLock<Weak<Registry>>,
    cache: IndexCache<Opened>,
    cost_ask_bytes: u64,
    spool_dir: RwLock<Option<PathBuf>>,
    spool_budget: SpoolBudget,
}

impl Default for ArchiveProvider {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl ArchiveProvider {
    /// A provider with these limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            registry: RwLock::new(Weak::new()),
            cache: IndexCache::new(CACHED_INDEXES, limits.max_index_bytes),
            cost_ask_bytes: read_tar::COST_ASK_BYTES,
            spool_dir: RwLock::new(None),
            spool_budget: SpoolBudget::default(),
        }
    }

    /// The same provider asking before a member of a sequential archive is
    /// read alone with more than `bytes` to read first (256 MiB by
    /// default).
    pub fn with_cost_question(mut self, bytes: u64) -> Self {
        self.cost_ask_bytes = bytes;
        self
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Registers a new provider with these limits as the archive provider
    /// of `registry`, and connects it: the registry the app asks for any
    /// place, archives included.
    pub fn install(
        mut registry: Registry,
        limits: Limits,
    ) -> (Arc<Registry>, Arc<ArchiveProvider>) {
        let provider = Arc::new(ArchiveProvider::new(limits));
        registry.set_archive(provider.clone());
        let registry = Arc::new(registry);
        provider.connect(&registry);
        (registry, provider)
    }

    /// The title line for a place inside an archive, with the format named
    /// when the bytes and the name disagree ("a.zip, 7z archive, in
    /// D:\docs"); from the cache, never opening anything.
    pub fn title(&self, place: &Place) -> String {
        let opened = place
            .archive()
            .and_then(|(container, _)| self.cache.peek(&container.to_string()));
        if let Some((_, "")) = place.archive()
            && let Some(parts) = opened.as_ref().and_then(|opened| opened.parts)
        {
            return crate::words::set_title(place, parts);
        }
        let sniffed = opened.map(|opened| opened.sniffed);
        crate::words::title(place, sniffed.as_ref())
    }

    /// The parts of a split archive, when `container` is a numbered part
    /// of one (`backup.7z.001`): found by name and number among its
    /// folder's entries, in numeric order. A missing or empty part means
    /// the set cannot be read whole, and says which.
    fn split_set(
        &self,
        container: &Place,
        ctx: &OpContext,
    ) -> Result<Option<SplitSet>, ArchiveError> {
        if container.archive().is_some() {
            return Ok(None);
        }
        let Some(name) = container.name() else {
            return Ok(None);
        };
        if diroxide_core::names::split_part(name).is_none()
            || ArchiveFormat::from_name(name).is_none()
        {
            return Ok(None);
        }
        let registry = self.registry()?;
        let provider = registry.resolve(container)?;
        let folder = container.parent().ok_or(VfsError::NotFound)?;
        let entries = provider.list(&folder, ctx)?;
        if !entries.iter().any(|entry| entry.name == name) {
            return Err(VfsError::NotFound.into());
        }
        let case_sensitive = provider.capabilities(&folder).case_sensitive;
        let set = match find_set(&folder, &entries, name, case_sensitive) {
            Ok(Discovery::Found(set)) => set,
            Ok(Discovery::Ask { no, .. }) => *no,
            Err(_) => {
                return Err(VfsError::UnsupportedFeature {
                    what: "a set of parts that is not split bytes".to_owned(),
                }
                .into());
            }
        };
        for problem in set.problems(None) {
            let (SetProblem::Missing(number) | SetProblem::Empty(number)) = problem else {
                continue;
            };
            let label = set.label(number).to_lowercase();
            let what = if matches!(problem, SetProblem::Missing(_)) {
                format!("{label} is missing")
            } else {
                format!("{label} is empty")
            };
            return Err(VfsError::Corrupt { what }.into());
        }
        let parts = set
            .parts
            .iter()
            .map(|part| SetPart {
                place: part.place.clone(),
                size: part.size,
                modified: part.modified.map(|t| (t.unix_seconds(), t.subsec_nanos())),
            })
            .collect();
        Ok(Some(SplitSet { provider, parts }))
    }

    /// Connects the provider to the registry it reaches archive files
    /// through, after the registry is built.
    pub fn connect(&self, registry: &Arc<Registry>) {
        if let Ok(mut held) = self.registry.write() {
            *held = Arc::downgrade(registry);
        }
    }

    fn registry(&self) -> Result<Arc<Registry>, VfsError> {
        self.registry
            .read()
            .ok()
            .and_then(|held| held.upgrade())
            .ok_or_else(|| VfsError::NoProvider("archive file".to_owned()))
    }

    /// Forgets the cached index of an archive and of everything nested in
    /// it, so the next open reads it again.
    pub fn forget(&self, container: &Place) {
        self.cache.forget(&container.to_string());
    }

    /// The container's bytes and its identity for the cache.
    /// What identifies a container, without reading it: a local one by its
    /// stat, a nested one by its record and its outer archive's identity.
    fn key_of(
        &self,
        container: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<ContainerKey, ArchiveError> {
        if let Some((outer, member)) = container.archive() {
            let opened = self.open_archive(outer, ctx, allow)?;
            let id = opened.index.find(member).ok_or(VfsError::NotFound)?;
            let record = opened.index.record(id).ok_or(VfsError::NotFound)?;
            if record.kind == RecordKind::Folder {
                return Err(VfsError::NotAFolder.into());
            }
            return Ok(ContainerKey {
                place: container.to_string(),
                size: record.size,
                modified: opened.key.modified,
            });
        }
        if let Some(set) = self.split_set(container, ctx)? {
            // A split archive is the same archive while every part keeps
            // its size and the newest keeps its time.
            return Ok(ContainerKey {
                place: container.to_string(),
                size: set.total(),
                modified: set.newest(),
            });
        }
        let registry = self.registry()?;
        let provider = registry.resolve(container)?;
        let entry = provider.stat(container, ctx)?;
        if entry.kind == EntryKind::Folder {
            return Err(VfsError::NotAFolder.into());
        }
        Ok(ContainerKey {
            place: container.to_string(),
            size: entry.size.unwrap_or(0),
            modified: entry.modified.map(|t| (t.unix_seconds(), t.subsec_nanos())),
        })
    }

    /// A container's bytes, and the parts it was read from: a local file
    /// opened for random reads; a split archive's parts as one seekable
    /// read, nothing joined; a stored zip member as a window of its outer
    /// archive; any other member read out into the spool.
    fn source_of(
        &self,
        container: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<(Source, Option<usize>), ArchiveError> {
        if let Some(set) = self.split_set(container, ctx)? {
            let count = set.parts.len();
            let parts = set
                .parts
                .into_iter()
                .map(|part| PartInfo::new(part.place, part.size))
                .collect();
            // The reader keeps its own context: a later read must not see
            // the opening job's cancel.
            let reader = PartsReader::with_parts(set.provider, parts, &OpContext::new())
                .map_err(PartsError::into_vfs)?;
            let source = Source::new(Box::new(reader)).map_err(VfsError::from)?;
            return Ok((source, Some(count)));
        }
        if let Some((outer, member)) = container.archive() {
            let opened = self.open_archive(outer, ctx, allow)?;
            let id = opened.index.find(member).ok_or(VfsError::NotFound)?;
            if let Backend::Zip(zip) = &opened.backend
                && let Some(window) = zip.stored_window(&opened.index, id)
            {
                return Ok((window, None));
            }
            let reader = self.read_member(container, ctx, allow, None)?;
            let dir = self.spool_dir.read().ok().and_then(|d| d.clone());
            let free = dir.as_deref().and_then(|dir| self.free_space(dir, ctx));
            let source = spool::spool(
                reader,
                &self.limits,
                dir.as_deref(),
                &self.spool_budget,
                free,
                &ctx.cancel,
            )?;
            return Ok((source, None));
        }
        let registry = self.registry()?;
        let provider = registry.resolve(container)?;
        let reader = provider.open_seekable(container, ctx)?;
        Ok((Source::new(reader).map_err(VfsError::from)?, None))
    }

    /// The bytes free on the spool folder's drive, when the registry can
    /// say.
    fn free_space(&self, dir: &std::path::Path, ctx: &OpContext) -> Option<u64> {
        let place = Place::parse(&dir.to_string_lossy()).ok()?;
        let registry = self.registry().ok()?;
        let provider = registry.resolve(&place).ok()?;
        provider.space(&place, ctx).ok()?.free_to_user
    }

    /// The same provider spooling nested archives too large for memory into
    /// `dir`: the state folder's `spool`, never the platform's temporary
    /// folder. Without one, such an archive is refused in words.
    pub fn with_spool_dir(self, dir: PathBuf) -> Self {
        self.set_spool_dir(Some(dir));
        self
    }

    /// Sets or clears the spool folder.
    pub fn set_spool_dir(&self, dir: Option<PathBuf>) {
        if let Ok(mut held) = self.spool_dir.write() {
            *held = dir;
        }
    }

    /// The spool bytes on disk now, for every nested archive open.
    pub fn spool_in_use(&self) -> u64 {
        self.spool_budget.in_use()
    }

    /// Opens an archive: its index, from the cache when the container is
    /// unchanged. A place nested more than the limit allows is refused.
    pub fn open_archive(
        &self,
        container: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<Arc<Opened>, ArchiveError> {
        let depth = nesting_depth(container) + 1;
        if depth > self.limits.max_nesting {
            return Err(refuse(
                "nesting",
                format!("{} archives deep", self.limits.max_nesting),
            )
            .into());
        }
        let key = self.key_of(container, ctx, allow)?;
        if let Some(opened) = self.cache.get(&key) {
            return Ok(opened);
        }
        let (source, parts) = self.source_of(container, ctx, allow)?;
        let name = container.name().unwrap_or("archive").to_owned();
        let opened = Arc::new(open_source(
            source,
            key.clone(),
            container.clone(),
            name,
            parts,
            &self.limits,
            ctx,
            allow,
        )?);
        let bytes = opened.index.memory();
        self.cache.put(key, opened.clone(), bytes);
        Ok(opened)
    }

    /// The archive and the record of a place inside it.
    fn locate(
        &self,
        place: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<(Arc<Opened>, u32), ArchiveError> {
        let Some((container, member)) = place.archive() else {
            return Err(VfsError::NoProvider("archive".to_owned()).into());
        };
        let opened = self.open_archive(container, ctx, allow)?;
        let id = opened.index.find(member).ok_or(VfsError::NotFound)?;
        Ok((opened, id))
    }

    /// Lists a folder inside an archive: its children only, in archive
    /// order, at most [`MAX_LISTED`] of them.
    pub fn list_folder(
        &self,
        place: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<Listing, ArchiveError> {
        let (opened, id) = self.locate(place, ctx, allow)?;
        if id != ROOT
            && opened
                .index
                .record(id)
                .is_some_and(|r| r.kind != RecordKind::Folder)
        {
            return Err(VfsError::NotAFolder.into());
        }
        let children = opened.index.children(id);
        let mut entries = Vec::with_capacity(children.len().min(MAX_LISTED));
        for (n, &child) in children.iter().take(MAX_LISTED).enumerate() {
            if n.is_multiple_of(4_096) {
                ctx.check()?;
            }
            if let Some(entry) = opened.index.entry(child) {
                entries.push(entry);
            }
        }
        Ok(Listing {
            entries,
            total: children.len(),
        })
    }

    /// Visits every member of an archive in archive order, in one pass
    /// where the format is sequential: a solid 7z block by block, a
    /// compressed tar from its start to its end (a duplicate's last copy
    /// where it lies), a zip in its own order. `each` gets the index, the
    /// record and a reader of the member's data through the limiter chain
    /// (an empty one for a folder, a link or a special entry), and returns
    /// whether to go on. Members are read by record, never by place, so a
    /// name a place cannot hold (one with `.zip!` in it) is visited too.
    /// What the unpack runner and Test archive build on.
    pub fn visit(
        &self,
        container: &Place,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
        each: &mut Visitor<'_>,
    ) -> Result<(), ArchiveError> {
        let opened = self.open_archive(container, ctx, allow)?;
        self.visit_opened(&opened, ctx, allow, budget, each)
    }

    /// [`ArchiveProvider::visit`] over an archive already opened, with
    /// this provider's limits: one opened from bytes with
    /// [`ArchiveProvider::open_bytes`] too.
    pub fn visit_opened(
        &self,
        opened: &Opened,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
        each: &mut Visitor<'_>,
    ) -> Result<(), ArchiveError> {
        let budget = budget.unwrap_or_else(|| Budget::new(&self.limits));
        opened.visit_records(ctx, allow, budget, &self.limits, &|_| true, each)
    }

    /// [`ArchiveProvider::visit_opened`] over the records `wanted` says
    /// yes to: the others are never opened (a zip's, a plain tar's) or are
    /// decoded and set aside without a call (a solid block's, a compressed
    /// tar's). What a runner that has done some members already uses to
    /// go on with the rest.
    pub fn visit_some(
        &self,
        opened: &Opened,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
        wanted: &dyn Fn(u32) -> bool,
        each: &mut Visitor<'_>,
    ) -> Result<(), ArchiveError> {
        let budget = budget.unwrap_or_else(|| Budget::new(&self.limits));
        opened.visit_records(ctx, allow, budget, &self.limits, wanted, each)
    }

    /// Opens an archive from any seekable bytes, under this provider's
    /// limits and never cached: a new archive read back before it takes
    /// its place (the test after packing). `place` is the place the bytes
    /// are for; its name decides the format with the first bytes, as a
    /// file's name does.
    pub fn open_bytes(
        &self,
        bytes: Box<dyn ReadSeek>,
        place: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<Opened, ArchiveError> {
        let source = Source::new(bytes).map_err(VfsError::from)?;
        let key = ContainerKey {
            place: place.to_string(),
            size: source.len(),
            modified: None,
        };
        let name = place.name().unwrap_or("archive").to_owned();
        open_source(
            source,
            key,
            place.clone(),
            name,
            None,
            &self.limits,
            ctx,
            allow,
        )
    }

    /// Reads a member through the limiter chain: its claims checked first
    /// (a member over the size limit asks), the job's budget shared when
    /// given, a password asked of the context when the member is locked.
    pub fn read_member(
        &self,
        place: &Place,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let (opened, id) = self.locate(place, ctx, allow)?;
        opened.read_record(id, ctx, allow, budget, &self.limits, self.cost_ask_bytes)
    }

    /// Reads one record of an archive already opened through the limiter
    /// chain, as [`ArchiveProvider::read_member`] reads a place.
    pub fn read_record(
        &self,
        opened: &Opened,
        id: u32,
        ctx: &OpContext,
        allow: Allowances,
        budget: Option<Budget>,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        opened.read_record(id, ctx, allow, budget, &self.limits, self.cost_ask_bytes)
    }
}

impl Provider for ArchiveProvider {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        let Some((container, _)) = place.archive() else {
            return Capabilities::read_only();
        };
        match self.cache.peek(&container.to_string()) {
            Some(opened) => capabilities_for(opened.format(), !opened.random_read()),
            None => match container.name().and_then(ArchiveFormat::from_name) {
                Some(format) => capabilities_for(format, false),
                None => capabilities_for(ArchiveFormat::Zip, false),
            },
        }
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        Ok(self.list_folder(place, ctx, Allowances::default())?.entries)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        let (opened, id) = self.locate(place, ctx, Allowances::default())?;
        if id == ROOT {
            let name = opened.name.clone();
            let mut entry = Entry::new(name, EntryKind::Folder);
            entry.size = opened.index.summary().total_size;
            return Ok(entry);
        }
        opened.index.entry(id).ok_or(VfsError::NotFound)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Ok(self.read_member(place, ctx, Allowances::default(), None)?)
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        let (opened, id) = self.locate(place, ctx, Allowances::default())?;
        let window = match &opened.backend {
            Backend::Zip(zip) => zip.stored_window(&opened.index, id),
            Backend::Tar(tar) => tar.window(&opened.index, id),
            Backend::Single(_) | Backend::SevenZ(_) => None,
        };
        match window {
            Some(window) => Ok(Box::new(window)),
            None => Err(VfsError::unsupported(
                "archive",
                "read a compressed member with random access",
            )),
        }
    }

    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(crate::words::read_only_error())
    }

    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(crate::words::read_only_error())
    }

    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(crate::words::read_only_error())
    }

    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(crate::words::read_only_error())
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(crate::words::read_only_error())
    }

    fn create_link(
        &self,
        _place: &Place,
        _link: &LinkRequest,
        _ctx: &OpContext,
    ) -> Result<(), VfsError> {
        Err(crate::words::read_only_error())
    }

    fn trash(&self, _place: &Place, _ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        Err(crate::words::read_only_error())
    }
}

/// What [`ArchiveProvider::visit`] calls for each member: the index, the
/// record, a reader of its data; it returns whether to go on.
pub type Visitor<'a> = dyn FnMut(&Index, u32, &mut dyn Read) -> Result<bool, VfsError> + 'a;

/// What a place inside an archive of this format can do: nothing but
/// reading in this alpha; names are case-sensitive; random reads where the
/// format allows them (a zip, a 7z that is not solid, an uncompressed
/// tar), so the app can word the cost of reading a sequential one.
pub fn capabilities_for(format: ArchiveFormat, solid: bool) -> Capabilities {
    let mut capabilities = Capabilities::read_only();
    capabilities.case_sensitive = true;
    capabilities.random_read = match format {
        ArchiveFormat::Zip | ArchiveFormat::Tar => true,
        ArchiveFormat::SevenZ => !solid,
        _ => false,
    };
    capabilities
}
