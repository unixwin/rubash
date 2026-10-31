//! rubash#2227 — pins for the face-2 half of the issue that this PR
//! lands: the EOF verdict for an unterminated `$[` hunt inside a
//! double-quoted span. The old span scan bounded the hunt by the
//! span's closing quote and named the DQUOTE at the SPAN's line; GNU
//! runs the hunt itself (extract_delimited_string, subst.c:1361-1525,
//! reached for `$[...]` at subst.c:1303), where the span's closing
//! quote opens a quote unit INSIDE the hunt whose EOF names the quote
//! at the UNIT's line. lexer/skip.rs now delegates the in-span hunt to
//! db_scan_bracket_hunt, so the verdict and its line come from the
//! hunt itself (including the q6 shape: a trailing quote closes the
//! unit the span's quote opened, and the hunt then names `]'`).
//!
//! The face-1 half (quote-state-aware `$[...]` extraction in the
//! lexer copiers and the expansion walkers' collectors) is NOT in
//! this PR: the recovered implementation delimited the unquoted and
//! error families correctly but regressed a body ending in a
//! deescaped backslash inside double quotes (the dblbs shape in
//! issue2227_backquote_and_escape_controls printed literally; master
//! and GNU agree on the arithmetic error) — the walker collector's
//! escape arm cannot distinguish a source escape from a cooked
//! backslash that came from a doubled one. That is carrier-layer
//! territory and needs its own brief; the implementation is
//! preserved in the fix workdir (face1-layers.diff). The two control
//! pins below hold GNU-agreeing shapes that already worked and must
//! not regress either way. Expectations hand-measured against GNU
//! bash 5.3.20 via script files (2026-10-31). Unix-only, mirroring
//! the #2217 pin file's harness.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2227-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run `script` as ./case.sh inside the case dir; return
/// (stdout, stderr, exit code).
fn run_script_full(case: &Case, script: &str) -> (String, String, Option<i32>) {
    let path = case.dir.join("case.sh");
    std::fs::write(&path, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Backquote bodies and escaped quotes inside a bracket body inside
/// double quotes (probe q3, lines 1/3/4): the backquote unit is
/// consumed whole, `\"` was already the #2217 control, and a doubled
/// backslash leaves a single literal backslash as the error token.
#[test]
fn issue2227_backquote_and_escape_controls() {
    let case = make_case("q3");
    let (stdout, stderr, code) = run_script_full(
        &case,
        r#"echo "bt: $[`echo 5`+1]"
echo "escq: $[\"1+1\"]"
echo "dblbs: $[1\\]+2"
echo "rc=$?"
"#,
    );
    assert_eq!(code, Some(0), "script failed: {stderr}");
    assert_eq!(stdout, "bt: 6\nescq: 2\nrc=1\n");
    assert!(
        stderr.contains(
            "1\\: arithmetic syntax error: invalid arithmetic operator (error token is \"\\\")"
        ),
        "unexpected stderr: {stderr}"
    );
}

/// Controls that already agreed with GNU and must not regress:
/// subscript brackets nest, a quoted subscript key works unquoted,
/// and a nested `$[...]` evaluates inside out.
#[test]
fn issue2227_nested_and_subscript_controls() {
    let case = make_case("controls");
    let (stdout, stderr, code) = run_script_full(
        &case,
        r#"a=(10 20 30)
echo "s1: $[a[1]+1]"
echo "s2: $[a["1"]+1]"
echo "n: $[$[1+1]+1]"
echo "bt: $[`echo 5`+1]"
"#,
    );
    assert_eq!(code, Some(0), "script failed: {stderr}");
    assert_eq!(stdout, "s1: 21\ns2: 21\nn: 3\nbt: 6\n");
}

/// Face 2 of the issue: an unterminated `$[` inside a double-quoted
/// span spanning lines. The span's closing quote opens a quote unit
/// inside the hunt, and the EOF verdict names that unit at ITS line
/// (line 2) — not the span's line (the old bounded-hunt model said
/// line 1). GNU 5.3.20: `line 2: unexpected EOF while looking for
/// matching `"'`, rc 2.
#[test]
fn issue2227_dquote_hunt_quote_unit_names_itself_at_its_line() {
    let case = make_case("f2b");
    let (stdout, stderr, code) = run_script_full(&case, "echo \"a $[1+\n2\"\n");
    assert_eq!(code, Some(2), "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("line 2: unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("line 1: unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "");
}

/// The same span with no closing quote at all: the hunt itself dies
/// at EOF and names `]'` at the hunt's line (GNU 5.3.20, rc 2).
#[test]
fn issue2227_dquote_hunt_without_closing_quote_names_bracket() {
    let case = make_case("f2a");
    let (stdout, stderr, code) = run_script_full(&case, "echo \"a $[1+\n2\n");
    assert_eq!(code, Some(2), "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `]'"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "");
}

/// A single-quote unit opened inside the hunt (itself inside a
/// double-quoted span) names itself at EOF — the old span scan
/// treated the `'` as data and the generic quote family blamed the
/// double quote. GNU 5.3.20 names `''` at line 1, rc 2.
#[test]
fn issue2227_squote_unit_inside_dquote_hunt_names_itself() {
    let case = make_case("x2");
    let (stdout, stderr, code) = run_script_full(&case, "echo \"a: $['x\n");
    assert_eq!(code, Some(2), "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `''"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "");
}

/// The trailing quote CLOSES the unit the span's closing quote opened,
/// so the hunt runs on to EOF and names `]'` (the q6 shape — the
/// verdict used to detour through the generic close-char family; it
/// now comes from the hunt itself, same text and line).
#[test]
fn issue2227_trailing_quote_closes_hunt_unit_then_bracket_eof() {
    let case = make_case("q6");
    let (stdout, stderr, code) = run_script_full(&case, "echo \"$[1+2\" \"\n");
    assert_eq!(code, Some(2), "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `]'"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "");
}
