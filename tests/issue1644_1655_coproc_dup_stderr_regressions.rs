//! rubash#1644 / #1655 — coproc drops dup output redirects.
//!
//! GNU redir.c do_redirections runs in the forked coproc
//! child after the coproc pipes are dup'd, so a trailing
//! `>&2` rebinds the child's stdout to the shell's stderr.
//! Rubash's spawn path only handled the single-field file
//! redirects and dropped every dup: a simple command's
//! output landed in the coproc pipe instead of the shell's
//! stderr (#1644), and a named group's output sat in the
//! pipe unread — lost entirely (#1655).
//!
//! The fix applies the coproc command's output redirects to
//! the child in list order (GNU's left-to-right semantics,
//! so `>file 2>&1` and `2>&1 >file` differ as they should),
//! with a cloneable spec per fd: a dup copies the current
//! spec, a word target on fd 1 is r_err_and_out (one open
//! for both fds, redir.c:832-838), and `2>&word` is an
//! ambiguous redirect (diagnostic once, no coproc created,
//! status 1 per the #846 coproc-failure convention — GNU
//! reports 0 there, matching rubash's existing file-open
//! failure status, not its message).
//!
//! Expectations verified against GNU bash 5.3.20. Stdin is
//! nulled so a regression blocks on nothing.
//!
//! The first seven pins are PR #1695's (a-rookie-of-C-language);
//! the `1>&2` explicit-spelling pin at the end is absorbed from
//! PR #1712 (commit 331c7435), the one spelling its suite did
//! not carry.

use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1644-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn simple_coproc_dup_stderr_goes_to_shell_stderr() {
    // #1644: the pipe must read EOF; err must be on stderr.
    let script = "coproc echo err >&2\ncat <&${COPROC[0]}; echo \"cat_rc=$?\"\nwait $COPROC_PID 2>/dev/null; echo \"wait_rc=$?\"\n";
    let (stdout, stderr, code) = rubash_script(script);
    assert_eq!(stdout, "cat_rc=0\nwait_rc=0\n");
    assert_eq!(stderr, "err\n");
    assert_eq!(code, Some(0));
}

#[test]
fn named_group_coproc_dup_stderr_not_lost() {
    // #1655: the group's output must reach the shell's stderr.
    let script = "coproc C2 { echo hi; } >&2\nsleep 0.3; wait $COPROC_PID 2>/dev/null\n";
    let (stdout, stderr, code) = rubash_script(script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "hi\n");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_stderr_to_pipe_alone() {
    let (stdout, stderr, _) =
        rubash_c("coproc echo hi 2>&1; cat <&${COPROC[0]}; wait $COPROC_PID; echo rc=$?");
    assert_eq!(stdout, "hi\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn coproc_redirect_order_matters_both_ways() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1644o-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let file = dir.join("o.txt").display().to_string();
    // `>file 2>&1`: stderr follows stdout into the file.
    let (stdout, _, _) = rubash_c(&format!(
        "coproc bash -c \"echo O; echo E >&2\" > {file} 2>&1; sleep 0.3; wait $COPROC_PID; cat {file}"
    ));
    assert_eq!(stdout, "O\nE\n");
    let _ = std::fs::remove_file(&file);
    // `2>&1 >file`: stderr stays on the pipe, stdout goes to the file.
    let (stdout, _, _) = rubash_c(&format!(
        "coproc bash -c \"echo O; echo E >&2\" 2>&1 > {file}; cat <&${{COPROC[0]}}; wait $COPROC_PID; echo \"file:\"; cat {file}"
    ));
    assert_eq!(stdout, "E\nfile:\nO\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_dup_stderr_follows_rebound_parent_stderr() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1644r-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let file = dir.join("e.txt").display().to_string();
    let script = format!(
        "exec 2> {file}\ncoproc echo err >&2\nsleep 0.3; wait $COPROC_PID 2>/dev/null\ncat {file}\n"
    );
    let (stdout, _, _) = rubash_script(&script);
    assert_eq!(stdout, "err\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_word_target_is_r_err_and_out_and_fd2_word_is_ambiguous() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1644w-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let file = dir.join("w.txt").display().to_string();
    let (stdout, _, _) = rubash_c(&format!(
        "coproc bash -c \"echo O; echo E >&2\" >& {file}; sleep 0.3; wait $COPROC_PID; cat {file}"
    ));
    assert_eq!(stdout, "O\nE\n");
    let (stdout, stderr, _) = rubash_c("coproc echo hi 2>&word; echo \"after rc=$?\"");
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(stderr.matches("ambiguous redirect").count(), 1);
    assert!(stderr.contains("word: ambiguous redirect"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coproc_plain_redirects_unchanged() {
    // The ordered loop must not regress the file cases it replaced.
    let dir = std::env::temp_dir().join(format!(
        "rubash-1644p-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let file = dir.join("p.txt").display().to_string();
    let (stdout, _, _) = rubash_c(&format!(
        "coproc echo hi > {file}; sleep 0.3; wait $COPROC_PID; cat {file}"
    ));
    assert_eq!(stdout, "hi\n");
    std::fs::write(&file, "OLD\n").expect("seed file");
    let (stdout, _, _) = rubash_c(&format!(
        "coproc echo NEW >> {file}; sleep 0.3; wait $COPROC_PID; cat {file}"
    ));
    assert_eq!(stdout, "OLD\nNEW\n");
    let (stdout, _, _) = rubash_c(&format!(
        "coproc bash -c \"echo O; echo E >&2\" &> {file}; sleep 0.3; wait $COPROC_PID; cat {file}"
    ));
    assert_eq!(stdout, "O\nE\n");
    let (stdout, _, _) =
        rubash_c("coproc echo hi; cat <&${COPROC[0]}; wait $COPROC_PID; echo rc=$?");
    assert_eq!(stdout, "hi\nrc=0\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1644_coproc_dup_stdout_to_stderr_explicit_fd_spelling() {
    // The `1>&2` spelling takes the same dup arm as `>&2`
    // (absorbed from PR #1712, commit 331c7435).
    let (stdout, stderr, code) = rubash_c("coproc echo err 1>&2; wait $COPROC_PID");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "err\n");
    assert_eq!(code, Some(0));
}
