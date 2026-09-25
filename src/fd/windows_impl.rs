//! Real-handle POSIX fd table (governance doc 3.6, layer 3 — engine landing).
//!
//! Direct port of `experiments/fork-hybrid/fdtable-poc/src/lib.rs`: a slot
//! array where each slot holds one `FdEntry` carrying a raw Windows HANDLE.
//! Core semantic (empirically verified in the POC):
//! **DuplicateHandle duplicates share the source handle's file offset**
//! (both refer to the same kernel file object), which is exactly what GNU
//! bash's fork+dup2 fd semantics need (`dup` in POSIX shares the open file
//! description; `open` does not).
//!
//! Shell-level operations modeled:
//!   - `open`  — CreateFileW / pipe end installed in a slot
//!   - `dup`   — DuplicateHandle (n>&m / <&m / >&m)
//!   - `close` — CloseHandle
//!   - `query` — slot inspection
//!   - `fork_table` — subshell: the whole table duplicated handle-by-handle
//!     (fork copies the fd table, not the file objects)
//!
//! Windows edge case carried over from the POC: a drained anonymous pipe
//! whose write end is closed reports ERROR_BROKEN_PIPE (109) from ReadFile
//! instead of a zero-byte read — `read_n` maps 109 to logical EOF.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub type HANDLE = isize; // isize so FdEntry/FdTable are Send+Sync
pub type BOOL = i32;
pub type DWORD = u32;

const INVALID_HANDLE_VALUE: HANDLE = -1;
const GENERIC_READ: DWORD = 0x8000_0000;
const GENERIC_WRITE: DWORD = 0x4000_0000;
const FILE_SHARE_READ: DWORD = 0x0000_0001;
const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
// POSIX unlink-while-open: without FILE_SHARE_DELETE Windows refuses to
// delete a file the shell still holds open (GNU redir.tests removes
// $TMPDIR/bash-c while fd 6 is `<>`-open).
const FILE_SHARE_DELETE: DWORD = 0x0000_0004;
const OPEN_EXISTING: DWORD = 3;
const CREATE_ALWAYS: DWORD = 2;
const FILE_ATTRIBUTE_TEMPORARY: DWORD = 0x0000_0100;
const FILE_BEGIN: DWORD = 0;
pub const HANDLE_FLAG_INHERIT: DWORD = 0x0000_0001;
const DUPLICATE_SAME_ACCESS: DWORD = 0x0000_0002;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> HANDLE;
    fn CreateFileW(
        name: *const u16,
        access: DWORD,
        share: DWORD,
        sa: *const c_void,
        disposition: DWORD,
        flags: DWORD,
        template: HANDLE,
    ) -> HANDLE;
    fn CreatePipe(
        hRead: *mut HANDLE,
        hWrite: *mut HANDLE,
        sa: *const SECURITY_ATTRIBUTES,
        size: DWORD,
    ) -> BOOL;
    fn DuplicateHandle(
        hSrcProc: HANDLE,
        hSrc: HANDLE,
        hDstProc: HANDLE,
        lpTarget: *mut HANDLE,
        access: DWORD,
        inherit: BOOL,
        options: DWORD,
    ) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn ReadFile(
        h: HANDLE,
        buf: *mut u8,
        n: DWORD,
        read: *mut DWORD,
        overlapped: *mut c_void,
    ) -> BOOL;
    fn WriteFile(
        h: HANDLE,
        buf: *const u8,
        n: DWORD,
        written: *mut DWORD,
        overlapped: *mut c_void,
    ) -> BOOL;
    fn SetFilePointer(h: HANDLE, dist: i32, dist_high: *mut i32, method: DWORD) -> DWORD;
    fn SetHandleInformation(h: HANDLE, mask: DWORD, flags: DWORD) -> BOOL;
    fn GetHandleInformation(h: HANDLE, flags: *mut DWORD) -> BOOL;
    fn GetFileType(h: HANDLE) -> DWORD;
    fn GetFinalPathNameByHandleW(h: HANDLE, buf: *mut u16, len: DWORD, flags: DWORD) -> DWORD;
    fn GetConsoleMode(h: HANDLE, mode: *mut DWORD) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: DWORD) -> DWORD;
    fn PeekNamedPipe(
        h: HANDLE,
        buf: *mut u8,
        buflen: DWORD,
        bytes_read: *mut DWORD,
        avail: *mut DWORD,
        left: *mut DWORD,
    ) -> BOOL;
}

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: *mut c_void,
    pub bInheritHandle: BOOL,
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdEntry {
    pub handle: HANDLE,
    /// inheritable flag mirrors bash's close-on-exec-adjacent bookkeeping;
    /// precise inheritance whitelists use it.
    pub inheritable: bool,
}

