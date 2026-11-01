//! rubash#2474 — `declare b[1 + 1]=(x y)` is a PARSE-TIME syntax
//! error in GNU (the script aborts, exit status 2), not the runtime
//! `cannot assign list to array member` diagnostic rubash used to
//! print before carrying on.
//!
//! GNU ground: read_token_word consumes a matched `[...]` span into
//! the word only where an assignment word is acceptable —
//! `token_index > 0 && assignment_acceptable(last_read_token) &&
//! token_is_ident(...)`, or inside a compound list (parse.y:5636) —
//! and assignment_acceptable is command-token position
//! (parse.y:3163). The `=(` compound VALUE has its own, wider gate
//! (parse.y:5653: assignment_acceptable OR PST_ASSIGNOK, and
//! PST_ASSIGNOK is what a declaration builtin's operand position
//! carries), but that gate never opens the SUBSCRIPT span: in a
//! declare/local/export/readonly operand a spaced subscript still
//! splits the word at the bracket whitespace, the following `(` is
//! read as a stray token, and the parser reports
//! `syntax error near unexpected token `('` for the whole line.
//! The UNSPACED form (`b[1+1]=(x y)`) stays one word in the same
//! position and fails at execution time instead.
//!
//! rubash's lexer glued the span at any position for the
//! element-compound form, so declare received one operand and the
//! runtime diagnostic fired. The span gate now follows parse.y:5636
//! exactly (assignment position only); the unspaced form keeps
//! reaching the runtime diagnostic through the
//! compound_assignment_start arm.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle
//! with script-file probes; the scratch path is normalized to WORK
//! and the script path to SCRIPT, mirroring tests/issue2463_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2474-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE; return (status, stdout, stderr)
/// with the scratch path normalized to WORK and the script path to
/// SCRIPT.
fn run_script(dir: &Path, script: &str) -> (i32, String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
            .replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        output.status.code().unwrap_or(-1),
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn declare_spaced_subscript_compound_is_a_syntax_error() {
    let dir = scratch("exact");
    let (status, stdout, stderr) = run_script(
        &dir,
        "declare b[1 + 1]=(x y)\necho after rc=$?\n",
    );
    assert_eq!(status, 2);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: syntax error near unexpected token `('\n\
         SCRIPT: line 1: `declare b[1 + 1]=(x y)'\n"
    );
}

#[test]
fn syntax_error_aborts_at_the_offending_command() {
    let dir = scratch("goodline");
    let (status, stdout, stderr) = run_script(
        &dir,
        "echo first\ndeclare b[1 + 1]=(x y)\necho after\n",
    );
    // GNU reads and executes complete commands in order: the good
    // line runs, the bad line is a parse error (status 2), and
    // nothing after it runs.
    assert_eq!(status, 2);
    assert_eq!(stdout, "first\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 2: syntax error near unexpected token `('\n\
         SCRIPT: line 2: `declare b[1 + 1]=(x y)'\n"
    );
}

#[test]
fn export_and_readonly_operands_are_syntax_errors_too() {
    let dir = scratch("siblings");
    for builtin in ["export", "readonly"] {
        let (status, stdout, stderr) = run_script(
            &dir,
            &format!("{builtin} b[1 + 1]=(x y)\necho after rc=$?\n"),
        );
        assert_eq!(status, 2, "{builtin}");
        assert_eq!(stdout, "", "{builtin}");
        assert_eq!(
            stderr,
            format!(
                "SCRIPT: line 1: syntax error near unexpected token `('\n\
                 SCRIPT: line 1: `{builtin} b[1 + 1]=(x y)'\n"
            ),
            "{builtin}"
        );
    }
}

#[test]
fn local_operand_inside_a_function_is_a_syntax_error() {
    let dir = scratch("local");
    let (status, stdout, stderr) = run_script(
        &dir,
        "f() { local b[1 + 1]=(x y); echo in-func rc=$?; }\nf\necho after rc=$?\n",
    );
    assert_eq!(status, 2);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: syntax error near unexpected token `('\n\
         SCRIPT: line 1: `f() { local b[1 + 1]=(x y); echo in-func rc=$?; }'\n"
    );
}

#[test]
fn unspaced_and_standalone_forms_still_fail_at_execution() {
    let dir = scratch("guards");
    // Unspaced subscript in the same operand position: one word,
    // runtime diagnostic, the script carries on.
    let (status, stdout, stderr) = run_script(
        &dir,
        "declare b[1+1]=(x y)\necho after rc=$?\n",
    );
    assert_eq!(status, 0);
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: b[1+1]: cannot assign list to array member\n"
    );
    // The standalone spaced form sits in assignment position, where
    // the span IS consumed: same runtime diagnostic (issue2463's
    // compound_shapes_are_unchanged pins this too).
    let (status, stdout, stderr) = run_script(&dir, "a[1 + 1]=(x y)\necho after rc=$?\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: a[1 + 1]: cannot assign list to array member\n"
    );
}
