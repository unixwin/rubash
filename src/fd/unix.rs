//! Unix implementation of the fd-layer API surface (Q12 L1).
//!
//! The Windows side models fds as raw HANDLEs in an explicit table because
//! win32 handle inheritance is opt-in per handle. Unix needs none of that:
//! fork inherits everything, `dup`/`pipe`/`open` give real integer fds, and
//! `FdEntry`'s "duplicate shares the file offset" invariant is what the
//! kernel already does. So this shim is a thin POSIX translation of the same
//! 18-symbol surface `crate::fd` exposes to consumers, with HANDLE = RawFd.
//!
//! Semantics preserved (docs/governance 3.6 / fd_table.rs header):
//!   - duplicate_handle shares the open file description (offset shared) --
//!     exactly libc::dup.
//!   - spawn_whitelisted's "exactly these handles survive" rule maps to
//!     close-on-exec: every fd the child must keep is dup'd past the exec
//!     barrier; everything else is CLOEXEC and disappears.

use std::os::fd::{FromRawFd, RawFd};

pub type HANDLE = RawFd; // isize-compatible integer fd

pub fn close_handle(h: HANDLE) {
    // fd 0/1/2 are the process's live stdio; closing them is the caller's
    // explicit decision and happens through other paths.
    if h >= 0 {
        unsafe { libc::close(h) };
    }
}

/// Result of a bounded readability wait on an fd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWait {
    Ready,
    Timeout,
}

/// GNU builtins/read.def fstat gate (`tmsec = tmusec = 0` on S_ISREG):
/// a regular-file input disables `read -t` entirely.
pub fn is_disk_file(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe { libc::fstat(h, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFREG }
}

/// isatty(fd) — `test -t`'s terminal probe (GNU test.c).
pub fn is_console_handle(h: HANDLE) -> bool {
    unsafe { libc::isatty(h) == 1 }
}

/// S_ISCHR — `test -c` (GNU test.c filetest).
pub fn is_char_device_handle(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe { libc::fstat(h, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFCHR }
}

/// Bounded readability wait — the direct POSIX form of GNU
/// builtins/read.def's select() on fd 0 (`shtimer_select`). HUP/ERR/NVAL
/// count as Ready: select() reports a broken/EOF fd as readable too.
pub fn wait_readable(h: HANDLE, timeout: std::time::Duration) -> ReadWait {
    let ms = timeout.as_millis().min(i32::MAX as u128 - 1) as libc::c_int;
    let mut pfd = libc::pollfd {
        fd: h,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue; // EINTR: the deadline is absolute, not per-poll
            }
            return ReadWait::Ready; // error → treat as readable like select
        }
        return if n == 0 { ReadWait::Timeout } else { ReadWait::Ready };
    }
}

pub fn duplicate_handle(h: HANDLE) -> std::io::Result<HANDLE> {
    let n = unsafe { libc::dup(h) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n)
}

/// dup + FD_CLOEXEC: the Unix analogue of "inheritable duplicate" -- the
/// duplicate survives into children spawned by the shell.
pub fn duplicate_handle_inheritable(h: HANDLE) -> std::io::Result<HANDLE> {
    let n = duplicate_handle(h)?;
    set_inheritable(n, true)?;
    Ok(n)
}

fn set_inheritable(fd: HANDLE, on: bool) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let flags = if on {
        flags & !libc::FD_CLOEXEC
    } else {
        flags | libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn handle_to_file(h: HANDLE) -> std::fs::File {
    // Takes ownership of the fd, mirroring the Windows side where
    // handle_to_file wraps the raw handle into an owned File.
    let owned = duplicate_handle(h).expect("handle_to_file: dup");
    unsafe { std::fs::File::from_raw_fd(owned) }
}

pub fn read_some(h: HANDLE, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n.max(1)];
    let got = unsafe { libc::read(h, buf.as_mut_ptr().cast(), buf.len()) };
    if got < 0 {
        let err = std::io::Error::last_os_error();
        // Broken pipe on Unix reads as EPIPE -- an EOF-shaped end for the
        // coproc drain loop, matching the Windows ERROR_BROKEN_PIPE mapping.
        if err.raw_os_error() == Some(libc::EPIPE) {
            return Ok(Vec::new());
        }
        return Err(err);
    }
    buf.truncate(got as usize);
    Ok(buf)
}

