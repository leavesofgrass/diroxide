//! Each plugin's log: its standard error, its stray output, its `log`
//! notifications and the host's own notes about it, in a file in the state
//! folder, never on the screen. Two files of 1 MiB at most: when the
//! current one is full it becomes the old one (`NAME.log.1`, replacing the
//! previous old one) and a new current one starts. The last five lines of
//! standard error are also kept in memory for the crash message.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::SystemTime;

use diroxide_core::Timestamp;

use crate::shapes::format_time;

/// The largest a log file grows before it rotates, in bytes.
pub const LOG_FILE_BYTES: u64 = 1024 * 1024;

/// How many lines of standard error the crash message keeps.
pub const TAIL_LINES: usize = 5;

/// The longest line written to the log, in characters; the rest is cut.
pub const LOG_LINE_CHARS: usize = 2000;

/// Where a log line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The plugin's standard error.
    Stderr,
    /// A standard-output line that was not a message.
    Stray,
    /// The plugin's own `log` notification, with its level.
    Plugin(&'static str),
    /// The host, about the plugin.
    Host,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Stderr => "stderr",
            Source::Stray => "stray",
            Source::Plugin(level) => level,
            Source::Host => "host",
        }
    }
}

#[derive(Debug)]
struct Inner {
    path: Option<PathBuf>,
    file: Option<File>,
    size: u64,
    limit: u64,
    tail: VecDeque<String>,
}

/// A plugin's log, shared by the threads that write to it. A log without
/// a path keeps only the tail, for tests and `--no-plugins` sessions.
#[derive(Clone, Debug)]
pub struct PluginLog {
    inner: Arc<Mutex<Inner>>,
}

impl PluginLog {
    /// A log at a path (`NAME.log`); its folder is made when needed.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::with_limit(Some(path.into()), LOG_FILE_BYTES)
    }

    /// A log that keeps only the tail in memory.
    pub fn in_memory() -> Self {
        Self::with_limit(None, LOG_FILE_BYTES)
    }

    /// A log with another rotation size, for tests.
    pub fn with_limit(path: Option<PathBuf>, limit: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path,
                file: None,
                size: 0,
                limit: limit.max(1),
                tail: VecDeque::new(),
            })),
        }
    }

    /// The current file's path, if any.
    pub fn path(&self) -> Option<PathBuf> {
        self.lock().path.clone()
    }

    /// The old file's path (`NAME.log.1`), if any.
    pub fn old_path(&self) -> Option<PathBuf> {
        self.path().map(|p| old_path(&p))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes one line, with the time and its source. A failure to write
    /// the file is ignored: a log must never stop a plugin.
    pub fn write(&self, source: Source, text: &str) {
        let text = cut(text.trim_end_matches(['\r', '\n']));
        let mut inner = self.lock();
        if source == Source::Stderr {
            if inner.tail.len() == TAIL_LINES {
                inner.tail.pop_front();
            }
            inner.tail.push_back(text.clone());
        }
        let line = format!(
            "{} {}: {}\n",
            format_time(Timestamp::from_system_time(SystemTime::now())),
            source.label(),
            text
        );
        let _ = inner.append(line.as_bytes());
    }

    /// The last lines of standard error, oldest first.
    pub fn tail(&self) -> Vec<String> {
        self.lock().tail.iter().cloned().collect()
    }

    /// Starts a thread that copies a stream of standard error into the
    /// log, line by line, lossy, until the stream ends.
    pub fn follow<R: Read + Send + 'static>(&self, stream: R) -> io::Result<()> {
        let log = self.clone();
        thread::Builder::new()
            .name("plugin-stderr".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader
                        .by_ref()
                        .take(LOG_LINE_CHARS as u64 * 4)
                        .read_until(b'\n', &mut line)
                    {
                        Ok(0) | Err(_) => break,
                        Ok(_) => log.write(Source::Stderr, &String::from_utf8_lossy(&line)),
                    }
                }
            })?;
        Ok(())
    }
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(LOG_LINE_CHARS) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

fn old_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

impl Inner {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        if self.file.is_none() {
            if let Some(folder) = path.parent() {
                fs::create_dir_all(folder)?;
            }
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = Some(file);
        }
        if self.size > 0 && self.size + bytes.len() as u64 > self.limit {
            // The current file becomes the old one; the previous old one
            // is replaced, which is the rotation's design.
            self.file = None;
            fs::rename(&path, old_path(&path))?;
            self.file = Some(OpenOptions::new().create(true).append(true).open(&path)?);
            self.size = 0;
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(bytes)?;
            self.size += bytes.len() as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_last_five_lines_of_standard_error() {
        let log = PluginLog::in_memory();
        for i in 0..8 {
            log.write(Source::Stderr, &format!("line {i}\n"));
        }
        log.write(Source::Stray, "not kept in the tail");
        assert_eq!(
            log.tail(),
            ["line 3", "line 4", "line 5", "line 6", "line 7"]
        );
    }

    #[test]
    fn two_files_rotate_at_the_limit() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("logs").join("line-counter.log");
        let log = PluginLog::with_limit(Some(path.clone()), 200);
        for i in 0..20 {
            log.write(Source::Plugin("info"), &format!("message number {i}"));
        }
        let current = fs::read_to_string(&path).unwrap();
        let old = fs::read_to_string(old_path(&path)).unwrap();
        assert!(current.len() <= 200, "{}", current.len());
        assert!(old.len() <= 200, "{}", old.len());
        assert!(current.contains("info: message number 19"), "{current}");
        assert!(!old.contains("message number 19"));
        assert!(!current.contains("message number 0\n"));
    }

    #[test]
    fn standard_error_is_followed_line_by_line() {
        let log = PluginLog::in_memory();
        let stream: &'static [u8] = b"first\r\nsecond\nlast without a break";
        log.follow(stream).unwrap();
        for _ in 0..500 {
            if log.tail().len() == 3 {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(log.tail(), ["first", "second", "last without a break"]);
    }

    #[test]
    fn long_lines_are_cut() {
        let log = PluginLog::in_memory();
        log.write(Source::Stderr, &"y".repeat(5000));
        assert_eq!(log.tail()[0].chars().count(), LOG_LINE_CHARS);
    }
}
