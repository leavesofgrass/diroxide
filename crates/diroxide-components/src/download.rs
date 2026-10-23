//! The one downloader: sources in order, a `.part` file with a note of
//! where its bytes came from, the size and the hash, then a rename; files
//! that already check out are kept; a stopped download goes on from its
//! `.part` file the next time.
//!
//! The design is textweaver's (`download.rs`, the owner's own code,
//! relicensed under answer 3): each file goes to `<name>.part` in a
//! staging folder beside the component's folder, is checked by size and
//! SHA-256 and renamed; when every file is checked they move into place
//! together, so the component's folder never holds a file that did not
//! match; a lock file in the staging folder, renewed every 10 seconds and
//! honored for 60, stops two programs fetching one component.
//!
//! Hardened for diroxide (R22 section 6.2, spike S3):
//!
//! - Nothing is fetched without the owner's yes: [`Downloader::download`]
//!   takes a [`Consent`], which only [`Consent::yes`] makes.
//! - The free space is checked first, for the bytes still to fetch and
//!   the unpacked size: "needs 300 MB, 120 MB free", before a byte moves.
//! - A note beside the `.part` file (`<name>.part.from`) keeps the address,
//!   the pin and the server's validator; a resume from the same address
//!   sends `If-Range`, so a file that changed comes whole.
//! - A server that answers a resume with the whole file is written from
//!   the start; one that cannot resume, or a resumed file whose hash does
//!   not match, costs one more transfer from zero from the same address,
//!   and only then the next address. A `.part` file as long as the pin is
//!   hashed before anything is fetched. The staged file is flushed to disk
//!   before its rename. No path through this code installs a file whose
//!   size and hash do not match its pin.
//! - Every transfer runs on its own thread and reports through a channel,
//!   so Escape stops the download at once, whatever the transport is
//!   doing, and a server that sends nothing for the stall time (60
//!   seconds) is given up on; what arrived is kept for the next try.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use diroxide_core::{Place, Secret};
use diroxide_vfs::{CancelToken, LocalProvider, OpContext, ProgressUpdate, Provider};

use crate::component::{Action, Component, LOCK_NAME, PART_SUFFIX, SIDECAR_SUFFIX, is_https};
use crate::error::ComponentError;
use crate::fetch::{FetchError, Fetcher, Request};
use crate::pin::{FilePin, host_target, is_plain_name};
use crate::receipt::Receipt;

/// How long another program's lock is honored without being renewed.
pub const LOCK_FRESH: Duration = Duration::from_secs(60);

/// How often a running download renews its lock.
pub const LOCK_RENEW: Duration = Duration::from_secs(10);

/// How long a server may send nothing before the download gives up on it.
pub const STALL: Duration = Duration::from_secs(60);

/// The base of the project's release downloads, from the repository in
/// the workspace's metadata: `<repository>/releases/download`. A file
/// with no address of its own is at `<base>/<id>-<version>/<name>`, so
/// the list never names an account.
pub fn project_releases() -> Option<String> {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    is_https(repository).then(|| format!("{repository}/releases/download"))
}

/// The owner's `components_source`, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A folder on this computer: files at `<folder>/<id>-<version>/<name>`,
    /// and its own list at `<folder>/components.toml`.
    Folder(PathBuf),
    /// A repository's release downloads (`owner/name` in the setting):
    /// files at `<base>/<id>-<version>/<name>`.
    Releases(String),
}

impl Source {
    /// Reads the setting: empty is none; an existing folder, or anything
    /// that is not `owner/name`, is a folder (a missing folder says so
    /// when it is used); `owner/name` is that repository's releases.
    pub fn from_setting(setting: &str) -> Option<Source> {
        let value = setting.trim();
        if value.is_empty() {
            return None;
        }
        let path = Path::new(value);
        if path.is_dir() || path.is_absolute() {
            return Some(Source::Folder(path.to_path_buf()));
        }
        match value.split_once('/') {
            Some((owner, name))
                if is_plain_name(owner) && is_plain_name(name) && !name.contains('/') =>
            {
                Some(Source::Releases(format!(
                    "https://github.com/{owner}/{name}/releases/download"
                )))
            }
            _ => Some(Source::Folder(path.to_path_buf())),
        }
    }

