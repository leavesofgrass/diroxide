//! The write session (R25 commit 60): the engine's own unfinished name,
//! every WRITE answer checked, fsync, the size compared, the CLOSE status
//! checked, posix-rename over the target, the aside road without it,
//! resume; and the fake server's lying writes, a failure at CLOSE, a
//! failed WRITE, a dropped link and a killed plugin.

use std::sync::{Arc, Mutex};

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::fake::FakeOpener;
use diro_sftp::sftp::fake::{FakeFs, Faults, Node};
use diroxide_plugins::base64;
use serde_json::{Value, json};

use crate::connect::{connect, fake_host};
use crate::host::{TestHost, kind};

const MIB: usize = 1024 * 1024;
const TARGET: &str = "sftp://build.example/home/ada/out.bin";

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 % 247) as u8).collect()
}

fn begin(
    host: &TestHost,
    place: &str,
    conflict: &str,
    resume: Option<u64>,
) -> Result<Value, Value> {
    let mut params = json!({"place": place, "conflict": conflict, "transfer": "inline"});
    if let Some(offset) = resume {
        params["resume"] = json!({"offset": offset});
    }
    host.call("provider/write_begin", params)
}

fn chunk(host: &TestHost, id: &Value, offset: usize, bytes: &[u8]) -> Result<Value, Value> {
    host.call(
        "provider/write_chunk",
        json!({"write_id": id, "offset": offset, "data": base64::encode(bytes), "final": false}),
    )
}

fn end(host: &TestHost, id: &Value, commit: bool) -> Result<Value, Value> {
    host.call(
        "provider/write_end",
        json!({"write_id": id, "commit": commit}),
    )
}

fn send_all(host: &TestHost, id: &Value, from: usize, data: &[u8]) {
    let mut at = from;
    while at < data.len() {
        let next = (at + MIB).min(data.len());
        chunk(host, id, at, &data[at..next]).unwrap();
        at = next;
    }
}

/// The unfinished files in a folder of the fake.
fn unfinished(fs: &Arc<Mutex<FakeFs>>, folder: &str) -> Vec<Vec<u8>> {
    let fs = fs.lock().unwrap();
    let prefix = format!("{folder}/.diro-part-");
    fs.paths()
        .into_iter()
        .filter(|p| p.starts_with(prefix.as_bytes()))
        .collect()
}

#[test]
fn a_write_commits_through_the_engines_unfinished_file() {
    let (host, opener, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    let data = pattern(2 * MIB + 77);
    let begun = begin(&host, TARGET, "fail", None).unwrap();
    assert!(begun.get("offset").is_none());
    let unfinished_place = begun["unfinished"].as_str().unwrap();
    assert!(
        unfinished_place.starts_with("sftp://build.example/home/ada/.diro-part-"),
        "{unfinished_place}"
    );
    let id = &begun["write_id"];
    chunk(&host, id, 0, &data[..MIB]).unwrap();
    // While it is written, the target is not there; the partial file is.
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
    assert_eq!(unfinished(opener.fs(), "/home/ada").len(), 1);
    send_all(&host, id, MIB, &data);
    let entry = end(&host, id, true).unwrap();
    assert_eq!(entry["name"], "out.bin");
    assert_eq!(entry["size"], data.len());
    assert_eq!(
        opener.fs().lock().unwrap().file(b"/home/ada/out.bin"),
        Some(&data[..])
    );
    assert!(unfinished(opener.fs(), "/home/ada").is_empty());
    let calls = opener.fs().lock().unwrap().calls.clone();
    assert!(calls.contains(&"Fstat"), "the size is compared");
    assert!(calls.contains(&"Rename"), "a new name is plain RENAME");
}

#[test]
fn overwrite_replaces_in_one_step_or_by_the_aside_road() {
    for faults in [
        Faults::default(),
        Faults {
            no_posix_rename: true,
            ..Faults::default()
        },
    ] {
        let (host, opener, _root) = fake_host(faults.clone());
        opener
            .fs()
            .lock()
            .unwrap()
            .add_file(b"/home/ada/out.bin", b"the old contents");
        connect(&host, "sftp://build.example/").unwrap();
        let taken = begin(&host, TARGET, "fail", None).unwrap_err();
        assert_eq!(kind(&taken), "already_exists");
        let id = begin(&host, TARGET, "overwrite", None).unwrap()["write_id"].clone();
        chunk(&host, &id, 0, b"new").unwrap();
        end(&host, &id, true).unwrap();
        assert_eq!(
            opener.fs().lock().unwrap().file(b"/home/ada/out.bin"),
            Some(&b"new"[..]),
            "{faults:?}"
        );
        assert!(
            unfinished(opener.fs(), "/home/ada").is_empty(),
            "{faults:?}"
        );
    }
}

#[test]
fn begin_checks_the_target_and_its_folder() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.fs().lock().unwrap().add_dir(b"/home/ada/docs");
    opener.fs().lock().unwrap().add_file(b"/home/ada/f", b"x");
    connect(&host, "sftp://build.example/").unwrap();
    let folder = begin(
        &host,
        "sftp://build.example/home/ada/docs",
        "overwrite",
        None,
    )
    .unwrap_err();
    assert_eq!(kind(&folder), "is_a_directory");
    let no_parent = begin(&host, "sftp://build.example/home/none/x", "fail", None).unwrap_err();
    assert_eq!(kind(&no_parent), "not_found");
    let file_parent = begin(&host, "sftp://build.example/home/ada/f/x", "fail", None).unwrap_err();
    assert_eq!(kind(&file_parent), "not_a_directory");
    let root = begin(&host, "sftp://build.example/", "fail", None).unwrap_err();
    assert_eq!(kind(&root), "invalid_name");
    let first = begin(&host, TARGET, "fail", None).unwrap();
    let second = begin(&host, TARGET, "fail", None).unwrap_err();
    assert_eq!(kind(&second), "conflict");
    let id = &first["write_id"];
    let out_of_order = chunk(&host, id, 5, b"x").unwrap_err();
    assert_eq!(kind(&out_of_order), "conflict");
    chunk(&host, id, 0, b"abc").unwrap();
    end(&host, id, false).unwrap();
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
    assert!(unfinished(opener.fs(), "/home/ada").is_empty());
    let gone = chunk(&host, id, 3, b"x").unwrap_err();
    assert_eq!(kind(&gone), "unavailable");
}

