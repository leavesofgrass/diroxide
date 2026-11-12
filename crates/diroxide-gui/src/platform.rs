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
//! - On Windows, the message box that says why the window could not
//!   start when its console was let go (textweaver's `src/safety.rs`).
//! - On Windows, the window's native menu bar ([`menu`]): the Win32 menu
//!   built from the app's menu model, shown and hidden on the window, the
//!   menu loop entered as Windows' own menu keys would, and the window's
//!   procedure subclassed to hear which item was chosen and when the menu
//!   closed.
//! - On Windows, text put on the clipboard ([`clipboard`]), with the
//!   cause in words when another program holds it.
//! - On Linux under Wayland, text put on the clipboard through the
//!   window's own Wayland connection ([`wayland`]), where the window
//!   runner's X11 clipboard does nothing.
//! - On Windows, the system's high contrast colors ([`contrast`]), which
//!   the window draws with while that mode is on.
//! - On Windows, winit's helper window named ([`helper`]), so a screen
//!   reader never meets a nameless pane on the desktop.
//! - On Windows, a program started in a console of its own
//!   ([`win::start_in_new_console`]) for the window's Shell here and Run
//!   command (`crate::spawn`), with its command line quoted as Windows
//!   programs split it; and a program with windows of its own started
//!   through the shell ([`win::start_through_shell`]), as File Explorer
//!   starts it.
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

    /// A message box with an OK button and the error icon, owned by no
    /// window, which waits for the person to close it.
    pub(crate) fn message_box(title: &str, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
        };
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (title, text) = (wide(title), wide(text));
        // SAFETY: both buffers are ours, end with a zero and outlive the
        // call, which only reads them; no owner window is passed.
        unsafe {
            let _ = MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }

    /// A program's command line as Windows programs split it back into
    /// arguments (the C runtime's rules): an argument with a space, a tab
    /// or a quote is quoted, its quotes escaped and the backslashes before
    /// a quote doubled.
    pub(crate) fn command_line(program: &std::ffi::OsStr, args: &[std::ffi::OsString]) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        let mut line: Vec<u16> = Vec::new();
        for (i, arg) in std::iter::once(program)
            .chain(args.iter().map(std::ffi::OsString::as_os_str))
            .enumerate()
        {
            if i > 0 {
                line.push(u16::from(b' '));
            }
            let units: Vec<u16> = arg.encode_wide().collect();
            let plain = !units.is_empty()
                && !units.iter().any(|&c| {
                    c == u16::from(b' ') || c == u16::from(b'\t') || c == u16::from(b'"')
                });
            if plain {
                line.extend(units);
                continue;
            }
            line.push(u16::from(b'"'));
            let mut backslashes = 0usize;
            for c in units {
                if c == u16::from(b'\\') {
                    backslashes += 1;
                    continue;
                }
                let repeat = if c == u16::from(b'"') {
                    backslashes * 2 + 1
                } else {
                    backslashes
                };
                line.extend(std::iter::repeat_n(u16::from(b'\\'), repeat));
                line.push(c);
                backslashes = 0;
            }
            line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
            line.push(u16::from(b'"'));
        }
        line
    }

    /// Starts a program in a console window of its own, which owns its
    /// input and output (the window's handles are not passed on, as the
    /// standard library would), with this process's environment and
    /// `env` over it, in `folder`; not waited for.
    pub(crate) fn start_in_new_console(
        program: &std::ffi::OsStr,
        args: &[std::ffi::OsString],
        folder: &std::path::Path,
        env: &[(String, std::ffi::OsString)],
    ) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, PROCESS_INFORMATION,
            STARTUPINFOW,
        };
        let mut line = command_line(program, args);
        line.push(0);
        let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os()
            .filter(|(name, _)| !env.iter().any(|(n, _)| name.eq_ignore_ascii_case(n)))
            .collect();
        vars.extend(
            env.iter()
                .map(|(name, value)| (std::ffi::OsString::from(name), value.clone())),
        );
        let mut block: Vec<u16> = Vec::new();
        for (name, value) in &vars {
            block.extend(name.encode_wide());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        }
        block.push(0);
        let folder: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: every buffer is ours, ends as CreateProcessW requires
        // (the command line and folder with a zero, the environment block
        // with two) and outlives the call; the command line is writable as
        // the call may change it; no handles are inherited and the
        // startup information names none; the two handles returned are
        // closed here, which leaves the program running.
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
                block.as_ptr().cast(),
                folder.as_ptr(),
                &startup,
                &mut info,
            ) != 0
        };
        if !ok {
            return Err(crate::spawn::plain_cause(&std::io::Error::last_os_error()));
        }
        // SAFETY: the handles are the ones CreateProcessW just gave us.
        unsafe {
            let _ = CloseHandle(info.hThread);
            let _ = CloseHandle(info.hProcess);
        }
        Ok(())
    }

    /// Starts a program with windows of its own through the shell, as
    /// File Explorer does (`ShellExecuteExW`, the open verb, no message
    /// box of the shell's: the app says the cause): its arguments quoted
    /// as Windows programs split them, in `folder`; not waited for. The
    /// shell takes no environment, so the program gets this process's.
    pub(crate) fn start_through_shell(
        program: &std::ffi::OsStr,
        args: &[std::ffi::OsString],
        folder: &std::path::Path,
    ) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
        };
        /// `SW_SHOWNORMAL`: the program's window as the system chooses.
        const SHOW_NORMAL: i32 = 1;
        let wide = |text: &std::ffi::OsStr| -> Vec<u16> { text.encode_wide().chain([0]).collect() };
        let verb = wide(std::ffi::OsStr::new("open"));
        let file = wide(program);
        let mut parameters = match args.split_first() {
            Some((first, rest)) => command_line(first, rest),
            None => Vec::new(),
        };
        parameters.push(0);
        let folder = wide(folder.as_os_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: verb.as_ptr(),
            lpFile: file.as_ptr(),
            lpParameters: parameters.as_ptr(),
            lpDirectory: folder.as_ptr(),
            nShow: SHOW_NORMAL,
            ..Default::default()
        };
        // SAFETY: every string is ours, ends with a zero and outlives the
        // call; the structure's size is its own; no process handle is
        // asked for, so none is left to close.
        let started = unsafe { ShellExecuteExW(&mut info) };
        if started == 0 {
            return Err(crate::spawn::plain_cause(&std::io::Error::last_os_error()));
        }
        Ok(())
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

