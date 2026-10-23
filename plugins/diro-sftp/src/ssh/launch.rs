//! Starting ssh (R24 sections 1.2 and 1.5; spike SF1).
//!
//! - **No console window, ever.** On Windows ssh starts with
//!   `CREATE_NO_WINDOW` inside a job that dies with the plugin (the
//!   process helper's tree); spike SF1 showed the helper it starts then
//!   shares that windowless console. ssh is never started detached, which
//!   gives the helper a console of its own.
//! - **No terminal on Unix.** ssh starts in a session of its own, so it
//!   has no controlling terminal to ask on; it is killed with its whole
//!   group when the connection ends.
//! - **The askpass road.** When ssh may ask, its environment names the
//!   plugin's own program as `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force`
//!   (without it the Windows build waits on its hidden console for ever,
//!   SF1), a dummy `DISPLAY` when none is set, and this connection's
//!   endpoint and token. Inherited askpass variables are removed first.
//!   No secret is ever in the arguments.
//! - **Standard error** is read by a thread into a ring of the last
//!   [`STDERR_RING`] bytes, for the error classifier; it is never the
//!   sentence the owner hears, only matched against.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout, ExitStatus};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_tools::{ProcessHandle, ProcessSpec, Stream, Tie};

use crate::askpass::{ENDPOINT_VARIABLE, TOKEN_VARIABLE};

/// How much of ssh's standard error is kept: its last 64 KiB.
pub const STDERR_RING: usize = 64 * 1024;

/// The dummy display older ssh wants before it uses a helper.
pub const DUMMY_DISPLAY: &str = "diro:0";

/// The helper's environment for one connection.
#[derive(Clone)]
pub struct AskpassEnv {
    /// The plugin's own program.
    pub helper: PathBuf,
    /// The endpoint the helper reaches.
    pub endpoint: String,
    /// The connection's token, in hexadecimal; never logged.
    pub token: String,
}

impl fmt::Debug for AskpassEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AskpassEnv")
            .field("helper", &self.helper)
            .field("endpoint", &self.endpoint)
            .field("token", &"hidden")
            .finish()
    }
}

/// What to start.
#[derive(Clone, Debug)]
pub struct Launch {
    /// The ssh program's absolute path.
    pub program: PathBuf,
    /// Its arguments ([`super::options::SessionOptions::arguments`]).
    pub args: Vec<OsString>,
    /// The helper's environment, when ssh may ask.
    pub askpass: Option<AskpassEnv>,
}

/// Why ssh could not be started.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    /// The system's words.
    #[error("ssh could not be started: {0}")]
    Start(String),
}

/// The last bytes of ssh's standard error.
#[derive(Debug, Default)]
struct Ring {
    bytes: VecDeque<u8>,
    ended: bool,
}

/// A running ssh.
pub struct SshProcess {
    handle: ProcessHandle,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Arc<Mutex<Ring>>,
}

impl fmt::Debug for SshProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshProcess")
            .field("id", &self.handle.id())
            .finish_non_exhaustive()
    }
}

impl Launch {
    /// The process specification: the program, the arguments as an array,
    /// the environment, the pipes and the tie.
    pub fn spec(&self) -> ProcessSpec {
        let mut spec = ProcessSpec::new(&self.program)
            .args(self.args.iter().cloned())
            .stdin(Stream::Piped)
            .stdout(Stream::Piped)
            .stderr(Stream::Piped)
            .env_remove("SSH_ASKPASS")
            .env_remove("SSH_ASKPASS_REQUIRE")
            .env_remove(ENDPOINT_VARIABLE)
            .env_remove(TOKEN_VARIABLE)
            .tie(if cfg!(windows) {
                Tie::Tree
            } else {
                Tie::Detached
            });
        if let Some(askpass) = &self.askpass {
            spec = spec
                .env("SSH_ASKPASS", &askpass.helper)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env(ENDPOINT_VARIABLE, &askpass.endpoint)
                .env(TOKEN_VARIABLE, &askpass.token);
            if std::env::var_os("DISPLAY").is_none() {
                spec = spec.env("DISPLAY", DUMMY_DISPLAY);
            }
        }
        spec
    }

