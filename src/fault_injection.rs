//! Deterministic (seed-reproducible) fault-injection harness for testing
//! `rubash` resilience across four failure arms (rubash#2811):
//!
//! 1. Nth spawn failure (`NotFound`, `PermissionDenied`, `WouldBlock`, etc.)
//! 2. fd-table watermark / saturation (constrained `RLIMIT_NOFILE` soft limit)
//! 3. `EINTR` storm on read / wait syscall boundaries
//! 4. Write failure (`BrokenPipe`, `ENOSPC`, `EIO`)
//!
//! In production release builds (`not(any(test, debug_assertions))`), every
//! hook compiles to an `#[inline(always)]` no-op with zero runtime state or
//! branches.

#[cfg(any(test, debug_assertions))]
mod active {
    use std::cell::RefCell;
    use std::io;

    /// Deterministic SplitMix64 PRNG so every seed produces an identical
    /// fault schedule across platforms and Rust versions.
    #[derive(Debug, Clone)]
    pub struct SplitMix64 {
        state: u64,
    }

    impl SplitMix64 {
        pub fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        pub fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        pub fn next_usize_range(&mut self, lo: usize, hi_inclusive: usize) -> usize {
            debug_assert!(lo <= hi_inclusive);
            let span = (hi_inclusive - lo) as u64 + 1;
            lo + (self.next_u64() % span) as usize
        }
    }

