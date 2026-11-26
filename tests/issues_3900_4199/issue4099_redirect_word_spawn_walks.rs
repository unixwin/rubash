//! Issue rubash#4099 — redirect-word parameter-expansion errors
//! (`:?`, nounset, bad substitution) across the foreground gate and
//! the two spawn-time redirect walks.
//!
//! The issue's headline shape — `:?` in the FIRST redirect word of
//! an ordinary command, a nounset word second — was misjudged as
//! script-fatal on pristine `ff966865`; PR #4086's unification of
//! redirect-word errors into `reject_invalid_redirects` fixed the
//! foreground face. The residual lived in the spawn-time walks,
//! which expand redirect words in the parent through the silent
//! `expand_redirect_target` without the gate's verdict:
//! - background stdio resolution manufactured the `:?` MESSAGE as
//!   the target file (`qmsg` created in the cwd), consumed the
//!   redirect, and lost or displaced the diagnostic;
//! - the coproc walk did the same, and a nounset word's empty
//!   expansion surfaced as a bogus `: Is a directory` open error.
//!
//! GNU anchors: redir.c:298 redirection_expand ->
//! expand_words_no_vars (the verdict is a property of the word);
//! execute_cmd.c:1761-1763 (the async child's do_redirections runs
//! after the fork, so the first error in list order is reported
//! through fd 2 as bound at that point and only the child dies);
//! execute_cmd.c:2477 execute_coproc (redirects applied in the
//! child via execute_in_subshell at :2555).
//!
//! Statement-status face (closed by rubash#4170): GNU's coproc
//! STATEMENT returns EXECUTION_SUCCESS once the child is forked
//! (execute_cmd.c:2592), whatever the child's redirects do.
//! Rubash's parent-side walk cannot fork first, so an erroring
//! coproc redirect word takes the walk's Declined disposition —
//! the diagnostic is printed once and the caller spawns a
//! stillborn child, so the coproc registers and the statement
//! reports 0. The pins below assert the diagnostic /
//! continuation / no-bogus-target faces together with that
//! status; tests/issue4170_coproc_declined_launch.rs pins the
//! registration / wait / routing faces.
//!
//! Verified against the cloud GNU Bash 5.3.20 oracle via script
//! files (stdout, stderr, exit status, and created files compared).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4099-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` as probe.sh in a fresh tempdir; return (stdout,
/// stderr, exit code, sorted names of files the run left in the
/// tempdir besides probe.sh, content of a file named err.txt if
/// the script created one).
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
    let err_txt = std::fs::read_to_string(dir.join("err.txt")).ok();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code, names, err_txt)
}

#[test]
fn issue4099_qmark_first_word_external_continues() {
    // The issue's repro (oracle dossier qB): the `:?` word leads,
    // the nounset word follows; GNU fails the command and continues.
    let (out, err, code, files, _) = run_script(
        "qb",
        "set -u\ncat >${UNSET1:?qmsg} 2>$UNSET2\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(
        files.is_empty(),
        "no target files may be created: {files:?}"
    );
}

#[test]
fn issue4099_nounset_first_word_external_continues() {
    // The qA interleave (the #4006-fixed control): the plain
    // nounset word leads and its diagnostic wins.
    let (out, err, code, files, _) = run_script(
        "qa",
        "set -u\ncat >$UNSET1 2>${UNSET2:?qmsg}\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: unbound variable\n");
    assert_eq!(code, Some(0));
    assert!(
        files.is_empty(),
        "no target files may be created: {files:?}"
    );
}

#[test]
fn issue4099_assignment_word_qmark_is_fatal() {
    // The contrast face: `:?` in an ASSIGNMENT word (no command
    // name) is shell-fatal in GNU — the boundary is the context,
    // not the operator.
    let (out, err, code, _, _) = run_script("assign", "x=${UNSET1:?qmsg}\necho \"rc=$?\"\n");
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 1: UNSET1: qmsg\n");
    assert_eq!(code, Some(1));
}