impl FdEntry {
    fn mark_inheritable(&mut self) -> Result<(), String> {
        let ok = unsafe { SetHandleInformation(self.handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        if ok == 0 {
            return Err("SetHandleInformation failed".into());
        }
        self.inheritable = true;
        Ok(())
    }

    pub fn is_inheritable(&self) -> bool {
        let mut flags: DWORD = 0;
        unsafe { GetHandleInformation(self.handle, &mut flags) != 0 && (flags & HANDLE_FLAG_INHERIT) != 0 }
    }
}

/// POSIX-style fd table: slots 0.. carry `Option<FdEntry>`.
#[derive(Debug, Default)]
pub struct FdTable {
    slots: Vec<Option<FdEntry>>,
}

impl FdTable {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn grow_to(&mut self, slot: usize) {
        if self.slots.len() <= slot {
            self.slots.resize(slot + 1, None);
        }
    }

    /// Install an already-open handle in a slot (replacing any prior entry,
    /// which is closed — POSIX `dup2` semantics).
    pub fn install(&mut self, slot: usize, handle: HANDLE) -> Result<(), String> {
        if handle == INVALID_HANDLE_VALUE || handle == 0 {
            return Err("install: invalid handle".into());
        }
        self.grow_to(slot);
        if let Some(old) = self.slots[slot].take() {
            unsafe { CloseHandle(old.handle) };
        }
        self.slots[slot] = Some(FdEntry { handle, inheritable: false });
        Ok(())
    }

    /// `N<file`: open a file for reading into `slot`.
    pub fn open_read(&mut self, slot: usize, path: &str) -> Result<(), String> {
        // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): synthetic /proc files
        // materialize through an anonymous pipe so downstream reads (and any
        // inherited-handle child) see ordinary byte-stream semantics. The
        // write side is closed immediately: a pipe reports EOF once drained,
        // which is exactly the procfs "static snapshot file" behavior.
        if let Some(content) = crate::proc_vfs::proc_file_content(path) {
            return self.install_read_bytes(slot, &content);
        }
        let wide = to_wide(path);
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_TEMPORARY,
                0,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(format!("open_read({path}): CreateFileW failed"));
        }
        self.install(slot, h)
    }

    /// Create a file for writing into `slot`.
    pub fn open_write(&mut self, slot: usize, path: &str) -> Result<(), String> {
        let wide = to_wide(path);
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_TEMPORARY,
                0,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(format!("open_write({path}): CreateFileW failed"));
        }
        self.install(slot, h)
    }

