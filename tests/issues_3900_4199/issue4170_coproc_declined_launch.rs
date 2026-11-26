//! Issue rubash#4170 — a coproc launch whose parent-side redirect
//! walk declines a redirect (parameter-expansion error, ambiguous
//! redirect, ordinary open failure) must still register the coproc
//! and report statement status 0.
//!
//! GNU applies a coproc's redirects in the forked child
//! (execute_cmd.c:2546 execute_coproc -> execute_in_subshell), so a
//! do_redirections failure there kills only the child: the parent
//! registers the coproc (coproc_setvars) and execute_coproc returns
//! EXECUTION_SUCCESS once the fork succeeds (execute_cmd.c:2592).
//! The child reports the diagnostic through fd 2 as bound by the
//! list's earlier entries (redir.c:135 redirection_error) and exits
//! 1 without running the body; `wait` reaps 1.
//!
//! Rubash's parent-side walk (apply_coproc_redirects) cannot fork
//! first. Before this fix its Declined disposition created no
//! coproc and failed the statement with status 1 (and the plain
//! open failures escaped as an Err with the same statement-level
//! result). The walk now reports the diagnostic through the fd-2
//! binding in force at the failure point and returns Declined; the
//! caller spawns a stillborn child (exits 1, body never runs) so
//! the ordinary spawn/registration path runs and the statement
//! reports 0.
//!
//! Verified against the cloud GNU Bash 5.3.20 oracle via script
//! files (statement status, COPROC_PID / NAME_PID registration,
//! wait status, diagnostic text and routing, created files).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4170-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` as probe.sh in a fresh tempdir; return (stdout,
/// stderr, exit code, sorted names of files the run left in the
/// tempdir besides probe.sh, content of diag.txt if present).
fn run_script(
    tag: &str,
    script: &str,
) -> (String, String, Option<i32>, Vec<String>, Option<String>) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read tempdir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "probe.sh")
        .collect();
    names.sort();
    let diag_txt = std::fs::read_to_string(dir.join("diag.txt")).ok();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code, names, diag_txt)
}

/// The shared tail: statement status, registration, reap status.
const TAIL: &str =
    "echo \"rc=$?\"\necho \"pid=${COPROC_PID:+SET}\"\nwait \"$COPROC_PID\"\necho \"wait_rc=$?\"\n";
const TAIL_OUT: &str = "rc=0\npid=SET\nwait_rc=1\n";

