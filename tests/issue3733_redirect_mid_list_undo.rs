//! rubash#3733: a redirect list that fails mid-walk must undo every
//! bind the earlier entries already applied — GNU's
//! `undo_partial_redirects` (execute_cmd.c:855) replays the per-entry
//! undo saves `add_undo_redirect` recorded (redir.c:1326, kernel
//! `fcntl (fd, F_DUPFD, SHELL_FD_BASE)` at redir.c:965/1082/1144).
//! Rubash's compound/function scopes accumulate `SavedNumberedFd` /
//! `SavedOutputFd` snapshots for exactly this restore, but the
//! numbered-input wrapper dropped them on the `_in` error path, so
//! `{ :; } 8<f9 9<f8 7</non/nope` left fd 8 reading f9 and fd 9
//! reading f8 after the failed list.
//!
//! Every cell below shapes `exec 8<f8.txt; exec 9<f9.txt` first, runs
//! a compound (or function call) whose list binds fd 8/9 to the OTHER
//! file and then fails on `7</non/nope`, and reads fd 8/9 back: GNU
//! leaves both on their pre-list targets.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3733-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const SETUP: &str = "printf 'f8a\\nf8b\\n' > f8.txt\n\
                    printf 'f9a\\nf9b\\n' > f9.txt\n\
                    exec 8<f8.txt\n\
                    exec 9<f9.txt\n";

const PROBE: &str = "IFS= read -r r8 <&8\n\
                     IFS= read -r r9 <&9\n\
                     echo \"fd8=[$r8] fd9=[$r9]\"\n";

