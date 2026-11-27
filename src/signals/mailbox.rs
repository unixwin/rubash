//! Signal mailbox registration, queueing, and pending-signal draining.

#[cfg(not(unix))]
use crate::signals::delivery::process_exists;
#[cfg(not(unix))]
use crate::signals::delivery::signal_hint;
#[cfg(unix)]
use crate::signals::dispositions::kernel_signals;
use std::io;

/// Install this process's signal-receiving backend.
///
/// Windows: a per-process `{pid}.alive` marker in the shared
/// `%LOCALAPPDATA%\rubash-signals` directory; other rubash processes deliver
/// cross-process signals as atomic `.part`-rename entries beside it
/// (deliver_rubash_signal).
///
/// Unix: real kernel signal delivery -- GNU sig.c:102 initialize_signals
/// installs handlers for the terminating-signal set (the table at
/// sig.c:133). signal_hook owns the async-signal-safe self-pipe; arrivals
/// are drained by take_pending_signals and dispatched by the shared
/// trap_exec::run_pending_signal_traps boundary poll. No marker file is
/// written, so deliver_rubash_signal finds no mailbox and routes rubash
/// targets through the real kill(2) instead (signal_process).
#[cfg(unix)]
pub fn register_signal_mailbox(_pid: u32) -> io::Result<()> {
    kernel_signals::install()
}

#[cfg(not(unix))]
pub fn register_signal_mailbox(pid: u32) -> io::Result<()> {
    let dir = signal_mailbox_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(signal_marker_path(pid), std::process::id().to_string())
}

/// Unix keeps the kernel dispositions for the whole process lifetime:
/// nothing to tear down, and caught handlers reset to default in respawned
/// children on exec anyway (bash subshell semantics for trapped signals).
#[cfg(unix)]
pub fn unregister_signal_mailbox(_pid: u32) {}

#[cfg(not(unix))]
pub fn unregister_signal_mailbox(pid: u32) {
    let _ = std::fs::remove_file(signal_marker_path(pid));
    let _ = std::fs::remove_file(signal_queue_path(pid));
    // startup21: enumerate with the pattern-limited FindFirstFileW scan the
    // poller already uses (pending_signal_entries) instead of read_dir over
    // the whole shared directory. The directory accumulates one marker or
    // queue file per process that ever ran (213 stale markers at lane
    // measurement time) and the full scan cost ~0.5ms of every shell's exit
    // path; the pattern query removes exactly the same set of files.
    let prefix = format!("{pid}.q.");
    if let Ok(entries) = pending_signal_entries(&signal_mailbox_dir(), &prefix) {
        for entry in entries {
            let _ = std::fs::remove_file(entry);
        }
    }
}

/// Startup21 (non-unix): register the mailbox OFF the Executor::new
/// critical path. The synchronous registration (create_dir_all + marker
/// write) costs ~0.5ms of Windows filesystem time per spawn, and the
/// marker is only ever consulted by OTHER processes' kill builtin
/// (deliver_rubash_signal) — this process's own execution never reads it.
/// A sub-millisecond delay before the marker appears only widens the
/// pre-mailbox window in which a cross-process kill falls back to the
/// signal_process route, which is the behavior shells had before the
/// mailbox existed. Ordering stays eager in the GNU sense: the
/// registration is initiated at Executor::new (cf. trap.c:102
/// initialize_signals at shell startup), and
/// join_signal_mailbox_registration() lets Drop wait for the thread before
/// unregistering, so a normally exiting shell never leaks its marker.
#[cfg(not(unix))]
static MAILBOX_REGISTRATION: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(None);

#[cfg(not(unix))]
pub fn register_signal_mailbox_async(pid: u32) {
    let mut slot = MAILBOX_REGISTRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A previous top-level executor in this process (unit tests) may still
    // hold the slot; joining first keeps registrations strictly ordered so
    // a late write can never resurrect a marker an executor just removed.
    if let Some(handle) = slot.take() {
        let _ = handle.join();
    }
    match std::thread::Builder::new()
        .name("rubash-sig-mailbox".to_string())
        .spawn(move || {
            let _ = register_signal_mailbox(pid);
            prune_stale_markers(pid);
        }) {
        Ok(handle) => *slot = Some(handle),
        // Thread spawn failure is not a shell error: fall back to the
        // synchronous registration the executor previously paid.
        Err(_) => {
            let _ = register_signal_mailbox(pid);
        }
    }
}