#[cfg(all(windows, feature = "gui"))]
pub(crate) mod contrast {
    //! Windows High Contrast ("Contrast themes" in Windows 11): whether it
    //! is on, and the system's own colors while it is, which the window
    //! then draws with instead of its theme ([`crate::theme::SystemColors`]).
    //! Two cheap calls, made on every step of the window, so turning the
    //! mode on or off applies at the next key. Adapted from textweaver's
    //! `crates/textweaver-xilem/src/system_colors.rs` (the owner's own
    //! project).

    use diroxide_theme::Rgb;
    use windows_sys::Win32::Graphics::Gdi::{
        COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
        COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
    };
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};

    use crate::theme::SystemColors;

    fn color(index: SYS_COLOR_INDEX) -> Rgb {
        // SAFETY: GetSysColor reads one system color by its index and has
        // no other effect; every index passed is a named constant.
        let bgr = unsafe { GetSysColor(index) };
        Rgb::new(
            (bgr & 0xff) as u8,
            ((bgr >> 8) & 0xff) as u8,
            ((bgr >> 16) & 0xff) as u8,
        )
    }

    /// The system's high contrast colors, or `None` when that mode is off.
    pub(crate) fn colors() -> Option<SystemColors> {
        let mut hc = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: SPI_GETHIGHCONTRAST fills the HIGHCONTRASTW it is given,
        // whose size is in `cbSize` as the call requires; the pointer is
        // to a live local for the call, and no setting is written (no
        // update flags).
        let read = unsafe {
            SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&raw mut hc).cast(), 0)
        };
        if read == 0 || hc.dwFlags & HCF_HIGHCONTRASTON == 0 {
            return None;
        }
        Some(SystemColors {
            window: color(COLOR_WINDOW),
            window_text: color(COLOR_WINDOWTEXT),
            highlight: color(COLOR_HIGHLIGHT),
            highlight_text: color(COLOR_HIGHLIGHTTEXT),
            hot_light: color(COLOR_HOTLIGHT),
            button_face: color(COLOR_BTNFACE),
            button_text: color(COLOR_BTNTEXT),
            gray_text: color(COLOR_GRAYTEXT),
        })
    }
}

