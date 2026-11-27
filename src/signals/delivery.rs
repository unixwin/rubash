//! Signal delivery primitives, console-signal hooks, and process-spawn signal policy.

#[cfg(unix)]
use crate::signals::dispositions::kernel_signals;
use crate::signals::mailbox::{
    queue_self_signal, signal_mailbox_dir, signal_marker_path, unregister_signal_mailbox,
};
use std::io;

pub fn process_exists(pid: u32) -> bool {
    pid == std::process::id() || signal_process(pid, 0).is_ok_or_permission_denied()
}

/// Plain kill(2) to one recorded pid: the delivery primitive for the
/// kill builtin's pid operands and for fg/bg's shared-group fallback.
/// It is NOT the resume model for a job-control job: GNU continue_job
/// (jobs.c:3926-3928) addresses the resume SIGCONT to the job's process
/// GROUP with killpg, because the group also holds processes the leader
/// forked after spawn — which no pid list the shell keeps can name — so
/// per-pid delivery over the job's recorded pids is not a killpg
/// equivalent there (rubash#2622). fg/bg use
/// [`send_signal_to_process_group`] whenever the job-control channel is
/// active; this per-pid path remains for the fallback model, where the
/// job's members all live in the shell's own process group and every
/// member is a recorded pid.
pub fn send_signal(pid: u32, signal: i32) -> Result<(), &'static str> {
    signal_process(pid, signal)
}

/// killpg(2) to a job's process group — GNU continue_job's resume
/// delivery (jobs.c:3926-3928 `killpg (jobs[job]->pgrp, SIGCONT)`;
/// start_job is jobs.c:3826). Group addressing reaches every member of
/// the group, including children the job's leader forked after spawn
/// that the shell never recorded as job pids (rubash#2622).
#[cfg(unix)]
pub fn send_signal_to_process_group(pgrp: i32, signal: i32) -> Result<(), &'static str> {
    let result = unsafe { libc::killpg(pgrp as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Err("No such process"),
        Some(libc::EPERM) => Err("Permission denied"),
        _ => Err("Failed to signal process"),
    }
}

/// Record a console `CTRL_C_EVENT` into this process's self-signal queue.
///
/// The niubash host owns `SetConsoleCtrlHandler` on Windows and must not
/// terminate the shell; it calls this so the engine's wait-slice and
/// command-boundary polls (`observe_signals_while_blocked` /
/// `run_pending_signal_traps`) observe Ctrl+C the same way they would a
/// mailbox SIGINT from another process. Without it, an in-process wait
/// such as `sleep_interruptibly` never sees the console event and the
/// command runs to completion (niubash D8).
///
/// Unix: no-op — the kernel `signal_hook` backend already receives SIGINT
/// for the process; a second injection would double-dispatch.
pub(crate) fn note_console_ctrl_c() {
    // PLAN row 11 (M-3): this hook is engine plumbing now. The
    // consumed faces are `script_driver::inject_console_ctrl_c`
    // (host handlers) and `Executor::inject_signal` /
    // `Executor::interrupt` (Tier A). Demoted to `pub(crate)`
    // in the Session-migration follow-up recorded in the API
    // ledger: niubash's migration consumes the faces above, so
    // nothing outside this crate names the hook any more.
    #[cfg(not(unix))]
    {
        // Linux mailbox wire format: SIGINT = 2.
        queue_self_signal(2);
    }
}

/// Record a console `CTRL_Z` (the host's ^Z keypress) into this
/// process's self-signal queue — the TSTP twin of
/// [`note_console_ctrl_c`] (rubash#3467, companion to niubash#877).
///
/// The niubash host owns the console input on Windows: ^Z is a
/// keypress, not a console control event, so no kernel delivery
/// exists for it — the host's watch thread calls the driver face
/// (`script_driver::inject_console_ctrl_z`) and this hook queues
/// signal 20 the same way a mailbox SIGTSTP from another process
/// would arrive. The foreground waits observe it through
/// `observe_signals_while_blocked` and stop the job instead of
/// ending it at 128+20; a trapped TSTP is re-queued there exactly
/// like a trapped SIGINT, so the trap runs and the command is not
/// stopped (GNU: a trapped SIGTSTP does not stop the shell).
///
/// Unix: no-op — the kernel delivers SIGTSTP to the foreground
/// process group itself and waitpid reports the stop; a second
/// injection would double-dispatch.
pub(crate) fn note_console_ctrl_z() {
    #[cfg(not(unix))]
    {
        // Linux mailbox wire format: SIGTSTP = 20.
        queue_self_signal(20);
    }
}

// rubash#375/perf3: the per-command signal-poll's Windows-port catch_flag.
// GNU run_pending_traps (trap.c:336) is `if (catch_flag == 0) return;` —
// one int test, because the KERNEL handler (sig.c trap_handler) sets
// catch_flag asynchronously the instant a signal arrives. Windows has no
// kernel signal delivery into the shell process, so the file mailbox is
// the transport and the empty check used to be a directory pattern query
// every 64th command (~100us per query under real-time scanning,
// 1.7us/command amortized — 15% of the 20000-iteration null-command
// loop's wall). A named auto-reset kernel Event is the faithful
// catch_flag port: cross-process senders SetEvent after their mailbox
// rename; the per-command check is WaitForSingleObject(, 0) on a handle
// opened once (~sub-us); the filesystem query runs only when the event
// fired OR on the retained every-64th safety tick (senders that cannot
// set the event — a different binary version, a restricted session —
// keep today's delivery cadence). Auto-reset coalescing is exactly the
// "at least one delivery is pending" contract: the drain reads ALL
// pending entries whenever it runs, and a delivery landing between the
// wait and the drain leaves the event set for the next command.
#[cfg(windows)]
pub(in crate::signals) mod signal_hint {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const EVENT_NAME_PREFIX: &str = "Local
ubash-signal-hint-";

    fn event_name(pid: u32) -> Vec<u16> {
        let wide: Vec<u16> = format!("{EVENT_NAME_PREFIX}{pid}").encode_utf16().collect();
        let mut name = wide;
        name.push(0);
        name
    }

    /// A kernel HANDLE is an opaque integer the kernel interprets; waiting
    /// on it from any thread is safe (object-level synchronization), so the
    /// OnceLock cell can carry it across threads.
    struct HintHandle(HANDLE);
    unsafe impl Send for HintHandle {}
    unsafe impl Sync for HintHandle {}

    fn open_or_create(pid: u32) -> Option<HintHandle> {
        // CreateEventW on an existing name returns the existing event with
        // ERROR_ALREADY_EXISTS — both directions are fine (the handle is
        // ours to wait on; the kernel refcounts the object).
        let handle = unsafe {
            CreateEventW(
                core::ptr::null(),
                0, // auto-reset
                0, // initially nonsignaled
                event_name(pid).as_ptr(),
            )
        };
        (!handle.is_null()).then(|| HintHandle(handle))
    }

