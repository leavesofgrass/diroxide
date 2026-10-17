//! Spike T1, sequential archives (research/alpha2-plan.md 4.11): what
//! listing a large tar.xz costs, and how fast it stops. A tar.xz of half
//! text and half noise is written to a scratch folder on disk, then
//! listed through the provider and the real local registry: the time, the
//! resident peak, the rate in compressed bytes a second (what the "Reading
//! ... of ... GB" line counts), and the cancel latency from the moment a
//! progress report cancels the token to the moment the listing returns.
//!
//! About 8 MiB normally; 256 MiB when `DIRO_SPIKE_LARGE=1` is set (the
//! container's 1 GiB run is the orchestrator's). Run with
//! `cargo nextest run -p diroxide-archive spike_sequential --no-capture`
//! (and `--release` for the figures in the hand-back).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_archive::words::reading_line;
use diroxide_archive::{Allowances, ArchiveError, ArchiveProvider, Limits};
use diroxide_core::Place;
use diroxide_vfs::{CancelToken, OpContext, ProgressSink, ProgressUpdate, Registry, VfsError};

use crate::build::noise;
use crate::memory::{Peak, mib, resident};
use crate::spike_listing::scratch;

/// The compressed size to reach.
fn target_bytes() -> u64 {
    if std::env::var_os("DIRO_SPIKE_LARGE").is_some_and(|v| v == "1") {
        256 << 20
    } else {
        8 << 20
    }
}

/// Counts the bytes written through it.
struct Counted<W> {
    inner: W,
    count: Arc<AtomicU64>,
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// One file's contents: lines of text, then as much noise.
fn contents(n: usize) -> Vec<u8> {
    let mut text = Vec::with_capacity(256 << 10);
    let mut line = 0usize;
    while text.len() < 128 << 10 {
        writeln!(
            text,
            "line {line} of file {n}: the archive is read once, from start to end, {}",
            line * 7919 % 10_007
        )
        .unwrap();
        line += 1;
    }
    text.extend_from_slice(&noise(128 << 10, n as u64 * 2 + 1));
    text
}

/// Writes the tar.xz; gives its size, the bytes inside and the files.
fn write_tar_xz(path: &Path, target: u64) -> (u64, u64, usize) {
    let count = Arc::new(AtomicU64::new(0));
    let file = Counted {
        inner: BufWriter::new(File::create(path).unwrap()),
        count: count.clone(),
    };
    let mut options = lzma_rust2::XzOptions::with_preset(1);
    options.set_block_size(NonZeroU64::new(8 << 20));
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8)) as u32;
    let xz = lzma_rust2::XzWriterMt::new(file, options, threads).unwrap();
    let mut tar = tar::Builder::new(xz);
    let mut inside = 0u64;
    let mut files = 0usize;
    // Half noise, half text: about 0.55 compressed bytes for each byte
    // inside, so the bytes inside decide when to stop (the threaded
    // writer holds whole blocks back, so its count lags).
    while inside < target * 2 {
        let data = contents(files);
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(1_760_000_000);
        tar.append_data(
            &mut header,
            format!("logs/day{:03}/file{files:05}.log", files / 100),
            data.as_slice(),
        )
        .unwrap();
        inside += data.len() as u64;
        files += 1;
    }
    let xz = tar.into_inner().unwrap();
    let mut file = xz.finish().unwrap();
    file.flush().unwrap();
    drop(file);
    (std::fs::metadata(path).unwrap().len(), inside, files)
}

/// Keeps the reports.
#[derive(Default)]
struct Reports(Mutex<Vec<ProgressUpdate>>);

impl ProgressSink for Reports {
    fn report(&self, update: ProgressUpdate) {
        self.0.lock().unwrap().push(update);
    }
}

fn provider() -> (Arc<Registry>, Arc<ArchiveProvider>) {
    ArchiveProvider::install(Registry::with_local(), Limits::default())
}

/// When to cancel a listing.
enum Cancel {
    /// From the progress sink, at the first report past this many bytes.
    AtBytes(u64),
    /// From another thread, after this many seconds.
    After(f64),
}

/// Cancels at the first report past `at` bytes, keeping the moment.
struct CancelAt {
    at: u64,
    token: CancelToken,
    when: Mutex<Option<Instant>>,
}