#[test]
fn a_server_that_keeps_fewer_bytes_is_caught_by_the_size() {
    let (host, opener, _root) = fake_host(Faults {
        short_writes: Some(1000),
        ..Faults::default()
    });
    connect(&host, "sftp://build.example/").unwrap();
    let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
    // Each WRITE of 32 KiB keeps 1,000 bytes and still answers OK.
    chunk(&host, &id, 0, &pattern(100_000)).unwrap();
    let refused = end(&host, &id, true).unwrap_err();
    assert_eq!(kind(&refused), "failed");
    assert_eq!(refused["message"], "Size differs after writing.");
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
    assert!(unfinished(opener.fs(), "/home/ada").is_empty());
}

#[test]
fn a_failure_at_close_is_no_commit() {
    let (host, opener, _root) = fake_host(Faults {
        fail_close: true,
        ..Faults::default()
    });
    connect(&host, "sftp://build.example/").unwrap();
    let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
    chunk(&host, &id, 0, b"all the bytes").unwrap();
    let refused = end(&host, &id, true).unwrap_err();
    assert_ne!(refused["message"], Value::Null);
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
}

#[test]
fn a_failed_write_in_the_middle_is_said() {
    let (host, opener, _root) = fake_host(Faults {
        fail_write_at: Some(3),
        ..Faults::default()
    });
    connect(&host, "sftp://build.example/").unwrap();
    let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
    // Ten WRITEs in flight; the third fails, later ones still succeed.
    let failed = chunk(&host, &id, 0, &pattern(10 * 32 * 1024)).unwrap_err();
    assert_ne!(kind(&failed), "");
    end(&host, &id, false).unwrap();
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
    assert!(unfinished(opener.fs(), "/home/ada").is_empty());
}

#[test]
fn a_dropped_link_leaves_the_unfinished_file_and_resume_goes_on() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.as_if_typed(true);
    connect(&host, "sftp://build.example/").unwrap();
    let data = pattern(3 * MIB);
    let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
    chunk(&host, &id, 0, &data[..MIB]).unwrap();
    opener.cut_all();
    let lost = chunk(&host, &id, MIB, &data[MIB..2 * MIB]).unwrap_err();
    assert_eq!(kind(&lost), "unavailable");
    assert!(
        opener
            .fs()
            .lock()
            .unwrap()
            .node(b"/home/ada/out.bin")
            .is_none()
    );
    // The server may hold more than the plugin saw answered: writes in
    // flight when the link dropped. The resume cuts it to the host's
    // offset.
    let partials = unfinished(opener.fs(), "/home/ada");
    assert_eq!(partials.len(), 1);
    let held = opener
        .fs()
        .lock()
        .unwrap()
        .file(&partials[0])
        .map_or(0, <[u8]>::len);
    assert!(held >= MIB, "{held}");
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    // The host connects again and resumes from what it believes is there.
    connect(&host, "sftp://build.example/").unwrap();
    let resumed = begin(&host, TARGET, "fail", Some(MIB as u64)).unwrap();
    assert_eq!(resumed["offset"], MIB);
    let id = resumed["write_id"].clone();
    send_all(&host, &id, MIB, &data);
    end(&host, &id, true).unwrap();
    assert_eq!(
        opener.fs().lock().unwrap().file(b"/home/ada/out.bin"),
        Some(&data[..])
    );
    assert!(unfinished(opener.fs(), "/home/ada").is_empty());
}

