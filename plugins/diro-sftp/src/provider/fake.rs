//! A test double: an [`Opener`] whose sessions are the fake SFTP server
//! (`sftp::fake`) in memory, with no ssh at all, so the provider's
//! methods can be tested against the fake's fault modes in CI. Each link
//! it opens can be cut on purpose, as a dropped network would.
//!
//! Public so this crate's integration tests and other lanes' tests use the
//! same one; the plugin itself never uses it.

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_plugins::protocol::{ErrorKind, RpcError};

use super::connect::{OpenRequest, Opened, Opener};
use crate::sftp::fake::{EndReader, FakeFs, Faults, spawn};
use crate::sftp::session::{Session, SessionConfig};

/// A reader that ends when its switch is thrown, as a stream does when
/// the network drops.
struct Cuttable {
    rx: Receiver<Vec<u8>>,
    cut: Arc<AtomicBool>,
    buffer: Vec<u8>,
}

impl Cuttable {
    fn new(mut inner: EndReader, cut: Arc<AtomicBool>) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                match inner.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if tx.send(chunk[..n].to_vec()).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Self {
            rx,
            cut,
            buffer: Vec::new(),
        }
    }
}

impl Read for Cuttable {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.buffer.is_empty() {
            if self.cut.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(bytes) => self.buffer = bytes,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        if self.cut.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let n = out.len().min(self.buffer.len());
        out[..n].copy_from_slice(&self.buffer[..n]);
        self.buffer.drain(..n);
        Ok(n)
    }
}

/// What a test sees of the sessions a [`FakeOpener`] opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    /// Whether the owner was looking.
    pub interactive: bool,
    /// Whether it was a connection's second session.
    pub second: bool,
}

/// Opens sessions on the fake server in memory.
#[derive(Debug)]
pub struct FakeOpener {
    fs: Arc<Mutex<FakeFs>>,
    faults: Mutex<Faults>,
    config: SessionConfig,
    cuts: Mutex<Vec<Arc<AtomicBool>>>,
    openings: Mutex<Vec<Opening>>,
    refuse: AtomicUsize,
    typed: AtomicBool,
}

impl FakeOpener {
    /// An opener over `fs` with `faults`.
    pub fn new(fs: Arc<Mutex<FakeFs>>, faults: Faults) -> Arc<Self> {
        Arc::new(Self {
            fs,
            faults: Mutex::new(faults),
            config: SessionConfig {
                call_timeout: Duration::from_secs(5),
                version_timeout: Duration::from_secs(5),
                ..SessionConfig::default()
            },
            cuts: Mutex::new(Vec::new()),
            openings: Mutex::new(Vec::new()),
            refuse: AtomicUsize::new(0),
            typed: AtomicBool::new(false),
        })
    }

    /// The file system it serves.
    pub fn fs(&self) -> &Arc<Mutex<FakeFs>> {
        &self.fs
    }

    /// Changes the faults of the sessions it opens from now on.
    pub fn set_faults(&self, faults: Faults) {
        *self.faults.lock().unwrap_or_else(PoisonError::into_inner) = faults;
    }

    /// Refuses the next `times` openings, as an unreachable server does.
    pub fn refuse_next(&self, times: usize) {
        self.refuse.store(times, Ordering::SeqCst);
    }

    /// Makes every opening count as one where the owner typed a secret.
    pub fn as_if_typed(&self, typed: bool) {
        self.typed.store(typed, Ordering::SeqCst);
    }

    /// Cuts every link it opened, as a dropped network does.
    pub fn cut_all(&self) {
        for cut in self
            .cuts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            cut.store(true, Ordering::SeqCst);
        }
    }

    /// Every opening so far, in order.
    pub fn openings(&self) -> Vec<Opening> {
        self.openings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Opener for FakeOpener {
    fn open(&self, request: &OpenRequest<'_>) -> Result<Opened, RpcError> {
        self.openings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Opening {
                interactive: request.interactive,
                second: request.second,
            });
        let refused = self
            .refuse
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if refused {
            return Err(RpcError::new(
                ErrorKind::Unavailable,
                format!("Cannot connect to {}.", request.target.host()),
            ));
        }
        if self.typed.load(Ordering::SeqCst) {
            request.memory.mark_typed();
        }
        let faults = self
            .faults
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (input, output) = spawn(Arc::clone(&self.fs), faults);
        let cut = Arc::new(AtomicBool::new(false));
        self.cuts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(&cut));
        let session = Session::start(Cuttable::new(input, cut), output, &self.config)
            .map_err(|e| RpcError::new(ErrorKind::Unavailable, e.to_string()))?;
        Ok(Opened {
            session,
            process: None,
            listener: None,
            ssh: Some("the fake server".to_owned()),
        })
    }
}