    /// Which of the 4 fault-injection arms a seeded plan targets.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FaultArm {
        SpawnFailure,
        FdWatermark,
        EintrStorm,
        WriteFailure,
    }

    /// Write error flavors supported by Arm 4.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WriteFaultKind {
        BrokenPipe,
        Enospc,
        Eio,
    }

    impl WriteFaultKind {
        pub fn to_io_error(self) -> io::Error {
            match self {
                WriteFaultKind::BrokenPipe => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "Broken pipe")
                }
                WriteFaultKind::Enospc => {
                    // Matches rubash#1898's /dev/full ENOSPC payload so
                    // sh_chkwrite renders "No space left on device" on all platforms.
                    io::Error::new(io::ErrorKind::Other, "No space left on device")
                }
                WriteFaultKind::Eio => io::Error::new(io::ErrorKind::Other, "Input/output error"),
            }
        }
    }

    /// Configuration for a deterministic fault-injection run.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FaultPlan {
        /// Arm 1: 1-based spawn index that fails (None = disabled).
        pub spawn_fail_on: Option<usize>,
        pub spawn_fail_kind: io::ErrorKind,
        pub spawn_fail_message: &'static str,

        /// Arm 2: simulated RLIMIT_NOFILE soft limit watermark (None = disabled).
        pub fd_watermark: Option<u32>,

        /// Arm 3: consecutive `EINTR` errors to inject per read/wait boundary,
        /// capped by `eintr_max_total` so every storm terminates deterministically.
        pub eintr_burst_per_op: usize,
        pub eintr_max_total: usize,

        /// Arm 4: 1-based write index that fails (None = disabled).
        pub write_fail_on: Option<usize>,
        pub write_fail_kind: WriteFaultKind,
        /// Optional fd filter (e.g. `Some(1)` so stderr diagnostics are spared).
        pub write_only_fd: Option<u32>,
    }

    impl Default for FaultPlan {
        fn default() -> Self {
            Self {
                spawn_fail_on: None,
                spawn_fail_kind: io::ErrorKind::NotFound,
                spawn_fail_message: "injected spawn failure",
                fd_watermark: None,
                eintr_burst_per_op: 0,
                eintr_max_total: 0,
                write_fail_on: None,
                write_fail_kind: WriteFaultKind::BrokenPipe,
                write_only_fd: None,
            }
        }
    }

    impl FaultPlan {
        /// Generate a deterministic plan for `arm` from `seed`.
        pub fn seeded(seed: u64, arm: FaultArm) -> Self {
            let mut rng = SplitMix64::new(seed);
            let mut plan = Self::default();
            match arm {
                FaultArm::SpawnFailure => {
                    plan.spawn_fail_on = Some(rng.next_usize_range(1, 3));
                    let kinds = [
                        (io::ErrorKind::NotFound, "No such file or directory"),
                        (io::ErrorKind::PermissionDenied, "Permission denied"),
                        (
                            io::ErrorKind::WouldBlock,
                            "Resource temporarily unavailable",
                        ),
                    ];
                    let idx = rng.next_usize_range(0, kinds.len() - 1);
                    plan.spawn_fail_kind = kinds[idx].0;
                    plan.spawn_fail_message = kinds[idx].1;
                }
                FaultArm::FdWatermark => {
                    let watermarks = [6u32, 10, 11, 12, 14];
                    let idx = rng.next_usize_range(0, watermarks.len() - 1);
                    plan.fd_watermark = Some(watermarks[idx]);
                }
                FaultArm::EintrStorm => {
                    plan.eintr_burst_per_op = rng.next_usize_range(2, 8);
                    plan.eintr_max_total = rng.next_usize_range(12, 48);
                }
                FaultArm::WriteFailure => {
                    plan.write_fail_on = Some(rng.next_usize_range(1, 2));
                    let kinds = [
                        WriteFaultKind::BrokenPipe,
                        WriteFaultKind::Enospc,
                        WriteFaultKind::Eio,
                    ];
                    let idx = rng.next_usize_range(0, kinds.len() - 1);
                    plan.write_fail_kind = kinds[idx];
                    plan.write_only_fd = Some(1);
                }
            }
            plan
        }
    }

    /// Recorded fault-injection telemetry for determinism assertions.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct FaultTrace {
        pub spawn_calls: usize,
        pub spawn_faults: usize,
        pub fd_limit_queries: usize,
        pub fd_limit_clamps: usize,
        pub eintr_read_injections: usize,
        pub eintr_wait_injections: usize,
        pub write_calls: usize,
        pub write_faults: usize,
        pub events: Vec<String>,
    }

    #[derive(Debug, Default)]
    struct InjectorState {
        plan: Option<FaultPlan>,
        trace: FaultTrace,
        eintr_burst_remaining: usize,
    }

    impl InjectorState {
        fn with_plan(plan: FaultPlan) -> Self {
            let burst = plan.eintr_burst_per_op;
            Self {
                plan: Some(plan),
                trace: FaultTrace::default(),
                eintr_burst_remaining: burst,
            }
        }
    }

    thread_local! {
        static INJECTOR: RefCell<InjectorState> = RefCell::new(InjectorState::default());
    }

    /// RAII guard installing a `FaultPlan` on the current thread and restoring
    /// the prior state on drop.
    pub struct FaultScope {
        previous: InjectorState,
    }

    impl FaultScope {
        pub fn install(plan: FaultPlan) -> Self {
            let previous = INJECTOR.with(|cell| {
                let mut guard = cell.borrow_mut();
                std::mem::replace(&mut *guard, InjectorState::with_plan(plan))
            });
            Self { previous }
        }

        pub fn trace(&self) -> FaultTrace {
            INJECTOR.with(|cell| cell.borrow().trace.clone())
        }
    }

    impl Drop for FaultScope {
        fn drop(&mut self) {
            INJECTOR.with(|cell| {
                let mut guard = cell.borrow_mut();
                *guard = std::mem::take(&mut self.previous);
            });
        }
    }

    pub(crate) fn maybe_inject_spawn_error() -> io::Result<()> {
        INJECTOR.with(|cell| {
            let mut state = cell.borrow_mut();
            let Some(plan) = state.plan.as_ref().cloned() else {
                return Ok(());
            };
            state.trace.spawn_calls += 1;
            let call_idx = state.trace.spawn_calls;
            if plan.spawn_fail_on == Some(call_idx) {
                state.trace.spawn_faults += 1;
                state
                    .trace
                    .events
                    .push(format!("spawn#{call_idx}:{:?}", plan.spawn_fail_kind));
                return Err(io::Error::new(
                    plan.spawn_fail_kind,
                    plan.spawn_fail_message,
                ));
            }
            Ok(())
        })
    }

    pub(crate) fn spawn_command(
        cmd: &mut std::process::Command,
    ) -> io::Result<std::process::Child> {
        maybe_inject_spawn_error()?;
        cmd.spawn()
    }

    pub(crate) fn clamp_open_files_limit(limit: Option<u32>) -> Option<u32> {
        INJECTOR.with(|cell| {
            let mut state = cell.borrow_mut();
            let Some(watermark) = state.plan.as_ref().and_then(|p| p.fd_watermark) else {
                return limit;
            };
            state.trace.fd_limit_queries += 1;
            let clamped = match limit {
                Some(current) => current.min(watermark),
                None => watermark,
            };
            if Some(clamped) != limit {
                state.trace.fd_limit_clamps += 1;
                state
                    .trace
                    .events
                    .push(format!("fd_limit:{limit:?}->{clamped}"));
            }
            Some(clamped)
        })
    }

    fn maybe_inject_eintr(is_read: bool) -> io::Result<()> {
        INJECTOR.with(|cell| {
            let mut state = cell.borrow_mut();
            let Some(plan) = state.plan.as_ref().cloned() else {
                return Ok(());
            };
            if plan.eintr_burst_per_op == 0 || plan.eintr_max_total == 0 {
                return Ok(());
            }
            let total_eintr = state.trace.eintr_read_injections + state.trace.eintr_wait_injections;
            if total_eintr >= plan.eintr_max_total {
                return Ok(());
            }
            if state.eintr_burst_remaining > 0 {
                state.eintr_burst_remaining -= 1;
                if is_read {
                    state.trace.eintr_read_injections += 1;
                    let n = state.trace.eintr_read_injections;
                    state.trace.events.push(format!("eintr:read#{n}"));
                } else {
                    state.trace.eintr_wait_injections += 1;
                    let n = state.trace.eintr_wait_injections;
                    state.trace.events.push(format!("eintr:wait#{n}"));
                }
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Interrupted system call",
                ));
            }
            // Burst finished for this operation; arm the next operation's burst.
            state.eintr_burst_remaining = plan.eintr_burst_per_op;
            Ok(())
        })
    }

    pub(crate) fn maybe_inject_read_eintr() -> io::Result<()> {
        maybe_inject_eintr(true)
    }

    pub(crate) fn maybe_inject_wait_eintr() -> io::Result<()> {
        maybe_inject_eintr(false)
    }

    pub(crate) fn maybe_inject_write_fault(fd: Option<u32>) -> io::Result<()> {
        INJECTOR.with(|cell| {
            let mut state = cell.borrow_mut();
            let Some(plan) = state.plan.as_ref().cloned() else {
                return Ok(());
            };
            let Some(fail_on) = plan.write_fail_on else {
                return Ok(());
            };
            if let (Some(target_fd), Some(actual_fd)) = (plan.write_only_fd, fd) {
                if target_fd != actual_fd {
                    return Ok(());
                }
            }
            state.trace.write_calls += 1;
            let call_idx = state.trace.write_calls;
            if call_idx == fail_on {
                state.trace.write_faults += 1;
                state.trace.events.push(format!(
                    "write#{call_idx}:fd={fd:?}:{:?}",
                    plan.write_fail_kind
                ));
                return Err(plan.write_fail_kind.to_io_error());
            }
            Ok(())
        })
    }
}

