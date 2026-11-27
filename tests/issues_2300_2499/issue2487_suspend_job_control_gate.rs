//! rubash#2487 — `suspend` read the monitor OPTION flag's state
//! through a path that never saw a non-interactive `set -m`, so a
//! script that enabled job control was told `cannot suspend: no job
//! control` and ran on. GNU keys the builtin on the job_control
//! VARIABLE (jobs.c:223): flags.c:257 routes every `set -m`/`set +m`
//! through set_job_control, so after `set -m` the builtin really
//! stops the shell (builtins/suspend.def:80-126 suspend_builtin:
//! killpg (shell_pgrp, SIGSTOP), return EXECUTION_SUCCESS), while a
//! fork boundary erases only the variable (jobs.c:5356-5366
//! without_job_control via execute_in_subshell /
//! execute_subshell_builtin_or_function) — the flag letter stays in
//! the child's `$-`. The command-substitution fork does NOT erase it
//! (subst.c:7143 command_substitute).
//!
//! Shapes pinned here are the r46 differential family's key faces
//! (ss01/ss02/ss03/ss18/ss19/ss23/ss24/ss25 + the g6/ss06 boundary
//! pair), each verified byte-for-byte against GNU bash 5.3.20 under
//! the same harness. The harness follows tests/issue1474's
//! supervised-child discipline (the child runs in its own session
//! via setsid — never an orphaned-pgrp pty harness; SIGCONT is
//! re-sent while the child is stopped; every wait is bounded; a
//! Drop guard SIGKILLs the session), which is the runner_r46.py
//! SIGCONT-pulse rescue model carried into the test.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2487-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn proc_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rsplit(") ").next()?;
    after.chars().next()
}

fn read_file(path: &PathBuf) -> String {
    let mut s = String::new();
    let _ = std::fs::File::open(path).map(|mut f| f.read_to_string(&mut s));
    s
}

fn send_sigcont(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGCONT);
        libc::kill(pid as i32, libc::SIGCONT);
    }
}

/// Supervised session child: SIGKILLs and reaps its session on
/// every exit path so a stopped rubash can never leak (the
/// rubash#1637 discipline from tests/issue1474).
struct SupervisedChild {
    child: std::process::Child,
    pid: u32,
}

impl SupervisedChild {
    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        let _ = self.wait_for_exit(Duration::from_secs(5));
    }
}

struct RunResult {
    stdout: String,
    stderr: String,
    status: std::process::ExitStatus,
}

