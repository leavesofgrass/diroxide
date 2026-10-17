//! Resident memory, for the spikes: the process's resident size from the
//! system (the working set on Windows), sampled on a thread so a short
//! peak is seen. Each test runs in its own process under nextest, so the
//! figures are the test's own.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// The resident size of this process now, in bytes.
pub fn resident() -> u64 {
    let Ok(pid) = sysinfo::get_current_pid() else {
        return 0;
    };
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    system.process(pid).map_or(0, sysinfo::Process::memory)
}

/// Watches the resident size until stopped and keeps the largest seen.
pub struct Peak {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Peak {
    /// Starts watching, every 5 milliseconds.
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(resident()));
        let thread = {
            let stop = stop.clone();
            let peak = peak.clone();
            thread::spawn(move || {
                let Ok(pid) = sysinfo::get_current_pid() else {
                    return;
                };
                let mut system = System::new();
                while !stop.load(Ordering::Relaxed) {
                    system.refresh_processes_specifics(
                        ProcessesToUpdate::Some(&[pid]),
                        true,
                        ProcessRefreshKind::nothing().with_memory(),
                    );
                    if let Some(process) = system.process(pid) {
                        peak.fetch_max(process.memory(), Ordering::Relaxed);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    /// Stops watching and gives the largest resident size seen, in bytes.
    pub fn stop(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.peak.load(Ordering::Relaxed).max(resident())
    }
}

/// Mebibytes, for the reports.
pub fn mib(bytes: u64) -> f64 {
    bytes as f64 / f64::from(1u32 << 20)
}