    /// Anonymous pipe ends installed in two slots.
    pub fn open_pipe(&mut self, read_slot: usize, write_slot: usize) -> Result<(), String> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut r, mut w): (HANDLE, HANDLE) = (0, 0);
        if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
            return Err("CreatePipe failed".into());
        }
        self.install(read_slot, r)?;
        self.install(write_slot, w)?;
        Ok(())
    }

    /// Q11 /proc P1: install `content` as a drained-pipe snapshot in `slot`.
    /// The write end is closed before returning, so reads hit EOF at content
    /// end -- the procfs static-snapshot behavior -- and the HANDLE remains a
    /// real byte-stream handle for inheritable-child redirection.
    pub fn install_read_bytes(&mut self, slot: usize, content: &[u8]) -> Result<(), String> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut r, mut w): (HANDLE, HANDLE) = (0, 0);
        if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
            return Err("install_read_bytes: CreatePipe failed".into());
        }
        if !content.is_empty() {
            let mut written: DWORD = 0;
            let ok = unsafe {
                WriteFile(
                    w,
                    content.as_ptr().cast(),
                    content.len() as DWORD,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || written as usize != content.len() {
                unsafe { CloseHandle(r) };
                unsafe { CloseHandle(w) };
                return Err("install_read_bytes: WriteFile failed".into());
            }
        }
        unsafe { CloseHandle(w) };
        self.install(slot, r)
    }

    /// `N>&M` / `N<&M`: duplicate the open file description into `slot`.
    /// Both slots then share the kernel file object (and thus the offset).
    pub fn dup(&mut self, from: usize, to: usize) -> Result<(), String> {
        let src = self.query(from).ok_or(format!("dup: fd {from} not open"))?;
        let mut target: HANDLE = 0;
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                src,
                GetCurrentProcess(),
                &mut target,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if ok == 0 {
            return Err(format!("dup {from}->{to}: DuplicateHandle failed"));
        }
        self.install(to, target)
    }

    /// `N<&-`: close a slot. Only this slot's handle is closed; duplicates
    /// in other slots (or other tables) keep the file object alive.
    pub fn close(&mut self, slot: usize) -> Result<(), String> {
        match self.slots.get_mut(slot).and_then(|s| s.take()) {
            Some(entry) => {
                if unsafe { CloseHandle(entry.handle) } == 0 {
                    return Err(format!("close {slot}: CloseHandle failed"));
                }
                Ok(())
            }
            None => Err(format!("close {slot}: fd not open")),
        }
    }

    pub fn query(&self, slot: usize) -> Option<HANDLE> {
        self.slots.get(slot).and_then(|s| s.as_ref().map(|e| e.handle))
    }

    pub fn entry(&self, slot: usize) -> Option<&FdEntry> {
        self.slots.get(slot).and_then(|s| s.as_ref())
    }

    pub fn is_open(&self, slot: usize) -> bool {
        self.query(slot).is_some()
    }

    /// Subshell / background job: duplicate every open slot into a fresh
    /// table. The child's handles are duplicates of the same kernel objects,
    /// so offsets stay shared across the boundary — POSIX fork semantics.
    pub fn fork_table(&self) -> Result<FdTable, String> {
        let mut child = FdTable::new();
        child.grow_to(self.slots.len().saturating_sub(1));
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(entry) = slot {
                let mut target: HANDLE = 0;
                let ok = unsafe {
                    DuplicateHandle(
                        GetCurrentProcess(),
                        entry.handle,
                        GetCurrentProcess(),
                        &mut target,
                        0,
                        0,
                        DUPLICATE_SAME_ACCESS,
                    )
                };
                if ok == 0 {
                    return Err(format!("fork_table: DuplicateHandle fd {i} failed"));
                }
                child.slots[i] = Some(FdEntry {
                    handle: target,
                    inheritable: entry.inheritable,
                });
            }
        }
        Ok(child)
    }

    /// Handles of the open slots marked inheritable — the spawn whitelist
    /// fed to PROC_THREAD_ATTRIBUTE_HANDLE_LIST.
    pub fn inheritable_handles(&self) -> Vec<HANDLE> {
        self.slots
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|e| e.inheritable)
            .map(|e| e.handle)
            .collect()
    }

    pub fn mark_inheritable(&mut self, slot: usize) -> Result<(), String> {
        self.grow_to(slot);
        match self.slots[slot].as_mut() {
            Some(e) => e.mark_inheritable(),
            None => Err(format!("mark_inheritable {slot}: fd not open")),
        }
    }

    // ---- convenience I/O ----

    /// Read up to `n` bytes from a slot (advancing the shared offset).
    pub fn read_n(&self, slot: usize, n: usize) -> Result<Vec<u8>, String> {
        let h = self.query(slot).ok_or(format!("read: fd {slot} not open"))?;
        read_some(h, n).map_err(|e| format!("read fd {slot}: {e}"))
    }

    pub fn write_all(&self, slot: usize, bytes: &[u8]) -> Result<(), String> {
        let h = self.query(slot).ok_or(format!("write: fd {slot} not open"))?;
        write_all(h, bytes).map_err(|e| format!("write fd {slot}: {e}"))
    }

    /// Explicit seek on the shared file object (SetFilePointer).
    pub fn seek(&self, slot: usize, pos: i32) -> Result<(), String> {
        let h = self.query(slot).ok_or(format!("seek: fd {slot} not open"))?;
        let r = unsafe { SetFilePointer(h, pos, std::ptr::null_mut(), FILE_BEGIN) };
        if r == 0xFFFF_FFFF {
            return Err(format!("seek fd {slot}: SetFilePointer failed"));
        }
        Ok(())
    }
}

/// Raw ReadFile on a bare HANDLE — child-side accessor for precisely
/// inherited handles passed via whitelists.
///
/// # Safety
/// `h` must be a valid readable HANDLE; `buf`/`n` a valid region.
pub unsafe fn raw_read(h: HANDLE, buf: *mut u8, n: DWORD, got: *mut DWORD) -> BOOL {
    ReadFile(h, buf, n, got, std::ptr::null_mut())
}

// ---- engine-facing free functions (FdTable-independent) ----

const FILE_APPEND_DATA: DWORD = 0x0000_0004;
const SYNCHRONIZE: DWORD = 0x0010_0000;
const CREATE_NEW: DWORD = 1;
const OPEN_ALWAYS: DWORD = 4;
const FILE_END: DWORD = 2;

fn create_file(
    path: &std::path::Path,
    access: DWORD,
    disposition: DWORD,
) -> std::io::Result<HANDLE> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            disposition,
            0,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE || h == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(h)
}

/// `N<file` — O_RDONLY equivalent.
pub fn open_file_read(path: &std::path::Path) -> std::io::Result<HANDLE> {
    // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): every redirect-read path
    // funnels through here (FileFd::open_read), so the synthetic /proc files
    // materialize as a drained pipe handle at this single choke point.
    let posix_text = path.to_string_lossy().replace('\\', "/");
    if let Some(content) = crate::proc_vfs::proc_file_content(&posix_text) {
        return install_bytes_read_pipe(&content);
    }
    create_file(path, GENERIC_READ, OPEN_EXISTING)
}