/// Run `script` under the built rubash in its own session. When
/// `prove_stop` is set, first poll (without sending any SIGCONT)
/// until the process is observed in state T — proof the stop is
/// real — then start the rescue pulses; otherwise pulse SIGCONT
/// from the start. Every wait is bounded; a timeout panics with
/// the captured output instead of hanging.
fn run_script(tag: &str, script: &str, prove_stop: bool) -> RunResult {
    let dir = tempdir(tag);
    let target = dir.join("t.sh");
    std::fs::write(&target, script).expect("write target");
    let out_path = dir.join("out.txt");
    let err_path = dir.join("err.txt");
    let out_file = std::fs::File::create(&out_path).expect("create out");
    let err_file = std::fs::File::create(&err_path).expect("create err");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().expect("spawn rubash");
    let pid = child.id();
    let mut sup = SupervisedChild { child, pid };

    if prove_stop {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut stopped = false;
        while Instant::now() < deadline {
            if proc_state(pid) == Some('T') {
                stopped = true;
                break;
            }
            if let Ok(Some(_)) = sup.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            stopped,
            "process never entered stopped state T; out: {:?}; err: {:?}",
            read_file(&out_path),
            read_file(&err_path)
        );
        // Nothing past the suspend may have run while stopped.
        std::thread::sleep(Duration::from_millis(300));
    }

    let done = Arc::new(AtomicBool::new(false));
    let rescuer = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                send_sigcont(pid);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    let status = sup.wait_for_exit(Duration::from_secs(30));
    done.store(true, Ordering::Relaxed);
    let _ = rescuer.join();
    let status = status.unwrap_or_else(|| {
        panic!(
            "child did not exit within 30s (state: {:?}); out: {:?}; err: {:?}",
            proc_state(pid),
            read_file(&out_path),
            read_file(&err_path)
        )
    });
    let result = RunResult {
        stdout: read_file(&out_path),
        stderr: read_file(&err_path),
        status,
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// r46 ss01: `set -m` arms the variable; suspend really stops the
/// shell, and after SIGCONT the script continues with rc 0.
#[test]
fn set_m_suspend_stops_then_resumes() {
    let r = run_script("ss01", "set -m\nsuspend\necho after\n", true);
    assert!(r.status.success(), "status: {}", r.status);
    assert_eq!(r.stdout, "after\n");
    assert_eq!(r.stderr, "");
}

/// r46 ss03: no job control anywhere — the refusal is unchanged.
/// builtins/suspend.def:103-107: job_control == 0 ->
/// sh_nojobs + EXECUTION_FAILURE.
#[test]
fn bare_suspend_refuses_without_job_control() {
    let r = run_script("ss03", "suspend\necho \"rc=$?\"\n", false);
    assert_eq!(r.stdout, "rc=1\n");
    assert!(
        r.stderr.contains("suspend: cannot suspend: no job control"),
        "stderr: {:?}",
        r.stderr
    );
}

/// r46 ss18: `set +m` disarms the variable again (flags.c:257 with
/// FLAG_OFF), so the refusal returns.
#[test]
fn set_m_then_plus_m_refuses() {
    let r = run_script("ss18", "set -m\nset +m\nsuspend\necho \"rc=$?\"\n", false);
    assert_eq!(r.stdout, "rc=1\n");
    assert!(
        r.stderr.contains("suspend: cannot suspend: no job control"),
        "stderr: {:?}",
        r.stderr
    );
}

/// `set -o monitor` is the same flags.c:257 write in long form.
#[test]
fn set_o_monitor_arms_the_gate() {
    let r = run_script("set-o", "set -o monitor\nsuspend\necho after\n", false);
    assert!(r.status.success(), "status: {}", r.status);
    assert_eq!(r.stdout, "after\n");
    assert_eq!(r.stderr, "");
}

/// r46 ss23: in a condition, suspend's EXECUTION_SUCCESS takes the
/// then-branch.
#[test]
fn set_m_suspend_in_if_condition() {
    let r = run_script(
        "ss23",
        "set -m\nif suspend; then echo then-branch; else echo else-branch; fi\n",
        false,
    );
    assert_eq!(r.stdout, "then-branch\n");
    assert_eq!(r.stderr, "");
}

/// r46 ss24: `suspend && ...` chains on the success status.
#[test]
fn set_m_suspend_and_chain() {
    let r = run_script("ss24", "set -m\nsuspend && echo chained\n", false);
    assert_eq!(r.stdout, "chained\n");
    assert_eq!(r.stderr, "");
}

/// r46 ss25: `suspend || ...` does NOT run the fallback.
#[test]
fn set_m_suspend_or_chain_skips_fallback() {
    let r = run_script(
        "ss25",
        "set -m\nsuspend || echo fallback\necho \"end rc=$?\"\n",
        false,
    );
    assert_eq!(r.stdout, "end rc=0\n");
    assert_eq!(r.stderr, "");
}

/// The command-substitution fork keeps the variable (subst.c:7143):
/// suspend inside `$( )` under `set -m` really stops and resumes.
#[test]
fn set_m_suspend_inside_command_substitution() {
    let r = run_script(
        "comsub",
        "set -m\nx=$(suspend; echo in-comsub)\necho \"got=[$x]\"\n",
        false,
    );
    assert_eq!(r.stdout, "got=[in-comsub]\n");
    assert_eq!(r.stderr, "");
}

/// A function body shares the caller's variable state: suspend in a
/// function under `set -m` stops the shell, rc 0 on resume.
#[test]
fn set_m_suspend_inside_function() {
    let r = run_script(
        "func",
        "set -m\nf() { suspend; echo \"in-func rc=$?\"; }\nf\necho main-after\n",
        false,
    );
    assert_eq!(r.stdout, "in-func rc=0\nmain-after\n");
    assert_eq!(r.stderr, "");
}

/// Boundary (the g6 face of r46 ss06): a `( )` subshell under
/// `set -m` starts with the variable erased (without_job_control),
/// so its suspend refuses even though `$-` still shows `m`; the
/// enclosing shell is unaffected.
#[test]
fn set_m_subshell_without_rearm_refuses() {
    let r = run_script(
        "g6",
        "set -m\n(suspend; echo sub-after)\necho main-after\n",
        false,
    );
    assert_eq!(r.stdout, "sub-after\nmain-after\n");
    assert!(
        r.stderr.contains("suspend: cannot suspend: no job control"),
        "stderr: {:?}",
        r.stderr
    );
}

/// Boundary (r46 ss19): a pipeline element is an
/// execute_subshell_builtin_or_function child — same erasure.
#[test]
fn set_m_pipeline_stage_refuses() {
    let r = run_script(
        "ss19",
        "set -m\nsuspend | cat\necho \"after rc=$?\"\n",
        false,
    );
    assert_eq!(r.stdout, "after rc=0\n");
    assert!(
        r.stderr.contains("suspend: cannot suspend: no job control"),
        "stderr: {:?}",
        r.stderr
    );
}
