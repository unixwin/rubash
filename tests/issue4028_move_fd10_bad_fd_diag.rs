use std::path::PathBuf;
use std::process::Command;

// rubash#4028: moving from an unbound fd (`8>&10-` while fd 10 is not
// open) must surface GNU's undo-period diagnostic pair. During the
// undoable walk the source close first saves the redirector with
// add_undo_redirect (redir.c:1171-1179 runs only when the redirector is
// open), and undoing that save F_DUPFDs it back (redir.c:1296+); with
// fd 10 unbound the dup fails and GNU prints
// `redirection error: cannot duplicate fd` before the per-redirection
// `10: Bad file descriptor`. The first line therefore appears exactly
// when the move is real, the walk is undoable, and the redirector fd
// is open at the move's position; it must not appear on active
// (non-undoable) walks, and a move whose source is bound must succeed
// silently.
//
// Pins use script files (not -c) so the `probe.sh:` prefixes and line
// numbers are stable and byte-comparable against the GNU oracle.

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash_issue4028_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create probe dir");
    dir
}

fn run_script(dir: &PathBuf, body: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("probe.sh"), body).expect("write probe.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .output()
        .expect("run rubash probe.sh");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
    (stdout, stderr, output.status.code())
}

fn first_line() -> &'static str {
    "probe.sh: redirection error: cannot duplicate fd: Bad file descriptor\n"
}

fn second_line(line_no: usize) -> String {
    format!("probe.sh: line {line_no}: 10: Bad file descriptor\n")
}

#[test]
fn move_from_unbound_fd10_exact_diagnostics_across_command_forms() {
    // Simple builtin form: exactly the per-redirection line, no first
    // line (fd 10 is unbound, so the undo save never ran).
    let dir = test_dir("form_simple");
    let (out, err, code) = run_script(&dir, "true 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(1));
    assert_eq!(code, Some(0));

    // Brace-group form.
    let dir = test_dir("form_group");
    let (out, err, code) = run_script(&dir, "{ true; } 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(1));
    assert_eq!(code, Some(0));

    // Function form: the failing redirect sits on line 2.
    let dir = test_dir("form_function");
    let (out, err, code) = run_script(&dir, "f() { true; }\nf 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(2));
    assert_eq!(code, Some(0));

    // Subshell-with-exec form.
    let dir = test_dir("form_exec");
    let (out, err, code) = run_script(&dir, "( exec 8>&10- )\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(1));
    assert_eq!(code, Some(0));
}

#[test]
fn move_first_line_present_when_redirector_open_on_undoable_walks() {
    // The redirector (fd 8) is bound earlier in the same walk, so the
    // source close runs its undo save and the undo dup fails: both
    // diagnostic lines, in GNU's order.
    let expected = |line_no: usize| format!("{}{}", first_line(), second_line(line_no));

    let dir = test_dir("present_simple");
    let (out, err, _code) = run_script(&dir, "true 8>/dev/null 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, expected(1));

    let dir = test_dir("present_group");
    let (out, err, _code) = run_script(&dir, "{ true; } 8>/dev/null 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, expected(1));

    let dir = test_dir("present_function");
    let (out, err, _code) =
        run_script(&dir, "f() { true; }\nf 8>/dev/null 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, expected(2));

    let dir = test_dir("present_exec");
    let (out, err, _code) = run_script(&dir, "( exec 8>/dev/null 8>&10- )\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, expected(1));
}

#[test]
fn move_first_line_absent_on_active_walks() {
    // Same open-redirector layout, but the walk is active (pipeline
    // element / subshell head): the save still runs, yet undo applies
    // it silently -- GNU emits only the per-redirection line.
    let dir = test_dir("absent_pipeline");
    let (out, err, _code) = run_script(
        &dir,
        "true 8>/dev/null 8>&10- | cat\necho rc=${PIPESTATUS[0]}\n",
    );
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(1));

    let dir = test_dir("absent_subshell_head");
    let (out, err, _code) = run_script(&dir, "( true ) 8>/dev/null 8>&10-\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, second_line(1));
}

#[test]
fn bound_move_succeeds_silently() {
    // Negative guard: a move whose source fd is bound succeeds at its
    // own position -- rc=0 and no diagnostics of either kind from the
    // move itself. The redirect sits on a plain command, so the
    // undoable walk then restores the prior fd state (fd 8 closed
    // again, fd 9 restored), and the trailing `cat <&8` fails with the
    // ordinary per-redirection line, exactly as GNU does (oracle
    // matrix g1 and a pin-shaped re-measurement: stdout `rc=0`,
    // stderr names fd 8 at the cat line, script rc 1). An earlier
    // draft of this pin expected the moved binding to persist past
    // the command (the cat printing `hello`); the oracle measurement
    // refutes that -- a plain command's move is temporary.
    let dir = test_dir("bound_move");
    std::fs::write(dir.join("in.txt"), "hello\n").expect("write in.txt");
    let (out, err, code) = run_script(
        &dir,
        "exec 9<in.txt\ntrue 8>&9-\necho rc=$?\ncat <&8\n",
    );
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "probe.sh: line 4: 8: Bad file descriptor\n");
    assert_eq!(code, Some(1));
}

#[test]
fn expanded_forms_keep_word_classification() {
    // `8>&$x` with x='10-': the dash arrives by expansion, so this is
    // the word-form redirect (ambiguous redirect), not a move at all.
    let dir = test_dir("word_form_dash");
    let (out, err, _code) = run_script(&dir, "x='10-'\ntrue 8>&$x\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(
        err,
        "probe.sh: line 2: 10-: ambiguous redirect\n"
    );

    // `8>&$x` with x='10': a plain dup of an unbound fd -- the
    // per-redirection line only, never the move first line. GNU
    // names the redirector fd (8) for this word form, not the dup
    // source (oracle matrix w2, byte-measured on all three trees).
    let dir = test_dir("word_form_plain");
    let (out, err, _code) = run_script(&dir, "x='10'\ntrue 8>&$x\necho rc=$?\n");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, "probe.sh: line 2: 8: Bad file descriptor\n");
}
