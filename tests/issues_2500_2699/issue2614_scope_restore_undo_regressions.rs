//! rubash#2614 — the compound scope's restore set leaked the closes
//! its `{var}` pass performed in the fd table: a move-form
//! `{v}>&9-` (or a pure `{v}>&-`) inside `{ ...; }` closed the
//! parent's fd 9 for good, where GNU registers the close in the
//! command's undo chain (redir.c do_redirections — the r_move_* dup
//! case saves the source via add_undo_redirect (redir_fd,
//! r_close_this, -1); the close case saves the descriptor it closes)
//! and undo_redirections restores it when the compound finishes.
//!
//! The fix makes the scope keep the same records: every `{var}`
//! entry's close targets are snapshotted before the applier performs
//! them and replayed on every exit, and the scope-end cleanup closes
//! exactly the descriptors the pass bound (the rubash#2281
//! discipline), never a descriptor a close-form variable merely
//! names. The numbered openers' move arms follow GNU's activity gate
//! for the source's undo record (rubash#1235: the close is undone
//! only if the move's TARGET was open at apply time —
//! add_undo_redirect for the source is gated on fcntl(redirector)):
//! the input opener now saves an activity-gated source, and the
//! output opener no longer saves it unconditionally.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (the oracle
//! binary, script-file probes). FD-ENVIRONMENT SENSITIVITY: the
//! drivers run as script files with stdin null and only fds 0/1/2
//! inherited, like the neighbouring fd pins; the pins assert file
//! contents, openness probes and SET state, never exact `{var}`
//! numbers (GNU's own undo-save copies shift allocations on
//! in-process faces).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2614-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as the shell's main script file in a fresh scratch
/// directory; return (stdout, stderr, exit code, scratch dir).
fn run_case(tag: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = scratch(tag);
    fs::write(dir.join("case.sh"), script).expect("write case script");
    let output = Command::new(RUBASH)
        .arg("case.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash case");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

fn file(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("read {name}"))
}

