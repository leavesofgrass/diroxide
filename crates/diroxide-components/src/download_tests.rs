//! The downloader against the fake fetcher and the loopback server (R22
//! section 6.6 and spike S3). The server's modes run with the built-in
//! client (the `fetch` feature) and with the real curl when the tools
//! registry finds one. Nothing here goes past 127.0.0.1.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use diroxide_core::Secret;
use diroxide_tools::{ToolRegistry, ToolStatus};
use diroxide_vfs::{CancelToken, OpContext, ProgressSink, ProgressUpdate};

use crate::component::{Component, LOCK_NAME, Status};
use crate::download::{Claim, Consent, Downloader, Event, FreeSpace, Source, Sources};
use crate::error::ComponentError;
use crate::fake::{FakeAnswer, FakeFetcher};
use crate::fetch::{Fetcher, StandardFetcher};
use crate::loopback::{Mode, Served, Server};
use crate::pin::{ANY_TARGET, FilePin};
use crate::transport_tests::sample;

const OWN: &str = "https://own.invalid/base";
const PUBLIC: &str = "https://public.invalid/releases";

fn component(bytes: &[u8]) -> Component {
    Component::new(
        "sample",
        "Sample data",
        vec![FilePin::of_bytes("pack.bin", bytes)],
    )
}

fn own_address() -> String {
    format!("{OWN}/sample-0.1.0/pack.bin")
}

fn public_address() -> String {
    format!("{PUBLIC}/sample-0.1.0/pack.bin")
}

fn both() -> Sources {
    Sources {
        source: Some(Source::Releases(OWN.to_owned())),
        releases: Some(PUBLIC.to_owned()),
    }
}

/// Plenty of room, or a set amount.
struct Room(u64);

impl FreeSpace for Room {
    fn free_bytes(&self, _path: &Path) -> Option<u64> {
        Some(self.0)
    }
}

fn downloader(dir: &Path, sources: Sources, fetcher: Arc<dyn Fetcher>) -> Downloader {
    Downloader::new(dir.join("components"), sources, fetcher)
        .with_target(ANY_TARGET)
        .with_space(Arc::new(Room(u64::MAX)))
        .with_stall(Duration::from_millis(400))
}

#[derive(Default)]
struct Sink(Mutex<Vec<ProgressUpdate>>);

impl ProgressSink for Sink {
    fn report(&self, update: ProgressUpdate) {
        self.0.lock().unwrap().push(update);
    }
}

fn run(
    d: &Downloader,
    c: &Component,
) -> (Result<crate::download::Outcome, ComponentError>, Vec<Event>) {
    run_with(d, c, &OpContext::new())
}

fn run_with(
    d: &Downloader,
    c: &Component,
    ctx: &OpContext,
) -> (Result<crate::download::Outcome, ComponentError>, Vec<Event>) {
    let mut events = Vec::new();
    let result = d.download(c, &Consent::yes(c), ctx, &mut |e| events.push(e.clone()));
    (result, events)
}

fn staging(d: &Downloader, c: &Component) -> PathBuf {
    c.staging_in(d.components_dir())
}

fn part_len(d: &Downloader, c: &Component) -> Option<u64> {
    fs::metadata(staging(d, c).join("pack.bin.part"))
        .ok()
        .map(|m| m.len())
}

fn installed(d: &Downloader, c: &Component) -> bool {
    c.status_in(d.components_dir(), ANY_TARGET) == Status::Installed
        && c.verify_in(d.components_dir(), ANY_TARGET)
            .iter()
            .all(|f| f.state == crate::component::FileState::Good)
}

fn reason(result: &Result<crate::download::Outcome, ComponentError>) -> String {
    match result {
        Err(ComponentError::Fetch { reason, .. }) => reason.clone(),
        Err(other) => other.to_string(),
        Ok(_) => "ok".to_owned(),
    }
}

// ---------------------------------------------------------------------
// The fake fetcher: the rules that need no server.

