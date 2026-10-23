//! Windows: a named pipe only the owner's own account can open (plan
//! answer 101; R24 section 5.3), the askpass endpoint of the SFTP
//! plugin, which also keeps its two-way proof as a second layer.
//!
//! - **Owner only.** The pipe's security descriptor is a protected DACL
//!   with one entry: full access for the account the process runs as.
//!   No group, no SYSTEM, no Administrators, no inherited entry.
//! - **First instance.** The first instance is created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a process that made a pipe of
//!   the same name before cannot squat it: the creation fails instead.
//! - **Local clients only.** `PIPE_REJECT_REMOTE_CLIENTS` refuses any
//!   client from another computer, and a name is always a plain word
//!   under `\\.\pipe\`, the local namespace.
//! - **A client that cannot be impersonated.** The client end opens with
//!   `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so a server that
//!   squatted a name could at most identify it, never act as it.
//! - **Timeouts.** Both ends use overlapped I/O with an event: a read, a
//!   write or a wait for a client that takes too long is cancelled and
//!   answers `TimedOut`, so no thread is left stuck.
//!
//! Every `unsafe` block says why it is sound and is reviewed at
//! integration with the rest of this module.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};

/// The local namespace of named pipes.
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// The longest pipe name taken, in characters.
pub const MAX_NAME_CHARS: usize = 200;

/// The size of each instance's buffers, in bytes.
const BUFFER: u32 = 4096;

