//! Foreground-wait termination-signal timing (rubash#3284; niubash#988
//! timing residual).
//!
//! GNU: a terminating signal arriving while the shell waits for a
//! foreground job kills the shell mid-wait — sig.c:590 termsig_handler
//! runs the EXIT trap and then sig.c:650 kill_shell re-raises the signal
//! so the shell dies BY it; jobs.c:3064 wait_for checks CHECK_TERMSIG
//! (quit.h:53) inside its wait loop, never waiting for the job to end
//! first. Measured GNU death: ~0.05s after delivery.
//!
//! Rubash's unix foreground waits block in `child.wait()` without
//! draining the signal mailbox, so a recorded terminating delivery is
//! consumed only at the next command-end boundary — death form correct
//! (signal death), timing late (child's natural end, ~2.4-3s for
//! `sleep 3`). These pins assert timing AND death form: the shell must
//! die by the signal well before the foreground child would exit on
//! its own.

#![cfg(unix)]

use std::fs;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Spawn rubash on a script FILE; return (stdout, exit code, killing
/// signal, time until the PROCESS exits).
///
/// The wall time is the process's death, NOT pipe EOF: a shell that
/// dies by signal mid-wait orphans the foreground child, and the
/// orphan holds the shell's stdout pipe open until its own natural
/// end — `wait_with_output` would report the orphan's lifetime, not
/// the shell's (a `wait`-builtin probe measured 5.2s that way while
/// the shell itself had died at 0.5s). So poll `try_wait` for the
/// exit and collect stdout through a chunked reader thread, draining
/// briefly after death instead of waiting for EOF.
fn run_script_timed(tag: &str, body: &str) -> (String, Option<i32>, Option<i32>, Duration) {
    let dir = std::env::temp_dir().join(format!("issue3284-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let start = Instant::now();
    let mut child: Child = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout_pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    // Drain stderr on a thread too so a verbose shell cannot block
    // on a full pipe while we poll.
    let mut stderr_pipe = child.stderr.take().expect("stderr pipe");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr_pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    });
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll rubash") {
            break status;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let elapsed = start.elapsed();
    // The shell has died; its output is already written. Give the
    // reader a short settle to forward the final chunk, then stop
    // collecting — do NOT wait for pipe EOF, which an orphaned
    // foreground child holds open until its own natural end.
    std::thread::sleep(Duration::from_millis(200));
    let mut out = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        out.extend_from_slice(&chunk);
    }
    drop(rx);
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out).into_owned(),
        status.code(),
        status.signal(),
        elapsed,
    )
}

/// A delivery at 0.3s must kill the shell in well under the child's
/// 5s lifetime. 2.5s leaves ample room for a loaded box to deliver
/// the background kill late, while an unfixed wait dies at ~5s.
const PROMPT_DEATH: Duration = Duration::from_millis(2500);

fn assert_prompt_termsig_death(
    tag: &str,
    stdout: &str,
    code: Option<i32>,
    signal: Option<i32>,
    elapsed: Duration,
    expected_signal: i32,
) {
    assert_eq!(
        stdout, "EXIT-TRAP\n",
        "{tag}: EXIT trap must run: {stdout:?}"
    );
    assert_eq!(code, None, "{tag}: must die by signal, not exit: {code:?}");
    assert_eq!(
        signal,
        Some(expected_signal),
        "{tag}: must die by signal {expected_signal}"
    );
    assert!(
        elapsed < PROMPT_DEATH,
        "{tag}: died at {elapsed:?}, must die promptly (before the 5s child ends)"
    );
}

#[test]
fn issue3284_term_during_external_wait_dies_promptly() {
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-ext",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\n/usr/bin/sleep 5\necho unreachable\n",
    );
    assert_prompt_termsig_death("term-ext", &stdout, code, signal, elapsed, libc::SIGTERM);
}

