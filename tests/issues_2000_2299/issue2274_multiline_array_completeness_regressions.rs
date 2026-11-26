//! rubash#2274 (= #2241 item (a)): the completeness scanner misjudged a
//! multi-line array assignment as closed whenever a command-position
//! opener shared its first physical line (`f() { A=(\n...`), so the
//! reader split the group per line and the run died with
//! `unexpected EOF while looking for matching ')'` (or, in a subshell,
//! a stray-`)` syntax error). That is the shape bats' respawn prelude
//! produces: function bodies are re-printed on one line while array
//! assignments keep their inner newlines.
//!
//! Root cause: the compound-assignment residual scan in
//! src/lexer/continuation.rs tracks the assignment-prefix region with
//! `seen_command_word`, but its position model never re-entered command
//! position after the constructs that open one. GNU's model is
//! parse.y:3157-3164 (`command_token_position` /
//! `assignment_acceptable`): an assignment word is acceptable whenever
//! the previous token leaves the reader at command position — after a
//! separator, after `(`, and after the reserved words that introduce a
//! list (`{`, `then`, `do`, `else`, condition introducers, `!`,
//! `time`). The scan now resets at exactly those points: the
//! classifier treats list-opener words as position-openers, the `(`
//! arm re-opens command position for the subshell body, and the
//! `;`/`|`/`&` arm resets even when whitespace already flushed the
//! pending word (`true && A=(...`).
//!
//! All expectations verified against GNU bash 5.3.20 via script files.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2274-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    let file = dir.join("case.sh");
    fs::write(&file, script).expect("write script");
    let output = Command::new(RUBASH)
        .arg(&file)
        .output()
        .expect("run rubash on script file");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_dash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
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

