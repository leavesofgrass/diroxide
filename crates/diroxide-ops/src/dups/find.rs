//! The search: list, group by size, quick check, full hash, and the byte
//! comparison on request.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{EntryKind, Place, Timestamp};
use diroxide_vfs::{OpContext, Provider, VfsError};

use super::hash::{ContentHasher, STEP, fill};
use super::{
    DupEnv, DupError, DupEstimate, DupFile, DupGroup, DupOptions, DupProgress, DupResult, DupSink,
    DupStage, LeftOut,
};
use crate::engine::JobControl;
use crate::pace::Pacer;
use crate::walk::walk;

/// The largest quick-check block read, whatever the options say.
const MOST_BLOCK: u64 = 16 * 1024 * 1024;

/// How long a paused search sleeps between looks at its control.
const PAUSE_STEP: Duration = Duration::from_millis(100);

/// Finds the duplicate files under `roots`, every root through its own
/// provider from `env.registry`, so two folders on two places are
/// compared as one search.
///
/// The steps: the walk (links never followed, other drives and loops
/// left out and said); files grouped by size; empty files, files under
/// the minimum and extra names of a hard-linked file counted and left
/// out; a quick check of the first and last [`DupOptions::partial_block`]
/// bytes (a file no longer than two blocks is hashed whole here); the
/// question of [`DupSink::confirm`] when the full hash would read more
/// than [`DupOptions::ask_above`]; the full hash ([`DupEnv::hasher`]);
/// the byte comparison when [`DupOptions::byte_compare`] asks. A file is
/// called a duplicate only on its full hash or its bytes.
///
/// Progress goes to the sink: each step at once, the rest at the
/// options' interval. Cancel stops it between reads with
/// [`DupError::Cancelled`]; Pause holds it, said once.
pub fn find_duplicates(
    roots: &[Place],
    options: &DupOptions,
    control: &JobControl,
    env: &DupEnv<'_>,
) -> Result<DupResult, DupError> {
    if roots.is_empty() {
        return Err(DupError::NoRoots);
    }
    let mut search = Search::new(options, control, env);
    search.list(roots)?;
    search.bucket_by_size();
    search.split_hard_links()?;
    search.quick_check()?;
    search.ask_first()?;
    search.full_hash()?;
    if options.byte_compare {
        search.compare_bytes()?;
    }
    Ok(search.result())
}

/// One file the walk found.
#[derive(Clone, Debug)]
struct Candidate {
    place: Place,
    provider: usize,
    size: u64,
    modified: Option<Timestamp>,
    found: usize,
    /// The provider's scheme, the volume and the inode, when known.
    identity: Option<(String, u64, u64)>,
    hard_links: Option<u64>,
    /// The quick check's hash, or the whole file's when it was short.
    quick: Option<Vec<u8>>,
    /// The full hash.
    full: Option<Vec<u8>>,
}

/// Says progress at the interval and holds while paused.
pub(super) struct Ticker<'a> {
    sink: &'a dyn DupSink,
    pacer: &'a dyn Pacer,
    interval: Duration,
    last: Duration,
    /// The progress said next.
    pub progress: DupProgress,
}

impl<'a> Ticker<'a> {
    pub(super) fn new(env: &DupEnv<'a>, interval: Duration) -> Self {
        Self {
            sink: env.sink,
            pacer: env.pacer,
            interval,
            last: Duration::ZERO,
            progress: DupProgress::start(DupStage::Listing, 0, 0),
        }
    }

    /// Starts a step and says it at once.
    pub(super) fn begin(&mut self, stage: DupStage, files_total: u64, bytes_total: u64) {
        self.progress = DupProgress::start(stage, files_total, bytes_total);
        self.say();
    }

    /// Says the progress when the interval has passed.
    pub(super) fn tick(&mut self) {
        if self.pacer.now().saturating_sub(self.last) >= self.interval {
            self.say();
        }
    }

    fn say(&mut self) {
        self.last = self.pacer.now();
        self.sink.progress(&self.progress);
    }