#[test]
fn issue3284_hup_during_external_wait_dies_promptly() {
    let (stdout, code, signal, elapsed) = run_script_timed(
        "hup-ext",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -HUP \"$me\" ) &\n/usr/bin/sleep 5\necho unreachable\n",
    );
    assert_prompt_termsig_death("hup-ext", &stdout, code, signal, elapsed, libc::SIGHUP);
}

#[test]
fn issue3284_term_during_pipeline_wait_dies_promptly() {
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-pipe",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\n/usr/bin/sleep 5 | cat\necho unreachable\n",
    );
    assert_prompt_termsig_death("term-pipe", &stdout, code, signal, elapsed, libc::SIGTERM);
}

#[test]
fn issue3284_term_during_comsub_wait_dies_promptly() {
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-comsub",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\nx=$(/usr/bin/sleep 5)\necho unreachable\n",
    );
    assert_prompt_termsig_death("term-comsub", &stdout, code, signal, elapsed, libc::SIGTERM);
}

#[test]
fn issue3284_term_during_exec_wait_dies_promptly() {
    // The exec form: on unix the standalone shell execve's the
    // replacement image (builtins/exec.rs try_execve), so the process
    // IS the sleep and a delivery to the shell pid kills it at once
    // by SIG_DFL — no EXIT trap runs, because the shell never exits,
    // it is replaced (GNU exec.def: the image is replaced, traps do
    // not run). Assert death form + timing only, no trap output.
    // The spawn-and-wait simulation face (library contexts, where
    // execve is impossible) is a separate lane: it waits in
    // run_external_exec_with_stdio without a mailbox drain, and needs
    // the executor threaded through — owed, see the report.
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-exec",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\nexec /usr/bin/sleep 5\necho unreachable\n",
    );
    assert_eq!(stdout, "", "term-exec: no EXIT trap after exec: {stdout:?}");
    assert_eq!(code, None, "term-exec: must die by signal: {code:?}");
    assert_eq!(signal, Some(libc::SIGTERM), "term-exec");
    assert!(
        elapsed < PROMPT_DEATH,
        "term-exec: died at {elapsed:?}, must die promptly"
    );
}

#[test]
fn issue3284_term_during_pipeline_first_member_wait_dies_promptly() {
    // The non-last member's wait: the last member exits at 0.2s, so
    // the 0.3s delivery lands while the shell waits for the FIRST
    // member (`sleep 5`) in wait_for_windows_pipeline_member — the
    // pipeline face the last-member wait does not cover.
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-pipe-first",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\n/usr/bin/sleep 5 | /usr/bin/sleep 0.2\necho unreachable\n",
    );
    assert_prompt_termsig_death(
        "term-pipe-first",
        &stdout,
        code,
        signal,
        elapsed,
        libc::SIGTERM,
    );
}

#[test]
fn issue3284_term_during_merged_pipeline_wait_dies_promptly() {
    // The merged-output pipeline face (`2>&1 |`, one pipe for fd 1
    // and fd 2): the stage waits in wait_pipeline_stage_merged_output.
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-merged",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\n/usr/bin/sleep 5 2>&1 | cat\necho unreachable\n",
    );
    assert_prompt_termsig_death("term-merged", &stdout, code, signal, elapsed, libc::SIGTERM);
}

#[test]
fn issue3284_term_during_wait_builtin_dies_promptly() {
    // The `wait` builtin already drains the mailbox (#1825); pinned
    // here as the delimited control face for the same delivery shape.
    let (stdout, code, signal, elapsed) = run_script_timed(
        "term-wait",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\n/usr/bin/sleep 5 &\nwait\necho unreachable\n",
    );
    assert_prompt_termsig_death("term-wait", &stdout, code, signal, elapsed, libc::SIGTERM);
}

#[test]
fn issue3284_external_wait_without_signal_completes_normally() {
    // Control: a foreground external wait with no delivery must still
    // run to the child's natural end and report its status.
    let (stdout, code, signal, _elapsed) =
        run_script_timed("control", "/usr/bin/sleep 0.2\necho done rc=$?\n");
    assert_eq!(stdout, "done rc=0\n", "control: {stdout:?}");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}