/// Create an anonymous pipe pre-filled with `content`, write end closed:
/// reads return the bytes then EOF (procfs static-snapshot semantics).
fn install_bytes_read_pipe(content: &[u8]) -> std::io::Result<HANDLE> {
    // Local extern (matches this file's declaration style; windows-sys types
    // differ from the local HANDLE/SECURITY_ATTRIBUTES aliases).
    #[link(name = "kernel32")]
    extern "system" {
        fn CreatePipe(
            read: *mut HANDLE,
            write: *mut HANDLE,
            attrs: *const SECURITY_ATTRIBUTES,
            size: DWORD,
        ) -> BOOL;
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut r, mut w): (HANDLE, HANDLE) = (0, 0);
    if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut written: DWORD = 0;
    let write_ok = content.is_empty()
        || unsafe {
            WriteFile(w, content.as_ptr().cast(), content.len() as DWORD, &mut written, std::ptr::null_mut())
        } != 0;
    if !write_ok || written as usize != content.len() {
        let err = std::io::Error::last_os_error();
        unsafe { CloseHandle(r) };
        unsafe { CloseHandle(w) };
        return Err(err);
    }
    unsafe { CloseHandle(w) };
    Ok(r)
}

/// `N>file` / `N>|file` — O_WRONLY|O_CREAT|O_TRUNC.
pub fn open_file_write_trunc(path: &std::path::Path) -> std::io::Result<HANDLE> {
    create_file(path, GENERIC_WRITE, CREATE_ALWAYS)
}

/// `set -C` non-clobber `>` — O_WRONLY|O_CREAT|O_EXCL.
pub fn open_file_create_new(path: &std::path::Path) -> std::io::Result<HANDLE> {
    create_file(path, GENERIC_WRITE, CREATE_NEW)
}

/// `N>>file` — O_WRONLY|O_CREAT|O_APPEND. A handle must carry
/// FILE_APPEND_DATA and NOT FILE_WRITE_DATA for Windows to force every
/// write to end-of-file regardless of the shared offset (MSDN:
/// FILE_APPEND_DATA "and not FILE_WRITE_DATA"); GENERIC_WRITE includes
/// FILE_WRITE_DATA, so a GENERIC_WRITE handle wrote at offset 0 and
/// `cmd >>existing` overwrote the head instead of appending.
pub fn open_file_append(path: &std::path::Path) -> std::io::Result<HANDLE> {
    create_file(path, FILE_APPEND_DATA | SYNCHRONIZE, OPEN_ALWAYS)
}

/// `N<>file` — O_RDWR|O_CREAT.
pub fn open_file_readwrite(path: &std::path::Path) -> std::io::Result<HANDLE> {
    create_file(path, GENERIC_READ | GENERIC_WRITE, OPEN_ALWAYS)
}

/// `N>&M` at the kernel level: the duplicate refers to the same file
/// object, so the file offset stays shared.
pub fn duplicate_handle(h: HANDLE) -> std::io::Result<HANDLE> {
    let mut target: HANDLE = 0;
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            h,
            GetCurrentProcess(),
            &mut target,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(target)
}

pub fn close_handle(h: HANDLE) {
    unsafe {
        CloseHandle(h);
    }
}

/// Result of a bounded readability wait on an OS handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWait {
    Ready,
    Timeout,
}

const FILE_TYPE_DISK: DWORD = 0x0001;
const FILE_TYPE_CHAR: DWORD = 0x0002;
const FILE_TYPE_PIPE: DWORD = 0x0003;
const WAIT_OBJECT_0: DWORD = 0;
const WAIT_TIMEOUT: DWORD = 0x102;

/// GNU builtins/read.def fstat gate (`tmsec = tmusec = 0` on S_ISREG):
/// a regular-file input disables `read -t` entirely.
pub fn is_disk_file(h: HANDLE) -> bool {
    (unsafe { GetFileType(h) }) == FILE_TYPE_DISK
}

/// The filesystem path an open disk-file handle refers to, or None for
/// pipes/consoles/devices. GNU's `open("/proc/self/fd/N", O_RDONLY)` on a
/// regular-file descriptor performs a FRESH open at offset 0, so callers
/// emulating /dev/fd reopen semantics need the path — a dup of the handle
/// would share the writer's file offset instead.
pub fn disk_file_path(h: HANDLE) -> Option<std::path::PathBuf> {
    if !is_disk_file(h) {
        return None;
    }
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as DWORD, 0) };
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    buf.truncate(n as usize);
    let path = String::from_utf16_lossy(&buf);
    // VOLUME_NAME_DOS yields \\?\C:\... — strip the device prefix.
    Some(std::path::PathBuf::from(
        path.strip_prefix(r"\\?\").unwrap_or(&path),
    ))
}

/// The isatty() port for `test -t` (GNU test.c → isatty(fd) → a
/// tty-specific ioctl): GetConsoleMode succeeds only on console handles.
/// NUL and other FILE_TYPE_CHAR devices fail it, matching isatty on
/// /dev/null.
pub fn is_console_handle(h: HANDLE) -> bool {
    let mut mode: DWORD = 0;
    (unsafe { GetConsoleMode(h, &mut mode) }) != 0
}

/// FILE_TYPE_CHAR check for `test -c` (GNU test.c filetest → S_ISCHR):
/// both the console and NUL qualify as character devices.
pub fn is_char_device_handle(h: HANDLE) -> bool {
    (unsafe { GetFileType(h) }) == FILE_TYPE_CHAR
}

