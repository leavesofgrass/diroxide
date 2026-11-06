//! A reader that may not cross threads (one that counts its references
//! without atomics, as `ext4-view` does), kept on a thread of its own: the
//! plugin's sixteen workers hand it work and wait for the answer, so the
//! reader is opened once and never moved.
//!
//! The waiting request's cancel flag travels with the work, so a host
//! read made on the reader's thread still stops when the request is
//! cancelled. The thread ends when the volume is closed.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::error::DiskError;
use crate::host::{self, Serving};

type Work<T> = Box<dyn FnOnce(&mut T) + Send>;

/// A value that lives on its own thread.
pub struct Pinned<T> {
    tx: Mutex<Sender<Work<T>>>,
}

impl<T: 'static> Pinned<T> {
    /// Makes the value on a new thread named `name`; its making's error
    /// comes back here.
    pub fn spawn(
        name: &str,
        make: impl FnOnce() -> Result<T, DiskError> + Send + 'static,
    ) -> Result<Self, DiskError> {
        let (tx, rx) = mpsc::channel::<Work<T>>();
        let (made_tx, made_rx) = mpsc::channel::<Result<(), DiskError>>();
        let cancel = host::serving();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let serving = cancel.as_ref().map(Serving::start);
                let mut value = match make() {
                    Ok(value) => {
                        let _ = made_tx.send(Ok(()));
                        value
                    }
                    Err(error) => {
                        let _ = made_tx.send(Err(error));
                        return;
                    }
                };
                drop(serving);
                while let Ok(work) = rx.recv() {
                    work(&mut value);
                }
            })
            .map_err(|e| DiskError::Io(e.to_string()))?;
        made_rx
            .recv()
            .map_err(|_| DiskError::Io("the reader's thread stopped".to_owned()))??;
        Ok(Self { tx: Mutex::new(tx) })
    }

    /// Runs `f` on the value's thread and gives back its answer.
    pub fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut T) -> R + Send + 'static,
    ) -> Result<R, DiskError> {
        let (answer_tx, answer_rx) = mpsc::channel();
        let cancel = host::serving();
        let work: Work<T> = Box::new(move |value: &mut T| {
            let _serving = cancel.as_ref().map(Serving::start);
            let _ = answer_tx.send(f(value));
        });
        self.tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(work)
            .map_err(|_| DiskError::Io("the reader's thread stopped".to_owned()))?;
        answer_rx
            .recv()
            .map_err(|_| DiskError::Io("the reader's thread stopped".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn a_value_that_cannot_cross_threads_serves_from_its_own() {
        let pinned = Pinned::spawn("test-pinned", || Ok(Rc::new(41))).unwrap();
        let answer = pinned.run(|value| **value + 1).unwrap();
        assert_eq!(answer, 42);
        let failed: Result<Pinned<Rc<u8>>, _> =
            Pinned::spawn("test-pinned", || Err(DiskError::NotFound));
        assert!(matches!(failed, Err(DiskError::NotFound)));
    }
}
