//! rubash#3195 — a heredoc / here-string whose explicit redirector
//! sits at or above the RLIMIT_NOFILE *soft* limit must fail
//! `Bad file descriptor` with status 1 and never bind; rubash bound
//! it silently on every command form (the #2635 residual: the limit
//! gates landed on the file/dup walks, but heredoc entries bind
//! through the textual body channels, not those walks).
//!
//! GNU redir.c: the heredoc body is materialized first
//! (here_document_to_fd, redir.c:1072), then the dup2 onto the
//! redirector fails EBADF when the redirector is at/above the soft
//! limit (redir.c:1120), and redirection_error renders
//! `<fd>: Bad file descriptor` (redir.c:135, the EBADF naming at
//! redir.c:166-175). The failure lands at the entry's own position
//! in the list: earlier entries' side effects stand, later entries
//! never run. `{var}` heredocs are exempt by construction — their
//! allocator stays below the limit.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes under `ulimit -n 16` (fd 15 passes; 16/17
//! fail), mirroring the issue2635 pin's harness.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3195-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr).
fn rubash_script(dir: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn file_contents(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok()
}

/// How many diagnostic lines END with `{fd}: Bad file descriptor` —
/// the redirect-failure shape. (A later `read -u {fd}` failure reads
/// `read: {fd}: invalid file descriptor: Bad file descriptor`, which
/// does not end with the bare fd form, so the two never conflate.)
fn redirect_diag_count(stderr: &str, fd: u32) -> usize {
    let tail = format!("{fd}: Bad file descriptor");
    stderr.lines().filter(|line| line.ends_with(&tail)).count()
}

#[test]
fn issue3195_bare_exec_heredoc_at_limit_fails() {
    // The issue's seed shape: the redirector never binds, so the
    // follow-up read reports the descriptor invalid.
    let dir = scratch("exec-heredoc");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 16<<EOF\nhello\nEOF\nread -u 16 line; echo \"line=$line\"\n",
    );
    assert_eq!(stdout, "line=\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
    assert!(
        stderr.contains("read: 16: invalid file descriptor"),
        "got: {stderr:?}"
    );
}

#[test]
fn issue3195_bare_exec_herestring_at_limit_fails() {
    let dir = scratch("exec-herestring");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 16<<<\"hello\"\nread -u 16 line; echo \"line=$line\"\n",
    );
    assert_eq!(stdout, "line=\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
    assert!(
        stderr.contains("read: 16: invalid file descriptor"),
        "got: {stderr:?}"
    );
}

#[test]
fn issue3195_bare_exec_heredoc_below_limit_binds() {
    // Control: fd 15 is the highest legal slot under a limit of 16.
    let dir = scratch("exec-below-limit");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 15<<EOF\nhello\nEOF\nread -u 15 line; echo \"line=$line\"\n",
    );
    assert_eq!(stdout, "line=hello\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3195_failed_exec_heredoc_undoes_earlier_binds() {
    // `exec 15<<A 16<<B`: the walk fails at 16 and the no-operand
    // exec's whole redirection set is undone — fd 15 does not stay
    // bound either (GNU probe: the read of 15 reports it invalid).
    let dir = scratch("exec-undo");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 15<<A 16<<B\nlo\nA\nhi\nB\nread -u 15 x; echo \"x=$x\"\n",
    );
    assert_eq!(stdout, "x=\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
    assert!(
        stderr.contains("read: 15: invalid file descriptor"),
        "got: {stderr:?}"
    );
}

#[test]
fn issue3195_exec_with_command_at_limit_never_runs() {
    let dir = scratch("exec-command");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec cat 16<<EOF\nhello\nEOF\necho notreached\n",
    );
    assert_eq!(stdout, "notreached\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_builtin_at_limit_never_runs() {
    let dir = scratch("builtin-echo");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\necho hi 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_no_output_builtin_at_limit_fails() {
    let dir = scratch("builtin-colon");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n: 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_builtin_read_at_limit_fails() {
    // The builtin's own list carries the heredoc: GNU reports the
    // redirect failure itself (status 1), not a read error.
    let dir = scratch("builtin-read");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nread -u 16 line 16<<EOF\nhello\nEOF\necho \"line=$line rc=$?\"\n",
    );
    assert_eq!(stdout, "line= rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_simple_command_herestring_at_limit_never_runs() {
    let dir = scratch("simple-herestring");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\necho hi 16<<<\"hello\"; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_group_at_limit_never_runs() {
    let dir = scratch("group");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n{ read -u 16 line; echo \"line=$line\"; } 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_while_at_limit_never_runs() {
    let dir = scratch("while");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nwhile read -u 16 l; do echo \"line=$l\"; done 16<<EOF\nhello\nEOF\necho done\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_subshell_at_limit_never_runs() {
    let dir = scratch("subshell");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n( read -u 16 l; echo \"line=$l\" ) 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_function_call_at_limit_never_runs() {
    let dir = scratch("function-call");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nf() { read -u 16 l; echo \"line=$l\"; }\nf 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_arith_compound_at_limit_never_runs() {
    let dir = scratch("arith");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n((x=1)) 16<<EOF\nhello\nEOF\necho \"x=$x rc=$?\"\n",
    );
    assert_eq!(stdout, "x= rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_first_over_limit_entry_in_list_fails() {
    // Two heredocs on one group, the FIRST already over the limit:
    // the walk stops there and names that entry's redirector.
    let dir = scratch("first-entry");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n{ read -u 17 l; echo \"line=$l\"; } 17<<EOF 16<<EOF2\nhello\nEOF\nsecond\nEOF2\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 17), 1, "got: {stderr:?}");
    assert_eq!(redirect_diag_count(&stderr, 16), 0, "got: {stderr:?}");
}

#[test]
fn issue3195_failure_lands_at_its_list_position() {
    // `>created.txt` BEFORE the heredoc: the open side effect stands.
    let dir = scratch("position-after");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\necho hi >created.txt 16<<EOF\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
    assert_eq!(file_contents(&dir, "created.txt").as_deref(), Some(""));
    // The heredoc BEFORE `>created.txt`: the walk stops, no creation.
    let dir = scratch("position-before");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\necho hi 16<<EOF >created.txt\nhello\nEOF\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
    assert_eq!(file_contents(&dir, "created.txt"), None);
}

#[test]
fn issue3195_assignment_command_at_limit_fails() {
    // A standalone-assignment (null) command carrying the heredoc:
    // the assignment stands (it is bound during word expansion,
    // before do_redirections), the command still fails at the entry.
    let dir = scratch("assignment");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nv=$(read -u 16 l; echo \"line=$l\") 16<<EOF\nhello\nEOF\necho \"v=$v rc=$?\"\n",
    );
    assert_eq!(stdout, "v=line= rc=1\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}

#[test]
fn issue3195_fd_var_heredoc_allocates_below_limit() {
    // Control: a `{var}` heredoc allocates a legal descriptor under
    // the limit and the body stays readable through it.
    let dir = scratch("fd-var");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec {v}<<EOF\nhello\nEOF\nread -u \"$v\" line; echo \"line=$line\"\nif [ \"$v\" -lt 16 ]; then echo below; fi\n",
    );
    assert_eq!(stdout, "line=hello\nbelow\n");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue3195_pipeline_stage_at_limit_fails_alone() {
    // The stage's redirect failure kills only the stage: the pipe
    // delivers EOF and the second stage reports 0 bytes.
    let dir = scratch("pipeline-stage");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\ncat 16<<EOF | wc -c\nhello\nEOF\n",
    );
    assert_eq!(stdout, "0\n");
    assert_eq!(redirect_diag_count(&stderr, 16), 1, "got: {stderr:?}");
}