/// Bounded wait until `h` has readable input — the Windows port of GNU
/// builtins/read.def's select()-backed read_timeout (`shtimer_select`).
/// Regular files are always readable (select reports them ready). Pipes
/// are not waitable objects, so poll PeekNamedPipe — a failed peek means
/// the pipe broke, which select would also report as readable (EOF).
/// Console input handles (CONIN$, `read < /dev/tty`) are waitable.
pub fn wait_readable(h: HANDLE, timeout: std::time::Duration) -> ReadWait {
    match unsafe { GetFileType(h) } {
        FILE_TYPE_CHAR => {
            let ms = timeout.as_millis().min(DWORD::MAX as u128 - 1) as DWORD;
            match unsafe { WaitForSingleObject(h, ms) } {
                WAIT_OBJECT_0 => ReadWait::Ready,
                WAIT_TIMEOUT => ReadWait::Timeout,
                _ => ReadWait::Ready,
            }
        }
        FILE_TYPE_PIPE => {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                let mut avail: DWORD = 0;
                let ok = unsafe {
                    PeekNamedPipe(
                        h,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut avail,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 || avail > 0 {
                    return ReadWait::Ready;
                }
                let now = std::time::Instant::now();
                if now >= deadline {
                    return ReadWait::Timeout;
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(now)
                        .min(std::time::Duration::from_millis(2)),
                );
            }
        }
        _ => ReadWait::Ready,
    }
}

/// Read up to `n` bytes; ERROR_BROKEN_PIPE (drained anonymous pipe) is EOF.
pub fn read_some(h: HANDLE, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n.max(1)];
    let mut got: DWORD = 0;
    let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), n as DWORD, &mut got, std::ptr::null_mut()) };
    if ok == 0 {
        const ERROR_BROKEN_PIPE: DWORD = 109;
        if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            return Ok(Vec::new());
        }
        return Err(std::io::Error::last_os_error());
    }
    buf.truncate(got as usize);
    Ok(buf)
}