    /// Starts ssh.
    pub fn spawn(&self) -> Result<SshProcess, LaunchError> {
        let mut handle = self
            .spec()
            .spawn()
            .map_err(|e| LaunchError::Start(e.to_string()))?;
        let stdin = handle.take_stdin();
        let stdout = handle.take_stdout();
        let stderr = Arc::new(Mutex::new(Ring::default()));
        if let Some(mut pipe) = handle.take_stderr() {
            let ring = Arc::clone(&stderr);
            let started = thread::Builder::new()
                .name("ssh-stderr".to_owned())
                .spawn(move || {
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = pipe.read(&mut chunk).unwrap_or(0);
                        let mut ring = ring.lock().unwrap_or_else(PoisonError::into_inner);
                        if n == 0 {
                            ring.ended = true;
                            return;
                        }
                        ring.bytes.extend(&chunk[..n]);
                        let over = ring.bytes.len().saturating_sub(STDERR_RING);
                        ring.bytes.drain(..over);
                    }
                });
            if started.is_err() {
                stderr.lock().unwrap_or_else(PoisonError::into_inner).ended = true;
            }
        }
        Ok(SshProcess {
            handle,
            stdin,
            stdout,
            stderr,
        })
    }
}

impl SshProcess {
    /// Its process id.
    pub fn id(&self) -> u32 {
        self.handle.id()
    }

    /// The pipe to ssh's standard input: the SFTP bytes the plugin sends.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    /// The pipe from ssh's standard output: the SFTP bytes the server
    /// sends.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.stdout.take()
    }

    /// The last bytes of standard error, as text (not cleaned: for the
    /// classifier, never shown as it is).
    pub fn stderr_text(&self) -> String {
        let ring = self.stderr.lock().unwrap_or_else(PoisonError::into_inner);
        let (a, b) = ring.bytes.as_slices();
        let mut bytes = a.to_vec();
        bytes.extend_from_slice(b);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Whether standard error has ended (ssh is gone or closed it).
    pub fn stderr_ended(&self) -> bool {
        self.stderr
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ended
    }

    /// How ssh ended, if it has.
    pub fn try_wait(&mut self) -> Option<ExitStatus> {
        self.handle.try_wait().ok().flatten()
    }

    /// Waits up to `timeout` for ssh to end.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Option<ExitStatus> {
        self.handle.wait_timeout(timeout).ok().flatten()
    }

    /// Ends ssh and everything it started (the helper too), then reaps it.
    pub fn kill(&mut self) -> Option<ExitStatus> {
        self.stdin.take();
        self.handle.kill_tree().ok()
    }
}

impl Drop for SshProcess {
    fn drop(&mut self) {
        // A session on Unix is not killed by the handle's own drop.
        self.stdin.take();
        let _ = self.handle.start_kill();
    }
}

/// The plugin's own program, the helper ssh starts.
pub fn own_program() -> Option<PathBuf> {
    std::env::current_exe().ok().filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(askpass: bool) -> Launch {
        Launch {
            program: PathBuf::from(if cfg!(windows) {
                r"C:\Windows\System32\OpenSSH\ssh.exe"
            } else {
                "/usr/bin/ssh"
            }),
            args: vec![
                "-s".into(),
                "--".into(),
                "build.example".into(),
                "sftp".into(),
            ],
            askpass: askpass.then(|| AskpassEnv {
                helper: PathBuf::from("/plugins/sftp/diro-sftp"),
                endpoint: "tcp:127.0.0.1:50000".to_owned(),
                token: "00ff".to_owned(),
            }),
        }
    }

    #[test]
    fn the_askpass_environment_is_set_and_inherited_values_removed() {
        let spec = launch(true).spec();
        let (removed, set) = spec.environment_changes();
        for name in [
            "SSH_ASKPASS",
            "SSH_ASKPASS_REQUIRE",
            ENDPOINT_VARIABLE,
            TOKEN_VARIABLE,
        ] {
            assert!(removed.iter().any(|n| n == name), "{name}");
        }
        let get = |name: &str| {
            set.iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("SSH_ASKPASS_REQUIRE").as_deref(), Some("force"));
        assert_eq!(
            get(ENDPOINT_VARIABLE).as_deref(),
            Some("tcp:127.0.0.1:50000")
        );
        assert!(get(TOKEN_VARIABLE).is_some());
        // The secret travels in the environment for the helper, never in
        // the arguments.
        assert!(spec.arguments().iter().all(|a| a != "00ff"));
        assert!(!format!("{:?}", launch(true).askpass).contains("00ff"));
        // A connection that may not ask gets no helper at all.
        let (_, set) = launch(false).spec().environment_changes();
        assert!(set.is_empty());
    }

    #[test]
    fn ssh_never_gets_a_console_window_or_a_terminal() {
        let spec = launch(true).spec();
        if cfg!(windows) {
            assert_eq!(spec.tied(), Tie::Tree);
            assert_eq!(
                spec.creation_flags(),
                diroxide_tools::process::CREATE_NO_WINDOW
            );
        } else {
            assert_eq!(spec.tied(), Tie::Detached);
        }
    }
}
