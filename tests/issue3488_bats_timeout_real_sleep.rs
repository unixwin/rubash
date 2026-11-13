//! rubash#3488: on unix, a parseable `sleep` must run as a real
//! spawned child process, not as an in-process simulation.
//!
//! The field case is the bats-core timeout watchdog
//! (test/timeout.bats "test longer than timeout" / "run longer
//! than timeout"): bats-exec-test's bats_start_timeout_countdown
//! traps TERM in the test shell and, once the countdown expires,
//! bats_kill_processes_of enumerates the test shell's whole
//! descendant tree via `ps` and TERMs every pid in it. Under GNU
//! the foreground `sleep` is a real child (jobs.c:2265 make_child;
//! the shell waits in jobs.c:3064 wait_for), so the delivery kills
//! the child, the wait returns at once, and the pending trap runs
//! at the next command boundary (execute_cmd.c:643
//! run_pending_traps) — the inner run ends after ~1s. The
//! in-process simulation has no process identity (rubash#2995):
//! there is no child in the tree for the killer to kill, the
//! shell-directed copy of the delivery is deferred by the
//! virtual-child model (an in-group sender reads as a shell-alone
//! delivery), and the run lasts the whole sleep — measured
//! 10271ms/10439ms for rubash vs 1021ms for GNU on timeout.bats
//! #2/#3, with only the final `((SECONDS < 10))` assertion
//! failing. The same script shapes below were run against GNU
//! bash 5.3.20: the watchdog shape finishes in 1.03s with the trap
//! run (rc 42).
//!
//! User ruling 2026-11-14 (decision (a)): the sleep row's
//! resolvable policy flips Replace -> Yield; on unix the
//! simple-command face stands down for every parseable form, so
//! sleep is a real child again. These tests pin the two halves of
//! that contract:
//!
//! 1. While a foreground `sleep` runs, the shell's child list
//!    (ps, the same enumeration bats performs) contains a real
//!    `sleep` process.
//! 2. A tree-wide TERM delivery after 1s ends the wait promptly
//!    (wall clock far below the full sleep) and the shell's TERM
//!    trap still runs at the boundary.
//!
//! On the pre-fix tree test 1 finds no child at all and test 2
//! runs the full 10s before the trap fires (content correct,
//! wall clock wrong — the bats failure signature). The killer in
//! test 2 miniaturizes bats_kill_processes_of as a process-group
//! delivery: the spawned shell runs in its own session, so its
//! group is exactly its descendant tree, and the recipient set —
//! shell plus real sleep child — is the same set bats TERMs pid
//! by pid. Test 1's script carries a trailing command so the
//! sleep is not in -c tail position (a lone tail command may
//! exec-replace the shell, GNU builtins/evalstring.c:125
//! should_suppress_fork — that shape is a real sleep by
//! construction and is not what bats runs).

#![cfg(unix)]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Spawn `rubash -c script` in its own session (so the test can
/// address its process group without touching the harness's) with
/// piped stdout/stderr.
fn spawn_rubash(script: &str) -> std::process::Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn().expect("spawn rubash")
}

/// (pid, ppid, comm) rows for the whole process table — the same
/// enumeration bats_find_processes_of performs with
/// `ps -Ao pid,ppid,...` (comm via ucomm, the basename form; the
/// header row fails the numeric parse and drops out).
fn process_table() -> Vec<(u32, u32, String)> {
    let output = Command::new("ps")
        .args(["-Ao", "pid,ppid,ucomm"])
        .output()
        .expect("run ps");
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let ppid: u32 = fields.next()?.parse().ok()?;
            let comm = fields.next()?.to_string();
            Some((pid, ppid, comm))
        })
        .collect()
}

/// Direct children of `pid` as (pid, comm) pairs.
fn children_of(pid: u32) -> Vec<(u32, String)> {
    process_table()
        .into_iter()
        .filter(|(_, ppid, _)| *ppid == pid)
        .map(|(child, _, comm)| (child, comm))
        .collect()
}

fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

struct Finished {
    stdout: String,
    code: Option<i32>,
    elapsed: Duration,
}

/// Run `script` to completion, bounded by `cap` (the scripts here
/// print a few bytes at most, so draining stdout after exit cannot
/// deadlock on a full pipe). On cap expiry the whole session group
/// is killed and the run reports with `code == None`.
fn run_bounded(script: &str, cap: Duration) -> Finished {
    let start = Instant::now();
    let mut child = spawn_rubash(script);
    let shell_pid = child.id();
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break Some(status);
        }
        if start.elapsed() >= cap {
            kill_group(shell_pid);
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    let _ = child.wait();
    Finished {
        stdout,
        code: status.and_then(|s| s.code()),
        elapsed: start.elapsed(),
    }
}

#[test]
fn parseable_sleep_spawns_a_real_child_process() {
    let mut child = spawn_rubash("sleep 30\necho done");
    let shell_pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen: Vec<(u32, String)> = Vec::new();
    let mut found = false;
    while Instant::now() < deadline {
        seen = children_of(shell_pid);
        if seen.iter().any(|(_, comm)| comm == "sleep") {
            found = true;
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    kill_group(shell_pid);
    let _ = child.wait();
    assert!(
        found,
        "no real `sleep` child under shell pid {shell_pid}; children seen: {seen:?}"
    );
}

#[test]
fn tree_wide_term_ends_foreground_sleep_promptly() {
    // The bats timeout shape, miniaturized: the test shell traps
    // TERM (bats_timeout_trap), a background killer TERMs the
    // whole tree after the 1s countdown (bats_kill_processes_of),
    // and the foreground command is a 10s sleep.
    let script = "trap 'echo TRAPPED; exit 42' TERM\n\
                  ( sleep 1; kill -TERM -$$ ) &\n\
                  sleep 10\n\
                  echo SURVIVED";
    let run = run_bounded(script, Duration::from_secs(15));
    assert_eq!(
        run.stdout, "TRAPPED\n",
        "the TERM trap must run at the boundary after the wait"
    );
    assert_eq!(run.code, Some(42));
    assert!(
        run.elapsed >= Duration::from_millis(800),
        "trap fired after {:?}, before the 1s countdown could expire — \
         the countdown did not govern the timing",
        run.elapsed
    );
    assert!(
        run.elapsed < Duration::from_secs(3),
        "wall clock {:?} ran toward the full 10s sleep — no real child \
         for the tree kill to land on",
        run.elapsed
    );
}