    /// Waits while the control is paused, said once on the way in and
    /// once on the way out; returns at once when cancelled.
    pub(super) fn hold(&mut self, control: &JobControl) {
        if !control.is_paused() || control.cancel.is_cancelled() {
            return;
        }
        self.progress.paused = true;
        self.say();
        while control.is_paused() && !control.cancel.is_cancelled() {
            self.pacer.sleep(PAUSE_STEP);
        }
        self.progress.paused = false;
        self.say();
    }

    /// Holds while paused, then `Err(Cancelled)` if Cancel was pressed.
    pub(super) fn checkpoint(&mut self, control: &JobControl) -> Result<(), DupError> {
        self.hold(control);
        if control.cancel.is_cancelled() {
            return Err(DupError::Cancelled);
        }
        Ok(())
    }
}

/// One search's state.
struct Search<'a> {
    options: &'a DupOptions,
    control: &'a JobControl,
    env: DupEnv<'a>,
    ctx: OpContext,
    ticker: Ticker<'a>,
    providers: Vec<Arc<dyn Provider>>,
    files: Vec<Candidate>,
    /// Groups of distinct candidates that may still match, by index.
    buckets: Vec<Vec<usize>>,
    /// The empty files, when they take part.
    empty: Vec<usize>,
    /// The extra names of a hard-linked candidate, by its index.
    links: HashMap<usize, Vec<usize>>,
    left_out: Vec<LeftOut>,
    files_seen: u64,
    empty_left_out: u64,
    small_left_out: u64,
    hard_links_left_out: u64,
    unreadable: u64,
    compared: bool,
}

impl<'a> Search<'a> {
    fn new(options: &'a DupOptions, control: &'a JobControl, env: &DupEnv<'a>) -> Self {
        Self {
            options,
            control,
            env: *env,
            ctx: OpContext::new().with_cancel(control.cancel.clone()),
            ticker: Ticker::new(env, options.progress_interval),
            providers: Vec::new(),
            files: Vec::new(),
            buckets: Vec::new(),
            empty: Vec::new(),
            links: HashMap::new(),
            left_out: Vec::new(),
            files_seen: 0,
            empty_left_out: 0,
            small_left_out: 0,
            hard_links_left_out: 0,
            unreadable: 0,
            compared: false,
        }
    }

    fn leave_out(&mut self, place: Place, why: String) {
        self.left_out.push(LeftOut { place, why });
    }

    fn unreadable(&mut self, place: Place, error: &VfsError) -> Result<(), DupError> {
        if matches!(error, VfsError::Cancelled) {
            return Err(DupError::Cancelled);
        }
        self.unreadable += 1;
        self.leave_out(place, format!("could not be read: {error}"));
        Ok(())
    }

    /// Walks every root; files become candidates, each place once.
    fn list(&mut self, roots: &[Place]) -> Result<(), DupError> {
        self.ticker.begin(DupStage::Listing, 0, 0);
        let mut seen: HashSet<String> = HashSet::new();
        for root in roots {
            self.ticker.checkpoint(self.control)?;
            let provider = match self.env.registry.resolve(root) {
                Ok(provider) => provider,
                Err(error) => {
                    self.leave_out(root.clone(), format!("cannot be searched: {error}"));
                    continue;
                }
            };
            let root_entry = match provider.stat(root, &self.ctx) {
                Ok(entry) => entry,
                Err(error) => {
                    self.unreadable(root.clone(), &error)?;
                    continue;
                }
            };
            self.ticker.progress.current = Some(root.to_string());
            let base = self.ticker.progress.files_done;
            let ticker = &mut self.ticker;
            let control = self.control;
            let walked = walk(
                provider.as_ref(),
                root,
                root_entry,
                &self.ctx,
                self.options.max_depth,
                &mut |total| {
                    ticker.hold(control);
                    control.cancel.check()?;
                    ticker.progress.files_done = base + total;
                    ticker.tick();
                    Ok(())
                },
            );
            let walked = match walked {
                Ok(walked) => walked,
                Err(VfsError::Cancelled) => return Err(DupError::Cancelled),
                Err(error) => {
                    self.unreadable(root.clone(), &error)?;
                    continue;
                }
            };
            let index = self.providers.len();
            self.providers.push(Arc::clone(&provider));
            for (place, why) in walked.left_out {
                self.leave_out(place, why);
            }
            for found in walked.found {
                if found.entry.kind != EntryKind::File || !seen.insert(found.place.to_string()) {
                    continue;
                }
                self.files_seen += 1;
                let mut entry = found.entry;
                if entry.size.is_none() {
                    match provider.stat(&found.place, &self.ctx) {
                        Ok(fresh) => entry = fresh,
                        Err(error) => {
                            self.unreadable(found.place, &error)?;
                            continue;
                        }
                    }
                }
                let Some(size) = entry.size else {
                    self.leave_out(found.place, "its size is not known".to_owned());
                    continue;
                };
                let identity = identity(provider.as_ref(), entry.volume, entry.inode);
                let at = self.files.len();
                self.files.push(Candidate {
                    place: found.place,
                    provider: index,
                    size,
                    modified: entry.modified,
                    found: at,
                    identity,
                    hard_links: entry.hard_links,
                    quick: None,
                    full: None,
                });
            }
        }
        Ok(())
    }