#[cfg(any(test, debug_assertions))]
pub(crate) use active::{
    clamp_open_files_limit, maybe_inject_read_eintr, maybe_inject_spawn_error,
    maybe_inject_wait_eintr, maybe_inject_write_fault, spawn_command,
};
#[cfg(any(test, debug_assertions))]
pub use active::{FaultArm, FaultPlan, FaultScope, FaultTrace, SplitMix64, WriteFaultKind};

#[cfg(not(any(test, debug_assertions)))]
mod disabled {
    use std::io;

    #[inline(always)]
    pub(crate) fn maybe_inject_spawn_error() -> io::Result<()> {
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn spawn_command(
        cmd: &mut std::process::Command,
    ) -> io::Result<std::process::Child> {
        cmd.spawn()
    }

    #[inline(always)]
    pub(crate) fn clamp_open_files_limit(limit: Option<u32>) -> Option<u32> {
        limit
    }

    #[inline(always)]
    pub(crate) fn maybe_inject_read_eintr() -> io::Result<()> {
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn maybe_inject_wait_eintr() -> io::Result<()> {
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn maybe_inject_write_fault(_fd: Option<u32>) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(not(any(test, debug_assertions)))]
pub(crate) use disabled::{
    clamp_open_files_limit, maybe_inject_read_eintr, maybe_inject_spawn_error,
    maybe_inject_wait_eintr, maybe_inject_write_fault, spawn_command,
};
