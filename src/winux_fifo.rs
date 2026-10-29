//! WinuxCmd FIFO marker bridge (rubash#1903; vfs audit §5 D10, §4.6).
//!
//! Windows has no filesystem FIFO node type. WinuxCmd emulates one
//! (its #1038): an on-disk regular file whose first bytes are the
//! marker `!<fifo>`, carrying FILE_ATTRIBUTE_SYSTEM, and whose reader
//! / writer opens bridge to a deterministic named pipe
//! `\\.\pipe\winuxcmd-fifo-<hash>` so WinuxCmd processes rendezvous
//! with POSIX blocking-open semantics. Until now the shell half did
//! not recognize the marker at all: a redirect opened it as a plain
//! file (writes truncated the marker, reads returned the marker
//! bytes) — the two halves of the same ecosystem never met.
//!
//! This module is the shell half of that protocol, byte-compatible
//! with WinuxCmd `native_path.cppm` / `file_io.cppm`:
//!   * detection: not a directory, FILE_ATTRIBUTE_SYSTEM set, first
//!     bytes equal the marker;
//!   * pipe name: FNV-1a 64 over the canonical native path (each
//!     UTF-16 unit hashed low byte then high byte), after folding
//!     `/` to `\` and ASCII A-Z to lowercase, exactly as
//!     `winux_fifo_pipe_name_w` computes it;
//!   * read open: this side is the pipe SERVER (CreateNamedPipeW +
//!     ConnectNamedPipe), like `open_fifo_read_handle`;
//!   * write open: this side is the pipe CLIENT (CreateFileW retry
//!     loop), like `open_fifo_write_handle`.
//!
//! One deliberate mechanism difference from the WinuxCmd bridge: its
//! opens block inside raw Win32 calls, which is fine for a short-lived
//! command process. A shell redirect open must stay interruptible —
//! GNU's blocking open(2) returns EINTR when a caught signal arrives —
//! so both waits here run in slices and poll the pending-signal queue
//! (see [`fifo_wait_interrupted`]). The server connect runs on a
//! helper thread performing the blocking ConnectNamedPipeW on a plain
//! synchronous handle while the caller polls; on interrupt the caller
//! closes the handle, which aborts the pending connect. The handle
//! that lands in the fd table is therefore an ordinary synchronous
//! pipe handle the fd layer's ReadFile/WriteFile paths already serve
//! (an OVERLAPPED handle would not be: this fd layer issues
//! NULL-overlapped ReadFile, the same constraint the netopen socket
//! split in rubash#1987 works around).
//!
//! Simulation-layer discipline: everything here is `#[cfg(windows)]`;
//! on other platforms detection answers false and no Unix FIFO path
//! changes in any way.

#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::io;
use std::path::Path;

/// The on-disk marker bytes (WinuxCmd `kFifoMarker`).
pub(crate) const FIFO_MARKER: &[u8] = b"!<fifo>";

