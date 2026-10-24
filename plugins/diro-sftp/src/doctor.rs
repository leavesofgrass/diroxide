//! `doctor`: one plain report of what the SFTP plugin stands on (R24,
//! "Quick wins"; R25 commit 75), each fact first in its line so it reads
//! on a 40-cell Braille line: which OpenSSH, whether it can ask passwords,
//! the agent, the ssh configuration, the sites file, the key store, the
//! askpass endpoint, and the helper's self-test.
//!
//! **No network.** Nothing here connects anywhere: the ssh is asked its
//! version (`-V`), the agent is asked to count its keys (`ssh-add -l`,
//! next to that ssh; no key, name or comment is read out), the
//! configuration and the sites file are read, and the helper is run once
//! against an endpoint of this plugin with a dummy prompt and a dummy
//! answer, which proves the whole askpass road but ssh. The key store is
//! named, never opened, so no system dialog appears.
//!
//! It runs as the plugin's `doctor` command (shown with `ui/show_text`)
//! and as `diro-sftp doctor` on the command line.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use diroxide_core::Secret;
use diroxide_tools::process::{ProcessSpec, Stream};

use crate::askpass::{ENDPOINT_VARIABLE, Listener, Prompt, Reply, TOKEN_VARIABLE};
use crate::plugin::NAME;
use crate::settings::Settings;
use crate::sites::Sites;
use crate::ssh::locate::probe;
use crate::ssh::{Found, LocateError, SearchPlaces, locate};

/// The prompt of the helper's self-test: plain words, no secret.
pub const SELF_TEST_PROMPT: &str = "diroxide doctor: helper self-test";

/// The answer the self-test expects back: not a secret either.
pub const SELF_TEST_ANSWER: &str = "diroxide-doctor-ok";

/// How long the helper and `ssh-add` each have. Short, and the two run
/// side by side in [`gather`], so the whole doctor answers well inside
/// the ten seconds the host's conformance suite waits for a command's
/// answer: a slow CI runner once pushed the probes past it, one after
/// another with ten seconds each.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// What the agent said, counting keys only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// Running, holding this many keys.
    Keys(usize),
    /// Running, holding none.
    NoKeys,
    /// Not running, or not reachable.
    NotRunning,
    /// Asked, and `ssh-add` gave no answer within the probe's time.
    NoAnswer,
    /// Not asked: no `ssh-add` beside the ssh in use.
    NotChecked,
}

/// What the helper's self-test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Helper {
    /// The dummy answer came back through the whole road.
    Works,
    /// It did not, and why in words.
    Failed(String),
    /// There is no helper program to run.
    NotFound,
}

/// Every fact of the report.
#[derive(Debug)]
pub struct Facts {
    /// The ssh in use, or why there is none.
    pub ssh: Result<Found, LocateError>,
    /// The agent.
    pub agent: Agent,
    /// Hosts in the ssh configuration, names left out, and whether its
    /// file exists.
    pub config: (usize, usize, bool),
    /// Saved sites and sites left out, or the file's refusal in words.
    pub sites: Result<(usize, usize), Vec<String>>,
    /// The key store's name.
    pub key_store: String,
    /// The askpass endpoint's kind in words.
    pub endpoint: &'static str,
    /// The helper's self-test.
    pub helper: Helper,
}

/// The endpoint's kind on this system.
pub fn endpoint_words(runtime: Option<&Path>) -> &'static str {
    if cfg!(windows) {
        "owner-only pipe and proof"
    } else if runtime.is_some() {
        "owner-only socket and proof"
    } else {
        "loopback, proof only"
    }
}