/// startup21 (non-unix): opportunistic mailbox-directory hygiene, run on
/// the background registration thread (never the shell's critical path —
/// process-exit executors no longer remove their own marker, so something
/// must keep the shared directory from growing without bound). Removes the
/// mailbox of every DEAD pid found among the first 128 marker entries:
/// exactly the cleanup deliver()'s dead-pid path performs on demand, done
/// here proactively. A live pid's marker is left untouched; removals are
/// idempotent, so racing an owner's own unregister is harmless.
#[cfg(not(unix))]
fn prune_stale_markers(own_pid: u32) {
    const PRUNE_LIMIT: usize = 128;
    let dir = signal_mailbox_dir();
    let Ok(entries) = pending_signal_entries(&dir, "") else {
        return;
    };
    let mut pruned = 0usize;
    for path in entries {
        if pruned >= PRUNE_LIMIT {
            break;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let Some(pid) = name
            .strip_suffix(".alive")
            .and_then(|stem| stem.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid || process_exists(pid) {
            continue;
        }
        unregister_signal_mailbox(pid);
        pruned += 1;
    }
}

/// Wait for an in-flight async mailbox registration to finish. Called by
/// the top-level Executor's Drop BEFORE unregister_signal_mailbox, so the
/// unregister cannot race a registration that has not written its marker
/// yet (that ordering would leak the marker file).
#[cfg(not(unix))]
pub fn join_signal_mailbox_registration() {
    if let Some(handle) = MAILBOX_REGISTRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        let _ = handle.join();
    }
}

#[cfg(unix)]
pub fn join_signal_mailbox_registration() {}

#[cfg(unix)]
pub fn register_signal_mailbox_async(_pid: u32) {
    // Unix keeps kernel dispositions (see register_signal_mailbox): there
    // is no marker file, so the async Windows path has no work — a no-op
    // keeps the init.rs call site platform-uniform.
    // rubash#1027: startup does only the fd-free capture/policy step;
    // the signal_hook backend (self-pipe socketpair) installs lazily on
    // the first trapped signal (kernel_signals::reconcile).
    kernel_signals::capture_entry_and_policy();
}

#[cfg(not(unix))]
fn parse_signal_lines(content: &str) -> Vec<i32> {
    content
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .collect()
}

/// Signals delivered by this very process (`kill` to `$$`): drained by the
/// executor's signal poll without any filesystem traffic.
static SELF_SIGNALS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Lock-free "at least one signal may be waiting" flag (rubash#437). The
/// per-command boundary poll (`run_pending_signal_traps`) runs once per
/// loop iteration; before this flag, `take_all_signals` took the
/// SELF_SIGNALS mutex AND the kernel backend mutex (plus a pending()
/// collect) on every poll even when nothing was pending — profiled as the
/// single hottest self-time frame in a `while` arithmetic loop. Producers
/// (queue_self_signal, and the kernel handler via kernel_signals::note
/// hook) set it with Release; the drain reads it with Acquire and clears
/// it only after both queues are empty, so a delivery that races the clear
/// either lands before the clear (drained next poll) or re-sets the flag.
/// All platforms: the Windows take_pending_signals branch uses the same
/// gate for the same in-process queue (#682 lock-free boundary fast path).
static PENDING_ANY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Counts polls so the mailbox-file scan (for signals sent by OTHER
/// processes) runs at a reduced interval instead of twice per command.
/// Unix has no file mailbox: kernel deliveries drain through
/// take_all_signals and this throttle does not exist.
#[cfg(not(unix))]
static FILE_POLL_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Unix drain: in-process queue (a `kill` to `$$` via queue_self_signal)
/// plus real kernel deliveries from the signal_hook backend.
#[cfg(unix)]
fn take_all_signals() -> Vec<i32> {
    // Lock-free fast path (rubash#437). Two independent producers feed the
    // two queues, and either one being provably empty lets us skip its
    // mutex entirely:
    //   * SELF_SIGNALS  -> guarded by PENDING_ANY (set in queue_self_signal).
    //   * kernel queue  -> a signal_hook handler is only installed for a
    //     signal that is actually trapped, so when HANDLER_SET == 0 no
    //     kernel delivery can be pending and the backend mutex + pending()
    //     collect can be skipped. This is the steady state of a script
    //     with no traps (the common loop case).
    let self_pending = PENDING_ANY.load(std::sync::atomic::Ordering::Acquire);
    let kernel_pending = kernel_signals::handler_any_installed();
    // rubash#1475: the fatal pair's handler is always installed, so its
    // queue is a third independent producer for this drain.
    let fatal_pending = kernel_signals::fatal_pending_any();
    if !self_pending && !kernel_pending && !fatal_pending {
        return Vec::new();
    }
    let mut signals = if self_pending {
        take_self_signals()
    } else {
        Vec::new()
    };
    if kernel_pending {
        signals.extend(kernel_signals::drain());
    }
    if fatal_pending {
        signals.extend(kernel_signals::drain_fatal());
    }
    // Clear only when both sources are provably empty again. A delivery
    // that raced this drain either set PENDING_ANY or installed a handler
    // (both Release stores), so re-reading here and clearing with Release
    // cannot lose a wakeup: the racing store is either visible now (flag
    // stays / self queue non-empty) or lands strictly after this clear.
    if take_self_signals().is_empty() && !kernel_signals::handler_any_installed() {
        PENDING_ANY.store(false, std::sync::atomic::Ordering::Release);
    }
    signals
}

pub(crate) fn queue_self_signal(signal: i32) {
    SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(signal);
    PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
    #[cfg(unix)]
    {
        kernel_signals::note_self_delivery(signal);
    }
}

fn take_self_signals() -> Vec<i32> {
    let mut queue = SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

/// rubash#1654: signals a flat subshell layer addressed to the shell's
/// own pid (`kill -s TERM $$` inside `( ... )`). In GNU the subshell
/// is a separate process and `$$` names the PARENT shell, whose own
/// dispositions judge the delivery; here the layers share one process
/// and one kernel disposition table, which the subshell body's
/// `trap ''` / `trap '...'` may just have reprogrammed — a raw
/// raise(2) is swallowed by the child's SIG_IGN, or queued under the
/// child's handler and then dropped as stale once the parent boundary
/// reconciles back. So while the ancestor route is armed (see below),
/// the kill builtin queues such deliveries here instead, and the
/// first depth-0 boundary poll dispatches them against the restored
/// top-level table (trap_exec::run_pending_signal_traps): trapped ->
/// run the action, ignored -> drop, untrapped -> the 128+sig death —
/// exactly the verdicts the addressed process would have produced.
/// Each entry also carries the sender exit status (rubash#2352, see
/// ANCESTOR_SENDER_STATUS): the last status completed before the
/// delivering kill builtin started, which is the addressed shell's
/// last_command_exit_value at the moment GNU's wait is preempted.
#[cfg(unix)]
static ANCESTOR_SIGNALS: std::sync::Mutex<Vec<(i32, i32)>> = std::sync::Mutex::new(Vec::new());

#[cfg(unix)]
pub(crate) fn queue_ancestor_signal(signal: i32) {
    ANCESTOR_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((signal, ancestor_sender_status()));
    #[cfg(unix)]
    {
        PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
        // An in-process routed delivery is by construction not a
        // kernel/group delivery (same record queue_self_signal makes).
        kernel_signals::note_self_delivery(signal);
    }
}

/// The queued ancestor deliveries: (signal, sender exit status) — see
/// ANCESTOR_SIGNALS for the status half's contract (rubash#2352).
#[cfg(unix)]
pub(crate) fn take_ancestor_signals() -> Vec<(i32, i32)> {
    let mut queue = ANCESTOR_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

#[cfg(unix)]
thread_local! {
    /// The exit status the queueing layer presents for the in-flight
    /// kill builtin (rubash#2352): Executor::execute_kill snapshots
    /// its exit_code — the last command completed BEFORE the builtin
    /// started — for the span of the dispatch, and queue_ancestor_
    /// signal stamps it onto the entry. The boundary dispatch needs
    /// it because GNU preempts the carrier command: the addressed
    /// shell is still WAITING for the delivering layer's construct,
    /// whose own status is therefore never committed to the parent's
    /// last_command_exit_value before the termsig verdict reads it
    /// (trap.c:1036). A plain Cell (no guard object) suffices next to
    /// ANCESTOR_ROUTE: execute_kill saves and restores it around the
    /// same span that arms the routes, so nested kill invocations
    /// cannot leak a stale snapshot.
    static ANCESTOR_SENDER_STATUS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

#[cfg(unix)]
fn ancestor_sender_status() -> i32 {
    ANCESTOR_SENDER_STATUS.with(|cell| cell.get())
}

/// RAII arm for the sender-status snapshot (rubash#2352), the same
/// save/replace discipline as AncestorRouteGuard: nested kill
/// invocations restore the outer dispatch's snapshot on drop.
#[cfg(unix)]
pub(crate) struct AncestorSenderStatusGuard {
    previous: i32,
}

#[cfg(unix)]
pub(crate) fn arm_ancestor_sender_status(status: i32) -> AncestorSenderStatusGuard {
    let previous = ANCESTOR_SENDER_STATUS.with(|cell| cell.replace(status));
    AncestorSenderStatusGuard { previous }
}

#[cfg(unix)]
impl Drop for AncestorSenderStatusGuard {
    fn drop(&mut self) {
        ANCESTOR_SENDER_STATUS.with(|cell| cell.set(self.previous));
    }
}

#[cfg(unix)]
thread_local! {
    /// Armed by Executor::execute_kill while the current executor is
    /// a flat subshell layer sharing this process (rubash#1654): kill
    /// deliveries addressed to the shell's own pid then route to
    /// ANCESTOR_SIGNALS instead of raise(2). A Session is confined to
    /// one thread and the builtin's delivery is synchronous, so a
    /// thread-local flag is exact; the guard restores the prior value
    /// so nested kill invocations cannot leak the routing.
    static ANCESTOR_ROUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(unix)]
pub(crate) fn ancestor_route_active() -> bool {
    ANCESTOR_ROUTE.with(|flag| flag.get())
}

/// RAII arm for the ancestor route (rubash#1654).
#[cfg(unix)]
pub(crate) struct AncestorRouteGuard {
    previous: bool,
}

#[cfg(unix)]
pub(crate) fn arm_ancestor_route() -> AncestorRouteGuard {
    let previous = ANCESTOR_ROUTE.with(|flag| flag.replace(true));
    AncestorRouteGuard { previous }
}

#[cfg(unix)]
impl Drop for AncestorRouteGuard {
    fn drop(&mut self) {
        ANCESTOR_ROUTE.with(|flag| flag.set(self.previous));
    }
}

thread_local! {
    /// Armed by Executor::execute_direct_shell_script for the span of
    /// an in-process child shell's run (rubash#2236): a kill delivery
    /// addressed to the hosting process's own pid from inside the
    /// child is judged by the CHILD, not delivered to the host. GNU
    /// runs the child as its own process, where the same kill(2) can
    /// only terminate the child; the emulation runs the child on this
    /// thread, so an unrouted raise(2) killed the outer shell with the
    /// child's signal. Routed deliveries join the ancestor lane
    /// (ANCESTOR_SIGNALS), whose boundary dispatch
    /// (run_pending_signal_traps) adjudicates against the
    /// then-current — child — trap table without a kernel-disposition
    /// judge: a trapped signal runs the child's action, an untrapped
    /// one ends the child with 128+signal and the parent survives.
    /// Signal 0 probes and
    /// non-adjudicable signals (SIGKILL, stop class) keep the raw
    /// path, the same boundary the ancestor route (#1654) draws.
    static DIRECT_CHILD_ROUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The signal of the most recent untrapped-signal adjudication
/// (rubash#2236). run_pending_signal_traps and
/// observe_signals_while_blocked record it when they end a layer with
/// 128+signal, so an in-process child's exit boundary can tell "the
/// child died by signal N" apart from a plain exit and queue the
/// parent's job-death notice, the way a wait(2) status would report
/// it. Cleared by execute_direct_shell_script on entry and taken on
/// exit, so a record can never cross a child boundary in either
/// direction.
static ADJUDICATED_SIGNAL_DEATH: std::sync::Mutex<Option<i32>> = std::sync::Mutex::new(None);

#[cfg(unix)]
pub(crate) fn direct_child_route_active() -> bool {
    DIRECT_CHILD_ROUTE.with(|flag| flag.get())
}

/// RAII arm for the direct-child route (rubash#2236).
pub(crate) struct DirectChildRouteGuard {
    previous: bool,
}

pub(crate) fn arm_direct_child_route() -> DirectChildRouteGuard {
    let previous = DIRECT_CHILD_ROUTE.with(|flag| flag.replace(true));
    DirectChildRouteGuard { previous }
}

impl Drop for DirectChildRouteGuard {
    fn drop(&mut self) {
        DIRECT_CHILD_ROUTE.with(|flag| flag.set(self.previous));
    }
}

pub(crate) fn note_adjudicated_signal_death(signal: i32) {
    *ADJUDICATED_SIGNAL_DEATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(signal);
}

pub(crate) fn take_adjudicated_signal_death() -> Option<i32> {
    ADJUDICATED_SIGNAL_DEATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

pub fn take_pending_signals(pid: u32) -> io::Result<Vec<i32>> {
    // Unix: real kernel deliveries drain through the in-process queue; no
    // filesystem traffic, and no cross-process file mailbox exists.
    #[cfg(unix)]
    {
        let _ = pid;
        return Ok(take_all_signals());
    }
    // In-process deliveries are drained unconditionally (a Mutex op, no
    // filesystem traffic). Signals from OTHER processes arrive via mailbox
    // files; with Windows Defender-style real-time scanning each
    // intercepted file operation costs ~0.25-0.5ms, so the file poll runs
    // every 64th poll (~every 32 commands) instead of twice per command.
    // Delivery latency for external kills stays in the millisecond range.
    // NOTE: a directory-mtime fast path was tried here and removed: NTFS
    // does not update a directory's LastWriteTime synchronously on entry
    // create/delete (verified 2026-09-12: mtime unchanged 0.5s after a
    // create), so it silently swallowed real deliveries.
    #[cfg(not(unix))]
    {
        // rubash#437's lock-free gate, Windows twin: PENDING_ANY marks a
        // non-empty in-process queue (queue_self_signal and
        // requeue_pending_signals raise it on every push, all platforms), so
        // a clear flag skips the SELF_SIGNALS mutex entirely. External
        // deliveries never enter this queue — they arrive through the file
        // mailbox polled below — so the gate cannot hide one. The clear
        // mirrors the unix drain (take_all_signals): re-take under the lock
        // before the Release store.
        let mut signals = if PENDING_ANY.load(std::sync::atomic::Ordering::Acquire) {
            take_self_signals()
        } else {
            Vec::new()
        };
        let tick = FILE_POLL_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // catch_flag shape (see mod signal_hint): the kernel event fires the
        // instant a same-version cross-process delivery lands, so the
        // filesystem query runs on the event; the periodic fallback poll
        // covers senders that cannot set the event (tight cadence when the
        // event infrastructure is unavailable, stretched otherwise).
        let fallback_period: u64 = if signal_hint::available() { 1024 } else { 64 };
        // unixwin/rubash#682: WaitForSingleObject(,0) is a real syscall
        // (~0.3-0.5us) charged to EVERY command boundary — a measurable slice
        // of the shape-independent per-command floor. The auto-reset event
        // latches a sender's pulse until a wait consumes it, so sampling it
        // every HINT_POLL_PERIOD-th poll loses no delivery: a pulse landing
        // between samples keeps the event set until the next sampled
        // boundary drains the mailbox. Worst-case added latency is
        // HINT_POLL_PERIOD-1 boundaries (~25us at the ~3.5us floor), far
        // inside the 1024-tick filesystem cadence eventless senders already
        // tolerate.
        const HINT_POLL_PERIOD: u64 = 8;
        if signals.is_empty()
            && (tick % fallback_period == 0
                || (tick % HINT_POLL_PERIOD == 0 && signal_hint::signaled()))
        {
            signals = take_file_signals(pid)?;
        }
        // Clear only when the flag says a producer ran (the gate load, not a
        // lock) and a re-take under the lock finds the queue drained — the
        // same discipline as the unix take_all_signals clear. A delivery
        // that raced the first take is delivered here, never dropped; a
        // delivery that raced the file poll keeps the flag up for the next
        // boundary.
        if signals.is_empty() && PENDING_ANY.load(std::sync::atomic::Ordering::Acquire) {
            let taken = take_self_signals();
            if taken.is_empty() {
                PENDING_ANY.store(false, std::sync::atomic::Ordering::Release);
            } else {
                signals = taken;
            }
        }
        Ok(signals)
    }
}

/// Unthrottled variant of take_pending_signals: always scans the
/// per-delivery file mailbox. Used where delivery latency is observable —
/// `wait`'s interruptible poll (wait.def:170-206) and the in-process
/// ${THIS_SH} child boundary that must drain the emulated child's queue.
pub fn take_pending_signals_now(pid: u32) -> io::Result<Vec<i32>> {
    #[cfg(unix)]
    {
        let _ = pid;
        return Ok(take_all_signals());
    }
    #[cfg(not(unix))]
    {
        let mut signals = take_self_signals();
        signals.extend(take_file_signals(pid)?);
        Ok(signals)
    }
}

/// Push drained signals back to the front of this process's pending queue
/// (preserving order). `wait` re-queues what it peeked so the command
/// boundary's run_pending_signal_traps still dispatches the trap actions
/// (wait.def:174 — "the trap associated with that signal shall be taken"
/// AFTER wait returns >128), and the in-process ${THIS_SH} child boundary
/// restores signals that were pending for the parent before it started.
pub fn requeue_pending_signals(signals: Vec<i32>) {
    if signals.is_empty() {
        return;
    }
    let mut queue = SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    queue.splice(..0, signals);
    // This is a SECOND producer of SELF_SIGNALS (the other is
    // queue_self_signal), so it must set the same lock-free wakeup flag:
    // the boundary poll's fast path (take_all_signals, and the Windows twin
    // in take_pending_signals) skips the self queue when PENDING_ANY is
    // clear, and a requeue that did not raise it would strand the re-queued
    // signal forever. `wait`/observe_signals_while_blocked
    // peek a delivery, put it back here, and rely on the NEXT command
    // boundary to dispatch the trap (wait.def:174).
    PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(not(unix))]
fn take_file_signals(pid: u32) -> io::Result<Vec<i32>> {
    let dir = signal_mailbox_dir();
    let mut signals = Vec::new();
    // Legacy single-file queue: take it atomically via rename so a signal
    // appended after the read is not dropped by the remove below. Check
    // existence first: an unconditional rename is a failing syscall on
    // every poll, and this poll runs on the executor's command boundaries.
    let legacy_queue = signal_queue_path(pid);
    if legacy_queue.try_exists()? {
        let legacy_taken = dir.join(format!("{pid}.queue.taking"));
        if std::fs::rename(&legacy_queue, &legacy_taken).is_ok() {
            if let Ok(content) = std::fs::read_to_string(&legacy_taken) {
                signals.extend(parse_signal_lines(&content));
            }
            let _ = std::fs::remove_file(&legacy_taken);
        }
    }
    // Per-delivery entries: senders write a unique .part file and rename it
    // into place, so the reader only ever sees fully written entries and a
    // concurrent delivery can never be lost. The old read-then-remove
    // window dropped a signal whenever a delivery landed between the two
    // steps, which made the trap9 kill-to-self from a busy loop
    // unreliable.
    let prefix = format!("{pid}.q.");
    let entries = pending_signal_entries(&dir, &prefix)?;
    for path in entries {
        if let Ok(content) = std::fs::read_to_string(&path) {
            signals.extend(parse_signal_lines(&content));
        }
        let _ = std::fs::remove_file(&path);
    }
    Ok(signals)
}

/// Enumerate pending-signal queue entries whose file name starts with
/// `prefix`. On Windows this uses FindFirstFileW with a `{prefix}*` pattern
/// so a real scan touches only matching names: the shared mailbox
/// directory accumulates one marker or queue file per process that ever
/// ran, and a full read_dir scan costs ~1ms per poll on busy systems
/// (measured 2026-09-12: 592 files -> 1.05ms per call). The pattern-limited
/// query halves that; the caller gates the scan itself to every 64th poll
/// so the per-command amortized cost is negligible.
#[cfg(not(unix))]
fn pending_signal_entries(
    dir: &std::path::Path,
    prefix: &str,
) -> io::Result<Vec<std::path::PathBuf>> {
    scan_pending_signal_entries(dir, prefix)
}

#[cfg(not(unix))]
fn scan_pending_signal_entries(
    dir: &std::path::Path,
    prefix: &str,
) -> io::Result<Vec<std::path::PathBuf>> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW,
        };

        // The pattern is the directory plus a separator and "{prefix}*";
        // only fully written entries match (senders rename .part files into
        // place), and the .part exclusion below keeps that guarantee.
        let mut pattern: Vec<u16> = dir
            .join(format!("{prefix}*"))
            .as_os_str()
            .encode_wide()
            .collect();
        pattern.push(0);

        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        let handle = unsafe { FindFirstFileW(pattern.as_ptr(), &mut data) };
        if handle == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            if error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND {
                // No matching entries (or the mailbox dir does not exist
                // yet): nothing pending, same as the NotFound branch of the
                // former read_dir-based scan.
                return Ok(Vec::new());
            }
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        let mut names = Vec::new();
        loop {
            let name = utf16_until_nul(&data.cFileName);
            if name.starts_with(prefix) && !name.ends_with(".part") {
                names.push(dir.join(&name));
            }
            data = unsafe { std::mem::zeroed() };
            if unsafe { FindNextFileW(handle, &mut data) } == 0 {
                unsafe { FindClose(handle) };
                break;
            }
        }
        names.sort();
        Ok(names)
    }
    #[cfg(not(windows))]
    {
        let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with(prefix) && !name.ends_with(".part"))
                        .unwrap_or(false)
                })
                .collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        entries.sort();
        Ok(entries)
    }
}

#[cfg(windows)]
fn utf16_until_nul(chars: &[u16]) -> String {
    let len = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// The shared signal-mailbox directory.
///
/// Windows (rubash#357): `%LOCALAPPDATA%\rubash-signals`, resolved ONCE per
/// process and cached. TMP/TEMP are deliberately NOT consulted:
///
/// - An MSYS parent forwards raw POSIX values (`TEMP=/tmp`), which Win32 file
///   APIs resolve against each process's CURRENT DRIVE — a process on `D:`
///   gets `D:\tmp\rubash-signals`, one on `C:` gets `C:\tmp\...`, while a
///   parent with the native `%TEMP%` reads `C:\Users\...\Temp\rubash-signals`.
///   Two such processes split into different boxes, the killer misses the
///   victim's `{pid}.alive` marker, and the kill falls back to
///   TerminateProcess — the trap never runs (143/NOTRAPPED, rubash#357).
/// - The process environment is also MUTABLE mid-run (a rubash child's env
///   restore can re-apply the raw spawn value), so re-reading temp_dir() per
///   call let the same process flip boxes between registration and polling
///   (observed: marker written to the native box, later polls scanning
///   `D:\tmp\rubash-signals` forever). The OnceLock pins one box for the
///   process lifetime.
///
/// `%LOCALAPPDATA%` is user-scoped, always an absolute Windows path, and
/// never MSYS-mangled, so every rubash process of the same user converges on
/// one box regardless of TMP/TEMP form or cwd drive. Fallbacks mirror the
/// user-profile chain (USERPROFILE, then the OS temp as a last resort) and
/// keep the same first-use caching.
///
/// Unix: there is no file mailbox (kernel dispositions own delivery — see
/// `register_signal_mailbox`); the plain temp join stays for the dead-marker
/// helpers that never execute there.
#[cfg(not(unix))]
pub(in crate::signals) fn signal_mailbox_dir() -> std::path::PathBuf {
    static MAILBOX_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    MAILBOX_DIR
        .get_or_init(|| {
            for key in ["LOCALAPPDATA", "USERPROFILE"] {
                if let Ok(value) = std::env::var(key) {
                    let path = std::path::PathBuf::from(&value);
                    if path.is_absolute() {
                        return path.join("rubash-signals");
                    }
                }
            }
            std::env::temp_dir().join("rubash-signals")
        })
        .clone()
}

#[cfg(unix)]
pub(in crate::signals) fn signal_mailbox_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("rubash-signals")
}

pub(in crate::signals) fn signal_marker_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.alive"))
}

#[cfg(not(unix))]
fn signal_queue_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.queue"))
}