    /// The source in words for the ask-first question, never its address:
    /// "your own components source".
    pub fn words(&self) -> &'static str {
        "your own components source"
    }
}

/// One address to try, and whether it is the owner's own source (the only
/// place a token may go).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    /// The address: a path or an `https` address.
    pub url: String,
    /// Whether it is the owner's own source.
    pub own_source: bool,
}

/// Where files may come from, in order: the owner's own source when one is
/// set, then each file's public address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    /// The owner's own source.
    pub source: Option<Source>,
    /// The base of the public release downloads, for files with no address
    /// of their own; `None` in tests, which never reach the network.
    pub releases: Option<String>,
}

impl Sources {
    /// The project's public releases only.
    pub fn public() -> Self {
        Self {
            source: None,
            releases: project_releases(),
        }
    }

    /// No public address at all: a folder source alone, for tests and for
    /// a run that must not reach the network.
    pub fn offline() -> Self {
        Self::default()
    }

    /// These sources with the owner's `components_source` tried first.
    pub fn with_source(mut self, setting: &str) -> Self {
        self.source = Source::from_setting(setting);
        self
    }

    /// The addresses to try for `file` of `component`, in order.
    pub fn addresses(&self, component: &Component, file: &FilePin) -> Vec<Address> {
        let release = component.release();
        let mut out = Vec::new();
        match &self.source {
            Some(Source::Folder(folder)) => out.push(Address {
                url: folder.join(&release).join(&file.name).display().to_string(),
                own_source: true,
            }),
            Some(Source::Releases(base)) => out.push(Address {
                url: format!("{base}/{release}/{}", file.name),
                own_source: true,
            }),
            None => {}
        }
        if let Some(url) = &file.url {
            out.push(Address {
                url: url.clone(),
                own_source: false,
            });
        } else if let Some(base) = &self.releases {
            out.push(Address {
                url: format!("{base}/{release}/{}", file.name),
                own_source: false,
            });
        }
        out
    }

    /// The source folder's own list, `components.toml`, when the source is
    /// a folder.
    pub fn list_path(&self) -> Option<PathBuf> {
        match &self.source {
            Some(Source::Folder(folder)) => Some(folder.join(crate::manifest::FILE_NAME)),
            _ => None,
        }
    }

    /// Whether every address is on this computer, so no transport is
    /// needed.
    pub fn is_offline(&self) -> bool {
        self.releases.is_none() && !matches!(self.source, Some(Source::Releases(_)))
    }
}

/// The owner's yes to one component's download, given after the question
/// said its size and license. The downloader refuses to start without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    id: String,
    version: String,
}

impl Consent {
    /// The yes: call it only when the owner answered yes to the question
    /// for `component` (the app's dialog, `diro components download`'s
    /// prompt, or its `--yes`).
    pub fn yes(component: &Component) -> Self {
        Self {
            id: component.id.clone(),
            version: component.version.clone(),
        }
    }

    fn covers(&self, component: &Component) -> bool {
        self.id == component.id && self.version == component.version
    }
}

/// How far a download has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Bytes so far, across the component (files kept count as done).
    pub done: u64,
    /// Bytes in the whole component.
    pub total: u64,
}

impl Progress {
    /// Whole percent done, 0 to 100.
    pub fn percent(&self) -> u64 {
        if self.total == 0 {
            return 100;
        }
        (u128::from(self.done.min(self.total)) * 100 / u128::from(self.total)) as u64
    }
}

/// What the downloader tells its caller as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Started: the bytes to fetch in all.
    Started(Progress),
    /// More bytes arrived.
    Progress(Progress),
    /// A file is fetched again from the start: the reason in words.
    Retrying {
        /// The file's name.
        file: String,
        /// Why, in words.
        reason: String,
    },
    /// Every file arrived; they are being checked and moved into place.
    Checking,
    /// An archive is being unpacked.
    Unpacking,
}

/// What a download or an install did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Files fetched (or copied) and checked.
    pub fetched: Vec<String>,
    /// Files already in place that checked out, kept as they were (a folder
    /// placed by hand is adopted, not fetched again).
    pub kept: Vec<String>,
    /// Members of an archive that were not unpacked, each with the reason
    /// in words.
    pub left_out: Vec<(String, String)>,
}