#[cfg(all(windows, feature = "gui"))]
pub(crate) mod helper {
    //! winit's helper window ("Winit Thread Event Target"): a window of
    //! this thread that winit keeps visible but transparent, so it gets
    //! the messages that carry events during a resize. UI Automation shows
    //! it as a top-level pane of the desktop with no name, which a screen
    //! reader's object navigation and window lists would meet as a
    //! nameless pane. It is named here, and kept a tool window (out of the
    //! taskbar and the task switcher), right after the event loop makes
    //! it. It is not hidden: winit needs it visible.

    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongPtrW, SetWindowLongPtrW,
        SetWindowTextW, WS_EX_TOOLWINDOW,
    };

    /// The helper window's class, as winit names it.
    const CLASS: &str = "Winit Thread Event Target";

    /// The name the helper window is given, its meaning first.
    pub(crate) const NAME: &str = "diroxide background helper";

    unsafe extern "system" fn visit(hwnd: HWND, found: LPARAM) -> i32 {
        let mut class = [0u16; 64];
        // SAFETY: the buffer is ours and its length is passed; the call
        // writes at most that many units.
        let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        if class == CLASS {
            // SAFETY: `found` is the address of the caller's local, which
            // outlives the enumeration.
            unsafe { *(found as *mut HWND) = hwnd };
            return 0;
        }
        1
    }

    /// Names this thread's helper window and keeps it a tool window.
    /// True when it was found.
    pub(crate) fn name() -> bool {
        let mut found: HWND = std::ptr::null_mut();
        // SAFETY: enumerates this thread's windows with a plain callback;
        // `found` is a local the callback writes, alive for the call.
        unsafe {
            let _ = EnumThreadWindows(GetCurrentThreadId(), Some(visit), &raw mut found as LPARAM);
        }
        if found.is_null() {
            return false;
        }
        let title: Vec<u16> = NAME.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `found` is a window of this thread that winit made and
        // keeps; the title is ours, ends with a zero, and outlives the
        // call, which copies it; the style is read and written back with
        // one more bit.
        unsafe {
            let _ = SetWindowTextW(found, title.as_ptr());
            let style = GetWindowLongPtrW(found, GWL_EXSTYLE);
            let tool = WS_EX_TOOLWINDOW as isize;
            if style & tool == 0 {
                let _ = SetWindowLongPtrW(found, GWL_EXSTYLE, style | tool);
            }
        }
        true
    }
}

#[cfg(all(unix, not(target_os = "macos"), feature = "gui"))]
pub(crate) mod wayland {
    //! Copied text on Wayland (alpha 4, R30 commit 113; alpha 3's known
    //! gap). The window runner opens only the X11 clipboard, which does
    //! nothing where there is no XWayland; here the window's own Wayland
    //! connection gets a clipboard through copypasta's Wayland half
    //! (smithay-clipboard, libwayland opened at run time), the crate the
    //! runner already uses, as Alacritty does with winit.
    //!
    //! A Wayland compositor gives the selection only to a client that has
    //! had an input event, so the clipboard must share the window's
    //! connection: a second connection could not set it. The connection is
    //! kept alive by the event loop's owned display handle, held here and
    //! dropped after the clipboard, whose drop joins its thread, so the
    //! clipboard never outlives the display it was made on.