/// The report, one fact a line, each line's fact first.
pub fn report(facts: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    match &facts.ssh {
        Ok(found) => {
            out.push(found.status());
            out.push(if found.version.askpass_force() {
                "Passwords: asked through the helper.".to_owned()
            } else {
                "Passwords: no, ssh below 8.4; keys work.".to_owned()
            });
        }
        Err(error) => {
            let lines = error.lines();
            out.push(format!(
                "ssh: {}",
                lines.first().cloned().unwrap_or_default()
            ));
            out.extend(lines.into_iter().skip(1));
        }
    }
    out.push(match facts.agent {
        Agent::Keys(1) => "Agent: running, 1 key.".to_owned(),
        Agent::Keys(n) => format!("Agent: running, {n} keys."),
        Agent::NoKeys => "Agent: running, no keys.".to_owned(),
        Agent::NotRunning => "Agent: not running.".to_owned(),
        Agent::NoAnswer => "Agent: no answer in time.".to_owned(),
        Agent::NotChecked => "Agent: not checked, no ssh-add.".to_owned(),
    });
    let (hosts, skipped, exists) = facts.config;
    out.push(match (exists, hosts) {
        (false, _) => "Config: no file.".to_owned(),
        (true, 1) => "Config: 1 host.".to_owned(),
        (true, n) => format!("Config: {n} hosts."),
    });
    match skipped {
        0 => {}
        1 => out.push("Config: 1 name left out, unsafe.".to_owned()),
        n => out.push(format!("Config: {n} names left out, unsafe.")),
    }
    match &facts.sites {
        Ok((0, 0)) => out.push("Sites: none saved.".to_owned()),
        Ok((saved, 0)) => out.push(format!("Sites: {saved} saved, no secrets.")),
        Ok((saved, skipped)) => out.push(format!("Sites: {saved} saved, {skipped} skipped.")),
        Err(lines) => out.extend(lines.iter().cloned()),
    }
    out.push(format!("Key store: {}.", facts.key_store));
    out.push(format!("Endpoint: {}.", facts.endpoint));
    out.push(match &facts.helper {
        Helper::Works => "Helper: works.".to_owned(),
        Helper::Failed(why) => format!("Helper failed: {why}"),
        Helper::NotFound => "Helper: not found.".to_owned(),
    });
    out.push(format!("Plugin: {NAME} {}.", env!("CARGO_PKG_VERSION")));
    out
}

/// The agent's answer from `ssh-add -l`'s exit code and output: 0 lists
/// keys (one line each, counted, never read out), 1 means none, 2 means
/// no agent.
pub fn agent_from(code: Option<i32>, output: &str) -> Agent {
    match code {
        Some(0) => match output.lines().filter(|l| !l.trim().is_empty()).count() {
            0 => Agent::NoKeys,
            n => Agent::Keys(n),
        },
        Some(1) => Agent::NoKeys,
        _ => Agent::NotRunning,
    }
}

/// Asks the agent beside `ssh` to count its keys, with no console window.
pub fn probe_agent(ssh: &Path) -> Agent {
    let name = if cfg!(windows) {
        "ssh-add.exe"
    } else {
        "ssh-add"
    };
    let ssh_add = ssh.with_file_name(name);
    if !ssh_add.is_file() {
        return Agent::NotChecked;
    }
    let Ok(mut child) = ProcessSpec::new(&ssh_add)
        .arg("-l")
        .stdin(Stream::Null)
        .stdout(Stream::Piped)
        .stderr(Stream::Null)
        .spawn()
    else {
        return Agent::NotChecked;
    };
    let reader = child.take_stdout().map(|mut out| {
        thread::spawn(move || {
            let mut text = Vec::new();
            let _ = out.by_ref().take(256 * 1024).read_to_end(&mut text);
            text
        })
    });
    let status = child.wait_timeout(PROBE_TIMEOUT).ok().flatten();
    if status.is_none() {
        let _ = child.kill_tree();
        let _ = reader.map(thread::JoinHandle::join);
        return Agent::NoAnswer;
    }
    let text = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    agent_from(
        status.and_then(|s| s.code()),
        &String::from_utf8_lossy(&text),
    )
}