/// The free space where the components live.
pub trait FreeSpace: Send + Sync {
    /// Bytes the owner may still write on the volume holding `path` (or its
    /// nearest existing folder), when known.
    fn free_bytes(&self, path: &Path) -> Option<u64>;
}

/// The volume's own free space, from the local provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSpace;

impl FreeSpace for SystemSpace {
    fn free_bytes(&self, path: &Path) -> Option<u64> {
        let place = Place::parse(&path.to_string_lossy()).ok()?;
        LocalProvider::new()
            .space(&place, &OpContext::new())
            .ok()?
            .free_to_user
    }
}

/// The downloader for one components folder.
pub struct Downloader {
    components_dir: PathBuf,
    target: String,
    sources: Sources,
    fetcher: Arc<dyn Fetcher>,
    token: Option<Arc<Secret>>,
    space: Arc<dyn FreeSpace>,
    stall: Duration,
}

impl std::fmt::Debug for Downloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloader")
            .field("components_dir", &self.components_dir)
            .field("target", &self.target)
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

impl Downloader {
    /// A downloader for the components under `components_dir`, for this
    /// computer, trying `sources` in order through `fetcher`.
    pub fn new(components_dir: PathBuf, sources: Sources, fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            components_dir,
            target: host_target().to_owned(),
            sources,
            fetcher,
            token: None,
            space: Arc::new(SystemSpace),
            stall: STALL,
        }
    }

    /// The same for another computer's files (tests, a portable folder made
    /// for another computer).
    pub fn with_target(mut self, target: &str) -> Self {
        self.target = target.to_owned();
        self
    }

    /// The same with a token for the owner's own source. It goes to that
    /// source's addresses only, never to a public one.
    pub fn with_token(mut self, token: Secret) -> Self {
        self.token = Some(Arc::new(token));
        self
    }

    /// The same with another way to read the free space (tests).
    pub fn with_space(mut self, space: Arc<dyn FreeSpace>) -> Self {
        self.space = space;
        self
    }

    /// The same with another stall time (tests).
    pub fn with_stall(mut self, stall: Duration) -> Self {
        self.stall = stall;
        self
    }

    /// The sources it tries.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// The components folder.
    pub fn components_dir(&self) -> &Path {
        &self.components_dir
    }

    /// The computer it fetches for.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Bytes still needed on the volume: what is left to fetch, and the
    /// unpacked size when the component unpacks and is not installed.
    pub fn needed(&self, component: &Component) -> u64 {
        let dir = component.dir_in(&self.components_dir);
        let staging = component.staging_in(&self.components_dir);
        let mut needed = 0;
        let mut all_there = true;
        for file in component.files_for(&self.target) {
            if file.size_matches(&dir.join(&file.name)) || Receipt::matching(&dir, file).is_some() {
                continue;
            }
            all_there = false;
            if file.size_matches(&staging.join(&file.name)) {
                continue;
            }
            let have = fs::symlink_metadata(staging.join(format!("{}{PART_SUFFIX}", file.name)))
                .map_or(0, |m| m.len().min(file.size));
            needed += file.size - have;
        }
        if component.action == Action::Unpack && !all_there {
            needed += component.unpacked_size;
        }
        needed
    }

    /// Downloads every file of `component` for this computer that is not
    /// already in place and checked, then moves them into its folder
    /// together. `consent` is the owner's yes; `ctx` carries the cancel
    /// token (Escape) and the job's progress sink; `events` hears each
    /// step. A stopped download keeps its `.part` files, and the next one
    /// goes on from them.
    pub fn download(
        &self,
        component: &Component,
        consent: &Consent,
        ctx: &OpContext,
        events: &mut dyn FnMut(&Event),
    ) -> Result<Outcome, ComponentError> {
        if !consent.covers(component) {
            return Err(ComponentError::NotAsked(component.id.clone()));
        }
        component.check().map_err(ComponentError::Manifest)?;
        let files = component.files_for(&self.target);
        if files.is_empty() {
            return Err(ComponentError::NoBuild(component.id.clone()));
        }
        let needed = self.needed(component);
        if needed > 0
            && let Some(free) = self.space.free_bytes(&self.components_dir)
            && free < needed
        {
            return Err(ComponentError::NoRoom { needed, free });
        }
        let dest = component.dir_in(&self.components_dir);
        let staging = component.staging_in(&self.components_dir);
        let mut claim = Claim::take(&component.id, &dest, staging.clone())?;
        let total: u64 = files.iter().map(|f| f.size).sum();
        let mut outcome = Outcome::default();
        let mut done = 0u64;
        events(&Event::Started(Progress { done, total }));
        for (index, file) in files.iter().enumerate() {
            if file.matches_file(&dest.join(&file.name)) || Receipt::matching(&dest, file).is_some()
            {
                done += file.size;
                outcome.kept.push(file.name.clone());
                events(&Event::Progress(Progress { done, total }));
                continue;
            }
            claim.stage(&component.id)?;
            let mut step = Step {
                downloader: self,
                component,
                file,
                staging: &staging,
                ctx,
                done,
                total,
                index,
                count: files.len(),
            };
            step.fetch(&mut claim, events)?;
            done += file.size;
            outcome.fetched.push(file.name.clone());
            events(&Event::Progress(Progress { done, total }));
        }
        events(&Event::Checking);
        if component.action == Action::Unpack {
            events(&Event::Unpacking);
        }
        outcome.left_out = finish(component, &dest, &staging, &outcome.fetched)?;
        Ok(outcome)
    }
}

