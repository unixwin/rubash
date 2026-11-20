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
use std::os::unix::process::ExitStatusExt;

pub type HANDLE = RawFd; // isize-compatible integer fd

/// niubash#519: DOS device names (CON, CONIN$, NUL, COMx, …) are a
/// Windows spelling class — no path is one off Windows. Stub mirror of
/// `windows_impl::is_dos_device_path` so `crate::fd` consumers resolve
/// on every target (the executor's own wrapper documents the same
/// "always false off Windows" contract).
pub fn is_dos_device_path(_path: &std::path::Path) -> bool {
    false
}

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

/// rubash#2476: whether a standard descriptor was CLOSED at exec —
/// the state GNU starts from, before any runtime initialization.
///
/// A Rust process cannot observe this from `main`: std::rt::init
/// sanitizes the standard descriptors, opening /dev/null over any
/// that were closed at exec, so by the time user code runs a
/// born-closed fd 1 is indistinguishable from a genuine
/// `>/dev/null` redirect — the descriptor is open and writes to it
/// succeed. GNU (C) keeps the descriptor closed: a builtin's
/// buffered output fails at sh_chkwrite's fflush (builtins/common.c:
/// 319) with EBADF, producing `echo: write error: Bad file
/// descriptor` and status 1.
///
/// The capture therefore runs in a loader constructor (ELF
/// `.init_array`; Mach-O `__mod_init_func`), which the dynamic
/// loader invokes before the Rust runtime's sanitization. It uses
/// only fcntl and atomic stores — no std, no allocation. Values:
/// 0 = never captured (unsupported platform / constructor not
/// linked), 1 = open at exec, 2 = closed at exec. Only the fd
/// table's fd-1 seed consults it (see FdTable::new): a descriptor
/// genuinely redirected from /dev/null was OPEN at exec (value 1)
/// and keeps the normal Stdout endpoint.
static STD_FD_AT_EXEC: [std::sync::atomic::AtomicU8; 3] = [
    std::sync::atomic::AtomicU8::new(0),
    std::sync::atomic::AtomicU8::new(0),
    std::sync::atomic::AtomicU8::new(0),
];

extern "C" fn capture_std_fds_at_exec() {
    for fd in 0..3i32 {
        let open = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) } >= 0;
        STD_FD_AT_EXEC[fd as usize].store(
            if open { 1 } else { 2 },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
#[used]
#[link_section = ".init_array"]
static STD_FD_AT_EXEC_CTOR: extern "C" fn() = capture_std_fds_at_exec;

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[used]
#[link_section = "__DATA,__mod_init_func"]
static STD_FD_AT_EXEC_CTOR: extern "C" fn() = capture_std_fds_at_exec;

/// True when descriptor `fd` (0..=2) was closed at exec, per the
/// loader-constructor capture above. False when the descriptor was
/// open, and false when no capture ran (the pre-#2476 behavior is
/// the safe fallback on platforms without a constructor section).
pub fn std_fd_closed_at_exec(fd: u32) -> bool {
    STD_FD_AT_EXEC
        .get(fd as usize)
        .is_some_and(|state| state.load(std::sync::atomic::Ordering::Relaxed) == 2)
}

/// GNU builtins/read.def fstat gate (`tmsec = tmusec = 0` on S_ISREG):
/// a regular-file input disables `read -t` entirely.
pub fn is_disk_file(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe {
        libc::fstat(h, &mut st) == 0
            && (u32::from(st.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFREG)
    }
}

/// rubash#3432: the (device, inode, mtime seconds, mtime
/// nanoseconds) of the open file description behind `h`, from
/// fstat(2) — the identity/mtime the `__RUBASH_FD_STAT_<fd>` mark
/// carries. GNU's sh_stat (lib/sh/eaccess.c:80) stats the descriptor
/// the process holds; in GNU the shell's OS fd N IS user fd N, so a
/// plain stat() of /proc/self/fd/N lands on the bound file, while
/// here the OS fd namespace is shifted (the cwd anchor holds OS fd
/// 3) and only the fstat of the fd-table entry's handle names the
/// same file. None when the descriptor is closed or fstat fails.
/// The nanosecond arm follows the cp_stat_mtime_nsec idiom in
/// src/executor/external_file_builtins.rs: the libc crate flattens
/// the timespec's nanoseconds into their own field on every unix
/// target the suite builds for, and the remaining targets answer 0.
pub fn fd_stat_parts(h: HANDLE) -> Option<(u64, u64, i64, i64)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(h, &mut st) } != 0 {
        return None;
    }
    Some((
        st.st_dev as u64,
        st.st_ino as u64,
        i64::from(st.st_mtime),
        stat_mtime_nsec(&st),
    ))
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "gnu"),
    all(target_os = "linux", not(target_env = "gnu")),
    target_os = "macos",
    target_os = "ios",
))]
fn stat_mtime_nsec(st: &libc::stat) -> i64 {
    // glibc, musl and bionic flatten the nanoseconds into their own
    // field; its width follows the target's c_long (i32 on armv7).
    // Darwin's C header keeps the st_mtimespec pair, but the libc
    // crate flattens it the same way.
    i64::from(st.st_mtime_nsec)
}

#[cfg(not(any(
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
)))]
fn stat_mtime_nsec(_st: &libc::stat) -> i64 {
    0
}