fn run_stdin(script: &str) -> (String, String, Option<i32>) {
    use std::io::Write;
    let mut child = Command::new(RUBASH)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash on stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("feed stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_array_ok(tag: &str, script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(code, Some(0), "{tag}: rc, stderr={stderr}");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(stderr, "", "{tag}: stderr must be empty");
}

/// The headline shape (#2241 probe p36a): the array assignment opens on
/// the same physical line as the function body's `{`.
#[test]
fn issue2274_array_first_line_of_function_body() {
    assert_array_ok(
        "func-sameline",
        "f() { A=(\n1\n2\n);}; f; echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
}

/// Control (probe p36b): the opener on its own line — green before the
/// fix (the newline reset the position state) and must stay green.
#[test]
fn issue2274_array_opener_on_own_line_control() {
    assert_array_ok(
        "opener-own-line",
        "f() {\nA=(\n1\n2\n);}; f; echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
}

/// Probe p36c: `if` body on the same line as `then`.
#[test]
fn issue2274_array_in_if_body_same_line() {
    assert_array_ok(
        "if-sameline",
        "if true; then A=(\n1\n2\n); echo \"A=${A[*]}\"; fi\n",
        "A=1 2\n",
    );
}

/// Probe p36d: subshell body on the same line as `(` (this face used to
/// report a stray-`)` syntax error instead of the EOF diagnostic).
#[test]
fn issue2274_array_in_subshell_same_line() {
    assert_array_ok(
        "subshell-sameline",
        "( A=(\n1\n2\n); echo \"A=${A[*]}\" )\n",
        "A=1 2\n",
    );
}

/// Top level: no compound wrapper at all.
#[test]
fn issue2274_array_at_top_level() {
    assert_array_ok("top-level", "A=(\n1\n2\n); echo \"A=${A[*]}\"\n", "A=1 2\n");
}

/// A brace group as the first statement of a function body.
#[test]
fn issue2274_array_in_group_inside_function() {
    assert_array_ok(
        "group-in-func",
        "f() { { A=(\n1\n); }; echo \"A=${A[*]}\"; }; f\n",
        "A=1\n",
    );
}

/// The remaining list openers on the same line: `do`, `else`, `!`, and
/// the connector family (`&&`, `||`) where whitespace flushed the
/// pending word before the separator arrived.
#[test]
fn issue2274_array_after_list_openers_and_connectors() {
    assert_array_ok(
        "do-sameline",
        "for i in x; do A=(\n1\n2\n); echo \"A=${A[*]}\"; done\n",
        "A=1 2\n",
    );
    assert_array_ok(
        "else-sameline",
        "if false; then :; else A=(\n1\n2\n); echo \"A=${A[*]}\"; fi\n",
        "A=1 2\n",
    );
    assert_array_ok("bang", "! A=(\n1\n2\n); echo \"A=${A[*]}\"\n", "A=1 2\n");
    assert_array_ok(
        "andand",
        "true && A=(\n1\n2\n); echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
    assert_array_ok(
        "oror",
        "false || A=(\n1\n2\n); echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
}

/// Declaration context on the same line (`local` as the body's first
/// word used to take the stray-`)` syntax-error face).
#[test]
fn issue2274_local_array_first_in_function() {
    assert_array_ok(
        "local-first",
        "f() { local A=(\n1\n2\n); echo \"A=${A[*]}\"; }; f\n",
        "A=1 2\n",
    );
}

/// Array bodies carrying the constructs the residual scan must skip as
/// units: comments, a backslash continuation, command/arithmetic
/// substitution elements, and subscripted elements.
#[test]
fn issue2274_array_body_constructs() {
    assert_array_ok(
        "comment-in-array",
        "f() { A=(1 # c )\n2\n);}; f; echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
    assert_array_ok(
        "continuation-in-array",
        "f() { A=(1 \\\n2\n);}; f; echo \"A=${A[*]}\"\n",
        "A=1 2\n",
    );
    assert_array_ok(
        "comsub-elements",
        "f() { A=($(echo x)\n$((1+2))\nend\n);}; f; echo \"A=${A[*]}\"\n",
        "A=x 3 end\n",
    );
    assert_array_ok(
        "subscript-elements",
        "f() { A=([0]=x\n[1]=y\n);}; f; echo \"A=${A[*]}\"\n",
        "A=x y\n",
    );
}

/// The completeness scanner serves every reader: the same shape via
/// `-c` and via stdin must behave identically to the script-file path.
#[test]
fn issue2274_array_same_line_via_dash_c_and_stdin() {
    let script = "f() { A=(\n1\n2\n);}; f; echo \"A=${A[*]}\"";
    let (stdout, stderr, code) = run_dash_c(script);
    assert_eq!(code, Some(0), "-c: rc, stderr={stderr}");
    assert_eq!(stdout, "A=1 2\n", "-c: stdout");
    let (stdout, stderr, code) = run_stdin(script);
    assert_eq!(code, Some(0), "stdin: rc, stderr={stderr}");
    assert_eq!(stdout, "A=1 2\n", "stdin: stdout");
}

/// #2241 respawn geometry: a re-printed prelude whose first definition
/// carries a multi-line array assignment on its definition line, with
/// `set -eET` + DEBUG/ERR traps whose handlers live in the same
/// prelude, and a background external command inside a function. The
/// nested process must complete cleanly — no `command not found`, no
/// EOF misjudgment.
#[test]
fn issue2274_respawn_prelude_geometry() {
    let script = "\
set -eET
bats_error_trap() { BATS_DEBUG_LAST_STACK_TRACE=(
\"source\"
\"trace\"
); }
bats_debug_trap() { :; }
trap 'bats_debug_trap' DEBUG
trap 'bats_error_trap' ERR
run_it() { /bin/true & wait $!; }
run_it
echo respawn-ok
";
    let (stdout, stderr, code) = run_script("respawn", script);
    assert_eq!(code, Some(0), "respawn: rc, stderr={stderr}");
    assert_eq!(stdout, "respawn-ok\n", "respawn: stdout");
    assert!(
        !stderr.contains("command not found"),
        "respawn: stderr must not contain 'command not found': {stderr}"
    );
}

/// Negative controls: genuinely unclosed constructs must still read as
/// incomplete and die with the EOF diagnostic family — the scanner's
/// real job must not break.
#[test]
fn issue2274_negative_controls_still_diagnosed() {
    let cases: [(&str, &str, &str); 4] = [
        ("neg-quote", "f() { echo \"abc\n}; f\n", "unexpected EOF"),
        ("neg-comsub", "A=$(echo x\necho done\n", "unexpected EOF"),
        // The brace family reports with the "unexpected end of file"
        // wording (GNU's yyerror shape), not the matched-pair wording.
        ("neg-brace", "f() { echo x\n", "unexpected end of file"),
        ("neg-array", "A=(1\n2\n", "unexpected EOF"),
    ];
    for (tag, script, needle) in cases {
        let (stdout, stderr, code) = run_script(tag, script);
        assert_ne!(code, Some(0), "{tag}: must fail, stdout={stdout}");
        assert!(
            stderr.contains(needle),
            "{tag}: stderr must contain {needle:?}: {stderr}"
        );
    }
}

/// Argument position must NOT become an assignment prefix region: GNU
/// rejects `echo A=(` at the `(` immediately. rubash's error recovery
/// text differs from GNU's (pre-existing divergence, unchanged by this
/// fix), but the run must fail with a syntax error — never execute as
/// if the array had been gathered.
#[test]
fn issue2274_argument_position_paren_is_not_compound() {
    let (_stdout, stderr, code) = run_script("neg-argpos", "echo A=(\n1\n)\n");
    assert_eq!(code, Some(2), "argpos: rc, stderr={stderr}");
    assert!(
        stderr.contains("syntax error"),
        "argpos: stderr must contain 'syntax error': {stderr}"
    );
}