/// One file's fetch.
struct Step<'a> {
    downloader: &'a Downloader,
    component: &'a Component,
    file: &'a FilePin,
    staging: &'a Path,
    ctx: &'a OpContext,
    done: u64,
    total: u64,
    index: usize,
    count: usize,
}

/// What the note beside a `.part` file says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Note {
    address: String,
    size: u64,
    sha256: String,
    validator: Option<String>,
}

impl Note {
    fn read(path: &Path) -> Option<Note> {
        let text = fs::read_to_string(path).ok()?;
        let mut note = Note::default();
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("address", v)) => note.address = v.to_owned(),
                Some(("size", v)) => note.size = v.parse().ok()?,
                Some(("sha256", v)) => note.sha256 = v.to_owned(),
                Some(("validator", v)) => note.validator = Some(v.to_owned()),
                _ => {}
            }
        }
        Some(note)
    }

    fn write(&self, path: &Path) -> Result<(), ComponentError> {
        let mut text = format!(
            "address {}\nsize {}\nsha256 {}\n",
            self.address, self.size, self.sha256
        );
        if let Some(validator) = &self.validator {
            text.push_str(&format!("validator {validator}\n"));
        }
        fs::write(path, text).map_err(|e| ComponentError::io(path, e))
    }
}

/// A message from a transfer's thread.
enum Message {
    Opened {
        start: u64,
        validator: Option<String>,
    },
    Data(Vec<u8>),
    End,
    Failed(FetchError),
}