/// D1 (the filed reproducer): the group's move-form `{v}>&9-` closes
/// fd 9 only for the group's duration; the parent's binding is
/// restored, so POST lands in seed.txt after W10.
#[test]
fn group_move_close_restores_parent_fd() {
    let (stdout, stderr, rc, dir) = run_case(
        "d1",
        "exec 9>seed.txt\n{ echo W10 >&$v; } {v}>&9-\necho POST >&9\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file(&dir, "seed.txt"), "W10\nPOST\n");
}

/// Control (SAME before the fix): the same move-close inside a
/// subshell — the close dies with the subshell either way.
#[test]
fn subshell_move_close_is_a_control() {
    let (stdout, stderr, rc, dir) = run_case(
        "subshell",
        "exec 9>seed.txt\n( echo W10 >&$v ) {v}>&9-\necho POST >&9\ncat seed.txt\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "W10\nPOST\n");
    assert_eq!(file(&dir, "seed.txt"), "W10\nPOST\n");
}

/// Control (SAME before the fix): the same move-close on a function
/// call — the function's own close machinery restores fd 9.
#[test]
fn function_move_close_is_a_control() {
    let (stdout, stderr, rc, dir) = run_case(
        "function",
        "exec 9>seed.txt\nwf() { echo W10 >&$v; }\nwf {v}>&9-\necho POST >&9\ncat seed.txt\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "W10\nPOST\n");
    assert_eq!(file(&dir, "seed.txt"), "W10\nPOST\n");
}

/// The move source is a `{var}` holding a regular file (not an
/// ambient numbered fd): the group moves it onto `$w`, and the
/// parent's `$u` descriptor must still be open afterwards.
#[test]
fn group_move_close_of_regular_file_var_restores() {
    let (stdout, stderr, rc, dir) = run_case(
        "filevar",
        "exec {u}>orig.txt\n[ -n \"$u\" ] && echo \"u-set\"\n{ echo MOVED >&$w; } {w}>&$u-\necho AFTER >&$u 2>/dev/null && echo \"u-open\" || echo \"u-closed\"\ncat orig.txt\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "u-set\nu-open\nMOVED\nAFTER\n");
    assert_eq!(file(&dir, "orig.txt"), "MOVED\nAFTER\n");
}

/// Close-then-reopen inside the group's own list: the move closes
/// fd 9, a later numbered entry rebinds it for the body, and at
/// scope end the parent's ORIGINAL binding (not the reopened one,
/// not the closed state) is what remains.
#[test]
fn group_close_then_reopen_in_list_restores_original() {
    let (stdout, stderr, rc, dir) = run_case(
        "reopen",
        "exec 9>seed.txt\n{ echo FIRST >&$v; echo SECOND >&9; } {v}>&9- 9>reopened.txt\necho POST >&9\necho \"seed=[$(cat seed.txt)]\"\necho \"reopened=[$(cat reopened.txt)]\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "seed=[FIRST\nPOST]\nreopened=[SECOND]\n");
    assert_eq!(file(&dir, "seed.txt"), "FIRST\nPOST\n");
    assert_eq!(file(&dir, "reopened.txt"), "SECOND\n");
}

/// The pure close form: `{v}>&-` with v naming the parent's fd 9
/// closes it for the group only (GNU saves the descriptor a close
/// names, redir.c r_close_this); the scope-end cleanup must not
/// re-close it through the variable's value.
#[test]
fn group_pure_close_form_restores() {
    let (stdout, stderr, rc, dir) = run_case(
        "pureclose",
        "exec 9>seed.txt\nv=9\n{ echo IN; } {v}>&-\necho POST >&9 2>/dev/null && echo \"nine-open\" || echo \"nine-closed\"\ncat seed.txt\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "IN\nnine-open\nPOST\n");
    assert_eq!(file(&dir, "seed.txt"), "POST\n");
}

/// Nested groups: the inner group's move-close restores fd 9 at the
/// INNER scope's end, so the outer body and the parent both still
/// write to the seed binding.
#[test]
fn nested_groups_inner_move_close_restores() {
    let (stdout, stderr, rc, dir) = run_case(
        "nest1",
        "exec 9>seed.txt\n{ { echo INNER >&$w; } {w}>&9-; echo MID >&9; }\necho POST >&9\ncat seed.txt\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "INNER\nMID\nPOST\n");
    assert_eq!(file(&dir, "seed.txt"), "INNER\nMID\nPOST\n");
}

/// Nested groups composing with an outer binding: the outer group
/// rebinds fd 9 to outer.txt; the inner move-close's snapshot is
/// THAT binding (restored at the inner end, so MID lands in
/// outer.txt), and the outer scope restores the seed binding.
#[test]
fn nested_groups_compose_with_outer_binding() {
    let (stdout, stderr, rc, dir) = run_case(
        "nest2",
        "exec 9>seed.txt\n{ { echo INNER >&$w; } {w}>&9-; echo MID >&9; } 9>outer.txt\necho POST >&9\necho \"seed=[$(cat seed.txt)]\"\necho \"outer=[$(cat outer.txt)]\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "seed=[POST]\nouter=[INNER\nMID]\n");
    assert_eq!(file(&dir, "seed.txt"), "POST\n");
    assert_eq!(file(&dir, "outer.txt"), "INNER\nMID\n");
}

/// GNU's activity gate (rubash#1235): a NUMBERED move onto a target
/// that was closed at apply time closes the source for good — the
/// scope must NOT restore fd 9 here.
#[test]
fn numbered_move_onto_closed_target_closes_source_for_good() {
    let (stdout, stderr, rc, dir) = run_case(
        "num-closed",
        "exec 9>seed.txt\n{ echo A >&8; } 8>&9-\necho POST9 >&9 2>/dev/null && echo \"9-open\" || echo \"9-closed\"\n{ : >&8; } 2>/dev/null && echo \"8-open\" || echo \"8-closed\"\necho \"seed=[$(cat seed.txt)]\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "the failed POST9 redirect reports EBADF, stderr: {stderr}"
    );
    assert_eq!(stdout, "9-closed\n8-closed\nseed=[A]\n");
    assert_eq!(file(&dir, "seed.txt"), "A\n");
}

/// The other half of the gate: a numbered move onto an OPEN target
/// restores both slots — fd 9 to the seed binding, fd 8 to its own
/// earlier binding.
#[test]
fn numbered_move_onto_open_target_restores_both() {
    let (stdout, stderr, rc, dir) = run_case(
        "num-open",
        "exec 9>seed.txt\nexec 8>other.txt\n{ echo A >&8; } 8>&9-\necho POST9 >&9\necho POST8 >&8\necho \"seed=[$(cat seed.txt)] other=[$(cat other.txt)]\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "seed=[A\nPOST9] other=[POST8]\n");
    assert_eq!(file(&dir, "seed.txt"), "A\nPOST9\n");
    assert_eq!(file(&dir, "other.txt"), "POST8\n");
}

/// The input opener's half of the gate: a numbered INPUT move onto
/// an open target restores the source (before the fix the input
/// opener never saved the source at all, so fd 9 leaked closed).
#[test]
fn numbered_input_move_onto_open_target_restores_source() {
    let (stdout, stderr, rc, _dir) = run_case(
        "in-open",
        "printf 'D9\\n' > d9.txt\nprintf 'D8\\n' > d8.txt\nexec 9<d9.txt\nexec 8<d8.txt\n{ IFS= read -r -u 8 line; echo \"via8=[$line]\"; } 8<&9-\n{ : <&9; } 2>/dev/null && echo \"nine-open\" || echo \"nine-closed\"\n{ : <&8; } 2>/dev/null && echo \"eight-open\" || echo \"eight-closed\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "via8=[D9]\nnine-open\neight-open\n");
}

/// And its closed-target half: a numbered input move onto a closed
/// target closes the source for good.
#[test]
fn numbered_input_move_onto_closed_target_closes_source_for_good() {
    let (stdout, stderr, rc, _dir) = run_case(
        "in-closed",
        "printf 'D9\\n' > d9.txt\nexec 9<d9.txt\n{ IFS= read -r -u 7 line; echo \"via7=[$line]\"; } 7<&9-\n{ : <&9; } 2>/dev/null && echo \"nine-open\" || echo \"nine-closed\"\n{ : <&7; } 2>/dev/null && echo \"seven-open\" || echo \"seven-closed\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "via7=[D9]\nnine-closed\nseven-closed\n");
}