/// The helper's self-test: an endpoint of this plugin answers the dummy
/// prompt with the dummy answer, and the helper program, run as ssh runs
/// it, must give that answer back on its standard output.
pub fn self_test(helper: Option<&Path>, runtime: Option<&Path>) -> Helper {
    let Some(helper) = helper else {
        return Helper::NotFound;
    };
    let handler = Arc::new(|prompt: &Prompt| {
        if prompt.raw == SELF_TEST_PROMPT.as_bytes() {
            Reply::Answer(Secret::new(SELF_TEST_ANSWER.to_owned()))
        } else {
            Reply::Refuse
        }
    });
    let mut listener = match Listener::open(runtime, handler) {
        Ok(listener) => listener,
        Err(e) => return Helper::Failed(format!("no endpoint, {e}.")),
    };
    let env = listener.askpass_env(helper.to_path_buf());
    let spawned = ProcessSpec::new(helper)
        .arg(SELF_TEST_PROMPT)
        .env(ENDPOINT_VARIABLE, &env.endpoint)
        .env(TOKEN_VARIABLE, &env.token)
        .env_remove("SSH_ASKPASS_PROMPT")
        .stdin(Stream::Null)
        .stdout(Stream::Piped)
        .stderr(Stream::Null)
        .spawn();
    let Ok(mut child) = spawned else {
        listener.close();
        return Helper::Failed("it did not start.".to_owned());
    };
    let reader = child.take_stdout().map(|mut out| {
        thread::spawn(move || {
            let mut text = Vec::new();
            let _ = out.by_ref().take(4096).read_to_end(&mut text);
            text
        })
    });
    let status = child.wait_timeout(PROBE_TIMEOUT).ok().flatten();
    if status.is_none() {
        let _ = child.kill_tree();
    }
    let text = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    let refused = listener.refused();
    listener.close();
    let answer = String::from_utf8_lossy(&text);
    match status.and_then(|s| s.code()) {
        _ if refused > 0 => Helper::Failed("its proof was refused.".to_owned()),
        Some(0) if answer.trim_end_matches(['\r', '\n']) == SELF_TEST_ANSWER => Helper::Works,
        Some(0) => Helper::Failed("a wrong answer came back.".to_owned()),
        None => Helper::Failed("no answer in time.".to_owned()),
        Some(_) => Helper::Failed("no answer.".to_owned()),
    }
}

/// Gathers the facts: the ssh by the settings' rules, the agent beside
/// it, the configuration and sites the plugin reads, the key store's
/// name, and the helper's self-test with `helper`.
pub fn gather(
    settings: &Settings,
    sites: &Sites,
    helper: Option<&Path>,
    runtime: Option<&Path>,
) -> Facts {
    let ssh = locate(
        settings.ssh_program.as_deref(),
        &SearchPlaces::from_env(),
        probe,
    );
    // The agent probe and the helper's self-test may each wait the
    // probe's time; they run side by side, so the whole gathering stays
    // inside the host's wait for a command's answer.
    let agent_path = ssh.as_ref().ok().map(|found| found.path.clone());
    let agent_probe =
        thread::spawn(move || agent_path.map_or(Agent::NotChecked, |path| probe_agent(&path)));
    let helper_test = self_test(helper, runtime);
    let agent = agent_probe.join().unwrap_or(Agent::NotChecked);
    let hosts = sites.config_hosts();
    let exists = sites.config_file().is_some_and(|f| f.is_file());
    let sites_facts = match sites.file() {
        Ok(file) => Ok((file.sites().len(), file.skipped().len())),
        Err(error) => Err(error.lines()),
    };
    Facts {
        ssh,
        agent,
        config: (hosts.hosts.len(), hosts.skipped.len(), exists),
        sites: sites_facts,
        key_store: sites.keyring().name(),
        endpoint: endpoint_words(runtime),
        helper: helper_test,
    }
}