/// Cancels the transfer's own token when the transfer is left, which
/// kills a curl or wget it started.
struct StopOnDrop(CancelToken);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl Step<'_> {
    fn part(&self) -> PathBuf {
        self.staging
            .join(format!("{}{PART_SUFFIX}", self.file.name))
    }

    fn note(&self) -> PathBuf {
        self.staging
            .join(format!("{}{SIDECAR_SUFFIX}", self.file.name))
    }

    fn staged(&self) -> PathBuf {
        self.staging.join(&self.file.name)
    }

    fn fetch_error(&self, error: FetchError) -> ComponentError {
        match error {
            FetchError::NoClient => ComponentError::NoTransport,
            other => ComponentError::Fetch {
                file: self.file.name.clone(),
                reason: other.reason(),
            },
        }
    }

    /// Removes the `.part` file and its note.
    fn discard(&self) -> Result<(), ComponentError> {
        remove_if_there(&self.part())?;
        remove_if_there(&self.note())
    }

    /// Moves the checked `.part` file to its staged name.
    fn accept(&self) -> Result<(), ComponentError> {
        let staged = self.staged();
        fs::rename(self.part(), &staged).map_err(|e| ComponentError::io(&staged, e))?;
        remove_if_there(&self.note())
    }

    /// Readies what a stopped download left: a staged file that checks
    /// out is done; a `.part` file as long as the pin is hashed (done when
    /// it matches); one longer than the pin, or whose note is for another
    /// pin, is removed. True when the file is already done.
    fn ready(&self) -> Result<bool, ComponentError> {
        let staged = self.staged();
        if staged.exists() {
            if self.file.matches_file(&staged) {
                return Ok(true);
            }
            remove_if_there(&staged)?;
        }
        let part = self.part();
        let Ok(meta) = fs::symlink_metadata(&part) else {
            remove_if_there(&self.note())?;
            return Ok(false);
        };
        let note = Note::read(&self.note());
        let same_pin =
            note.is_some_and(|n| n.size == self.file.size && n.sha256 == self.file.sha256);
        if !meta.is_file() || meta.len() > self.file.size || !same_pin {
            self.discard()?;
            return Ok(false);
        }
        if meta.len() == self.file.size {
            if self.file.matches_file(&part) {
                self.accept()?;
                return Ok(true);
            }
            self.discard()?;
        }
        Ok(false)
    }

    fn fetch(
        &mut self,
        claim: &mut Claim,
        events: &mut dyn FnMut(&Event),
    ) -> Result<(), ComponentError> {
        if self.ready()? {
            return Ok(());
        }
        let addresses = self.downloader.sources.addresses(self.component, self.file);
        if addresses.is_empty() {
            return Err(ComponentError::NoSource {
                file: self.file.name.clone(),
            });
        }
        let mut last: Option<ComponentError> = None;
        for address in &addresses {
            if address.own_source
                && let Some(Source::Folder(folder)) = &self.downloader.sources.source
                && !folder.is_dir()
            {
                last = Some(ComponentError::Fetch {
                    file: self.file.name.clone(),
                    reason: "the source folder is missing".to_owned(),
                });
                continue;
            }
            let mut retried = false;
            loop {
                self.ctx
                    .cancel
                    .check()
                    .map_err(|_| ComponentError::Cancelled)?;
                let from = fs::symlink_metadata(self.part()).map_or(0, |m| m.len());
                let known = Note::read(&self.note()).filter(|n| n.address == address.url);
                let if_range = known.and_then(|n| n.validator).filter(|_| from > 0);
                Note {
                    address: address.url.clone(),
                    size: self.file.size,
                    sha256: self.file.sha256.clone(),
                    validator: if_range.clone(),
                }
                .write(&self.note())?;
                match self.transfer(address, from, if_range, claim, events) {
                    Ok(resumed) => {
                        if self.file.matches_file(&self.part()) {
                            return self.accept();
                        }
                        self.discard()?;
                        if resumed && !retried {
                            retried = true;
                            events(&Event::Retrying {
                                file: self.file.name.clone(),
                                reason: "the resumed file did not match".to_owned(),
                            });
                            continue;
                        }
                        last = Some(ComponentError::Hash {
                            file: self.file.name.clone(),
                        });
                        break;
                    }
                    Err(Attempt::Stop(ComponentError::Cancelled)) => {
                        return Err(ComponentError::Cancelled);
                    }
                    Err(Attempt::CannotResume) if !retried => {
                        retried = true;
                        self.discard()?;
                        events(&Event::Retrying {
                            file: self.file.name.clone(),
                            reason: FetchError::CannotResume.reason(),
                        });
                    }
                    Err(Attempt::CannotResume) => {
                        self.discard()?;
                        last = Some(self.fetch_error(FetchError::CannotResume));
                        break;
                    }
                    Err(Attempt::Stop(e @ ComponentError::Size { .. })) => {
                        self.discard()?;
                        last = Some(e);
                        break;
                    }
                    Err(Attempt::Stop(e)) => {
                        last = Some(e);
                        break;
                    }
                }
            }
        }
        Err(last.unwrap_or(ComponentError::NoSource {
            file: self.file.name.clone(),
        }))
    }

    /// One transfer from one address into the `.part` file, from byte
    /// `from`. Ok(true) when it went on from `from`; the caller checks the
    /// hash.
    fn transfer(
        &self,
        address: &Address,
        from: u64,
        if_range: Option<String>,
        claim: &mut Claim,
        events: &mut dyn FnMut(&Event),
    ) -> Result<bool, Attempt> {
        let attempt = CancelToken::new();
        let _stop = StopOnDrop(attempt.clone());
        let (tx, rx) = mpsc::sync_channel::<Message>(16);
        {
            let fetcher = Arc::clone(&self.downloader.fetcher);
            let url = address.url.clone();
            let limit = self.file.size;
            let token = address
                .own_source
                .then(|| self.downloader.token.clone())
                .flatten();
            let attempt = attempt.clone();
            thread::spawn(move || {
                let request = Request {
                    address: &url,
                    from,
                    if_range: if_range.as_deref(),
                    limit,
                    token: token.as_deref(),
                    cancel: &attempt,
                };
                let fetched = match fetcher.open(&request) {
                    Ok(fetched) => fetched,
                    Err(error) => {
                        let _ = tx.send(Message::Failed(error));
                        return;
                    }
                };
                let opened = Message::Opened {
                    start: fetched.start,
                    validator: fetched.validator,
                };
                if tx.send(opened).is_err() {
                    return;
                }
                let mut reader = fetched.reader;
                loop {
                    let mut buf = vec![0u8; 1 << 16];
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _ = tx.send(Message::End);
                            return;
                        }
                        Ok(n) => {
                            buf.truncate(n);
                            if tx.send(Message::Data(buf)).is_err() {
                                return;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            let _ = tx.send(Message::Failed(FetchError::from_io(&e)));
                            return;
                        }
                    }
                }
            });
        }
        let part = self.part();
        let mut out: Option<File> = None;
        let mut got = 0u64;
        let mut resumed = false;
        let mut heard = Instant::now();
        let flush = |out: &mut Option<File>| -> Result<(), ComponentError> {
            if let Some(file) = out.as_mut() {
                file.flush().map_err(|e| ComponentError::io(&part, e))?;
            }
            Ok(())
        };
        loop {
            if self.ctx.cancel.is_cancelled() {
                flush(&mut out)?;
                return Err(ComponentError::Cancelled.into());
            }
            let message = match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    if heard.elapsed() >= self.downloader.stall {
                        flush(&mut out)?;
                        return Err(self.fetch_error(FetchError::Timeout).into());
                    }
                    claim.renew();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    flush(&mut out)?;
                    return Err(self.fetch_error(FetchError::Lost).into());
                }
            };
            heard = Instant::now();
            match message {
                Message::Opened { start, validator } => {
                    let file = if from > 0 && start == from {
                        resumed = true;
                        got = from;
                        OpenOptions::new().append(true).open(&part)
                    } else if start == 0 {
                        got = 0;
                        File::create(&part)
                    } else {
                        return Err(Attempt::CannotResume);
                    }
                    .map_err(|e| ComponentError::io(&part, e))?;
                    out = Some(file);
                    if validator.is_some() {
                        Note {
                            address: address.url.clone(),
                            size: self.file.size,
                            sha256: self.file.sha256.clone(),
                            validator,
                        }
                        .write(&self.note())?;
                    }
                    self.report(got, events);
                }
                Message::Data(bytes) => {
                    let Some(file) = out.as_mut() else {
                        return Err(self.fetch_error(FetchError::Lost).into());
                    };
                    let after = got + bytes.len() as u64;
                    if after > self.file.size {
                        return Err(ComponentError::Size {
                            file: self.file.name.clone(),
                            got: after,
                            expected: self.file.size,
                        }
                        .into());
                    }
                    file.write_all(&bytes)
                        .map_err(|e| ComponentError::io(&part, e))?;
                    got = after;
                    self.report(got, events);
                    claim.renew();
                }
                Message::End => {
                    if let Some(file) = out.as_mut() {
                        file.flush().map_err(|e| ComponentError::io(&part, e))?;
                        file.sync_all().map_err(|e| ComponentError::io(&part, e))?;
                    }
                    if got < self.file.size {
                        return Err(self.fetch_error(FetchError::Lost).into());
                    }
                    return Ok(resumed);
                }
                Message::Failed(FetchError::CannotResume) => {
                    flush(&mut out)?;
                    return Err(Attempt::CannotResume);
                }
                Message::Failed(FetchError::TooLarge) => {
                    return Err(ComponentError::Size {
                        file: self.file.name.clone(),
                        got: self.file.size + 1,
                        expected: self.file.size,
                    }
                    .into());
                }
                Message::Failed(error) => {
                    flush(&mut out)?;
                    return Err(self.fetch_error(error).into());
                }
            }
        }
    }

    fn report(&self, got: u64, events: &mut dyn FnMut(&Event)) {
        let done = self.done + got.min(self.file.size);
        events(&Event::Progress(Progress {
            done,
            total: self.total,
        }));
        self.ctx.progress.report(ProgressUpdate {
            items_done: self.index as u64,
            items_total: Some(self.count as u64),
            bytes_done: done,
            bytes_total: Some(self.total),
            current: Some(self.file.name.clone()),
        });
    }
}

