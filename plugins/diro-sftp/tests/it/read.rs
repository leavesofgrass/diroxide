//! Ranged read with the spool and the handle cache (R25 commit 59), and
//! the transfer session of answer 87: ranges in the middle and at the
//! end, a range past the end, the spool kept to its folder, one handle
//! for many chunks, a second session for transfers that asks nothing.

use std::time::Duration;

use diro_sftp::sftp::fake::Faults;
use diro_sftp::ssh::Target;
use diroxide_plugins::base64;
use serde_json::{Value, json};

use crate::connect::{connect, fake_host, ssh_host};
use crate::fakes::log_lines;
use crate::host::{TestHost, kind};

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn read(host: &TestHost, place: &str, offset: u64, length: u64) -> (Vec<u8>, bool) {
    let answer = host
        .call(
            "provider/read",
            json!({"place": place, "offset": offset, "length": length, "transfer": "inline"}),
        )
        .unwrap();
    let data = base64::decode(answer["data"].as_str().unwrap()).unwrap();
    assert_eq!(answer["length"], data.len());
    (data, answer["eof"].as_bool().unwrap())
}

fn connection(host: &TestHost) -> std::sync::Arc<diro_sftp::provider::Connection> {
    host.plugin()
        .provider()
        .connection(&Target::new("build.example", None, None).unwrap())
        .unwrap()
}

const BIG: &str = "sftp://build.example/home/ada/big.bin";

#[test]
fn ranges_in_the_middle_at_the_end_and_past_it() {
    let (host, opener, _root) = fake_host(Faults {
        short_reads: Some(7000),
        ..Faults::default()
    });
    let data = pattern(3 * 1024 * 1024 + 123);
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/home/ada/big.bin", &data);
    connect(&host, "sftp://build.example/").unwrap();
    let (middle, eof) = read(&host, BIG, 1_000_000, 1024 * 1024);
    assert_eq!(middle, &data[1_000_000..1_000_000 + 1024 * 1024]);
    assert!(!eof);
    let end = data.len() as u64 - 100;
    let (tail, eof) = read(&host, BIG, end, 1024 * 1024);
    assert_eq!(tail, &data[data.len() - 100..]);
    assert!(eof);
    let (none, eof) = read(&host, BIG, data.len() as u64 + 5, 1024);
    assert!(none.is_empty());
    assert!(eof);
    let missing = host
        .call(
            "provider/read",
            json!({"place": "sftp://build.example/home/ada/none", "offset": 0, "length": 10}),
        )
        .unwrap_err();
    assert_eq!(kind(&missing), "not_found");
}

#[test]
fn spooled_reads_stay_in_the_spool() {
    let (host, opener, root) = fake_host(Faults::default());
    let data = pattern(300 * 1024);
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/home/ada/big.bin", &data);
    connect(&host, "sftp://build.example/").unwrap();
    let spool = root.path().join("spool");
    let answer = host
        .call(
            "provider/read",
            json!({"place": BIG, "offset": 1000, "length": 200_000,
                "transfer": "file", "spool_path": "a.part"}),
        )
        .unwrap();
    assert_eq!(answer["length"], 200_000);
    assert_eq!(answer["eof"], false);
    assert_eq!(
        std::fs::read(spool.join("a.part")).unwrap(),
        &data[1000..201_000]
    );
    let full = spool.join("b.part");
    let answer = host
        .call(
            "provider/read",
            json!({"place": BIG, "offset": 0, "length": 1_000_000,
                "transfer": "file", "spool_path": full}),
        )
        .unwrap();
    assert_eq!(answer["eof"], true);
    assert_eq!(std::fs::read(&full).unwrap(), data);
    for bad in [
        json!("../escape.part"),
        json!(root.path().join("escape.part")),
        json!("a.part"),
    ] {
        let refused = host
            .call(
                "provider/read",
                json!({"place": BIG, "offset": 0, "length": 10,
                    "transfer": "file", "spool_path": bad}),
            )
            .unwrap_err();
        assert_eq!(kind(&refused), "invalid_params", "{bad}");
    }
    assert!(!root.path().join("escape.part").exists());
}

#[test]
fn many_chunks_use_one_handle_on_the_transfer_session() {
    let (host, opener, _root) = fake_host(Faults::default());
    let data = pattern(3 * 1024 * 1024);
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/home/ada/big.bin", &data);
    connect(&host, "sftp://build.example/").unwrap();
    let opens = || {
        opener
            .fs()
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| **c == "Open")
            .count()
    };
    // The first read starts the second session (answer 87).
    read(&host, BIG, 0, 1024);
    assert!(host.wait_for(|h| connection(h).sessions() == 2));
    let before = opens();
    let mut all = Vec::new();
    for chunk in 0..3u64 {
        let (bytes, _) = read(&host, BIG, chunk * 1024 * 1024, 1024 * 1024);
        all.extend(bytes);
    }
    assert_eq!(all, data);
    assert_eq!(opens() - before, 1);
    let openings = opener.openings();
    assert_eq!(openings.len(), 2);
    assert!(!openings[0].second);
    assert!(openings[1].second);
}

#[test]
fn a_connection_that_needed_a_typed_secret_keeps_one_session() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.as_if_typed(true);
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/home/ada/big.bin", b"x");
    connect(&host, "sftp://build.example/").unwrap();
    read(&host, BIG, 0, 10);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(connection(&host).sessions(), 1);
    assert_eq!(opener.openings().len(), 1);
}

#[test]
fn the_second_ssh_session_takes_the_password_from_memory() {
    let (host, log, _root) = ssh_host(
        json!([
            {"ask": "ada@build.example's password: ", "accept": "s3cret-test"},
            {"serve": {"files": {"/home/ada/a.txt": "hello there"}}}
        ]),
        vec![json!({"text": "s3cret-test"})],
    );
    connect(&host, "sftp://build.example/").unwrap();
    let place = "sftp://build.example/home/ada/a.txt";
    assert_eq!(read(&host, place, 0, 100).0, b"hello there");
    assert!(host.wait_for(|h| connection(h).sessions() == 2));
    assert_eq!(read(&host, place, 6, 100).0, b"there");
    // Asked once; both sessions' prompts matched.
    assert_eq!(host.asked().len(), 1);
    let asked: Vec<Value> = log_lines(&log)
        .into_iter()
        .filter(|l| l.get("asked").is_some())
        .collect();
    assert_eq!(asked.len(), 2);
    assert!(asked.iter().all(|l| l["matched"] == true));
}
