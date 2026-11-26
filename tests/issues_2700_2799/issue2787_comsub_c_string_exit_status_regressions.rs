//! rubash#2787 (owner decision D3, 2026-11-04) — a read-time
//! command-substitution body failure in a `-c` string exits 127.
//!
//! GNU parses a `$(` body at read time, so a body grammar error is
//! a parse failure of the enclosing input and nothing else in that
//! input runs. The exit status is the DRIVER's: the file and stdin
//! readers exit 2 (the parser_error exit, error.c:300-316), but
//! run_one_command (shell.c:1457-1482) maps the `-c` string's
//! FORCE_EOF unwind to `last_command_exit_value = 127`
//! (shell.c:1471-1472). With errexit armed the report exits before
//! the remap and the status stays 2. The substitution's own status
//! stays 2 everywhere; only the abort out of the top-level string
//! is remapped. Plain `-c` syntax errors (no substitution) stay 2,
//! and a backtick body's failure stays contained in its child
//! (issue #515): the enclosing command continues and exits 0.
//!
//! Statuses measured against GNU bash 5.3.20 (2026-11-04). The
//! diagnostic TEXT for the `$(` shapes is pinned by issue2751's
//! file-mode pins and the #2783 family; this file pins the exit
//! status and the run-nothing semantics, which are the decision's
//! subject.
//!
//! NOT pinned here (rubash#2789): the READ-TIME faces of the same
//! GNU rule. GNU validates a comsub body when the enclosing
//! command is READ, so a body that never executes still kills
//! the input (`false && echo $(a |)` -> 127 in `-c`, file dies
//! with 2), and a body whose failure surfaces while the string
//! itself is being parsed (`echo $(case x in)`, which rubash's
//! extractor classifies as unclosed-EOF) takes the remap too.
//! Rubash validates bodies at execution (the #2751 screen), so
//! those two faces are a separate reader-level change, filed as
//! #2789; every shape below reaches the execution screen and is
//! fully covered by the remap.

use std::io::Write;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    run_args(&["-c", script])
}

fn run_args(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

fn run_file(script: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash2787-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

fn run_stdin(script: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

/// The headline face: a pipeline-shaped body error in a `-c`
/// string exits 127 and produces no output.
#[test]
fn issue2787_c_string_comsub_body_error_exits_127() {
    let run = run_c("echo $(a |)");
    assert_eq!(run.code, Some(127));
    assert_eq!(run.stdout, "");
}

/// The whole body-grammar class takes the remap, not just the
/// pipeline shape: if / while / arith-for bodies inside the
/// string all exit 127. (The unterminated-`case` body is the
/// parse-time face — it never reaches the execution screen and
/// is #2789, see the file header.)
#[test]
fn issue2787_c_string_comsub_body_error_class_exits_127() {
    for script in [
        "echo $(if true then echo x; fi)",
        "echo $(while; do :; done)",
        "echo $(for ((i=0;i<3)) do echo; done)",
    ] {
        let run = run_c(script);
        assert_eq!(run.code, Some(127), "script: {script}");
        assert_eq!(run.stdout, "", "script: {script}");
    }
}

/// The abort is total for the executed faces: commands after
/// the failing one never run, and an assignment form aborts the
/// string the same way. (The short-circuited, never-executed
/// form is the read-time face — #2789, see the file header.)
#[test]
fn issue2787_c_string_aborts_the_whole_string() {
    for script in ["echo $(a |); echo after", "x=$(a |); echo after"] {
        let run = run_c(script);
        assert_eq!(run.code, Some(127), "script: {script}");
        assert_eq!(run.stdout, "", "script: {script}");
    }
}

/// Nesting does not dilute the remap: the inner failure surfaces
/// through the outer substitution and the top `-c` driver still
/// exits 127; a subshell around the failure does not contain the
/// string-level abort either.
#[test]
fn issue2787_c_string_nested_and_subshell_faces_exit_127() {
    for script in ["echo $(echo $(a |))", "(echo $(a |)); echo after"] {
        let run = run_c(script);
        assert_eq!(run.code, Some(127), "script: {script}");
        assert_eq!(run.stdout, "", "script: {script}");
    }
}

/// Errexit disarms the remap: with `-e` the report exits before
/// run_one_command's FORCE_EOF mapping, so the status stays 2.
#[test]
fn issue2787_c_string_errexit_keeps_2() {
    let run = run_args(&["-e", "-c", "echo $(a |)"]);
    assert_eq!(run.code, Some(2));
}

/// Plain `-c` syntax errors are NOT remapped — only the
/// substitution read-time failure is: a grammar error in the
/// string itself, an unterminated substitution, and an
/// unterminated brace all stay 2.
#[test]
fn issue2787_c_string_plain_syntax_errors_stay_2() {
    for script in ["if then; fi", "echo $(", "echo ${"] {
        let run = run_c(script);
        assert_eq!(run.code, Some(2), "script: {script}");
    }
}

/// The file route keeps the parser_error exit: 2, and the rest of
/// the file does not run (the failure is read-time there too).
#[test]
fn issue2787_file_route_stays_2() {
    let run = run_file("echo $(a |)\necho after\n");
    assert_eq!(run.code, Some(2));
    assert_eq!(run.stdout, "");
}

/// The stdin route likewise stays 2.
#[test]
fn issue2787_stdin_route_stays_2() {
    let run = run_stdin("echo $(a |)\necho after\n");
    assert_eq!(run.code, Some(2));
    assert_eq!(run.stdout, "");
}

/// A backtick body's failure stays contained in the substituted
/// child (issue #515): the enclosing echo still runs, the script
/// continues, and the exit status is 0.
#[test]
fn issue2787_backtick_body_error_stays_contained() {
    let run = run_c("echo `a |`; echo ok");
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "\nok\n");
}