/// S_ISFIFO — the handle refers to a pipe/FIFO. GNU test.c's `-p`
/// file test on an fd path resolves through the descriptor itself, so
/// fd slots backed by a real OS pipe (e.g. `exec N< <(...)`, which
/// stores the pipe reader as a File endpoint) must classify as pipes
/// (rubash#1467).
pub fn is_pipe_handle(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe {
        libc::fstat(h, &mut st) == 0
            && (u32::from(st.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFIFO)
    }
}

/// rubash#2385: open the complementary (write) end of the pipe a
/// read handle refers to — the operation GNU's open() of
/// `/proc/self/fd/N` performs on a pipe descriptor (the magic-link
/// open attaches to the pipe's other end). The returned File owns a
/// NEW open file description of the pipe's write side; closing it
/// never touches `h`. None when `h` is not a FIFO or the link open
/// fails. The O_WRONLY open cannot block: the caller holds the
/// pipe's read end open through `h` itself, so a reader exists.
pub fn complementary_write_file(h: HANDLE) -> Option<std::fs::File> {
    if !is_pipe_handle(h) {
        return None;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(format!("/proc/self/fd/{h}"))
        .ok()
}

/// S_ISSOCK — the handle refers to a socket. The Windows twin
/// (windows_impl::is_socket_handle) exists for the spawn layer; the
/// fd-fact classification (rubash#2132) needs the same judgment on
/// unix, where an inherited socketpair end is otherwise
/// indistinguishable from a pipe to a mode-only probe.
pub fn is_socket_handle(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe {
        libc::fstat(h, &mut st) == 0
            && (u32::from(st.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFSOCK)
    }
}

/// The filesystem path an open regular-file descriptor refers to, or
/// None for pipes/ttys/devices. GNU's `open("/proc/self/fd/N", O_RDONLY)`
/// on a regular file performs a FRESH open at offset 0 — callers
/// emulating /dev/fd reopen semantics need the path (dup would share the
/// writer's offset). `/proc` itself is the authority; non-proc hosts fall
/// back to fstat+FCNTL-less unknown → None.
pub fn disk_file_path(h: HANDLE) -> Option<std::path::PathBuf> {
    if !is_disk_file(h) {
        return None;
    }
    std::fs::read_link(format!("/proc/self/fd/{h}")).ok()
}

/// isatty(fd) — `test -t`'s terminal probe (GNU test.c).
pub fn is_console_handle(h: HANDLE) -> bool {
    unsafe { libc::isatty(h) == 1 }
}

/// True when the tty is in canonical (line) input mode — the mode where
/// the kernel line discipline gives the VEOF char (Ctrl-D) its EOF
/// meaning. With ICANON off (the one-char/raw modes read.def installs
/// for `-n`/`-d`/deadline reads) 0x04 is ordinary data. On a canonical
/// tty the kernel never delivers the VEOF byte itself, so this only
/// tells the read transports whether a 0x04 byte they DO receive (a
/// Windows console has no kernel VEOF) carries EOF semantics.
pub fn console_line_input_mode(h: HANDLE) -> bool {
    if !is_console_handle(h) {
        return false;
    }
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    unsafe { libc::tcgetattr(h, &mut termios) == 0 && (termios.c_lflag & libc::ICANON) != 0 }
}

/// S_ISCHR — `test -c` (GNU test.c filetest).
pub fn is_char_device_handle(h: HANDLE) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe {
        libc::fstat(h, &mut st) == 0
            && (u32::from(st.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFCHR)
    }
}

/// Guard that switches a tty input fd to per-character reads for the
/// duration of a `read` builtin call — the termios form of GNU
/// builtins/read.def ttfd_onechar (ICANON off): a single keystroke
/// completes the read without a newline. `None` when `h` is not a tty.
pub struct ConsoleOnecharGuard {
    fd: HANDLE,
    saved: libc::termios,
}

impl ConsoleOnecharGuard {
    /// The tty driver is not echoing in this mode; consumed bytes go back to
    /// the same terminal so `read -n1` looks like its line-mode form.
    pub fn echo_byte(&self, byte: u8) {
        if !is_console_handle(self.fd) {
            return;
        }
        let _ = write_all(self.fd, &[byte]);
    }
}

impl Drop for ConsoleOnecharGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSADRAIN, &self.saved);
        }
    }
}

/// Silenced echo with canonical buffering kept — the read.def:603-621
/// `ttfd_noecho` shape (`read -s` without -n/-d).
pub fn enter_console_noecho_mode(h: HANDLE) -> Option<ConsoleOnecharGuard> {
    if !is_console_handle(h) {
        return None;
    }
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    unsafe {
        if libc::tcgetattr(h, &mut saved) != 0 {
            return None;
        }
    }
    let mut silenced = saved;
    silenced.c_lflag &= !libc::ECHO;
    unsafe {
        if libc::tcsetattr(h, libc::TCSADRAIN, &silenced) != 0 {
            return None;
        }
    }
    Some(ConsoleOnecharGuard { fd: h, saved })
}

