//! copy-data copy, and statvfs for space and volumes (R25 commit 62): a
//! copy between two sites never uses copy-data, a copy on one server
//! keeps the write session's order, the free-space arithmetic, and a
//! server without statvfs said in words, never zero.

use diro_sftp::sftp::fake::Faults;
use serde_json::{Value, json};

use crate::connect::{connect, fake_host};
use crate::host::{TestHost, kind};

fn hint(host: &TestHost, from: &str, to: &str) -> Value {
    host.call(
        "provider/copy_hint",
        json!({"from": from, "to": to, "move": false}),
    )
    .unwrap()
}

#[test]
fn server_side_only_on_one_connection_that_copies() {
    let (host, _opener, _root) = fake_host(Faults {
        copy_data: true,
        ..Faults::default()
    });
    let a = "sftp://build.example/srv/a.txt";
    // Not connected: nothing is asked, and it streams.
    let before = hint(&host, a, "sftp://build.example/srv/b.txt");
    assert_eq!(before["mode"], "stream");
    assert_eq!(before["why"], "Not connected: copied through here.");
    connect(&host, "sftp://build.example/").unwrap();
    connect(&host, "sftp://other.example/").unwrap();
    connect(&host, "sftp://ada@build.example/").unwrap();
    let same = hint(&host, a, "sftp://build.example/srv/b.txt");
    assert_eq!(same["mode"], "server_side");
    assert_eq!(same["why"], "Copied on the server.");
    // Two sites, or two accounts on one host: through here.
    for to in [
        "sftp://other.example/srv/b.txt",
        "sftp://ada@build.example/srv/b.txt",
    ] {
        let other = hint(&host, a, to);
        assert_eq!(other["mode"], "stream", "{to}");
        assert_eq!(other["why"], "Another server: copied through here.");
    }
    let refused = host
        .call(
            "provider/copy",
            json!({"from": a, "to": "sftp://other.example/srv/b.txt", "overwrite": false}),
        )
        .unwrap_err();
    assert_eq!(kind(&refused), "unsupported");
}

#[test]
fn a_server_without_copy_data_streams() {
    let (host, _opener, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    let answer = hint(
        &host,
        "sftp://build.example/srv/a.txt",
        "sftp://build.example/srv/b.txt",
    );
    assert_eq!(answer["mode"], "stream");
    assert_eq!(answer["why"], "This server cannot copy on its own.");
    let refused = host
        .call(
            "provider/copy",
            json!({"from": "sftp://build.example/srv/a.txt",
                "to": "sftp://build.example/srv/b.txt", "overwrite": false}),
        )
        .unwrap_err();
    assert_eq!(kind(&refused), "unsupported");
}

#[test]
fn a_copy_on_the_server_keeps_the_write_order_and_the_attributes() {
    let (host, opener, _root) = fake_host(Faults {
        copy_data: true,
        ..Faults::default()
    });
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    {
        let mut fs = opener.fs().lock().unwrap();
        fs.add_file(b"/srv/a.bin", &data);
        fs.add_file(b"/srv/taken.bin", b"old");
        fs.add_dir(b"/srv/dir");
    }
    connect(&host, "sftp://build.example/").unwrap();
    host.call(
        "provider/set_attributes",
        json!({"place": "sftp://build.example/srv/a.bin", "mode": 0o640,
            "modified": "2023-11-14T22:13:20Z"}),
    )
    .unwrap();
    let copy = |to: &str, overwrite: bool| {
        host.call(
            "provider/copy",
            json!({"from": "sftp://build.example/srv/a.bin",
                "to": format!("sftp://build.example/srv/{to}"),
                "overwrite": overwrite, "preserve": true}),
        )
    };
    let made = copy("b.bin", false).unwrap();
    assert_eq!(made["size"], data.len());
    assert_eq!(made["mode"], 0o640);
    assert_eq!(made["modified"], "2023-11-14T22:13:20Z");
    assert_eq!(
        opener.fs().lock().unwrap().file(b"/srv/b.bin"),
        Some(&data[..])
    );
    assert_eq!(
        kind(&copy("taken.bin", false).unwrap_err()),
        "already_exists"
    );
    assert_eq!(kind(&copy("dir", true).unwrap_err()), "is_a_directory");
    copy("taken.bin", true).unwrap();
    assert_eq!(
        opener.fs().lock().unwrap().file(b"/srv/taken.bin"),
        Some(&data[..])
    );
    let folder = host
        .call(
            "provider/copy",
            json!({"from": "sftp://build.example/srv/dir",
                "to": "sftp://build.example/srv/dir2", "overwrite": false}),
        )
        .unwrap_err();
    assert_eq!(kind(&folder), "unsupported");
    let fs = opener.fs().lock().unwrap();
    assert!(
        fs.paths()
            .iter()
            .all(|p| !p.starts_with(b"/srv/.diro-part-")),
        "no unfinished file is left"
    );
    assert!(fs.calls.contains(&"Extended"));
}

#[test]
fn volumes_answer_free_space_from_statvfs() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/srv/a.bin", &vec![0u8; 3 * 4096]);
    connect(&host, "sftp://build.example/").unwrap();
    let answer = host
        .call(
            "provider/volumes",
            json!({"place": "sftp://build.example/srv/not/made/yet"}),
        )
        .unwrap();
    let row = &answer["volumes"][0];
    assert_eq!(row["name"], "build.example");
    assert_eq!(row["place"], "sftp://build.example/");
    // The fake: 1,000,000 blocks of 4096; three used; 1,000 kept back.
    assert_eq!(row["total"], 4_096_000_000u64);
    assert_eq!(row["free"], (1_000_000u64 - 3 - 1000) * 4096);
    assert!(answer.get("note").is_none());
    let home = host
        .call(
            "provider/volumes",
            json!({"authority": "sftp://build.example"}),
        )
        .unwrap();
    assert_eq!(home["volumes"][0]["name"], "Home on build.example");
    assert_eq!(home["volumes"][0]["place"], "sftp://build.example/home/ada");
}

#[test]
fn a_server_without_statvfs_says_so_and_never_zero() {
    let (host, _opener, _root) = fake_host(Faults {
        no_statvfs: true,
        ..Faults::default()
    });
    connect(&host, "sftp://build.example/").unwrap();
    let answer = host
        .call(
            "provider/volumes",
            json!({"place": "sftp://build.example/srv"}),
        )
        .unwrap();
    let row = &answer["volumes"][0];
    assert!(row.get("free").is_none());
    assert!(row.get("total").is_none());
    assert_eq!(answer["note"], "This server does not report free space.");
}
