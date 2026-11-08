//! Windows' own Security dialog for a file or folder (AP's request 1, R11
//! and R29 4.3): the file's Properties sheet opened on its Security page,
//! as File Explorer opens it, through `ShellExecuteExW` with the
//! `properties` verb, the page named in the parameters, and the item
//! list the verb needs. Change owner hands a Windows file to it, since a
//! new owner there is the system's own dialog's work. Started and never
//! waited for; the dialog is the system's, and so is its reading by NVDA
//! and JAWS. Elsewhere there is no such dialog, and the call says so.
//!
//! The call's shape is built apart from the call ([`SecurityCall`]), so it
//! is tested on every system without opening a window.

use std::path::Path;

/// The verb that opens a file's Properties sheet.
pub const PROPERTIES_VERB: &str = "properties";

/// The sheet's page, by its tab's name: the Security page.
pub const SECURITY_PAGE: &str = "Security";

/// What [`open_security_dialog`] hands the shell, before it becomes the
/// shell's structure: the verb, the page, the file, and whether the shell
/// is asked for the item list (the `properties` verb needs it) and to
/// show no message box of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecurityCall {
    /// The verb: `properties`.
    pub verb: &'static str,
    /// The parameters: the page, `Security`.
    pub page: &'static str,
    /// The file or folder, by its full path.
    pub file: std::path::PathBuf,
    /// `SEE_MASK_INVOKEIDLIST`: the verb runs on the item's list.
    pub item_list: bool,
    /// `SEE_MASK_FLAG_NO_UI`: a failure comes back as an error, never as
    /// the shell's own message box.
    pub no_message_box: bool,
}

impl SecurityCall {
    /// The call for one file or folder.
    pub fn for_file(file: &Path) -> Self {
        Self {
            verb: PROPERTIES_VERB,
            page: SECURITY_PAGE,
            file: file.to_path_buf(),
            item_list: true,
            no_message_box: true,
        }
    }
}

/// Opens Windows' Security dialog for `file`, started and never waited
/// for. Elsewhere it says the system has none.
pub fn open_security_dialog(file: &Path) -> std::io::Result<()> {
    let call = SecurityCall::for_file(file);
    #[cfg(windows)]
    {
        shell_execute(&call)
    }
    #[cfg(not(windows))]
    {
        let _ = call;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this system has no Security dialog",
        ))
    }
}

#[cfg(windows)]
fn shell_execute(call: &SecurityCall) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW,
    };

    /// `SW_SHOWNORMAL`: the sheet's window as the system chooses.
    const SHOW_NORMAL: i32 = 1;

    let wide = |text: &std::ffi::OsStr| -> Vec<u16> { text.encode_wide().chain([0]).collect() };
    let verb = wide(std::ffi::OsStr::new(call.verb));
    let page = wide(std::ffi::OsStr::new(call.page));
    let file = wide(call.file.as_os_str());
    let mut mask = 0;
    if call.item_list {
        mask |= SEE_MASK_INVOKEIDLIST;
    }
    if call.no_message_box {
        mask |= SEE_MASK_FLAG_NO_UI;
    }
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: mask,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: page.as_ptr(),
        nShow: SHOW_NORMAL,
        ..Default::default()
    };
    // SAFETY: every string is NUL-terminated and outlives the call; the
    // structure's size is its own.
    let started = unsafe { ShellExecuteExW(&mut info) };
    if started == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The call's shape, on every system, with no window opened: the
    /// Properties verb on the item's list, the Security page, the file
    /// whole, and no message box of the shell's.
    #[test]
    fn the_call_opens_the_properties_sheet_on_its_security_page() {
        let file = Path::new("work").join("main.rs");
        let call = SecurityCall::for_file(&file);
        assert_eq!(call.verb, "properties");
        assert_eq!(call.page, "Security");
        assert_eq!(call.file, file);
        assert!(call.item_list && call.no_message_box);
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_there_is_no_security_dialog() {
        let error = open_security_dialog(Path::new("/tmp/x")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert_eq!(error.to_string(), "this system has no Security dialog");
    }
}