    /// Groups by size; empty and small files counted and left out unless
    /// the options keep them.
    fn bucket_by_size(&mut self) {
        let mut by_size: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (at, file) in self.files.iter().enumerate() {
            if file.size == 0 {
                if self.options.include_empty {
                    self.empty.push(at);
                } else {
                    self.empty_left_out += 1;
                }
                continue;
            }
            if file.size < self.options.min_size {
                self.small_left_out += 1;
                continue;
            }
            by_size.entry(file.size).or_default().push(at);
        }
        self.buckets = by_size.into_values().filter(|b| b.len() > 1).collect();
    }

    /// Finds the names that are one file (hard links), reading the
    /// identity where the listing did not give it; the extra names are
    /// left out, or kept beside their first name.
    fn split_hard_links(&mut self) -> Result<(), DupError> {
        let mut buckets = std::mem::take(&mut self.buckets);
        if !self.empty.is_empty() {
            buckets.push(std::mem::take(&mut self.empty));
        }
        let mut kept = Vec::new();
        for bucket in buckets {
            let mut first_of: HashMap<(String, u64, u64), usize> = HashMap::new();
            let mut names: HashMap<usize, u64> = HashMap::new();
            let mut distinct = Vec::new();
            let mut extra: Vec<(usize, usize)> = Vec::new();
            for at in bucket {
                self.ticker.checkpoint(self.control)?;
                if self.files[at].identity.is_none() {
                    let provider = Arc::clone(&self.providers[self.files[at].provider]);
                    if let Ok(entry) = provider.stat(&self.files[at].place, &self.ctx) {
                        self.files[at].identity =
                            identity(provider.as_ref(), entry.volume, entry.inode);
                        self.files[at].hard_links = entry.hard_links;
                    }
                }
                match self.files[at].identity.clone() {
                    Some(key) => match first_of.get(&key) {
                        Some(&first) => {
                            *names.entry(first).or_insert(1) += 1;
                            extra.push((at, first));
                        }
                        None => {
                            first_of.insert(key, at);
                            distinct.push(at);
                        }
                    },
                    None => distinct.push(at),
                }
            }
            for (at, first) in extra {
                if self.options.include_hard_links {
                    self.links.entry(first).or_default().push(at);
                    continue;
                }
                let links = names
                    .get(&first)
                    .copied()
                    .unwrap_or(2)
                    .max(self.files[first].hard_links.unwrap_or(0));
                let name = self.files[first]
                    .place
                    .name()
                    .map(|n| diroxide_core::names::display_name(n).into_owned())
                    .unwrap_or_default();
                self.hard_links_left_out += 1;
                let place = self.files[at].place.clone();
                self.leave_out(place, format!("same file as {name}, {links} links"));
            }
            if distinct.len() > 1 {
                kept.push(distinct);
            }
        }
        // The empty files, if they take part, need no reading: they are
        // equal. They wait at the end until the result.
        if let Some(last) = kept.last()
            && last.first().is_some_and(|&at| self.files[at].size == 0)
        {
            self.empty = kept.pop().unwrap_or_default();
        }
        self.buckets = kept;
        Ok(())
    }

