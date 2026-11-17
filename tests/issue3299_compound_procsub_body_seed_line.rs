//! rubash#3299 regressions: a process-substitution body on a COMPOUND
//! command's stdin redirect reports the wrong diagnostic line — GNU
//! reports the AMBIENT line, rubash reported the compound's start line.
//!
//! Root cause: the procsub fork inherits the parent's `line_number`
//! (subst.c:6412 `make_child`, evalstring.c:346 `line_number--` then
//! the child counts up while parsing the body), and for compound
//! commands that number is whatever `line_number` was when
//! `do_redirections` ran — execute_cmd.c:854 runs BEFORE the dispatch
//! switch stamps anything, so group/while/until/if/for/select/case/
//! arith/cond/time/`!`/coproc all inherit the ambient line, not their
//! own `->line`. Only cm_simple (:936 `SET_LINE_NUMBER
//! (Simple->line)`) and cm_subshell (:696 `SET_LINE_NUMBER
//! (Subshell->line)`) stamp before their redirects expand. The fix
//! seeds compound procsubs from the executor's `ambient_line` cell
//! (the top-level command's parse-end line, frozen at the enclosing
//! compound's established line inside a pre-parsed body, or the
//! function definition's body-open line inside a function body),
//! falling back to the command's own `end_line` then its start line
//! — the same chain `set_current_line` uses for ambient commands.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle). The ambient rule is
//! NOT the command's own end line: a group inside a while/if body
//! reports the WHILE/IF end line (6/5), a group in a pipeline
//! reports the PIPELINE end line (3), and a group in a function body
//! reports the DEFINITION line (1). The d8 leading-redirect simple
//! form (`< <(foo\<NL>bar) echo done` -> line 2) stays covered by
//! the #3210 stem (Simple->line = first-element end line).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3299-{}-{}",
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

fn check(script: &str, expected_stdout: &str, expected_stderr: &str, expected_code: i32) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_code),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, expected_stderr, "script: {script:?}");
}

// -----------------------------------------------------------------
// The issue's two forms: compound stdin procsub -> ambient line 2
// -----------------------------------------------------------------

/// `{...} < <(foo\<NL>bar)`: GNU reports line 2 (the redirect's
/// stop line = the command's parse-end line), rubash reported 1.
#[test]
fn group_stdin_procsub_reports_ambient_line() {
    check(
        "{ read x; echo \"got:$x\"; } < <(foo\\\nbar)\n",
        "got:\n",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// `while ...; done < <(foo\<NL>bar)`: same, line 2.
#[test]
fn while_stdin_procsub_reports_ambient_line() {
    check(
        "while read -r line; do echo \"$line\"; done < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Controls: simple commands and subshells keep their own line
// -----------------------------------------------------------------

/// `exec < <(...)`: Simple->line, line 1 (already green, #3297).
#[test]
fn exec_stdin_procsub_reports_simple_line() {
    check(
        "exec < <(foo\\\nbar)\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        0,
    );
}

/// `echo < <(...)`: Simple->line, line 1.
#[test]
fn echo_stdin_procsub_reports_simple_line() {
    check(
        "echo < <(foo\\\nbar)\n",
        "\n",
        "case.sh: line 1: foobar: command not found\n",
        0,
    );
}

/// `( ... ) < <(...)`: Subshell->line (execute_cmd.c:696), line 1.
#[test]
fn subshell_stdin_procsub_reports_subshell_line() {
    check(
        "( read x ) < <(foo\\\nbar)\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        1,
    );
}

// -----------------------------------------------------------------
// Other compound shapes: all ambient (do_redirections precedes
// the switch, so for/select/case/arith/cond never stamp own-line
// for their own redirects)
// -----------------------------------------------------------------

/// `if ...; fi < <(...)`: ambient line 2.
#[test]
fn if_stdin_procsub_reports_ambient_line() {
    check(
        "if true; then read x; fi < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        1,
    );
}

/// `for ...; done < <(...)`: ambient line 2, NOT For->line (1).
#[test]
fn for_stdin_procsub_reports_ambient_line() {
    check(
        "for i in 1; do read x; done < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        1,
    );
}

/// `(( ... )) < <(...)`: ambient line 2.
#[test]
fn arith_stdin_procsub_reports_ambient_line() {
    check(
        "(( x = 1 )) < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// `[[ ... ]] < <(...)`: ambient line 2.
#[test]
fn cond_stdin_procsub_reports_ambient_line() {
    check(
        "[[ -n x ]] < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// `case ... esac < <(...)`: ambient line 2.
#[test]
fn case_stdin_procsub_reports_ambient_line() {
    check(
        "case $x in a) echo a;; esac < <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Ambient-frame distinguishers: the seed is the ENCLOSING frame's
// line, not the compound's own end line
// -----------------------------------------------------------------

/// Group inside a while body: the group ends on line 3, but GNU
/// reports the while's end line (6).
#[test]
fn group_in_while_body_reports_while_end_line() {
    check(
        "while true; do\n  { read x; } < <(foo\\\nbar)\n  echo hi\n  break\ndone\n",
        "hi\n",
        "case.sh: line 6: foobar: command not found\n",
        0,
    );
}

/// Group inside an if body: the group ends on line 3, but GNU
/// reports the if's end line (5).
#[test]
fn group_in_if_body_reports_if_end_line() {
    check(
        "if true; then\n  { read x; } < <(foo\\\nbar)\n  echo hi\nfi\n",
        "hi\n",
        "case.sh: line 5: foobar: command not found\n",
        0,
    );
}

/// Group as a pipeline stage: the group ends on line 2, but GNU
/// reports the pipeline's end line (3).
#[test]
fn group_in_pipeline_reports_pipeline_end_line() {
    check(
        "{ read x; } < <(foo\\\nbar) |\ncat\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// Group in a function body: the group ends on line 3, but GNU
/// reports the definition line (1).
#[test]
fn group_in_function_body_reports_definition_line() {
    check(
        "f() {\n  { read x; } < <(foo\\\nbar)\n}\nf\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        1,
    );
}

// -----------------------------------------------------------------
// Multi-line owners: the ambient is the owner's parse-end line
// -----------------------------------------------------------------

/// Multi-line group: the group itself ends on line 3.
#[test]
fn multiline_group_reports_group_end_line() {
    check(
        "{ read x\n} < <(foo\\\nbar)\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        1,
    );
}

/// Multi-line while with its own redirect on `done`: line 4.
#[test]
fn multiline_while_own_redirect_reports_while_end_line() {
    check(
        "while read -r l\ndo echo \"$l\"\ndone < <(foo\\\nbar)\n",
        "",
        "case.sh: line 4: foobar: command not found\n",
        0,
    );
}