/// Whether a pipe name is plain: letters, digits, `-`, `_` and `.`,
/// never a path, a server or a dot name.
pub fn plain_pipe_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn full_name(name: &str) -> io::Result<Vec<u16>> {
    if !plain_pipe_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a pipe name must be plain letters and digits",
        ));
    }
    Ok(format!("{PIPE_PREFIX}{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

fn milliseconds(timeout: Option<Duration>) -> u32 {
    match timeout {
        // INFINITE.
        None => u32::MAX,
        Some(t) => u32::try_from(t.as_millis()).unwrap_or(u32::MAX - 1),
    }
}

/// Memory the system gave with `LocalAlloc`, freed when dropped.
struct LocalBox(*mut c_void);

impl Drop for LocalBox {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a call that documents LocalFree
            // as the way to free it, and nothing else frees it.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// The SID of the account this process runs as, in its text form
/// (`S-1-5-21-...`), from the process token.
fn own_sid() -> io::Result<String> {
    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
    // closing; `token` is a valid place for the opened handle.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle was just opened, not null, and nothing else
    // owns it.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0u32;
    // SAFETY: a null buffer of length 0 asks only for the size needed,
    // written to `needed`.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            null_mut(),
            0,
            &raw mut needed,
        )
    };
    if (needed as usize) < size_of::<TOKEN_USER>() {
        return Err(io::Error::last_os_error());
    }
    // A buffer of u64 keeps the TOKEN_USER at the start aligned.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: the buffer holds at least `needed` bytes, the size the first
    // call asked for, and lives across the call.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            needed,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call filled the buffer with a TOKEN_USER at its start,
    // and the buffer is aligned for it; the SID it points to lies inside
    // the buffer, which lives until the end of this function.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = null_mut();
    // SAFETY: `sid` is a valid SID inside `buffer`; the call allocates the
    // text with LocalAlloc and writes its pointer to `text`.
    let ok = unsafe { ConvertSidToStringSidW(sid, &raw mut text) };
    if ok == 0 || text.is_null() {
        return Err(io::Error::last_os_error());
    }
    let text = LocalBox(text.cast::<c_void>());
    let start = text.0.cast::<u16>();
    let mut len = 0usize;
    // SAFETY: the text is NUL-terminated, as ConvertSidToStringSidW
    // promises; reading stops at the NUL.
    while unsafe { *start.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units were just read from the live allocation.
    let units = unsafe { std::slice::from_raw_parts(start, len) };
    Ok(String::from_utf16_lossy(units))
}

/// The SDDL text of the pipe's security descriptor: the account as the
/// owner, and a protected DACL with one entry, full access for it.
pub fn owner_only_sddl(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;{sid})")
}

/// A security descriptor the system allocated, freed when dropped.
struct Descriptor(LocalBox);

impl Descriptor {
    /// The descriptor that lets only this process's account in.
    fn owner_only() -> io::Result<Self> {
        let sddl: Vec<u16> = owner_only_sddl(&own_sid()?)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated and lives across the call; the
        // call allocates the descriptor with LocalAlloc and writes its
        // pointer to `descriptor`; the size is not wanted.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(LocalBox(descriptor)))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
    }
}

fn manual_event() -> io::Result<OwnedHandle> {
    // SAFETY: no security attributes and no name are allowed: an unnamed,
    // manual-reset event that starts unsignaled.
    let event = unsafe { CreateEventW(null(), 1, 0, null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle was just returned, not null, and nothing else
    // owns it.
    Ok(unsafe { OwnedHandle::from_raw_handle(event) })
}

/// Waits for an overlapped operation that answered pending, up to
/// `timeout`; on a timeout the operation is cancelled and waited out, so
/// `overlapped` is never left in use.
fn finish(handle: HANDLE, overlapped: &OVERLAPPED, timeout: Option<Duration>) -> io::Result<u32> {
    // SAFETY: the event belongs to `overlapped`, which lives across the
    // wait.
    let waited = unsafe { WaitForSingleObject(overlapped.hEvent, milliseconds(timeout)) };
    let mut done = 0u32;
    if waited == WAIT_TIMEOUT {
        // SAFETY: the handle and `overlapped` are the pair of the pending
        // operation; after the cancel, GetOverlappedResult with wait set
        // returns only once the operation has ended, so `overlapped` is
        // free when this returns.
        unsafe {
            CancelIoEx(handle, overlapped);
            GetOverlappedResult(handle, overlapped, &raw mut done, 1);
        }
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the pipe did not answer in time",
        ));
    }
    if waited != WAIT_OBJECT_0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the operation has ended (its event is set); the call reads
    // its result from `overlapped`.
    let ok = unsafe { GetOverlappedResult(handle, overlapped, &raw mut done, 1) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(done)
}

/// One end of a connected pipe, server or client.
#[derive(Debug)]
pub struct PipeStream {
    handle: OwnedHandle,
    event: OwnedHandle,
    timeout: Option<Duration>,
}

impl PipeStream {
    /// Opens the client end of the owner's pipe `name`, waiting up to
    /// `timeout` for every read and write.
    pub fn connect(name: &str, timeout: Option<Duration>) -> io::Result<Self> {
        let path = full_name(name)?;
        // SAFETY: `path` is NUL-terminated and lives across the call; no
        // security attributes and no template are allowed.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle was just opened, valid, and nothing else
        // owns it.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        Ok(Self {
            handle,
            event: manual_event()?,
            timeout,
        })
    }

    /// Sets how long a read or a write may wait; `None` waits for ever.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// The process id of the client at the other end, for a server end.
    pub fn client_process_id(&self) -> io::Result<u32> {
        let mut id = 0u32;
        // SAFETY: the handle is a pipe owned by `self`; `id` is a valid
        // place for the answer.
        let ok = unsafe { GetNamedPipeClientProcessId(self.handle.as_raw_handle(), &raw mut id) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(id)
    }

    fn overlapped(&self) -> OVERLAPPED {
        OVERLAPPED {
            hEvent: self.event.as_raw_handle(),
            ..OVERLAPPED::default()
        }
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let handle = self.handle.as_raw_handle();
        let overlapped = self.overlapped();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: the buffer holds `len` bytes and, with `overlapped`, lives
        // until `finish` has seen the operation end.
        let ok = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr(),
                len,
                null_mut(),
                (&raw const overlapped).cast_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                return Ok(0);
            }
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
        }
        match finish(handle, &overlapped, self.timeout) {
            Ok(n) => Ok(n as usize),
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let handle = self.handle.as_raw_handle();
        let overlapped = self.overlapped();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: the buffer holds `len` bytes and, with `overlapped`, lives
        // until `finish` has seen the operation end.
        let ok = unsafe {
            WriteFile(
                handle,
                buf.as_ptr(),
                len,
                null_mut(),
                (&raw const overlapped).cast_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
        }
        finish(handle, &overlapped, self.timeout).map(|n| n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The server's waiting instance: its handle, its event, and its pending
/// connect, kept between waits. A pending one is only ever held by the
/// pipe, whose drop cancels it and waits it out.
struct Waiting {
    handle: OwnedHandle,
    event: OwnedHandle,
    overlapped: Box<OVERLAPPED>,
    pending: bool,
}

impl Waiting {
    /// Cancels a pending connect and waits until it has ended, so the
    /// boxed `overlapped` is no longer in use.
    fn cancel(&mut self) {
        if self.pending {
            let mut done = 0u32;
            // SAFETY: the connect is pending on this handle with this
            // `overlapped`; after the cancel, GetOverlappedResult with wait
            // set returns only once it has ended.
            unsafe {
                CancelIoEx(self.handle.as_raw_handle(), &*self.overlapped);
                GetOverlappedResult(
                    self.handle.as_raw_handle(),
                    &*self.overlapped,
                    &raw mut done,
                    1,
                );
            }
            self.pending = false;
        }
    }
}

/// The server end: a named pipe only this account can open.
pub struct OwnerPipe {
    name: String,
    path: Vec<u16>,
    descriptor: Descriptor,
    waiting: Option<Waiting>,
}

impl std::fmt::Debug for OwnerPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerPipe")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl OwnerPipe {
    /// Creates the pipe `name` (plain characters, under `\\.\pipe\`), its
    /// first instance only: it fails when any process holds the name.
    pub fn create(name: &str) -> io::Result<Self> {
        let path = full_name(name)?;
        let descriptor = Descriptor::owner_only()?;
        let mut pipe = Self {
            name: name.to_owned(),
            path,
            descriptor,
            waiting: None,
        };
        pipe.waiting = Some(pipe.instance(true)?);
        Ok(pipe)
    }

    /// The pipe's name, without the `\\.\pipe\` prefix.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn instance(&self, first: bool) -> io::Result<Waiting> {
        let attributes = self.descriptor.attributes();
        let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: the name is NUL-terminated and, with the attributes and
        // the descriptor they point to, lives across the call.
        let handle = unsafe {
            CreateNamedPipeW(
                self.path.as_ptr(),
                open,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                &raw const attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle was just created, valid, and nothing else
        // owns it.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let event = manual_event()?;
        let overlapped = Box::new(OVERLAPPED {
            hEvent: event.as_raw_handle(),
            ..OVERLAPPED::default()
        });
        Ok(Waiting {
            handle,
            event,
            overlapped,
            pending: false,
        })
    }

    /// Waits up to `wait` for a client: its stream, whose reads and writes
    /// wait up to `timeout`; `None` when none came in time (the instance
    /// keeps waiting for the next call).
    pub fn accept(
        &mut self,
        wait: Duration,
        timeout: Option<Duration>,
    ) -> io::Result<Option<PipeStream>> {
        let mut waiting = match self.waiting.take() {
            Some(waiting) => waiting,
            None => self.instance(false)?,
        };
        let handle = waiting.handle.as_raw_handle();
        if !waiting.pending {
            // SAFETY: the handle is a pipe instance owned by `waiting`; the
            // boxed `overlapped` stays at one address and lives until the
            // connect has ended (see `Waiting`'s drop).
            let ok = unsafe { ConnectNamedPipe(handle, &mut *waiting.overlapped) };
            if ok == 0 {
                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(code) if code == ERROR_PIPE_CONNECTED as i32 => {
                        return Ok(Some(self.connected(waiting, timeout)?));
                    }
                    Some(code) if code == ERROR_IO_PENDING as i32 => waiting.pending = true,
                    _ => return Err(error),
                }
            }
        }
        // SAFETY: the event belongs to the pending connect's `overlapped`.
        let waited =
            unsafe { WaitForSingleObject(waiting.event.as_raw_handle(), milliseconds(Some(wait))) };
        if waited == WAIT_TIMEOUT {
            self.waiting = Some(waiting);
            return Ok(None);
        }
        let mut done = 0u32;
        // SAFETY: the connect has ended (its event is set); the call reads
        // its result.
        let ok = unsafe { GetOverlappedResult(handle, &*waiting.overlapped, &raw mut done, 1) };
        waiting.pending = false;
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(self.connected(waiting, timeout)?))
    }

    /// Hands a connected instance over as a stream and starts the next
    /// waiting instance. Nothing is pending on it any more.
    fn connected(&mut self, waiting: Waiting, timeout: Option<Duration>) -> io::Result<PipeStream> {
        let Waiting { handle, event, .. } = waiting;
        self.waiting = Some(self.instance(false)?);
        Ok(PipeStream {
            handle,
            event,
            timeout,
        })
    }
}

// SAFETY: the raw pointers inside are the security descriptor, which the
// pipe owns and only reads, from the process heap that any thread may
// free, and the boxed OVERLAPPED of the waiting instance, which only the
// thread holding the pipe touches; handles are process-wide. Moving the
// pipe to another thread moves all of them with it.
unsafe impl Send for OwnerPipe {}

impl Drop for OwnerPipe {
    fn drop(&mut self) {
        if let Some(waiting) = &mut self.waiting {
            waiting.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn name(tag: &str) -> String {
        format!("diroxide-test-{tag}-{}", std::process::id())
    }

    #[test]
    fn names_are_plain_words_under_the_local_namespace() {
        for bad in [
            "",
            "a\\b",
            "..",
            ".x",
            "a/b",
            r"\\server\pipe\x",
            "a b",
            "a:b",
        ] {
            assert!(!plain_pipe_name(bad), "{bad}");
            assert_eq!(
                OwnerPipe::create(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert!(plain_pipe_name("diroxide-askpass-0a1b2c"));
    }

    #[test]
    fn the_descriptor_lets_only_this_account_in() {
        let sid = own_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        assert_eq!(
            owner_only_sddl("S-1-5-21-1-2-3-1001"),
            "O:S-1-5-21-1-2-3-1001D:P(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
        assert!(Descriptor::owner_only().is_ok());
    }

    #[test]
    fn a_client_and_the_server_talk_both_ways() {
        let name = name("talk");
        let mut pipe = OwnerPipe::create(&name).unwrap();
        let client_name = name.clone();
        let client = thread::spawn(move || {
            let mut stream =
                PipeStream::connect(&client_name, Some(Duration::from_secs(5))).unwrap();
            stream.write_all(b"hello").unwrap();
            let mut answer = [0u8; 5];
            stream.read_exact(&mut answer).unwrap();
            answer
        });
        let mut server = loop {
            if let Some(stream) = pipe
                .accept(Duration::from_millis(50), Some(Duration::from_secs(5)))
                .unwrap()
            {
                break stream;
            }
        };
        assert_eq!(server.client_process_id().unwrap(), std::process::id());
        let mut hello = [0u8; 5];
        server.read_exact(&mut hello).unwrap();
        assert_eq!(&hello, b"hello");
        server.write_all(b"world").unwrap();
        assert_eq!(&client.join().unwrap(), b"world");
        // A second client gets the next instance.
        let second_name = name.clone();
        let second = thread::spawn(move || PipeStream::connect(&second_name, None).is_ok());
        let mut found = false;
        for _ in 0..100 {
            if pipe
                .accept(Duration::from_millis(50), None)
                .unwrap()
                .is_some()
            {
                found = true;
                break;
            }
        }
        assert!(found && second.join().unwrap());
    }

    /// The DACL Windows holds for the live pipe, read back from its
    /// handle as SDDL: one entry, for this account, and nothing inherited.
    #[test]
    fn the_live_pipe_holds_one_entry_for_this_account() {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};
        let pipe = OwnerPipe::create(&name("dacl")).unwrap();
        let handle = pipe.waiting.as_ref().unwrap().handle.as_raw_handle();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        let wanted = DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION;
        // SAFETY: the handle is the pipe's, alive across the call; the call
        // allocates the descriptor with LocalAlloc and writes its pointer.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                wanted,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &raw mut descriptor,
            )
        };
        assert_eq!(status, 0);
        let descriptor = LocalBox(descriptor);
        let mut text: *mut u16 = null_mut();
        // SAFETY: the descriptor is the one just read and lives across the
        // call, which allocates the text with LocalAlloc.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                wanted,
                &raw mut text,
                null_mut(),
            )
        };
        assert_ne!(ok, 0);
        let text = LocalBox(text.cast::<c_void>());
        let start = text.0.cast::<u16>();
        let mut len = 0usize;
        // SAFETY: the text is NUL-terminated; reading stops at the NUL.
        while unsafe { *start.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` units were just read from the live allocation.
        let sddl = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(start, len) });
        let sid = own_sid().unwrap();
        assert_eq!(sddl, format!("O:{sid}D:P(A;;FA;;;{sid})"), "{sddl}");
    }

    #[test]
    fn the_pipe_moves_to_another_thread() {
        fn send<T: Send>() {}
        send::<OwnerPipe>();
        send::<PipeStream>();
    }

    #[test]
    fn a_name_in_use_is_never_shared() {
        let name = name("first");
        let _pipe = OwnerPipe::create(&name).unwrap();
        // The first-instance flag: a second creation of the name fails.
        assert!(OwnerPipe::create(&name).is_err());
    }

    #[test]
    fn waits_end_in_time_and_leave_no_thread_stuck() {
        let name = name("wait");
        let mut pipe = OwnerPipe::create(&name).unwrap();
        let started = std::time::Instant::now();
        assert!(
            pipe.accept(Duration::from_millis(100), None)
                .unwrap()
                .is_none()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        let client_name = name.clone();
        let client = thread::spawn(move || {
            let stream = PipeStream::connect(&client_name, None).unwrap();
            thread::sleep(Duration::from_millis(600));
            drop(stream);
        });
        let mut server = loop {
            if let Some(s) = pipe
                .accept(Duration::from_millis(50), Some(Duration::from_millis(150)))
                .unwrap()
            {
                break s;
            }
        };
        let mut byte = [0u8; 1];
        let silent = server.read(&mut byte).unwrap_err();
        assert_eq!(silent.kind(), io::ErrorKind::TimedOut);
        client.join().unwrap();
        server.set_timeout(Some(Duration::from_secs(5)));
        assert_eq!(server.read(&mut byte).unwrap(), 0, "the client closed");
        assert!(PipeStream::connect("diroxide-test-no-such-pipe", None).is_err());
    }
}
