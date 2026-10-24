//! The container's OpenSSH server tests (R25 commit 76; answer 99): the
//! plugin through the real `ssh` against Debian's own `sshd`, which each
//! test starts and stops itself as the container's ordinary user, on a
//! loopback port, from `docker/sftp/sshd_config`, with a host key, a
//! client key and a known-hosts file made at test time in its own folder.
//! The password is the one `docker/sftp/run-tests.sh` made for this run.
//!
//! The cases: a password, a key, keyboard-interactive, a key passphrase,
//! a new host key, `internal-sftp`, a noisy shell startup file, a
//! read-only server, and the small volume `/dev/shm` for the room check;
//! then the spikes Q1 (resume after a cut link and a killed server) and
//! Q2 (how long a dead link takes to be noticed), SF3's Linux half (a
//! binary file both ways) and the real standard-error texts of the
//! refusals, classified as the table says. With `DIRO_SFTP_CAPTURE=1` the
//! texts and the prompts are also written to `fixtures/sftp/`, the test
//! folder and the port replaced by `DIR` and `PORT`.
//!
//! Every test is ignored natively, with its reason; `run-tests.sh` runs
//! them in the container. Nothing reaches the network: every address is
//! 127.0.0.1.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::SshOpener;
use diro_sftp::sites::MemoryKeyring;
use diro_sftp::ssh::stderr::{Cause, classify};
use diroxide_plugins::base64;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::fakes::FAKE_SSH;
use crate::host::{TestHost, kind};
use crate::plugin_exe;

const MIB: usize = 1024 * 1024;
const SSH: &str = "/usr/bin/ssh";
const SSHD: &str = "/usr/sbin/sshd";
const TEMPLATE: &str = include_str!("../../../../docker/sftp/sshd_config");

/// How a case signs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Auth {
    Password,
    Key,
    Keyboard,
    Passphrase,
}

impl Auth {
    fn server_lines(self) -> &'static str {
        match self {
            Auth::Password => {
                "PasswordAuthentication yes\nKbdInteractiveAuthentication no\nPubkeyAuthentication no"
            }
            Auth::Keyboard => {
                "PasswordAuthentication no\nKbdInteractiveAuthentication yes\nPubkeyAuthentication no"
            }
            Auth::Key | Auth::Passphrase => {
                "PasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes"
            }
        }
    }

    fn client_method(self) -> &'static str {
        match self {
            Auth::Password => "password",
            Auth::Keyboard => "keyboard-interactive",
            Auth::Key | Auth::Passphrase => "publickey",
        }
    }
}

/// One server for one test.
struct Case {
    auth: Auth,
    /// The whole Subsystem line, or empty for a server with none.
    subsystem: String,
    extra: Vec<String>,
    /// Whether the client already trusts the host key.
    known: bool,
}

impl Case {
    fn new(auth: Auth) -> Self {
        Self {
            auth,
            subsystem: "Subsystem sftp /usr/lib/openssh/sftp-server".to_owned(),
            extra: Vec::new(),
            known: true,
        }
    }
}

fn user() -> String {
    std::env::var("USER").expect("USER names the container's user")
}

fn password() -> String {
    let file = std::env::var("DIRO_SFTP_TEST_PASSWORD_FILE")
        .expect("run-tests.sh sets DIRO_SFTP_TEST_PASSWORD_FILE");
    std::fs::read_to_string(file).unwrap().trim().to_owned()
}