#[test]
fn nothing_is_fetched_without_the_owners_yes() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let c = component(b"abc");
    let d = downloader(tmp.path(), both(), fake.clone());
    let mut other = c.clone();
    other.id = "other".into();
    let result = d.download(&c, &Consent::yes(&other), &OpContext::new(), &mut |_| {});
    assert_eq!(
        result.unwrap_err(),
        ComponentError::NotAsked("sample".into())
    );
    let mut newer = c.clone();
    newer.version = "0.2.0".into();
    assert!(
        d.download(&newer, &Consent::yes(&c), &OpContext::new(), &mut |_| {})
            .is_err()
    );
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());
}

#[test]
fn a_file_that_does_not_match_is_refused_and_the_next_source_tried() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let good = sample(3000);
    let mut bad = good.clone();
    bad[100] ^= 1;
    fake.serve(own_address(), FakeAnswer::Bytes(bad));
    fake.serve(public_address(), FakeAnswer::Bytes(good.clone()));
    let c = component(&good);
    let d = downloader(tmp.path(), both(), fake.clone());
    let (result, events) = run(&d, &c);
    let outcome = result.unwrap();
    assert_eq!(outcome.fetched, ["pack.bin"]);
    assert!(installed(&d, &c));
    assert_eq!(fake.requests().len(), 2);
    assert_eq!(part_len(&d, &c), None);
    assert!(
        !staging(&d, &c).exists(),
        "the staging folder goes when empty"
    );
    assert!(matches!(events.first(), Some(Event::Started(p)) if p.total == 3000));
    assert!(events.contains(&Event::Checking));
    // Both wrong: refused, nothing placed, nothing left in staging.
    fs::remove_file(c.dir_in(d.components_dir()).join("pack.bin")).unwrap();
    let mut worse = good.clone();
    worse[0] ^= 1;
    fake.serve(public_address(), FakeAnswer::Bytes(worse));
    let (result, _) = run(&d, &c);
    assert_eq!(
        result.unwrap_err(),
        ComponentError::Hash {
            file: "pack.bin".into()
        }
    );
    assert!(!c.dir_in(d.components_dir()).join("pack.bin").exists());
    assert_eq!(part_len(&d, &c), None);
}

#[test]
fn a_token_goes_to_the_owners_source_only() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(10);
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    let c = component(&bytes);
    let d = downloader(tmp.path(), both(), fake.clone()).with_token(Secret::new("t".into()));
    run(&d, &c).0.unwrap();
    let requests = fake.requests();
    assert_eq!(requests[0].address, own_address());
    assert!(requests[0].token);
    assert_eq!(requests[1].address, public_address());
    assert!(!requests[1].token);
}

#[test]
fn a_folder_placed_by_hand_is_adopted_and_nothing_is_fetched() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let c = component(b"abc");
    let d = downloader(tmp.path(), both(), fake.clone());
    let dir = c.dir_in(d.components_dir());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("pack.bin"), b"abc").unwrap();
    let outcome = run(&d, &c).0.unwrap();
    assert_eq!(outcome.kept, ["pack.bin"]);
    assert!(fake.requests().is_empty());
    assert_eq!(d.needed(&c), 0);
}

#[test]
fn a_missing_source_folder_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let c = component(b"abc");
    let sources = Sources {
        source: Some(Source::Folder(tmp.path().join("no-such-folder"))),
        releases: None,
    };
    let d = downloader(tmp.path(), sources, fake.clone());
    let (result, _) = run(&d, &c);
    assert_eq!(reason(&result), "the source folder is missing");
    assert!(fake.requests().is_empty());
}