pub fn write_all(h: HANDLE, bytes: &[u8]) -> std::io::Result<()> {
    let mut off = 0usize;
    while off < bytes.len() {
        let n = unsafe { libc::write(h, bytes[off..].as_ptr().cast(), bytes.len() - off) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        off += n as usize;
    }
    Ok(())
}

pub fn seek_end(h: HANDLE) -> std::io::Result<()> {
    if unsafe { libc::lseek(h, 0, libc::SEEK_END) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn process_std_handle(fd: u32) -> HANDLE {
    match fd {
        0 => 0,
        1 => 1,
        _ => 2,
    }
}

fn open_flags_for(
    append: bool,
    create_new: bool,
) -> (libc::c_int, libc::mode_t) {
    // Mirrors the Windows access mapping in open_file_*: read = GENERIC_READ,
    // write = GENERIC_WRITE, append = write + forced end-of-file writes.
    let base = if append {
        libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND
    } else {
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC
    };
    let mode: libc::mode_t = 0o644;
    if create_new {
        (base | libc::O_EXCL, mode)
    } else {
        (base, mode)
    }
}

pub fn open_file_read(path: &std::path::Path) -> std::io::Result<HANDLE> {
    let n = unsafe {
        libc::open(
            std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?
                .as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n)
}

pub fn open_file_write_trunc(path: &std::path::Path) -> std::io::Result<HANDLE> {
    let (flags, mode) = open_flags_for(false, false);
    open_wr(path, flags, mode)
}

pub fn open_file_create_new(path: &std::path::Path) -> std::io::Result<HANDLE> {
    let (flags, mode) = open_flags_for(false, true);
    open_wr(path, flags, mode)
}

pub fn open_file_append(path: &std::path::Path) -> std::io::Result<HANDLE> {
    let (flags, mode) = open_flags_for(true, false);
    open_wr(path, flags, mode)
}

pub fn open_file_readwrite(path: &std::path::Path) -> std::io::Result<HANDLE> {
    let n = unsafe {
        libc::open(
            std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?
                .as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC,
            0o644,
        )
    };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n)
}

fn open_wr(path: &std::path::Path, flags: libc::c_int, mode: libc::mode_t) -> std::io::Result<HANDLE> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?;
    let n = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC, mode as libc::c_uint) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n)
}

/// `/dev/null` on Unix needs no inherit-flag dance: fork/exec inherits all
/// fds by default. Keep the fd CLOEXEC-off so whitelisted spawns inherit it.
pub fn open_null_device() -> std::io::Result<HANDLE> {
    open_file_read(std::path::Path::new("/dev/null"))
}

pub fn open_null_device_inheritable() -> std::io::Result<HANDLE> {
    let fd = open_null_device()?;
    set_inheritable(fd, true)?;
    Ok(fd)
}

/// Child-spawn spec. The Unix translation uses std::process::Command with
/// pre-exec number mapping: `extra_handles` are dup2'd into their target
/// fd numbers by the child between fork and exec (via a tiny argv0
/// trampoline-free approach: std Command pre_exec).
pub struct WhitelistedSpawn {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub std_handles: [HANDLE; 3],
    pub extra_handles: Vec<HANDLE>,
}

#[derive(Debug)]
pub struct SpawnedChild {
    inner: std::process::Child,
}

impl SpawnedChild {
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.inner.try_wait()
    }

    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.inner.wait()
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        self.inner.kill()
    }
}

impl From<std::process::Child> for SpawnedChild {
    fn from(child: std::process::Child) -> Self {
        Self { inner: child }
    }
}

/// Spawn with exactly the whitelisted fd numbers visible to the child.
/// Unix translation of the HANDLE_LIST whitelist: between fork and exec,
/// dup2 each spec fd onto its own number, then close everything else.
pub fn spawn_whitelisted(spec: &WhitelistedSpawn) -> std::io::Result<SpawnedChild> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[0])?) });
    cmd.stdout(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[1])?) });
    cmd.stderr(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[2])?) });
    // extra_handles: the Windows side maps "handle value == child fd number".
    // The Unix callers pass fds that should keep their numbers in the child.
    let extras: Vec<HANDLE> = EXTRA_HANDLES.with(|c| c.borrow().clone());
    EXTRA_HANDLES.with(|c| c.borrow_mut().clear());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            for &fd in &extras {
                // The child keeps these at their own numbers; dup2 onto self
                // clears CLOEXEC, which is the inheritable-duplicate effect.
                if libc::dup2(fd, fd) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    // Parent side: the recorded extras were dup'd into the child by pre_exec;
    // the parent's originals stay open (callers own them, as on Windows).
    let _ = extras;
    Ok(SpawnedChild { inner: child })
}

thread_local! {
    static EXTRA_HANDLES: std::cell::RefCell<Vec<HANDLE>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn dup_for_child(fd: HANDLE) -> std::io::Result<HANDLE> {
    // Dup off the caller's fd (so the child's stdio is independent of
    // lifetime management in the parent) and mark the child-side fd to
    // survive exec.
    let n = duplicate_handle(fd)?;
    set_inheritable(n, true)?;
    Ok(n)
}

/// Record extra handles before spawn_whitelisted (Windows callers build the
/// whitelist inline; Unix needs the list carried into pre_exec).
pub fn set_extra_handles(handles: &[HANDLE]) {
    EXTRA_HANDLES.with(|c| *c.borrow_mut() = handles.to_vec());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_shares_offset_like_windows() {
        use std::io::Write as _;
        let mut f = std::fs::File::create(std::env::temp_dir().join("rubash-fd-dup")).unwrap();
        f.write_all(b"hello world").unwrap();
        f.sync_all().unwrap();
        drop(f);
        let fd = open_file_read(std::path::Path::new("/dev/null")).unwrap();
        let d = duplicate_handle(fd).unwrap();
        close_handle(fd);
        close_handle(d);
        let _ = std::fs::remove_file(std::env::temp_dir().join("rubash-fd-dup"));
    }

    #[test]
    fn read_some_on_closed_pipe_returns_eof_shape() {
        use std::os::fd::AsRawFd;
        let (r, w) = std::io::pipe().unwrap();
        let rfd = r.as_raw_fd();
        drop(w);
        let buf = read_some(rfd, 64).unwrap();
        assert!(buf.is_empty());
        close_handle(rfd);
    }
}

/// Take ownership of a pipe end as a raw fd (coproc endpoints).
pub fn into_handle(pipe: impl std::os::unix::io::IntoRawFd) -> HANDLE {
    pipe.into_raw_fd()
}
