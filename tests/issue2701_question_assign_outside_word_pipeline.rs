//! rubash#2701 — `:?` / `:=` semantics are lost outside the main word
//! pipeline (redirect words, arithmetic, case patterns, select lists,
//! subscripts, nested fragments).
//!
//! GNU: one expansion walk owns the error semantics. The `:?` operator on
//! an unset/null parameter reports `<name>: <word>` and returns the
//! expand_wdesc_fatal sentinel in a non-interactive shell (subst.c:10347
//! case '?', subst.c:10414 parameter_brace_expand_error, subst.c:10418
//! fatal return); every consumer expanding through
//! call_expand_word_internal (subst.c:4288-4296) then sets
//! last_command_exit_value = EXECUTION_FAILURE and jumps FORCE_EOF, so
//! the whole script dies with rc 1 — redirect words included
//! (redir.c:298 redirection_expand uses the same pipeline). `:=`
//! performs the assignment at the evaluation point, whatever the
//! context.
//!
//! rubash at base: the core operator walk is value-only
//! (expand_braced_ops.rs `:?` arm returns the message word as the
//! VALUE), and the correct behaviour is layered on as compensating
//! pre-passes (parameter_expansion_error /
//! apply_parameter_assignment_expansions_in_word) that only the
//! simple-command pipeline runs. Contexts expanding words without both
//! pre-passes silently degrade `:?` to `:-` and drop `:=`.
//!
//! Expectations measured byte-for-byte against GNU bash 5.3.20 in
//! script-file mode (2026-11-09). The two `word_*` cells are the
//! main-pipeline controls the pre-passes already cover.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    /// Files left in the case dir besides case.sh: (name, contents).
    files: Vec<(String, String)>,
}

fn run_file(tag: &str, script: &str, stdin: &[u8]) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2701-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), format!("{script}\n")).expect("write fixture");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    if !stdin.is_empty() {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin)
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait rubash");
    let mut files: Vec<(String, String)> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "case.sh" {
            let contents = std::fs::read(entry.path()).expect("read side-effect file");
            files.push((name, String::from_utf8_lossy(&contents).into_owned()));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
        files,
    }
}

fn check(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32, files: &[(&str, &str)]) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
    let expected: Vec<(String, String)> = files
        .iter()
        .map(|(n, c)| (n.to_string(), c.to_string()))
        .collect();
    assert_eq!(run.files, expected, "{tag}: side-effect files");
}

const ERR_BOOM: &str = "case.sh: line 1: unsetvar: boom\n";

#[test]
fn issue2701_redirect_word_question_is_fatal() {
    let run = run_file(
        "redir-q",
        "echo hi > ${unsetvar:?boom}\necho continued rc=$?",
        b"",
    );
    // GNU creates no file and never reaches the continuation line.
    check("redir-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_redirect_word_assign_sticks() {
    let run = run_file(
        "redir-a",
        "echo hi > ${unsetvar:=out}\necho \"var=<$unsetvar>\"",
        b"",
    );
    check("redir-a", &run, "var=<out>\n", "", 0, &[("out", "hi\n")]);
}

#[test]
fn issue2701_arithmetic_question_is_fatal() {
    let run = run_file(
        "arith-q",
        "echo $(( ${unsetvar:?boom} + 1 ))\necho continued rc=$?",
        b"",
    );
    check("arith-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_arithmetic_assign_sticks() {
    let run = run_file(
        "arith-a",
        "echo $(( ${unsetvar:=5} + 1 ))\necho \"var=<$unsetvar>\"",
        b"",
    );
    check("arith-a", &run, "6\nvar=<5>\n", "", 0, &[]);
}

#[test]
fn issue2701_case_pattern_question_is_fatal() {
    let run = run_file(
        "case-q",
        "case x in ${unsetvar:?boom}) echo hit;; *) echo fall;; esac\necho continued rc=$?",
        b"",
    );
    check("case-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_subscript_question_is_fatal() {
    let run = run_file(
        "sub-q",
        "arr=(a b); echo ${arr[${unsetvar:?boom}]}\necho continued rc=$?",
        b"",
    );
    check("sub-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_patsub_replacement_question_is_fatal() {
    let run = run_file(
        "patsub-q",
        "v=abc; echo ${v/b/${unsetvar:?boom}}\necho continued rc=$?",
        b"",
    );
    check("patsub-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_nested_fragment_question_is_fatal() {
    let run = run_file(
        "nested-q",
        "echo \"${outer:-${unsetvar:?boom}}\"\necho continued rc=$?",
        b"",
    );
    check("nested-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_select_list_question_is_fatal() {
    let run = run_file(
        "select-q",
        "select s in ${unsetvar:?boom}; do echo \"got $s\"; break; done\necho continued rc=$?",
        b"1\n",
    );
    // GNU errors while expanding the select list: no menu is printed.
    check("select-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_chain_state_dies_with_the_script() {
    // One-line chain: state set before the failing redirect must not be
    // observable afterwards — the fatal error ends the whole script.
    let run = run_file("chain", "x=1; echo hi > ${y:?why}; echo \"x=$x\"", b"");
    check("chain", &run, "", "case.sh: line 1: y: why\n", 1, &[]);
}

#[test]
fn issue2701_main_word_question_control() {
    // Control: the main command-word pipeline is the one face the
    // compensating pre-passes cover; this must hold before and after.
    let run = run_file(
        "word-q",
        "echo ${unsetvar:?boom}\necho continued rc=$?",
        b"",
    );
    check("word-q", &run, "", ERR_BOOM, 1, &[]);
}

#[test]
fn issue2701_main_word_assign_control() {
    let run = run_file(
        "word-a",
        "echo ${unsetvar:=zz}\necho \"var=<$unsetvar>\"",
        b"",
    );
    check("word-a", &run, "zz\nvar=<zz>\n", "", 0, &[]);
}
