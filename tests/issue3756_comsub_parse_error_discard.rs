//! rubash#3756 — a `$(` body grammar failure is READ-TIME and fatal.
//!
//! GNU parses a command-substitution body while the enclosing word is
//! read (`parse.y:4451 parse_comsub` hands the body to the grammar), so
//! a body grammar failure is yyerror -> `jump_to_top_level (DISCARD)`:
//! the whole input unit is abandoned and NOTHING in it executes. After
//! #3756 released truncated-case comsub units (`$(case x in a|)`), the
//! verdict was decided at execution time instead, and two things let
//! the command run anyway: the per-substitution entry reset cleared the
//! body-error latch (a later `$(echo ok)` in the same word list
//! resurrected the command) and the abort gate also demanded the
//! `last_command_substitution_status == 2` slot, which the documented
//! left-to-right `$?` overlay overwrites with the next child's exit.
//! The extractor's case machine additionally never armed its pattern
//! region for a plain `case WORD in` (GNU special_case_tokens rule 6,
//! `parse.y:3369-3386`), so a skipped-branch body was never screened.
//!
//! Expected values are WSL GNU bash 5.3.20 output: `-c` takes
//! shell.c:1471-1472's 127 remap, a script file the parser_error exit 2.
//! stderr is asserted with `contains` (the program-name prefix and the
//! echoed offending line differ per route).

use std::process::Command;

const NEAR: &str = "syntax error near unexpected token `)'";

fn rubash_dash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_script(body: &str, tag: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3756-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create probe dir");
    let path = dir.join("case.sh");
    std::fs::write(&path, body).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reported regression: a sibling substitution in the same word list
/// used to resurrect the command (`ok` printed, rc 0).
#[test]
fn sibling_substitution_does_not_resurrect_the_command() {
    for script in [
        "echo $(case x in a|) $(echo ok)",
        "echo $(case x in a|) $(echo ok) $(echo ok2)",
        "v=$(case x in a|) w=$(echo ok); echo after",
        "echo a$(case x in a|)b",
    ] {
        let (stdout, stderr, code) = rubash_dash_c(script);
        assert_eq!(code, Some(127), "script {script:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "nothing may run: script {script:?}");
        assert!(stderr.contains(NEAR), "stderr={stderr:?}");
    }
}

/// A body the program would skip still kills the unit: GNU's read-time
/// verdict does not depend on the branch being taken.
#[test]
fn skipped_branch_body_still_kills_the_unit() {
    let (stdout, stderr, code) = rubash_dash_c("false && echo $(case x in a|); echo after");
    assert_eq!(code, Some(127), "stderr={stderr:?}");
    assert_eq!(stdout, "", "the `after` command must not run");
    assert!(stderr.contains(NEAR), "stderr={stderr:?}");
}

/// Script-file form: the parser_error exit is 2, still with no output.
#[test]
fn script_file_form_exits_two_without_running() {
    let (stdout, stderr, code) =
        rubash_script("echo $(case x in a|) $(echo ok)\necho after\n", "file");
    assert_eq!(code, Some(2), "stderr={stderr:?}");
    assert_eq!(stdout, "", "no line may execute");
    assert!(stderr.contains(NEAR), "stderr={stderr:?}");
}

/// Controls: a contained body (backtick, parsed in the substituted
/// child) keeps the script running, and complete case comsubs are
/// untouched.
#[test]
fn contained_and_complete_forms_are_unaffected() {
    let (stdout, stderr, code) = rubash_dash_c("x=`case x in a|`; echo \"rc=$?\"");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=2\n", "backtick errors stay contained");

    let (stdout, stderr, code) = rubash_dash_c("echo $(case a in a) echo hi;; esac)");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "hi\n");

    let (stdout, stderr, code) = rubash_dash_c("echo $(case x in esac) $(echo ok)");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "ok\n", "an empty case body is valid");
}