/// How a transfer ended badly: the server cannot go on from the byte
/// asked (the step starts again from zero, once), or anything else.
enum Attempt {
    CannotResume,
    Stop(ComponentError),
}

impl From<ComponentError> for Attempt {
    fn from(error: ComponentError) -> Self {
        Attempt::Stop(error)
    }
}

fn remove_if_there(path: &Path) -> Result<(), ComponentError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ComponentError::io(path, e)),
    }
}

/// Moves the checked files `names` from `staging` into `dest`, then
/// unpacks the component's archives when its action is
/// [`Action::Unpack`]. Returns the archive members left out, with the
/// reason.
pub(crate) fn finish(
    component: &Component,
    dest: &Path,
    staging: &Path,
    names: &[String],
) -> Result<Vec<(String, String)>, ComponentError> {
    fs::create_dir_all(dest).map_err(|e| ComponentError::io(dest, e))?;
    for name in names {
        let to = dest.join(name);
        if fs::symlink_metadata(&to).is_ok_and(|m| m.is_file()) {
            fs::remove_file(&to).map_err(|e| ComponentError::io(&to, e))?;
        }
        fs::rename(staging.join(name), &to).map_err(|e| ComponentError::io(&to, e))?;
        if component.action == Action::Place
            && let Some(file) = component.file(name)
            && file.exec.as_deref() == Some(name.as_str())
        {
            set_executable(&to);
        }
    }
    Ok(Vec::new())
}