    /// The bytes and files of the buckets whose full hash is still to
    /// read.
    fn to_hash(&self) -> (u64, u64) {
        self.buckets
            .iter()
            .flatten()
            .filter(|&&at| self.files[at].full.is_none())
            .fold((0, 0), |(files, bytes), &at| {
                (files + 1, bytes + self.files[at].size)
            })
    }

    /// Reads the first and last blocks of every candidate and regroups.
    fn quick_check(&mut self) -> Result<(), DupError> {
        let total = self.buckets.iter().map(Vec::len).sum::<usize>() as u64;
        let block = self.options.partial_block.clamp(1, MOST_BLOCK);
        self.ticker.begin(DupStage::QuickCheck, total, 0);
        for at in self.buckets.iter().flatten().copied().collect::<Vec<_>>() {
            self.ticker.checkpoint(self.control)?;
            let file = &self.files[at];
            let provider = Arc::clone(&self.providers[file.provider]);
            let place = file.place.clone();
            let size = file.size;
            self.ticker.progress.current = place.name().map(str::to_owned);
            match quick(
                provider.as_ref(),
                &place,
                size,
                block,
                &self.ctx,
                self.env.hasher,
            ) {
                Ok((digest, whole)) => {
                    if whole {
                        self.files[at].full = Some(digest.clone());
                    }
                    self.files[at].quick = Some(digest);
                }
                Err(error) => self.unreadable(place, &error)?,
            }
            self.ticker.progress.files_done += 1;
            self.ticker.tick();
        }
        self.regroup(|file| file.quick.clone());
        Ok(())
    }

    /// Asks before a full hash larger than the options allow.
    fn ask_first(&mut self) -> Result<(), DupError> {
        let (files, bytes) = self.to_hash();
        if let Some(most) = self.options.ask_above
            && bytes > most
        {
            let estimate = DupEstimate { files, bytes };
            if !self.env.sink.confirm(&estimate) {
                return Err(DupError::Declined(estimate));
            }
        }
        Ok(())
    }

    /// Hashes every candidate left whole and regroups.
    fn full_hash(&mut self) -> Result<(), DupError> {
        let (files, bytes) = self.to_hash();
        if files == 0 {
            // The quick check read every file whole: nothing to say.
            return Ok(());
        }
        self.ticker.begin(DupStage::Hashing, files, bytes);
        for at in self.buckets.iter().flatten().copied().collect::<Vec<_>>() {
            if self.files[at].full.is_some() {
                continue;
            }
            self.ticker.checkpoint(self.control)?;
            let file = &self.files[at];
            let provider = Arc::clone(&self.providers[file.provider]);
            let place = file.place.clone();
            let size = file.size;
            let base = self.ticker.progress.bytes_done;
            self.ticker.progress.current = place.name().map(str::to_owned);
            let ticker = &mut self.ticker;
            let control = self.control;
            let result = full(
                provider.as_ref(),
                &place,
                size,
                &self.ctx,
                self.env.hasher,
                &mut |done| {
                    ticker.hold(control);
                    ticker.progress.bytes_done = base + done;
                    ticker.tick();
                },
            );
            match result {
                Ok(digest) => self.files[at].full = Some(digest),
                Err(error) => self.unreadable(place, &error)?,
            }
            self.ticker.progress.bytes_done = base + size;
            self.ticker.progress.files_done += 1;
            self.ticker.tick();
        }
        self.regroup(|file| file.full.clone());
        Ok(())
    }

