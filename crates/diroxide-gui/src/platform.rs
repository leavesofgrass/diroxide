//! The one module of the GUI crate that may use `unsafe` (the GUI alpha's
//! contract step; the workspace denies it everywhere else but the
//! platform module of `diroxide-vfs`). Each block is one system call with
//! no pointer kept past it, and says why it is sound.
//!
//! - The process's environment, set once before any other thread starts,
//!   for what wgpu reads at the window's start ([`set_env_before_threads`]).
//! - On Windows, the console calls behind [`crate::console`]: how many
//!   programs share this console, letting it go, a new one for the
//!   terminal when the window cannot start, and the times that measure how
//!   long a console stood before it was let go. Adapted from textweaver's
//!   `crates/textweaver-xilem/src/console.rs`, the owner's own project.
//!
//! Elsewhere the console calls do nothing: a program started from a
//! terminal on Linux or macOS writes to it, and one started from the
//! desktop has none.

#![allow(unsafe_code)]

/// Sets an environment variable of this process.
///
/// Only for the start of `diro`, before the app, its runtime or the window
/// start a thread; [`crate::prepare`] says so to its caller.
pub(crate) fn set_env_before_threads(name: &str, value: &str) {
    // SAFETY: called from `diro`'s main thread before the app, the runtime
    // or the window exists, so no other thread reads or writes the
    // environment at the same time (the reason `set_var` is unsafe on
    // Unix). On Windows the call is also synchronized by the system.
    unsafe {
        std::env::set_var(name, value);
    }
}

#[cfg(windows)]
pub(crate) mod win {
    //! Win32 console calls, each a plain call with its arguments by value.

    use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_CHAR, GetFileType};
    use windows_sys::Win32::System::Console::{
        AllocConsole, FreeConsole, GetConsoleMode, GetConsoleProcessList, GetStdHandle,
        STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    /// The three standard handles, in order.
    pub(crate) const STANDARD: [STD_HANDLE; 3] =
        [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];

    /// How many processes are attached to this process's console, this one
    /// included; 0 when it has no console.
    pub(crate) fn console_process_count() -> u32 {
        let mut ids = [0u32; 4];
        // SAFETY: the buffer is ours and its length is passed; the call
        // writes at most that many ids and returns the total count.
        unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) }
    }

    /// The standard handle, or `None` when it is not set.
    pub(crate) fn std_handle(which: STD_HANDLE) -> Option<HANDLE> {
        // SAFETY: GetStdHandle only reads this process's handle table.
        let handle = unsafe { GetStdHandle(which) };
        (!handle.is_null() && handle as isize != -1).then_some(handle)
    }

    /// Whether the handle is a console (not a pipe or a file).
    pub(crate) fn is_console(handle: HANDLE) -> bool {
        let mut mode = 0u32;
        // SAFETY: both calls take the handle by value and only query it;
        // `mode` is a local the second call writes.
        unsafe { GetFileType(handle) == FILE_TYPE_CHAR && GetConsoleMode(handle, &mut mode) != 0 }
    }

    /// Lets go of the console, and clears the standard handles that
    /// pointed at it (`consoles`), so a later [`alloc`] sets them again;
    /// handles redirected to a pipe or a file are kept. True when the
    /// console was let go.
    pub(crate) fn free(consoles: &[STD_HANDLE]) -> bool {
        // SAFETY: FreeConsole detaches this process from its console;
        // SetStdHandle clears handles of that console, which are no longer
        // valid in this process.
        unsafe {
            if FreeConsole() == 0 {
                return false;
            }
            for which in consoles {
                let _ = SetStdHandle(*which, std::ptr::null_mut());
            }
        }
        true
    }

    /// A new console for this process, keeping the standard handles that
    /// were redirected (`kept`). True when it was made.
    pub(crate) fn alloc(kept: &[(STD_HANDLE, HANDLE)]) -> bool {
        // SAFETY: AllocConsole makes a console and points the standard
        // handles at it; SetStdHandle puts back handles this process
        // already owns.
        unsafe {
            if AllocConsole() == 0 {
                return false;
            }
            for (which, handle) in kept {
                let _ = SetStdHandle(*which, *handle);
            }
        }
        true
    }

    /// Milliseconds since this process was created, from the system's
    /// clock; `None` when the times cannot be read.
    pub(crate) fn ms_since_process_start() -> Option<u64> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user, mut now) =
            (zero, zero, zero, zero, zero);
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
        // closing; GetProcessTimes and GetSystemTimeAsFileTime write the
        // locals passed to them.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            ) != 0
        };
        if !ok {
            return None;
        }
        // SAFETY: as above; writes the local.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        // FILETIME counts 100-nanosecond intervals.
        ticks(now).checked_sub(ticks(created)).map(|d| d / 10_000)
    }
}