    use copypasta::ClipboardProvider;
    use copypasta::wayland_clipboard::{Clipboard as Selection, create_clipboards_from_external};
    use masonry_winit::winit::event_loop::OwnedDisplayHandle;
    use masonry_winit::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

    /// The clipboard on the window's Wayland connection.
    pub(crate) struct Clipboard {
        // Field order is drop order: the selection (and its thread) goes
        // before the display handle that keeps the connection open.
        selection: Selection,
        _display: OwnedDisplayHandle,
    }

    impl Clipboard {
        /// The clipboard for the event loop's display, when it is a
        /// Wayland display; `None` on X11, where the runner's clipboard
        /// serves.
        pub(crate) fn open(display: OwnedDisplayHandle) -> Option<Self> {
            let raw = match display.display_handle().ok()?.as_raw() {
                RawDisplayHandle::Wayland(handle) => handle.display.as_ptr(),
                _ => return None,
            };
            // SAFETY: `raw` is the `wl_display` of the event loop's own
            // Wayland connection, a valid display; `display`, kept in the
            // returned value and dropped after the selection, holds that
            // connection open for as long as the selection (and the thread
            // it starts, joined when it drops) uses it. The primary
            // selection made with it is dropped at once.
            let (_primary, selection) = unsafe { create_clipboards_from_external(raw) };
            Some(Self {
                selection,
                _display: display,
            })
        }

        /// Puts `text` on the clipboard; the cause in words when it could
        /// not.
        pub(crate) fn set_text(&mut self, text: &str) -> Result<(), String> {
            self.selection
                .set_contents(text.to_owned())
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(all(windows, feature = "gui"))]
pub(crate) mod clipboard {
    //! Text put on the Windows clipboard, written here rather than through
    //! the window runner's clipboard, which ends the window when the
    //! clipboard cannot be opened (another program holding it for a
    //! moment); here that is a sentence and the window goes on. Each call
    //! is a plain Win32 call; the memory block is ours until the
    //! clipboard takes it.

    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };

    /// The clipboard's format for UTF-16 text.
    const CF_UNICODETEXT: u32 = 13;

    /// Puts `text` on the clipboard as Unicode text; the cause in words
    /// when it could not.
    pub(crate) fn set_text(text: &str) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let bytes = wide.len() * 2;
        // Another program may hold the clipboard for a moment, so opening
        // it is tried a few times.
        let mut opened = false;
        for _ in 0..10 {
            // SAFETY: opens the clipboard with no owner window; closed
            // below on every path.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            return Err("another program is using it".to_owned());
        }
        // SAFETY: the clipboard is open (and closed at the end); the block
        // is allocated with the size of `wide`, locked while `wide` is
        // copied into it, unlocked, and handed to the clipboard, which
        // owns it from then on; it is freed here only when the clipboard
        // did not take it.
        let result = unsafe {
            if EmptyClipboard() == 0 {
                Err("it could not be emptied".to_owned())
            } else {
                let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if block.is_null() {
                    Err("there was no memory for the text".to_owned())
                } else {
                    let to = GlobalLock(block).cast::<u16>();
                    if to.is_null() {
                        let _ = GlobalFree(block);
                        Err("there was no memory for the text".to_owned())
                    } else {
                        std::ptr::copy_nonoverlapping(wide.as_ptr(), to, wide.len());
                        let _ = GlobalUnlock(block);
                        if SetClipboardData(CF_UNICODETEXT, block).is_null() {
                            let _ = GlobalFree(block);
                            Err("it did not take the text".to_owned())
                        } else {
                            Ok(())
                        }
                    }
                }
            }
        };
        // SAFETY: closes the clipboard this function opened.
        unsafe {
            let _ = CloseClipboard();
        }
        result
    }
}