impl ProgressSink for CancelAt {
    fn report(&self, update: ProgressUpdate) {
        if update.bytes_done >= self.at && !self.token.is_cancelled() {
            *self.when.lock().unwrap() = Some(Instant::now());
            self.token.cancel();
        }
    }
}

/// Lists the archive on a fresh provider and cancels it: the time from
/// the cancel to the listing's return.
fn cancel_latency(place: &Place, how: Cancel) -> Duration {
    let (_registry, archive) = provider();
    let token = CancelToken::new();
    let (result, returned, cancelled_at) = match how {
        Cancel::AtBytes(at) => {
            let sink = Arc::new(CancelAt {
                at,
                token: token.clone(),
                when: Mutex::new(None),
            });
            let ctx = OpContext::new()
                .with_cancel(token)
                .with_progress(sink.clone());
            let result = archive.open_archive(place, &ctx, Allowances::default());
            let returned = Instant::now();
            let when = sink.when.lock().unwrap().expect("cancelled");
            (result, returned, when)
        }
        Cancel::After(seconds) => {
            let ctx = OpContext::new().with_cancel(token.clone());
            let timer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f64(seconds));
                token.cancel();
                Instant::now()
            });
            let result = archive.open_archive(place, &ctx, Allowances::default());
            let returned = Instant::now();
            (result, returned, timer.join().unwrap())
        }
    };
    assert!(
        matches!(result, Err(ArchiveError::Vfs(VfsError::Cancelled))),
        "{result:?}"
    );
    // Nothing was cached by the cancelled listing: the next one reads again.
    let again = archive.open_archive(place, &OpContext::new(), Allowances::default());
    assert!(again.is_ok(), "{again:?}");
    returned.saturating_duration_since(cancelled_at)
}

#[test]
fn a_large_tar_xz_lists_at_a_steady_rate_and_stops_within_a_second() {
    let dir = scratch();
    let path = dir.path().join("big.tar.xz");
    let (packed, inside, files) = write_tar_xz(&path, target_bytes());
    let place = Place::parse(&path.to_string_lossy()).unwrap();

    // The listing, whole.
    let (_registry, archive) = provider();
    let reports = Arc::new(Reports::default());
    let ctx = OpContext::new().with_progress(reports.clone());
    let before = resident();
    let peak = Peak::start();
    let start = Instant::now();
    let opened = archive
        .open_archive(&place, &ctx, Allowances::default())
        .unwrap();
    let seconds = start.elapsed().as_secs_f64();
    let peak = peak.stop();
    assert_eq!(opened.index().summary().members, files as u64);
    let rate = packed as f64 / seconds;
    let lines = reports.0.lock().unwrap();
    let first = lines.first().expect("the listing reports progress");
    let line = reading_line("big.tar.xz", first.bytes_done, first.bytes_total);
    assert!(line.starts_with("Reading big.tar.xz: "), "{line}");
    assert!(line.chars().count() <= 40, "{line}");
    eprintln!(
        "T1 list: {:.1} MiB tar.xz ({:.1} MiB inside, {files} files) in {seconds:.2} s; \
         {:.1} MiB/s compressed, {:.1} MiB/s inside; resident {:.0} MiB before, peak {:.0} MiB; \
         {} progress lines, the first \"{line}\"",
        mib(packed),
        mib(inside),
        mib(rate as u64),
        mib((inside as f64 / seconds) as u64),
        mib(before),
        mib(peak),
        lines.len(),
    );
    drop(lines);

    // The same listing, cancelled half way: from the progress sink (the
    // listing's own thread, so the moment is sure to fall inside it), and,
    // in the large run, from another thread a third of the way in, as the
    // jobs line's Cancel does.
    let mut latencies = vec![cancel_latency(&place, Cancel::AtBytes(packed / 2))];
    if target_bytes() > 16 << 20 {
        latencies.push(cancel_latency(&place, Cancel::After(seconds / 3.0)));
    }
    for latency in &latencies {
        eprintln!(
            "T1 cancel: stopped {:.1} ms after the cancel",
            latency.as_secs_f64() * 1000.0
        );
        assert!(*latency < Duration::from_secs(1), "{latency:?}");
    }
    eprintln!(
        "T1 promise: a 14.0 GB tar.xz like this one lists in about {:.0} s",
        14.0e9 / rate
    );
}
