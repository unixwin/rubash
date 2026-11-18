//! rubash#3578 (under-limit pin): with `ulimit -n 16` keeping fds
//! 13-15 below the RLIMIT_NOFILE soft limit, a redirect-only `exec`
//! must bind EVERY heredoc / here-string entry of its redirect list
//! at that entry's list position — `exec 14<<A 15<<B` leaves BOTH fd
//! 14 and fd 15 readable with their own bodies. The pre-#3752 walk
//! bound only the LAST `heredoc_redirects` entry, so fd 14's binding
//! was lost (the issue's seed shape, found by PR #3576's review
//! probes).
//!
//! GNU applies each redirect in do_redirections' list order —
//! here_document_to_fd (redir.c:423) materializes the body and dup2's
//! it onto the redirector at redir.c:1065-1072/:1120 — and a
//! redirect-only exec keeps those binds persistent: execute_builtin_
//! or_function runs do_redirections(RX_ACTIVE|RX_UNDOABLE)
//! (execute_cmd.c:5606) and exec_builtin's no-operand form disposes
//! the undo list (builtins/exec.def:134), committing every bind.
//!
//! Every expectation byte-verified against WSL GNU bash 5.3.20
//! script-file probes under `ulimit -n 16` (fd 15 is the highest
//! legal slot; the #3195 pin owns the at/over-limit failure cells).

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3578-limit-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn issue3578_under_limit_seed_binds_both_full_bodies() {
    // The issue's seed shape: `exec 14<<A 15<<B` under the fd limit.
    // Both descriptors stay bound to their OWN bodies — multi-line
    // bodies read back whole through `cat <&N`, proving the bind is
    // the body stream, not just its first line.
    let (stdout, stderr, code, _dir) = run_script(
        "seed",
        "ulimit -n 16\n\
         exec 14<<A 15<<B\n\
         l1-14\n\
         l2-14\n\
         A\n\
         l1-15\n\
         l2-15\n\
         B\n\
         cat <&14; echo \"--\"; cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(
        stdout, "l1-14\nl2-14\n--\nl1-15\nl2-15\n",
        "stdout={stdout:?}"
    );
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue3578_under_limit_seed_read_u_both() {
    // The issue's literal reproducer: both fds readable via `read -u`.
    let (stdout, stderr, code, _dir) = run_script(
        "seed-read",
        "ulimit -n 16\n\
         exec 14<<A 15<<B\n\
         first\n\
         A\n\
         second\n\
         B\n\
         read -u 14 x; read -u 15 y; echo \"x=$x y=$y\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=first y=second\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue3578_under_limit_reversed_fd_order() {
    // `exec 15<<B 14<<A`: bodies arrive in OPERATOR order (B's body
    // first), and each fd still binds its own body — the walk binds
    // at each entry's list position, not by fd number.
    let (stdout, stderr, code, _dir) = run_script(
        "reversed",
        "ulimit -n 16\n\
         exec 15<<B 14<<A\n\
         body-b\n\
         B\n\
         body-a\n\
         A\n\
         cat <&14; echo \"--\"; cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "body-a\n--\nbody-b\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue3578_under_limit_three_heredocs_at_boundary() {
    // fds 13/14/15 — the three highest legal slots under the limit —
    // all bind in one list.
    let (stdout, stderr, code, _dir) = run_script(
        "three",
        "ulimit -n 16\n\
         exec 13<<A 14<<B 15<<C\n\
         one\n\
         A\n\
         two\n\
         B\n\
         three\n\
         C\n\
         cat <&13; echo \"--\"; cat <&14; echo \"--\"; cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "one\n--\ntwo\n--\nthree\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue3578_under_limit_heredoc_and_numbered_here_string() {
    // A numbered heredoc and a numbered here-string in the same list:
    // both bind at their positions under the limit.
    let (stdout, stderr, code, _dir) = run_script(
        "mixed",
        "ulimit -n 16\n\
         exec 14<<A 15<<<\"hs-body\"\n\
         hd-body\n\
         A\n\
         cat <&14; echo \"--\"; cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "hd-body\n--\nhs-body\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue3578_under_limit_binds_persist_into_next_command() {
    // GNU commits the redirect-only exec's binds (exec.def disposes
    // the undo list), so a LATER simple command still reads both
    // descriptors — the loss was not a per-command staging artifact.
    let (stdout, stderr, code, _dir) = run_script(
        "persist",
        "ulimit -n 16\n\
         exec 14<<A 15<<B\n\
         first\n\
         A\n\
         second\n\
         B\n\
         echo marker\n\
         cat <&14; echo \"--\"; cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "marker\nfirst\n--\nsecond\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}