/// True when `path` names a WinuxCmd FIFO marker file. Always false
/// off Windows.
pub(crate) fn path_is_winux_fifo(path: &Path) -> bool {
    #[cfg(windows)]
    {
        imp::path_is_winux_fifo(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Validation-pure access probe for a marker target (the Windows twin
/// of the access(2) probe the Unix FIFO carve-outs use): proves
/// existence and the requested access WITHOUT taking the rendezvous —
/// opening the marker *file* is inert because the rendezvous lives in
/// the named pipe, not in the marker. Returns the io error the real
/// open would surface, or None when access is granted.
#[cfg(windows)]
pub(crate) fn marker_access_error(path: &Path, write: bool) -> Option<io::Error> {
    match std::fs::File::open(path) {
        Ok(_) => {}
        Err(error) => return Some(error),
    }
    if write {
        // A write-access verdict without create/truncate side effects
        // (mirrors W_OK): the open itself changes nothing on disk.
        match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(_) => None,
            Err(error) => Some(error),
        }
    } else {
        None
    }
}

/// Create a FIFO marker file at `path` in the WinuxCmd on-disk format
/// (used by the emulated `mkfifo`): CREATE_NEW semantics, marker
/// bytes, then FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_ARCHIVE (plus
/// FILE_ATTRIBUTE_READONLY when `readonly`). On a write failure the
/// partial file is removed, like `create_winux_fifo_w`.
#[cfg(windows)]
pub(crate) fn create_winux_fifo(path: &Path, readonly: bool) -> io::Result<()> {
    imp::create_winux_fifo(path, readonly)
}

/// The deterministic `\\.\pipe\winuxcmd-fifo-<hash>` endpoint name for
/// a marker path (WinuxCmd `winux_fifo_pipe_name_w`). The canonical
/// form both sides hash is the physical absolute native path; the
/// shell side canonicalizes with the same verbatim-stripped native
/// form the #1995 path work produces.
#[cfg(windows)]
pub(crate) fn winux_fifo_pipe_name(path: &Path) -> String {
    let canonical =
        crate::executor::path::canonicalize_native(path).unwrap_or_else(|_| path.to_path_buf());
    pipe_name_from_canonical(&canonical.to_string_lossy())
}

/// Pure core of the pipe-name derivation, split out so the byte
/// protocol is unit-testable on any platform: fold `/` to `\`,
/// lowercase ASCII A-Z, FNV-1a 64 over each UTF-16 unit's low then
/// high byte, format as 16 lowercase hex digits.
pub(crate) fn pipe_name_from_canonical(canonical: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for unit in canonical.encode_utf16() {
        let unit = if unit == u16::from(b'/') {
            u16::from(b'\\')
        } else if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
            unit + 32
        } else {
            unit
        };
        for byte in [unit & 0xFF, (unit >> 8) & 0xFF] {
            hash = (hash ^ u64::from(byte as u8)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("\\\\.\\pipe\\winuxcmd-fifo-{hash:016x}")
}

/// Whether a fifo rendezvous wait must abort: a pending signal whose
/// boundary adjudication would interrupt the current operation —
/// trapped (non-empty action), or untrapped except the two classes
/// GNU's wait machinery drops while blocked (SIGCHLD bookkeeping,
/// SIGQUIT's shell self-ignore, sig.c:333). The pending queue is
/// peeked (take + requeue, the `wait` builtin's own pattern), never
/// consumed: the command boundary still dispatches whatever was seen.
/// The aborted open reports EINTR and the redirection fails; the
/// boundary poll that follows runs the trap or the 128+sig settle,
/// which is GNU's composition for a signal-interrupted open.
#[cfg(windows)]
pub(crate) fn fifo_wait_interrupted(env_vars: &HashMap<String, String>) -> bool {
    let signals =
        crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
    if signals.is_empty() {
        return false;
    }
    let mut interrupted = false;
    for &signal in &signals {
        // The trap_exec name composition (signal_trap_name), spelled
        // through the public kill path: that module is executor-private,
        // and its whole body is this one translate + SIG prefix.
        let Some(name) =
            crate::builtins::kill::translate_signal(&signal.to_string()).map(|n| format!("SIG{n}"))
        else {
            continue;
        };
        match crate::builtins::trap::get_trap_action(env_vars, &name) {
            Some(action) if !action.is_empty() => interrupted = true,
            Some(_) => {}
            None if signal == crate::builtins::kill::SIGCHLD_NUMBER => {}
            None if signal == crate::builtins::kill::SIGQUIT_NUMBER => {}
            None => interrupted = true,
        }
    }
    crate::builtins::kill::requeue_pending_signals(signals);
    interrupted
}

/// Reduced interrupt predicate for callers that hold no Executor (the
/// `open_redirect_append` flush path): a pending SIGINT — the one
/// delivery a blocked fifo writer must not outwait. Peek + requeue,
/// never consumes.
#[cfg(windows)]
pub(crate) fn fifo_sigint_pending() -> bool {
    let signals =
        crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
    let pending = signals.contains(&crate::builtins::kill::SIGINT_NUMBER);
    crate::builtins::kill::requeue_pending_signals(signals);
    pending
}

/// Open the read (server) side of the rendezvous for the marker at
/// `path`: create the named-pipe instance and wait for a writer to
/// connect. `interrupted` is polled between wait slices; when it
/// fires the pending connect is aborted and an Interrupted error
/// returned (GNU's EINTR shape for a blocking open).
#[cfg(windows)]
pub(crate) fn open_winux_fifo_read(
    path: &Path,
    interrupted: &dyn Fn() -> bool,
) -> io::Result<std::fs::File> {
    imp::open_winux_fifo_read(path, interrupted)
}

/// Open the write (client) side of the rendezvous for the marker at
/// `path`: wait until a reader creates the endpoint, then connect.
/// Same interrupt contract as [`open_winux_fifo_read`].
#[cfg(windows)]
pub(crate) fn open_winux_fifo_write(
    path: &Path,
    interrupted: &dyn Fn() -> bool,
) -> io::Result<std::fs::File> {
    imp::open_winux_fifo_write(path, interrupted)
}

/// Slurp a marker target's whole stream for a builtin text consumer
/// (the `read`/`mapfile` self-slurp faces and the FUNCTION_STDIN
/// staging channel): Some(bytes) read through the rendezvous when
/// `path` is a WinuxCmd fifo marker, None when it is not a marker —
/// the caller's plain-file read proceeds unchanged. A failed
/// rendezvous surfaces as Some(Err) so the caller keeps its existing
/// read-failure shape. Plain-reading the marker file instead would
/// hand the consumer the 7 marker bytes and leave the writer waiting
/// for a server that never exists.
#[cfg(windows)]
pub(crate) fn slurp_marker_bytes(
    path: &Path,
    env_vars: &HashMap<String, String>,
) -> Option<io::Result<Vec<u8>>> {
    if !path_is_winux_fifo(path) {
        return None;
    }
    let mut file = match open_winux_fifo_read(path, &|| fifo_wait_interrupted(env_vars)) {
        Ok(file) => file,
        Err(error) => return Some(Err(error)),
    };
    use std::io::Read as _;
    let mut buf = Vec::new();
    Some(file.read_to_end(&mut buf).map(|_| buf))
}

#[cfg(windows)]
mod imp {
    #![allow(non_snake_case)]

    use super::{winux_fifo_pipe_name, FIFO_MARKER};
    use crate::fd::{BOOL, DWORD, HANDLE};
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const INVALID_HANDLE_VALUE: HANDLE = -1;
    const GENERIC_WRITE: DWORD = 0x4000_0000;
    const OPEN_EXISTING: DWORD = 3;
    const FILE_ATTRIBUTE_NORMAL: DWORD = 0x0000_0080;
    const FILE_ATTRIBUTE_SYSTEM: DWORD = 0x0000_0004;
    const FILE_ATTRIBUTE_ARCHIVE: DWORD = 0x0000_0020;
    const FILE_ATTRIBUTE_READONLY: DWORD = 0x0000_0001;
    const PIPE_ACCESS_INBOUND: DWORD = 0x0000_0001;
    const PIPE_TYPE_BYTE: DWORD = 0x0000_0000;
    const PIPE_READMODE_BYTE: DWORD = 0x0000_0000;
    const PIPE_WAIT: DWORD = 0x0000_0000;
    const PIPE_UNLIMITED_INSTANCES: DWORD = 255;
    const ERROR_FILE_NOT_FOUND: DWORD = 2;
    const ERROR_PIPE_BUSY: DWORD = 231;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: DWORD,
            pipe_mode: DWORD,
            max_instances: DWORD,
            out_buffer: DWORD,
            in_buffer: DWORD,
            timeout: DWORD,
            sa: *const c_void,
        ) -> HANDLE;
        fn ConnectNamedPipe(h: HANDLE, overlapped: *mut c_void) -> BOOL;
        fn CreateFileW(
            name: *const u16,
            access: DWORD,
            share: DWORD,
            sa: *const c_void,
            disposition: DWORD,
            flags: DWORD,
            template: HANDLE,
        ) -> HANDLE;
        fn WaitNamedPipeW(name: *const u16, timeout: DWORD) -> BOOL;
        fn SetFileAttributesW(name: *const u16, attrs: DWORD) -> BOOL;
        fn GetLastError() -> DWORD;
        fn CloseHandle(h: HANDLE) -> BOOL;
        fn Sleep(ms: DWORD);
    }

    fn wide_z(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn path_wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn interrupted_error() -> io::Error {
        io::Error::new(io::ErrorKind::Interrupted, "Interrupted system call")
    }

    pub(super) fn path_is_winux_fifo(path: &Path) -> bool {
        use std::os::windows::fs::MetadataExt;
        let Ok(meta) = std::fs::metadata(path) else {
            return false;
        };
        if meta.is_dir() {
            return false;
        }
        if meta.file_attributes() & FILE_ATTRIBUTE_SYSTEM == 0 {
            return false;
        }
        use std::io::Read;
        let Ok(mut file) = std::fs::File::open(path) else {
            return false;
        };
        let mut buffer = [0u8; 7];
        matches!(file.read_exact(&mut buffer), Ok(())) && &buffer == FIFO_MARKER
    }

    pub(super) fn create_winux_fifo(path: &Path, readonly: bool) -> io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        if let Err(error) = file.write_all(FIFO_MARKER) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        drop(file);
        let mut attrs = FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_ARCHIVE;
        if readonly {
            attrs |= FILE_ATTRIBUTE_READONLY;
        }
        let wide = path_wide(path);
        if unsafe { SetFileAttributesW(wide.as_ptr(), attrs) } == 0 {
            let error = io::Error::last_os_error();
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn open_winux_fifo_read(
        path: &Path,
        interrupted: &dyn Fn() -> bool,
    ) -> io::Result<std::fs::File> {
        let pipe_name = winux_fifo_pipe_name(path);
        let wide = wide_z(&pipe_name);
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // ConnectNamedPipeW blocks until a writer connects. It runs on
        // a helper thread so the caller can poll the interrupt
        // predicate in slices; closing the handle aborts a pending
        // connect (the pipe object is gone, the call fails).
        let (sender, receiver) = std::sync::mpsc::channel::<io::Result<()>>();
        std::thread::spawn(move || {
            let ok = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            let result = if ok != 0 {
                Ok(())
            } else {
                let error = io::Error::last_os_error();
                // ERROR_PIPE_CONNECTED: a writer connected between
                // create and connect — a completed rendezvous.
                if error.raw_os_error() == Some(535) {
                    Ok(())
                } else {
                    Err(error)
                }
            };
            let _ = sender.send(result);
        });
        loop {
            match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(Ok(())) => {
                    use std::os::windows::io::FromRawHandle;
                    return Ok(unsafe { std::fs::File::from_raw_handle(handle as *mut c_void) });
                }
                Ok(Err(error)) => {
                    unsafe { CloseHandle(handle) };
                    return Err(error);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if interrupted() {
                        unsafe { CloseHandle(handle) };
                        // Give the connect call a bounded grace period
                        // to unwind against the closed handle; the
                        // error is already decided either way.
                        let _ = receiver.recv_timeout(std::time::Duration::from_secs(1));
                        return Err(interrupted_error());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    unsafe { CloseHandle(handle) };
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "fifo connect thread exited",
                    ));
                }
            }
        }
    }

    pub(super) fn open_winux_fifo_write(
        path: &Path,
        interrupted: &dyn Fn() -> bool,
    ) -> io::Result<std::fs::File> {
        let pipe_name = winux_fifo_pipe_name(path);
        let wide = wide_z(&pipe_name);
        loop {
            if interrupted() {
                return Err(interrupted_error());
            }
            let handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    0,
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                use std::os::windows::io::FromRawHandle;
                return Ok(unsafe { std::fs::File::from_raw_handle(handle as *mut c_void) });
            }
            let code = unsafe { GetLastError() };
            match code {
                // All instances busy: wait a slice for one to free up.
                ERROR_PIPE_BUSY => {
                    unsafe { WaitNamedPipeW(wide.as_ptr(), 25) };
                }
                // No reader has created the endpoint yet: keep waiting
                // (GNU's O_WRONLY fifo open blocks until a reader).
                ERROR_FILE_NOT_FOUND => {
                    unsafe { Sleep(10) };
                }
                _ => {
                    return Err(io::Error::from_raw_os_error(
                        i32::try_from(code).unwrap_or(0),
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Byte-protocol pins against WinuxCmd `winux_fifo_pipe_name_w`:
    /// the expected values were computed from the reference algorithm
    /// (FNV-1a 64 over the normalized canonical string's UTF-16 units,
    /// low byte then high byte) independently of this implementation.
    #[test]
    fn issue1903_pipe_name_matches_winuxcmd_derivation() {
        assert_eq!(
            super::pipe_name_from_canonical("C:\\Users\\Test\\fifo1"),
            "\\\\.\\pipe\\winuxcmd-fifo-89b5847cd542bb9b"
        );
        assert_eq!(
            super::pipe_name_from_canonical("D:\\fifo"),
            "\\\\.\\pipe\\winuxcmd-fifo-1f87fd11e8c5ea69"
        );
        // Forward slashes and uppercase fold before hashing.
        assert_eq!(
            super::pipe_name_from_canonical("C:/Users/Test/fifo1"),
            super::pipe_name_from_canonical("C:\\Users\\Test\\fifo1")
        );
    }

    #[test]
    fn issue1903_marker_detection_is_inert_off_windows() {
        #[cfg(not(windows))]
        assert!(!super::path_is_winux_fifo(std::path::Path::new(
            "/nonexistent"
        )));
    }
}
