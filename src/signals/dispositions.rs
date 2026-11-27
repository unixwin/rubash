//! Kernel signal-disposition capture, reconciliation, and fatal-signal routing.

/// Signal numbers (1..=64) whose disposition was SIG_IGN when this
/// process started — GNU trap.c's original_signals snapshot, feeding
/// the trap table's hard-ignore seed (trap.rs seed_startup_traps,
/// rubash#1124). Probed once per process; empty on non-unix, where the
/// trap table is transported through the environment instead.
#[cfg(unix)]
pub(crate) fn entry_ignored_signal_numbers() -> Vec<i32> {
    let mask = kernel_signals::entry_ignored_mask_all();
    (1..=64)
        .filter(|sig| mask & (1u64 << (sig - 1)) != 0)
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn entry_ignored_signal_numbers() -> Vec<i32> {
    Vec::new()
}

/// rubash#1882: the fd-free startup policy capture WITHOUT the mailbox
/// registration, for executor children that must not own a TERM mailbox
/// (coproc children — a blocked coprocess reader cannot consume a queued
/// TERM) but must still run under the shell's signal policy: entry
/// dispositions captured, SIGPIPE reset to SIG_DFL (#1333), SIGQUIT
/// policy applied. Off Unix there is no kernel-disposition capture to
/// run (the mailbox marker is the only registration artifact), so this
/// is a no-op there.
pub fn capture_entry_policy_without_mailbox() {
    #[cfg(unix)]
    kernel_signals::capture_entry_and_policy();
}

/// rubash#2084: root executors take a share of the process signal
/// policy at birth (see kernel_signals::acquire_session_policy); the
/// paired release restores the pre-session dispositions when the
/// last root executor of the era drops. Non-unix processes have no
/// kernel dispositions to scope: no-ops keep the executor call sites
/// platform-uniform.
pub fn session_dispositions_acquire() {
    #[cfg(unix)]
    kernel_signals::acquire_session_policy();
}

/// Pair of [`session_dispositions_acquire`].
pub fn session_dispositions_release() {
    #[cfg(unix)]
    kernel_signals::release_session_policy();
}

/// Real kernel signal backend (the unix half of the signal-delivery seam).
///
/// GNU model (rubash#226): a NON-INTERACTIVE script shell installs no handler
/// for a terminating signal until a `trap` names it. sig.c:102
/// initialize_signals -> sig.c:313 initialize_shell_signals: every shell sets
/// `SIGQUIT -> SIG_IGN` (sig.c:333), but initialize_terminating_signals --
/// which installs termsig_sighandler for the whole terminating set
/// (sig.c:127-260) -- runs only when `interactive` (sig.c:315-316) or when an
/// EXIT trap / `trap` display forces it (sig.c:226-229). Everything else
/// stays SIG_DFL, so the KERNEL kills a blocked script shell on delivery --
/// there is no unkillable state. The `trap` builtin then installs one
/// handler per named signal (trap.c trap_builtin -> sig.c:830
/// set_signal_handler with `act.sa_flags = 0`, sig.c:835 -- NO SA_RESTART:
/// the handler may interrupt blocking syscalls exactly like GNU's), `trap ''`
/// sets SIG_IGN (trap.def: ignore), and `trap -` restores the entry
/// disposition. Arrivals under an installed handler are queued (the
/// record-while-blocked contract, trap.c:537-543 set_trap_state) and
/// dispatched at command boundaries by trap_exec::run_pending_signal_traps.
/// The dispatch side is platform-shared: this backend and the Windows file
/// mailbox both surface as the same `Vec<i32>` from take_pending_signals.
///
/// Deliberately NOT registered:
/// - SIGCHLD: the reap-point dispatcher
///   (Executor::run_sigchld_trap_for_reaped_child) already fires the CHLD
///   trap exactly once per reaped child; a kernel SIGCHLD here would
///   double-fire it.
/// - SIGKILL is uncatchable by design.
///
/// The stop class (SIGTSTP/SIGTTIN/SIGTTOU) and SIGCONT ARE registered
/// (rubash#2344): GNU trap.c:838 set_signal installs trap_handler for
/// ANY trapped signal, the stop class included, and defers execution
/// to trap.c:328 run_pending_traps, so a trapped stop signal never
/// stops the shell. Registration only makes a signal ELIGIBLE for the
/// trap table's wishes: an untrapped stop signal's wish is Default, so
/// its disposition stays SIG_DFL and the kernel stop is unchanged; the
/// handler goes in only while a trap names the signal, and the
/// boundary dispatcher's stale-handler check drops a delivery that
/// raced a trap clear — so registration cannot turn an untrapped stop
/// into a spurious 128+N exit (the failure mode an earlier draft of
/// this comment feared from catching them unconditionally).
#[cfg(unix)]
pub(in crate::signals) mod kernel_signals {
    use signal_hook::iterator::exfiltrator::WithOrigin;
    use signal_hook::iterator::SignalsInfo;
    use signal_hook::low_level::siginfo::Cause;
    use std::io;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    static KERNEL_SIGNALS: OnceLock<Mutex<SignalsInfo<WithOrigin>>> = OnceLock::new();

    /// Bitmask (same bit layout as HANDLER_SET) of managed signals whose
    /// most recently drained kernel delivery was KERNEL-caused
    /// (siginfo si_code SI_KERNEL: generated by the terminal driver for
    /// ^C/^\\, not by a kill(2) from any process). The distinction is
    /// observable only here, at the drain — the pending queues carry
    /// bare signal numbers — and the one consumer is the builtin-sleep
    /// fast path's virtual-child model (trap_exec sleep_interruptibly,
    /// rubash#1034): a real foreground child dies only from the
    /// group-addressed terminal delivery, so only a kernel-caused
    /// arrival may end the virtual sleep early; a kill(2) delivery
    /// addressed to this shell alone leaves GNU's real child running
    /// and the trap deferred until the command completes.
    static KERNEL_CAUSED: AtomicU64 = AtomicU64::new(0);

    /// Whether the most recently drained delivery of `sig` was
    /// kernel-caused (see KERNEL_CAUSED).
    pub(in crate::signals) fn last_delivery_kernel_caused(sig: i32) -> bool {
        if !is_managed(sig) {
            return false;
        }
        KERNEL_CAUSED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// An in-process self delivery (the kill builtin addressing this
    /// shell alone) supersedes the origin record for `sig`: it is by
    /// construction NOT a kernel/group delivery.
    pub(in crate::signals) fn note_self_delivery(sig: i32) {
        if is_managed(sig) {
            KERNEL_CAUSED.fetch_and(!bit(sig), Ordering::Relaxed);
            // rubash#2241: the sender record agrees — a self delivery
            // is an in-group, shell-only addressing by construction.
            record_sender_pid(sig, std::process::id() as i32);
        }
    }

    /// Sender pid of the most recently drained kill(2)-caused delivery,
    /// indexed by signal number (0 = none recorded, kernel-caused, or
    /// no sender information). Companion to KERNEL_CAUSED: the origin
    /// bit alone cannot tell a GROUP-addressed kill(2) — the delivery
    /// bats' CTRL-C tests use (kill -INT -- -pgid, rubash#2241) — from
    /// one addressed to this shell alone (rubash#1034's deferral
    /// shape); the sender's process-group membership can, because a
    /// group delivery's sender necessarily sits OUTSIDE the target
    /// group while a shell-only `kill -INT $$` from one of the shell's
    /// own jobs comes from INSIDE it. Consumed only by the
    /// virtual-child model in trap_exec::sleep_interruptibly.
    static LAST_SENDER_PID: OnceLock<Mutex<[i32; 128]>> = OnceLock::new();

    fn sender_pid_table() -> &'static Mutex<[i32; 128]> {
        LAST_SENDER_PID.get_or_init(|| Mutex::new([0; 128]))
    }

    fn record_sender_pid(sig: i32, pid: i32) {
        if !is_managed(sig) || !(0..128).contains(&sig) {
            return;
        }
        if let Ok(mut table) = sender_pid_table().lock() {
            table[sig as usize] = pid;
        }
    }

    /// Sender pid recorded for the latest drained delivery of `sig`
    /// (see LAST_SENDER_PID); None when nothing usable was recorded.
    pub(in crate::signals) fn last_delivery_sender_pid(sig: i32) -> Option<u32> {
        if !is_managed(sig) || !(0..128).contains(&sig) {
            return None;
        }
        let table = sender_pid_table().lock().ok()?;
        match table[sig as usize] {
            pid if pid > 0 => Some(pid as u32),
            _ => None,
        }
    }

    /// Monotonic count of kernel-caused SIGINT deliveries drained so
    /// far (rubash#1990). Unlike KERNEL_CAUSED — a last-wins origin bit
    /// a later kill(2) delivery of the same signal can flip back — the
    /// epoch never decreases, so a consumer that snapshots it at the
    /// start of a wait can tell "a terminal ^C arrived DURING this
    /// wait" even if other deliveries interleave. The foreground wait
    /// (executor external_inner) uses it to forward the interrupt to a
    /// child the terminal delivery never reached.
    static SIGINT_KERNEL_EPOCH: AtomicU64 = AtomicU64::new(0);

    /// Current value of the kernel-SIGINT epoch (see SIGINT_KERNEL_EPOCH).
    pub(in crate::signals) fn sigint_kernel_epoch() -> u64 {
        SIGINT_KERNEL_EPOCH.load(Ordering::Relaxed)
    }

    /// The `trap ''` ignore set, recorded from the trap TABLE by the
    /// boundary reconcile (reconcile_kernel_trap_dispositions) — never
    /// from the live kernel state word (rubash#1991). The state word
    /// also carries shell-policy ignores that exist only for this
    /// process: an interactive shell ignores SIGTERM for itself
    /// (sig.c:338-339, where get_original_signal snapshots the entry
    /// disposition BEFORE the ignore is installed). GNU's fork-child
    /// restore (trap.c:1489 restore_original_signals) restores from
    /// that entry snapshot plus the trap '' set, so a policy ignore
    /// must not leak into children — reading the live state word in
    /// pre_exec did exactly that, and every child of an interactive
    /// shell inherited SIG_IGN for SIGTERM (kill %1 then "succeeded"
    /// while the job ran on, immune to TERM).
    static TRAP_IGNORED: AtomicU64 = AtomicU64::new(0);

    /// Record whether `sig` is currently ignored via `trap ''`.
    pub(in crate::signals) fn set_trap_ignored(sig: i32, ignored: bool) {
        if !is_managed(sig) {
            return;
        }
        if ignored {
            TRAP_IGNORED.fetch_or(bit(sig), Ordering::Relaxed);
        } else {
            TRAP_IGNORED.fetch_and(!bit(sig), Ordering::Relaxed);
        }
    }

    /// True when `sig` is ignored via `trap ''` (see TRAP_IGNORED).
    pub(in crate::signals) fn trap_ignored(sig: i32) -> bool {
        is_managed(sig) && TRAP_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// Clear the whole trap '' record (the reconcile fast path, where
    /// no trap-table keys exist at all).
    pub(in crate::signals) fn clear_trap_ignored_all() {
        TRAP_IGNORED.store(0, Ordering::Relaxed);
    }

    /// The named signals the backend manages dispositions for (the
    /// signal_hook-catchable set a trap can name and the executor can
    /// dispatch). SIGPIPE joined the set with rubash#1333: it used to be
    /// deliberately excluded because the Rust runtime pre-ignores it
    /// (see capture_entry_and_policy, which resets it to SIG_DFL), but
    /// GNU leaves SIGPIPE at SIG_DFL in a script shell, so an untrapped
    /// delivery must kill the shell (exit 141) and a PIPE trap must
    /// fire.
    ///
    /// rubash#1511 extends the original seven (HUP/INT/QUIT/TERM/USR1/
    /// USR2/PIPE) to every remaining trappable signal: until then a trap
    /// (or `trap ''`) on TRAP/ABRT/ALRM/STKFLT/XCPU/XFSZ/VTALRM/PROF/IO/
    /// PWR/SYS/RT* changed nothing at the kernel, so a trapped
    /// terminating signal still killed the shell by its default action
    /// and a trapped URG/WINCH (default-ignore) was never delivered —
    /// GNU installs one handler per trapped signal (sig.c:830), so all
    /// of them run their trap. SIGSEGV/SIGBUS are NOT here: the fatal
    /// set below routes them (rubash#1475), and rubash#1511 routes
    /// SIGILL/SIGFPE there too (signal-hook refuses to register them).
    /// SIGCHLD stays unregistered for the reason in the module doc.
    /// The stop class and SIGCONT close the set (rubash#2344): they
    /// were the last trappable signals the trap table recorded but the
    /// kernel never heard about — a trapped TSTP stopped the shell for
    /// real and the trap was silently dropped on resume, where GNU
    /// trap.c:838 set_signal installs the deferring handler for them
    /// exactly as for every other trapped signal.
    const MANAGED_CORE: &[i32] = &[
        libc::SIGHUP as i32,
        libc::SIGINT as i32,
        libc::SIGQUIT as i32,
        libc::SIGTERM as i32,
        libc::SIGUSR1 as i32,
        libc::SIGUSR2 as i32,
        libc::SIGPIPE as i32,
        libc::SIGTRAP as i32,
        libc::SIGABRT as i32,
        libc::SIGALRM as i32,
        libc::SIGXCPU as i32,
        libc::SIGXFSZ as i32,
        libc::SIGVTALRM as i32,
        libc::SIGPROF as i32,
        libc::SIGIO as i32,
        libc::SIGSYS as i32,
        libc::SIGURG as i32,
        libc::SIGWINCH as i32,
        libc::SIGTSTP as i32,
        libc::SIGTTIN as i32,
        libc::SIGTTOU as i32,
        libc::SIGCONT as i32,
    ];

    /// Linux/Android-only managed additions: SIGSTKFLT and SIGPWR
    /// (no Darwin/BSD counterpart in GNU's table either) plus the
    /// POSIX realtime block, whose bounds are runtime values (glibc
    /// SIGRTMIN()=34, SIGRTMAX()=64; bionic 41..64; Darwin/BSD have
    /// none, where GNU's signames.c compiles the block out too).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn extend_managed(signals: &mut Vec<i32>) {
        signals.push(libc::SIGSTKFLT as i32);
        signals.push(libc::SIGPWR as i32);
        let lo = libc::SIGRTMIN();
        let hi = libc::SIGRTMAX();
        if lo > 0 && hi >= lo {
            signals.extend(lo..=hi);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn extend_managed(_signals: &mut Vec<i32>) {}

    /// The full managed list: MANAGED_CORE plus the platform extras.
    fn managed_signals() -> &'static [i32] {
        static MANAGED: OnceLock<Vec<i32>> = OnceLock::new();
        MANAGED.get_or_init(|| {
            let mut signals = MANAGED_CORE.to_vec();
            extend_managed(&mut signals);
            signals
        })
    }

    fn is_managed(sig: i32) -> bool {
        managed_signals().contains(&sig)
    }

    /// Bitmask (1 << index into MANAGED) of signals currently routed to the
    /// signal_hook handler.
    static HANDLER_SET: AtomicU64 = AtomicU64::new(0);

    /// The signal_hook dispatcher's installed sigaction, captured verbatim
    /// at the first live handler install (rubash#2348). signal-hook's
    /// Handle caches each signal's registration, so a later add_signal for
    /// the same signal is a no-op — but reconcile's Default/Ignore
    /// transitions overwrite the OS disposition with raw sigaction calls
    /// (SIG_DFL / SIG_IGN). After such a flip the backend slot is still
    /// registered while the kernel no longer routes deliveries to it, so
    /// re-arming a trap on that signal (GNU trap.c:838 set_signal
    /// reinstalling trap_handler after the trap.c:1398 REVERT round-trip)
    /// silently leaves the flipped disposition in place: a Default death
    /// or an Ignore swallow sticks. ensure_handler detects the desync —
    /// Handler wish, non-handler OS disposition — and reinstalls this
    /// captured dispatcher, reconnecting the still-registered slot.
    static BACKEND_ACTION: OnceLock<libc::sigaction> = OnceLock::new();

    /// Packed per-signal disposition state (2 bits each: 0=Default,
    /// 1=Ignore, 2=Handler) so the per-boundary reconcile skips
    /// unchanged signals — no sigaction traffic on the steady-state
    /// command path. Two words: rubash#1511's full managed list runs
    /// past 32 signals (102 state bits on Linux).
    static DISPOSITION_STATE: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

    /// (word, shift) of `sig`'s 2-bit field in DISPOSITION_STATE (two
    /// bits per signal, in managed_signals() order).
    fn state_slot(sig: i32) -> (usize, u32) {
        let bits = 2 * managed_signals()
            .iter()
            .position(|&m| m == sig)
            .expect("managed signal");
        (bits / 64, (bits % 64) as u32)
    }

    fn current_state(sig: i32) -> u8 {
        let (word, shift) = state_slot(sig);
        ((DISPOSITION_STATE[word].load(Ordering::Relaxed) >> shift) & 0b11) as u8
    }

    fn store_state(sig: i32, state: u8) {
        let (word, shift) = state_slot(sig);
        let mask = 0b11u64 << shift;
        let previous = DISPOSITION_STATE[word].fetch_and(!mask, Ordering::SeqCst);
        DISPOSITION_STATE[word].fetch_or((state as u64) << shift, Ordering::SeqCst);
        let _ = previous;
    }

    /// Bitmask of signals that were SIG_IGN when this process was exec'd.
    /// POSIX 2.11: a non-interactive shell keeps entry-ignored signals
    /// ignored (GNU sig.c:273-277 marks them hard-ignored).
    static ENTRY_IGNORED: AtomicU64 = AtomicU64::new(0);

    /// Process-wide probe of EVERY signal's entry disposition (bit
    /// sig-1), captured once on first use. GNU trap.c initialize_traps
    /// snapshots original_signals[] for all NSIG signals at startup;
    /// trap -p lists the SIG_IGN ones as `trap -- '' SIG` and the trap
    /// builtin refuses to trap or reset them (SIG_HARD_IGNORE,
    /// trap.c:850). The MANAGED-only ENTRY_IGNORED above cannot serve
    /// that display: entry-ignored SIGPIPE/SIGXFSZ/realtime signals are
    /// outside the managed set (rubash#1124). The probe runs before any
    /// rubash code changes a disposition (seed_startup_traps in
    /// Executor::new precedes install()'s SIGQUIT policy ignore), and
    /// the OnceLock keeps later executors (subshell clones) from
    /// re-probing a live, already-mutated disposition table.
    static ENTRY_IGNORED_ALL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

    pub(in crate::signals) fn entry_ignored_mask_all() -> u64 {
        *ENTRY_IGNORED_ALL.get_or_init(|| {
            let mut mask = 0u64;
            for sig in 1..=64 {
                if current_disposition_is_ignored(sig) {
                    mask |= 1u64 << (sig - 1);
                }
            }
            // SIGPIPE is excluded: the Rust runtime sets it to SIG_IGN
            // before main() runs, so its observed disposition is the
            // runtime's, never the invoking environment's — attributing
            // it to the parent would list `trap -- '' SIGPIPE` in shells
            // whose parent left it at SIG_DFL (GNU lists nothing there).
            // The DISPOSITION itself is separately reset to SIG_DFL in
            // capture_entry_and_policy (rubash#1333); only the entry
            // attribution stays unreconstructed, since a parent that
            // genuinely ignored SIGPIPE is indistinguishable here.
            mask &= !(1u64 << (libc::SIGPIPE as u64 - 1));
            mask
        })
    }

    fn bit(sig: i32) -> u64 {
        1 << managed_signals()
            .iter()
            .position(|&m| m == sig)
            .expect("managed signal")
    }

    fn current_disposition_is_ignored(sig: i32) -> bool {
        // sigaction(nullptr) queries without installing; the returned
        // sa_handler distinguishes SIG_IGN.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) };
        current.sa_sigaction == libc::SIG_IGN
    }

    /// Install the handler for `sig` and clear SA_RESTART from its flags.
    /// signal_hook's registry hardcodes SA_RESTART; GNU installs trap
    /// handlers with sa_flags = 0 (sig.c:835), so a caught signal interrupts
    /// a blocked read/wait exactly like GNU's does.
    fn ensure_handler(signals: &SignalsInfo<WithOrigin>, sig: i32) -> io::Result<()> {
        if HANDLER_SET.load(Ordering::Relaxed) & bit(sig) == 0 {
            signals.add_signal(sig)?;
            HANDLER_SET.fetch_or(bit(sig), Ordering::SeqCst);
        }
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if current.sa_sigaction == libc::SIG_DFL || current.sa_sigaction == libc::SIG_IGN {
            // rubash#2348: the OS disposition is not the backend
            // dispatcher even though the Handler wish is live — a raw
            // Default/Ignore flip overwrote it while signal-hook kept
            // the registration cached, so the add_signal above (if any)
            // installed nothing. Reinstall the captured dispatcher; the
            // still-registered slot (including its chained previous
            // disposition) receives deliveries again. First-ever install
            // cannot reach here with an empty cell: add_signal has just
            // put the dispatcher in place, so `current` would name it.
            if let Some(backend) = BACKEND_ACTION.get() {
                let backend = *backend;
                if unsafe { libc::sigaction(sig, &backend, std::ptr::null_mut()) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                current = backend;
            }
        } else if BACKEND_ACTION.get().is_none() {
            // First live install: capture the dispatcher verbatim (its
            // SA_RESTART bit is normalized by the shared block below,
            // and again on every reinstall, which re-enters that block).
            let _ = BACKEND_ACTION.set(current);
        }
        if current.sa_flags & libc::SA_RESTART != 0 {
            current.sa_flags &= !libc::SA_RESTART;
            // The whole struct is copied (incl. glibc's sa_restorer), so
            // re-installing it only flips the flag.
            if unsafe { libc::sigaction(sig, &current, std::ptr::null_mut()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn set_kernel_handler(sig: i32, handler: usize) {
        let action = libc::sigaction {
            sa_sigaction: handler,
            sa_flags: 0,
            sa_mask: unsafe { std::mem::zeroed() },
            // glibc/musl/OH/bionic all expose sa_restorer as Option; None
            // lets the kernel supply the default restorer.
            #[cfg(any(target_os = "linux", target_os = "android"))]
            sa_restorer: None,
        };
        unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
    }

    /// Effective disposition for one managed signal, derived from the trap
    /// table: a real action -> handler, `trap ''` -> SIG_IGN, no trap ->
    /// entry disposition (SIG_DFL, or SIG_IGN for entry-ignored signals and
    /// SIGQUIT -- sig.c:333 sets SIGQUIT to SIG_IGN for every shell).
    #[derive(Clone, Copy)]
    pub(in crate::signals) enum Disposition {
        Handler,
        Ignore,
        Default,
    }

    /// Mask of the used bits in DISPOSITION_STATE word `word` (2 bits per
    /// managed signal; the tail of the last word is unused).
    fn state_mask(word: u32) -> u64 {
        let total_bits = 2 * managed_signals().len() as u32;
        let used = total_bits.saturating_sub(64 * word);
        if used >= 64 {
            u64::MAX
        } else if used == 0 {
            0
        } else {
            (1u64 << used) - 1
        }
    }

    /// The packed wish-space state of the ENTRY baseline: state 1
    /// (Ignore) for each signal that was SIG_IGN when this process was
    /// exec'd, 0 (Default) for every other managed signal. GNU snapshots
    /// the entry dispositions once in initialize_traps
    /// (trap.c:154, original_signals[] at trap.c:99; an entry-ignored
    /// signal is SIG_HARD_IGNORE, trap.c:63) — a recorded baseline the
    /// per-command path never re-derives. Built once from ENTRY_IGNORED,
    /// which capture_entry_and_policy fills before the first boundary
    /// reconcile; a caller that somehow ran earlier would freeze the
    /// all-Default baseline and merely never take the fast path for a
    /// seeded table (the pre-rubash#2630 status quo), never a wrong skip.
    static BASELINE_STATE: std::sync::OnceLock<[u64; 2]> = std::sync::OnceLock::new();

    fn baseline_state() -> [u64; 2] {
        *BASELINE_STATE.get_or_init(|| {
            let mut words = [0u64; 2];
            let ignored = ENTRY_IGNORED.load(Ordering::Relaxed);
            for (index, _) in managed_signals().iter().enumerate() {
                if ignored & (1u64 << index) != 0 {
                    let bits = 2 * index;
                    words[bits / 64] |= 1u64 << (bits % 64);
                }
            }
            words
        })
    }

    /// True when the recorded disposition state equals the entry
    /// baseline, so a reconcile whose wish-list is also the baseline has
    /// nothing to do. This is the boundary fast path's state gate since
    /// rubash#2630: seed_startup_traps materializes each entry-ignored
    /// signal as an empty trap-table entry whose wish is Ignore, so the
    /// recorded state parks those signals at 1 forever and the
    /// pre-rubash#2630 all-Default state test could never pass — every
    /// command boundary paid the full slow-path reconcile (~2x per
    /// command) in exactly the supervisor/nohup launch shape that
    /// ignores signals on entry. The gate stays sound because it pairs
    /// with the table-side baseline test in
    /// reconcile_kernel_trap_dispositions: whenever the recorded state
    /// has not yet converged to the baseline (first boundary, a trap
    /// mutation, the FG_INT_HOLD window, the SIGINT REVERT rewrite of
    /// the baseline itself) the slow path runs and pushes it there.
    pub(in crate::signals) fn at_baseline_state() -> bool {
        let baseline = baseline_state();
        DISPOSITION_STATE[0].load(Ordering::Relaxed) & state_mask(0) == baseline[0] & state_mask(0)
            && DISPOSITION_STATE[1].load(Ordering::Relaxed) & state_mask(1)
                == baseline[1] & state_mask(1)
    }

    pub(in crate::signals) fn reconcile(dispositions: &[(i32, Disposition)]) {
        // rubash#1027: the signal_hook backend is installed lazily, on
        // the first Handler wish — before that, Ignore/Default wishes
        // are plain sigaction calls that need no backend (and no
        // self-pipe socketpair on fds 3/4). Startup ran only
        // capture_entry_and_policy (fd-free).
        let mut signals_guard = None;
        for &(sig, ref want) in dispositions {
            // rubash#1250: a foreground builtin wait (sleep fast path)
            // holds untrapped SIGINT at the mailbox handler so the
            // delivery becomes an observable abort instead of a DFL
            // death. The hold outranks the trap table's Default wish
            // until released — without this, every per-slice
            // reconcile from observe_signals_while_blocked would flip
            // the routing back to SIG_DFL mid-sleep.
            if FG_INT_HOLD.load(Ordering::Relaxed)
                && sig == libc::SIGINT as i32
                && matches!(want, Disposition::Default)
            {
                continue;
            }
            let target = match want {
                Disposition::Handler => 2u8,
                Disposition::Ignore => 1u8,
                Disposition::Default => 0u8,
            };
            if current_state(sig) == target {
                continue; // steady state: no sigaction traffic
            }
            match want {
                Disposition::Handler => {
                    if signals_guard.is_none() {
                        if install().is_err() {
                            continue;
                        }
                        let Some(mutex) = KERNEL_SIGNALS.get() else {
                            continue;
                        };
                        let Ok(guard) = mutex.lock() else {
                            continue;
                        };
                        signals_guard = Some(guard);
                    }
                    let signals = signals_guard.as_ref().expect("installed above");
                    if ensure_handler(signals, sig).is_err() {
                        continue;
                    }
                }
                Disposition::Ignore => {
                    set_kernel_handler(sig, libc::SIG_IGN);
                    HANDLER_SET.fetch_and(!bit(sig), Ordering::SeqCst);
                }
                Disposition::Default => {
                    let entry_ignored = ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0;
                    // rubash#2347: a reverted entry-ignored SIGINT takes
                    // the fatal default, not the entry-ignore base
                    // (trap.def:228-251 REVERT special case).
                    let base =
                        if sig == libc::SIGINT as i32 && SIGINT_REVERTED.load(Ordering::Relaxed) {
                            libc::SIG_DFL
                        } else if entry_ignored || sig == libc::SIGQUIT as i32 {
                            libc::SIG_IGN
                        } else {
                            libc::SIG_DFL
                        };
                    set_kernel_handler(sig, base);
                    HANDLER_SET.fetch_and(!bit(sig), Ordering::SeqCst);
                    // A delivery that raced the flip may have left the
                    // registry flag set; the boundary dispatch drops such
                    // entries via handler_installed() instead of acting on
                    // them (draining here would swallow OTHER signals'
                    // pending deliveries).
                }
            }
            store_state(sig, target);
        }
    }

    /// True when `sig` is currently routed to the signal_hook handler. A
    /// pending-queue entry for a signal that is NOT handler-installed is a
    /// stale registry flag from a delivery that raced a disposition flip
    /// (trap cleared): SIG_DFL would have killed the process and SIG_IGN
    /// swallows, so neither can produce a legitimate pending entry.
    pub(in crate::signals) fn handler_installed(sig: i32) -> bool {
        if fatal_index(sig).is_some() {
            // The fatal-set handler is installed unconditionally at
            // startup (rubash#1475); a queued entry for the set is
            // legitimate by construction, never a stale registry flag.
            return true;
        }
        is_managed(sig) && HANDLER_SET.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// True when `sig` was SIG_IGN at process entry (captured by
    /// capture_entry_and_policy). Entry-ignored dispositions are sticky
    /// (trap.c:850): neither traps nor the interactive SIGINT pin
    /// (rubash#1662) may route them to the handler.
    pub(in crate::signals) fn entry_ignored(sig: i32) -> bool {
        is_managed(sig) && ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// The stop class: SIGTSTP/SIGTTIN/SIGTTOU, whose default action
    /// is to stop the process (SIGCONT is their resume counterpart).
    pub(in crate::signals) fn is_stop_class(sig: i32) -> bool {
        sig == libc::SIGTSTP as i32 || sig == libc::SIGTTIN as i32 || sig == libc::SIGTTOU as i32
    }

    /// rubash#2347: set once `trap - SIGINT` runs against an
    /// entry-ignored SIGINT. GNU restore_default_signal early-returns on
    /// hard-ignored signals, but trap_builtin's REVERT branch then
    /// unconditionally installs the fatal SIGINT handler anyway
    /// (builtins/trap.def:228-251, citing POSIX interp 751) — the one
    /// signal whose REVERT overrides the entry ignore. SIGQUIT is the
    /// opposite hard-code (forced SIG_IGN, already matched) and TERM /
    /// USR1 reverts stay no-ops, so this flag keys on SIGINT alone.
    /// Once set, SIGINT's Default wish resolves to the plain fatal
    /// default instead of the entry-ignore base, and the interactive
    /// SIGINT pin may route it again. Trap-set on SIGINT stays a
    /// hard-ignore no-op (trap.rs), exactly as in GNU.
    static SIGINT_REVERTED: AtomicBool = AtomicBool::new(false);

    pub(in crate::signals) fn sigint_reverted() -> bool {
        SIGINT_REVERTED.load(Ordering::Relaxed)
    }

    /// Apply the trap.def REVERT special case for an entry-ignored
    /// SIGINT, synchronously like the builtin does: a non-interactive
    /// shell gets the fatal default (SIG_DFL) right now — reconcile's
    /// fast path may otherwise skip a table that reads all-Default —
    /// while an interactive shell only records the flag and lets the
    /// reconcile below install its SIGINT pin (GNU installs
    /// sigint_sighandler there).
    pub(in crate::signals) fn revert_sigint(interactive: bool) {
        SIGINT_REVERTED.store(true, Ordering::Relaxed);
        if !interactive {
            set_kernel_handler(libc::SIGINT as i32, libc::SIG_DFL);
            HANDLER_SET.fetch_and(!bit(libc::SIGINT as i32), Ordering::SeqCst);
            store_state(libc::SIGINT as i32, 0);
        }
    }

    /// Whether the boundary dispatcher can adjudicate `sig` at all
    /// (the managed set or the fatal set). The rubash#1654 ancestor
    /// route queues only signals whose top-level verdict the
    /// dispatcher owns; SIGKILL and the stop class are not adjudicable
    /// and keep raw raise(2) semantics. The stop class joined the
    /// managed set for kernel-disposition purposes in rubash#2344, but
    /// it stays non-adjudicable HERE (SIGCONT with it): these lanes
    /// synthesize death verdicts (128+sig) for a sibling layer, and a
    /// stop/resume has no death verdict to synthesize — the raw
    /// raise(2) under the then-current kernel disposition (the
    /// deferring handler if a layer trapped the signal, the SIG_DFL
    /// stop otherwise) is the faithful outcome.
    pub(in crate::signals) fn adjudicable(sig: i32) -> bool {
        (is_managed(sig) && !is_stop_class(sig) && sig != libc::SIGCONT as i32)
            || fatal_index(sig).is_some()
    }

    /// rubash#1475: the fatal set, SIGSEGV and SIGBUS. Both are caught
    /// by the Rust runtime before main() — its stack-overflow guard
    /// installs a handler for them (visible in /proc/self/status SigCgt
    /// on every rubash process) — and that handler RETURNS for a signal
    /// that was SENT rather than faulted, so an untrapped `kill -SEGV
    /// $$`, whether from this shell's own kill builtin (a real raise(2)
    /// on unix) or from any other process, was silently swallowed and
    /// the shell ran on. GNU runs both at SIG_DFL in a script shell
    /// (sig.c initialize_signals never catches them), so an untrapped
    /// delivery kills the shell and a trapped one runs its trap.
    /// signal_hook cannot serve the pair: its handler is a signo queue
    /// writer with no siginfo, and returning from a handler for a
    /// synchronous fault re-executes the faulting instruction forever.
    ///
    /// rubash#1511 adds SIGILL and SIGFPE to the set: signal-hook's
    /// registry flatly refuses to register them (it panics with
    /// "Attempted to register forbidden signal"), so the managed
    /// backend cannot route them either. They share the synchronous-
    /// fault nature that demands the si_code split below — a SENT
    /// delivery gets shell semantics, a genuine fault chains — so they
    /// ride this backend rather than the signo queue.
    /// The custom handler below splits arrivals on siginfo.si_code:
    ///   - si_code <= 0 (SI_USER / SI_TKILL / SI_QUEUE / ...): the
    ///     signal was SENT. With a trap armed, count it for the boundary
    ///     dispatcher (drained through drain_fatal into the same
    ///     Vec<i32> as the managed set); with `trap ''`, drop it;
    ///     untrapped, restore SIG_DFL and re-raise so the process dies
    ///     BY the signal on the spot, exactly as a default disposition.
    ///   - si_code > 0: a hardware fault (SEGV_MAPERR, BUS_ADRALN...).
    ///     Chain to the disposition saved at install time — in practice
    ///     the runtime's overflow handler — so stack-overflow detection
    ///     and genuine-fault death keep working byte for byte.
    const FATAL: [i32; 4] = [
        libc::SIGSEGV as i32,
        libc::SIGBUS as i32,
        libc::SIGILL as i32,
        libc::SIGFPE as i32,
    ];

    /// Queued SENT deliveries per fatal signal (handler increments,
    /// drain_fatal swaps out; a plain atomic add is async-signal-safe).
    static FATAL_PENDING: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    /// Trap routing for SENT deliveries, per fatal signal:
    /// 0 = Default (die), 1 = Ignore (drop), 2 = Handler (queue).
    /// Mirrored from the trap table by reconcile_fatal.
    static FATAL_STATE: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    /// Dispositions saved when the fatal handler was installed — the
    /// hardware-fault chain targets. Written once inside
    /// capture_entry_and_policy's Once before the handler is installed;
    /// the Once's happens-before edge covers the handler's later reads.
    static FATAL_PREV_HANDLER: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    static FATAL_PREV_FLAGS: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    fn fatal_index(sig: i32) -> Option<usize> {
        FATAL.iter().position(|&f| f == sig)
    }

    extern "C" fn fatal_signal_handler(
        sig: i32,
        info: *mut libc::siginfo_t,
        context: *mut libc::c_void,
    ) {
        let Some(index) = fatal_index(sig) else {
            return;
        };
        let si_code = if info.is_null() {
            0
        } else {
            unsafe { (*info).si_code }
        };
        if si_code > 0 {
            // Hardware fault: chain to the saved disposition.
            let prev = FATAL_PREV_HANDLER[index].load(Ordering::Relaxed) as usize;
            match prev {
                libc::SIG_IGN => return,
                libc::SIG_DFL => {
                    set_kernel_handler(sig, libc::SIG_DFL);
                    unsafe { libc::raise(sig) };
                    return;
                }
                _ => {}
            }
            let prev_flags = FATAL_PREV_FLAGS[index].load(Ordering::Relaxed) as i32;
            // SAFETY: prev is the handler address the process had
            // installed for this signal at startup (the Rust runtime's
            // overflow guard); invoking it with the delivery's own
            // arguments is exactly what the kernel would have done.
            unsafe {
                if prev_flags & libc::SA_SIGINFO != 0 {
                    let handler: extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void) =
                        std::mem::transmute(prev);
                    handler(sig, info, context);
                } else {
                    let handler: extern "C" fn(i32) = std::mem::transmute(prev);
                    handler(sig);
                }
            }
            return;
        }
        // Sent by kill/tgkill/raise/sigqueue: shell semantics apply.
        match FATAL_STATE[index].load(Ordering::Relaxed) {
            2 => {
                FATAL_PENDING[index].fetch_add(1, Ordering::Relaxed);
            }
            1 => {}
            _ => {
                set_kernel_handler(sig, libc::SIG_DFL);
                unsafe { libc::raise(sig) };
            }
        }
    }

    /// Install the fatal-set handler, saving the current dispositions
    /// as the hardware-fault chain targets. Called from
    /// capture_entry_and_policy's Once, before any trap can exist, so
    /// both signals start at Default routing (an untrapped sent delivery
    /// dies). A signal that is SIG_IGN at this point (entry-ignored, or
    /// on a platform whose runtime ignores it) starts at Ignore routing
    /// and reconcile_fatal never arms it — POSIX hard-ignore.
    fn install_fatal_handlers() {
        for (index, &sig) in FATAL.iter().enumerate() {
            let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
            unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) };
            FATAL_PREV_HANDLER[index].store(current.sa_sigaction as u64, Ordering::Relaxed);
            FATAL_PREV_FLAGS[index].store(current.sa_flags as i64 as u64, Ordering::Relaxed);
            if current.sa_sigaction == libc::SIG_IGN {
                FATAL_STATE[index].store(1, Ordering::Relaxed);
            }
            let action = libc::sigaction {
                sa_sigaction: fatal_signal_handler as *const () as usize,
                sa_flags: libc::SA_SIGINFO,
                sa_mask: unsafe { std::mem::zeroed() },
                #[cfg(any(target_os = "linux", target_os = "android"))]
                sa_restorer: None,
            };
            unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
        }
    }

    /// Drain the fatal set's queued SENT deliveries (take_all_signals
    /// merges them with the managed drain).
    pub(in crate::signals) fn drain_fatal() -> Vec<i32> {
        let mut signals = Vec::new();
        for (index, &sig) in FATAL.iter().enumerate() {
            let count = FATAL_PENDING[index].swap(0, Ordering::Relaxed);
            for _ in 0..count {
                signals.push(sig);
            }
        }
        signals
    }

    pub(in crate::signals) fn fatal_pending_any() -> bool {
        FATAL_PENDING
            .iter()
            .any(|count| count.load(Ordering::Relaxed) > 0)
    }

    /// True when every fatal signal sits at Default routing, so the
    /// boundary reconcile's fast path may skip the trap-table walk.
    pub(in crate::signals) fn fatal_at_default() -> bool {
        FATAL_STATE
            .iter()
            .all(|state| state.load(Ordering::Relaxed) == 0)
    }

    /// Mirror the trap table's wishes for the fatal set (the managed
    /// reconcile's Disposition mapping): Handler queues sent deliveries
    /// for the boundary dispatcher, Ignore drops them, Default restores
    /// die-on-delivery. Entry-ignored signals stay pinned at Ignore.
    pub(in crate::signals) fn reconcile_fatal(dispositions: &[(i32, Disposition)]) {
        for &(sig, want) in dispositions {
            let Some(index) = fatal_index(sig) else {
                continue;
            };
            if FATAL_PREV_HANDLER[index].load(Ordering::Relaxed) as usize == libc::SIG_IGN {
                continue;
            }
            let target = match want {
                Disposition::Handler => 2u64,
                Disposition::Ignore => 1u64,
                Disposition::Default => 0u64,
            };
            FATAL_STATE[index].store(target, Ordering::Relaxed);
        }
    }

    /// rubash#1250: foreground-builtin SIGINT hold. While set, the
    /// reconcile above refuses to route SIGINT back to Default, so a
    /// builtin wait's temporary mailbox routing survives the per-slice
    /// reconciles from observe_signals_while_blocked. Single-threaded
    /// executor: no concurrent trap-table mutation can interleave, and
    /// the hold only ever arms when SIGINT is untrapped (a trapped
    /// signal keeps its handler and never reaches the hold).
    static FG_INT_HOLD: AtomicBool = AtomicBool::new(false);

    /// rubash#1824: the steady state the live hold found on entry,
    /// recorded by the FIRST holder only (a nested hold finds the
    /// outer hold's Handler routing and must not overwrite the
    /// record). The release restores this state instead of always
    /// routing back to Default: since the rubash#1662 interactive
    /// pin, an interactive shell's untrapped SIGINT already sits at
    /// the mailbox Handler when a foreground wait begins, and
    /// flipping it to Default on release would reopen the unpinned
    /// window the pin closed until the next boundary reconcile.
    static FG_INT_WAS_DEFAULT: AtomicBool = AtomicBool::new(true);

    fn sigint_at_default() -> bool {
        // sigaction(null) query, same probe as the entry-ignored
        // snapshot: only SIG_DFL counts as the Default steady state.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGINT, std::ptr::null(), &mut current) } != 0 {
            return false;
        }
        current.sa_sigaction == libc::SIG_DFL
    }

    /// Route untrapped SIGINT to the mailbox handler and hold it there.
    /// Deliveries become observable pending entries (aborted as 128+SIGINT
    /// by the waiter's slice loop) instead of DFL deaths.
    pub(in crate::signals) fn hold_fg_sigint() {
        if !FG_INT_HOLD.swap(true, Ordering::SeqCst) {
            FG_INT_WAS_DEFAULT.store(sigint_at_default(), Ordering::SeqCst);
        }
        reconcile(&[(libc::SIGINT as i32, Disposition::Handler)]);
    }

    /// Release the hold: clear the flag first so the recorded wish
    /// applies, then restore the steady state the hold found
    /// (rubash#1824) — Default when the wait started from SIG_DFL,
    /// Handler when the rubash#1662 pin had already routed SIGINT
    /// to the mailbox.
    pub(in crate::signals) fn release_fg_sigint() {
        FG_INT_HOLD.store(false, Ordering::SeqCst);
        let target = if FG_INT_WAS_DEFAULT.load(Ordering::SeqCst) {
            Disposition::Default
        } else {
            Disposition::Handler
        };
        reconcile(&[(libc::SIGINT as i32, target)]);
    }

    static ENTRY_CAPTURED: std::sync::Once = std::sync::Once::new();

    /// rubash#2084: whether the entry *policy* (SIGPIPE reset, SIGQUIT
    /// ignore, fatal-handler install) is currently applied to this
    /// process. Split from `ENTRY_CAPTURED`: the entry *facts*
    /// (`ENTRY_IGNORED`) are birth facts, captured once — but the
    /// policy installs are a session resource. The last root
    /// executor's drop restores the pre-session dispositions and
    /// clears this flag; the next root birth re-applies.
    /// Re-application is consistent because `install_fatal_handlers`
    /// re-reads the then-current dispositions as its chain targets.
    static ENTRY_POLICY_APPLIED: AtomicBool = AtomicBool::new(false);

    /// The fd-free part of startup (rubash#1027): capture entry
    /// dispositions and apply the SIGQUIT policy ignore. Neither needs
    /// the signal_hook self-pipe, so this runs at shell startup without
    /// allocating the socketpair that used to sit on fds 3/4 of every
    /// shell (and of every process listing /proc/$$/fd) for its whole
    /// life. GNU has no self-pipe at all (sig.c handlers set flags), so
    /// the pair must not exist before a trap actually needs the backend.
    pub(in crate::signals) fn capture_entry_and_policy() {
        ENTRY_CAPTURED.call_once(|| {
            // Capture entry dispositions BEFORE changing anything:
            // signals ignored on entry must stay ignored (POSIX 2.11,
            // GNU sig.c:273). SIGPIPE is not recorded: rubash#1333 —
            // its entry disposition is unobservable (the Rust runtime
            // sets SIG_IGN before main() on every process, whatever
            // the parent did).
            for &sig in managed_signals() {
                if sig == libc::SIGPIPE as i32 {
                    continue;
                }
                if current_disposition_is_ignored(sig) {
                    ENTRY_IGNORED.fetch_or(bit(sig), Ordering::SeqCst);
                }
            }
        });
        apply_entry_policy();
    }

    fn apply_entry_policy() {
        if ENTRY_POLICY_APPLIED.swap(true, Ordering::SeqCst) {
            return;
        }
        // rubash#1333: SIGPIPE's entry disposition is unobservable —
        // the Rust runtime sets SIG_IGN before main() on every
        // process, whatever the parent did. GNU's script shell runs
        // with SIGPIPE at SIG_DFL (sig.c initialize_signals never
        // ignores it), so reset it: an untrapped delivery (self
        // `kill -PIPE`, EPIPE on a closed pipe) must kill the shell
        // with 141 and a PIPE trap must be installable. This
        // supersedes the rubash#440 rationale for keeping the runtime
        // ignore: write errors now surface as the signal, exactly as
        // under GNU. Applied here rather than in the capture so the
        // entry *facts* stay one-shot while the policy is restorable.
        set_kernel_handler(libc::SIGPIPE as i32, libc::SIG_DFL);
        store_state(libc::SIGPIPE as i32, 0);
        // sig.c:102 initialize_signals -> sig.c:333: every shell
        // ignores SIGQUIT (script shells do not die on ^\).
        if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(libc::SIGQUIT as i32) == 0
            && !current_disposition_is_ignored(libc::SIGQUIT as i32)
        {
            set_kernel_handler(libc::SIGQUIT as i32, libc::SIG_IGN);
        }
        store_state(libc::SIGQUIT as i32, 1);
        // rubash#1475: replace the runtime's SIGSEGV/SIGBUS handlers
        // with the fatal-pair handler (see install_fatal_handlers).
        install_fatal_handlers();
    }

    /// rubash#2084: root-executor ownership of the process signal
    /// policy, behind the one process-state critical section (the
    /// same lock the reconcile slow path and the env mirror use), so
    /// acquire/snapshot and release/restore are atomic against
    /// disposition reconciles and env writes of concurrent sessions.
    /// `(live root count, pre-policy snapshot)`.
    static SESSION_POLICY: Mutex<(u64, Option<Vec<(i32, libc::sigaction)>>)> =
        Mutex::new((0, None));

    fn current_sigaction(sig: i32) -> libc::sigaction {
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: querying this process's own disposition; the
        // out-pointer is a valid stack slot.
        unsafe {
            libc::sigaction(sig, std::ptr::null(), &mut current);
        }
        current
    }

    /// Take a share of the process signal policy. The first share of
    /// an era snapshots the full `sigaction` structs of every managed
    /// signal and every fatal signal — taken before any policy of
    /// this era is applied (the caller acquires before the mailbox
    /// registration that applies it), so the snapshot is exactly the
    /// dispositions the host had.
    pub(in crate::signals) fn acquire_session_policy() {
        let _process_state = crate::executor::local_helpers::acquire_process_state();
        let mut slot = SESSION_POLICY.lock().unwrap_or_else(|e| e.into_inner());
        if slot.0 == 0 {
            let mut snapshot = Vec::new();
            for &sig in managed_signals() {
                snapshot.push((sig, current_sigaction(sig)));
            }
            for &sig in FATAL.iter() {
                snapshot.push((sig, current_sigaction(sig)));
            }
            slot.1 = Some(snapshot);
        }
        slot.0 += 1;
    }

    /// Drop one share; the last drop reinstalls the snapshot verbatim
    /// and rewinds the recorded-state words to match it, so the host
    /// process is exactly as it was before the session era (the #2084
    /// evidence: a child spawned after an in-process Session inherited
    /// an ignored SIGQUIT and showed `trap -- '' SIGQUIT`). A snapshot
    /// handler that is neither SIG_DFL nor SIG_IGN records state 3
    /// ("foreign"): it never equals a reconcile target, so a later
    /// era's reconciles always re-apply, and the baseline-state fast
    /// path stays honestly closed.
    pub(in crate::signals) fn release_session_policy() {
        let _process_state = crate::executor::local_helpers::acquire_process_state();
        let mut slot = SESSION_POLICY.lock().unwrap_or_else(|e| e.into_inner());
        slot.0 = slot.0.saturating_sub(1);
        if slot.0 > 0 {
            return;
        }
        let Some(snapshot) = slot.1.take() else {
            return;
        };
        for (sig, action) in &snapshot {
            // SAFETY: reinstalling a disposition this process itself
            // queried earlier; `sig` is a managed or fatal signal.
            unsafe {
                libc::sigaction(*sig, action, std::ptr::null_mut());
            }
            if let Some(idx) = fatal_index(*sig) {
                FATAL_STATE[idx].store(
                    if action.sa_sigaction == libc::SIG_IGN {
                        1
                    } else {
                        0
                    },
                    Ordering::SeqCst,
                );
            } else {
                store_state(
                    *sig,
                    match action.sa_sigaction {
                        libc::SIG_DFL => 0,
                        libc::SIG_IGN => 1,
                        _ => 3,
                    },
                );
                HANDLER_SET.fetch_and(!bit(*sig), Ordering::SeqCst);
            }
        }
        ENTRY_POLICY_APPLIED.store(false, Ordering::SeqCst);
    }

    /// RAII guard: SIGPIPE blocked on the current thread while alive.
    /// The startup reset (rubash#1333) puts SIGPIPE at SIG_DFL for the
    /// shell process itself — GNU parity for the shell's own writes and
    /// for signal delivery. But an in-process pipeline member SHARES
    /// this process, while GNU forks every member
    /// (execute_cmd.c:2620): a member's write to a closed pipe kills
    /// only the member there. Blocking SIGPIPE for the duration of a
    /// member's execution (and on short-lived payload-writer threads)
    /// makes those writes fail with EPIPE instead, and the stage
    /// machinery converts the failure into the member's 141 status —
    /// the model the pre-#1333 process-wide SIG_IGN provided
    /// implicitly (niubash#345 producers must not take the shell down).
    pub struct SigpipeBlockGuard {
        previous: libc::sigset_t,
    }

    impl SigpipeBlockGuard {
        fn new() -> Self {
            let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
            let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe {
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGPIPE);
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous);
            }
            Self { previous }
        }
    }

    impl Drop for SigpipeBlockGuard {
        fn drop(&mut self) {
            unsafe {
                // A write to a broken pipe with SIGPIPE blocked returns
                // EPIPE but the kernel still leaves the signal PENDING
                // for this thread; it would deliver the instant the mask
                // clears and kill the shell for what was only a member's
                // broken pipe. When the restored mask leaves SIGPIPE
                // unblocked (outermost guard), consume a pending
                // SIGPIPE first. Nested guards restore a still-blocked
                // mask and skip the drain — the outer guard owns it.
                if libc::sigismember(&self.previous, libc::SIGPIPE) == 0 {
                    let mut set: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut set);
                    libc::sigaddset(&mut set, libc::SIGPIPE);
                    dequeue_pending_sigpipe(&set);
                }
                libc::pthread_sigmask(libc::SIG_SETMASK, &self.previous, std::ptr::null_mut());
            }
        }
    }

    /// Dequeue a pending SIGPIPE for this thread without ever blocking
    /// (SIGPIPE is blocked on the thread at this point, so a pending one
    /// just sits in the thread's pending set). Linux/Android spell this
    /// `sigtimedwait` with a zero timeout; that call does not exist in
    /// Darwin's libc (E0425 there), where the portable equivalent is
    /// `sigpending` to test + `sigwait` to consume — sigwait returns
    /// immediately because the signal is already pending, and no other
    /// thread in this process ever waits on SIGPIPE.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn dequeue_pending_sigpipe(set: &libc::sigset_t) {
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let _ = libc::sigtimedwait(set, std::ptr::null_mut(), &timeout);
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe fn dequeue_pending_sigpipe(set: &libc::sigset_t) {
        let mut pending: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut pending);
        if libc::sigpending(&mut pending) == 0 && libc::sigismember(&pending, libc::SIGPIPE) == 1 {
            let mut received: libc::c_int = 0;
            let _ = libc::sigwait(set, &mut received);
        }
    }

    pub(in crate::signals) fn sigpipe_block_guard() -> SigpipeBlockGuard {
        SigpipeBlockGuard::new()
    }

    /// Block SIGPIPE on the calling thread with no restore (for
    /// short-lived writer threads that exit after one payload).
    pub(in crate::signals) fn block_sigpipe_on_this_thread() {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
    }

    /// Unblock SIGPIPE on the calling thread (child pre-exec: the
    /// pipeline guard's block must not cross an exec).
    pub(in crate::signals) fn block_sigpipe_on_this_thread_unblock() {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        }
    }

    pub(in crate::signals) fn install() -> io::Result<()> {
        capture_entry_and_policy();
        if KERNEL_SIGNALS.get().is_some() {
            return Ok(());
        }
        // Empty initial set: no kernel handler exists until a trap names a
        // signal (GNU leaves non-interactive terminating signals SIG_DFL,
        // sig.c:315-316).
        let signals = SignalsInfo::new([] as [i32; 0])?;
        let _ = KERNEL_SIGNALS.set(Mutex::new(signals));
        // Entry-ignored signals stay ignored (captured above); record their
        // state so the first reconcile does not "restore" them to SIG_DFL.
        for &sig in managed_signals() {
            if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0 {
                store_state(sig, 1);
            }
        }
        Ok(())
    }

    pub(in crate::signals) fn drain() -> Vec<i32> {
        let Some(mutex) = KERNEL_SIGNALS.get() else {
            return Vec::new();
        };
        let Ok(mut signals) = mutex.lock() else {
            return Vec::new();
        };
        signals
            .pending()
            .map(|origin| {
                // Record the delivery's origin for the virtual-child
                // model (see KERNEL_CAUSED) before the queues flatten
                // it to a bare signal number. The WithOrigin slot keeps
                // the latest origin per signal, which is exactly the
                // granularity of this record.
                if is_managed(origin.signal) {
                    if matches!(origin.cause, Cause::Kernel) {
                        KERNEL_CAUSED.fetch_or(bit(origin.signal), Ordering::Relaxed);
                        // rubash#2241: a kernel delivery has no sender
                        // process; clear any stale sender record so the
                        // virtual-child model never pairs this arrival
                        // with an earlier kill(2)'s sender.
                        record_sender_pid(origin.signal, 0);
                        if origin.signal == libc::SIGINT {
                            SIGINT_KERNEL_EPOCH.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        KERNEL_CAUSED.fetch_and(!bit(origin.signal), Ordering::Relaxed);
                        // rubash#2241: keep the sender for the
                        // group-addressing test in the virtual-child
                        // model (see LAST_SENDER_PID).
                        record_sender_pid(
                            origin.signal,
                            origin.process.map(|p| p.pid).unwrap_or(0),
                        );
                    }
                }
                origin.signal
            })
            .collect()
    }

    /// True when at least one managed signal is routed to the signal_hook
    /// handler, i.e. the kernel backend can have a pending delivery. A
    /// lock-free atomic read: with no handler installed no kernel delivery
    /// is possible, so the boundary poll (rubash#437) can skip the backend
    /// mutex and `pending()` collect entirely on the untrapped path.
    pub(in crate::signals) fn handler_any_installed() -> bool {
        HANDLER_SET.load(Ordering::Relaxed) != 0
    }

    /// Fork-child disposition restore (rubash#229). GNU's forked
    /// disk-command child runs trap.c:1489 restore_original_signals
    /// between fork and exec (execute_cmd.c:4224): every trapped signal
    /// returns to its ENTRY disposition (reset_signal -> original_signals,
    /// SIG_DFL unless it was SIG_IGN on entry), a `trap ''`-ignored signal
    /// STAYS ignored (trap.c:1451-1455 keeps IGNORE_SIG at SIG_IGN), and
    /// the shell-policy SIGQUIT ignore (sig.c:333) never crosses exec —
    /// without the restore, `sh -c 'kill -QUIT $$'` children inherit the
    /// SIG_IGN and exit 0 instead of dying with 131. Runs in pre_exec:
    /// sigaction and atomic loads are async-signal-safe.
    pub(in crate::signals) fn pre_exec_child_dispositions() -> io::Result<()> {
        // rubash#1333: the pipeline guard blocks SIGPIPE on the
        // EXECUTING thread; a signal mask survives fork+exec, so a
        // child spawned mid-pipeline would inherit the block and its
        // writes would fail EPIPE (a coreutils producer then prints
        // "standard output: Broken pipe" where GNU's child dies
        // silently by the signal). Children always start unblocked;
        // the disposition loop below gives SIGPIPE its entry default.
        block_sigpipe_on_this_thread_unblock();
        for &sig in managed_signals() {
            let action = if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0 {
                // Ignored at shell entry: POSIX keeps it ignored for children.
                libc::SIG_IGN
            } else if sig == libc::SIGQUIT as i32 {
                // The sig.c:333 policy ignore is shell-only; the entry
                // disposition (SIG_DFL here, entry-ignored handled above)
                // is what children exec with.
                libc::SIG_DFL
            } else if trap_ignored(sig) {
                // trap '' ignore (trap.c:1451-1455): survives into
                // children. Read from the trap-table record, NOT the
                // live state word: the word also holds shell-policy
                // ignores (interactive SIGTERM, sig.c:339) that GNU
                // never folds into original_signals, so they must not
                // cross into children (rubash#1991).
                libc::SIG_IGN
            } else {
                // Trapped (handler) or default: children get the entry
                // disposition, which is SIG_DFL for a plain script shell.
                libc::SIG_DFL
            };
            set_kernel_handler(sig, action);
        }
        Ok(())
    }
}

/// The signals `kernel_signals` manages, paired with their trap-table
/// short name and the literal `__RUBASH_TRAP_SIG*` env key. Built once
/// (the realtime block's numbers and names are runtime values, so the
/// table cannot be a const) and reused by every per-command-boundary
/// reconcile, which never allocates a key string after init
/// (rubash#437: the boundary poll runs once or more per loop iteration).
/// Short names match both signal_trap_name's SIG{name} form and the
/// SIGNALS table's spellings. rubash#1511 extends the original seven to
/// the full managed set (see MANAGED_CORE).
#[cfg(unix)]
fn managed_trap_keys() -> &'static [(i32, String, String)] {
    static KEYS: std::sync::OnceLock<Vec<(i32, String, String)>> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<(i32, String, String)> = Vec::new();
        let push = |keys: &mut Vec<(i32, String, String)>, sig: i32, short: &str| {
            keys.push((sig, short.to_string(), format!("__RUBASH_TRAP_SIG{short}")));
        };
        push(&mut keys, libc::SIGHUP as i32, "HUP");
        push(&mut keys, libc::SIGINT as i32, "INT");
        push(&mut keys, libc::SIGQUIT as i32, "QUIT");
        push(&mut keys, libc::SIGTERM as i32, "TERM");
        push(&mut keys, libc::SIGUSR1 as i32, "USR1");
        push(&mut keys, libc::SIGUSR2 as i32, "USR2");
        push(&mut keys, libc::SIGPIPE as i32, "PIPE");
        push(&mut keys, libc::SIGTRAP as i32, "TRAP");
        push(&mut keys, libc::SIGABRT as i32, "ABRT");
        push(&mut keys, libc::SIGALRM as i32, "ALRM");
        push(&mut keys, libc::SIGXCPU as i32, "XCPU");
        push(&mut keys, libc::SIGXFSZ as i32, "XFSZ");
        push(&mut keys, libc::SIGVTALRM as i32, "VTALRM");
        push(&mut keys, libc::SIGPROF as i32, "PROF");
        push(&mut keys, libc::SIGIO as i32, "IO");
        push(&mut keys, libc::SIGSYS as i32, "SYS");
        push(&mut keys, libc::SIGURG as i32, "URG");
        push(&mut keys, libc::SIGWINCH as i32, "WINCH");
        push(&mut keys, libc::SIGTSTP as i32, "TSTP");
        push(&mut keys, libc::SIGTTIN as i32, "TTIN");
        push(&mut keys, libc::SIGTTOU as i32, "TTOU");
        push(&mut keys, libc::SIGCONT as i32, "CONT");
        extend_trap_keys(&mut keys);
        keys
    })
}