/// Enter per-character tty read mode. Returns `None` (and changes nothing)
/// when `h` is not a tty.
pub fn enter_console_onechar_mode(h: HANDLE) -> Option<ConsoleOnecharGuard> {
    if !is_console_handle(h) {
        return None;
    }
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    unsafe {
        if libc::tcgetattr(h, &mut saved) != 0 {
            return None;
        }
    }
    let mut raw = saved;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    unsafe {
        if libc::tcsetattr(h, libc::TCSADRAIN, &raw) != 0 {
            return None;
        }
    }
    Some(ConsoleOnecharGuard { fd: h, saved })
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
        return if n == 0 {
            ReadWait::Timeout
        } else {
            ReadWait::Ready
        };
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
///
/// rubash#3917 round 6 (f4cond S1): the duplicate is requested at or
/// above SHELL_FD_BASE (10, redir.c:86) — NEVER the lowest free number.
/// A plain dup lands on the lowest free descriptor, which can be a user
/// fd the shell considers CLOSED (e.g. 3 after `exec 3<&-`); made
/// inheritable and inherited by a plain-spawn child (the `>(...)` body,
/// which takes no pre_exec close-all), it resurrects that number in the
/// child's fd table, so the body's `>&3` succeeds where GNU reports
/// `Bad file descriptor`. The number itself is only a transport key —
/// every consumer re-dups it onto its target number at restore
/// (init.rs) or takes it as a stdio slot — so the floor costs nothing
/// and matches dup_adopted_fd's floor below.
pub fn duplicate_handle_inheritable(h: HANDLE) -> std::io::Result<HANDLE> {
    let n = unsafe { libc::fcntl(h, libc::F_DUPFD_CLOEXEC, 10) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    set_inheritable(n, true)?;
    Ok(n)
}

/// rubash#1838: duplicate an inherited descriptor for adoption into an
/// executor's fd table. The table's FileFd drop closes the handle it
/// holds, so the adopted handle must be the executor's own duplicate —
/// never the host process's original descriptor: an Executor embedded
/// in a host process (the test harness, niubash) does not own the
/// host's descriptors, and closing one out from under a host-side
/// owner aborts the process on std's IO-safety check when that owner
/// drops. The duplicate names the same open file description (shared
/// offset and status flags), so shell-visible semantics are unchanged.
/// F_DUPFD_CLOEXEC matches this module's other executor-owned
/// descriptors — the spawn path arranges child layouts explicitly, so
/// table handles are never inherited raw.
///
/// The duplicate is requested at or above SHELL_FD_BASE (10,
/// redir.c:86) — the floor GNU's own descriptor saves use
/// (redir.c:940). The adopted duplicate is an executor-internal
/// handle; the low numbers are the user's redirection space. Parking
/// the duplicate on a free low number makes it a phantom occupant of
/// that number: /proc/$$/fd names it there, and the persistent-exec
/// placement (FdTable::place_persistent_file) mistakes the adoption
/// entry holding it for a live binding on the number and refuses the
/// unconditional dup2 GNU performs (redir.c:1017/:1120) — so
/// `exec N>file` stayed virtual whenever an inherited descriptor's
/// adoption had landed on N first (rubash#3917).
pub fn dup_adopted_fd(fd: u32) -> Option<HANDLE> {
    let n = unsafe { libc::fcntl(fd as HANDLE, libc::F_DUPFD_CLOEXEC, 10) };
    if n < 0 {
        None
    } else {
        Some(n)
    }
}

/// GNU general.h:257 HIGH_FD_MAX and general.c:702-733
/// `move_to_high_fd(fd, check_new=1, maxfd=-1)`: the ceiling a
/// persistent internal descriptor is moved below — the descriptor
/// table size capped at 256 (20 when the query fails). bash spends
/// this on exactly the class the cwd anchor belongs to: fds that must
/// outlive the shell's own bookkeeping but must not collide with the
/// low numbers user redirections walk (script fd shell.c:1700,
/// coproc parent pipe subst.c:6392, saved stdout subst.c:7037).
pub(crate) const HIGH_FD_MAX: i32 = 256;

/// The scan ceiling of `move_to_high_fd` with maxfd=-1: the live
/// descriptor-table size (`sysconf(_SC_OPEN_MAX)`, the
/// getdtablesize(2) equivalent bash's compat layer resolves to),
/// with 20 substituted when the query fails and capped at
/// HIGH_FD_MAX — general.c:705-713.
fn high_fd_ceiling() -> i32 {
    let n = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    if n <= 0 {
        20
    } else {
        n.min(HIGH_FD_MAX as libc::c_long) as i32
    }
}

/// rubash#3640: GNU general.c:702 `move_to_high_fd` (check_new=1,
/// maxfd=-1). The session cwd anchor (`ShellState::cwd_anchor`) holds
/// a directory descriptor open for the session's whole residence in
/// its cwd so a later deletion still leaves spawns and in-process
/// opens a live inode to land on (rubash#3198/#3268). Opened plainly,
/// that descriptor took the lowest free OS fd — 3 in every ordinary
/// session — shifting the whole user fd space up one kernel
/// descriptor: `exec 3>file` put the file on OS fd 4 while fd 3 still
/// read as the cwd in /proc/$$/fd (the #3432 namespace shift this
/// removes at the root). GNU keeps its own persistent internals out
/// of the low range with move_to_high_fd; the anchor now takes the
/// same treatment.
///
/// Scans DOWN from ceiling-1 to 4 for the first free descriptor
/// (general.c:715-719 `for (nfds--; ...; nfds--)`), dup2's `fd` onto
/// it, closes the original, and returns the new number. dup2 clears
/// FD_CLOEXEC on the target, so the close-on-exec bit the caller's
/// open set is re-armed — internal fds must not leak raw into
/// exec'd children (the anchor is only ever spent pre-exec:
/// stamp_spawn_cwd/spawn_cwd_stamp fchdir in pre_exec,
/// fd::spawn_whitelisted's cwd_anchor_fd likewise). When no slot in
/// (3, ceiling) is free, or the dup fails, `fd` is returned
/// unchanged — GNU's own fallback (general.c:730-733).
pub fn move_to_high_fd(fd: HANDLE) -> HANDLE {
    if fd < 0 {
        return fd;
    }
    let ceiling = high_fd_ceiling();
    for target in (4..ceiling).rev() {
        // F_GETFD < 0 means the slot is free (EBADF) — the same probe
        // GNU's check_new loop runs. `fd` itself is open, so it never
        // passes this test and can never be dup2'd onto itself.
        if unsafe { libc::fcntl(target, libc::F_GETFD, 0) } >= 0 {
            continue;
        }
        if unsafe { libc::dup2(fd, target) } < 0 {
            return fd;
        }
        let flags = unsafe { libc::fcntl(target, libc::F_GETFD) };
        unsafe {
            libc::fcntl(target, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            libc::close(fd);
        }
        return target;
    }
    fd
}

/// `move_to_high_fd` for an owned `File`: the returned File owns the
/// relocated descriptor (or the original one when no high slot was
/// free). Used by the cwd-anchor opens so `CwdAnchor::dir` keeps a
/// plain `Arc<File>` while its kernel fd sits above the user range.
pub fn move_file_to_high_fd(file: std::fs::File) -> std::fs::File {
    use std::os::fd::IntoRawFd;
    let fd = file.into_raw_fd();
    // SAFETY: move_to_high_fd either returns `fd` itself (still owned
    // by nobody — into_raw_fd released it) or a fresh dup2 target it
    // now owns; from_raw_fd re-adopts exactly one live descriptor.
    unsafe { std::fs::File::from_raw_fd(move_to_high_fd(fd)) }
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

/// rubash#3300: decode a 32-hex-char respawn proof nonce into its 16
/// raw bytes. Shared by the spawner (`mint_respawn_proof`, below) and
/// the child's entry check (`respawn_proof_validated`, init.rs) so the
/// two sides can never disagree on the encoding.
pub(crate) fn respawn_proof_nonce_bytes(hex: &str) -> Option<[u8; 16]> {
    // Byte arithmetic only — no byte->char widening (the utf8
    // boundary hygiene script forbids widening a byte that could be
    // a UTF-8 continuation byte; a nonce is hex ASCII, but the
    // decode must not assume that before validating).
    fn hex_val(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = hex.as_bytes();
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        let hi = hex_val(bytes[2 * i])?;
        let lo = hex_val(bytes[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

/// rubash#3300: mint a per-spawn provenance proof for a fork-shaped
/// respawn child — a fresh anonymous pipe holding a fresh 16-byte
/// nonce, with the write end already closed (so the child reads nonce
/// + EOF whenever it runs), and the read end placed at the lowest free
/// descriptor >= 100 with CLOEXEC cleared so it survives exec.
///
/// The nonce is NOT a secret (it rides the environment in the clear
/// next to the fd number); its job is binding — it ties this pipe to
/// this spawn, so a coincidental pipe at the same number cannot
/// validate (2^-128). The unforgeability comes from pipe possession,
/// not nonce secrecy: env text can be copied, an inherited fd cannot.
///
/// Mint failure is fail-open: any failure returns None and the caller
/// spawns without proof keys, i.e. today's behavior — a spawn must
/// never fail because the proof could not be minted. The caller closes
/// the returned fd after the spawn lands (like every other
/// parent-side dup); the child closes its own copy on successful
/// validation.
pub(crate) fn mint_respawn_proof(nonce_hex: &str) -> Option<(RawFd, String)> {
    let nonce = respawn_proof_nonce_bytes(nonce_hex)?;
    let mut pipe_fds = [0 as RawFd; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return None;
    }
    let (read_end, write_end) = (pipe_fds[0], pipe_fds[1]);
    // Write the nonce, EINTR-tolerant; the 16 bytes always fit the
    // pipe buffer, so this completes before the write end closes.
    let mut written = 0;
    while written < nonce.len() {
        let n = unsafe {
            libc::write(
                write_end,
                nonce[written..].as_ptr() as *const libc::c_void,
                nonce.len() - written,
            )
        };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            unsafe {
                libc::close(read_end);
                libc::close(write_end);
            }
            return None;
        }
        if n == 0 {
            unsafe {
                libc::close(read_end);
                libc::close(write_end);
            }
            return None;
        }
        written += n as usize;
    }
    // Close the write end BEFORE spawn: the pipe now holds nonce +
    // EOF, so the proof is valid from spawn until consumed regardless
    // of when the spawner dies — the time-independence the orphan
    // race needs.
    unsafe {
        libc::close(write_end);
    }
    // Lowest free descriptor >= 100 (above stdio and the coproc
    // save-slot scan 10..64; F_DUPFD_CLOEXEC skips open fds, so the
    // number can never collide with a carrier). CLOEXEC first so a
    // racing spawn cannot observe it half-staged; cleared below so it
    // survives exec.
    let chosen = unsafe { libc::fcntl(read_end, libc::F_DUPFD_CLOEXEC, 100) };
    if chosen < 0 {
        unsafe {
            libc::close(read_end);
        }
        return None;
    }
    unsafe {
        libc::close(read_end);
    }
    if set_inheritable(chosen, true).is_err() {
        unsafe {
            libc::close(chosen);
        }
        return None;
    }
    Some((chosen, nonce_hex.to_string()))
}

pub fn handle_to_file(h: HANDLE) -> std::fs::File {
    // Takes ownership of the fd, mirroring the Windows side where
    // handle_to_file wraps the raw handle into an owned File (no duplicate).
    // The old unix body dup'd and owned the COPY, leaking the caller's
    // original: BackgroundCapture::duplicate_write (shell_options.rs) hands
    // its fresh write-end dup in here, and the leaked intermediate kept the
    // capture pipe's write end open in the parent forever, so
    // BackgroundCapture::drain's close-then-read-to-EOF could never see EOF
    // -- the redir7.sub backquote-comsub hang (rubash#225), where GNU's
    // comsub read (subst.c:7434-7443: parent closes fildes[1], then
    // read_comsub) completes once every inherited writer exits.
    unsafe { std::fs::File::from_raw_fd(h) }
}

/// Anonymous pipe for background-job output capture (rubash#169); both ends
/// CLOEXEC — per-child writers go through `duplicate_handle_inheritable`.
pub fn create_capture_pipe() -> std::io::Result<(HANDLE, HANDLE)> {
    let mut fds = [0 as libc::c_int; 2];
    // libc::pipe2 is glibc/musl-only; Darwin exposes pipe + fcntl instead.
    #[cfg(target_os = "linux")]
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(not(target_os = "linux"))]
    for fd in fds {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    }
    // rubash#3917: keep internal plumbing out of the user range — a
    // persistent `exec N>file` now dup2's onto its kernel number,
    // and a capture pipe sitting on a low number would be the
    // descriptor it displaces. move_to_high_fd returns the fd
    // unchanged when no high slot is free (GNU's own fallback,
    // general.c:730-733), so this never fails the pipe.
    let read_end = move_to_high_fd(fds[0]);
    let write_end = move_to_high_fd(fds[1]);
    Ok((read_end, write_end))
}

/// Read all bytes currently available without blocking (rubash#169
/// background-capture drain): flip O_NONBLOCK, read until EAGAIN, restore.
pub fn read_available(file: &std::fs::File) -> std::io::Result<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let got = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if got > 0 {
            out.extend_from_slice(&chunk[..got as usize]);
            continue;
        }
        if got == 0 {
            break; // EOF: every writer (incl. the original end) is gone
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            break;
        }
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
        return Err(err);
    }
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
    Ok(out)
}

pub fn read_some(h: HANDLE, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n.max(1)];
    let got = loop {
        let got = unsafe { libc::read(h, buf.as_mut_ptr().cast(), buf.len()) };
        if got >= 0 {
            break got;
        }
        let err = std::io::Error::last_os_error();
        // A caught signal (the kernel-signal backend, builtins/kill.rs) must
        // not fake EOF or fail the read; retry and let the pending-signal
        // queue dispatch at the next command boundary.
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        // Broken pipe on Unix reads as EPIPE -- an EOF-shaped end for the
        // coproc drain loop, matching the Windows ERROR_BROKEN_PIPE mapping.
        if err.raw_os_error() == Some(libc::EPIPE) {
            return Ok(Vec::new());
        }
        return Err(err);
    };
    buf.truncate(got as usize);
    Ok(buf)
}

/// Write all bytes to an opened redirect-target `File` through the
/// raw-handle primitive (rubash#2102). A netopen target's File wraps a
/// SOCKET handle: std's `File::write_all` reaches WriteFile on Windows,
/// which rejects a socket with ERROR_INVALID_PARAMETER — every write
/// failed `Invalid argument`, live peer or dead, and a dead peer never
/// surfaced its real error. The primitive routes sockets through send
/// (see windows_impl::is_socket_handle / rubash#1976), so a closed peer
/// reports its genuine failure (the EPIPE/ECONNRESET family) through
/// sh_chkwrite. On Unix the primitive is the same write(2) std issues.
pub fn write_file_all(file: &std::fs::File, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    write_all(file.as_raw_fd(), bytes)
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

/// Absolute seek — GNU read.def zsyncfd (builtins/read.def:940) parity,
/// see windows_impl::seek_absolute.
pub fn seek_absolute(h: HANDLE, pos: u64) -> std::io::Result<()> {
    // 32-bit unixes (armv7 android) have off_t = i32; saturate instead of
    // truncating (matching zsyncfd's behavior for absurd offsets).
    let offset = libc::off_t::try_from(pos).unwrap_or(libc::off_t::MAX);
    if unsafe { libc::lseek(h, offset, libc::SEEK_SET) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Current file position.
pub fn file_position(h: HANDLE) -> std::io::Result<u64> {
    let pos = unsafe { libc::lseek(h, 0, libc::SEEK_CUR) };
    if pos < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(pos as u64)
}

pub fn process_std_handle(fd: u32) -> HANDLE {
    match fd {
        0 => 0,
        1 => 1,
        _ => 2,
    }
}

/// Live direction of an open descriptor: `(readable, writable)` from the
/// F_GETFL access mode, or `None` when the descriptor is not open.
pub fn fd_directions(fd: u32) -> Option<(bool, bool)> {
    let flags = unsafe { libc::fcntl(fd as libc::c_int, libc::F_GETFL, 0) };
    if flags < 0 {
        return None;
    }
    match flags & libc::O_ACCMODE {
        libc::O_RDONLY => Some((true, false)),
        libc::O_WRONLY => Some((false, true)),
        _ => Some((true, true)),
    }
}

/// Descriptors >= 3 that are open in this process, each with its live
/// direction — the process-inherited descriptor set a fresh shell starts
/// with. GNU keeps no registry of inherited descriptors: redir.c
/// validates a dup source against the kernel (`fcntl (redirector,
/// F_GETFD, 0) != -1`, redir.c:1160, do_redirection_internal's r_dup
/// arm), so kernel state IS the ground truth rubash's virtual FdTable
/// must be seeded from at Executor::new (rubash#1557).
///
/// Enumeration prefers /proc/self/fd (exact at any descriptor number);
/// the directory stream's own descriptor appears in the listing, so
/// every candidate is validated through `fd_directions` after the
/// stream is dropped — the stream descriptor is closed by then and
/// drops out. Without /proc, probe 3..=1023 directly.
pub fn inherited_open_fds() -> Vec<(u32, bool, bool)> {
    let mut candidates: Vec<u32> = Vec::new();
    let mut listed = false;
    if let Ok(stream) = std::fs::read_dir("/proc/self/fd") {
        listed = true;
        for entry in stream.flatten() {
            if let Ok(fd) = entry.file_name().to_string_lossy().parse::<u32>() {
                if fd >= 3 {
                    candidates.push(fd);
                }
            }
        }
    }
    if !listed {
        candidates = (3..=1023).collect();
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
        .into_iter()
        .filter_map(|fd| fd_directions(fd).map(|(readable, writable)| (fd, readable, writable)))
        .collect()
}

/// Display/reopen path label for an inherited descriptor: the readlink
/// target when it names a real filesystem object, else the /dev/fd alias
/// (pipes, sockets and anon inodes readlink to `pipe:[...]`-style
/// pseudo-names that no open(2) accepts).
pub fn inherited_fd_path(fd: u32) -> std::path::PathBuf {
    if let Ok(target) = std::fs::read_link(format!("/proc/self/fd/{fd}")) {
        if target.is_absolute() && !target.to_string_lossy().contains(":[") {
            return target;
        }
    }
    std::path::PathBuf::from(format!("/dev/fd/{fd}"))
}

fn open_flags_for(append: bool, create_new: bool) -> (libc::c_int, libc::mode_t) {
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

fn open_wr(
    path: &std::path::Path,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> std::io::Result<HANDLE> {
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
    /// G-02: the child's working directory (the spawning session's cwd).
    /// None keeps the process cwd (host-level spawns with no session).
    pub cwd: Option<std::path::PathBuf>,
    /// rubash#3268 (e): raw fd of the spawning session's cwd anchor,
    /// spent only when `cwd` no longer resolves (the directory was
    /// deleted): the child is fchdir'd into the anchor inode between
    /// fork and exec — the same landing stamp_spawn_cwd gives
    /// Command-based spawns. The fd is borrowed: it must stay open in
    /// this process for the duration of the spawn call. None (or a
    /// resolvable `cwd`) leaves the spawn exactly as before.
    pub cwd_anchor_fd: Option<HANDLE>,
    /// rubash#2241: job-control grouping for async respawns — the child
    /// takes its own process group between fork and exec (GNU jobs.c:2390
    /// make_child) and the parent repeats the call after spawn
    /// (jobs.c:2455). Set from the spawning shell's monitor option.
    pub new_process_group: bool,
}

#[derive(Debug)]
pub struct SpawnedChild {
    inner: SpawnedChildInner,
}

#[derive(Debug)]
enum SpawnedChildInner {
    Std(std::process::Child),
    /// A child created by a raw `fork()` (rubash#2925: the direct async
    /// spawn cannot use `Command::spawn`, whose error-pipe sync blocks
    /// the parent until the child execs and loses the kill-before-ignore
    /// race). Stable Rust cannot wrap a bare pid in a `Child`, so this
    /// variant owns the pid plus the cached reap status — the same cache
    /// `Child` keeps, so a second `try_wait`/`wait` returns the recorded
    /// status instead of ECHILD (the multi-member wait's ECHILD arm
    /// consults the handle's word exactly this way).
    RawFork {
        pid: u32,
        status: Option<std::process::ExitStatus>,
    },
}

impl SpawnedChild {
    pub fn id(&self) -> u32 {
        match &self.inner {
            SpawnedChildInner::Std(child) => child.id(),
            SpawnedChildInner::RawFork { pid, .. } => *pid,
        }
    }

    /// Construct from a bare pid (raw-fork spawn, rubash#2925).
    pub fn from_pid(pid: u32) -> Self {
        Self {
            inner: SpawnedChildInner::RawFork { pid, status: None },
        }
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.try_wait(),
            SpawnedChildInner::RawFork { pid, status } => {
                if let Some(done) = status {
                    return Ok(Some(done.clone()));
                }
                let mut raw: libc::c_int = 0;
                let reaped = unsafe { libc::waitpid(*pid as libc::pid_t, &mut raw, libc::WNOHANG) };
                if reaped == *pid as libc::pid_t {
                    let done = std::process::ExitStatus::from_raw(raw);
                    *status = Some(done.clone());
                    Ok(Some(done))
                } else if reaped == 0 {
                    Ok(None)
                } else {
                    Err(std::io::Error::last_os_error())
                }
            }
        }
    }

    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.wait(),
            SpawnedChildInner::RawFork { pid, status } => {
                if let Some(done) = status {
                    return Ok(done.clone());
                }
                loop {
                    let mut raw: libc::c_int = 0;
                    let reaped = unsafe { libc::waitpid(*pid as libc::pid_t, &mut raw, 0) };
                    if reaped == *pid as libc::pid_t {
                        let done = std::process::ExitStatus::from_raw(raw);
                        *status = Some(done.clone());
                        return Ok(done);
                    }
                    if reaped < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.raw_os_error() == Some(libc::EINTR) {
                            continue;
                        }
                        return Err(error);
                    }
                }
            }
        }
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            SpawnedChildInner::Std(child) => child.kill(),
            SpawnedChildInner::RawFork { pid, .. } => {
                if unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) } == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
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

/// Spawn with exactly the whitelisted fd numbers visible to the child.
/// Unix translation of the HANDLE_LIST whitelist: between fork and exec,
/// dup2 each spec fd onto its own number, then close everything else.
pub fn spawn_whitelisted(spec: &WhitelistedSpawn) -> std::io::Result<SpawnedChild> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        // rubash#3198: a session cwd whose directory has been unlinked
        // must not refuse the respawn — `current_dir` re-resolves the
        // path in the child and fails ENOENT for a deleted directory.
        //
        // rubash#3268 (e): "unstamped" used to be the whole story —
        // the child silently inherited THIS process's cwd, a live but
        // foreign directory, and was born there believing it was the
        // session's directory. When the caller supplies the session's
        // anchor fd, land the child in the anchor inode instead
        // (fchdir between fork and exec — the stamp_spawn_cwd
        // mechanism the executor's own spawn faces use for foreign
        // children): the respawned child is then born inside the
        // deleted directory itself, its birth getcwd fails, and its
        // own birth machinery (birth anchor on "." plus the
        // transported-PWD adoption in Executor::new) reconstructs the
        // session there. GNU's forked child never leaves the inode.
        if cwd.is_dir() {
            cmd.current_dir(cwd);
        } else if let Some(anchor_fd) = spec.cwd_anchor_fd {
            // SAFETY: pre_exec runs in the forked child before exec;
            // fchdir is async-signal-safe, and the fd is the caller's
            // live anchor handle (open in this process, inherited
            // across the fork).
            unsafe {
                use std::os::unix::process::CommandExt as _;
                cmd.pre_exec(move || {
                    if libc::fchdir(anchor_fd) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
    }
    // The spec's env IS the child environment (the caller composes it from
    // the ingested process environment plus the shell's exported state,
    // then strips what a
    // subshell must not inherit — e.g. the __RUBASH_TRAP* keys, POSIX
    // 2.11: caught traps reset in a subshell). Merging with this process's
    // environment instead would resurrect every stripped key as a stale
    // OS-env copy (trap.tests: background `rubash -c` children ran the
    // parent's inherited EXIT trap at exit — three spurious "exiting").
    cmd.env_clear();
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[0])?) });
    cmd.stdout(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[1])?) });
    cmd.stderr(unsafe { Stdio::from_raw_fd(dup_for_child(spec.std_handles[2])?) });
    // extra_handles: the Windows side maps "handle value == child fd number",
    // and the __RUBASH_FD_HANDLE_{fd} env keys the child replays its fd table
    // from carry those numbers, so each extra must arrive AT ITS OWN NUMBER:
    // dup2(fd, fd) between fork and exec pins the number and clears CLOEXEC
    // (the inheritable-duplicate effect).
    let extras = spec.extra_handles.clone();
    let new_process_group = spec.new_process_group;
    // rubash#3917 (round 5, follow-up): the whitelist contract above
    // ("dup2 each spec fd onto its own number, then close everything
    // else") was only half implemented — the close half was missing, so
    // a runtime descriptor leaked into every respawned child and was
    // adopted there as a user fd, resurrecting a number the user had
    // explicitly closed (`exec N<&-`, issue1184 f4cond S1/S2). The leak
    // is the signal self-pipe socketpair: installed lazily on the first
    // Handler wish (kill.rs install), its ends land on the lowest free
    // fds — fd 3 once the close above freed it — and, being non-CLOEXEC,
    // survive execve into the child because this spawn never closed
    // them. Highest fd the close loop must cover: every fd the parent
    // holds, floored at 1024 (the spawn_async_direct floor).
    let mut max_fd: libc::c_int = 1024;
    for &extra in &extras {
        max_fd = max_fd.max(extra);
    }
    // The cwd anchor is spent by the fchdir pre_exec above (registered
    // first, so it runs first); keep it out of the close loop as
    // belt-and-braces against hook ordering. It is a high directory fd
    // (#3640 move_to_high_fd), not a user-visible low number.
    let anchor_fd = spec.cwd_anchor_fd;
    if let Some(anchor) = anchor_fd {
        max_fd = max_fd.max(anchor);
    }
    // rubash#3219: if the composed spawn env carries emulated-limit
    // cells (a fork region's recorded ulimit pairs ride the spawn env
    // for child shells), the child is born with those pairs applied —
    // the virtualized set never moved the spawning process's hard side.
    let limit_pairs = crate::builtins::ulimit::emulated_limit_pairs_from_env(&spec.env);
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
            // Close everything else above fd 2: stdio (0,1,2) was already
            // arranged by std before pre_exec, the extras above were just
            // pinned to their numbers, and the cwd anchor is excluded (see
            // above). Every other inherited descriptor — the signal
            // self-pipe included — dies here, so the child's startup
            // adoption can only see what the parent's table stamped.
            // Async-signal-safe: close(2) only, no allocation (the keep
            // check is a linear scan of the already-moved extras).
            for fd in 3..=max_fd {
                if extras.contains(&fd) {
                    continue;
                }
                if Some(fd) == anchor_fd {
                    continue;
                }
                // The whole pre_exec closure body is one unsafe block,
                // so close() needs no nested unsafe of its own.
                libc::close(fd);
            }
            // GNU's forked children restore the entry signal dispositions
            // between fork and exec (trap.c:1489 restore_original_signals,
            // rubash#229): a policy/trap ignore in the shell must not make
            // exec'd children unkillable by SIGQUIT et al. rubash#737/#866:
            // the PDEATHSIG arm is decided INSIDE this hook from the
            // spawning process's in-process wrapper role (never the ambient
            // env — rubash#866 — and the race is adjudicated against the
            // expected parent pid, never the constant 1 — rubash#867).
            crate::builtins::kill::child_pre_exec_reset()?;
            // rubash#2241: job-control grouping, decided by the parent
            // and carried into the closure as a plain bool (never
            // re-read from ambient state in the child).
            if new_process_group {
                crate::builtins::kill::child_pre_exec_setpgid()?;
            }
            // rubash#3219: apply the region's recorded ulimit pairs
            // carried in the spawn env (computed above, parent-side).
            crate::builtins::ulimit::apply_emulated_limits_in_child(&limit_pairs);
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    // rubash#2241: parent half of the grouping (GNU jobs.c:2455).
    if new_process_group {
        crate::builtins::kill::group_spawned_child(child.id());
    }
    // Parent side: the extras were dup2'd onto their own numbers in the
    // child by pre_exec; the parent's originals stay open (callers own them,
    // as on Windows).
    Ok(SpawnedChild {
        inner: SpawnedChildInner::Std(child),
    })
}

fn dup_for_child(fd: HANDLE) -> std::io::Result<HANDLE> {
    // Dup off the caller's fd (so the child's stdio is independent of
    // lifetime management in the parent) and mark the child-side fd to
    // survive exec.
    let n = duplicate_handle(fd)?;
    set_inheritable(n, true)?;
    Ok(n)
}

/// Unix counterpart of the Windows sliced child wait (windows_impl.rs
/// wait_child_slice): kernel signal delivery interrupts waitpid and the
/// signal_hook backend (builtins/kill.rs kernel_signals) records arrivals
/// in memory, which the unthrottled take_all_signals drains at the command
/// boundary — the plain blocking wait already carries GNU's
/// interruptible-wait semantics (jobs.c:3064 wait_for), so no slicing is
/// needed and the timeout is unused.
pub fn wait_child_slice(
    child: &mut std::process::Child,
    _timeout: std::time::Duration,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    child.wait().map(Some)
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
        use std::os::fd::IntoRawFd;
        let (r, w) = std::io::pipe().unwrap();
        // into_raw_fd, not as_raw_fd: the test's trailing close_handle must
        // be the ONLY close — as_raw_fd leaves `r` owning the fd too, and
        // its Drop closes it a second time, tripping the io_safety abort.
        let rfd = r.into_raw_fd();
        drop(w);
        let buf = read_some(rfd, 64).unwrap();
        assert!(buf.is_empty());
        close_handle(rfd);
    }

    #[test]
    fn spawn_whitelisted_keeps_extra_handles_at_their_numbers() {
        use std::os::fd::AsRawFd;
        let dir = std::env::temp_dir();
        let path = dir.join("rubash-fd-extra-probe");
        std::fs::write(&path, b"rubash-extra-probe").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let extra = duplicate_handle_inheritable(file.as_raw_fd()).unwrap();
        // The child must find the SAME fd number open: the
        // __RUBASH_FD_HANDLE_{fd} env keys replayed in init.rs name these
        // numbers, so a spawn that renumbers them breaks fd>=3 inheritance.
        let mut child = spawn_whitelisted(&WhitelistedSpawn {
            // bash, not POSIX sh: the SHELL_FD_BASE floor puts the extra
            // at >= 10 and dash rejects a multi-digit dup target outright
            // ("sh: 1: Syntax error: Bad fd number" for <&10 under dash
            // 0.5.12 — verified: the literal AND the variable-expanded
            // form are both refused, left side included). The fd-number
            // contract under test cannot be expressed in the dash lexer
            // past fd 9, so the numeric-dup probe needs bash; the
            // /proc/self/fd alternative stays rejected because macOS has
            // no /proc.
            program: "bash".into(),
            args: vec![
                "-c".to_string(),
                format!("exec grep -q rubash-extra-probe <&{extra}"),
            ],
            env: Vec::new(),
            std_handles: [0, 1, 2],
            extra_handles: vec![extra],
            cwd: None,
            cwd_anchor_fd: None,
            new_process_group: false,
        })
        .unwrap();
        let status = child.wait().unwrap();
        close_handle(extra);
        drop(file);
        let _ = std::fs::remove_file(&path);
        assert!(status.success(), "child could not read extra fd {extra}");
    }
}

/// Take ownership of a pipe end as a raw fd (coproc endpoints).
pub fn into_handle(pipe: impl std::os::unix::io::IntoRawFd) -> HANDLE {
    pipe.into_raw_fd()
}