#[test]
fn too_little_room_stops_before_a_byte_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let mut c = component(&sample(3000));
    c.action = crate::component::Action::Unpack;
    c.unpacked_size = 7000;
    let d = downloader(tmp.path(), both(), fake.clone()).with_space(Arc::new(Room(5000)));
    assert_eq!(d.needed(&c), 10_000);
    let (result, _) = run(&d, &c);
    assert_eq!(
        result.unwrap_err(),
        ComponentError::NoRoom {
            needed: 10_000,
            free: 5000
        }
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn a_folder_that_cannot_be_written_stops_before_a_byte_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fs::write(
        tmp.path().join("components"),
        b"a file where the folder goes",
    )
    .unwrap();
    let c = component(b"abc");
    let d = downloader(tmp.path(), both(), fake.clone());
    let (result, _) = run(&d, &c);
    assert!(
        matches!(result, Err(ComponentError::Io { .. })),
        "{result:?}"
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn two_downloads_of_one_component_do_not_run_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(100);
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    let c = component(&bytes);
    let d = downloader(tmp.path(), both(), fake.clone());
    let dest = c.dir_in(d.components_dir());
    // In this program.
    let held = Claim::take("sample", &dest, staging(&d, &c)).unwrap();
    assert_eq!(
        run(&d, &c).0.unwrap_err(),
        ComponentError::Busy("sample".into())
    );
    drop(held);
    // Another program's fresh lock.
    let lock = staging(&d, &c).join(LOCK_NAME);
    fs::create_dir_all(staging(&d, &c)).unwrap();
    fs::write(&lock, b"").unwrap();
    assert_eq!(
        run(&d, &c).0.unwrap_err(),
        ComponentError::Busy("sample".into())
    );
    assert!(fake.requests().is_empty());
    // A lock left by a program that stopped two minutes ago is taken over.
    let old = SystemTime::now() - Duration::from_secs(120);
    fs::File::options()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    run(&d, &c).0.unwrap();
    assert!(installed(&d, &c));
    assert!(!lock.exists());
}

#[test]
fn a_part_as_long_as_the_pin_is_hashed_first() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(2000);
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    let c = component(&bytes);
    let d = downloader(tmp.path(), both(), fake.clone());
    let st = staging(&d, &c);
    fs::create_dir_all(&st).unwrap();
    let note = format!(
        "address {}\nsize 2000\nsha256 {}\n",
        public_address(),
        c.files[0].sha256
    );
    fs::write(st.join("pack.bin.part"), &bytes).unwrap();
    fs::write(st.join("pack.bin.part.from"), &note).unwrap();
    run(&d, &c).0.unwrap();
    assert!(
        fake.requests().is_empty(),
        "a whole, matching .part needs no request"
    );
    assert!(installed(&d, &c));
    // A whole .part that does not match is removed and fetched from zero.
    fs::remove_file(c.dir_in(d.components_dir()).join("pack.bin")).unwrap();
    fs::create_dir_all(&st).unwrap();
    fs::write(st.join("pack.bin.part"), vec![0u8; 2000]).unwrap();
    fs::write(st.join("pack.bin.part.from"), &note).unwrap();
    run(&d, &c).0.unwrap();
    assert_eq!(fake.requests().len(), 2);
    assert!(fake.requests().iter().all(|r| r.from == 0));
    assert!(installed(&d, &c));
}

#[test]
fn a_part_for_another_pin_is_not_resumed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(2000);
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    let c = component(&bytes);
    let d = downloader(tmp.path(), both(), fake.clone());
    let st = staging(&d, &c);
    fs::create_dir_all(&st).unwrap();
    fs::write(st.join("pack.bin.part"), &bytes[..500]).unwrap();
    fs::write(
        st.join("pack.bin.part.from"),
        "address x\nsize 9\nsha256 00\n",
    )
    .unwrap();
    run(&d, &c).0.unwrap();
    assert!(
        fake.requests().iter().all(|r| r.from == 0),
        "{:?}",
        fake.requests()
    );
}

