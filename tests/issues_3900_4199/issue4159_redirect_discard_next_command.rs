//! Issue unixwin/rubash#4159 regression: a DISCARD-class expansion
//! failure during redirect processing (`> ${a[1x]}` — the redirect word's
//! subscript evaluation evalerrors, GNU expr.c evalerror ->
//! jump_to_top_level (DISCARD)) abandoned the failing command's own list
//! AND silently swallowed the NEXT command in the script: it never ran
//! (no output, no side effects) while the script continued after it.
//!
//! GNU's DISCARD abandons only the list that contained the failing
//! command (eval.c:111 reader_loop, case DISCARD); the reader resumes at
//! the next list with last_command_exit_value = 1 (shell.c:1477). The
//! leak mechanism: the redirect-gate scan latches the arithmetic
//! expansion-error flags after the owning command's
//! abort_on_expansion_errors funnel has passed, so the NEXT command's
//! funnel consumed them as its own failure. The discard settlement in
//! the list loop (and the fresh-unit reset) now consumes those latches
//! with the rest of the evalerror family.
//!
//! All expectations below were taken from the real GNU bash 5.3.20
//! oracle with the same script files (stdout / status / produced files
//! byte-compared). stderr is asserted loosely — the primary arithmetic
//! diagnostic only — because the diagnostic-channel face (a diagnostic
//! the command's own `2>/dev/null` should have eaten, and the extra
//! `ambiguous redirect` line) is the separate #4114/#4122 surface.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    dir: PathBuf,
}

fn run_script(name: &str, script: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("issue4159-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create probe dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
        dir,
    }
}

fn read_file(run: &Run, name: &str) -> Option<String> {
    std::fs::read_to_string(run.dir.join(name)).ok()
}

fn diag_count(run: &Run) -> usize {
    run.stderr.matches("value too great for base").count()
}

#[test]
fn function_redirect_discard_lets_next_line_run() {
    // The issue's minimal reproduction: GNU prints first and second.
    let run = run_script(
        "repro",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\necho first\necho second\n",
    );
    assert_eq!(run.stdout, "first\nsecond\n");
    assert_eq!(run.code, Some(0));
    assert_eq!(diag_count(&run), 0, "stderr: {}", run.stderr);
}

#[test]
fn toplevel_redirect_discard_lets_next_line_run() {
    // Same failure at top level: the failing command reports 1 (the
    // DISCARD catch value, shell.c:1477) and the next line runs.
    let run = run_script(
        "toplevel",
        "echo hi > ${a[1x]}\necho \"after=$?\"\necho end\n",
    );
    assert_eq!(run.stdout, "after=1\nend\n");
    assert_eq!(run.code, Some(0));
    assert_eq!(diag_count(&run), 1, "stderr: {}", run.stderr);
}