#[test]
fn resume_answers_the_partial_files_length_cut_or_zero() {
    let (host, opener, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    // No partial file: it starts over.
    let fresh = begin(&host, TARGET, "fail", Some(500)).unwrap();
    assert_eq!(fresh["offset"], 0);
    let id = fresh["write_id"].clone();
    chunk(&host, &id, 0, &pattern(1000)).unwrap();
    host.notify(
        "provider/disconnect",
        json!({"place": "sftp://build.example/"}),
    );
    assert!(host.wait_for(|h| h.plugin().provider().connected().is_empty()));
    connect(&host, "sftp://build.example/").unwrap();
    // The host believes more is there than the partial holds.
    let shorter = begin(&host, TARGET, "fail", Some(5000)).unwrap();
    assert_eq!(shorter["offset"], 1000);
    end(&host, &shorter["write_id"], false).unwrap();
    // Abandoned: removed. A partial longer than the host's offset is cut.
    let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
    chunk(&host, &id, 0, &pattern(1000)).unwrap();
    host.notify(
        "provider/disconnect",
        json!({"place": "sftp://build.example/"}),
    );
    assert!(host.wait_for(|h| h.plugin().provider().connected().is_empty()));
    connect(&host, "sftp://build.example/").unwrap();
    let cut = begin(&host, TARGET, "fail", Some(400)).unwrap();
    assert_eq!(cut["offset"], 400);
    let partials = unfinished(opener.fs(), "/home/ada");
    assert_eq!(
        opener
            .fs()
            .lock()
            .unwrap()
            .file(&partials[0])
            .map(<[u8]>::len),
        Some(400)
    );
    let id = cut["write_id"].clone();
    chunk(&host, &id, 400, &pattern(1000)[400..]).unwrap();
    end(&host, &id, true).unwrap();
    assert_eq!(
        opener.fs().lock().unwrap().file(b"/home/ada/out.bin"),
        Some(&pattern(1000)[..])
    );
}

#[test]
fn a_killed_plugin_leaves_only_the_unfinished_file() {
    let fs = Arc::new(Mutex::new(FakeFs::default()));
    let root = tempfile::tempdir().unwrap();
    let data = pattern(2 * MIB);
    {
        let opener = FakeOpener::new(Arc::clone(&fs), Faults::default());
        let host = TestHost::start(
            Plugin::with_opener(opener),
            root.path(),
            json!({}),
            Vec::new(),
        );
        connect(&host, "sftp://build.example/").unwrap();
        let id = begin(&host, TARGET, "fail", None).unwrap()["write_id"].clone();
        chunk(&host, &id, 0, &data[..MIB]).unwrap();
        // The plugin goes away with no write_end.
    }
    assert!(fs.lock().unwrap().node(b"/home/ada/out.bin").is_none());
    let partials = unfinished(&fs, "/home/ada");
    assert_eq!(partials.len(), 1);
    assert!(matches!(
        fs.lock().unwrap().node(&partials[0]),
        Some(Node::File(bytes)) if bytes.len() == MIB
    ));
    // A new plugin resumes it.
    let opener = FakeOpener::new(Arc::clone(&fs), Faults::default());
    let again = tempfile::tempdir().unwrap();
    let host = TestHost::start(
        Plugin::with_opener(opener),
        again.path(),
        json!({}),
        Vec::new(),
    );
    connect(&host, "sftp://build.example/").unwrap();
    let resumed = begin(&host, TARGET, "fail", Some(MIB as u64)).unwrap();
    assert_eq!(resumed["offset"], MIB);
    send_all(&host, &resumed["write_id"], MIB, &data);
    end(&host, &resumed["write_id"], true).unwrap();
    assert_eq!(
        fs.lock().unwrap().file(b"/home/ada/out.bin"),
        Some(&data[..])
    );
}