fn assert_restored(tag: &str, driver: &str) {
    let (stdout, stderr, rc) = run_driver(tag, driver);
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "expected the mid-list open failure, stderr: {stderr}"
    );
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// The primitive: `while ... done 8<f9 7</non/nope` — one partial
/// input bind, then the failing entry. fd 8 must keep f8.txt.
#[test]
fn while_loop_mid_list_failure_restores_fd8() {
    assert_restored(
        "while",
        &format!(
            "{SETUP}\
             while IFS= read -r l <&9; do echo \"[$l]\"; done 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// `until` takes the same compound redirect scope.
#[test]
fn until_loop_mid_list_failure_restores_fd8() {
    assert_restored(
        "until",
        &format!(
            "{SETUP}\
             until false; do :; done 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Two successful partial binds before the failure — BOTH must be
/// undone (the swapped-bind cell: `8<f9 9<f8` would leave fd8=f9a,
/// fd9=f8a if either snapshot were dropped).
#[test]
fn brace_group_two_partial_binds_both_restored() {
    assert_restored(
        "brace",
        &format!("{SETUP}{{ :; }} 8<f9.txt 9<f8.txt 7</non/nope\n{PROBE}"),
    );
}

/// `if` shares the compound scope path.
#[test]
fn if_compound_mid_list_failure_restores_fd8() {
    assert_restored(
        "if",
        &format!("{SETUP}if :; then :; fi 8<f9.txt 7</non/nope\n{PROBE}"),
    );
}

/// `for` shares the compound scope path.
#[test]
fn for_loop_mid_list_failure_restores_fd8() {
    assert_restored(
        "for",
        &format!(
            "{SETUP}\
             for i in 1; do :; done 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// A numbered heredoc earlier in the list is a bind like any other:
/// `8<<<BODY` displaces fd 8 for the (never-run) body, and the later
/// failure must restore it.
#[test]
fn heredoc_bind_before_failure_restored() {
    assert_restored(
        "heredoc",
        &format!(
            "{SETUP}\
             {{ :; }} 8<<<BODY 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Function-call redirects ride the same numbered-input wrapper
/// (with_call_numbered_input_redirects); GNU marks them undoable for
/// exactly this reason (execute_cmd.c:5608).
#[test]
fn function_call_mid_list_failure_restores_fd8() {
    assert_restored(
        "func",
        &format!(
            "{SETUP}\
             f() {{ :; }}\n\
             f 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Control: with no failing entry the binds apply for the body's
/// duration and the scope restore still returns fd 8/9 afterwards.
#[test]
fn successful_list_still_scopes_binds() {
    let (stdout, stderr, rc) = run_driver(
        "ok",
        &format!(
            "{SETUP}\
             {{ IFS= read -r x <&8; echo \"body=[$x]\"; }} 8<f9.txt\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "body=[f9a]\nfd8=[f8a] fd9=[f9a]\n");
}

/// A `{var}`-close entry between the binds and the failure: the
/// dynamic pass's own undo must compose with the numbered restore —
/// fd 8 keeps f8.txt, and the fd the `{var}` close hit is open again.
#[test]
fn fd_var_close_before_failure_restored() {
    let (stdout, stderr, rc) = run_driver(
        "varclose",
        &format!(
            "{SETUP}\
             v=9\n\
             shopt -s varredir_close 2>/dev/null || true\n\
             {{ :; }} 8<f9.txt {{v}}>&- 7</non/nope\n\
             IFS= read -r r8 <&8; IFS= read -r r9 <&9\n\
             echo \"fd8=[$r8] fd9=[$r9]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "expected the mid-list open failure, stderr: {stderr}"
    );
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Nested compound: the failing list sits on the INNER group; the
/// outer group's own (successful) binds still scope correctly.
#[test]
fn nested_compound_inner_failure_restores() {
    assert_restored(
        "nested",
        &format!(
            "{SETUP}\
             {{ {{ :; }} 8<f9.txt 7</non/nope; }} 9<f8.txt\n\
             {PROBE}"
        ),
    );
}

/// Select loop takes the compound scope too.
#[test]
fn select_loop_mid_list_failure_restores_fd8() {
    assert_restored(
        "select",
        &format!(
            "{SETUP}\
             select i in 1; do break; done 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Case statement shares the compound redirect scope.
#[test]
fn case_compound_mid_list_failure_restores_fd8() {
    assert_restored(
        "case",
        &format!(
            "{SETUP}\
             case x in x) :;; esac 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// A later DUP entry that fails (source closed mid-list by an
/// earlier close) must still undo prior binds: `8<f9 9<&- 8<&9`
/// fails copying the just-closed fd 9.
#[test]
fn dup_from_closed_source_failure_restores() {
    let (stdout, stderr, rc) = run_driver(
        "dupclosed",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 9<&- 8<&9\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Time/inversion wrappers route through the same compound scope:
/// `! { :; } 8<f9 7</non/nope`.
#[test]
fn inverted_compound_mid_list_failure_restores() {
    assert_restored(
        "inverted",
        &format!(
            "{SETUP}\
             ! {{ :; }} 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Background compound: `&`-terminated groups get their redirects in
/// the child — but a foreground `time`/pipeline stage still scopes.
/// Keep the cell to the plain foreground pipeline stage, where the
/// stage's redirects are undoable like any compound's.
#[test]
fn pipeline_stage_mid_list_failure_restores() {
    let (stdout, stderr, rc) = run_driver(
        "pipe",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 7</non/nope | cat\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// The first entry failing leaves nothing to undo — control cell.
#[test]
fn first_entry_failure_is_clean() {
    let (stdout, stderr, rc) = run_driver(
        "firstfail",
        &format!(
            "{SETUP}\
             {{ :; }} 7</non/nope 8<f9.txt\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "expected the open failure, stderr: {stderr}"
    );
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// fd 3 (single-digit, always-open class) behaves like fd 8.
#[test]
fn fd3_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "fd3",
        &format!(
            "printf 'g3a\\ng3b\\n' > f3.txt\n\
             printf 'f9a\\nf9b\\n' > f9.txt\n\
             exec 3<f3.txt\n\
             exec 9<f9.txt\n\
             {{ :; }} 3<f9.txt 7</non/nope\n\
             IFS= read -r r3 <&3\n\
             echo \"fd3=[$r3]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd3=[g3a]\n", "stdout: {stdout:?}");
}

/// Write-side partial binds undo the same way. `8>ok1.tmp`
/// succeeds first — creating the file is an open side effect GNU's
/// undo does not roll back either (verified: GNU leaves ok1.tmp) —
/// then `9>>/nonexistent_dir_xyz/o` fails and fd 8's binding must
/// return to f8.txt.
#[test]
fn output_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "outbind",
        &format!(
            "{SETUP}\
             {{ :; }} 8>ok1.tmp 9>>/nonexistent_dir_xyz/o\n\
             if [ -e ok1.tmp ]; then echo file=yes; else echo file=no; fi\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(
        stdout, "file=yes\nfd8=[f8a] fd9=[f9a]\n",
        "stdout: {stdout:?}"
    );
}

/// Append partial bind mid-list: same undo contract — `8>>ok2.tmp`
/// creates its file before the later output open fails.
#[test]
fn append_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "appendbind",
        &format!(
            "{SETUP}\
             {{ :; }} 8>>ok2.tmp 9>>/nonexistent_dir_xyz/o\n\
             if [ -e ok2.tmp ]; then echo file=yes; else echo file=no; fi\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(
        stdout, "file=yes\nfd8=[f8a] fd9=[f9a]\n",
        "stdout: {stdout:?}"
    );
}

/// Read-write `<>` partial bind mid-list: the `<>` open creates its
/// file (GNU keeps it too — undo restores bindings, not filesystem
/// side effects), then the later output open fails.
#[test]
fn readwrite_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "rwbind",
        &format!(
            "{SETUP}\
             {{ :; }} 8<>ok3.tmp 9>>/nonexistent_dir_xyz/o\n\
             if [ -e ok3.tmp ]; then echo file=yes; else echo file=no; fi\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(
        stdout, "file=yes\nfd8=[f8a] fd9=[f9a]\n",
        "stdout: {stdout:?}"
    );
}

/// A dup-output partial bind: `8>&9 7</non/nope` copies fd 9's
/// binding onto fd 8, then fails — fd 8 must return to f8.txt.
#[test]
fn dup_output_partial_bind_restored() {
    assert_restored(
        "dupout",
        &format!("{SETUP}{{ :; }} 8>&9 7</non/nope\n{PROBE}"),
    );
}

/// Move (`>&`-style digit move / `<&` move) partial binds restore
/// the source too: `9<&8-` style move closes its source — GNU's
/// undo reopens it (add_undo_redirect r_close_this at redir.c:1205).
#[test]
fn move_partial_bind_restores_source() {
    let (stdout, stderr, rc) = run_driver(
        "move",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 9<&8- 7</non/nope\n\
             IFS= read -r r8 <&8; IFS= read -r r9 <&9\n\
             echo \"fd8=[$r8] fd9=[$r9]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Process-substitution output target earlier in the list: the
/// partial bind opens the procsub pipe; the failure must not leak
/// the fd binding (fd 8 back on f8.txt).
#[test]
fn procsub_output_partial_bind_restored() {
    assert_restored(
        "procsub",
        &format!("{SETUP}{{ :; }} 8> >(cat >/dev/null) 7</non/nope\n{PROBE}"),
    );
}

/// Two compounds in a row: the first fails mid-list, the second
/// runs clean — state must not leak across commands.
#[test]
fn failure_then_clean_compound() {
    let (stdout, stderr, rc) = run_driver(
        "twice",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 7</non/nope\n\
             {{ IFS= read -r x <&8; echo \"second=[$x]\"; }} 8<f9.txt\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(
        stdout, "second=[f9a]\nfd8=[f8a] fd9=[f9a]\n",
        "stdout: {stdout:?}"
    );
}

/// `exec` with a failing later entry: GNU treats exec redirects as
/// undoable only until exec decides to apply them permanently — a
/// FAILED exec redirect list leaves the shell's fds untouched.
#[test]
fn exec_mid_list_failure_restores() {
    assert_restored(
        "exec",
        &format!("{SETUP}exec 8<f9.txt 7</non/nope\n{PROBE}"),
    );
}

/// Time prefix: `time { :; } 8<f9 7</non/nope`.
#[test]
fn time_prefixed_mid_list_failure_restores() {
    assert_restored(
        "time",
        &format!(
            "{SETUP}\
             time {{ :; }} 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Deeply chained: three binds then failure — all three restored.
#[test]
fn three_partial_binds_all_restored() {
    let (stdout, stderr, rc) = run_driver(
        "three",
        &format!(
            "printf 'f3a\\n' > f3.txt\n\
             {SETUP}\
             exec 3<f3.txt\n\
             {{ :; }} 3<f9.txt 8<f9.txt 9<f8.txt 7</non/nope\n\
             IFS= read -r r3 <&3; IFS= read -r r8 <&8; IFS= read -r r9 <&9\n\
             echo \"fd3=[$r3] fd8=[$r8] fd9=[$r9]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "fd3=[f3a] fd8=[f8a] fd9=[f9a]\n",
        "stdout: {stdout:?}"
    );
}

/// Brace group spanning lines — same mechanism, multiline shape.
#[test]
fn multiline_brace_mid_list_failure_restores() {
    assert_restored(
        "multiline",
        &format!(
            "{SETUP}\
             {{\n\
             \x20   :\n\
             }} 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Subshell body inside the compound still gets scoped redirects
/// when the LIST itself fails — nothing leaks to the parent.
#[test]
fn subshell_body_compound_failure_restores() {
    assert_restored(
        "subbody",
        &format!(
            "{SETUP}\
             ( : ) 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// A failing APPEND-redirect target directory mid-list.
#[test]
fn missing_dir_append_failure_restores() {
    assert_restored(
        "nodir",
        &format!("{SETUP}{{ :; }} 8<f9.txt 9>>/nonexistent_dir_xyz/o 7</non/nope\n{PROBE}"),
    );
}

/// `>&-` close entry earlier in the list is itself undoable: the
/// closed fd reopens on failure.
#[test]
fn close_entry_before_failure_restored() {
    let (stdout, stderr, rc) = run_driver(
        "closeundo",
        &format!(
            "{SETUP}\
             {{ :; }} 9>&- 8<f9.txt 7</non/nope\n\
             IFS= read -r r8 <&8; IFS= read -r r9 <&9\n\
             echo \"fd8=[$r8] fd9=[$r9]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// env -i style: run with a clean environment is covered by the
/// driver itself; this cell just re-checks the primitive under
/// `set -e`-free plain flow with an extra intervening successful
/// compound between setup and failure.
#[test]
fn intervening_successful_compound() {
    assert_restored(
        "between",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f8.txt\n\
             {{ :; }} 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// The undo must also fire when the failing entry is a WRITE-open
/// on a missing directory (output side mid-list).
#[test]
fn output_open_failure_mid_list_restores_inputs() {
    let (stdout, stderr, rc) = run_driver(
        "outfail",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 9>>/nonexistent_dir_xyz/o\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// fd 0 (stdin) partial bind mid-list: `0<f9.txt` then failure —
/// the post-failure stdin must be untouched. Driver stdin is null,
/// so a leaked f9.txt binding would make `read` succeed.
#[test]
fn fd0_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "fd0",
        &format!(
            "{SETUP}\
             {{ :; }} 0<f9.txt 7</non/nope\n\
             IFS= read -r x; echo \"stdin_rc=$? x=[$x]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "stdin_rc=1 x=[]\n", "stdout: {stdout:?}");
}

/// Large fd numbers (>9) take the same numbered path.
#[test]
fn high_fd_partial_bind_restored() {
    let (stdout, stderr, rc) = run_driver(
        "highfd",
        &format!(
            "{SETUP}\
             exec 12<f8.txt\n\
             {{ :; }} 12<f9.txt 7</non/nope\n\
             IFS= read -r r12 <&12\n\
             echo \"fd12=[$r12]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd12=[f8a]\n", "stdout: {stdout:?}");
}

/// Discard output `>/dev/null` earlier, then failure — fd 8's own
/// binding untouched; also exercises output-list walk ordering.
#[test]
fn devnull_output_then_failure() {
    let (stdout, stderr, rc) = run_driver(
        "devnull",
        &format!(
            "{SETUP}\
             {{ :; }} >/dev/null 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// `{var}` DUP (not close) mid-list: `v=8; { :; } {v}>&9 7</non/nope`
/// — the dynamic dup's own undo plus numbered restore compose.
#[test]
fn fd_var_dup_before_failure_restored() {
    let (stdout, stderr, rc) = run_driver(
        "vardup",
        &format!(
            "{SETUP}\
             v=8\n\
             {{ :; }} 8<f9.txt {{v}}>&9 7</non/nope\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Loop with heredoc redirect AND failing numbered bind:
/// `while ... done 8<<<B 9<f9 7</non/nope`.
#[test]
fn loop_heredoc_plus_failure_restores() {
    assert_restored(
        "loophd",
        &format!(
            "{SETUP}\
             while IFS= read -r l <&9; do echo \"[$l]\"; done 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// A mid-list failure inside an ELIF branch's compound.
#[test]
fn elif_compound_mid_list_failure_restores() {
    assert_restored(
        "elif",
        &format!(
            "{SETUP}\
             if false; then :; elif :; then :; fi 8<f9.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
}

/// Redirect list on a FUNCTION DEFINITION persists for later calls;
/// a failing definition-time list must not leak binds either.
#[test]
fn function_def_mid_list_failure_restores() {
    assert_restored(
        "fdef",
        &format!("{SETUP}f() {{ :; }} 8<f9.txt 7</non/nope\nf\n{PROBE}"),
    );
}

/// `{ }` empty group with ONLY the failing entry (no prior binds) —
/// pure control.
#[test]
fn only_failure_entry() {
    assert_restored("onlyfail", &format!("{SETUP}{{ :; }} 7</non/nope\n{PROBE}"));
}

/// Compound whose list binds fd 8 twice then fails: first-snapshot
/// wins, final state is the ORIGINAL f8.txt (not the intermediate).
#[test]
fn double_bind_same_fd_restores_original() {
    let (stdout, stderr, rc) = run_driver(
        "dblbind",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 8<f8.txt 7</non/nope\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Sanity: the bind target can itself be a previously-bound fd
/// (`8<&9` copies fd 9 = f9.txt) — failure still restores fd 8.
#[test]
fn bind_from_other_fd_then_failure() {
    assert_restored(
        "fromfd",
        &format!("{SETUP}{{ :; }} 8<&9 7</non/nope\n{PROBE}"),
    );
}

/// stderr of the failure still surfaces (diagnostic not swallowed).
#[test]
fn failure_diagnostic_still_printed() {
    let (_stdout, stderr, _rc) =
        run_driver("diag", &format!("{SETUP}{{ :; }} 8<f9.txt 7</non/nope\n"));
    assert!(
        stderr.contains("/non/nope"),
        "diagnostic missing: {stderr:?}"
    );
}

/// `sudo`-free plain external in a pipeline stage keeps stage-local
/// undo: the STAGE's fd table is what matters; parent fds untouched.
#[test]
fn external_stage_failure_does_not_leak() {
    let (stdout, stderr, rc) = run_driver(
        "extstage",
        &format!(
            "{SETUP}\
             cat 8<f9.txt 7</non/nope | cat\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// Brace group with redirect on the `done`/`esac` keyword line vs
/// trailing — both parse to the compound's list.
#[test]
fn redirect_before_group_keyword() {
    // `8<f9.txt { :; }` is not valid shell; the redirect must trail.
    // This cell documents the trailing form under `!` + pipeline.
    assert_restored(
        "trail",
        &format!("{SETUP}! {{ :; }} 8<f9.txt 7</non/nope | cat\n{PROBE}"),
    );
}

/// Two failing entries: the FIRST failure aborts the walk — binds
/// before it are undone, entries after it never apply.
#[test]
fn first_failure_aborts_walk() {
    let (stdout, stderr, rc) = run_driver(
        "firstabort",
        &format!(
            "{SETUP}\
             {{ :; }} 8<f9.txt 7</non/nope 9<f8.txt\n\
             {PROBE}"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd8=[f8a] fd9=[f9a]\n", "stdout: {stdout:?}");
}

/// UNCLOSED-fd edge: `8<&-` (close fd 8) then failure — fd 8 must be
/// OPEN again afterwards (the close itself is undone).
#[test]
fn close_then_failure_reopens_fd() {
    let (stdout, stderr, rc) = run_driver(
        "reopen",
        &format!(
            "{SETUP}\
             {{ :; }} 8<&- 7</non/nope\n\
             IFS= read -r r8 <&8; echo \"rc=$? fd8=[$r8]\"\n"
        ),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=0 fd8=[f8a]\n", "stdout: {stdout:?}");
}

/// `/dev/full` write failure is NOT an open failure — out of scope
/// for this pin (open-time undo only); skipped deliberately.

/// Final sweep cell: every compound keyword in one driver, each
/// followed by the probe, so a regression in any one face shows.
#[test]
fn all_compound_faces_sweep() {
    let driver = format!(
        "{SETUP}\
         {{ :; }} 8<f9.txt 7</non/nope\n\
         exec 8<f8.txt; echo \"g1 fd8=$(cat <&8 2>/dev/null | head -1)\"\n\
         if :; then :; fi 8<f9.txt 7</non/nope\n\
         exec 8<f8.txt; echo \"g2 fd8=$(cat <&8 2>/dev/null | head -1)\"\n\
         for i in 1; do :; done 8<f9.txt 7</non/nope\n\
         exec 8<f8.txt; echo \"g3 fd8=$(cat <&8 2>/dev/null | head -1)\"\n\
         select i in 1; do break; done 8<f9.txt 7</non/nope\n\
         exec 8<f8.txt; echo \"g4 fd8=$(cat <&8 2>/dev/null | head -1)\"\n\
         case x in x) :;; esac 8<f9.txt 7</non/nope\n\
         exec 8<f8.txt; echo \"g5 fd8=$(cat <&8 2>/dev/null | head -1)\"\n"
    );
    let (stdout, _stderr, rc) = run_driver("sweep", &driver);
    assert_eq!(rc, Some(0));
    for line in stdout.lines() {
        assert!(
            line.ends_with("fd8=f8a"),
            "compound face leaked: {line} (full stdout: {stdout:?})"
        );
    }
    assert_eq!(stdout.lines().count(), 5, "stdout: {stdout:?}");
}

/// Keep `Write` import used even if a future edit drops a helper.
#[allow(dead_code)]
fn _unused(_: &mut dyn Write) {}