pub fn write_all(h: HANDLE, bytes: &[u8]) -> std::io::Result<()> {
    let mut off = 0usize;
    while off < bytes.len() {
        let mut n: DWORD = 0;
        let ok = unsafe {
            WriteFile(
                h,
                bytes[off..].as_ptr(),
                (bytes.len() - off) as DWORD,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        off += n as usize;
    }
    Ok(())
}

/// Move the shared file offset to end-of-file (append-by-seek for `<>`
/// readers, or `consume` semantics).
pub fn seek_end(h: HANDLE) -> std::io::Result<()> {
    let r = unsafe { SetFilePointer(h, 0, std::ptr::null_mut(), FILE_END) };
    if r == 0xFFFF_FFFF {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Adopt a raw HANDLE into a `std::fs::File` (ownership transferred —
/// the File closes it on drop).
pub fn handle_to_file(h: HANDLE) -> std::fs::File {
    use std::os::windows::io::FromRawHandle;
    std::fs::File::from(unsafe {
        std::os::windows::io::OwnedHandle::from_raw_handle(
            h as std::os::windows::io::RawHandle,
        )
    })
}

/// `N>&M` dup that also marks the duplicate inheritable — a fresh handle we
/// own, so the shared slot handle's inherit flag is never mutated (no
/// cross-thread spawn race).
pub fn duplicate_handle_inheritable(h: HANDLE) -> std::io::Result<HANDLE> {
    let mut target: HANDLE = 0;
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            h,
            GetCurrentProcess(),
            &mut target,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(target)
}

/// Our own std handle for `fd` (0/1/2). Returns INVALID_HANDLE_VALUE/0 when
/// the process has no console std handle (GUI subsystem).
pub fn process_std_handle(fd: u32) -> HANDLE {
    const STD_INPUT_HANDLE: DWORD = 0xFFFF_FFF6;
    const STD_OUTPUT_HANDLE: DWORD = 0xFFFF_FFF5;
    const STD_ERROR_HANDLE: DWORD = 0xFFFF_FFF4;
    extern "system" {
        fn GetStdHandle(n: DWORD) -> HANDLE;
    }
    let n = match fd {
        0 => STD_INPUT_HANDLE,
        1 => STD_OUTPUT_HANDLE,
        _ => STD_ERROR_HANDLE,
    };
    unsafe { GetStdHandle(n) }
}

/// `NUL` device handle (child stdio for a null-redirected fd).
pub fn open_null_device() -> std::io::Result<HANDLE> {
    let h = unsafe {
        CreateFileW(
            to_wide("NUL").as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE || h == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(h)
}

/// Inheritable `NUL` handle — HANDLE_LIST entries must carry
/// HANDLE_FLAG_INHERIT, which sa=null does not set.
pub fn open_null_device_inheritable() -> std::io::Result<HANDLE> {
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let h = unsafe {
        CreateFileW(
            to_wide("NUL").as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &mut sa as *mut SECURITY_ATTRIBUTES as *const c_void,
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE || h == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(h)
}

// ---------------------------------------------------------------------------
// STARTUPINFOEXW spawn (governance 3.6 step 3): pass a whitelist of
// inheritable handles to the child via PROC_THREAD_ATTRIBUTE_HANDLE_LIST so
// `rubash -c` children see the parent's fd table instead of inheriting every
// inheritable handle in the process.
// ---------------------------------------------------------------------------

const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
const STARTF_USESTDHANDLES: DWORD = 0x0000_0100;
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x0000_0400;

#[repr(C)]
struct STARTUPINFOW {
    cb: DWORD,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: DWORD,
    dwY: DWORD,
    dwXSize: DWORD,
    dwYSize: DWORD,
    dwXCountChars: DWORD,
    dwYCountChars: DWORD,
    dwFillAttribute: DWORD,
    dwFlags: DWORD,
    wShowWindow: u16,
    cbReserved2: u16,
    lpReserved2: *mut u8,
    hStdInput: HANDLE,
    hStdOutput: HANDLE,
    hStdError: HANDLE,
}

#[repr(C)]
struct STARTUPINFOEXW {
    si: STARTUPINFOW,
    attr: *mut c_void,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    hProcess: HANDLE,
    hThread: HANDLE,
    dwProcessId: DWORD,
    dwThreadId: DWORD,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const c_void,
        ta: *const c_void,
        inherit: BOOL,
        flags: DWORD,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: DWORD,
        flags: DWORD,
        size: *mut usize,
    ) -> BOOL;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: DWORD,
        attr: usize,
        value: *mut c_void,
        size: usize,
        prev: *mut c_void,
        ret_size: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
}

/// Everything needed to spawn a child whose inherited handles are exactly
/// `std_handles` + `extra_handles`.
pub struct WhitelistedSpawn {
    /// Program image path (passed as lpApplicationName).
    pub program: std::path::PathBuf,
    /// Arguments (excluding argv[0]); joined with msvcrt quoting.
    pub args: Vec<String>,
    /// Complete environment block contents.
    pub env: Vec<(String, String)>,
    /// Child stdin/stdout/stderr (hStdInput/Output/Error).
    pub std_handles: [HANDLE; 3],
    /// Extra handles the child inherits at the same numeric values.
    pub extra_handles: Vec<HANDLE>,
}

fn push_quoted_arg(cmdline: &mut Vec<u16>, arg: &str) {
    // msvcrt argument quoting (same rules std::process::Command applies).
    let needs_quotes =
        arg.is_empty() || arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quotes {
        cmdline.extend(arg.encode_utf16());
        return;
    }
    cmdline.push('"' as u16);
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    cmdline.push('\\' as u16);
                }
                backslashes = 0;
                cmdline.push('"' as u16);
            }
            _ => {
                for _ in 0..backslashes {
                    cmdline.push('\\' as u16);
                }
                backslashes = 0;
                cmdline.extend(c.to_string().encode_utf16());
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        cmdline.push('\\' as u16);
    }
    cmdline.push('"' as u16);
}

/// Spawn `spec.program` with exactly the whitelisted handles inheritable.
/// All handles in `std_handles`/`extra_handles` must be marked inheritable
/// (use `duplicate_handle_inheritable` / `open_null_device`); a
/// non-inheritable entry makes CreateProcessW fail with
/// ERROR_INVALID_PARAMETER. Ownership of the handles stays with the caller —
/// close them after the child has spawned.
pub fn spawn_whitelisted(spec: &WhitelistedSpawn) -> std::io::Result<SpawnedChild> {
    let mut cmdline: Vec<u16> = Vec::new();
    push_quoted_arg(&mut cmdline, &spec.program.to_string_lossy());
    for arg in &spec.args {
        cmdline.push(' ' as u16);
        push_quoted_arg(&mut cmdline, arg);
    }
    cmdline.push(0);

    let program_w = to_wide(&spec.program.to_string_lossy());

    let mut env_pairs: Vec<(String, String)> = spec.env.clone();
    // The Unicode environment block must be sorted (case-insensitive).
    env_pairs.sort_by(|a, b| a.0.to_uppercase().cmp(&b.0.to_uppercase()));
    let mut env_block: Vec<u16> = Vec::new();
    for (k, v) in &env_pairs {
        env_block.extend(format!("{k}={v}").encode_utf16());
        env_block.push(0);
    }
    env_block.push(0);

    // Whitelist = std handles + extras, deduplicated.
    let mut handles: Vec<HANDLE> = Vec::new();
    for h in spec
        .std_handles
        .iter()
        .copied()
        .chain(spec.extra_handles.iter().copied())
    {
        if h == 0 || h == INVALID_HANDLE_VALUE || handles.contains(&h) {
            continue;
        }
        handles.push(h);
    }

    // Raw attribute list — the HANDLE_LIST value must be a bare HANDLE[]
    // buffer (passing a Vec's address would serialize ptr/len/cap).
    let mut attr_size: usize = 0;
    unsafe {
        let _ = InitializeProcThreadAttributeList(
            std::ptr::null_mut(),
            1,
            0,
            &mut attr_size,
        );
    }
    let attr_buf = unsafe {
        std::alloc::alloc(std::alloc::Layout::from_size_align(attr_size.max(1), 16).unwrap())
    };
    if attr_buf.is_null() {
        return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "attr list"));
    }
    if unsafe {
        InitializeProcThreadAttributeList(attr_buf as _, 1, 0, &mut attr_size)
    } == 0
    {
        unsafe {
            std::alloc::dealloc(
                attr_buf,
                std::alloc::Layout::from_size_align(attr_size.max(1), 16).unwrap(),
            )
        };
        return Err(std::io::Error::last_os_error());
    }
    if unsafe {
        UpdateProcThreadAttribute(
            attr_buf as _,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handles.as_mut_ptr() as _,
            std::mem::size_of::<HANDLE>() * handles.len(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        unsafe {
            DeleteProcThreadAttributeList(attr_buf as _);
            std::alloc::dealloc(
                attr_buf,
                std::alloc::Layout::from_size_align(attr_size.max(1), 16).unwrap(),
            );
        }
        return Err(std::io::Error::last_os_error());
    }

    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.si.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
    si.si.dwFlags = STARTF_USESTDHANDLES;
    si.si.hStdInput = spec.std_handles[0];
    si.si.hStdOutput = spec.std_handles[1];
    si.si.hStdError = spec.std_handles[2];
    si.attr = attr_buf as _;

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            program_w.as_ptr(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env_block.as_ptr() as _,
            std::ptr::null(),
            &si.si,
            &mut pi,
        )
    };
    unsafe {
        DeleteProcThreadAttributeList(attr_buf as _);
        std::alloc::dealloc(
            attr_buf,
            std::alloc::Layout::from_size_align(attr_size.max(1), 16).unwrap(),
        );
    }
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        CloseHandle(pi.hThread);
    }
    use std::os::windows::io::FromRawHandle;
    let owned = unsafe {
        std::os::windows::io::OwnedHandle::from_raw_handle(
            pi.hProcess as std::os::windows::io::RawHandle,
        )
    };
    Ok(SpawnedChild {
        inner: SpawnedChildInner::Whitelisted {
            process: owned,
            pid: pi.dwProcessId,
        },
    })
}

/// Process handle for a spawned child — either a std::process::Child
/// (Command path) or a raw process handle from `spawn_whitelisted`
/// (STARTUPINFOEXW path; stable Rust cannot wrap a raw handle in `Child`).
#[derive(Debug)]
pub struct SpawnedChild {
    inner: SpawnedChildInner,
}

#[derive(Debug)]
enum SpawnedChildInner {
    Std(std::process::Child),
    Whitelisted {
        process: std::os::windows::io::OwnedHandle,
        pid: u32,
    },
}

impl SpawnedChild {
    pub fn id(&self) -> u32 {
        match &self.inner {
            SpawnedChildInner::Std(child) => child.id(),
            SpawnedChildInner::Whitelisted { pid, .. } => *pid,
        }
    }

    fn whitelisted_status(
        process: &std::os::windows::io::OwnedHandle,
        timeout_ms: DWORD,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::ExitStatusExt;
        extern "system" {
            fn WaitForSingleObject(h: HANDLE, ms: DWORD) -> DWORD;
            fn GetExitCodeProcess(h: HANDLE, code: *mut DWORD) -> BOOL;
        }
        const WAIT_OBJECT_0: DWORD = 0;
        const WAIT_TIMEOUT: DWORD = 0x102;
        let raw = process.as_raw_handle() as HANDLE;
        let r = unsafe { WaitForSingleObject(raw, timeout_ms) };
        match r {
            WAIT_OBJECT_0 => {
                let mut code: DWORD = 0;
                if unsafe { GetExitCodeProcess(raw, &mut code) } == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(Some(std::process::ExitStatus::from_raw(code)))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.try_wait(),
            SpawnedChildInner::Whitelisted { process, .. } => {
                Self::whitelisted_status(process, 0)
            }
        }
    }

    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.wait(),
            SpawnedChildInner::Whitelisted { process, .. } => {
                Ok(Self::whitelisted_status(process, 0xFFFF_FFFF)?
                    .unwrap_or_else(|| {
                        use std::os::windows::process::ExitStatusExt;
                        std::process::ExitStatus::from_raw(1)
                    }))
            }
        }
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.kill(),
            SpawnedChildInner::Whitelisted { process, .. } => {
                use std::os::windows::io::AsRawHandle;
                extern "system" {
                    fn TerminateProcess(h: HANDLE, code: DWORD) -> BOOL;
                }
                let raw = process.as_raw_handle() as HANDLE;
                if unsafe { TerminateProcess(raw, 1) } == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            }
        }
    }
}

impl From<std::process::Child> for SpawnedChild {
    fn from(child: std::process::Child) -> Self {
        Self {
            inner: SpawnedChildInner::Std(child),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_file(name: &str, contents: &[u8]) -> String {
        let dir = std::env::temp_dir().join("fdtable-engine-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path.to_str().unwrap().to_string()
    }

    const PAYLOAD: &[u8] = b"abcdefghij";

    #[test]
    fn dup_shares_file_offset() {
        let path = make_file("offset.txt", PAYLOAD);
        let mut t = FdTable::new();
        t.open_read(3, &path).unwrap();
        assert_eq!(t.read_n(3, 4).unwrap(), b"abcd");
        t.dup(3, 4).unwrap();
        // slot 4 continues where slot 3 left off - the core hypothesis.
        assert_eq!(t.read_n(4, 3).unwrap(), b"efg");
        // and slot 3 continues where 4 left off.
        assert_eq!(t.read_n(3, 3).unwrap(), b"hij");
        assert_eq!(t.read_n(4, 1).unwrap(), b"" /* EOF, shared offset */);
    }

    #[test]
    fn seek_via_one_slot_moves_other() {
        let path = make_file("seek.txt", PAYLOAD);
        let mut t = FdTable::new();
        t.open_read(3, &path).unwrap();
        t.dup(3, 5).unwrap();
        t.seek(5, 8).unwrap();
        assert_eq!(t.read_n(3, 2).unwrap(), b"ij");
    }

    #[test]
    fn close_one_slot_other_slot_alive() {
        let path = make_file("close.txt", PAYLOAD);
        let mut t = FdTable::new();
        t.open_read(3, &path).unwrap();
        t.dup(3, 4).unwrap();
        t.close(3).unwrap();
        assert!(!t.is_open(3));
        assert!(t.is_open(4));
        assert_eq!(t.read_n(4, 4).unwrap(), b"abcd", "duplicate keeps the object alive");
        t.close(4).unwrap();
        assert!(t.read_n(4, 1).is_err());
    }

    #[test]
    fn fork_table_close_isolation() {
        let path = make_file("fork.txt", PAYLOAD);
        let mut parent = FdTable::new();
        parent.open_read(3, &path).unwrap();
        let mut child = parent.fork_table().unwrap();
        // child closes its fd: `( exec 3<&- )` must not touch the parent
        child.close(3).unwrap();
        assert!(parent.is_open(3));
        assert_eq!(parent.read_n(3, 4).unwrap(), b"abcd");
        // and the child's duplicate was properly closed, too
        assert!(child.read_n(3, 1).is_err());
    }

    #[test]
    fn fork_table_offsets_stay_shared_with_parent() {
        let path = make_file("forkshare.txt", PAYLOAD);
        let mut parent = FdTable::new();
        parent.open_read(3, &path).unwrap();
        let child = parent.fork_table().unwrap();
        assert_eq!(child.read_n(3, 4).unwrap(), b"abcd");
        // child's read advanced the shared object: parent continues at 'e'
        assert_eq!(parent.read_n(3, 3).unwrap(), b"efg");
    }

    #[test]
    fn pipe_buffer_shared_across_dup() {
        let mut t = FdTable::new();
        t.open_pipe(3, 9).unwrap();
        t.write_all(9, b"first\nsecond\n").unwrap();
        t.close(9).unwrap(); // EOF for readers; payload stays buffered
        t.dup(3, 4).unwrap();
        assert_eq!(t.read_n(3, 6).unwrap(), b"first\n");
        // duplicate read end consumes the same stream, not a copy
        assert_eq!(t.read_n(4, 7).unwrap(), b"second\n");
        assert_eq!(t.read_n(3, 1).unwrap(), b"");
    }

    #[test]
    fn install_closes_replaced_entry() {
        let p1 = make_file("inst1.txt", PAYLOAD);
        let p2 = make_file("inst2.txt", b"XYZ");
        let mut t = FdTable::new();
        t.open_read(3, &p1).unwrap();
        t.open_read(3, &p2).unwrap(); // replaces fd 3; old handle closed
        assert_eq!(t.read_n(3, 3).unwrap(), b"XYZ");
    }

    #[test]
    fn inheritable_flag_roundtrip() {
        let path = make_file("inh.txt", PAYLOAD);
        let mut t = FdTable::new();
        t.open_read(3, &path).unwrap();
        assert!(!t.entry(3).unwrap().is_inheritable());
        t.mark_inheritable(3).unwrap();
        assert!(t.entry(3).unwrap().is_inheritable());
        assert_eq!(t.inheritable_handles(), vec![t.query(3).unwrap()]);
    }
}

/// Take ownership of a pipe end as a raw HANDLE (coproc endpoints).
pub fn into_handle(pipe: impl std::os::windows::io::IntoRawHandle) -> HANDLE {
    pipe.into_raw_handle() as HANDLE
}