/// Marks a program executable, on Unix: owner, group and others may run
/// it; only the owner may write it.
#[cfg(unix)]
pub(crate) fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
pub(crate) fn set_executable(_path: &Path) {}

/// Components being downloaded or installed in this program, by folder.
static BUSY: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// The claim on one component's folder: in this program (a set of
/// folders), and across programs (a lock file in the staging folder,
/// renewed while the download runs and honored for a minute after).
pub(crate) struct Claim {
    dest: PathBuf,
    staging: PathBuf,
    lock: Option<PathBuf>,
    renewed: Instant,
}

impl Claim {
    /// Claims `dest` for `id`, in this program.
    pub(crate) fn take(id: &str, dest: &Path, staging: PathBuf) -> Result<Claim, ComponentError> {
        let mut busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
        if !busy
            .get_or_insert_with(HashSet::new)
            .insert(dest.to_path_buf())
        {
            return Err(ComponentError::Busy(id.to_owned()));
        }
        Ok(Claim {
            dest: dest.to_path_buf(),
            staging,
            lock: None,
            renewed: Instant::now(),
        })
    }

    /// Makes the staging folder and takes its lock file, once. A lock
    /// another program renewed within the last minute says busy; an older
    /// one was left by a program that stopped, and is taken over.
    pub(crate) fn stage(&mut self, id: &str) -> Result<&Path, ComponentError> {
        if self.lock.is_none() {
            fs::create_dir_all(&self.staging).map_err(|e| ComponentError::io(&self.staging, e))?;
            let lock = self.staging.join(LOCK_NAME);
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let fresh = fs::metadata(&lock)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age < LOCK_FRESH);
                    if fresh {
                        return Err(ComponentError::Busy(id.to_owned()));
                    }
                    fs::write(&lock, b"").map_err(|e| ComponentError::io(&lock, e))?;
                }
                Err(e) => return Err(ComponentError::io(&lock, e)),
            }
            self.lock = Some(lock);
            self.renewed = Instant::now();
        }
        Ok(&self.staging)
    }

    /// Renews the lock file now and then during a long download.
    pub(crate) fn renew(&mut self) {
        if let Some(lock) = &self.lock
            && self.renewed.elapsed() >= LOCK_RENEW
        {
            let _ = fs::write(lock, b"");
            self.renewed = Instant::now();
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(lock) = self.lock.take() {
            let _ = fs::remove_file(&lock);
            // Only an empty staging folder goes; `.part` files of a
            // stopped download stay for the next one.
            let _ = fs::remove_dir(&self.staging);
        }
        let mut busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(set) = busy.as_mut() {
            set.remove(&self.dest);
        }
    }
}
