//! rubash#3301 regressions: a coproc body whose first leaf command
//! is split by a backslash-newline continuation reported the first
//! leaf's START line, where GNU reports the line the first element
//! finished on (GNU parse.y:5849-5851 `simplecmd_lineno` /
//! parse.y:810-811, execute_cmd.c:936 `SET_LINE_NUMBER`).
//!
//! The coproc child is a respawn: it re-parses the rendered body
//! text (`bash_command_sequence_text`), whose continuations have
//! already collapsed onto one line, so the child's diagnostic line
//! is exactly the seed the parent ships in `__RUBASH_CURRENT_LINE`
//! (`first_leaf_command_line`, compound_exec.rs). That seed took the
//! leaf's `line` — the first token's START line — so
//! `coproc { foooo\<NL>bar; }` reported line 1 where GNU's forked
//! child, inheriting `line_number` past the folded word, reports
//! line 2.
//!
//! The seed is the first leaf's GNU stamp
//! (`simple_command_first_word_end_line`), not its start line — the
//! same stamp the #3210 procsub seed and the #3054 span transport
//! already carry. The #3209 function-definition plant is respected
//! through the stamp: `restamp_function_body_first_command` restamps
//! a redirect-first first body command's `line` AND
//! `first_word_end_line` to the definition line, so
//! `f() { coproc >x\<NL>y foooo; }` still reports line 1 (the
//! guard cell below pins that interaction).
//!
//! Expectations measured against GNU bash 5.3.20 script-file probes
//! (self-compiled oracle, `~/workspace/rubash-audit/build-gnu/
//! bash-5.3/bash`): headline line 2, redirect-first root line 2,
//! named coproc line 2, `$LINENO` face `x=2`, plant guard line 1.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3301_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// The coproc child fails (127) but the parent's exit status is the
/// last command's (`sleep`), so the script exits 0; the diagnostic
/// arrives on the shared stderr.
fn assert_diagnostic(run: &Run, expected_stderr: &str) {
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "", "stdout: {}", run.stdout);
    assert_eq!(run.stderr, expected_stderr, "stderr: {}", run.stderr);
    let _ = &run.dir;
}

#[test]
fn coproc_folded_first_word_reports_end_line() {
    let run = run_script("coproc { foooo\\\nbar; }\nsleep 0.5\n");
    assert_diagnostic(&run, "case.sh: line 2: foooobar: command not found\n");
}

#[test]
fn named_coproc_folded_first_word_reports_end_line() {
    let run = run_script("coproc C { foooo\\\nbar; }\nsleep 0.5\n");
    assert_diagnostic(&run, "case.sh: line 2: foooobar: command not found\n");
}

#[test]
fn coproc_redirect_first_folded_target_reports_end_line() {
    // No function-definition plant at root: the redirect captures
    // the target's end line (parse.y:810-811).
    let run = run_script("coproc { >x\\\ny foooo; }\nsleep 0.5\n");
    assert_diagnostic(&run, "case.sh: line 2: foooo: command not found\n");
}

#[test]
fn coproc_folded_first_word_lineno_is_end_line() {
    let run = run_script("coproc { ec\\\nho \"x=$LINENO\" >&2; }\nsleep 0.5\n");
    assert_diagnostic(&run, "x=2\n");
}

#[test]
fn coproc_unfolded_first_word_still_reports_start_line() {
    let run = run_script("coproc { foooo; }\nsleep 0.5\n");
    assert_diagnostic(&run, "case.sh: line 1: foooo: command not found\n");
}

/// #3209 plant guard: inside a function body the redirect-first
/// first command's stamp IS the definition line — the seed change
/// must not move this cell to line 2.
#[test]
fn coproc_redirect_first_inside_function_still_reports_definition_line() {
    let run = run_script("f() { coproc >x\\\ny foooo; }\nf\nsleep 0.5\n");
    assert_diagnostic(&run, "case.sh: line 1: foooo: command not found\n");
}