#[cfg(all(windows, feature = "gui"))]
pub(crate) mod menu {
    //! The window's Win32 menu bar (textweaver's ADR-0046, written on
    //! windows-sys rather than muda, which the workspace does not have).
    //! Each call is a plain Win32 call with its arguments by value or a
    //! buffer of ours; the menu handles are ours until a window holds
    //! them.

    use std::sync::{Mutex, PoisonError};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateMenu, CreatePopupMenu, DestroyMenu, DrawMenuBar, HMENU, MF_POPUP,
        MF_STRING, PostMessageW, SC_KEYMENU, SetMenu, WM_COMMAND, WM_EXITMENULOOP, WM_SYSCOMMAND,
    };

    use crate::menus::{Entry, TopMenu};

    /// What the menu bar tells the window.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum MenuEvent {
        /// The item with this id was chosen.
        Chosen(u16),
        /// The menu closed (chosen or not).
        Closed,
    }

    /// Where the subclassed window procedure sends the menu's events.
    type Sink = Box<dyn Fn(MenuEvent) + Send>;
    static SINK: Mutex<Option<Sink>> = Mutex::new(None);

    /// The id the window's subclass has.
    const SUBCLASS_ID: usize = 0x6469_726f; // "diro"

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// A built menu bar and the commands its item ids stand for (id 1 is
    /// the first command).
    pub(crate) struct Bar {
        hmenu: HMENU,
        commands: Vec<String>,
        /// The window that holds it now, if any.
        on: Option<isize>,
    }

    fn append(menu: HMENU, entries: &[Entry], commands: &mut Vec<String>) {
        for entry in entries {
            let text = wide(&entry.windows_text());
            match entry {
                Entry::Item { command, .. } => {
                    commands.push(command.clone());
                    let id = commands.len();
                    // SAFETY: `menu` is a menu this module made; `text` is
                    // ours, ends with a zero, and outlives the call, which
                    // copies it.
                    unsafe {
                        let _ = AppendMenuW(menu, MF_STRING, id, text.as_ptr());
                    }
                }
                Entry::Submenu { entries, .. } => {
                    // SAFETY: CreatePopupMenu takes nothing; the popup is
                    // handed to `menu`, which destroys it with itself.
                    unsafe {
                        let popup = CreatePopupMenu();
                        if popup.is_null() {
                            continue;
                        }
                        append(popup, entries, commands);
                        let _ =
                            AppendMenuW(menu, MF_STRING | MF_POPUP, popup as usize, text.as_ptr());
                    }
                }
            }
        }
    }

    impl Bar {
        /// The menu bar for `tree`, not yet on a window; `None` when the
        /// system would not make a menu.
        pub(crate) fn build(tree: &[TopMenu]) -> Option<Self> {
            // SAFETY: CreateMenu takes nothing and returns a handle that
            // is ours until a window holds it or it is destroyed.
            let hmenu = unsafe { CreateMenu() };
            if hmenu.is_null() {
                return None;
            }
            let mut commands = Vec::new();
            for top in tree {
                let text = wide(&top.label);
                // SAFETY: as in `append`.
                unsafe {
                    let popup = CreatePopupMenu();
                    if popup.is_null() {
                        continue;
                    }
                    append(popup, &top.entries, &mut commands);
                    let _ = AppendMenuW(hmenu, MF_STRING | MF_POPUP, popup as usize, text.as_ptr());
                }
            }
            Some(Self {
                hmenu,
                commands,
                on: None,
            })
        }

        /// The command an item id stands for.
        pub(crate) fn command(&self, id: u16) -> Option<&str> {
            self.commands
                .get(usize::from(id).checked_sub(1)?)
                .map(String::as_str)
        }

        /// Every item's text as the menu holds it, depth first, each with
        /// its depth (0 for the bar's menus): what UI Automation reads.
        #[cfg(test)]
        pub(crate) fn texts(&self) -> Vec<(usize, String)> {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetMenuItemCount, GetMenuStringW, GetSubMenu, MF_BYPOSITION,
            };
            fn walk(menu: HMENU, depth: usize, out: &mut Vec<(usize, String)>) {
                // SAFETY: `menu` is a live menu of this bar; the buffer is
                // ours and its length is passed; GetSubMenu only reads.
                unsafe {
                    let n = GetMenuItemCount(menu);
                    for i in 0..n.max(0) {
                        let mut buf = [0u16; 256];
                        let len = GetMenuStringW(
                            menu,
                            i as u32,
                            buf.as_mut_ptr(),
                            buf.len() as i32,
                            MF_BYPOSITION,
                        );
                        let text = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
                        out.push((depth, text));
                        let sub = GetSubMenu(menu, i);
                        if !sub.is_null() {
                            walk(sub, depth + 1, out);
                        }
                    }
                }
            }
            let mut out = Vec::new();
            walk(self.hmenu, 0, &mut out);
            out
        }

        /// Shows the bar on the window `hwnd`.
        pub(crate) fn attach(&mut self, hwnd: isize) -> bool {
            // SAFETY: `hwnd` is the window's handle, given by winit for
            // the window this process made; SetMenu and DrawMenuBar take
            // it and our menu by value.
            let ok = unsafe {
                let ok = SetMenu(hwnd as HWND, self.hmenu) != 0;
                let _ = DrawMenuBar(hwnd as HWND);
                ok
            };
            if ok {
                self.on = Some(hwnd);
            }
            ok
        }

        /// Takes the bar off its window (hidden); the menu stays ours.
        pub(crate) fn detach(&mut self) {
            if let Some(hwnd) = self.on.take() {
                // SAFETY: as in `attach`; a null menu takes the bar away
                // without destroying it.
                unsafe {
                    let _ = SetMenu(hwnd as HWND, std::ptr::null_mut());
                    let _ = DrawMenuBar(hwnd as HWND);
                }
            }
        }

        /// Whether the bar is on a window now.
        pub(crate) fn shown(&self) -> bool {
            self.on.is_some()
        }
    }

    impl Drop for Bar {
        fn drop(&mut self) {
            self.detach();
            // SAFETY: the menu is ours again (detached), so destroying it
            // and its popups frees nothing a window still uses.
            unsafe {
                let _ = DestroyMenu(self.hmenu);
            }
        }
    }

    /// Enters the window's menu bar as Windows' own menu keys do: with no
    /// letter the bar's first menu is selected (Alt alone, F10); with a
    /// letter, the menu with that access key opens.
    pub(crate) fn enter(hwnd: isize, letter: Option<char>) {
        let key = letter.map_or(0, |c| c as usize as isize);
        // SAFETY: posts a message to our own window by value.
        unsafe {
            let _ = PostMessageW(
                hwnd as HWND,
                WM_SYSCOMMAND,
                SC_KEYMENU as WPARAM,
                key as LPARAM,
            );
        }
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        let event = match message {
            // A menu's command: the high word of wParam is 0 and lParam
            // names no control.
            WM_COMMAND if (wparam >> 16) & 0xffff == 0 && lparam == 0 => {
                Some(MenuEvent::Chosen((wparam & 0xffff) as u16))
            }
            WM_EXITMENULOOP => Some(MenuEvent::Closed),
            _ => None,
        };
        if let Some(event) = event {
            let sink = SINK.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(sink) = sink.as_ref() {
                sink(event);
            }
            if matches!(event, MenuEvent::Chosen(_)) {
                return 0;
            }
        }
        // SAFETY: the message goes on to the window's own procedure (and
        // winit's), as DefSubclassProc is meant to be called.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// Listens to the window's menu messages, sending each to `sink`.
    /// True when the window's procedure was subclassed.
    pub(crate) fn watch(hwnd: isize, sink: Sink) -> bool {
        *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(sink);
        // SAFETY: `hwnd` is our window; the procedure is a plain function
        // that lives as long as the program, and passes every message on.
        unsafe { SetWindowSubclass(hwnd as HWND, Some(procedure), SUBCLASS_ID, 0) != 0 }
    }
}
