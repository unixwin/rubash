//! Wait-status decoding for reaped children.
//!
//! GNU jobs.c:2958 process_exit_status: WIFSIGNALED -> 128 + WTERMSIG,
//! otherwise WEXITSTATUS. The WIFSTOPPED arm is unreachable through Rust's
//! Child::wait (it waits without WUNTRACED, so a stopped child simply keeps
//! the caller waiting). On Windows the raw exit code is the entire status:
//! rubash's own TerminateProcess kills already encode 128+signal
//! (builtins/kill.rs signal_process), so `code()` is the faithful decode and
//! this helper is the single place a future NTSTATUS -> signal mapping
//! would land.

pub(crate) fn process_exit_status(status: &std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    status.code().unwrap_or(1)
}

/// WTERMSIG of a reaped child, or None when it exited normally. This is the
/// cfg bridge for `ExitStatus::signal` (a unix-only trait method) so callers
/// like the rubash#229 signal-death notices stay cross-platform.
pub(crate) fn exit_status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        return status.signal();
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The 128+signal exit-code decode of a reaped child, or None.
///
/// Windows-only in effect (unix returns None: the kernel wait status
/// is authoritative there). Windows has no WTERMSIG, but rubash's own
/// kills already encode 128+signal in the exit code
/// (builtins/kill.rs `signal_process` → TerminateProcess(128+signal)),
/// and the boundary reap already decodes exactly this convention
/// (job_builtins.rs `apply_job_completion`: `status > 128` →
/// `status - 128`). A `wait` that reaps the child itself needs the same
/// decode (rubash#3803): the immediate-wait face
/// (`sleep 30 & kill -9 %1; wait %1`) loses the race in the boundary
/// sweep — the sweep's `try_wait` runs microseconds after the kill, the
/// poll loop below does the reap — and with `exit_status_signal`
/// returning None on Windows the notice was never queued, so `wait`
/// printed nothing while GNU prints the death notice (jobs.c:3385
/// `wait_for` → `notify_and_cleanup`, jobs.c:4625
/// `notify_of_job_status`).
///
/// Deliberately NOT folded into `exit_status_signal`: the other
/// callers of that helper (jobs-table `term_signal`, the SIGINT-unwind
/// sites, the #1604 `fresh_fatal` gate) must keep seeing None on
/// Windows — their Windows wording/timing faces are separate issues,
/// and decoding there would silently re-render the jobs table
/// (`Exit 137` → `Killed`) and re-time the #1604 stash. Only the
/// wait-reap notice path uses this decode, mirroring the boundary
/// reap's already-established convention — including its accepted
/// ambiguity (a natural `exit 137` reads as signal 9, exactly as the
/// boundary reap already reports it).
pub(crate) fn exit_code_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        let _ = status;
        None
    }
    #[cfg(not(unix))]
    {
        let code = status.code()?;
        if code > 128 {
            Some(code - 128)
        } else {
            None
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn run_sh(script: &str) -> std::process::ExitStatus {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .status()
            .expect("spawn sh")
    }

    #[test]
    fn signaled_child_reports_128_plus_signal() {
        let status = run_sh("kill -TERM $$");
        assert_eq!(process_exit_status(&status), 128 + libc::SIGTERM);
    }

    #[test]
    fn exited_child_reports_its_code() {
        let status = run_sh("exit 42");
        assert_eq!(process_exit_status(&status), 42);
    }
}
