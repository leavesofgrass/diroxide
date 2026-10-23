//! Spike SF1 (R24, R25 section 7): does the real `ssh.exe` of this
//! Windows computer ask through an askpass helper when it has no console
//! window, and does any window appear?
//!
//! No real server is needed to reach ssh's first question: a fake server
//! on 127.0.0.1 speaks the SSH transport up to the key exchange's reply
//! and offers a host key ssh has never seen. ssh checks the host key
//! against its known hosts before it checks the signature, so it asks
//! "Are you sure you want to continue connecting" through the same
//! passphrase reader it uses for passwords. The helper is the fake ssh in
//! its recorder mode: it records how it was started and answers "no", so
//! ssh refuses the key and writes nothing. ssh is pointed at an empty
//! configuration file and an empty known-hosts file under the test's own
//! folder, with no agent and no key: none of the owner's ssh files is
//! read or written.
//!
//! Run by hand on Windows, never in CI:
//!
//! ```text
//! cargo nextest run --all-features --run-ignored only spike_sf1
//! ```
//!
//! It prints the matrix (program, start flags, environment) and writes it
//! to `sf1-results.txt` in the temporary folder.
//!
//! **Results on the owner's Windows 11 (build 26200), Friday, October 23,
//! 2026:**
//!
//! - The in-box `OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2` in
//!   `System32\OpenSSH`: with `SSH_ASKPASS` alone it never starts the
//!   helper and waits on its console until killed, in every start mode (a
//!   hang, not a failure). With `SSH_ASKPASS_REQUIRE=force` it starts the
//!   helper for the host key question in every start mode and exits 255
//!   with "Host key verification failed." after the helper's "no";
//!   `DISPLAY` changes nothing. `SSH_ASKPASS_PROMPT` is not set for that
//!   question. Started with `CREATE_NO_WINDOW`, ssh has one console with
//!   no window (its own conhost, window handle 0) and the helper shares
//!   it: no new console, no window on any process. Started detached, the
//!   helper gets a new console of its own, so the plugin never starts ssh
//!   detached.
//! - Git for Windows' `OpenSSH_10.3p1` (an MSYS build): without `force`
//!   it fails at once ("Host key verification failed."); with it, the
//!   helper is started; no console and no window.
//! - The question's text, LF line ends, is kept in
//!   `fixtures/sftp/prompts/`: "fingerprint is SHA256:..." with a final
//!   period on Windows, "fingerprint is: SHA256:..." without one on MSYS.
//! - Not reached without a real server: the password, passphrase and
//!   keyboard-interactive prompts. They go through the same reader in
//!   ssh, so the same road; the container tests (S-C) prove each.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

const FAKE_SSH: &str = env!("CARGO_BIN_EXE_diro-fake-ssh");

fn u32_be(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    u32_be(out, u32::try_from(bytes.len()).unwrap());
    out.extend_from_slice(bytes);
}

/// One unencrypted SSH packet: length, padding length, payload, padding
/// to a multiple of eight with at least four bytes.
fn packet(payload: &[u8]) -> Vec<u8> {
    let mut padding = 8 - (5 + payload.len()) % 8;
    if padding < 4 {
        padding += 8;
    }
    let mut out = Vec::new();
    u32_be(
        &mut out,
        u32::try_from(1 + payload.len() + padding).unwrap(),
    );
    out.push(u8::try_from(padding).unwrap());
    out.extend_from_slice(payload);
    out.extend(std::iter::repeat_n(0u8, padding));
    out
}

