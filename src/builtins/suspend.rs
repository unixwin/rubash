//! suspend module.
//!
//! GNU Bash source ownership:
//! - builtins/suspend.def suspend_builtin (suspend.def:80-126: the
//!   guards below in source order, then `killpg (shell_pgrp, SIGSTOP)`
//!   and EXECUTION_SUCCESS)

use std::io;

use crate::executor::builtin_diag::{Diag, DiagKind, EX_USAGE};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

/// Returns the builtin status plus a `too_many_arguments` flag. GNU
/// suspend.def reports an operand with `suspend: too many arguments` —
/// the builtins/common.c no_args shape, whose DISCARD jump (exit class,
/// rubash#705) the Executor arms when it sees the flag: the rest of the
/// current list is abandoned with $?=2, and under `-c` the shell exits 1.
/// A bad OPTION is different: plain EX_USAGE and the shell continues.
///
/// The scan is one `BuiltinOpt` pass over the table row (`"f"`,
/// suspend.def:86): `--help` in option position prints the long help
/// (status 2); any other option word is an invalid option (usage +
/// `EX_USAGE`).
pub fn execute_with_io<'a>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut Vec<u8>,
    job_control: bool,
    login_shell: bool,
) -> io::Result<(i32, bool)> {
    let (force, help, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut force = false;
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Opt(b'f') => force = true,
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is `"f"`: no other option character
                // (and no argument-taking option) can ever arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("suspend spec is \"f\"")
                }
            }
        }
        let operands = opt.operands().to_vec();
        (force, help, operands)
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok((EX_USAGE, false));
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok((diag.status(), false));
    }

    if !operands.is_empty() {
        diag.report(DiagKind::TooManyArgs);
        return Ok((EX_USAGE, true));
    }

    // builtins/suspend.def:101-114: without -f the guards fire in
    // source order — the job_control variable first (jobs.c:223; fed
    // by flags.c:257 when `set -m`/`set +m` runs, erased by
    // without_job_control at fork boundaries — not the monitor flag,
    // whose letter stays in the child's `$-` either way), then the
    // login-shell refusal. With -f both guards are skipped.
    if !force {
        if !job_control {
            diag.free(format_args!("cannot suspend: no job control"));
            return Ok((EXECUTION_FAILURE, false));
        }
        if login_shell {
            diag.free(format_args!("cannot suspend a login shell"));
            return Ok((EXECUTION_FAILURE, false));
        }
    }

    stop_shell_group();
    Ok((EXECUTION_SUCCESS, false))
}

/// GNU builtins/suspend.def:123: the shell stops its own process
/// group — `killpg (shell_pgrp, SIGSTOP)` — and suspend_builtin
/// returns EXECUTION_SUCCESS once continued (rubash#1097).
/// shell_pgrp is this process's group (jobs.c:4737
/// `shell_pgrp = getpgid (0)`); SIGSTOP/SIGCONT ride the kernel
/// defaults (never in kill.rs's managed set), so no trap bookkeeping
/// is needed — a trapped SIGCONT does not fire on resume, matching
/// GNU's suspend_continue handler swap (suspend.def:119). Non-unix
/// has no SIGSTOP: keep the historical success no-op there.
fn stop_shell_group() {
    #[cfg(unix)]
    {
        // SAFETY: getpgid/killpg with our own pgrp are pure queries.
        let pgrp = unsafe { libc::getpgid(0) };
        if pgrp > 0 {
            // GNU is single-threaded: its killpg stop is dequeued by
            // the executing thread at the killpg exit path, so the
            // shell stops AT the builtin and one SIGCONT resumes it.
            // In this multithreaded executor neither half holds for a
            // bare killpg (measured on a two-thread harness and on
            // this binary under load):
            // - rubash#1474: a process-directed SIGSTOP is dequeued
            //   by whichever thread the kernel wakes; a RUNNING
            //   executing thread crosses many syscall exits without
            //   joining the stop, so commands after `suspend -f`
            //   execute before the stop lands (a whole `e1..e5`
            //   tail). Only a THREAD-directed SIGSTOP stops the
            //   executing thread deterministically, at that call's
            //   own exit path.
            // - rubash#2203: sending BOTH (killpg, then a
            //   thread-directed SIGSTOP as the #1474 clamp did)
            //   leaves two stop generations pending in this
            //   process; the one the executing thread does not
            //   dequeue survives the stop episode, so the first
            //   SIGCONT (pid- or group-addressed — exactly what
            //   `fg` sends) is followed by a re-stop and a second
            //   SIGCONT is required (15/40 runs under load; the
            //   #1637 harness note records the same lost wakeup).
            // So: every OTHER process in the group gets its own
            // process-directed SIGSTOP (for them, a single
            // generation), and this process stops solely through
            // the thread-directed SIGSTOP — prompt (#1474) and the
            // only in-process generation, hence consumed by the
            // episode: one SIGCONT resumes (#2203). A member that
            // exits mid-enumeration is ignored (ESRCH); a process
            // joining the group concurrently is the one corner
            // where this differs from killpg's atomicity.
            #[cfg(target_os = "linux")]
            {
                if stop_other_group_members(pgrp) {
                    unsafe {
                        let pid = libc::getpid();
                        let tid = libc::syscall(libc::SYS_gettid) as libc::pid_t;
                        libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGSTOP);
                    }
                } else {
                    // /proc unavailable: fall back to the historical
                    // killpg + thread-directed clamp (prompt; the
                    // #2203 second-generation corner returns only
                    // in this degraded path).
                    unsafe {
                        libc::killpg(pgrp, libc::SIGSTOP);
                        let pid = libc::getpid();
                        let tid = libc::syscall(libc::SYS_gettid) as libc::pid_t;
                        libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGSTOP);
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            unsafe {
                libc::killpg(pgrp, libc::SIGSTOP);
                // rubash#1474 clamp, unchanged on this platform:
                // stop the executing thread at a signal boundary.
                libc::pthread_kill(libc::pthread_self(), libc::SIGSTOP);
            }
        }
    }
}

/// Linux: send a process-directed SIGSTOP to every process in `pgrp`
/// except this one (see the rubash#2203 sequencing note in
/// `execute_with_io`). Returns false when /proc cannot be enumerated
/// at all, so the caller can fall back to killpg.
#[cfg(target_os = "linux")]
fn stop_other_group_members(pgrp: libc::pid_t) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    let own_pid = unsafe { libc::getpid() };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue; // raced an exit, or not ours to inspect
        };
        // comm is parenthesized and may itself contain spaces/parens;
        // the fields after the LAST ')' are state, ppid, pgrp, ...
        let Some(after_comm) = stat.rsplit(')').next() else {
            continue;
        };
        let mut fields = after_comm.split_whitespace();
        let _state = fields.next();
        let _ppid = fields.next();
        let Some(member_pgrp) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if member_pgrp == pgrp {
            // ESRCH (it exited) and EPERM alike: nothing more this
            // builtin can do for that member; keep going.
            unsafe {
                libc::kill(pid, libc::SIGSTOP);
            }
        }
    }
    true
}