/// The report for the command line, `diro-sftp doctor [--ssh-program
/// PATH] [--ssh-config PATH]`: the settings' defaults but for the two the
/// arguments name, the state folder the host names in `DIRO_STATE_DIR`
/// when it is set, the system's key store (named, not opened), and this
/// program as the helper.
pub fn command_line(args: &[String]) -> Vec<String> {
    let mut settings = Settings::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let value = words.next().map(PathBuf::from);
        match word.as_str() {
            "--ssh-program" => settings.ssh_program = value,
            "--ssh-config" => settings.ssh_config_file = value,
            _ => {}
        }
    }
    let sites = Sites::with_keyring(crate::sites::SystemKeyring::new());
    sites.set_config_file(settings.ssh_config_file.clone());
    let state = std::env::var_os("DIRO_STATE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    sites.set_dir(state.clone());
    let helper = crate::ssh::launch::own_program();
    report(&gather(
        &settings,
        &sites,
        helper.as_deref(),
        state.as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::MAX_LINE_CELLS;
    use crate::ssh::{Origin, SshVersion};

    fn found(version: &str) -> Found {
        Found {
            path: PathBuf::from("C:/Windows/System32/OpenSSH/ssh.exe"),
            version: SshVersion::parse(version).unwrap(),
            origin: Origin::System32,
        }
    }

    #[test]
    fn the_golden_report_puts_each_fact_first() {
        let facts = Facts {
            ssh: Ok(found("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2")),
            agent: Agent::Keys(2),
            config: (5, 1, true),
            sites: Ok((3, 0)),
            key_store: "Windows Credential Manager".to_owned(),
            endpoint: "owner-only pipe and proof",
            helper: Helper::Works,
        };
        let lines = report(&facts);
        assert_eq!(
            lines[..9],
            [
                "ssh: OpenSSH for Windows 9.5 in System32",
                "Passwords: asked through the helper.",
                "Agent: running, 2 keys.",
                "Config: 5 hosts.",
                "Config: 1 name left out, unsafe.",
                "Sites: 3 saved, no secrets.",
                "Key store: Windows Credential Manager.",
                "Endpoint: owner-only pipe and proof.",
                "Helper: works.",
            ]
        );
        assert!(lines[9].starts_with("Plugin: SFTP places 0.1.0"));
        for line in &lines {
            assert!(line.chars().count() <= MAX_LINE_CELLS, "{line}");
        }
    }

    #[test]
    fn missing_pieces_are_said_in_words() {
        let facts = Facts {
            ssh: Err(LocateError::NotFound { windows: false }),
            agent: Agent::NotChecked,
            config: (0, 0, false),
            sites: Err(vec!["Sites file refused: it holds a password.".to_owned()]),
            key_store: "none here".to_owned(),
            endpoint: "loopback, proof only",
            helper: Helper::Failed("no answer.".to_owned()),
        };
        let lines = report(&facts);
        assert_eq!(lines[0], "ssh: OpenSSH was not found.");
        assert_eq!(lines[1], "Install the OpenSSH client package.");
        assert!(lines.contains(&"Agent: not checked, no ssh-add.".to_owned()));
        assert!(lines.contains(&"Config: no file.".to_owned()));
        assert!(lines.contains(&"Sites file refused: it holds a password.".to_owned()));
        assert!(lines.contains(&"Helper failed: no answer.".to_owned()));
        let old = Facts {
            ssh: Ok(found("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2")),
            ..facts
        };
        assert_eq!(report(&old)[1], "Passwords: no, ssh below 8.4; keys work.");
    }

    #[test]
    fn the_agent_is_counted_never_read_out() {
        let listing = "256 SHA256:abc ada@example (ED25519)\n3072 SHA256:def x (RSA)\n";
        assert_eq!(agent_from(Some(0), listing), Agent::Keys(2));
        assert_eq!(
            agent_from(Some(1), "The agent has no identities."),
            Agent::NoKeys
        );
        assert_eq!(agent_from(Some(2), ""), Agent::NotRunning);
        assert_eq!(agent_from(None, ""), Agent::NotRunning);
        let lines = report(&Facts {
            ssh: Err(LocateError::NotFound { windows: true }),
            agent: agent_from(Some(0), listing),
            config: (0, 0, true),
            sites: Ok((0, 0)),
            key_store: "x".to_owned(),
            endpoint: "x",
            helper: Helper::NotFound,
        });
        assert!(!lines.join("\n").contains("SHA256"));
        assert!(!lines.join("\n").contains("ada@example"));
    }

    #[test]
    fn a_missing_helper_is_said() {
        assert_eq!(self_test(None, None), Helper::NotFound);
        let nothing = Path::new("no-such-helper-program");
        assert!(matches!(self_test(Some(nothing), None), Helper::Failed(_)));
    }
}