#[test]
fn issue4099_builtin_redirect_qmark_is_fatal() {
    // A builtin's redirects run in the main shell: fatal, like GNU.
    let (out, err, code, _, _) = run_script(
        "builtin",
        "set -u\necho hi >${UNSET1:?qmsg} 2>$UNSET2\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(1));
}

#[test]
fn issue4099_background_reports_first_error_in_list_order() {
    // Pre-fix the parent walk created a file named `qmsg` from the
    // `:?` word, consumed it, and the child reported the SECOND
    // word's nounset error instead.
    let (out, err, code, files, _) = run_script(
        "bg",
        "set -u\ncat >${UNSET1:?qmsg} 2>$UNSET2 &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_background_qmark_not_swallowed_by_later_devnull() {
    // The error point precedes the `2>/dev/null` entry in list
    // order, so the diagnostic reaches the shell's stderr even
    // though fd 2 is redirected later in the same list.
    let (out, err, code, files, _) = run_script(
        "bg_qonly",
        "set -u\ncat >${UNSET1:?qmsg} 2>/dev/null &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_background_nounset_first_word_reports() {
    let (out, err, code, files, _) = run_script(
        "bg_qa",
        "set -u\ncat >$UNSET1 2>${UNSET2:?qmsg} &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: unbound variable\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_background_qmark_after_devnull_stays_silent() {
    // Here the `2>/dev/null` entry precedes the error point, so
    // GNU routes the diagnostic to /dev/null: silence is correct.
    let (out, err, code, files, _) = run_script(
        "bg_qsecond",
        "set -u\ncat 2>/dev/null >${UNSET1:?qmsg} &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_background_later_redirect_file_not_created() {
    // The child dies at the first word, so the later `2>err.txt`
    // is never applied (and the parent walk must not create it
    // either while resolving the spawn stdio).
    let (out, err, code, files, _) = run_script(
        "bg_nosa",
        "set -u\ncat >${UNSET1:?qmsg} 2>err.txt &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "err.txt must not be created: {files:?}");
}

#[test]
fn issue4099_background_diagnostic_routes_into_earlier_err_file() {
    // The `2>err.txt` entry precedes the error point: GNU's child
    // reports through it, and the shell's own stderr stays empty.
    let (out, err, code, files, err_txt) = run_script(
        "bg_route",
        "set -u\ncat 2>err.txt >${UNSET1:?qmsg} &\nwait\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
    assert_eq!(files, vec!["err.txt".to_string()]);
    assert_eq!(err_txt.as_deref(), Some("probe.sh: line 2: UNSET1: qmsg\n"));
}

#[test]
fn issue4099_background_badsub_reports_without_bogus_file() {
    // Pre-fix the walk expanded `pre${-x}` to `pre` and created it.
    let (out, err, code, files, _) =
        run_script("bg_badsub", "cat > pre${-x} &\nwait\necho \"rc=$?\"\n");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 1: pre${-x}: bad substitution\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `pre` target: {files:?}");
}

#[test]
fn issue4099_coproc_qmark_reports_and_script_continues() {
    // Statement status is GNU's 0 since rubash#4170 (the Declined
    // launch still registers); the diagnostic, the continuation,
    // and the absence of a bogus target are the #4099 faces.
    let (out, err, code, files, _) = run_script(
        "cop",
        "set -u\ncoproc cat >${UNSET1:?qmsg} 2>$UNSET2\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_coproc_nounset_reports_unbound() {
    // Pre-fix this printed `: Is a directory` (the empty expansion
    // opened as the session directory).
    let (out, err, code, files, _) =
        run_script("cop_noun", "set -u\ncoproc cat >$UNSET1\necho \"rc=$?\"\n");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 2: UNSET1: unbound variable\n");
    assert_eq!(code, Some(0));
    assert!(
        files.is_empty(),
        "no target files may be created: {files:?}"
    );
}

#[test]
fn issue4099_coproc_input_word_qmark_reports() {
    let (out, err, code, files, _) =
        run_script("cop_input", "coproc cat <${UNSET1:?qmsg}\necho \"rc=$?\"\n");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 1: UNSET1: qmsg\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `qmsg` target: {files:?}");
}

#[test]
fn issue4099_coproc_badsub_reports_and_script_continues() {
    // Pre-fix the walk created `pre` and the script aborted.
    let (out, err, code, files, _) =
        run_script("cop_badsub", "coproc cat > pre${-x}\necho \"rc=$?\"\n");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 1: pre${-x}: bad substitution\n");
    assert_eq!(code, Some(0));
    assert!(files.is_empty(), "no bogus `pre` target: {files:?}");
}