    fn hint_handle() -> Option<HANDLE> {
        static HANDLE_CELL: OnceLock<Option<HintHandle>> = OnceLock::new();
        // The handle lives for the whole process: the OS closes it at exit
        // (same lifecycle as the {pid}.alive marker) — one handle per
        // process is the documented trade.
        HANDLE_CELL
            .get_or_init(|| open_or_create(std::process::id()))
            .as_ref()
            .map(|holder| holder.0)
    }

    /// True when a cross-process sender signaled a pending mailbox entry.
    /// Any failure (restricted session, exhausted handles) reads as false —
    /// the periodic fallback tick keeps polling the filesystem, so delivery
    /// semantics never depend on the event.
    pub fn signaled() -> bool {
        let Some(handle) = hint_handle() else {
            return false;
        };
        const WAIT_OBJECT_0: u32 = 0;
        unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 }
    }

    /// Whether this process's hint event exists. When it does, every
    /// same-version sender pulses it at delivery time, so the filesystem
    /// fallback cadence stretches (eventless senders — an older binary, a
    /// foreign tool writing the mailbox format — still get delivered, just
    /// at the stretched cadence instead of the tight one).
    pub fn available() -> bool {
        hint_handle().is_some()
    }

    /// Best-effort signal from the SENDER side (deliver_rubash_signal):
    /// open the target's event and pulse it. Failures are ignored — the
    /// mailbox entry itself is the source of truth.
    pub fn signal_target(pid: u32) {
        use windows_sys::Win32::System::Threading::{OpenEventW, SetEvent};

        const EVENT_MODIFY_STATE: u32 = 0x0002;
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, event_name(pid).as_ptr()) };
        if handle.is_null() {
            return;
        }
        unsafe {
            SetEvent(handle);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
pub(in crate::signals) mod signal_hint {
    #[cfg_attr(unix, allow(dead_code))]
    pub fn signaled() -> bool {
        false
    }
    #[allow(unused_variables)]
    pub fn signal_target(pid: u32) {}
}

pub(crate) fn deliver_rubash_signal(pid: u32, signal: i32) -> io::Result<bool> {
    if !signal_marker_path(pid).is_file() {
        return Ok(false);
    }
    // A background Rubash process owns its own mailbox even though it
    // inherits the parent shell identity for expansion semantics. Do not
    // reject that child merely because the marker names the parent.
    // SIGKILL is not trappable. STOP/TSTP/CONT must use the native Windows
    // process-state backend rather than a mailbox event.
    //
    // NOTE: these are Linux wire-format numbers (9=KILL, 17=CHLD, 18=CONT,
    // 19=STOP), and this whole branch is reachable only on the non-unix
    // mailbox path: on unix nothing ever creates the `{pid}.alive` marker
    // (register_signal_mailbox installs the kernel backend instead), so the
    // `is_file()` check above returns first and delivery routes through the
    // real kill(2) in signal_process with platform numbering. Do NOT
    // constant-ize these against libc — that would renumber the Windows
    // mailbox wire format.
    if matches!(signal, 9 | 17 | 18 | 19) {
        return Ok(false);
    }
    if !process_exists(pid) {
        unregister_signal_mailbox(pid);
        return Ok(false);
    }
    if signal == 0 {
        return Ok(true);
    }

    // In-process delivery: a signal to the current shell never touches the
    // filesystem. The per-command signal poll drains this queue directly;
    // on Windows each intercepted filesystem operation costs ~0.25-0.5ms
    // under real-time scanning, so both this write and the reader's poll
    // must stay off the per-command hot path.
    if pid == std::process::id() {
        queue_self_signal(signal);
        return Ok(true);
    }

    std::fs::create_dir_all(signal_mailbox_dir())?;
    // Deliver as a unique per-signal entry: write to a .part file and
    // rename it into place. The rename is atomic, so the reading shell
    // either misses the entry entirely or observes it fully written; a
    // shared append-queue could lose the write to the reader's
    // read-then-remove window.
    static SIGNAL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SIGNAL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = format!("{pid}.q.{}.{}", std::process::id(), seq);
    let part = signal_mailbox_dir().join(format!("{base}.part"));
    let full = signal_mailbox_dir().join(&base);
    std::fs::write(
        &part,
        format!(
            "{signal}
"
        ),
    )?;
    std::fs::rename(&part, &full)?;
    signal_hint::signal_target(pid);
    Ok(true)
}

#[cfg(windows)]
pub(crate) fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
    };

    // Linux numbering: CONT=18 resumes, STOP=19/TSTP=20 suspend. SIGCHLD
    // (17) is delivered-but-ignored by default, so kill -CHLD succeeds as a
    // no-op instead of terminating (GNU kill.def sends it through kill(2)
    // where the default disposition is SIG_DFL-ignored).
    if signal == 17 {
        return Ok(());
    }
    if signal == 18 || signal == 19 || signal == 20 {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == -1isize as _ {
            return Err("Cannot enumerate process threads");
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        let mut found = false;
        let mut result = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while result {
            if entry.th32OwnerProcessID == pid {
                found = true;
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Cannot open process thread");
                }
                let failed = if signal == 18 {
                    unsafe { ResumeThread(thread) == u32::MAX }
                } else {
                    unsafe { SuspendThread(thread) == u32::MAX }
                };
                unsafe { CloseHandle(thread) };
                if failed {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Failed to change process state");
                }
            }
            result = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        return if found {
            Ok(())
        } else {
            Err("No such process")
        };
    }

    use windows_sys::Win32::System::Threading::PROCESS_TERMINATE;
    let access = if signal == 0 {
        PROCESS_QUERY_LIMITED_INFORMATION
    } else {
        PROCESS_TERMINATE
    };

    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        return Err(match error {
            ERROR_ACCESS_DENIED => "Permission denied",
            ERROR_INVALID_PARAMETER => "No such process",
            _ => "Cannot open process",
        });
    }

    if signal == 0 {
        let mut exit_code = 0;
        let process_is_active = unsafe {
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32
        };
        if !process_is_active {
            unsafe { CloseHandle(handle) };
            return Err("No such process");
        }
    } else if unsafe { TerminateProcess(handle, (128 + signal) as u32) == 0 } {
        unsafe { CloseHandle(handle) };
        return Err("Failed to terminate process");
    }

    unsafe { CloseHandle(handle) };
    Ok(())
}