    /// Compares the bytes of every member with its group's first, and
    /// splits a group whose bytes differ.
    fn compare_bytes(&mut self) -> Result<(), DupError> {
        let files = self.buckets.iter().map(|b| b.len() as u64 - 1).sum::<u64>();
        let bytes = self
            .buckets
            .iter()
            .map(|b| (b.len() as u64 - 1) * self.files[b[0]].size * 2)
            .sum::<u64>();
        self.ticker.begin(DupStage::Comparing, files, bytes);
        let mut kept = Vec::new();
        for bucket in std::mem::take(&mut self.buckets) {
            // Each class is a list whose first member is its example.
            let mut classes: Vec<Vec<usize>> = vec![vec![bucket[0]]];
            for &at in &bucket[1..] {
                self.ticker.checkpoint(self.control)?;
                let mut placed = false;
                let mut failed = false;
                for class in &mut classes {
                    let example = class[0];
                    let base = self.ticker.progress.bytes_done;
                    let ticker = &mut self.ticker;
                    let control = self.control;
                    let a = &self.files[example];
                    let b = &self.files[at];
                    ticker.progress.current = b.place.name().map(str::to_owned);
                    let same = same_bytes(
                        self.providers[a.provider].as_ref(),
                        &a.place,
                        self.providers[b.provider].as_ref(),
                        &b.place,
                        &self.ctx,
                        &mut |done| {
                            ticker.hold(control);
                            ticker.progress.bytes_done = base + done;
                            ticker.tick();
                        },
                    );
                    match same {
                        Ok(true) => {
                            class.push(at);
                            placed = true;
                            break;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            let place = self.files[at].place.clone();
                            self.unreadable(place, &error)?;
                            failed = true;
                            break;
                        }
                    }
                }
                if !placed && !failed {
                    classes.push(vec![at]);
                }
                self.ticker.progress.files_done += 1;
                self.ticker.tick();
            }
            kept.extend(classes.into_iter().filter(|c| c.len() > 1));
        }
        self.buckets = kept;
        self.compared = true;
        Ok(())
    }

    /// Splits every bucket by a key; files without one (unreadable) drop.
    fn regroup(&mut self, key: impl Fn(&Candidate) -> Option<Vec<u8>>) {
        let mut kept = Vec::new();
        for bucket in std::mem::take(&mut self.buckets) {
            let mut by_key: BTreeMap<Vec<u8>, Vec<usize>> = BTreeMap::new();
            for at in bucket {
                if let Some(k) = key(&self.files[at]) {
                    by_key.entry(k).or_default().push(at);
                }
            }
            kept.extend(by_key.into_values().filter(|b| b.len() > 1));
        }
        self.buckets = kept;
    }

    /// The groups, the largest waste first.
    fn result(mut self) -> DupResult {
        let mut buckets = std::mem::take(&mut self.buckets);
        let empty = std::mem::take(&mut self.empty);
        if empty.len() > 1 {
            buckets.push(empty);
        }
        let mut groups: Vec<DupGroup> = buckets
            .into_iter()
            .map(|bucket| self.group(bucket))
            .collect();
        groups.sort_by(|a, b| {
            b.waste()
                .cmp(&a.waste())
                .then(b.size.cmp(&a.size))
                .then_with(|| a.files[0].found.cmp(&b.files[0].found))
        });
        DupResult {
            groups,
            files_seen: self.files_seen,
            left_out: self.left_out,
            empty_left_out: self.empty_left_out,
            small_left_out: self.small_left_out,
            hard_links_left_out: self.hard_links_left_out,
            unreadable: self.unreadable,
            hasher: self.env.hasher.name(),
        }
    }

    fn group(&self, bucket: Vec<usize>) -> DupGroup {
        let first = &self.files[bucket[0]];
        let size = first.size;
        let digest = first.full.clone().unwrap_or_default();
        let mut members: Vec<(usize, Option<usize>)> = Vec::new();
        for &at in &bucket {
            members.push((at, None));
            for &link in self.links.get(&at).into_iter().flatten() {
                members.push((link, Some(at)));
            }
        }
        members.sort_by_key(|&(at, _)| self.files[at].found);
        let position: HashMap<usize, usize> = members
            .iter()
            .enumerate()
            .map(|(index, &(at, _))| (at, index))
            .collect();
        let files = members
            .iter()
            .map(|&(at, of)| {
                let file = &self.files[at];
                DupFile {
                    place: file.place.clone(),
                    modified: file.modified,
                    found: file.found,
                    same_file_as: of.and_then(|of| position.get(&of).copied()),
                }
            })
            .collect();
        DupGroup {
            size,
            files,
            digest,
            bytes_compared: self.compared,
        }
    }
}

