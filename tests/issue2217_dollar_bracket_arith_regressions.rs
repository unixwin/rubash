//! rubash#2217 — old-style arithmetic `$[expr]` was never expanded when
//! it appeared inside a double-quoted span: the lexer's double-quote
//! cookers (lexer/quotes.rs `remove_double_quoted_into` and its cursor
//! twin) treated a `$` not followed by `(`, `{`, a special parameter,
//! or a name start as a literal dollar and encoded it as DATA_DOLLAR,
//! so the expansion walkers — whose `$[` arms already existed and
//! worked unquoted — never saw a live `$[`. GNU subst.c param_expand
//! has a dedicated `case '['` (subst.c:10912) fed by
//! extract_arithmetic_subst (subst.c:1301): `$[` is an expansion
//! opener in every context where `$(` is, double quotes included.
//! The cookers now copy the whole `$[...]` span verbatim (plain
//! bracket counting — parse.y:5527 parse_matched_pair P_ARITH counts
//! at parse.y:3974-3980, the same rule the walkers' collectors and
//! the EOF hunt apply), the way they already copy `$(...)` spans.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! (2026-10-30). Unix-only, mirroring the #2210 pin file's harness.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2217-{tag}-{}", std::process::id()));
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

/// Run `script`, assert exit 0, return stdout.
fn run_script(case: &Case, script: &str) -> String {
    let (stdout, stderr, code) = run_script_full(case, script);
    assert_eq!(code, Some(0), "script failed: {stderr}");
    stdout
}

/// The filed repro and its immediate family: `$[...]` inside a
/// double-quoted argument expands exactly like the unquoted form.
#[test]
fn issue2217_dquoted_basic() {
    let case = make_case("dqbasic");
    let out = run_script(
        &case,
        r#"body='a\(b'
echo "x=$[1+1] $body"
echo "q=$[2*3]"
"#,
    );
    assert_eq!(out, "x=2 a\\(b\nq=6\n");
}

/// Inside double quotes: nesting, mixing with `$(( ))` both ways,
/// array subscripts (inner brackets nest), variables, empty
/// expression, and a trailing extra `]` staying literal.
#[test]
fn issue2217_dquoted_variants() {
    let case = make_case("dqvariants");
    let out = run_script(
        &case,
        r#"a=5
arr=(10 20 30)
echo "n=$[$[1+1]+1]"
echo "m=$(( $[1+1] + 1 ))"
echo "m2=$[ $((1+1)) + 1 ]"
echo "s=$[arr[1]+1]"
echo "v=$[a+1] u=$[nosuchvar+1]"
echo "e=$[ ]"
echo "x=$[1+1]]"
echo "p=$[(1+2)*3] t=$[ 1 ? 10 : 20 ]"
"#,
    );
    assert_eq!(out, "n=3\nm=3\nm2=3\ns=21\nv=6 u=1\ne=0\nx=2]\np=9 t=10\n");
}

/// Quotes inside the bracket expression, the expression itself inside
/// double quotes: `\"` is quote syntax for the arithmetic pre-expansion
/// (GNU expand_arith_string with Q_DOUBLE_QUOTES, subst.c arithsub
/// path) and the value evaluates; a single-quoted operand is passed to
/// the evaluator verbatim and is an arithmetic syntax error whose
/// diagnostic names the quoted text, the command is discarded, and
/// the next command sees status 1.
#[test]
fn issue2217_dquoted_inner_quotes() {
    let case = make_case("dqinner");
    let (stdout, stderr, code) = run_script_full(
        &case,
        r#"echo "dq2=$[\"1+1\"]"
echo "sq2=$['1+1']"
echo "after rc=$?"
"#,
    );
    assert_eq!(code, Some(0), "script failed: {stderr}");
    assert_eq!(stdout, "dq2=2\nafter rc=1\n");
    assert!(
        stderr.contains("'1+1': arithmetic syntax error: operand expected"),
        "unexpected stderr: {stderr}"
    );
}

/// A malformed expression inside double quotes: GNU reports the
/// arithmetic syntax error against the post-expansion string, the
/// enclosing command never runs, and the script continues with the
/// failure visible in `$?`.
#[test]
fn issue2217_dquoted_bad_expression() {
    let case = make_case("dqbad");
    let (stdout, stderr, code) = run_script_full(
        &case,
        r#"echo "bad=$[1+]"
echo "after rc=$?"
"#,
    );
    assert_eq!(code, Some(0), "script failed: {stderr}");
    assert_eq!(stdout, "after rc=1\n");
    assert!(
        stderr.contains("1+: arithmetic syntax error: operand expected"),
        "unexpected stderr: {stderr}"
    );
}

/// Assignment positions: an unquoted RHS already worked on master;
/// a double-quoted RHS was stored literally. Both are one class.
#[test]
fn issue2217_assignment_positions() {
    let case = make_case("assign");
    let out = run_script(
        &case,
        r#"v=$[1+1]
w="q=$[2*3]"
echo "u=$v"
echo "s5: $w"
"#,
    );
    assert_eq!(out, "u=2\ns5: q=6\n");
}

/// Unquoted controls (green on master — the walkers' arms were always
/// reachable there) plus heredoc, guarding the class boundary: the
/// fix must not disturb the paths that already worked.
#[test]
fn issue2217_unquoted_and_heredoc_controls() {
    let case = make_case("controls");
    let out = run_script(
        &case,
        r#"echo u1: $[1+1]
echo u2: $[1+1]abc
cat <<HD
hd: $[1+1]
HD
"#,
    );
    assert_eq!(out, "u1: 2\nu2: 2abc\nhd: 2\n");
}

/// Single-quoted `$[...]` stays literal (single quotes are data —
/// GNU prints it verbatim), and an escaped dollar in double quotes
/// stays literal too.
#[test]
fn issue2217_literal_contexts_stay_literal() {
    let case = make_case("literal");
    let out = run_script(
        &case,
        r#"echo 'sq: $[1+1]'
echo "esc: \$[1+1]"
"#,
    );
    assert_eq!(out, "sq: $[1+1]\nesc: $[1+1]\n");
}

/// Unterminated `$[` at end of input: the reader verdict — GNU aborts
/// the script with `unexpected EOF while looking for matching \`]'`
/// and status 2 (parse.y parse_matched_pair); output before the
/// construct still appears.
#[test]
fn issue2217_unterminated_eof_verdict() {
    let case = make_case("unterm");
    let (stdout, stderr, code) = run_script_full(&case, "echo \"before\"\necho $[1+1\n");
    assert_eq!(stdout, "before\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `]'"),
        "unexpected stderr: {stderr}"
    );
}