#[cfg(unix)]
pub(crate) fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    // A signal this shell sends to ITSELF must be delivered before the
    // kill builtin returns: GNU's single-threaded bash runs the SIGUSR1
    // handler during kill(2), so the pending-trap dispatch at the very
    // next command boundary (still inside the calling function) sees it —
    // trap9.sub's `func() { kill -USR1 $$; }` runs its trap inside func.
    // This process is multithreaded, and kill(2) hands the signal to any
    // thread that does not block it, so the handler's queue write may lag
    // past the function's last boundary (~35% of runs: the trap fired
    // after the function returned and its `return` errored at top level).
    // raise(3) targets the calling thread: the handler completes before
    // raise returns, restoring GNU's observable ordering.
    if pid == std::process::id() {
        let result = unsafe { libc::raise(signal) };
        return if result == 0 {
            Ok(())
        } else {
            Err("Failed to signal process")
        };
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }

    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Err("No such process"),
        Some(libc::EPERM) => Err("Permission denied"),
        _ => Err("Failed to signal process"),
    }
}

/// kill(2) with a signed pid: a negative value addresses a process
/// group (GNU kill.def kill_pid passes the operand through verbatim).
/// rubash#1631.
#[cfg(unix)]
pub(crate) fn signal_process_group(pid: i32, signal: i32) -> Result<(), &'static str> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Err("No such process"),
        Some(libc::EPERM) => Err("Permission denied"),
        _ => Err("Failed to signal process"),
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn signal_process(_pid: u32, signal: i32) -> Result<(), &'static str> {
    if signal == 0 {
        Err("No such process")
    } else {
        Err("Failed to signal process")
    }
}

trait KillResultExt {
    fn is_ok_or_permission_denied(&self) -> bool;
}

impl KillResultExt for Result<(), &'static str> {
    fn is_ok_or_permission_denied(&self) -> bool {
        self.is_ok() || matches!(self, Err(message) if *message == "Permission denied")
    }
}

/// rubash#866: the reserved `__RUBASH_*` namespace key that used to carry
/// the forgeable constant "1" into the wrapper's environment. It now carries
/// an unguessable spawn-time nonce (see `new_background_wrapper_nonce`) and
/// is only ever honored in combination with the matching argv flag
/// (`BACKGROUND_WRAPPER_FLAG`) at handshake time; a user
/// `export __RUBASH_BACKGROUND_WRAPPER=<anything>` arms nothing and is
/// scrubbed from the process environment at startup.
pub const BACKGROUND_WRAPPER_ENV_KEY: &str = "__RUBASH_BACKGROUND_WRAPPER";

/// rubash#3932: the background payload's form, transported to the wrapper
/// respawn child so its async arm knows which GNU face governs the unit's
/// tail (`simple`: the forked async child's unconditional CMD_NO_FORK,
/// execute_cmd.c:4553; `subshell`: an absorbed `( list )` body whose mark
/// consults should_optimize_fork — redirects block, traps gate at spawn,
/// execute_cmd.c:1799-1806; `compound`: a group/for/if/... body that GNU
/// never marks, execute_cmd.c:1756 requiring cm_simple — the child keeps
/// the live level). Handshake-gated like the role nonce: a user-forged
/// value arms nothing.
pub const BACKGROUND_WRAPPER_PAYLOAD_FACE_KEY: &str = "__RUBASH_BG_PAYLOAD_FACE";

/// rubash#866: the internal argv flag carrying the handshake nonce. Like the
/// other `--internal-*` faces it is consumed by the host CLI's startup
/// consume call before any option parser ever sees it; the wrapper role arms
/// only when the flag's nonce and the environment nonce agree (both
/// constructed by the wrapper spawn site in compound_exec), so guessing or
/// exporting the environment name alone is not enough.
///
/// The flag is niu-CLI-specific: a host that never calls
/// `consume_background_wrapper_handshake` (an embedder's own launcher, or a
/// foreign bash on PATH) would reject it as an invalid option and kill the
/// background command. The wrapper spawn site therefore attaches it only
/// when `background_wrapper_flag_consumer_present()` is set — rubash#873
/// CI: the niubash launcher respawned its host CLI with the flag and
/// the launcher's GNU-aligned parser replied
/// `bash: --internal-bg-wrapper=<nonce>: invalid option` + usage, killing
/// every wrapper respawn.
pub const BACKGROUND_WRAPPER_FLAG: &str = "--internal-bg-wrapper=";

/// rubash#873: marks that THIS process's command-line face understands the
/// background-wrapper handshake — set unconditionally at the top of
/// `consume_background_wrapper_handshake`, which rubash's own main() calls
/// at every startup through the Tier B face
/// `script_driver::consume_background_wrapper_handshake` (embedders adopt
/// the same one call through that face). The wrapper spawn
/// site consults it before splicing `BACKGROUND_WRAPPER_FLAG` into a
/// respawned child's argv: a host that never consumes must never RECEIVE
/// the flag, or the child's option parser rejects it and the background
/// command dies (the niubash launcher CI failure). The env nonce channel
/// stays unconditionally: for a non-consuming child it is an inert reserved
/// key (a consumer scrub it at startup; a non-consumer never reads it), and
/// a single channel can never arm — that is the rubash#866 contract.
static BACKGROUND_WRAPPER_FLAG_CONSUMER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// rubash#873: whether this process's CLI consumes the background-wrapper
/// handshake (see `BACKGROUND_WRAPPER_FLAG_CONSUMER`). Gates the argv side
/// of the handshake at the wrapper spawn site.
pub fn background_wrapper_flag_consumer_present() -> bool {
    BACKGROUND_WRAPPER_FLAG_CONSUMER.load(std::sync::atomic::Ordering::Acquire)
}