#[test]
fn redirect_discard_status_after_function_call_is_one() {
    let run = run_script(
        "status",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\necho \"after=$?\"\necho end\n",
    );
    assert_eq!(run.stdout, "after=1\nend\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_next_command_file_side_effect_lands() {
    // v1/v6: the swallowed command carried an explicit redirect — the
    // file was never created. GNU creates it.
    let run = run_script(
        "sideeffect",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\necho data > created.txt\ncat created.txt\n",
    );
    assert_eq!(run.stdout, "data\n");
    assert_eq!(read_file(&run, "created.txt").as_deref(), Some("data\n"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_next_command_assignment_lands() {
    // c5: an assignment in the victim slot was swallowed (y stayed
    // empty). GNU binds it.
    let run = run_script(
        "assign",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\ny=assigned\necho \"y=$y\"\n",
    );
    assert_eq!(run.stdout, "y=assigned\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_next_command_external_runs() {
    // v5: the victim was an external command, not a builtin.
    let run = run_script(
        "external",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\n\"${THIS_SH}\" -c 'echo ext'\necho end\n",
    );
    assert_eq!(run.stdout, "ext\nend\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_second_function_call_runs() {
    // v10: a whole second call of the same function was swallowed (its
    // side effect and its own failure never happened). GNU runs it.
    let run = run_script(
        "doublecall",
        "f() { echo x >> log.txt; echo 2>/dev/null > ${a[1x]}; }\nf\nf\necho end\n",
    );
    assert_eq!(run.stdout, "end\n");
    assert_eq!(read_file(&run, "log.txt").as_deref(), Some("x\nx\n"));
    assert_eq!(diag_count(&run), 0, "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_last_command_status_is_one() {
    // The pM tail: the failing call is the script's last command, so
    // the DISCARD status becomes the script's status.
    let run = run_script("lastfunc", "f() { echo 2>/dev/null > ${a[1x]}; }\nf\n");
    assert_eq!(run.stdout, "");
    assert_eq!(run.code, Some(1));
    let run = run_script("lasttop", "echo hi > ${a[1x]}\n");
    assert_eq!(run.stdout, "");
    assert_eq!(run.code, Some(1));
}

#[test]
fn redirect_discard_same_line_tail_still_discarded() {
    // The boundary itself must not over-run: commands sharing the
    // failing command's list (same line, `;`-joined) are still
    // discarded; only the NEXT line runs.
    let run = run_script(
        "sameline",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf; echo sameline\necho nextline\n",
    );
    assert_eq!(run.stdout, "nextline\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_second_failure_reports_too() {
    // Two failing lines in a row: the second command must be dispatched
    // (and fail on its own), not be eaten by the first failure.
    let run = run_script(
        "secondfail",
        "echo hi > ${a[1x]}\necho mid > ${a[1x]}\necho l3\necho l4\n",
    );
    assert_eq!(run.stdout, "l3\nl4\n");
    assert_eq!(diag_count(&run), 2, "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn redirect_discard_compound_victim_runs() {
    // The victim slot held compound commands (if / brace group): both
    // run under GNU.
    let run = run_script(
        "ifvictim",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\nif true; then echo inif; fi\necho end\n",
    );
    assert_eq!(run.stdout, "inif\nend\n");
    let run = run_script(
        "bracevictim",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf\n{ echo inbrace; }\necho end\n",
    );
    assert_eq!(run.stdout, "inbrace\nend\n");
}

#[test]
fn redirect_discard_andor_list_status() {
    // The failing call heads an && list: the list is abandoned with
    // status 1 and the next line observes it.
    let run = run_script(
        "andor",
        "f() { echo 2>/dev/null > ${a[1x]}; }\nf && echo yes\necho \"next=$?\"\n",
    );
    assert_eq!(run.stdout, "next=1\n");
    assert_eq!(run.code, Some(0));
}

// --- Delimitation controls: these agreed with GNU before the fix and
// --- must keep agreeing (the fix must not widen the discard).

#[test]
fn pure_expansion_failure_control() {
    // v7: a pure expansion failure with no redirect is not this face.
    let run = run_script("pure", "x=${a[1x]}\necho \"after=$?\"\necho end\n");
    assert_eq!(run.stdout, "after=1\nend\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn error_return_redirect_failure_control() {
    // v8: an error-return class redirect failure (open failure) never
    // armed the discard.
    let run = run_script(
        "errret",
        "echo hi > /no/such/dir/f\necho \"after=$?\"\necho end\n",
    );
    assert_eq!(run.stdout, "after=1\nend\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn subshell_isolation_control() {
    // v9: the failure inside a subshell reaches only the subshell's own
    // top level.
    let run = run_script("subshell", "( echo hi > ${a[1x]} )\necho after\n");
    assert_eq!(run.stdout, "after\n");
    assert_eq!(run.code, Some(0));
    let run = run_script(
        "subshellfunc",
        "f() { echo 2>/dev/null > ${a[1x]}; }\n( f )\necho after\n",
    );
    assert_eq!(run.stdout, "after\n");
    assert_eq!(run.code, Some(0));
}