#[test]
fn a_cut_download_resumes_and_a_resume_that_does_not_match_starts_over_once() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(5000);
    fake.serve(public_address(), FakeAnswer::CutAfter(bytes.clone(), 2000));
    let c = component(&bytes);
    let d = downloader(
        tmp.path(),
        Sources {
            source: None,
            releases: Some(PUBLIC.into()),
        },
        fake.clone(),
    );
    let (result, _) = run(&d, &c);
    assert_eq!(reason(&result), "the connection was lost");
    assert_eq!(part_len(&d, &c), Some(2000));
    // The file changed on the server: the resumed file does not match, so
    // it starts over once from zero, and that does not match either.
    let mut changed = bytes.clone();
    changed[4000] ^= 1;
    fake.serve(public_address(), FakeAnswer::Bytes(changed));
    let (result, events) = run(&d, &c);
    assert_eq!(
        result.unwrap_err(),
        ComponentError::Hash {
            file: "pack.bin".into()
        }
    );
    assert!(events.iter().any(|e| matches!(e, Event::Retrying { .. })));
    let froms: Vec<u64> = fake.requests().iter().map(|r| r.from).collect();
    assert_eq!(froms, [0, 2000, 0]);
    assert_eq!(part_len(&d, &c), None);
    // Back as pinned: one transfer from zero.
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    run(&d, &c).0.unwrap();
    assert!(installed(&d, &c));
}

#[test]
fn progress_reaches_the_jobs_sink() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let bytes = sample(200_000);
    fake.serve(public_address(), FakeAnswer::Bytes(bytes.clone()));
    let c = component(&bytes);
    let d = downloader(tmp.path(), both(), fake);
    let sink = Arc::new(Sink::default());
    let ctx = OpContext::new().with_progress(sink.clone());
    let (result, events) = run_with(&d, &c, &ctx);
    result.unwrap();
    let updates = sink.0.lock().unwrap().clone();
    let last = updates.last().unwrap();
    assert_eq!(last.bytes_done, 200_000);
    assert_eq!(last.bytes_total, Some(200_000));
    assert_eq!(last.current.as_deref(), Some("pack.bin"));
    let percents: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Event::Progress(p) => Some(p.percent()),
            _ => None,
        })
        .collect();
    assert_eq!(percents.last(), Some(&100));
    assert!(percents.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn a_source_folder_on_this_computer_needs_no_network() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = sample(4000);
    let folder = tmp.path().join("source");
    fs::create_dir_all(folder.join("sample-0.1.0")).unwrap();
    fs::write(folder.join("sample-0.1.0").join("pack.bin"), &bytes).unwrap();
    let c = component(&bytes);
    let sources = Sources::offline().with_source(&folder.display().to_string());
    assert!(sources.is_offline());
    let d = downloader(tmp.path(), sources, Arc::new(StandardFetcher::local_only()));
    run(&d, &c).0.unwrap();
    assert!(installed(&d, &c));
}

#[test]
fn the_setting_reads_as_a_folder_or_a_repository() {
    assert_eq!(Source::from_setting("  "), None);
    assert_eq!(
        Source::from_setting("example-org/parts"),
        Some(Source::Releases(
            "https://github.com/example-org/parts/releases/download".into()
        ))
    );
    assert!(matches!(
        Source::from_setting("a/b/c"),
        Some(Source::Folder(_))
    ));
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().display().to_string();
    assert_eq!(
        Source::from_setting(&dir),
        Some(Source::Folder(tmp.path().to_path_buf()))
    );
    let c = component(b"abc");
    let public = Sources::public();
    let addresses = public.addresses(&c, &c.files[0]);
    assert_eq!(addresses.len(), 1);
    assert!(addresses[0].url.starts_with("https://"));
    assert!(
        addresses[0]
            .url
            .ends_with("/releases/download/sample-0.1.0/pack.bin")
    );
    assert!(!addresses[0].own_source);
}

#[test]
fn the_volume_says_its_free_space() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        crate::download::SystemSpace
            .free_bytes(&tmp.path().join("not-yet"))
            .is_some()
    );
}

// ---------------------------------------------------------------------
// The loopback server's modes (spike S3), with each real transport.

