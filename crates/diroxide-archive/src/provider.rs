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
use std::sync::{Arc, RwLock, Weak};

use diroxide_core::format::{count, human_size};
use diroxide_core::{Entry, EntryKind, Place, SecretKind, SecretRequest};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

use crate::error::ArchiveError;
use crate::format::ArchiveFormat;
use crate::guard::{Allowances, Budget, Claims, Guarded, check_member, refuse};
use crate::index::{ContainerKey, Index, IndexCache, MAX_LISTED, ROOT, RecordFlags, RecordKind};
use crate::limits::Limits;
use crate::names::nesting_depth;
use crate::read_zip::{self, ZipBackend};
use crate::sniff::{SNIFF_LEN, Sniffed, decide};
use crate::source::Source;

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
}

#[derive(Debug)]
enum Backend {
    Zip(ZipBackend),
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
        }
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
        }
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
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
    fn source_of(
        &self,
        container: &Place,
        ctx: &OpContext,
    ) -> Result<(Source, ContainerKey), ArchiveError> {
        if container.archive().is_some() {
            return Err(
                VfsError::unsupported("archive", "open an archive inside another yet").into(),
            );
        }
        let registry = self.registry()?;
        let provider = registry.resolve(container)?;
        let entry = provider.stat(container, ctx)?;
        if entry.kind == EntryKind::Folder {
            return Err(VfsError::NotAFolder.into());
        }
        let key = ContainerKey {
            place: container.to_string(),
            size: entry.size.unwrap_or(0),
            modified: entry.modified.map(|t| (t.unix_seconds(), t.subsec_nanos())),
        };
        let reader = provider.open_seekable(container, ctx)?;
        let source = Source::new(reader).map_err(VfsError::from)?;
        Ok((source, key))
    }

    /// Opens an archive: its index, from the cache when the container is
    /// unchanged. A place nested more than the limit allows is refused.
    pub fn open_archive(
        &self,
        container: &Place,
        ctx: &OpContext,
        allow: Allowances,
    ) -> Result<Arc<Opened>, ArchiveError> {
        let _ = allow;
        let depth = nesting_depth(container) + 1;
        if depth > self.limits.max_nesting {
            return Err(refuse(
                "nesting",
                format!("{} archives deep", self.limits.max_nesting),
            )
            .into());
        }
        let (source, key) = self.source_of(container, ctx)?;
        if let Some(opened) = self.cache.get(&key) {
            return Ok(opened);
        }
        let name = container.name().unwrap_or("archive").to_owned();
        let mut head = vec![0u8; SNIFF_LEN];
        let got = source.read_at(0, &mut head).map_err(VfsError::from)?;
        head.truncate(got);
        let sniffed = decide(&name, &head)?;
        let (index, backend) = match sniffed.format {
            ArchiveFormat::Zip => {
                let (index, zip) = read_zip::open(source, &name, &self.limits, &ctx.cancel)?;
                (index, Backend::Zip(zip))
            }
            other => {
                return Err(VfsError::UnsupportedFeature {
                    what: format!(
                        "{} reading, which arrives in a later step",
                        other.spoken_name()
                    ),
                }
                .into());
            }
        };
        let opened = Arc::new(Opened {
            key: key.clone(),
            container: container.clone(),
            name,
            sniffed,
            index,
            backend,
        });
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
        let record = *opened.index.record(id).ok_or(VfsError::NotAFolder)?;
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
        let name = opened.index.name(id).to_owned();
        check_member(&name, record.size, &self.limits, allow)?;
        let password = if record.flags.contains(RecordFlags::ENCRYPTED) {
            let request = SecretRequest::new(SecretKind::ArchivePassword, opened.name.clone());
            Some(ctx.ask_secret(&request).ok_or(VfsError::PasswordRequired)?)
        } else {
            None
        };
        let inner: Box<dyn Read + Send> = match &opened.backend {
            Backend::Zip(zip) => {
                Box::new(zip.read(&opened.index, id, password, ctx.cancel.clone())?)
            }
        };
        let claims = Claims {
            name,
            size: Some(record.size),
            packed: record.packed(),
            crc32: record
                .flags
                .contains(RecordFlags::HAS_CRC)
                .then_some(record.crc32),
        };
        let budget = budget.unwrap_or_else(|| Budget::new(&self.limits));
        Ok(Box::new(Guarded::new(
            inner,
            claims,
            self.limits,
            budget,
            allow,
            ctx.cancel.clone(),
        )))
    }

    fn not_built(&self, operation: &str) -> VfsError {
        VfsError::unsupported(self.scheme(), &format!("{operation} yet"))
    }
}

impl Provider for ArchiveProvider {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        let mut capabilities = Capabilities::read_only();
        capabilities.case_sensitive = true;
        capabilities
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

    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(self.not_built("write into archives"))
    }

    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("make folders in archives"))
    }

    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("rename in archives"))
    }

    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(self.not_built("delete from archives"))
    }

    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(self.not_built("change attributes in archives"))
    }
}
