//! Streamed list and stat (R25 commit 58): a 5,000-entry folder in
//! batches of 1,000, hostile names dropped and counted, names that are
//! not UTF-8 as bytes, links shown and never followed, and ssh killed in
//! the middle of a listing.

use serde_json::{Value, json};

use diro_sftp::sftp::fake::Faults;
use diroxide_plugins::base64;

use crate::connect::{connect, fake_host, ssh_host};
use crate::host::kind;

fn names(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_large_folder_streams_in_batches_of_1000() {
    let (host, opener, _root) = fake_host(Faults::default());
    {
        let mut fs = opener.fs().lock().unwrap();
        for i in 0..5000 {
            fs.add_file(format!("/home/ada/big/f{i:05}").as_bytes(), b"x");
        }
    }
    connect(&host, "sftp://build.example/").unwrap();
    let answer = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada/big", "partial_token": "p9"}),
        )
        .unwrap();
    let partials: Vec<Value> = host
        .notices("$/progress")
        .into_iter()
        .filter(|p| p["token"] == "p9")
        .collect();
    assert_eq!(partials.len(), 5);
    let mut all = Vec::new();
    for batch in &partials {
        assert_eq!(batch["value"]["kind"], "partial");
        let rows = batch["value"]["entries"].as_array().unwrap();
        assert_eq!(rows.len(), 1000);
        all.extend(names(rows));
    }
    all.extend(names(answer["entries"].as_array().unwrap()));
    assert_eq!(answer["complete"], true);
    assert_eq!(all.len(), 5000);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 5000);
    // Without a token, one answer holds them all.
    let whole = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada/big"}),
        )
        .unwrap();
    assert_eq!(whole["entries"].as_array().unwrap().len(), 5000);
}

#[test]
fn hostile_names_are_dropped_and_counted_and_raw_names_kept() {
    let (host, opener, _root) = fake_host(Faults {
        hostile_names: true,
        ..Faults::default()
    });
    {
        let mut fs = opener.fs().lock().unwrap();
        fs.add_file(b"/home/ada/d/a.txt", b"hello");
        fs.add_file(b"/home/ada/d/caf\xe9.txt", b"x");
        fs.add_file(b"/home/ada/d/.diro-part-0a1b2c3d", b"partial");
        fs.add_dir(b"/home/ada/d/sub");
        fs.add_link(b"/home/ada/d/up", b"../../../etc");
    }
    connect(&host, "sftp://build.example/").unwrap();
    let answer = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada/d"}),
        )
        .unwrap();
    let entries = answer["entries"].as_array().unwrap().clone();
    let mut shown = names(&entries);
    shown.sort();
    assert_eq!(shown, ["a.txt", "caf\u{FFFD}.txt", "sub", "up"]);
    // Two dot names past the first of each, a slash twice, a NUL, empty.
    assert_eq!(answer["skipped"], 6);
    let said = host.notices("ui/announce");
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["text"], "6 names skipped: unsafe.");
    assert_eq!(said[0]["importance"], "status");
    let raw = entries
        .iter()
        .find(|e| e["name"] == "caf\u{FFFD}.txt")
        .unwrap();
    assert_eq!(
        base64::decode(raw["name_raw"].as_str().unwrap()).unwrap(),
        b"caf\xe9.txt"
    );
    let sub = entries.iter().find(|e| e["name"] == "sub").unwrap();
    assert_eq!(sub["kind"], "folder");
    let up = entries.iter().find(|e| e["name"] == "up").unwrap();
    assert_eq!(up["kind"], "link");
    assert_eq!(up["link_target"], "../../../etc");
    let file = entries.iter().find(|e| e["name"] == "a.txt").unwrap();
    assert_eq!(file["size"], 5);
    assert_eq!(file["mode"], 0o644);
    assert!(file.get("name_raw").is_none());
}

#[test]
fn stat_reads_files_folders_and_links_itself() {
    let (host, opener, _root) = fake_host(Faults::default());
    {
        let mut fs = opener.fs().lock().unwrap();
        fs.add_file(b"/srv/report.txt", b"twelve bytes");
        fs.add_link(b"/srv/latest", b"report.txt");
    }
    connect(&host, "sftp://build.example/").unwrap();
    let stat = |place: &str, follow: bool| {
        host.call(
            "provider/stat",
            json!({"place": place, "follow_links": follow}),
        )
    };
    let file = stat("sftp://build.example/srv/report.txt", false).unwrap();
    assert_eq!(file["name"], "report.txt");
    assert_eq!(file["kind"], "file");
    assert_eq!(file["size"], 12);
    assert_eq!(file["place"], "sftp://build.example/srv/report.txt");
    let folder = stat("sftp://build.example/srv", false).unwrap();
    assert_eq!(folder["kind"], "folder");
    let link = stat("sftp://build.example/srv/latest", false).unwrap();
    assert_eq!(link["kind"], "link");
    assert_eq!(link["link_target"], "report.txt");
    let followed = stat("sftp://build.example/srv/latest", true).unwrap();
    assert_eq!(followed["kind"], "file");
    assert_eq!(stat("sftp://build.example/", false).unwrap()["name"], "/");
    let missing = stat("sftp://build.example/srv/none", false).unwrap_err();
    assert_eq!(kind(&missing), "not_found");
    let not_folder = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/srv/report.txt"}),
        )
        .unwrap_err();
    assert_eq!(kind(&not_folder), "not_a_directory");
}

#[test]
fn ssh_killed_mid_listing_ends_the_listing_and_is_said_once() {
    let files: serde_json::Map<String, Value> = (0..150)
        .map(|i| (format!("/home/ada/d/f{i:03}"), json!("x")))
        .collect();
    // Answers stop after the fourth request (limits, the home folder,
    // OPENDIR, the first READDIR), and ssh dies a little later.
    let (host, _log, _root) = ssh_host(
        json!([
            {"ask": "ada@build.example's password: ", "accept": "s3cret-test"},
            {"serve": {"files": files, "faults": {"stall_after": 4}, "for_ms": 2500}}
        ]),
        vec![json!({"text": "s3cret-test"})],
    );
    connect(&host, "sftp://build.example/").unwrap();
    let failed = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada/d"}),
        )
        .unwrap_err();
    assert_eq!(kind(&failed), "unavailable", "{failed}");
    assert_eq!(failed["message"], "Connection lost: build.example.");
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(host.notices("provider/lost").len(), 1);
    let after = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada/d"}),
        )
        .unwrap_err();
    assert_eq!(kind(&after), "auth_required");
}
