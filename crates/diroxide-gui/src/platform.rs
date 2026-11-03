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
