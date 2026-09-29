//! Issue rubash#306 regressions (wt13/fixpack lane).
//!
//! `eval "echo >"` — a syntax error inside an eval'd string aborted the
//! WHOLE script instead of ending only the eval: GNU contains the error,
//! eval returns rc=2, and the caller's remaining commands run.
//!
//! GNU anchors:
//! - builtins/evalstring.c:579-599 (parse_and_execute parse-failure arm):
//!   a syntax error in the string being evaluated only `break's the
//!   parse_and_execute loop and returns EX_BADUSAGE; jump_to_top_level
//!   fires solely when `posixly_correct' (evalstring.c:583-589). The eval'd
//!   parse-error diagnosis set rubash's reader-abort flag
//!   (command_execute mark_parse_error) on its way up; the fix takes that
//!   flag inside execute_eval_source so the containment cannot leave it
//!   set for the script driver (which would abort the whole script).
//! - error.c:324-327 (parser_error): under live errexit the FIRST line of a
//!   syntax-error report is followed by an immediate exit_shell(2) — the
//!   offending-line echo (parse.y:6814 print_offending_line) never runs,
//!   no eval containment applies, and the forced status is 2 (plain
//!   `bad=(' exits 1, `set -e; bad=(' exits 2).
//! - execute_cmd.c:4998-5016: eval/source running under CMD_IGNORE_RETURN
//!   (`eval ... || x', if/while conditions) clears
//!   exit_immediately_on_error for the builtin's duration — both report
//!   lines print and eval's rc=2 is catchable there.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; the 16-case matrix
//! lives under target/issue-suites/results/fixpack306/).

use std::process::Command;

/// Run a script FILE in its own scratch directory (relative name `case.sh`
/// keeps the diagnostic prefix stable) and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i306-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Core repro: `eval "echo >"` reports the two-line syntax error, eval
/// yields rc=2, and the script CONTINUES (final rc=0).
#[test]
fn eval_syntax_error_returns_two_and_script_continues() {
    let (stdout, stderr, code) =
        rubash_file("echo start\neval \"echo >\"\necho \"rc=$?\"\necho end\n");
    assert_eq!(
        stdout, "start\nrc=2\nend\n",
        "script must continue after eval's contained parse error"
    );
    assert_eq!(
        stderr,
        "case.sh: eval: line 2: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 2: `echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// One parse unit dies wholesale: `echo mid; echo >' is a single command
/// list, so NOTHING in the eval string runs before the rc=2 (GNU
/// parse_and_execute parses the complete list before executing).
#[test]
fn eval_syntax_error_discards_whole_list_unit() {
    let (stdout, stderr, code) =
        rubash_file("echo one\neval 'echo mid; echo >'\necho \"rc=$?\"\necho two\n");
    assert_eq!(stdout, "one\nrc=2\ntwo\n", "mid must NOT print");
    assert_eq!(
        stderr,
        "case.sh: eval: line 2: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 2: `echo mid; echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// Inside a function body the containment is the same: `echo in-f' still
/// runs after the failing eval and the function returns eval's 2.
#[test]
fn eval_syntax_error_inside_function_continues_body() {
    let (stdout, stderr, code) =
        rubash_file("f() { eval 'echo >'; echo \"inner-rc=$?\"; }\nf\necho \"outer-rc=$?\"\n");
    assert_eq!(stdout, "inner-rc=2\nouter-rc=0\n");
    assert_eq!(
        stderr,
        "case.sh: eval: line 1: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 1: `echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// Nested eval: each layer contains the inner error; caller sees rc=2.
#[test]
fn nested_eval_syntax_error_is_contained_at_every_layer() {
    let (stdout, stderr, code) =
        rubash_file("echo start\neval \"eval 'echo >'\"\necho \"nested-rc=$?\"\necho end\n");
    assert_eq!(stdout, "start\nnested-rc=2\nend\n");
    assert_eq!(
        stderr,
        "case.sh: eval: line 2: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 2: `echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// set -e + bare eval: error.c:324-327 exits the shell right after the
/// FIRST diagnostic line — no offending-line echo, status 2, script dead.
#[test]
fn eval_syntax_error_under_errexit_prints_one_line_and_exits_two() {
    let (stdout, stderr, code) =
        rubash_file("set -e\necho start\neval \"echo >\"\necho unreachable\n");
    assert_eq!(stdout, "start\n");
    assert_eq!(
        stderr,
        "case.sh: eval: line 3: syntax error near unexpected token `newline'\n"
    );
    assert_eq!(code, Some(2));
}

/// set -e + `|| ' context (CMD_IGNORE_RETURN, execute_cmd.c:4998-5016):
/// errexit is suspended for the eval builtin, so both lines print, the
/// right side catches rc=2, and the script continues.
#[test]
fn eval_syntax_error_under_or_guard_prints_both_lines_and_continues() {
    let (stdout, stderr, code) =
        rubash_file("set -e\necho start\neval \"echo >\" || echo caught\necho end\n");
    assert_eq!(stdout, "start\ncaught\nend\n");
    assert_eq!(
        stderr,
        "case.sh: eval: line 3: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 3: `echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// set -e + if-condition (also IGNORE_RETURN): two lines, else branch runs.
#[test]
fn eval_syntax_error_in_if_condition_takes_else_branch() {
    let (stdout, stderr, code) = rubash_file(
        "set -e\necho start\nif eval 'echo >'; then echo then; else echo else; fi\necho end-if\n",
    );
    assert_eq!(stdout, "start\nelse\nend-if\n");
    assert_eq!(
        stderr,
        "case.sh: eval: line 3: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 3: `echo >'\n"
    );
    assert_eq!(code, Some(0));
}

/// error.c:326 forces exit status 2 at a parser_error under live errexit:
/// `set -e; bad=(' exits 2 while plain `bad=(' keeps its status 1
/// (parse_compound_assignment EOF family).
#[test]
fn unclosed_array_assignment_under_errexit_exits_two() {
    let (stdout, stderr, code) = rubash_file("set -e\necho start\nbad=(\necho end\n");
    assert_eq!(stdout, "start\n");
    assert_eq!(
        stderr,
        "case.sh: line 3: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(2));
}

/// The non-errexit twin keeps GNU's status 1 for the unclosed `name=(`
/// array list (regression guard for the errexit override above).
#[test]
fn unclosed_array_assignment_without_errexit_exits_one() {
    let (stdout, stderr, code) = rubash_file("echo start\nbad=(\necho end\n");
    assert_eq!(stdout, "start\n");
    assert_eq!(
        stderr,
        "case.sh: line 2: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(1));
}

/// set -e + eval of the unclosed-construct (EOF) flavor: parser_error's
/// exit_shell(2) is terminal — even a following function-body command must
/// not run. (The diagnostic's line number still tracks the function
/// definition line instead of GNU's invocation line — pre-existing
/// divergence on master, asserted loosely here; see the lane report.)
#[test]
fn eval_unterminated_construct_under_errexit_exits_immediately() {
    let (stdout, stderr, code) =
        rubash_file("set -e\nf() { eval 'x() { _;}>_[${'; echo in-f; }\nf\necho unreachable\n");
    assert_eq!(stdout, "", "in-f must not run");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `}'"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(2));
}