/// Linux/Android-only trap-key additions: SIGSTKFLT and SIGPWR plus
/// the realtime block, spelled with the nearer-anchor form the SIGNALS
/// table (and GNU signames.c) uses — RTMIN+k up to the midpoint,
/// RTMAX-m above it.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn extend_trap_keys(keys: &mut Vec<(i32, String, String)>) {
    keys.push((
        libc::SIGSTKFLT as i32,
        "STKFLT".to_string(),
        "__RUBASH_TRAP_SIGSTKFLT".to_string(),
    ));
    keys.push((
        libc::SIGPWR as i32,
        "PWR".to_string(),
        "__RUBASH_TRAP_SIGPWR".to_string(),
    ));
    let lo = libc::SIGRTMIN();
    let hi = libc::SIGRTMAX();
    if lo > 0 && hi >= lo {
        for n in lo..=hi {
            let k = n - lo;
            let m = hi - n;
            let short = if k == 0 {
                "RTMIN".to_string()
            } else if m == 0 {
                "RTMAX".to_string()
            } else if k <= m {
                format!("RTMIN+{k}")
            } else {
                format!("RTMAX-{m}")
            };
            keys.push((n, short.clone(), format!("__RUBASH_TRAP_SIG{short}")));
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
#[cfg_attr(not(unix), allow(dead_code))]
fn extend_trap_keys(_keys: &mut Vec<(i32, String, String)>) {}

/// rubash#2347: the trap builtin's REVERT special case for an
/// entry-ignored SIGINT (builtins/trap.def:228-251). Called from
/// trap.rs's `trap - SIGINT` path, which remove_trap has already
/// (correctly, per the hard-ignore rule) treated as a table no-op;
/// this is the disposition half GNU's builtin performs anyway.
#[cfg(unix)]
pub fn revert_entry_ignored_sigint(interactive: bool) {
    kernel_signals::revert_sigint(interactive);
}

/// Whether the trap-table key `key` names a signal that was ignored on
/// process entry. The key suffix is the trap short name; the managed set
/// resolves through managed_trap_keys + the managed entry mask, the
/// fatal quartet through the all-signal entry probe
/// (kernel_signals::entry_ignored_mask_all). An unrecognized suffix is
/// NOT entry-ignored (a user-forged key stays conservatively armed,
/// exactly the pre-rubash#2630 verdict).
#[cfg(unix)]
fn trap_key_is_entry_ignored(key: &str) -> bool {
    let Some(short) = key.strip_prefix("__RUBASH_TRAP_SIG") else {
        return false;
    };
    if let Some((sig, _, _)) = managed_trap_keys()
        .iter()
        .find(|(_, name, _)| name == short)
    {
        return kernel_signals::entry_ignored(*sig);
    }
    let sig = match short {
        "SEGV" => libc::SIGSEGV,
        "BUS" => libc::SIGBUS,
        "ILL" => libc::SIGILL,
        "FPE" => libc::SIGFPE,
        _ => return false,
    };
    kernel_signals::entry_ignored_mask_all() & (1u64 << (sig as u64 - 1)) != 0
}

/// Whether the trap-table entry under `key` is a LIVE trap the slow
/// reconcile must apply (rubash#2630's table-side baseline test). A
/// missing key is unarmed; a non-empty action is armed; an empty action
/// is a user's `trap '' SIG` and stays armed — EXCEPT when the signal
/// was ignored on entry: seed_startup_traps materializes exactly such
/// an entry for every entry-ignored signal (GNU trap.c:154's
/// original_signals baseline in table form), and its wish (Ignore) IS
/// the entry disposition, so counting it as armed made the boundary
/// fast path unreachable whenever the shell was launched with ignored
/// signals. The distinction is safe to draw here because the fast path
/// also requires the recorded state words to equal the entry baseline
/// (kernel_signals::at_baseline_state): a baseline-looking table whose
/// kernel state has not converged still takes the slow path.
#[cfg(unix)]
fn trap_entry_is_armed(env_vars: &std::collections::HashMap<String, String>, key: &str) -> bool {
    match env_vars.get(key) {
        None => false,
        Some(action) if !action.is_empty() => true,
        Some(_) => !trap_key_is_entry_ignored(key),
    }
}

/// Map the executor's trap table onto the kernel dispositions (unix arm of
/// rubash#226). Mirrors GNU's per-signal rules: `trap 'action' SIG` installs
/// the handler (sig.c:830, sa_flags=0), `trap '' SIG` sets SIG_IGN, no trap
/// restores the entry disposition (SIG_DFL for a plain script). Called from
/// the `trap` builtin's mutation paths and the command-boundary poll so the
/// kernel state can never lag the trap table by more than one command.
#[cfg(unix)]
pub fn reconcile_kernel_trap_dispositions(env_vars: &std::collections::HashMap<String, String>) {
    use kernel_signals::Disposition;
    // Steady-state fast path (rubash#437). The overwhelming majority of
    // boundary polls run with no managed signal trapped at all, so every
    // disposition sits at the entry baseline (Default, or Ignore for
    // entry-ignored signals — rubash#2630) and the kernel already holds
    // it. Detect
    // that with six allocation-free `get`s — using the static key table so
    // no `format!` runs — and skip the disposition array + mutex entirely.
    // A non-empty reset list forces the slow path even when no key is
    // present, because a reset entry can only exist while a key does; the
    // reverse (key present, no reset) is the common trapped case and falls
    // through anyway.
    let no_reset = !env_vars.contains_key("__RUBASH_TRAP_RESET");
    // Any armed signal trap forces the slow path. Every trap-table key
    // for a signal shares the __RUBASH_TRAP_SIG prefix — the managed
    // set's and the fatal quartet's alike (rubash#1475: the pair's
    // routing bits live outside the managed state, but their keys must
    // still defeat this fast path). Two detections of the same verdict,
    // chosen by table size (rubash#2062): on a small table a prefix
    // pass over the keys is cheaper than hashing ~55 probes; on a
    // large table that pass is Θ(#variables) at every command
    // boundary — Θ(n²) over a script that creates n variables — so
    // probe the fixed key universe instead: the managed table itself
    // (the same generative table the slow path iterates, so the probe
    // set cannot fall behind it the way the rubash#1511-era hand list
    // did) plus the fatal quartet's literal keys, which
    // reconcile_fatal consults below. Those are the only
    // __RUBASH_TRAP_SIG* keys with a reader: a user-forged key outside
    // the universe changes no disposition on either path (the slow
    // path never consults it), so the verdict is unchanged.
    let any_armed = if env_vars.len() <= 512 {
        env_vars
            .keys()
            .filter(|key| key.starts_with("__RUBASH_TRAP_SIG"))
            .any(|key| trap_entry_is_armed(env_vars, key))
    } else {
        managed_trap_keys()
            .iter()
            .any(|(_, _, key)| trap_entry_is_armed(env_vars, key.as_str()))
            || trap_entry_is_armed(env_vars, "__RUBASH_TRAP_SIGSEGV")
            || trap_entry_is_armed(env_vars, "__RUBASH_TRAP_SIGBUS")
            || trap_entry_is_armed(env_vars, "__RUBASH_TRAP_SIGILL")
            || trap_entry_is_armed(env_vars, "__RUBASH_TRAP_SIGFPE")
    };
    // An armed EXIT trap also forces the slow path: it routes every
    // otherwise-untrapped managed signal through the handler
    // (sig.c:226-229, rubash#1332 — see the disposition loop below).
    let exit_armed = env_vars
        .get("__RUBASH_TRAP_EXIT")
        .is_some_and(|action| !action.is_empty());
    // rubash#1662: an interactive shell's untrapped SIGINT is pinned to
    // the mailbox handler (see the disposition loop below), so the
    // all-Default steady state is never the interactive steady state --
    // the fast path must not skip the pin's first application.
    let interactive = env_vars.get("__RUBASH_INTERACTIVE").map(String::as_str) == Some("1");
    if !any_armed
        && !exit_armed
        && !interactive
        && no_reset
        && kernel_signals::at_baseline_state()
        && kernel_signals::fatal_at_default()
    {
        // No LIVE trap keys exist on this path (only seeded
        // entry-ignored baseline entries may be present), so the
        // trap '' record can hold at most entry-ignored bits — and
        // those are restored for fork children from the entry snapshot
        // itself (pre_exec_child_dispositions consults ENTRY_IGNORED
        // before TRAP_IGNORED), never from this record (rubash#1991,
        // rubash#2630).
        kernel_signals::clear_trap_ignored_all();
        return;
    }
    // G-01: the slow path mutates process-wide kernel dispositions —
    // serialize it against sibling sessions' reconciles and env
    // mirror writes. The fast path above reads only atomics and
    // stays lock-free.
    let _process_state = crate::executor::local_helpers::acquire_process_state();
    let reset = env_vars
        .get("__RUBASH_TRAP_RESET")
        .map(|value| value.as_str())
        .unwrap_or("");
    // GNU sig.c:226-229: an EXIT trap forces
    // initialize_terminating_signals — `if (signal_is_trapped (EXIT))
    // set_signal_handler for every terminating signal` — even in a
    // non-interactive shell. Without it an untrapped SIGTERM kills the
    // process at the kernel and the EXIT trap never runs; with it the
    // delivery queues here and the boundary dispatch turns it into the
    // 128+signal exit, whose top-level path runs the EXIT trap first
    // (rubash#1332).
    let exit_armed = env_vars
        .get("__RUBASH_TRAP_EXIT")
        .is_some_and(|action| !action.is_empty())
        && !reset_contains(reset, "EXIT");
    let mut dispositions: Vec<(i32, Disposition)> = Vec::new();
    for (sig, short, key) in managed_trap_keys() {
        // rubash#1991: keep the trap '' record in step with the table
        // this reconcile is applying — it is the ONLY ignore source the
        // fork-child restore may consult besides the entry snapshot
        // (see kernel_signals::TRAP_IGNORED).
        kernel_signals::set_trap_ignored(
            *sig,
            matches!(env_vars.get(key.as_str()), Some(action) if action.is_empty())
                && !reset_contains(reset, short),
        );
        let want = match env_vars.get(key.as_str()) {
            // rubash#2347: a reverted entry-ignored SIGINT
            // (trap.def:228-251) outranks the table: the seeded
            // entry-ignore entry (an "" action) must not re-impose
            // Ignore on every boundary reconcile. Non-interactive it
            // sits at the fatal default; interactive it rides the
            // SIGINT pin (GNU's sigint_sighandler).
            _ if *sig == libc::SIGINT as i32 && kernel_signals::sigint_reverted() => {
                if interactive {
                    Disposition::Handler
                } else {
                    Disposition::Default
                }
            }
            // A real action and not reset-listed -> handler (trap.c
            // trap_builtin -> sig.c:830 set_signal_handler).
            Some(action) if !action.is_empty() && !reset_contains(reset, short) => {
                Disposition::Handler
            }
            // Reset-listed (subshell reset, trap.c:1480 reset_signal_handlers
            // restores the original disposition) behaves like no trap.
            Some(_) if reset_contains(reset, short) => Disposition::Default,
            // trap '' SIG: ignored (trap.def ignore path, kernel SIG_IGN).
            Some(_) => Disposition::Ignore,
            // No trap of its own: an armed EXIT trap still routes the
            // signal through the handler (sig.c:226-229 above) — but
            // only for GNU's terminating set (see exit_reroutes).
            None if exit_armed && exit_reroutes(*sig) => Disposition::Handler,
            // rubash#1662: GNU sig.c:315-316 -- initialize_terminating_
            // signals is interactive-only: an interactive shell catches
            // SIGINT itself from startup, independent of the trap table.
            // Without the pin, an untrapped interactive SIGINT sits at
            // SIG_DFL in every window the foreground guards do not cover
            // (the prompt reader, in-process builtin execution), and the
            // kernel default action kills the shell on the first ^C.
            // The mailbox delivery becomes the boundary dispatch's
            // 128+SIGINT abort instead. Entry-ignored SIGINT stays
            // ignored (the pin must not un-ignore it).
            None if interactive
                && *sig == libc::SIGINT as i32
                && (!kernel_signals::entry_ignored(*sig) || kernel_signals::sigint_reverted()) =>
            {
                Disposition::Handler
            }
            // rubash#2233: SIGHUP is in the same terminating set
            // (sig.c:128) and the same interactive install: GNU's
            // interactive shell CATCHES a hangup so termsig_handler
            // (sig.c:590) can save history, hang up every job
            // (jobs.c:1783 hangup_all_jobs) and only then die BY the
            // signal (kill_shell restores SIG_DFL and re-raises). At
            // SIG_DFL the kernel kills the shell in place — no job
            // hangup, no EXIT trap, no history save. The mailbox pin
            // routes the delivery to the boundary dispatch, whose
            // untrapped branch records the hangup-death mark for
            // finish_shell's funnel. Entry-ignored SIGHUP stays
            // ignored (the pin must not un-ignore it), exactly like
            // the SIGINT pin above.
            None if interactive
                && *sig == libc::SIGHUP as i32
                && !kernel_signals::entry_ignored(*sig) =>
            {
                Disposition::Handler
            }
            // The same sig.c block: an interactive shell also ignores
            // SIGTERM (initialize_shell_signals: `if (interactive) ...
            // set_signal_handler (SIGTERM, SIG_IGN)`), so a directed
            // TERM cannot kill it either. An armed EXIT trap still wins
            // via the reroute arm above (Handler), and a user trap on
            // TERM is unaffected (the Some arms).
            None if interactive
                && *sig == libc::SIGTERM as i32
                && !kernel_signals::entry_ignored(*sig) =>
            {
                Disposition::Ignore
            }
            // rubash#2836(i): GNU jobs.c:4949 initialize_job_signals
            // — an interactive shell ignores the stop class
            // (SIGTSTP/SIGTTOU/SIGTTIN) so ^Z at an empty prompt
            // cannot stop the shell itself; the terminal delivery
            // still reaches the foreground job's own process group.
            // Non-interactive shells keep SIG_DFL (the else-if
            // job_control arm is #2836(ii) territory, not this
            // slice). A user trap still wins via the Some arms
            // above, and a reset still restores Default via the
            // reset arm above. The ignore is a shell-policy
            // disposition like the interactive SIGTERM pin: it is
            // NOT recorded in TRAP_IGNORED, so
            // pre_exec_child_dispositions restores SIG_DFL for
            // children (trap.c:1489 restore_original_signals).
            None if interactive && kernel_signals::is_stop_class(*sig) => Disposition::Ignore,
            None => Disposition::Default,
        };
        dispositions.push((*sig, want));
    }
    kernel_signals::reconcile(&dispositions);
    // rubash#1475: mirror the fatal set's trap state into the fatal
    // handler's routing bits, with the same mapping as the managed set
    // (a reset-listed fatal trap behaves like no trap; an armed EXIT
    // trap does NOT reroute SIGSEGV/SIGBUS — GNU's sig.c:226-229
    // applies to the terminating-signal handler set, and a fatal
    // signal's default death already runs the EXIT trap through the
    // shell's exit path. SIGILL/SIGFPE below are the exception: GNU's
    // terminating set names them, so they use the rerouting mapping).
    let fatal_want = |short: &str, key: &str| match env_vars.get(key) {
        Some(action) if !action.is_empty() && !reset_contains(reset, short) => Disposition::Handler,
        Some(_) if reset_contains(reset, short) => Disposition::Default,
        Some(_) => Disposition::Ignore,
        None => Disposition::Default,
    };
    // rubash#1511: SIGILL/SIGFPE ride the fatal backend (signal-hook
    // refuses them), but unlike the pair they DO reroute under an
    // armed EXIT trap — GNU's terminating set names them (oracle:
    // EXIT trap + untrapped kill -ILL runs the EXIT trap, dies 132).
    let fatal_want_rerouted = |short: &str, key: &str| match env_vars.get(key) {
        Some(action) if !action.is_empty() && !reset_contains(reset, short) => Disposition::Handler,
        Some(_) if reset_contains(reset, short) => Disposition::Default,
        Some(_) => Disposition::Ignore,
        None if exit_armed => Disposition::Handler,
        None => Disposition::Default,
    };
    kernel_signals::reconcile_fatal(&[
        (
            libc::SIGSEGV as i32,
            fatal_want("SEGV", "__RUBASH_TRAP_SIGSEGV"),
        ),
        (
            libc::SIGBUS as i32,
            fatal_want("BUS", "__RUBASH_TRAP_SIGBUS"),
        ),
        (
            libc::SIGILL as i32,
            fatal_want_rerouted("ILL", "__RUBASH_TRAP_SIGILL"),
        ),
        (
            libc::SIGFPE as i32,
            fatal_want_rerouted("FPE", "__RUBASH_TRAP_SIGFPE"),
        ),
    ]);
}

/// Whether an armed EXIT trap reroutes `sig` through the handler
/// (sig.c:226-229 initialize_terminating_signals). GNU's terminating
/// set is a compile-time C table (sig.c:127 terminating_signals[]), so
/// it cannot name the realtime signals, and it excludes the
/// default-ignore pair — oracle-verified against GNU 5.3.20 (EXIT trap
/// armed, untrapped self-kill): ILL, FPE, TRAP, ABRT, ALRM, XCPU, PROF
/// and SYS run the EXIT trap and die at 128+n, while URG and WINCH are
/// ignored outright and RTMIN dies without the EXIT trap running.
/// The same table names no stop-class signal and no SIGCONT — their
/// default actions are stop/resume, not termination — so an armed EXIT
/// trap must not reroute them either: an untrapped TSTP still stops
/// the shell (rubash#2344). rubash#1511.
#[cfg(unix)]
pub(crate) fn exit_reroutes(sig: i32) -> bool {
    if sig == libc::SIGURG as i32
        || sig == libc::SIGWINCH as i32
        || kernel_signals::is_stop_class(sig)
        || sig == libc::SIGCONT as i32
    {
        return false;
    }
    exit_reroutes_platform(sig)
}

/// The realtime half of exit_reroutes: on Linux/Android the RT block
/// sits at SIGRTMIN() and above and never reroutes; elsewhere there
/// is no RT block to exclude.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn exit_reroutes_platform(sig: i32) -> bool {
    sig < libc::SIGRTMIN()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
#[cfg_attr(not(unix), allow(dead_code))]
fn exit_reroutes_platform(_sig: i32) -> bool {
    true
}

#[cfg(unix)]
fn reset_contains(reset: &str, short: &str) -> bool {
    // Reset entries carry the table's SIG-prefixed name (trap.rs
    // normalize_signal keeps the SIG prefix, e.g. "SIGTERM"). Compare
    // without allocating: iterate the colon-separated entries and match
    // "SIG" + short by parts.
    reset.split(':').any(|entry| {
        entry.len() == short.len() + 3 && entry.starts_with("SIG") && entry[3..] == *short
    })
}

/// Whether `sig` is currently caught by the kernel backend (unix arm). A
/// pending entry whose signal is not caught is stale (see
/// kernel_signals::handler_installed) and must be dropped, not dispatched.
#[cfg(unix)]
pub fn kernel_handler_installed(signal: i32) -> bool {
    kernel_signals::handler_installed(signal)
}

/// Whether `sig` is in the stop class (SIGTSTP/SIGTTIN/SIGTTOU; see
/// kernel_signals::is_stop_class, rubash#2344).
#[cfg(unix)]
pub fn kernel_is_stop_class(signal: i32) -> bool {
    kernel_signals::is_stop_class(signal)
}

/// Whether the most recently drained kernel delivery of `signal` was
/// kernel-caused (terminal-generated, si_code SI_KERNEL) rather than
/// sent by a kill(2) from some process (rubash#1034; see
/// kernel_signals::KERNEL_CAUSED).
#[cfg(unix)]
pub fn last_delivery_kernel_caused(signal: i32) -> bool {
    kernel_signals::last_delivery_kernel_caused(signal)
}

/// Sender pid of the most recently drained kill(2) delivery of
/// `signal`, when one was recorded (rubash#2241; see
/// kernel_signals::LAST_SENDER_PID).
#[cfg(unix)]
pub fn last_delivery_sender_pid(signal: i32) -> Option<u32> {
    kernel_signals::last_delivery_sender_pid(signal)
}

/// Monotonic count of kernel-caused SIGINT deliveries drained so far
/// (rubash#1990) — see kernel_signals::SIGINT_KERNEL_EPOCH.
#[cfg(unix)]
pub fn sigint_kernel_epoch() -> u64 {
    kernel_signals::sigint_kernel_epoch()
}

/// Whether the boundary dispatcher can adjudicate `signal` (managed or
/// fatal set) — see kernel_signals::adjudicable (rubash#1654).
#[cfg(unix)]
pub fn boundary_adjudicable_signal(signal: i32) -> bool {
    kernel_signals::adjudicable(signal)
}

/// rubash#1250: hold untrapped SIGINT at the mailbox handler across a
/// foreground builtin wait (the `sleep` fast path). While held, the
/// reconcile above will not route SIGINT back to Default.
#[cfg(unix)]
pub fn hold_fg_sigint() {
    kernel_signals::hold_fg_sigint()
}

/// Block SIGPIPE on the current thread until the returned guard drops
/// (see kernel_signals::SigpipeBlockGuard for the model).
#[cfg(unix)]
pub fn sigpipe_block_guard() -> kernel_signals::SigpipeBlockGuard {
    kernel_signals::sigpipe_block_guard()
}

/// Block SIGPIPE on the calling thread permanently (writer threads).
#[cfg(unix)]
pub fn block_sigpipe_on_this_thread() {
    kernel_signals::block_sigpipe_on_this_thread()
}

/// rubash#2893: child pre-exec for coproc spawns — undo any SIGPIPE
/// block in force on the spawning thread so it cannot cross the exec.
/// A coproc created while its command runs as a pipeline stage spawns
/// under the stage's sigpipe_block_guard; GNU's forked coproc child
/// (execute_cmd.c:2477 execute_coproc -> make_child) carries the
/// shell's ordinary disposition (SIGPIPE unblocked, SIG_DFL), so a
/// write to a reader-less coproc pipe kills the child silently
/// instead of surfacing a `Broken pipe' diagnostic from the child
/// shell. Only the mask is touched: the respawned child's own startup
/// re-seeds its dispositions, and a foreign (direct-plan) child gets
/// exactly the ambient program environment GNU would give it.
#[cfg(unix)]
pub fn child_pre_exec_unblock_sigpipe() -> std::io::Result<()> {
    kernel_signals::block_sigpipe_on_this_thread_unblock();
    Ok(())
}

/// rubash#1250: release the foreground SIGINT hold (restores the
/// steady state the hold found — Default, or the rubash#1662
/// interactive pin's Handler routing; rubash#1824).
#[cfg(unix)]
pub fn release_fg_sigint() {
    kernel_signals::release_fg_sigint()
}

#[cfg(all(test, unix))]
mod kernel_signal_tests {
    use crate::signals::mailbox::{register_signal_mailbox, take_pending_signals};
    #[test]
    fn kernel_signal_reaches_pending_queue() {
        register_signal_mailbox(std::process::id()).unwrap();
        // rubash#226 model: no handler exists until a trap names the signal
        // (GNU leaves non-interactive terminating signals SIG_DFL), so the
        // test must install one first or the raise below would kill the test
        // process through the kernel default action.
        let sig = libc::SIGUSR1 as i32;
        super::kernel_signals::reconcile(&[(sig, super::kernel_signals::Disposition::Handler)]);
        assert!(super::kernel_signals::handler_installed(sig));
        unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGUSR1) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if take_pending_signals(std::process::id())
                .unwrap()
                .contains(&libc::SIGUSR1)
            {
                // Restore the default disposition so later tests in this
                // process are not affected by the caught handler.
                super::kernel_signals::reconcile(&[(
                    sig,
                    super::kernel_signals::Disposition::Default,
                )]);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("SIGUSR1 did not reach the pending-signal queue");
    }
}