#[test]
fn issue4170_param_error_launch_registers() {
    let script = format!("coproc cat > ${{UNSET1:?qmsg}}\n{TAIL}");
    let (out, err, code, files, _) = run_script("param", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 1: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4170_nounset_launch_registers() {
    let script = format!("set -u\ncoproc cat > $UNSET2\n{TAIL}");
    let (out, err, code, files, _) = run_script("nounset", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 2: UNSET2: unbound variable\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no target files: {files:?}");
}

#[test]
fn issue4170_badsub_launch_registers() {
    let script = format!("coproc cat > pre${{-x}}\n{TAIL}");
    let (out, err, code, files, _) = run_script("badsub", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 1: pre${-x}: bad substitution\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `pre` target: {files:?}");
}

#[test]
fn issue4170_open_failure_launch_registers() {
    let script = format!("coproc cat > /nonexistent-dir-xyz/f\n{TAIL}");
    let (out, err, code, files, _) = run_script("openfail", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_input_open_failure_launch_registers() {
    let script = format!("coproc cat < /nonexistent-dir-xyz/f\n{TAIL}");
    let (out, err, code, files, _) = run_script("infail", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_ambiguous_launch_registers() {
    let script = format!("coproc cat 2>&word\n{TAIL}");
    let (out, err, code, files, _) = run_script("ambig", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 1: word: ambiguous redirect\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_numbered_ambiguous_launch_registers() {
    let script = format!("coproc cat 3>&word\n{TAIL}");
    let (out, err, code, files, _) = run_script("numambig", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 1: word: ambiguous redirect\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_input_param_error_launch_registers() {
    // The #4099 hang shape: an erroring redirect_in word. GNU
    // reports it from the child and the launch still registers.
    let script = format!("coproc cat <${{UNSET1:?qmsg}}\n{TAIL}");
    let (out, err, code, files, _) = run_script("inparam", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(err, "probe.sh: line 1: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4170_open_failure_diagnostic_follows_bound_fd2() {
    // GNU's child reports through fd 2 as bound at the failure
    // point: an earlier `2>diag.txt` captures the diagnostic and
    // the shell's own stderr stays clean.
    let (out, err, code, files, diag) = run_script(
        "route",
        "coproc cat 2>diag.txt > /nonexistent-dir-xyz/f\necho \"rc=$?\"\necho \"pid=${COPROC_PID:+SET}\"\n",
    );
    assert_eq!(out, "rc=0\npid=SET\n");
    assert_eq!(err, "");
    assert_eq!(
        diag.as_deref(),
        Some("probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n")
    );
    assert_eq!(code, Some(0));
    assert_eq!(files, vec!["diag.txt".to_string()]);
}

#[test]
fn issue4170_param_error_diagnostic_follows_bound_fd2() {
    let (out, err, code, files, diag) = run_script(
        "route_param",
        "coproc cat 2>diag.txt > ${UNSET1:?qmsg}\necho \"rc=$?\"\necho \"pid=${COPROC_PID:+SET}\"\n",
    );
    assert_eq!(out, "rc=0\npid=SET\n");
    assert_eq!(err, "");
    assert_eq!(diag.as_deref(), Some("probe.sh: line 1: UNSET1: qmsg\n"));
    assert_eq!(code, Some(0));
    assert_eq!(files, vec!["diag.txt".to_string()]);
}

#[test]
fn issue4170_earlier_redirect_side_effects_stand() {
    // List order: the good redirect before the failing one is
    // applied (its file exists), exactly as in GNU's child.
    let script = format!("coproc cat >good.txt > /nonexistent-dir-xyz/f\n{TAIL}");
    let (out, err, code, files, _) = run_script("goodbad", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert_eq!(files, vec!["good.txt".to_string()]);
}

#[test]
fn issue4170_body_never_runs() {
    // A direct-route (foreign program) body: the doomed launch
    // must not execute it — no sentinel appears.
    let script = format!("coproc touch sentinel > /nonexistent-dir-xyz/f\n{TAIL}");
    let (out, err, code, files, _) = run_script("norun", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "body ran or stray files: {files:?}");
}

#[test]
fn issue4170_named_group_launch_registers() {
    let (out, err, code, files, _) = run_script(
        "named",
        "coproc NC { cat; } > /nonexistent-dir-xyz/f\necho \"rc=$?\"\necho \"pid=${NC_PID:+SET} arr=${NC[@]+SET}\"\nwait \"$NC_PID\"\necho \"wait_rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\npid=SET arr=SET\nwait_rc=1\n");
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_inverted_doomed_launch() {
    // GNU execute_coproc returns invert ? EXECUTION_FAILURE :
    // EXECUTION_SUCCESS — a doomed launch under `!` reports 1
    // and still registers.
    let (out, err, code, files, _) = run_script(
        "invert",
        "! coproc cat > /nonexistent-dir-xyz/f\necho \"rc=$?\"\necho \"pid=${COPROC_PID:+SET}\"\n",
    );
    assert_eq!(out, "rc=1\npid=SET\n");
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_read_end_sees_eof() {
    // The registered read end EOFs when the stillborn child exits
    // (its stdout was the coproc pipe writer).
    let (out, err, code, files, _) = run_script(
        "eof",
        "coproc cat > /nonexistent-dir-xyz/f\necho \"rc=$?\"\nread -t 5 line <&\"${COPROC[0]}\"\necho \"read_rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\nread_rc=1\n");
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_jobs_lists_doomed_coproc() {
    let (out, err, code, files, _) = run_script(
        "jobs",
        "coproc cat > /nonexistent-dir-xyz/f\njobs\necho \"rc=$?\"\n",
    );
    assert!(
        out.contains("coproc cat"),
        "jobs must list the doomed coproc: {out:?}"
    );
    assert!(out.contains("rc=0"), "jobs rc line: {out:?}");
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-dir-xyz/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no files: {files:?}");
}

#[test]
fn issue4170_noclobber_launch_registers() {
    // A noclobber refusal is an ordinary do_redirections failure
    // in the child (redir.c redirection_error), same disposition.
    let script = format!("touch existing\nset -C\ncoproc cat > existing\n{TAIL}");
    let (out, err, code, files, _) = run_script("noclobber", &script);
    assert_eq!(out, TAIL_OUT);
    assert_eq!(
        err,
        "probe.sh: line 3: existing: cannot overwrite existing file\n"
    );
    assert_eq!(code, Some(0));
    assert_eq!(files, vec!["existing".to_string()]);
}