/// The identity of an entry, when its provider gives volume and inode.
fn identity(
    provider: &dyn Provider,
    volume: Option<u64>,
    inode: Option<u64>,
) -> Option<(String, u64, u64)> {
    Some((provider.scheme().to_owned(), volume?, inode?))
}

/// The quick check of one file: the hash of its first and last `block`
/// bytes and its size, and whether that hash is of the whole file (a
/// file no longer than two blocks is hashed whole). A place without
/// random reads gives its first block only.
pub(super) fn quick(
    provider: &dyn Provider,
    place: &Place,
    size: u64,
    block: u64,
    ctx: &OpContext,
    hasher: &dyn ContentHasher,
) -> Result<(Vec<u8>, bool), VfsError> {
    if size <= block.saturating_mul(2) {
        return full(provider, place, size, ctx, hasher, &mut |_| {}).map(|d| (d, true));
    }
    let block = block as usize;
    let mut data = vec![0u8; block];
    let mut tail = Vec::new();
    let seekable = if provider.capabilities(place).random_read {
        match provider.open_seekable(place, ctx) {
            Ok(reader) => Some(reader),
            Err(VfsError::Unsupported { .. }) => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    match seekable {
        Some(mut reader) => {
            let got = fill(&mut reader, &mut data, &ctx.cancel)?;
            data.truncate(got);
            reader
                .seek(SeekFrom::Start(size - block as u64))
                .map_err(VfsError::from)?;
            tail.resize(block, 0);
            let got = fill(&mut reader, &mut tail, &ctx.cancel)?;
            tail.truncate(got);
        }
        None => {
            let mut reader = provider.read(place, ctx)?;
            let got = fill(&mut reader, &mut data, &ctx.cancel)?;
            data.truncate(got);
        }
    }
    data.extend_from_slice(&tail);
    data.extend_from_slice(&size.to_le_bytes());
    let digest = hasher.hash(&mut Cursor::new(data), &ctx.cancel, &mut |_| {})?;
    Ok((digest, false))
}

/// The full hash of one file; a file whose length is no longer `size`
/// changed since it was listed.
pub(super) fn full(
    provider: &dyn Provider,
    place: &Place,
    size: u64,
    ctx: &OpContext,
    hasher: &dyn ContentHasher,
    on_step: &mut dyn FnMut(u64),
) -> Result<Vec<u8>, VfsError> {
    ctx.check()?;
    let mut reader = provider.read(place, ctx)?;
    let mut read = 0u64;
    let digest = hasher.hash(reader.as_mut(), &ctx.cancel, &mut |done| {
        read = done;
        on_step(done);
    })?;
    if read != size {
        return Err(VfsError::Changed);
    }
    Ok(digest)
}

/// Whether two files hold the same bytes, read side by side in steps;
/// `on_step` hears the bytes read from both.
pub(super) fn same_bytes(
    provider_a: &dyn Provider,
    a: &Place,
    provider_b: &dyn Provider,
    b: &Place,
    ctx: &OpContext,
    on_step: &mut dyn FnMut(u64),
) -> Result<bool, VfsError> {
    ctx.check()?;
    let mut left = provider_a.read(a, ctx)?;
    let mut right = provider_b.read(b, ctx)?;
    let mut one = vec![0u8; STEP];
    let mut two = vec![0u8; STEP];
    let mut done = 0u64;
    loop {
        ctx.check()?;
        let got_one = fill(left.as_mut() as &mut dyn Read, &mut one, &ctx.cancel)?;
        let got_two = fill(right.as_mut() as &mut dyn Read, &mut two, &ctx.cancel)?;
        if got_one != got_two || one[..got_one] != two[..got_two] {
            return Ok(false);
        }
        if got_one == 0 {
            return Ok(true);
        }
        done += 2 * got_one as u64;
        on_step(done);
        if got_one < STEP {
            return Ok(true);
        }
    }
}