fn read_packet(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > 256 * 1024 {
        return Err(std::io::Error::other("packet too long"));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    let padding = body[0] as usize;
    Ok(body[1..len - padding].to_vec())
}

fn kexinit() -> Vec<u8> {
    let mut p = vec![20u8];
    p.extend_from_slice(&[7u8; 16]);
    for list in [
        "curve25519-sha256,curve25519-sha256@libssh.org",
        "ssh-ed25519",
        "aes128-ctr,aes256-ctr,chacha20-poly1305@openssh.com",
        "aes128-ctr,aes256-ctr,chacha20-poly1305@openssh.com",
        "hmac-sha2-256,hmac-sha2-512",
        "hmac-sha2-256,hmac-sha2-512",
        "none",
        "none",
        "",
        "",
    ] {
        string(&mut p, list.as_bytes());
    }
    p.push(0);
    u32_be(&mut p, 0);
    p
}

/// The key exchange's reply: a host key ssh has never seen, any server
/// half, and a signature ssh never gets to check.
fn kex_reply() -> Vec<u8> {
    let mut key = Vec::new();
    string(&mut key, b"ssh-ed25519");
    let public: Vec<u8> = (0u8..32)
        .map(|i| i.wrapping_mul(37).wrapping_add(11))
        .collect();
    string(&mut key, &public);
    let mut signature = Vec::new();
    string(&mut signature, b"ssh-ed25519");
    string(&mut signature, &[0u8; 64]);
    let mut p = vec![31u8];
    string(&mut p, &key);
    string(&mut p, &[9u8; 32]);
    string(&mut p, &signature);
    p
}

/// What the fake server saw of one client.
#[derive(Debug, Default)]
struct Saw {
    banner: String,
    kex_init: bool,
}

/// Serves one client up to the key exchange's reply, then waits for it to
/// leave.
fn serve_one(listener: TcpListener) -> Saw {
    let mut saw = Saw::default();
    // ssh may fail before it connects: wait for it a while, never for ever.
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(_) => return saw,
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let _ = stream.write_all(b"SSH-2.0-diroxideSpike_1.0\r\n");
    // The banner is read a byte at a time: the client's first packet often
    // arrives in the same read, and a buffered reader would swallow it.
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.last() != Some(&b'\n') && line.len() < 256 {
        if stream.read_exact(&mut byte).is_err() {
            return saw;
        }
        line.push(byte[0]);
    }
    saw.banner = String::from_utf8_lossy(&line).trim().to_owned();
    // The client's KEXINIT; nothing of it matters to this spike.
    if read_packet(&mut stream).is_err() {
        return saw;
    }
    let _ = stream.write_all(&packet(&kexinit()));
    match read_packet(&mut stream) {
        Ok(p) if p.first() == Some(&30) => saw.kex_init = true,
        _ => return saw,
    }
    let _ = stream.write_all(&packet(&kex_reply()));
    let mut rest = Vec::new();
    let _ = stream.read_to_end(&mut rest);
    saw
}

/// One run of the matrix.
struct Run {
    program: PathBuf,
    flags: (&'static str, u32),
    env: (&'static str, &'static [(&'static str, &'static str)]),
    pause_ms: u64,
    /// The configuration file's text (empty for SF1).
    config: String,
    /// Options passed before the others (the plugin's Windows overrides).
    overrides: &'static [&'static str],
}

/// What one run showed.
#[derive(Debug, Default)]
struct Outcome {
    exit: Option<i32>,
    timed_out: bool,
    seconds: f64,
    helper_calls: Vec<serde_json::Value>,
    stderr: String,
    windows: String,
}

fn printable(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Asks PowerShell, itself started with no console window, for the window
/// handles of a process and of everything its parent started: a value
/// other than 0 is a window the owner would see.
fn windows_of(helper_pid: u64) -> String {
    let script = format!(
        "$helper = {helper_pid}\n\
         $parent = (Get-CimInstance Win32_Process -Filter \"ProcessId=$helper\").ParentProcessId\n\
         $rows = @()\n\
         foreach ($k in Get-CimInstance Win32_Process) {{\n\
           if ($k.ProcessId -eq $helper -or $k.ProcessId -eq $parent -or $k.ParentProcessId -eq $helper -or $k.ParentProcessId -eq $parent) {{\n\
             $g = Get-Process -Id $k.ProcessId -ErrorAction SilentlyContinue\n\
             $rows += \"$($k.Name) pid $($k.ProcessId) parent $($k.ParentProcessId) window $([int64]$g.MainWindowHandle)\"\n\
           }}\n\
         }}\n\
         $rows -join '; '\n"
    );
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = diroxide_plugins::base64::encode(&utf16);
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            let rows = printable(&o.stdout);
            if rows.is_empty() {
                format!("nothing found; powershell said [{}]", printable(&o.stderr))
            } else {
                rows
            }
        })
        .unwrap_or_else(|e| format!("not asked: {e}"))
}

