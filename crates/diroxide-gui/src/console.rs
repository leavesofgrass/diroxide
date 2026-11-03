//! One `diro` program on Windows (spike S8 of the GUI alpha): the console
//! a window does not need.
//!
//! `diro.exe` is a console program, so a shell waits for it, `diro ls |
//! findstr` and `diro serve --stdio` keep their pipes, and `diro --tui`
//! draws in the terminal it was started from, as before the GUI. Started
//! from Explorer, a shortcut or the Run box, Windows gives it a console of
//! its own before `main` runs. When the GUI opens, [`release_for_window`]
//! lets go of that console, so its window closes; that console is what
//! flashes, and the time it stood is measured
//! ([`Released::stood_ms`]). A console shared with a shell is kept: the
//! shell waits for the window, its errors print there, and Control C there
//! ends it, as for any program started in the foreground.
//!
//! When the window cannot start after its console was let go,
//! [`regain_for_terminal`] makes a new one, so `diro` can say why and open
//! the terminal there. Standard handles redirected to a pipe or a file are
//! kept throughout.
//!
//! Elsewhere these do nothing: a program started from a terminal on Linux
//! or macOS writes to it, and one started from the desktop has none.

use std::io::Write;
use std::path::Path;

/// What [`release_for_window`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Released {
    /// True when the console was let go (`diro` was alone on it).
    pub freed: bool,
    /// How long the console stood, in milliseconds from the process's
    /// start to the moment it was let go; `None` when nothing was let go
    /// or the system's times could not be read.
    pub stood_ms: Option<u64>,
}

/// The environment variable that names a file for the window's start
/// times (the console let go, the window shown), one line each, for the
/// bench and spike S8. Unset, nothing is written.
pub const TIMES_VAR: &str = "DIROXIDE_GUI_TIMES";

/// Appends a line to the file [`TIMES_VAR`] names, when it is set; a file
/// that cannot be written is left alone (the times are a measurement, never
/// a reason to stop).
pub fn note_time(line: &str) {
    let Some(file) = std::env::var_os(TIMES_VAR).filter(|v| !v.is_empty()) else {
        return;
    };
    let file = Path::new(&file);
    if let Ok(mut out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = writeln!(out, "{line}");
    }
}

/// Whether `diro` is the only program on its console: true when Windows
/// made the console for it (Explorer, a shortcut, the Run box), false
/// when a shell or another program shares it, and false on other systems.
pub fn alone_on_console() -> bool {
    imp::alone_on_console()
}

/// Lets go of the console when `diro` is alone on it and the GUI is
/// opening, so the console's window closes; handles redirected to a pipe
/// or a file are kept. Writes the time the console stood to the file
/// [`TIMES_VAR`] names. Does nothing when a shell shares the console, and
/// elsewhere than Windows.
pub fn release_for_window() -> Released {
    let released = imp::release_for_window();
    if released.freed {
        match released.stood_ms {
            Some(ms) => note_time(&format!("console let go {ms} ms after the process started")),
            None => note_time("console let go; the process's start time was not readable"),
        }
    }
    released
}

/// After [`release_for_window`] let go of the console and the window could
/// not start: a new console, so `diro` can say why and open the terminal
/// in it. True when there is a console to use.
pub fn regain_for_terminal(released: Released) -> bool {
    if !released.freed {
        return true;
    }
    imp::regain_for_terminal()
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::Released;
    use crate::platform::win;

    /// True while the console is let go.
    static FREED: AtomicBool = AtomicBool::new(false);

    pub(super) fn alone_on_console() -> bool {
        win::console_process_count() == 1
    }

    pub(super) fn release_for_window() -> Released {
        if !alone_on_console() || FREED.load(Ordering::Acquire) {
            return Released::default();
        }
        let consoles: Vec<_> = win::STANDARD
            .into_iter()
            .filter(|which| win::std_handle(*which).is_some_and(win::is_console))
            .collect();
        if !win::free(&consoles) {
            return Released::default();
        }
        FREED.store(true, Ordering::Release);
        Released {
            freed: true,
            stood_ms: win::ms_since_process_start(),
        }
    }

    pub(super) fn regain_for_terminal() -> bool {
        if !FREED.swap(false, Ordering::AcqRel) {
            return true;
        }
        let kept: Vec<_> = win::STANDARD
            .into_iter()
            .filter_map(|which| win::std_handle(which).map(|handle| (which, handle)))
            .collect();
        win::alloc(&kept)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Released;

    pub(super) fn alone_on_console() -> bool {
        false
    }

    pub(super) fn release_for_window() -> Released {
        Released::default()
    }

    pub(super) fn regain_for_terminal() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test runs under cargo, which shares the console (or has none), so
    /// nothing is let go and nothing needs regaining.
    #[test]
    fn a_shared_console_is_kept() {
        let released = release_for_window();
        assert_eq!(released, Released::default());
        assert!(regain_for_terminal(released));
        assert!(!alone_on_console());
    }
}
