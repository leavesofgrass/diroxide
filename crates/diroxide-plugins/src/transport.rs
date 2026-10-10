//! How the host talks to a plugin: lines in, lines out. Today a process's
//! standard input and output; a WebAssembly component can be a second
//! transport later without changing the plugin API.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::error::PluginError;

/// Sends and receives protocol lines.
pub trait PluginTransport: Send {
    /// Sends one line; the transport adds the line break.
    fn send(&mut self, line: &str) -> Result<(), PluginError>;

    /// Waits up to `timeout` for one line; `Ok(None)` when none came.
    fn receive(&mut self, timeout: Duration) -> Result<Option<String>, PluginError>;

    /// Ends the conversation and stops the plugin.
    fn close(&mut self) -> Result<(), PluginError>;
}

/// A plugin process spoken to over standard input and output. A reader
/// thread forwards its output lines, so a receive can time out.
#[derive(Debug)]
pub struct StdioTransport {
    name: String,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Result<String, String>>,
}

impl StdioTransport {
    /// Starts a plugin's program in its folder, with no shell: the first
    /// item of `program` is the executable and the rest its arguments.
    /// Standard error goes to the host's error output for now; Wave 3
    /// sends it to the plugin's log.
    pub fn spawn(name: &str, program: &[String], folder: &Path) -> Result<Self, PluginError> {
        let start_error = |message: String| PluginError::Start {
            plugin: name.to_owned(),
            message,
        };
        let (executable, args) = program
            .split_first()
            .ok_or_else(|| start_error("the program is empty".to_owned()))?;
        let mut child = Command::new(executable)
            .args(args)
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| start_error(e.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| start_error("its output could not be read".to_owned()))?;
        let stdin = child.stdin.take();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let failed = line.is_err();
                if sender.send(line.map_err(|e| e.to_string())).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            name: name.to_owned(),
            child,
            stdin,
            lines,
        })
    }
}

impl PluginTransport for StdioTransport {
    fn send(&mut self, line: &str) -> Result<(), PluginError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::Stopped(self.name.clone()))?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|_| PluginError::Stopped(self.name.clone()))
    }

    fn receive(&mut self, timeout: Duration) -> Result<Option<String>, PluginError> {
        match self.lines.recv_timeout(timeout) {
            Ok(Ok(line)) => Ok(Some(line)),
            Ok(Err(message)) => Err(PluginError::Io(message)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(PluginError::Stopped(self.name.clone())),
        }
    }

    fn close(&mut self) -> Result<(), PluginError> {
        self.stdin = None;
        match self.child.try_wait() {
            Ok(Some(_)) => Ok(()),
            _ => self
                .child
                .kill()
                .and_then(|()| self.child.wait().map(|_| ()))
                .map_err(|e| PluginError::Io(e.to_string())),
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
