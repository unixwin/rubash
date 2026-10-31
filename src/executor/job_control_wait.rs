//! rubash#2398 — the terminal half of the interactive job-control
//! channel: controlling-tty discovery, the tcsetpgrp handoff/reclaim
//! pair, and the gate predicate the foreground spawn/wait sites share.
//!
//! GNU contract (bash 5.3, jobs.c): an interactive shell with job
//! control gives the terminal to the foreground job's process group
//! before waiting on it and takes it back when the wait ends —
//! give_terminal_to (jobs.c:4999) wraps tcsetpgrp in a sigprocmask
//! block of SIGTTOU/SIGTTIN/SIGTSTP/SIGCHLD and ignores errors;
//! start_job calls it for the job's pgrp (jobs.c:3918), wait_for
//! reclaims it for shell_pgrp (jobs.c:3253). The tty itself is the
//! one initialize_job_control settles on (jobs.c:4750): stderr when
//! it is a tty, else /dev/tty (jobs.c:4763). The waits themselves
//! live with their callers (external_inner's foreground-child wait,
//! job_builtins' fg wait); this module is only the shared terminal
//! machinery and the single gate both sides consult, so a spawn and
//! its wait can never disagree about whether the channel is live.

use super::*;

#[cfg(unix)]
use std::os::unix::io::RawFd;

/// A controlling-terminal fd for tcsetpgrp handoffs. Borrowed std
/// fds are left open on drop; a /dev/tty opened by
/// [`controlling_tty`] is owned and closed.
#[cfg(unix)]
pub(in crate::executor) struct ControllingTty {
    fd: RawFd,
    owned: bool,
}

#[cfg(unix)]
impl Drop for ControllingTty {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

/// Find the session's controlling terminal (jobs.c:4750/4763): the
/// first std fd that is a tty — GNU settles on stderr, but an
/// interactive rubash session may hold the tty on any of the three —
/// else open /dev/tty for the caller's scope.
#[cfg(unix)]
pub(in crate::executor) fn controlling_tty() -> Option<ControllingTty> {
    for fd in [libc::STDERR_FILENO, libc::STDIN_FILENO, libc::STDOUT_FILENO] {
        if unsafe { libc::isatty(fd) } == 1 {
            return Some(ControllingTty { fd, owned: false });
        }
    }
    let path = std::ffi::CString::new("/dev/tty").expect("static cstring");
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
    if fd >= 0 {
        Some(ControllingTty { fd, owned: true })
    } else {
        None
    }
}

/// jobs.c:4999 give_terminal_to: hand the terminal to `pgrp`,
/// blocking the terminal-generated signals (and SIGCHLD) around the
/// tcsetpgrp so the hand-off itself cannot stop or interrupt the
/// shell; failures are silent, exactly as in GNU.
#[cfg(unix)]
pub(in crate::executor) fn give_terminal_to(tty: &ControllingTty, pgrp: i32) {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTTOU, libc::SIGTTIN, libc::SIGTSTP, libc::SIGCHLD] {
            libc::sigaddset(&mut set, signal);
        }
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old);
        let _ = libc::tcsetpgrp(tty.fd, pgrp);
        libc::sigprocmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
    }
}

/// The shell's own process group — the group wait_for reclaims the
/// terminal for (jobs.c:3253 give_terminal_to (shell_pgrp, 0)).
#[cfg(unix)]
pub(in crate::executor) fn shell_pgrp() -> i32 {
    unsafe { libc::getpgrp() }
}

impl Executor {
    /// rubash#2398: the stop-detection channel is live exactly when
    /// GNU would run waitchld with WUNTRACED — an interactive shell
    /// with the monitor option on, at the top level of the session
    /// (GNU's `subshell_environment == 0` condition, jobs.c:4077),
    /// with a controlling terminal to hand over. Spawn, wait, and fg
    /// all consult this one predicate so the three can never drift
    /// apart. Unix-only machinery: on other targets the channel does
    /// not exist and every consumer keeps its existing path.
    pub(in crate::executor) fn job_control_channel_active(&self) -> bool {
        #[cfg(unix)]
        {
            self.invocation_interactive()
                && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor")
                && !self.shell_state.in_command_substitution.get()
                && self.shell_state.subshell_depth.get() == 0
                && controlling_tty().is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// Queue the boundary stop notice for a job the job-control wait
    /// just observed stopping (rubash#2398). The line itself renders
    /// at the next reader boundary inside
    /// report_pending_signal_notices, in the same position GNU's
    /// notify_of_job_status prints it (eval.c:355) — the job-table
    /// state at render time is what fills the line in, so only the
    /// pid is queued.
    pub(crate) fn queue_stopped_job_notice(&self, pid: u32) {
        self.pending_stopped_job_notices.borrow_mut().push(pid);
    }
}
