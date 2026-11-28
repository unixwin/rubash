//! Issue rubash#4255: an input process substitution whose `/dev/fd` pipe is
//! never read must not turn the producer's broken pipe into
//! `printf: write error: Bad file descriptor` on Windows.
//!
//! GNU model (vendored third_party/bash): process_substitute forks the
//! producer (subst.c:6362; make_child at subst.c:6412) and the child calls
//! restore_original_signals (subst.c:6426). reset_or_restore_signal_handlers
//! (trap.c:1433-1452) resets caught traps but preserves SIG_IGN. Thus a
//! producer with the default SIGPIPE disposition dies silently as if by
//! SIGPIPE when its write meets a readerless pipe; `trap '' PIPE` instead
//! makes the write fail EPIPE, and sh_chkwrite (builtins/common.c:320-334)
//! reports `write error: Broken pipe`. A genuinely closed stdout (`>&-`)
//! is EBADF and keeps `write error: Bad file descriptor`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4255-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> Run {
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// The producer waits until after the consumer command has finished, so the
/// substitution pipe is readerless without depending on process scheduling.
fn synchronized_unread_procsub_script(prefix: &str) -> String {
    format!(
        "{prefix}echo <(while [ ! -e go ]; do sleep 0.01; done; printf 'hi\\n')\n\
         touch go\n\
         sleep 0.2\n\
         echo done\n"
    )
}

#[test]
fn issue4255_unread_input_procsub_default_sigpipe_is_silent() {
    let dir = scratch("default");
    let run = run_script(&dir, &synchronized_unread_procsub_script(""));
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "/dev/fd/63\ndone\n");
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4255_immediate_input_procsub_builtin_writes_are_silent() {
    let dir = scratch("immediate");
    let run = run_script(
        &dir,
        "echo <(printf 'hi\\n')\necho <(echo hi)\necho <(pwd)\necho done\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "/dev/fd/63\n/dev/fd/63\n/dev/fd/63\ndone\n");
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4255_ignored_sigpipe_reports_broken_pipe_not_bad_fd() {
    let dir = scratch("ignored");
    let run = run_script(&dir, &synchronized_unread_procsub_script("trap '' PIPE\n"));
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "/dev/fd/63\ndone\n");
    assert!(
        run.stderr.contains("printf: write error: Broken pipe"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4255_ignored_pipe_disposition_crosses_procsub_fork() {
    let dir = scratch("visibility");
    let run = run_script(&dir, "trap '' PIPE\ncat <(trap -p PIPE)\necho done\n");
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "trap -- '' SIGPIPE\ndone\n");
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4255_inherited_ignore_is_not_a_hard_ignore_in_the_child() {
    let dir = scratch("retrap");
    let run = run_script(
        &dir,
        "trap '' PIPE\ncat <(trap 'echo inner' PIPE; echo rc=$?; trap -p PIPE)\necho done\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\ntrap -- 'echo inner' SIGPIPE\ndone\n");
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4255_genuinely_closed_stdout_still_reports_bad_fd() {
    let dir = scratch("closed");
    let run = run_script(
        &dir,
        "printf 'hi\\n' >&-\necho \"printf_rc=$?\"\necho hi >&-\necho \"echo_rc=$?\"\npwd >&-\necho \"pwd_rc=$?\"\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "printf_rc=1\necho_rc=1\npwd_rc=1\n");
    assert_eq!(
        run.stderr,
        "case.sh: line 1: printf: write error: Bad file descriptor\n\
         case.sh: line 3: echo: write error: Bad file descriptor\n\
         case.sh: line 5: pwd: write error: Bad file descriptor\n",
        "stderr: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}