/// rubash#866: in-process wrapper role. Decided ONCE per process at startup
/// by `consume_background_wrapper_handshake` (nonce handshake) and consulted
/// by `child_pre_exec_reset` between fork and exec — never by reading the
/// ambient environment, which a user script can forge. Atomic because the
/// reader runs in the post-fork child (async-signal-safe: no locks, no
/// allocation — the old `std::env::var` probe allocated).
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static BACKGROUND_WRAPPER_ROLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// rubash#867: the parent pid an armed child must still see from getppid().
/// Captured at handshake time (this process's own pid: the wrapper is the
/// parent of every child it spawns) instead of comparing against the
/// constant 1 — inside a PID namespace a healthy child of the namespace's
/// PID 1 legitimately observes getppid() == 1 before any fork race, and the
/// old constant comparison made it SIGKILL itself at spawn.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static BACKGROUND_WRAPPER_EXPECTED_PARENT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// rubash#866: per-process counter folded into the nonce so two wrappers
/// spawned within the same clock tick never share a handshake value.
static WRAPPER_NONCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// rubash#866: 128 bits of spawn-time handshake material as 32 hex chars.
/// Entropy: wall-clock nanoseconds, pid, a process-local counter and an
/// ASLR-derived stack address, run through SplitMix64. No dependency on a
/// RNG crate; a user script cannot observe or predict the value another
/// process drew at its spawn instant, which is the whole point — the
/// forgeable constant "1" let `export __RUBASH_BACKGROUND_WRAPPER=1` impose
/// PDEATHSIG semantics on children that never went through the wrapper
/// spawn path.
pub fn new_background_wrapper_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = WRAPPER_NONCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let anchor = &counter as *const u64 as u64;
    let mut z = nanos
        ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ counter.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ anchor.rotate_left(17);
    // SplitMix64 finalizer (Steele et al.), two draws for 128 bits.
    let draw = |z: &mut u64| {
        *z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut v = *z;
        v = (v ^ (v >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v = (v ^ (v >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        v ^ (v >> 31)
    };
    let hi = draw(&mut z);
    let lo = draw(&mut z);
    format!("{hi:016x}{lo:016x}")
}

/// rubash#866: consume the background-wrapper spawn handshake — called from
/// main() before any parser or executor touches argv (same consume-on-read
/// face as NIU_INVOKED_AS and `--internal-yes|head|wc`). The wrapper role
/// arms only when the argv flag nonce and the environment nonce AGREE (both
/// were constructed by our own spawn site); afterwards the environment key
/// is scrubbed unconditionally so a user-forged value can neither arm
/// anything nor leak into grandchildren. On mismatch (hand forgery, stale
/// spawn, tampering) the process simply runs without the wrapper role.
///
/// The call itself (regardless of whether a handshake is present) marks the
/// process as a flag consumer — see `BACKGROUND_WRAPPER_FLAG_CONSUMER`.
/// Hosts that adopt this call opt into receiving the argv channel on their
/// own wrapper respawns; hosts that never call it get env-only respawns and
/// the flag never reaches their CLI.
pub(crate) fn consume_background_wrapper_handshake(args: &mut Vec<String>) {
    BACKGROUND_WRAPPER_FLAG_CONSUMER.store(true, std::sync::atomic::Ordering::Release);
    let nonce = match args
        .get(1)
        .and_then(|arg| arg.strip_prefix(BACKGROUND_WRAPPER_FLAG))
    {
        Some(nonce) => nonce.to_owned(),
        None => {
            // No flag: a plain (user-invoked) rubash. Still scrub a forged
            // ambient value — nothing may read it anymore.
            std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
            return;
        }
    };
    args.remove(1);
    if std::env::var(BACKGROUND_WRAPPER_ENV_KEY).as_deref() != Ok(nonce.as_str()) {
        std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        return;
    }
    std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
    #[cfg(unix)]
    {
        BACKGROUND_WRAPPER_ROLE.store(true, std::sync::atomic::Ordering::Release);
        BACKGROUND_WRAPPER_EXPECTED_PARENT
            .store(std::process::id(), std::sync::atomic::Ordering::Release);
        // rubash#2626: a wrapper-role process proxies individually
        // addressed signals to its foreground pipeline (see
        // WRAPPER_RELAY_SIGNALS). Installed here, at role-arming
        // time, because this is the one moment the process is
        // still single-threaded with its entry dispositions
        // untouched — the saved-disposition table the relay falls
        // back to is only meaningful if captured now.
        wrapper_relay_install_once();
    }
}

/// Test/inspection accessor for the in-process wrapper role.
#[cfg(unix)]
pub fn background_wrapper_role_armed() -> bool {
    BACKGROUND_WRAPPER_ROLE.load(std::sync::atomic::Ordering::Acquire)
}

/// rubash#3932: non-unix builds never interpose the wrapper role, so the
/// -c tail exec-replacement face is never suppressed there.
#[cfg(not(unix))]
pub fn background_wrapper_role_armed() -> bool {
    false
}

/// ------------------------------------------------------------------
/// rubash#2626: the background-wrapper signal relay (unix).
///
/// GNU runs a background pipeline's stages as the shell's direct
/// children and `$!` names the LAST stage (execute_cmd.c
/// execute_pipeline; verified against bash 5.3.20: `sleep 51 |
/// sleep 52 &` then `kill -TERM $!` leaves `sleep 51` running).
/// rubash's async spawn interposes a wrapper process (the
/// `--internal-bg-wrapper` respawn above) that forks the stages, so
/// `$!` names the wrapper — and a plain-pid signal addressed to `$!`
/// used to land on the wrapper alone: the wrapper died and the
/// stages were taken down with it by the rubash#737 PDEATHSIG arm,
/// the exact opposite of the GNU fate (the addressed stage dies,
/// its siblings run on).
///
/// The relay makes the wrapper transparent for individually
/// addressed signals while it supervises a foreground pipeline:
/// the executor publishes the pipeline's last stage pid as the
/// relay target for the duration of its wait (see
/// `WrapperRelayTargetGuard`), and a delivery to the wrapper is
/// forwarded to that pid instead of acting on the wrapper. The
/// wrapper then outlives the addressed stage and keeps supervising
/// the survivors, exiting when the whole pipeline has finished —
/// the parent shell's job therefore persists and reports exactly
/// as GNU's does.
///
/// Mechanics, mirroring the fatal set's model in kernel_signals:
/// at role-arming time (once, from
/// `consume_background_wrapper_handshake`, before any other thread
/// exists) the pre-existing disposition of each relay signal is
/// saved and a minimal recording handler is installed; the handler
/// only writes the signal number to a self-pipe (async-signal-safe),
/// and a dedicated thread performs the delivery decision:
///   * the installed disposition is no longer the relay handler
///     (a trap reconcile replaced it, or a reset restored the
///     default): re-raise once — the live owner (engine handler,
///     SIG_DFL, SIG_IGN) disposes of the delivery;
///   * a relay target is published: forward the signal to it;
///   * otherwise restore the saved disposition and re-raise, so
///     an untrapped delivery still kills the wrapper BY the
///     signal and an entry-ignored one is still swallowed.
/// A trap installed by the wrapper's own body therefore keeps
/// priority over the proxy (the reconcile replaces the relay
/// handler), and signals the relay cannot catch (SIGKILL/SIGSTOP)
/// keep the standing rubash#737 contract — PDEATHSIG still arms
/// for wrapper-spawned children, so an uncatchable wrapper death
/// behaves exactly as it did before this relay existed.
///
/// The set is the asynchronous, externally addressable kill
/// family. Deliberately excluded: SIGKILL/SIGSTOP (uncatchable),
/// SIGCONT (resume semantics are the kernel's, not a proxy's),
/// SIGCHLD (the engine's own reaping notices), SIGPIPE (the
/// wrapper's own output writes depend on its process-wide
/// disposition), SIGTRAP and the synchronous fault family
/// (SIGILL/SIGBUS/SIGFPE/SIGSEGV — the fatal set's territory),
/// SIGWINCH (terminal-size tracking).
#[cfg(unix)]
const WRAPPER_RELAY_SIGNALS: &[libc::c_int] = &[
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGABRT,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGALRM,
    libc::SIGTERM,
    libc::SIGXCPU,
    libc::SIGXFSZ,
    libc::SIGVTALRM,
    libc::SIGPROF,
    #[cfg(target_os = "linux")]
    libc::SIGSTKFLT,
];

/// The currently proxied pid (the last stage of the foreground
/// pipeline the wrapper is waiting on); 0 = no target published.
#[cfg(unix)]
static WRAPPER_RELAY_TARGET: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Saved pre-relay dispositions, indexed by signal number (every
/// relay signal is < 32). Written once by the install, before the
/// relay thread spawns; that spawn's happens-before edge covers
/// the relay thread's reads (the FATAL_PREV_HANDLER model).
#[cfg(unix)]
static WRAPPER_RELAY_PREV: [std::sync::atomic::AtomicU64; 32] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 32];
#[cfg(unix)]
static WRAPPER_RELAY_PREV_FLAGS: [std::sync::atomic::AtomicU64; 32] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 32];

/// Write end of the relay self-pipe; -1 until installed. The fd is
/// O_CLOEXEC, so no spawned child inherits it across exec.
#[cfg(unix)]
static WRAPPER_RELAY_PIPE_WR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// RAII publication of the relay target: the executor's pipeline
/// wait holds one of these for exactly the span in which the
/// wrapper supervises that pipeline. Inert outside wrapper-role
/// processes (and off unix).
pub struct WrapperRelayTargetGuard {
    #[cfg(unix)]
    pid: u32,
}

impl WrapperRelayTargetGuard {
    pub fn publish(pid: u32) -> Self {
        #[cfg(unix)]
        {
            if pid != 0 && background_wrapper_role_armed() {
                WRAPPER_RELAY_TARGET.store(pid, std::sync::atomic::Ordering::Release);
                return Self { pid };
            }
            Self { pid: 0 }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Self {}
        }
    }
}

impl Drop for WrapperRelayTargetGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.pid != 0 {
            // Clear only our own publication; a nested wait that
            // published a different target keeps its span.
            let _ = WRAPPER_RELAY_TARGET.compare_exchange(
                self.pid,
                0,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
}

/// The recording handler: async-signal-safe by construction — one
/// non-blocking write of the signal number to the relay pipe.
#[cfg(unix)]
extern "C" fn wrapper_relay_handler(sig: libc::c_int) {
    let fd = WRAPPER_RELAY_PIPE_WR.load(std::sync::atomic::Ordering::Relaxed);
    if fd >= 0 {
        let byte = sig as u8;
        unsafe {
            libc::write(fd, &byte as *const u8 as *const libc::c_void, 1);
        }
    }
}

#[cfg(unix)]
fn wrapper_relay_install() {
    use std::sync::atomic::Ordering;
    let mut fds: [libc::c_int; 2] = [0; 2];
    // libc::pipe2 is glibc/musl-only; Darwin exposes pipe + fcntl
    // instead (same pattern as fd/unix.rs create_capture_pipe).
    #[cfg(target_os = "linux")]
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return;
    }
    #[cfg(not(target_os = "linux"))]
    for fd in fds {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags >= 0 {
            unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
        }
    }
    let (read_end, write_end) = (fds[0], fds[1]);
    // rubash#3917: keep the relay pipe out of the user range (see
    // create_capture_pipe) — a persistent `exec N>file` now dup2's
    // onto its kernel number, and a relay pipe sitting on a low
    // number would be the descriptor it displaces.
    let read_end = crate::fd::move_to_high_fd(read_end);
    let write_end = crate::fd::move_to_high_fd(write_end);
    // The handler's write must never block on a full pipe.
    unsafe {
        let flags = libc::fcntl(write_end, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(write_end, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    WRAPPER_RELAY_PIPE_WR.store(write_end, Ordering::Release);
    for &sig in WRAPPER_RELAY_SIGNALS {
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        unsafe { libc::sigaction(sig, std::ptr::null_mut(), &mut current) };
        WRAPPER_RELAY_PREV[sig as usize].store(current.sa_sigaction as u64, Ordering::Relaxed);
        WRAPPER_RELAY_PREV_FLAGS[sig as usize]
            .store(current.sa_flags as i64 as u64, Ordering::Relaxed);
        let action = libc::sigaction {
            sa_sigaction: wrapper_relay_handler as *const () as usize,
            // SA_RESTART: the wrapper's own blocking waits (its
            // pipeline reap loop) must resume after a delivery the
            // relay thread will dispose of.
            sa_flags: libc::SA_RESTART,
            sa_mask: unsafe { std::mem::zeroed() },
            #[cfg(any(target_os = "linux", target_os = "android"))]
            sa_restorer: None,
        };
        unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
    }
    std::thread::spawn(move || wrapper_relay_loop(read_end));
}

/// Once-only install, called when the wrapper role arms. Never in
/// the lib's own unit-test process (cfg!(test)): arming a role
/// there is a bookkeeping exercise, and a test process must not
/// grow live handlers or a relay thread.
#[cfg(unix)]
fn wrapper_relay_install_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if cfg!(test) {
        return;
    }
    ONCE.call_once(wrapper_relay_install);
}

#[cfg(unix)]
fn wrapper_relay_loop(read_end: libc::c_int) {
    let mut buffer = [0u8; 128];
    loop {
        let count = unsafe {
            libc::read(
                read_end,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            )
        };
        if count <= 0 {
            // The write end lives in a static for the process
            // lifetime, so EOF is not expected; treat any failure
            // as transient and keep the relay alive.
            std::thread::sleep(std::time::Duration::from_millis(1));
            continue;
        }
        for &byte in &buffer[..count as usize] {
            wrapper_relay_deliver(byte as libc::c_int);
        }
    }
}

#[cfg(unix)]
fn wrapper_relay_deliver(sig: libc::c_int) {
    use std::sync::atomic::Ordering;
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    unsafe { libc::sigaction(sig, std::ptr::null_mut(), &mut current) };
    if current.sa_sigaction != wrapper_relay_handler as *const () as usize {
        // The disposition moved after this delivery was recorded.
        // The byte exists at all only because our handler ran, and
        // when the engine's registry trampoline owns the signal it
        // CHAINS to the previously installed handler — us — after
        // recording the delivery itself. Re-raising here would
        // re-enter the trampoline, record the same delivery again,
        // chain to us again, and loop without bound (measured: one
        // `kill -TERM $!` against a trapping wrapper body echoed
        // its trap action ~200 times). The live owner has already
        // recorded this delivery through its own path, so the
        // relay's copy is a duplicate: drop it. The residual race —
        // a delivery that reached our handler in the instant before
        // a flip and is dropped here instead of handed over — is
        // confined to the reconcile window itself, where GNU's own
        // answer depends on the same coin flip (the signal raced
        // the trap's installation).
        return;
    }
    let target = WRAPPER_RELAY_TARGET.load(Ordering::Acquire);
    if target != 0 {
        unsafe { libc::kill(target as libc::pid_t, sig) };
        return;
    }
    // No pipeline to proxy for: restore the disposition saved at
    // install time and re-raise, reproducing it exactly (the
    // fatal set's die-on-delivery model, generalized to whatever
    // was there — default, ignore, or a foreign handler, which
    // the re-raise then invokes with the real delivery info).
    let prev = WRAPPER_RELAY_PREV[sig as usize].load(Ordering::Relaxed) as usize;
    let prev_flags = WRAPPER_RELAY_PREV_FLAGS[sig as usize].load(Ordering::Relaxed) as i32;
    let action = libc::sigaction {
        sa_sigaction: prev,
        sa_flags: prev_flags,
        sa_mask: unsafe { std::mem::zeroed() },
        #[cfg(any(target_os = "linux", target_os = "android"))]
        sa_restorer: None,
    };
    unsafe {
        libc::sigaction(sig, &action, std::ptr::null_mut());
        libc::raise(sig);
    }
    // If the restored disposition swallowed the delivery (SIG_IGN
    // or a handler that returned), the relay handler is gone for
    // this signal and the relay has disengaged for it — the same
    // end state a plain disposition flip would have produced.
}

/// rubash#1035: environment key marking a `rubash -c` respawn as an
/// asynchronous (`&`) job child while job_control is off (the `monitor`
/// option is unset). GNU execute_cmd.c setup_async_signals forces
/// SIGINT/SIGQUIT to SIG_IGN in every such forked child; a respawned
/// process cannot inherit that disposition through spawn (the parent holds
/// no ignore itself), so the background spawn site stamps this key into
/// the child's environment block and startup consumes it (see
/// [`apply_async_child_entry_ignores`]) before the trap table seeds and
/// the kernel backend captures entry dispositions.
///
/// Forgery note (rewritten by rubash#2134): an earlier reading held
/// that a user `export` of this key "only buys the spawner kernel
/// ignore for that process subtree — the same disposition `trap '' INT`
/// achieves deliberately". That equivalence is false in the direction
/// that matters: `trap '' INT` is a deliberate act INSIDE the victim
/// shell, while this key is set by a DIFFERENT process — any script a
/// user runs can export it and every shell that script spawns is then
/// born ignoring SIGINT/SIGQUIT (the F3 census probe: a child launched
/// with the key ambient survived `kill -INT $own_pid` and exited 0
/// where the control died 130; with `-i` the same key reached the
/// interactive face). GNU reaches this disposition only through actual
/// parentage — jobs.c's async fork path (setup_async_signals,
/// jobs.c:4043) sets it in the forked child itself, and dispositions
/// are inherited across fork/exec, never asserted by data. The entry
/// point below therefore applies the ignore only when the process's
/// respawn provenance validates against its actual parent
/// (rubash#2136) and the invocation is not interactive-shaped; the key
/// is scrubbed either way. It still never arms traps or PDEATHSIG
/// (those read the trap table / the nonce handshake, rubash#866), and
/// the spawn site overwrites a forged ambient value on every
/// background respawn and removes it otherwise, so `set -m` (job
/// control on) children never see it.
#[cfg(unix)]
pub const ASYNC_CHILD_ENV_KEY: &str = "__RUBASH_ASYNC_CHILD";

/// Consume [`ASYNC_CHILD_ENV_KEY`] at process startup (rubash#1035): a
/// background-job respawn starts life with SIGINT/SIGQUIT ignored, exactly
/// as if forked by GNU's setup_async_signals path. Must run before the
/// executor seeds the trap table and before the kernel backend captures
/// entry dispositions, so the ignore surfaces both as `trap -- '' SIGINT`
/// (via the ORIG_IGN transport the spawn site adds for exec-like bodies)
/// and as the kernel entry disposition (ENTRY_IGNORED) for the rest.
/// Scrubs the key unconditionally afterwards so it never reaches
/// `export -p` or grandchildren environments (kernel inheritance carries
/// the disposition there, as fork does in GNU).
///
/// The async-child ignore is a setup_async_signals disposition, NOT a
/// hard ignore: GNU's forked async child lists `trap -- '' SIGINT` yet
/// still lets a later `trap` override it (trap.c's ASYNCSIG special-case
/// in set_signal — the golden trap-signal-matrix fires its INT handler).
/// The entry-disposition fold in seed_startup_traps must therefore tell
/// this ignore apart from a foreign parent's inherited ignore, which IS
/// hard (trap.c original_signals). This flag is the marker: it is set
/// here, before the trap table seeds, and read only by that fold.
#[cfg(unix)]
static ASYNC_CHILD_ENTRY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when this process started as an asynchronous (`&`) job child
/// (entry SIGINT/SIGQUIT ignored by the spawn site, not by inheritance).
#[cfg(unix)]
pub(crate) fn async_child_entry() -> bool {
    ASYNC_CHILD_ENTRY.load(std::sync::atomic::Ordering::Acquire)
}

/// True when this process's own command line is interactive-shaped:
/// an `-i`/`--interactive` flag before the command operand, or no `-c`
/// and no script operand at all (a stdin-driven session). Genuine
/// async respawns are always `-c` children (the command-file spill
/// variant keeps `-c` with an empty operand), so the scan stops —
/// not-interactive — at the first `-c`, `--`, or non-option operand.
#[cfg(unix)]
fn invocation_is_interactive_shaped() -> bool {
    for arg in std::env::args_os().skip(1) {
        let token = arg.to_string_lossy();
        if token == "-c" || token == "--" || token == "-" || !token.starts_with('-') {
            return false;
        }
        if token == "--interactive" {
            return true;
        }
        if !token.starts_with("--") && token[1..].contains('i') {
            return true;
        }
    }
    // No command operand anywhere: commands come from stdin, which is
    // the interactive session shape when attached to a terminal.
    true
}

#[cfg(unix)]
pub fn apply_async_child_entry_ignores() {
    let claimed = std::env::var(ASYNC_CHILD_ENV_KEY).as_deref() == Ok("1");
    std::env::remove_var(ASYNC_CHILD_ENV_KEY);
    if !claimed {
        return;
    }
    // rubash#2134: the marker is data in the environment, which any
    // parent can write — it classifies nothing by itself. Apply the
    // entry ignores only for a process whose async-child classification
    // is established by its actual parentage (the rubash#2136
    // provenance validation over the live process environment) and
    // whose invocation is a `-c` respawn shape, never an interactive
    // one: an interactive shell must be born with default dispositions
    // so Ctrl+C keeps its meaning (jobs.c has no path where an
    // interactive shell inherits an async child's ignores).
    if !crate::executor::respawn_provenance_validated_from_process_env()
        || invocation_is_interactive_shaped()
    {
        return;
    }
    ASYNC_CHILD_ENTRY.store(true, std::sync::atomic::Ordering::Release);
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
}

/// PR_SET_PDEATHSIG + race check against the EXPECTED parent pid. Runs
/// between fork and exec: syscalls and atomic loads only, no allocation.
#[cfg(target_os = "linux")]
fn arm_pdeathsig(expected_parent: u32) -> std::io::Result<()> {
    // PR_SET_PDEATHSIG: send SIGKILL to this process when its parent dies.
    const PR_SET_PDEATHSIG: i32 = 1;
    let ret = unsafe { libc::prctl(PR_SET_PDEATHSIG, libc::SIGKILL as u64, 0, 0, 0) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // rubash#867: race — the parent may have died between fork and prctl,
    // in which case we were reparented (to init, a subreaper, or the
    // namespace's PID 1). Self-kill only when the observed parent DIVERGES
    // from the expected one; never against the constant 1, which a healthy
    // child of a namespace-PID-1 parent matches before any race is possible.
    if unsafe { libc::getppid() } != expected_parent as libc::pid_t {
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
    }
    Ok(())
}

/// pre_exec hook for spawned child processes (unix arm of rubash#229):
/// restore GNU's child signal dispositions between fork and exec. A no-op
/// when the backend was never installed (the process then still holds its
/// entry dispositions everywhere).
///
/// rubash#737/#866: a process that consumed the background-wrapper handshake
/// additionally arms PDEATHSIG for the children IT spawns, decided from the
/// in-process role — the ambient environment is never consulted here (it is
/// user-forgeable), and the race check compares against the wrapper's own
/// pid instead of the constant 1 (rubash#867).
///
/// Behavior note (why this is an exact predicate swap): pre_exec runs in the
/// forked child BEFORE execve, so the old `env::var` probe here read the
/// FORKING PARENT's environ — the check was always a parent-state predicate,
/// never a property of the child's own environment block. Swapping "parent
/// env contains the marker" for "parent holds the in-process role" keeps
/// every spawn's arming decision byte-identical in clean environments (the
/// main shell never arms — so a wrapper itself survives its shell's exit, as
/// GNU's background jobs do — and a wrapper arms every child it spawns),
/// while a forged `export` arms nothing.
#[cfg(unix)]
pub fn child_pre_exec_reset() -> std::io::Result<()> {
    kernel_signals::pre_exec_child_dispositions()?;
    // rubash#737: if this process is a background-job wrapper, arrange for
    // this child to die when the wrapper dies. Without this, `kill $!` kills
    // the wrapper but leaves the actual work (grandchild) orphaned and
    // running.
    #[cfg(target_os = "linux")]
    if BACKGROUND_WRAPPER_ROLE.load(std::sync::atomic::Ordering::Acquire) {
        let expected =
            BACKGROUND_WRAPPER_EXPECTED_PARENT.load(std::sync::atomic::Ordering::Acquire);
        arm_pdeathsig(expected)?;
    }
    Ok(())
}

/// pre_exec hook for asynchronous (`&`) child spawns (unix arm of
/// rubash#1035): GNU execute_cmd.c setup_async_signals runs in every
/// forked async child while job_control == 0 and forces SIGINT/SIGQUIT to
/// SIG_IGN before exec — a background `sleep` survives `kill -INT $!`,
/// and an exec'd script inherits the ignore as SIG_HARD_IGNORE (trap.c
/// set_signal refuses to trap it, `trap -p` shows `trap -- '' SIGINT`).
/// Runs after the plain entry-disposition restore on purpose: a parent
/// `trap '...' INT` must not leak a catchable disposition into the async
/// child (GNU ignores unconditionally). The background spawn site selects
/// this hook instead of [`child_pre_exec_reset`]; the hook itself takes no
/// decision input beyond the call — pre_exec runs post-fork and must not
/// consult user-forgeable ambient state (rubash#866).
#[cfg(unix)]
pub fn child_pre_exec_reset_async() -> std::io::Result<()> {
    child_pre_exec_reset()?;
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
    Ok(())
}

/// rubash#2241: the child side of job-control process grouping. GNU
/// jobs.c make_child places every job child into the job's process
/// group between fork and exec (jobs.c:2390 `setpgid (mypid,
/// pipeline_pgrp)`); for a single-process job pipeline_pgrp is the
/// child's own pid (jobs.c:2446-2449). Without it the child stays in
/// the shell's inherited group and the group-addressed signal form
/// `kill -SIG -- -$!` — the only way to emulate a terminal ^C against
/// a whole job tree — fails with ESRCH. Spawn sites select this
/// fragment only when the monitor option is on; the decision is the
/// parent's, made at hook-selection time, never re-read in the child
/// (rubash#866). The parent repeats the call after the spawn returns
/// (see [`group_spawned_child`]).
#[cfg(unix)]
pub fn child_pre_exec_setpgid() -> std::io::Result<()> {
    if unsafe { libc::setpgid(0, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Composed pre_exec hook for a job-control async spawn: the plain
/// entry-disposition reset, then the grouping fragment above.
#[cfg(unix)]
pub fn child_pre_exec_reset_grouped() -> std::io::Result<()> {
    child_pre_exec_reset()?;
    child_pre_exec_setpgid()
}

/// Child side of joining an EXISTING job process group (rubash#2836
/// slice 4: pipeline stages after the first). GNU jobs.c make_child
/// places every job child into the job's process group between fork
/// and exec (jobs.c:2390 `setpgid (mypid, pipeline_pgrp)`); the first
/// stage leads the group (pgrp 0 == its own pid, the
/// `child_pre_exec_reset_grouped` shape), later stages join it. The
/// entry-disposition reset runs first for the same reason as the
/// single-child hook: the interactive shell ignores the stop signals
/// (slice 1's reconcile arm), and without the trap.c:1489 restore the
/// stage would inherit SIG_IGN for SIGTSTP and a terminal ^Z could
/// never stop it. Returns the closure so the spawn site passes it to
/// pre_exec directly (pre_exec composes: an earlier reset call is
/// kept, not replaced).
#[cfg(unix)]
pub fn child_pre_exec_reset_join_group(
    pgrp: i32,
) -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
    move || {
        child_pre_exec_reset()?;
        if unsafe { libc::setpgid(0, pgrp) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Parent half of the job-control grouping (rubash#2241): GNU repeats
/// the child's setpgid in the parent after fork (jobs.c:2455, "to
/// avoid possible race conditions", POSIX Rationale B.4.3.3) so the
/// group already exists when the spawn returns and `$!` names it. A
/// failure is an expected race — the child grouped itself first, or
/// already exited — and is ignored, as in GNU.
#[cfg(unix)]
pub fn group_spawned_child(pid: u32) {
    unsafe {
        libc::setpgid(pid as libc::pid_t, pid as libc::pid_t);
    }
}

/// Parent half for a pipeline stage after the first (rubash#2836 slice
/// 4): the same jobs.c:2455 repeat, into the job's group instead of the
/// child's own pid — the leader's group must already exist when the
/// later stage's spawn returns, so a ^Z arriving between spawns finds
/// the whole job in one group.
#[cfg(unix)]
pub fn group_spawned_child_into(pid: u32, pgrp: u32) {
    unsafe {
        libc::setpgid(pid as libc::pid_t, pgrp as libc::pid_t);
    }
}

/// rubash#866/#867: the wrapper handshake is unforgeable — the role arms
/// only when the argv nonce and the env nonce agree (both constructed by the
/// wrapper spawn site), both channels are consumed afterwards, and a forged
/// ambient value arms nothing and is scrubbed. These tests touch
/// process-global state (the role statics + the env key); a Drop guard
/// restores them so sibling tests in this binary are unaffected even on
/// assertion failure.
#[cfg(all(test, unix))]
mod background_wrapper_handshake_tests {
    use super::{
        background_wrapper_flag_consumer_present, background_wrapper_role_armed,
        consume_background_wrapper_handshake, new_background_wrapper_nonce,
        BACKGROUND_WRAPPER_ENV_KEY, BACKGROUND_WRAPPER_EXPECTED_PARENT, BACKGROUND_WRAPPER_FLAG,
        BACKGROUND_WRAPPER_ROLE,
    };
    use std::sync::atomic::Ordering;

    /// Serializes the env-key mutation across these tests (plain `cargo
    /// test` runs them as threads of one process; nextest isolates them).
    static HANDSHAKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct RoleGuard;
    impl RoleGuard {
        fn new() -> Self {
            Self
        }
    }
    impl Drop for RoleGuard {
        fn drop(&mut self) {
            BACKGROUND_WRAPPER_ROLE.store(false, Ordering::Release);
            BACKGROUND_WRAPPER_EXPECTED_PARENT.store(0, Ordering::Release);
            std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        }
    }

    fn args_with_flag(nonce: &str) -> Vec<String> {
        vec![
            "rubash".to_string(),
            format!("{BACKGROUND_WRAPPER_FLAG}{nonce}"),
            "-c".to_string(),
            "true".to_string(),
        ]
    }

    #[test]
    fn forged_env_value_arms_nothing_and_is_scrubbed() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        // The #866 attack: plain `export __RUBASH_BACKGROUND_WRAPPER=1` in
        // the environment of a rubash that never went through the wrapper
        // spawn path.
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, "1");
        let mut args = vec!["rubash".to_string(), "-c".to_string(), "true".to_string()];
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        // The forged value must not survive to be inherited by children.
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
        // argv untouched (no flag to strip).
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn argv_flag_without_matching_env_nonce_arms_nothing() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        let nonce = new_background_wrapper_nonce();
        let mut args = args_with_flag(&nonce);
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        // The internal flag is still consumed: the GNU option parser must
        // never see it.
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn nonce_mismatch_between_channels_arms_nothing() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, new_background_wrapper_nonce());
        let forged = new_background_wrapper_nonce();
        let mut args = args_with_flag(&forged);
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
    }

    #[test]
    fn agreeing_channels_arm_role_capture_expected_parent_and_consume_both() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        let nonce = new_background_wrapper_nonce();
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, &nonce);
        let mut args = args_with_flag(&nonce);
        consume_background_wrapper_handshake(&mut args);
        assert!(background_wrapper_role_armed());
        // rubash#867: the armed children compare getppid() against THIS
        // process's pid, never the constant 1.
        assert_eq!(
            BACKGROUND_WRAPPER_EXPECTED_PARENT.load(Ordering::Acquire),
            std::process::id()
        );
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn nonces_are_unique_and_well_formed() {
        let a = new_background_wrapper_nonce();
        let b = new_background_wrapper_nonce();
        assert_ne!(a, b, "successive spawn-time nonces must differ");
        assert_eq!(a.len(), 32);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// rubash#873: any consume call — even the plain-invocation scrub —
    /// marks the process as an argv-flag consumer, which is what licenses
    /// the wrapper spawn site to splice `--internal-bg-wrapper=` into a
    /// respawned child. The inverse (no consume call → no flag) cannot be
    /// asserted in this shared test process (sibling tests above already
    /// flipped the bit; only nextest's process-per-test lane could); it is
    /// pinned end-to-end by the niubash launcher suite, whose host CLI
    /// never consumed and must never RECEIVE the flag (rubash#873 CI).
    #[test]
    fn consume_marks_the_process_as_a_flag_consumer() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut args = vec!["rubash".to_string(), "-c".to_string(), "true".to_string()];
        consume_background_wrapper_handshake(&mut args);
        assert!(background_wrapper_flag_consumer_present());
    }
}

/// rubash#867: the armed child's race check compares getppid() against the
/// EXPECTED parent pid, never the constant 1. Driven here through the real
/// role statics and the real pre_exec hook; the spawned processes are this
/// test's own children. Mutates process-global state — the guard restores
/// it, and the linux-only lib lane runs under nextest (process-per-test) in
/// CI and --test-threads=1 locally, so no sibling test observes the window.
#[cfg(all(test, target_os = "linux"))]
mod pdeathsig_expected_parent_tests {
    use super::{
        child_pre_exec_reset, BACKGROUND_WRAPPER_EXPECTED_PARENT, BACKGROUND_WRAPPER_ROLE,
    };
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::sync::atomic::Ordering;

    struct RoleGuard;
    impl Drop for RoleGuard {
        fn drop(&mut self) {
            BACKGROUND_WRAPPER_ROLE.store(false, Ordering::Release);
            BACKGROUND_WRAPPER_EXPECTED_PARENT.store(0, Ordering::Release);
        }
    }

    fn spawn_armed(expected_parent: u32) -> std::process::ExitStatus {
        BACKGROUND_WRAPPER_ROLE.store(true, Ordering::Release);
        BACKGROUND_WRAPPER_EXPECTED_PARENT.store(expected_parent, Ordering::Release);
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo ARMED-ALIVE"]);
        unsafe {
            cmd.pre_exec(|| child_pre_exec_reset());
        }
        cmd.status().expect("spawn armed child")
    }

    #[test]
    fn healthy_child_of_expected_parent_survives_arming() {
        let _guard = RoleGuard;
        let status = spawn_armed(std::process::id());
        // A healthy child of a namespace-PID-1 parent lands here too: this
        // assert is exactly the constant-1 comparison's false positive,
        // which must never fire.
        assert!(
            status.success(),
            "healthy armed child was killed: {status:?}"
        );
    }

    #[test]
    fn child_whose_parent_diverges_from_expected_self_kills() {
        let _guard = RoleGuard;
        // Deliberately wrong expectation: the observed ppid (this test's
        // pid) diverges, so the race check must fire — proving the check
        // reads the expected pid rather than `== 1` (on this host the old
        // check's `getppid() == 1` was false for the same child).
        let wrong = std::process::id().wrapping_add(0x4000_0000);
        let status = spawn_armed(wrong);
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL), "status: {status:?}");
    }
}