/// The transports to run the server's modes with.
fn transports() -> Vec<(&'static str, Arc<dyn Fetcher>)> {
    let mut list: Vec<(&'static str, Arc<dyn Fetcher>)> = Vec::new();
    #[cfg(feature = "fetch")]
    list.push((
        "built-in",
        Arc::new(crate::http::HttpFetcher::for_loopback()),
    ));
    if let Ok(ToolStatus::Found(curl)) = ToolRegistry::builtin().find("curl") {
        list.push((
            "curl",
            Arc::new(crate::curl::CurlFetcher::for_loopback(curl)),
        ));
    }
    list
}

const PATH: &str = "rel/sample-0.1.0/pack.bin";

/// A downloader for the server. The stall time is generous: a curl
/// started while other tests run can take a second to connect.
fn on_server(tmp: &Path, server: &Server, fetcher: Arc<dyn Fetcher>) -> Downloader {
    let sources = Sources {
        source: None,
        releases: Some(server.url("rel")),
    };
    downloader(tmp, sources, fetcher).with_stall(Duration::from_secs(10))
}

fn step(bytes: &[u8], mode: Mode) -> Served {
    Served {
        bytes: bytes.to_vec(),
        mode,
    }
}

/// The bytes the server sent bodies of, for the cost of a mode.
fn requests(server: &Server) -> Vec<Option<String>> {
    server.seen().into_iter().map(|s| s.range).collect()
}

#[test]
fn s3_a_server_that_honors_the_range_resumes() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        server.serve_steps(
            PATH,
            vec![
                step(&bytes, Mode::Truncate(20_000)),
                step(&bytes, Mode::Honor),
            ],
        );
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher);
        let (result, _) = run(&d, &c);
        assert_eq!(reason(&result), "the connection was lost", "{name}");
        assert_eq!(part_len(&d, &c), Some(20_000), "{name}");
        run(&d, &c).0.unwrap();
        assert!(installed(&d, &c), "{name}");
        assert_eq!(
            requests(&server),
            [None, Some("bytes=20000-".into())],
            "{name}"
        );
    }
}

#[test]
fn s3_a_server_that_ignores_the_range_costs_one_whole_transfer() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        server.serve_steps(
            PATH,
            vec![
                step(&bytes, Mode::Truncate(20_000)),
                step(&bytes, Mode::IgnoreRange),
            ],
        );
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher);
        let _ = run(&d, &c);
        run(&d, &c).0.unwrap();
        assert!(installed(&d, &c), "{name}");
        let seen = requests(&server);
        // The built-in client writes the whole file it is given in answer
        // to its range request; curl stops (code 33) and the downloader
        // asks again from zero. Either way, one whole transfer more.
        let expected: &[Option<String>] = if name == "curl" {
            &[None, Some("bytes=20000-".into()), None]
        } else {
            &[None, Some("bytes=20000-".into())]
        };
        assert_eq!(seen, expected, "{name}");
    }
}

#[test]
fn s3_a_wrong_content_range_starts_over_from_zero() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        server.serve_steps(
            PATH,
            vec![
                step(&bytes, Mode::Truncate(20_000)),
                step(&bytes, Mode::WrongRange),
            ],
        );
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher);
        let _ = run(&d, &c);
        let (result, events) = run(&d, &c);
        result.unwrap();
        assert!(installed(&d, &c), "{name}");
        assert!(
            events.iter().any(|e| matches!(e, Event::Retrying { .. })),
            "{name}"
        );
        assert_eq!(
            requests(&server),
            [None, Some("bytes=20000-".into()), None],
            "{name}"
        );
    }
}