fn run_once(run: &Run, dir: &Path, n: usize) -> Outcome {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || serve_one(listener));
    let config = dir.join(format!("config_{n}"));
    let known = dir.join(format!("known_hosts_{n}"));
    let global = dir.join(format!("known_hosts_global_{n}"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let record = dir.join(format!("helper_{n}_{stamp}.jsonl"));
    std::fs::write(&config, run.config.as_bytes()).unwrap();
    std::fs::write(&known, b"").unwrap();
    std::fs::write(&global, b"").unwrap();
    let mut command = Command::new(&run.program);
    command
        .arg("-F")
        .arg(&config)
        .args(run.overrides)
        .args(["-o", "BatchMode=no", "-o", "StrictHostKeyChecking=ask"])
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", known.display()))
        .arg("-o")
        .arg(format!("GlobalKnownHostsFile={}", global.display()))
        .args([
            "-o",
            "UpdateHostKeys=no",
            "-o",
            "CheckHostIP=no",
            "-o",
            "IdentityAgent=none",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "PasswordAuthentication=no",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "LogLevel=ERROR",
            "-T",
            "-x",
            "-a",
            "-s",
            "-p",
        ])
        .arg(port.to_string())
        .args(["-l", "spike", "--", "127.0.0.1", "sftp"])
        .env_remove("SSH_ASKPASS")
        .env_remove("SSH_ASKPASS_REQUIRE")
        .env_remove("DISPLAY")
        .env_remove("SSH_AUTH_SOCK")
        .env("FAKE_ASKPASS_RECORD", &record)
        .env("FAKE_ASKPASS_PAUSE_MS", run.pause_ms.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in run.env.1 {
        if *value == "HELPER" {
            command.env(name, FAKE_SSH);
        } else {
            command.env(name, value);
        }
    }
    if run.flags.1 != 0 {
        command.creation_flags(run.flags.1);
    }
    let started = Instant::now();
    let mut child = command.spawn().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    let mut outcome = Outcome::default();
    if run.pause_ms > 0 {
        // While the helper waits, look for windows.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let pid = std::fs::read_to_string(&record)
                .ok()
                .and_then(|text| text.lines().next().map(str::to_owned))
                .and_then(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
                .and_then(|v| v["pid"].as_u64());
            if let Some(pid) = pid {
                outcome.windows = windows_of(pid);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    let deadline = started + Duration::from_secs(15 + run.pause_ms / 1000);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            outcome.exit = status.code();
            break;
        }
        if Instant::now() > deadline {
            outcome.timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    outcome.seconds = started.elapsed().as_secs_f64();
    drop(child.stdin.take());
    outcome.stderr = printable(&rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default());
    outcome.stderr.truncate(300);
    outcome.helper_calls = std::fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let saw = server.join().unwrap_or_default();
    if !saw.kex_init {
        outcome
            .stderr
            .push_str(" [the fake server saw no key exchange]");
    }
    // Nothing was written to the known-hosts files: the helper said no.
    assert_eq!(std::fs::read(&known).unwrap(), b"", "known hosts changed");
    outcome
}

fn programs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        out.push(
            Path::new(&root)
                .join("System32")
                .join("OpenSSH")
                .join("ssh.exe"),
        );
    }
    if let Some(files) = std::env::var_os("ProgramFiles") {
        out.push(
            Path::new(&files)
                .join("Git")
                .join("usr")
                .join("bin")
                .join("ssh.exe"),
        );
    }
    out.into_iter().filter(|p| p.is_file()).collect()
}

fn version(program: &Path) -> String {
    Command::new(program)
        .arg("-V")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| printable(&o.stderr))
        .unwrap_or_default()
}

#[test]
#[ignore = "spike SF1: starts the real ssh.exe of this computer; run by hand"]
fn spike_sf1_windows_askpass_matrix() {
    let dir = std::env::temp_dir().join("sf1");
    std::fs::create_dir_all(&dir).unwrap();
    let flags: [(&str, u32); 3] = [
        ("no window", CREATE_NO_WINDOW),
        ("detached", DETACHED_PROCESS),
        ("inherit", 0),
    ];
    let envs: [(&str, &[(&str, &str)]); 3] = [
        ("askpass only", &[("SSH_ASKPASS", "HELPER")]),
        (
            "askpass, force",
            &[("SSH_ASKPASS", "HELPER"), ("SSH_ASKPASS_REQUIRE", "force")],
        ),
        (
            "askpass, force, display",
            &[
                ("SSH_ASKPASS", "HELPER"),
                ("SSH_ASKPASS_REQUIRE", "force"),
                ("DISPLAY", "diro:0"),
            ],
        ),
    ];
    let mut report = String::new();
    let mut n = 0;
    for program in programs() {
        let _ = writeln!(
            report,
            "PROGRAM {} : {}",
            program.display(),
            version(&program)
        );
        for flag in flags {
            for env in envs {
                n += 1;
                // Long enough for PowerShell to start and look for windows
                // while the helper waits.
                let pause_ms = if flag.1 != 0 && env.0 == "askpass, force" {
                    8000
                } else {
                    0
                };
                let outcome = run_once(
                    &Run {
                        program: program.clone(),
                        flags: flag,
                        env,
                        pause_ms,
                        config: String::new(),
                        overrides: &["-o", "ControlMaster=no", "-o", "ControlPath=none"],
                    },
                    &dir,
                    n,
                );
                let calls: Vec<String> = outcome
                    .helper_calls
                    .iter()
                    .map(|c| {
                        let prompt: String = c["args"][0]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .map(|ch| if ch.is_control() { '|' } else { ch })
                            .collect();
                        format!(
                            "prompt [{prompt}] SSH_ASKPASS_PROMPT={} console={} stdin_tty={} stderr_tty={}",
                            c["ssh_askpass_prompt"], c["has_console"], c["stdin_is_terminal"],
                            c["stderr_is_terminal"]
                        )
                    })
                    .collect();
                let _ = writeln!(
                    report,
                    "RUN {n}: {} / {}: exit {:?}, timed out {}, {:.1} s, helper calls {}; {}; stderr [{}]{}",
                    flag.0,
                    env.0,
                    outcome.exit,
                    outcome.timed_out,
                    outcome.seconds,
                    outcome.helper_calls.len(),
                    calls.join(" / "),
                    outcome.stderr,
                    if outcome.windows.is_empty() {
                        String::new()
                    } else {
                        format!("; windows: {}", outcome.windows)
                    }
                );
            }
        }
    }
    println!("{report}");
    std::fs::write(std::env::temp_dir().join("sf1-results.txt"), &report).unwrap();
    assert!(n > 0, "no ssh program on this computer");
}

/// Spike SF5 (R24): a user's `ControlMaster` line makes the Windows build
/// fail, and the plugin's two overrides fix it. The configuration names a
/// control path under the test's own folder.
///
/// **Results, Friday, October 23, 2026:** with `ControlMaster auto`,
/// `ControlPath` and `ControlPersist 60` in the configuration, the in-box
/// OpenSSH for Windows 9.5p2 fails at once ("getsockname failed: Not a
/// socket", exit code -1) before it connects; with `-o ControlMaster=no
/// -o ControlPath=none` it reaches the host key question through the
/// helper, three runs of three; `ControlPersist=no` is not needed. Git's
/// MSYS 10.3p1 reaches the question with or without the overrides. One
/// earlier overridden run waited 15 seconds with no key exchange: the
/// fake server's own buffered banner read (see SF1b), since fixed.
#[test]
#[ignore = "spike SF5: starts the real ssh.exe of this computer; run by hand"]
fn spike_sf5_controlmaster_on_windows() {
    let dir = std::env::temp_dir().join("sf5");
    std::fs::create_dir_all(&dir).unwrap();
    let config = format!(
        "Host *\n    ControlMaster auto\n    ControlPath {}/cm-%C\n    ControlPersist 60\n",
        dir.display().to_string().replace('\\', "/")
    );
    let mut report = String::new();
    let mut n = 100;
    for program in programs() {
        let _ = writeln!(
            report,
            "PROGRAM {} : {}",
            program.display(),
            version(&program)
        );
        for overrides in [
            [].as_slice(),
            &["-o", "ControlMaster=no", "-o", "ControlPath=none"],
            &[
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-o",
                "ControlPersist=no",
            ],
        ] {
            n += 1;
            let outcome = run_once(
                &Run {
                    program: program.clone(),
                    flags: ("no window", CREATE_NO_WINDOW),
                    env: (
                        "askpass, force",
                        &[("SSH_ASKPASS", "HELPER"), ("SSH_ASKPASS_REQUIRE", "force")],
                    ),
                    pause_ms: 0,
                    config: config.clone(),
                    overrides,
                },
                &dir,
                n,
            );
            let _ = writeln!(
                report,
                "RUN {n}: overrides {}: exit {:?}, timed out {}, {:.1} s, helper calls {}; stderr [{}]",
                overrides.join(" "),
                outcome.exit,
                outcome.timed_out,
                outcome.seconds,
                outcome.helper_calls.len(),
                outcome.stderr
            );
        }
    }
    println!("{report}");
    std::fs::write(std::env::temp_dir().join("sf5-results.txt"), &report).unwrap();
}

/// Spike SF1, second half (R25 commit 51): the whole road with the
/// plugin's own pieces. ssh is started by the plugin's launcher (no
/// console window, in a job), its helper is the plugin's own program, and
/// the question reaches the plugin's endpoint after the proof, where a
/// test handler records it and says no.
///
/// **Results, Friday, October 23, 2026:** with the in-box OpenSSH for
/// Windows 9.5p2 and Git's MSYS 10.3p1 alike, the host key question
/// reached the plugin's endpoint once, after the proof, with no proof
/// refused; ssh ended with 255 ("Host key verification failed.") at once
/// and the known-hosts file stayed empty.
#[test]
#[ignore = "spike SF1: starts the real ssh.exe of this computer; run by hand"]
fn spike_sf1_the_plugins_own_helper_carries_the_question() {
    use std::sync::{Arc, Mutex};

    use diro_sftp::askpass::{Listener, Prompt, Reply};
    use diro_sftp::ssh::Launch;

    let dir = std::env::temp_dir().join("sf1-real");
    std::fs::create_dir_all(&dir).unwrap();
    let mut report = String::new();
    for program in programs() {
        let seen: Arc<Mutex<Vec<Prompt>>> = Arc::new(Mutex::new(Vec::new()));
        let handler_seen = Arc::clone(&seen);
        let mut listener = Listener::open(
            Some(&dir),
            Arc::new(move |prompt: &Prompt| {
                handler_seen.lock().unwrap().push(prompt.clone());
                Reply::Refuse
            }),
        )
        .unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_one(tcp));
        let config = dir.join("empty_config");
        let known = dir.join("known_hosts");
        std::fs::write(&config, b"").unwrap();
        std::fs::write(&known, b"").unwrap();
        let args: Vec<std::ffi::OsString> = [
            "-F".to_owned(),
            config.display().to_string(),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", known.display()),
            "-o".to_owned(),
            format!("GlobalKnownHostsFile={}", known.display()),
            "-o".to_owned(),
            "IdentityAgent=none".to_owned(),
            "-o".to_owned(),
            "UpdateHostKeys=no".to_owned(),
            "-o".to_owned(),
            "BatchMode=no".to_owned(),
            "-o".to_owned(),
            "ControlMaster=no".to_owned(),
            "-o".to_owned(),
            "ControlPath=none".to_owned(),
            "-o".to_owned(),
            "LogLevel=ERROR".to_owned(),
            "-s".to_owned(),
            "-T".to_owned(),
            "-p".to_owned(),
            port.to_string(),
            "-l".to_owned(),
            "spike".to_owned(),
            "--".to_owned(),
            "127.0.0.1".to_owned(),
            "sftp".to_owned(),
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let started = Instant::now();
        let mut ssh = Launch {
            program: program.clone(),
            args,
            askpass: Some(listener.askpass_env(crate::plugin_exe())),
        }
        .spawn()
        .unwrap();
        let status = ssh.wait_timeout(Duration::from_secs(20));
        if status.is_none() {
            ssh.kill();
        }
        let _ = server.join();
        let prompts = seen.lock().unwrap().clone();
        let _ = writeln!(
            report,
            "PROGRAM {}: exit {:?} in {:.1} s; prompts through the plugin's endpoint {}; refused proofs {}; first [{}]; stderr [{}]",
            version(&program),
            status.and_then(|s| s.code()),
            started.elapsed().as_secs_f64(),
            prompts.len(),
            listener.refused(),
            prompts
                .first()
                .map(|p| p.text().replace('\n', "|"))
                .unwrap_or_default(),
            printable(ssh.stderr_text().as_bytes())
        );
        assert_eq!(std::fs::read(&known).unwrap(), b"", "known hosts changed");
        assert_eq!(prompts.len(), 1, "{report}");
        assert!(prompts[0].text().contains("continue connecting"));
        listener.close();
    }
    println!("{report}");
    std::fs::write(std::env::temp_dir().join("sf1-real-results.txt"), &report).unwrap();
}

/// Spike SF1b: ssh inside the host's jobs. The host's process helper
/// starts the plugin as a tree (a kill-on-close job and process-wrap's
/// job); the plugin starts ssh. Three ways, three times each: ssh started
/// straight through the process helper's tree, and ssh started by the
/// plugin's own launcher from a plugin that runs as a tree and from one
/// that runs with no job (a relay mode of the fake ssh stands in for the
/// plugin and calls the launcher itself).
///
/// **Results, Friday, October 23, 2026, on Windows 11 with the in-box
/// OpenSSH for Windows 9.5p2:** every way connects and reaches the host
/// key question, nine runs of nine, under nextest and with the test
/// program run alone. (An earlier reading that a tree start hung ssh was
/// this file's own fake server swallowing the client's first packet with
/// a buffered read of the banner; fixed, it is gone.)
#[test]
#[ignore = "spike SF1b: starts the real ssh.exe of this computer; run by hand"]
fn spike_sf1b_ssh_inside_the_hosts_jobs() {
    let dir = std::env::temp_dir().join("sf1b");
    std::fs::create_dir_all(&dir).unwrap();
    let program = programs().into_iter().next().unwrap();
    for _ in 0..3 {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_one(tcp));
        let config = dir.join("empty_config");
        let known = dir.join("known_hosts");
        std::fs::write(&config, b"").unwrap();
        std::fs::write(&known, b"").unwrap();
        let started = Instant::now();
        let mut handle = diroxide_tools::ProcessSpec::new(&program)
            .arg("-F")
            .arg(&config)
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known.display()))
            .args([
                "-o",
                "IdentityAgent=none",
                "-o",
                "BatchMode=no",
                "-s",
                "-T",
                "-p",
            ])
            .arg(port.to_string())
            .args(["--", "127.0.0.1", "sftp"])
            .env("SSH_ASKPASS", FAKE_SSH)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("FAKE_ASKPASS_RECORD", dir.join("r.jsonl"))
            .stderr(diroxide_tools::Stream::Piped)
            .spawn()
            .unwrap();
        let status = handle.wait_timeout(Duration::from_secs(15)).unwrap();
        if status.is_none() {
            let _ = handle.kill_tree();
        }
        let saw = server.join().unwrap_or_default();
        println!(
            "CASE ssh through the process helper's tree: exit {:?} in {:.1} s, kex {}",
            status.and_then(|s| s.code()),
            started.elapsed().as_secs_f64(),
            saw.kex_init
        );
        assert!(saw.kex_init);
    }
    for tie in [
        diroxide_tools::Tie::Tree,
        diroxide_tools::Tie::Tree,
        diroxide_tools::Tie::Tree,
        diroxide_tools::Tie::Detached,
        diroxide_tools::Tie::Detached,
        diroxide_tools::Tie::Detached,
    ] {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_one(tcp));
        let config = dir.join("empty_config");
        let known = dir.join("known_hosts");
        std::fs::write(&config, b"").unwrap();
        std::fs::write(&known, b"").unwrap();
        let started = Instant::now();
        let mut handle = diroxide_tools::ProcessSpec::new(FAKE_SSH)
            .arg("--relay")
            .arg(&program)
            .arg("-F")
            .arg(&config)
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known.display()))
            .args([
                "-o",
                "IdentityAgent=none",
                "-o",
                "BatchMode=no",
                "-s",
                "-T",
                "-p",
            ])
            .arg(port.to_string())
            .args(["--", "127.0.0.1", "sftp"])
            .env("SSH_ASKPASS", FAKE_SSH)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("FAKE_ASKPASS_RECORD", dir.join("r.jsonl"))
            .stdin(diroxide_tools::Stream::Null)
            .stdout(diroxide_tools::Stream::Null)
            .tie(tie)
            .spawn()
            .unwrap();
        let status = handle.wait_timeout(Duration::from_secs(25)).unwrap();
        let saw = server.join().unwrap_or_default();
        println!(
            "CASE plugin tied {tie:?}, ssh by its launcher: relay exit {:?} in {:.1} s, kex {}",
            status.and_then(|s| s.code()),
            started.elapsed().as_secs_f64(),
            saw.kex_init
        );
        assert!(saw.kex_init);
    }
}

