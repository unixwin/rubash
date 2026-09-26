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