#[test]
fn s3_a_file_that_changed_between_attempts_is_never_installed() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        let mut changed = bytes.clone();
        changed[30_000] ^= 1;
        server.serve_steps(
            PATH,
            vec![
                step(&bytes, Mode::Truncate(20_000)),
                step(&changed, Mode::Honor),
            ],
        );
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher);
        let _ = run(&d, &c);
        let (result, _) = run(&d, &c);
        assert_eq!(
            result.unwrap_err(),
            ComponentError::Hash {
                file: "pack.bin".into()
            },
            "{name}"
        );
        assert_eq!(
            c.status_in(d.components_dir(), ANY_TARGET),
            Status::NotInstalled,
            "{name}"
        );
        assert_eq!(part_len(&d, &c), None, "{name}");
        // The worst cost: the cut transfer, then one more whole one (the
        // built-in client's If-Range brings the whole file at once; curl
        // resumes, fails the hash, and starts over once).
        let whole = server.seen().iter().filter(|s| s.range.is_none()).count();
        assert!(whole <= 2, "{name}: {:?}", requests(&server));
    }
}

#[test]
fn s3_a_cut_body_and_a_silent_server_keep_what_arrived() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        server.serve(PATH, &bytes, Mode::Stall(10_000));
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher).with_stall(Duration::from_secs(2));
        let started = Instant::now();
        let (result, _) = run(&d, &c);
        let words = reason(&result);
        assert_eq!(words, "the server took too long", "{name}");
        assert!(started.elapsed() < Duration::from_secs(10), "{name}");
        // What reached the downloader is kept; curl may still hold a few
        // bytes it had not handed on when it was stopped.
        let kept = part_len(&d, &c).unwrap_or(0);
        assert!(kept > 0 && kept <= 10_000, "{name}: {kept}");
    }
}

#[test]
fn s3_a_body_longer_than_the_pin_is_refused_and_removed() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(50_000);
        server.serve(PATH, &bytes, Mode::Longer(vec![7; 1000]));
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher);
        let (result, _) = run(&d, &c);
        assert!(
            matches!(result, Err(ComponentError::Size { .. })),
            "{name}: {result:?}"
        );
        assert_eq!(part_len(&d, &c), None, "{name}");
        assert_eq!(
            c.status_in(d.components_dir(), ANY_TARGET),
            Status::NotInstalled,
            "{name}"
        );
    }
}

#[test]
fn s3_escape_stops_at_once_and_the_next_try_resumes_to_the_pin() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(80_000);
        server.serve_steps(
            PATH,
            vec![step(&bytes, Mode::Stall(30_000)), step(&bytes, Mode::Honor)],
        );
        let c = component(&bytes);
        let d = on_server(tmp.path(), &server, fetcher).with_stall(Duration::from_secs(30));
        let cancel = CancelToken::new();
        let ctx = OpContext::new().with_cancel(cancel.clone());
        let mut stopped_at = None;
        let result = d.download(&c, &Consent::yes(&c), &ctx, &mut |e| {
            if let Event::Progress(p) = e
                && p.done >= 30_000
                && stopped_at.is_none()
            {
                stopped_at = Some(Instant::now());
                cancel.cancel();
            }
        });
        assert_eq!(result.unwrap_err(), ComponentError::Cancelled, "{name}");
        let took = stopped_at.unwrap().elapsed();
        assert!(took < Duration::from_secs(2), "{name}: {took:?}");
        assert_eq!(part_len(&d, &c), Some(30_000), "{name}");
        run(&d, &c).0.unwrap();
        assert!(installed(&d, &c), "{name}");
        assert_eq!(requests(&server)[1], Some("bytes=30000-".into()), "{name}");
    }
}

#[test]
fn s3_not_found_tries_the_next_address() {
    for (name, fetcher) in transports() {
        let tmp = tempfile::tempdir().unwrap();
        let server = Server::start();
        let bytes = sample(1000);
        server.serve("public/sample-0.1.0/pack.bin", &bytes, Mode::Honor);
        let c = component(&bytes);
        let sources = Sources {
            source: Some(Source::Releases(server.url("own"))),
            releases: Some(server.url("public")),
        };
        let d = downloader(tmp.path(), sources, fetcher).with_stall(Duration::from_secs(10));
        run(&d, &c).0.unwrap();
        assert!(installed(&d, &c), "{name}");
        assert_eq!(server.seen().len(), 2, "{name}");
    }
}
