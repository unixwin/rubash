//! suspend module.
//!
//! GNU Bash source ownership:
//! - builtins/suspend.def suspend_builtin (`killpg (shell_pgrp, SIGSTOP)`
//!   on -f; `sh_nojobs` failure without job control otherwise)

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

/// Returns the builtin status plus a `too_many_arguments` flag. GNU
/// suspend.def reports an operand with `suspend: too many arguments` —
/// the builtins/common.c no_args shape, whose DISCARD jump (exit class,
/// rubash#705) the Executor arms when it sees the flag: the rest of the
/// current list is abandoned with $?=2, and under `-c` the shell exits 1.
/// A bad OPTION is different: plain EX_USAGE and the shell continues.
pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<(i32, bool)>
where
    E: Write,
{
    let mut force = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'f' => force = true,
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}suspend: -{other}: invalid option"
                    )?;
                    writeln!(stderr, "suspend: usage: suspend [-f]")?;
                    return Ok((EX_USAGE, false));
                }
            }
        }
        index += 1;
    }

    if index < args.len() {
        writeln!(stderr, "{diagnostic_prefix}suspend: too many arguments")?;
        return Ok((EX_USAGE, true));
    }

    if force {
        // GNU builtins/suspend.def suspend_builtin: with -f the login-shell
        // and no-job-control guards are skipped and the shell stops its own
        // process group — `killpg (shell_pgrp, SIGSTOP)` — returning SUCCESS
        // once continued (rubash#1097). shell_pgrp is this process's group
        // (jobs.c:4737 `shell_pgrp = getpgid (0)`); SIGSTOP/SIGCONT ride the
        // kernel defaults (never in kill.rs's managed set), so no trap
        // bookkeeping is needed. Non-unix has no SIGSTOP: keep the
        // historical success no-op there.
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
        return Ok((EXECUTION_SUCCESS, false));
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}suspend: cannot suspend: no job control"
    )?;
    Ok((EXECUTION_FAILURE, false))
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
