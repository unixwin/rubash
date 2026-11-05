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

/// rubash#2622: deliver a resumed job's SIGCONT the way GNU
/// continue_job does (jobs.c:3926-3928). With the job-control channel
/// live, the caller passes the job's pgrp and the signal is addressed
/// to the whole process GROUP via killpg — the group also holds
/// children the leader forked after spawn (the `sleep` under
/// `sh -c 'sleep 3; …'`), which appear in no pid list the shell keeps,
/// so per-pid delivery would resume the leader alone and wedge the job
/// forever. The per-pid loop over the recorded members remains only
/// for the shared-group fallback model (no channel, `group_pgrp` is
/// None): there every job member lives in the shell's own group and is
/// a recorded pid. SIGCONT follows the target numbering (18 Linux /
/// 19 Darwin) via kill::SIGCONT_NUMBER — jobs.c:3928 uses the
/// platform's own number, not a Linux literal.
pub(in crate::executor) fn deliver_job_sigcont(group_pgrp: Option<i32>, pids: &[u32]) {
    #[cfg(unix)]
    {
        if let Some(pgrp) = group_pgrp {
            let _ = crate::builtins::kill::send_signal_to_process_group(
                pgrp,
                crate::builtins::kill::SIGCONT_NUMBER,
            );
            return;
        }
    }
    #[cfg(not(unix))]
    let _ = group_pgrp;
    for member in pids {
        let _ = crate::builtins::kill::send_signal(*member, crate::builtins::kill::SIGCONT_NUMBER);
    }
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

    /// rubash#2836 slice 4: the subshell twin of
    /// [`job_control_channel_active`]. GNU's subshell is a real forked
    /// process in its own process group, so its foreground children are
    /// waited on with WUNTRACED by the subshell's own waitchld — but the
    /// subshell_environment condition (jobs.c:4077) is about the
    /// subshell's own wait, not about whether the subshell's CHILD gets
    /// job-control treatment. Rubash runs the subshell in-process, so
    /// there is no subshell process to own the group: the external child
    /// itself takes the treatment directly — own process group, terminal
    /// handover, WUNTRACED wait — and on a stop it is parked as the
    /// subshell's job while the subshell command is abandoned by the
    /// stop-unwind (see wait_fg_child_job_control's subshell arm).
    /// The `monitor` OPTION (not the `job_control` variable, which the
    /// subshell entry erases per jobs.c:5356-5366 without_job_control)
    /// is the gate, exactly as for the top-level channel.
    pub(in crate::executor) fn job_control_channel_active_in_subshell(&self) -> bool {
        #[cfg(unix)]
        {
            self.invocation_interactive()
                && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor")
                && !self.shell_state.in_command_substitution.get()
                && self.shell_state.subshell_depth.get() > 0
                && controlling_tty().is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}