fn capture() -> bool {
    std::env::var("DIRO_SFTP_CAPTURE").is_ok_and(|v| v == "1")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn keygen(path: &Path, passphrase: &str) {
    let status = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-C",
            "diroxide-test",
            "-N",
            passphrase,
            "-f",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The server and the files of one test, removed when it ends.
struct Server {
    dir: tempfile::TempDir,
    port: u16,
    child: Child,
}

impl Server {
    fn start(case: &Case) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("diro-sftp-test-")
            .tempdir()
            .unwrap();
        let d = dir.path();
        keygen(&d.join("host_ed25519"), "");
        keygen(&d.join("id_client"), "");
        keygen(&d.join("id_pass"), "test-passphrase-only");
        keygen(&d.join("id_other"), "");
        let mut authorized = read(&d.join("id_client.pub"));
        authorized.push_str(&read(&d.join("id_pass.pub")));
        std::fs::write(d.join("authorized_keys"), authorized).unwrap();
        let port = free_port();
        if case.known {
            let key = read(&d.join("host_ed25519.pub"));
            let key: Vec<&str> = key.split_whitespace().take(2).collect();
            std::fs::write(
                d.join("known_hosts"),
                format!("[127.0.0.1]:{port} {} {}\n", key[0], key[1]),
            )
            .unwrap();
        } else {
            std::fs::write(d.join("known_hosts"), "").unwrap();
        }
        // A server that prints before SFTP starts, as a shell startup file
        // that prints does: the same bytes on the wire. (sshd 10.0 sets a
        // SetEnv BASH_ENV, but the subsystem's shell printed nothing from
        // it in the container, so the text comes from the program itself.)
        let noisy = d.join("noisy-sftp-server");
        std::fs::write(
            &noisy,
            "#!/bin/sh\necho \"Welcome to the test server.\"\nexec /usr/lib/openssh/sftp-server\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&noisy, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = TEMPLATE
            .replace("@PORT@", &port.to_string())
            .replace("@DIR@", &d.display().to_string())
            .replace("@USER@", &user())
            .replace("@AUTH@", case.auth.server_lines())
            .replace(
                "@EXTRA@",
                &case
                    .extra
                    .join("\n")
                    .replace("@DIR@", &d.display().to_string()),
            )
            .replace(
                "Subsystem sftp @SUBSYSTEM@",
                &case.subsystem.replace("@DIR@", &d.display().to_string()),
            );
        std::fs::write(d.join("sshd_config"), config).unwrap();
        let identity = if case.auth == Auth::Passphrase {
            "id_pass"
        } else {
            "id_client"
        };
        let client = format!(
            "Host *\n    UserKnownHostsFile {d}/known_hosts\n    GlobalKnownHostsFile /dev/null\n    \
             StrictHostKeyChecking {strict}\n    IdentityAgent none\n    IdentitiesOnly yes\n    \
             IdentityFile {d}/{identity}\n    PreferredAuthentications {method}\n    \
             UpdateHostKeys no\n    LogLevel ERROR\n",
            d = d.display(),
            strict = if case.known { "yes" } else { "ask" },
            method = case.auth.client_method(),
        );
        std::fs::write(d.join("ssh_config"), client).unwrap();
        let child = Command::new(SSHD)
            .args(["-D", "-e", "-f"])
            .arg(d.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sshd starts");
        let server = Self { dir, port, child };
        server.wait_for_port();
        server
    }

    fn wait_for_port(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("sshd did not listen on {}", self.port);
    }

    fn dir(&self) -> &Path {
        self.dir.path()
    }

    fn client_config(&self) -> PathBuf {
        self.dir().join("ssh_config")
    }

    fn place(&self, path: &str) -> String {
        format!("sftp://{}@127.0.0.1:{}{path}", user(), self.port)
    }

    /// Every process under the listening sshd: the sessions and their
    /// sftp servers.
    fn session_pids(&self) -> Vec<u32> {
        let mut out = Vec::new();
        let mut parents = vec![self.child.id()];
        while let Some(parent) = parents.pop() {
            for entry in std::fs::read_dir("/proc").unwrap().flatten() {
                let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                    continue;
                };
                let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
                let ppid = after.split_whitespace().nth(1).and_then(|p| p.parse().ok());
                if ppid == Some(parent) {
                    out.push(pid);
                    parents.push(pid);
                }
            }
        }
        out
    }

    /// Sends a signal to every session process.
    fn signal_sessions(&self, signal: &str) -> usize {
        let pids = self.session_pids();
        for pid in &pids {
            let _ = Command::new("kill")
                .arg(signal)
                .arg(pid.to_string())
                .status();
        }
        pids.len()
    }

    /// The local path on the server of a place's path.
    fn local(&self, path: &str) -> PathBuf {
        PathBuf::from(path)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.signal_sessions("-KILL");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A host whose plugin starts the real ssh with the case's client
/// configuration, and a keyring in memory.
fn host(server: &Server, answers: Vec<Value>) -> (TestHost, Arc<MemoryKeyring>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let keyring = MemoryKeyring::new();
    let plugin = Plugin::with_parts(
        Arc::new(SshOpener::with_helper(plugin_exe())),
        keyring.clone(),
    );
    let host = TestHost::start(
        plugin,
        root.path(),
        json!({
            "ssh_program": SSH,
            "ssh_config_file": server.client_config(),
            "connect_timeout": 10,
        }),
        answers,
    );
    (host, keyring, root)
}

fn connect(host: &TestHost, place: &str) -> Result<Value, Value> {
    host.call(
        "provider/connect",
        json!({"place": place, "interactive": true}),
    )
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut block = Sha256::digest([seed]).to_vec();
    while out.len() < len {
        block = Sha256::digest(&block).to_vec();
        out.extend_from_slice(&block);
    }
    out.truncate(len);
    out
}

fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn begin(host: &TestHost, place: &str, resume: Option<u64>) -> Result<Value, Value> {
    let mut params = json!({"place": place, "conflict": "overwrite", "transfer": "inline"});
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

fn send(host: &TestHost, id: &Value, data: &[u8], from: usize, to: usize) -> Result<(), Value> {
    let mut at = from;
    while at < to {
        let next = (at + MIB).min(to);
        chunk(host, id, at, &data[at..next])?;
        at = next;
    }
    Ok(())
}

fn upload(host: &TestHost, place: &str, data: &[u8]) {
    let id = begin(host, place, None).unwrap()["write_id"].clone();
    send(host, &id, data, 0, data.len()).unwrap();
    end(host, &id, true).unwrap();
}

fn download(host: &TestHost, place: &str, from: usize, len: usize) -> Result<Vec<u8>, Value> {
    let mut out = Vec::new();
    let mut at = from;
    while at < from + len {
        let want = MIB.min(from + len - at);
        let answer = host.call(
            "provider/read",
            json!({"place": place, "offset": at, "length": want, "transfer": "inline"}),
        )?;
        let data = base64::decode(answer["data"].as_str().unwrap()).unwrap();
        if data.is_empty() {
            break;
        }
        at += data.len();
        out.extend(data);
    }
    Ok(out)
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> Option<Duration> {
    let started = Instant::now();
    while started.elapsed() < limit {
        if done() {
            return Some(started.elapsed());
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

fn ssh_version() -> String {
    let out = Command::new(SSH).arg("-V").output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    // "OpenSSH_10.0p2 Debian-7..." gives "10.0p2".
    text.split_whitespace()
        .next()
        .and_then(|w| w.strip_prefix("OpenSSH_"))
        .unwrap_or("unknown")
        .trim_end_matches(',')
        .to_owned()
}

/// Writes a captured text into `fixtures/sftp/`, the test folder and the
/// port replaced, when capturing.
fn keep(server: Option<&Server>, folder: &str, name: &str, text: &str) {
    if !capture() {
        return;
    }
    let mut text = text.replace('\r', "");
    if let Some(server) = server {
        text = text
            .replace(&server.dir().display().to_string(), "DIR")
            .replace(&server.port.to_string(), "PORT");
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/sftp")
        .join(folder)
        .join(format!("{name}-linux-{}.txt", ssh_version()));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

// --- The sign-in cases.

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_password_signs_in_and_the_keyring_fills_it_next_time() {
    let server = Server::start(&Case::new(Auth::Password));
    let sites = format!(
        "[[site]]\nid = \"t3st\"\nname = \"test\"\nhost = \"127.0.0.1\"\nport = {}\nuser = \"{}\"\nsecret = \"keyring\"\n",
        server.port,
        user()
    );
    let (host, keyring, root) = host(
        &server,
        vec![json!({"text": password()}), json!({"answer": true})],
    );
    std::fs::write(root.path().join("state/sites.toml"), sites).unwrap();
    connect(&host, &server.place("/")).unwrap();
    let asked = host.asked();
    assert_eq!(asked[0]["kind"], "secret");
    assert_eq!(
        asked[0]["title"],
        format!("Password for {} at 127.0.0.1", user())
    );
    assert_eq!(asked[1]["title"], "Remember this password? y or n");
    assert_eq!(keyring.len(), 1);
    let listed = host
        .call(
            "provider/list",
            json!({"place": server.place(server.dir().to_str().unwrap())}),
        )
        .unwrap();
    assert!(listed.to_string().contains("known_hosts"), "{listed}");
    host.command("disconnect", Some("site:t3st"), "/work")
        .unwrap();
    connect(&host, &server.place("/")).unwrap();
    assert_eq!(
        host.asked().len(),
        2,
        "filled from the keyring, nothing asked"
    );
    let file = read(&root.path().join("state/sites.toml"));
    assert!(!file.contains(&password()));
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_key_signs_in_and_binary_files_round_trip() {
    // SF3's Linux half: binary-clean SFTP over ssh's pipes, both ways.
    let server = Server::start(&Case::new(Auth::Key));
    let (host, _, _root) = host(&server, Vec::new());
    connect(&host, &server.place("/")).unwrap();
    assert!(host.asked().is_empty());
    let data = pattern(16 * MIB + 333, 7);
    let path = format!("{}/up.bin", server.dir().display());
    let started = Instant::now();
    upload(&host, &server.place(&path), &data);
    let up = started.elapsed();
    assert_eq!(sha(&std::fs::read(&path).unwrap()), sha(&data));
    let started = Instant::now();
    let back = download(&host, &server.place(&path), 0, data.len()).unwrap();
    let down = started.elapsed();
    assert_eq!(sha(&back), sha(&data));
    eprintln!("SF3 linux: 16 MiB up in {up:?}, down in {down:?}, hashes equal");
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_keyboard_interactive_is_the_servers_question() {
    let server = Server::start(&Case::new(Auth::Keyboard));
    let (host, _, _root) = host(&server, vec![json!({"text": password()})]);
    connect(&host, &server.place("/")).unwrap();
    let asked = host.asked();
    assert_eq!(asked[0]["kind"], "secret");
    assert_eq!(asked[0]["title"], "Question from 127.0.0.1:");
    let words = asked[0]["message"][0].as_str().unwrap().to_owned();
    // ssh names the account first from 8.4 on, so this can never read as
    // ssh's own password prompt.
    assert!(
        words.starts_with(&format!("({}@127.0.0.1)", user())),
        "{words}"
    );
    keep(
        Some(&server),
        "prompts",
        "keyboard-interactive",
        &format!("{words}\n"),
    );
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_key_passphrase_is_asked_by_the_keys_name() {
    let server = Server::start(&Case::new(Auth::Passphrase));
    let (host, _, _root) = host(&server, vec![json!({"text": "test-passphrase-only"})]);
    connect(&host, &server.place("/")).unwrap();
    let asked = host.asked();
    assert_eq!(asked[0]["title"], "Key passphrase for id_pass");
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_new_host_key_is_asked_and_ssh_writes_known_hosts() {
    let server = Server::start(&Case {
        known: false,
        ..Case::new(Auth::Key)
    });
    let (host, _, _root) = host(&server, vec![json!({"answer": true})]);
    connect(&host, &server.place("/")).unwrap();
    let asked = host.asked();
    assert_eq!(asked[0]["title"], "New server key");
    let message: Vec<&str> = asked[0]["message"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(message[2], "Key type: ED25519.");
    assert!(
        message.iter().any(|l| l.starts_with("Part 1 of")),
        "{message:?}"
    );
    // ssh wrote the key, not the plugin.
    assert!(read(&server.dir().join("known_hosts")).contains("ssh-ed25519"));
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_internal_sftp_serves_the_same() {
    let server = Server::start(&Case {
        subsystem: "Subsystem sftp internal-sftp".to_owned(),
        ..Case::new(Auth::Key)
    });
    let (host, _, _root) = host(&server, Vec::new());
    let answer = connect(&host, &server.place("/")).unwrap();
    assert_eq!(answer["capabilities"]["atomic_rename"], true);
    let data = pattern(MIB + 5, 3);
    let path = format!("{}/internal.bin", server.dir().display());
    upload(&host, &server.place(&path), &data);
    assert_eq!(sha(&std::fs::read(&path).unwrap()), sha(&data));
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_noisy_shell_startup_file_is_said() {
    let server = Server::start(&Case {
        subsystem: "Subsystem sftp @DIR@/noisy-sftp-server".to_owned(),
        ..Case::new(Auth::Key)
    });
    let (host, _, _root) = host(&server, Vec::new());
    let refused = connect(&host, &server.place("/")).unwrap_err();
    assert_eq!(refused["message"], "The server's shell printed text first.");
    assert_eq!(
        refused["data"]["hint"],
        "Make its startup file quiet for sftp."
    );
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_a_read_only_server_refuses_writes_in_words() {
    let server = Server::start(&Case {
        subsystem: "Subsystem sftp internal-sftp -R".to_owned(),
        ..Case::new(Auth::Key)
    });
    let (host, _, _root) = host(&server, Vec::new());
    connect(&host, &server.place("/")).unwrap();
    let path = format!("{}/new-folder", server.dir().display());
    let refused = host
        .call("provider/create_dir", json!({"place": server.place(&path)}))
        .unwrap_err();
    eprintln!("read-only create_dir: {refused}");
    assert_eq!(kind(&refused), "permission_denied", "{refused}");
    assert!(!Path::new(&path).exists());
    let write = begin(
        &host,
        &server.place(&format!("{}/x.bin", server.dir().display())),
        None,
    );
    eprintln!("read-only write_begin: {write:?}");
    assert!(write.is_err());
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_the_small_volume_reports_its_room_and_a_full_disk_is_said() {
    let server = Server::start(&Case::new(Auth::Key));
    let (host, _, _root) = host(&server, Vec::new());
    connect(&host, &server.place("/")).unwrap();
    let folder = format!("/dev/shm/diro-room-{}", server.port);
    host.call(
        "provider/create_dir",
        json!({"place": server.place(&folder)}),
    )
    .unwrap();
    let answer = host
        .call("provider/volumes", json!({"place": server.place(&folder)}))
        .unwrap();
    let row = &answer["volumes"][0];
    let df = Command::new("df")
        .args(["-B1", "--output=avail,size", "/dev/shm"])
        .output()
        .unwrap();
    let df = String::from_utf8_lossy(&df.stdout).to_string();
    let numbers: Vec<u64> = df
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let (free, total) = (
        row["free"].as_u64().unwrap(),
        row["total"].as_u64().unwrap(),
    );
    eprintln!(
        "V4: statvfs free {free} total {total}; df avail {} size {}",
        numbers[0], numbers[1]
    );
    assert!(free.abs_diff(numbers[0]) <= MIB as u64);
    assert_eq!(total, numbers[1]);
    // More than the room: the write fails in words and leaves nothing.
    let size = usize::try_from(free).unwrap() + 2 * MIB;
    let data = vec![0x5Au8; size];
    let place = server.place(&format!("{folder}/big.bin"));
    let id = begin(&host, &place, None).unwrap()["write_id"].clone();
    let failed = send(&host, &id, &data, 0, size)
        .err()
        .or_else(|| end(&host, &id, true).err())
        .expect("a full disk refuses the write");
    eprintln!("room: {failed}");
    let _ = end(&host, &id, false);
    let left: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    for name in &left {
        let _ = std::fs::remove_file(Path::new(&folder).join(name));
    }
    let _ = std::fs::remove_dir(&folder);
    assert_eq!(kind(&failed), "no_space", "{failed}");
    assert!(left.is_empty(), "left behind: {left:?}");
}

// --- Spike Q1: resume against a real server.

fn window_matches(partial: &[u8], source: &[u8]) -> bool {
    let head = 64 * 1024;
    let tail = MIB;
    let len = partial.len();
    if len > source.len() {
        return false;
    }
    partial[..head.min(len)] == source[..head.min(len)]
        && partial[len.saturating_sub(tail)..] == source[len.saturating_sub(tail)..len]
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_q1_resume_after_a_cut_link_and_a_killed_server() {
    let server = Server::start(&Case::new(Auth::Key));
    let data = pattern(32 * MIB, 11);
    let mut results = Vec::new();
    for mode in ["client cut", "server killed", "corrupt tail"] {
        let (host, _, _root) = host(&server, Vec::new());
        let path = format!(
            "{}/q1-{}.bin",
            server.dir().display(),
            mode.replace(' ', "-")
        );
        let place = server.place(&path);
        connect(&host, &place).unwrap();
        let begun = begin(&host, &place, None).unwrap();
        let id = begun["write_id"].clone();
        let unfinished = begun["unfinished"].as_str().unwrap().to_owned();
        let unfinished_path = unfinished
            .split_once(&format!(":{}", server.port))
            .unwrap()
            .1
            .to_owned();
        send(&host, &id, &data, 0, 8 * MIB).unwrap();
        match mode {
            "server killed" => {
                server.signal_sessions("-KILL");
            }
            _ => {
                let target =
                    diro_sftp::ssh::Target::new("127.0.0.1", Some(&user()), Some(server.port))
                        .unwrap();
                assert!(host.plugin().provider().cancel_connection(&target));
            }
        }
        let after = send(&host, &id, &data, 8 * MIB, 9 * MIB);
        assert!(after.is_err(), "{mode}: the link is gone");
        let partial = std::fs::read(server.local(&unfinished_path)).unwrap();
        let held = partial.len();
        let tail_equal = partial[held - MIB..] == data[held - MIB..held];
        if mode == "corrupt tail" {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(server.local(&unfinished_path))
                .unwrap();
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start((held - 4096) as u64)).unwrap();
            file.write_all(&[0u8; 4096]).unwrap();
        }
        let partial = std::fs::read(server.local(&unfinished_path)).unwrap();
        let window = window_matches(&partial, &data);
        let from = if window { held as u64 } else { 0 };
        // The host's way: connect again and resume; a link that is still
        // being found dead, or reopened, is tried again shortly.
        let mut resumed = Err(Value::Null);
        for _ in 0..50 {
            resumed = connect(&host, &place).and_then(|_| begin(&host, &place, Some(from)));
            if resumed.is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let resumed = resumed.unwrap();
        let offset = resumed["offset"].as_u64().unwrap_or(0) as usize;
        let id = resumed["write_id"].clone();
        send(&host, &id, &data, offset, data.len()).unwrap();
        end(&host, &id, true).unwrap();
        let equal = sha(&std::fs::read(&path).unwrap()) == sha(&data);
        results.push(format!(
            "Q1 {mode}: acknowledged 8388608, server held {held}, tail equal {tail_equal}, \
             window check {window}, resumed at {offset}, final hash equal {equal}"
        ));
        assert!(equal, "{mode}");
        assert_eq!(window, mode != "corrupt tail", "{mode}");
    }
    // A download cut in the middle goes on from the bytes it holds.
    let (host, _, _root) = host(&server, Vec::new());
    let path = format!("{}/q1-source.bin", server.dir().display());
    std::fs::write(&path, &data).unwrap();
    connect(&host, &server.place(&path)).unwrap();
    let mut got = download(&host, &server.place(&path), 0, 8 * MIB).unwrap();
    server.signal_sessions("-KILL");
    // The read may meet the dead link or the reconnect; either way it
    // goes on below from the bytes already held.
    let _ = download(&host, &server.place(&path), 8 * MIB, MIB);
    let mut rest = Err(Value::Null);
    for _ in 0..50 {
        rest = connect(&host, &server.place(&path)).and_then(|_| {
            download(
                &host,
                &server.place(&path),
                got.len(),
                data.len() - got.len(),
            )
        });
        if rest.is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    got.extend(rest.unwrap());
    let equal = sha(&got) == sha(&data);
    results.push(format!(
        "Q1 download: cut at 8388608 by a killed server, resumed, hash equal {equal}"
    ));
    assert!(equal);
    for line in results {
        eprintln!("{line}");
    }
}

// --- Spike Q2: how long a dead link takes to be noticed.

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_q2_a_dead_link_is_noticed_within_a_minute() {
    // A typed password: no automatic reconnect, so the loss is said.
    let server = Server::start(&Case::new(Auth::Password));
    for (mode, signal, limit) in [
        ("closed server", "-KILL", Duration::from_secs(10)),
        ("frozen server", "-STOP", Duration::from_secs(90)),
    ] {
        let (host, _, _root) = host(&server, vec![json!({"text": password()})]);
        connect(&host, &server.place("/")).unwrap();
        let stopped = server.signal_sessions(signal);
        assert!(stopped > 0, "{mode}: no session found");
        let noticed = wait_until(limit, || !host.notices("provider/lost").is_empty());
        let lost = host.notices("provider/lost");
        eprintln!("Q2 {mode}: provider/lost after {noticed:?}: {lost:?}");
        server.signal_sessions("-KILL");
        // ServerAliveInterval 15 and ServerAliveCountMax 3: ssh gives up on
        // the fourth unanswered keepalive, about 60 seconds in.
        assert!(
            noticed.is_some_and(|t| t <= Duration::from_secs(75)),
            "{mode}: {noticed:?}"
        );
    }
}

// --- The real standard-error texts and prompts.

/// Runs the real ssh as the plugin would, `-s` for sftp, and gives its
/// standard error.
fn run_ssh(
    config: &Path,
    port: u16,
    user: &str,
    extra: &[&str],
    askpass: Option<(&Path, &str)>,
) -> String {
    let mut command = Command::new(SSH);
    command
        .args(["-F"])
        .arg(config)
        .arg("-p")
        .arg(port.to_string())
        .args(["-l", user, "-o", "ConnectTimeout=3"])
        .args(extra)
        .args(["-s", "--", "127.0.0.1", "sftp"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    match askpass {
        Some((record, answer)) => {
            command
                .env("SSH_ASKPASS", FAKE_SSH)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", ":0")
                .env("FAKE_ASKPASS_RECORD", record)
                .env("FAKE_ASKPASS_ANSWER", answer);
        }
        None => {
            command.args(["-o", "BatchMode=yes"]);
        }
    }
    let mut child = command.spawn().unwrap();
    let mut text = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let _ = child.wait();
    text
}

fn prompts_recorded(record: &Path) -> Vec<String> {
    std::fs::read_to_string(record)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["args"][0].as_str().map(str::to_owned))
        .collect()
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_the_real_refusals_classify_as_the_table_says() {
    let mut seen: Vec<(&str, Cause, Cause)> = Vec::new();
    let me = user();

    // Nothing listens on the port.
    let server = Server::start(&Case::new(Auth::Key));
    let nobody = free_port();
    let text = run_ssh(&server.client_config(), nobody, &me, &[], None);
    keep(
        None,
        "stderr",
        "connection-refused",
        &text.replace(&nobody.to_string(), "PORT"),
    );
    seen.push((
        "connection-refused",
        classify(&text),
        Cause::ConnectionRefused,
    ));

    // An unknown key with strict checking, no question possible.
    let strict = Server::start(&Case {
        known: false,
        ..Case::new(Auth::Key)
    });
    let text = run_ssh(
        &strict.client_config(),
        strict.port,
        &me,
        &["-o", "StrictHostKeyChecking=yes"],
        None,
    );
    keep(
        Some(&strict),
        "stderr",
        "host-key-verification-failed",
        &text,
    );
    seen.push((
        "host-key-verification-failed",
        classify(&text),
        Cause::KeyNotTrusted,
    ));

    // A changed key: known_hosts holds another one.
    let other = read(&server.dir().join("id_client.pub"));
    let other: Vec<&str> = other.split_whitespace().take(2).collect();
    std::fs::write(
        server.dir().join("known_hosts"),
        format!("[127.0.0.1]:{} {} {}\n", server.port, other[0], other[1]),
    )
    .unwrap();
    let text = run_ssh(&server.client_config(), server.port, &me, &[], None);
    keep(Some(&server), "stderr", "changed-host-key", &text);
    seen.push(("changed-host-key", classify(&text), Cause::ChangedKey));
    drop(server);

    // A wrong password, and the prompts as ssh gives them.
    let password_server = Server::start(&Case::new(Auth::Password));
    let record = password_server.dir().join("askpass.log");
    let text = run_ssh(
        &password_server.client_config(),
        password_server.port,
        &me,
        &["-o", "NumberOfPasswordPrompts=1"],
        Some((record.as_path(), "a-wrong-password")),
    );
    keep(Some(&password_server), "stderr", "permission-denied", &text);
    seen.push(("permission-denied", classify(&text), Cause::SignInRefused));
    let prompts = prompts_recorded(&record);
    assert_eq!(
        prompts[0],
        format!("{me}@127.0.0.1's password: "),
        "{prompts:?}"
    );
    keep(
        Some(&password_server),
        "prompts",
        "password",
        &format!("{}\n", prompts[0]),
    );

    // The key passphrase prompt.
    let pass = Server::start(&Case::new(Auth::Passphrase));
    let record = pass.dir().join("askpass.log");
    let _ = run_ssh(
        &pass.client_config(),
        pass.port,
        &me,
        &[],
        Some((record.as_path(), "test-passphrase-only")),
    );
    let prompts = prompts_recorded(&record);
    assert!(
        prompts[0].starts_with("Enter passphrase for key '"),
        "{prompts:?}"
    );
    keep(
        Some(&pass),
        "prompts",
        "key-passphrase",
        &format!("{}\n", prompts[0]),
    );

    // A new host key's question.
    let fresh = Server::start(&Case {
        known: false,
        ..Case::new(Auth::Key)
    });
    let record = fresh.dir().join("askpass.log");
    let _ = run_ssh(
        &fresh.client_config(),
        fresh.port,
        &me,
        &[],
        Some((record.as_path(), "no")),
    );
    let prompts = prompts_recorded(&record);
    assert!(
        prompts[0].contains("Are you sure you want to continue connecting"),
        "{prompts:?}"
    );
    keep(
        Some(&fresh),
        "prompts",
        "new-host-key",
        &format!("{}\n", prompts[0]),
    );

    // No sftp subsystem.
    let bare = Server::start(&Case {
        subsystem: String::new(),
        ..Case::new(Auth::Key)
    });
    let text = run_ssh(&bare.client_config(), bare.port, &me, &[], None);
    keep(Some(&bare), "stderr", "no-subsystem", &text);
    seen.push(("no-subsystem", classify(&text), Cause::NoSubsystem));

    // Too many authentication failures: two keys the server does not know,
    // one try allowed.
    let tight = Server::start(&Case {
        extra: vec!["MaxAuthTries 1".to_owned()],
        ..Case::new(Auth::Key)
    });
    std::fs::write(tight.dir().join("authorized_keys"), "").unwrap();
    let text = run_ssh(
        &tight.client_config(),
        tight.port,
        &me,
        &[
            "-o",
            &format!("IdentityFile={}/id_other", tight.dir().display()),
        ],
        None,
    );
    keep(
        Some(&tight),
        "stderr",
        "too-many-authentication-failures",
        &text,
    );
    seen.push((
        "too-many-authentication-failures",
        classify(&text),
        Cause::SignInRefused,
    ));

    // A configuration with an error.
    let bad = tight.dir().join("bad_config");
    std::fs::write(&bad, "NoSuchOption yes\n").unwrap();
    let text = run_ssh(&bad, tight.port, &me, &[], None);
    keep(Some(&tight), "stderr", "bad-config", &text);
    seen.push(("bad-config", classify(&text), Cause::BadConfig));

    // A port that accepts and never says a word.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    let holder = thread::spawn(move || {
        let conn = silent.accept();
        thread::sleep(Duration::from_secs(6));
        drop(conn);
    });
    let text = run_ssh(&tight.client_config(), silent_port, &me, &[], None);
    keep(
        None,
        "stderr",
        "banner-timed-out",
        &text.replace(&silent_port.to_string(), "PORT"),
    );
    seen.push(("banner-timed-out", classify(&text), Cause::TimedOut));
    let _ = holder.join();

    // The server closes the connection under ssh, and a frozen one times
    // out its keepalives.
    for (name, signal, expected) in [
        ("closed-by-server", "-KILL", Cause::ClosedByServer),
        ("keepalive-timed-out", "-STOP", Cause::Unknown),
    ] {
        let live = Server::start(&Case::new(Auth::Key));
        let mut child = Command::new(SSH)
            .args(["-F"])
            .arg(live.client_config())
            .arg("-p")
            .arg(live.port.to_string())
            .args(["-l", me.as_str()])
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ServerAliveInterval=1",
                "-o",
                "ServerAliveCountMax=2",
                // ssh says the keepalive timeout at INFO, which a LogLevel
                // of ERROR (the tests' configuration, and some owners')
                // hides: the plugin then has only ssh's exit to go on.
                "-o",
                "LogLevel=INFO",
            ])
            .args(["-s", "--", "127.0.0.1", "sftp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The SFTP init, so the session is up before the signal.
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&[0, 0, 0, 5, 1, 0, 0, 0, 3]).unwrap();
        let mut version = [0u8; 4];
        child
            .stdout
            .take()
            .unwrap()
            .read_exact(&mut version)
            .unwrap();
        live.signal_sessions(signal);
        let mut text = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let _ = child.wait();
        drop(stdin);
        keep(Some(&live), "stderr", name, &text);
        seen.push((name, classify(&text), expected));
    }

    for (name, got, wanted) in &seen {
        eprintln!("stderr {name}: classified {got:?}, the table wants {wanted:?}");
    }
    let wrong: Vec<_> = seen
        .iter()
        .filter(|(_, got, wanted)| got != wanted)
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
#[ignore = "needs the container's OpenSSH server: docker/sftp/run-tests.sh"]
fn server_nothing_listens_outside_loopback() {
    let server = Server::start(&Case::new(Auth::Key));
    let config = read(&server.dir().join("sshd_config"));
    assert!(config.contains("ListenAddress 127.0.0.1"));
    let tcp = read(Path::new("/proc/net/tcp"));
    let port = format!(":{:04X}", server.port);
    let listening: Vec<&str> = tcp
        .lines()
        .filter(|l| l.contains(&port) && l.split_whitespace().nth(3) == Some("0A"))
        .collect();
    assert!(!listening.is_empty());
    // 0100007F is 127.0.0.1 in /proc/net/tcp's order.
    assert!(
        listening
            .iter()
            .all(|l| l.split_whitespace().nth(1).unwrap().starts_with("0100007F")),
        "{listening:?}"
    );
}