/// Spike SF6, the changed key: ssh's words when a host's key differs from
/// the one known hosts records. The known-hosts file is the test's own,
/// with a made-up key for the fake server's address. The captured text,
/// the port and the file's path made plain, becomes the fixture
/// `fixtures/sftp/stderr/changed-host-key-*.txt`; the helper must never
/// be asked.
///
/// **Results, Friday, October 23, 2026:** both Windows builds refuse at
/// once with the warning block and "Host key verification failed.",
/// exit 255, and never start the helper.
#[test]
#[ignore = "spike SF6: starts the real ssh.exe of this computer; run by hand"]
fn spike_sf6_changed_host_key_capture() {
    let dir = std::env::temp_dir().join("sf6");
    std::fs::create_dir_all(&dir).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sftp/stderr");
    std::fs::create_dir_all(&fixtures).unwrap();
    for program in programs() {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        let server = thread::spawn(move || serve_one(tcp));
        let config = dir.join("empty_config");
        let known = dir.join("known_hosts");
        let record = dir.join("record.jsonl");
        let _ = std::fs::write(&record, b"");
        let mut blob = Vec::new();
        string(&mut blob, b"ssh-ed25519");
        string(&mut blob, &[42u8; 32]);
        let line = format!(
            "[127.0.0.1]:{port} ssh-ed25519 {}\n",
            diroxide_plugins::base64::encode(&blob)
        );
        std::fs::write(&config, b"").unwrap();
        std::fs::write(&known, line.as_bytes()).unwrap();
        let output = Command::new(&program)
            .arg("-F")
            .arg(&config)
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known.display()))
            .arg("-o")
            .arg(format!("GlobalKnownHostsFile={}", known.display()))
            .args([
                "-o",
                "IdentityAgent=none",
                "-o",
                "BatchMode=no",
                "-o",
                "UpdateHostKeys=no",
                "-o",
                "CheckHostIP=no",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-s",
                "-T",
                "-p",
            ])
            .arg(port.to_string())
            .args(["--", "127.0.0.1", "sftp"])
            .env("SSH_ASKPASS", FAKE_SSH)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("FAKE_ASKPASS_RECORD", &record)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .output()
            .unwrap();
        let _ = server.join();
        let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
        let plain = stderr
            .replace(&format!("[127.0.0.1]:{port}"), "build.example")
            .replace(
                &known.display().to_string(),
                "C:/Users/ada/.ssh/known_hosts",
            )
            .replace(
                &known.display().to_string().replace('\\', "\\\\"),
                "C:/Users/ada/.ssh/known_hosts",
            );
        assert_eq!(output.status.code(), Some(255), "{plain}");
        assert!(
            plain.contains("REMOTE HOST IDENTIFICATION HAS CHANGED"),
            "{plain}"
        );
        assert_eq!(
            std::fs::read_to_string(&record).unwrap_or_default(),
            "",
            "the helper was asked about a changed key"
        );
        assert_eq!(
            std::fs::read_to_string(&known).unwrap(),
            line,
            "known hosts changed"
        );
        let name = if version(&program).starts_with("OpenSSH_for_Windows_9.5") {
            "changed-host-key-windows-9.5p2.txt"
        } else {
            "changed-host-key-msys-10.3p1.txt"
        };
        std::fs::write(fixtures.join(name), plain).unwrap();
    }
}
