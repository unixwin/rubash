//! rubash#1656 — follow-up to rubash#1641, same parse surface: a
//! `case` clause with a missing or invalid terminator was silently
//! accepted and the FOLLOWING clause swallowed into the current body.
//! Seed: `case x in a) echo A;\;& b) echo B;; esac` ran rc 0 with no
//! output at all (subject x) instead of a syntax error.
//!
//! GNU grammar (parse.y:1237-1256 case_clause_sequence): after a
//! clause's compound_list only SEMI_SEMI / SEMI_AND / SEMI_SEMI_AND
//! or ESAC may follow. `\;&` is a word `;` backgrounded by `&`, not
//! SEMI_AND, so the `b)` that follows is a pattern close with no
//! grammar production admitting it: syntax error at that `)`, rc 2,
//! nothing executes.
//!
//! Expectations hand-measured against GNU bash 5.3.20 (script files):
//! every rejection below is rc 2, stdout empty (the `echo AFTER`
//! line never runs), stderr `syntax error near unexpected token `)'`.
//! Nuance pinned by the last test: `\;&` as the FINAL clause before
//! `esac` is legal (it is just a word plus `&` inside the body, and
//! ESAC closes the clause) — the error exists only when another
//! clause follows without a real terminator.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1656-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn assert_stray_paren_rejected(stdout: &str, stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(2), "rc: stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "", "nothing may execute: stderr={stderr:?}");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "stderr should name the stray `)': {stderr:?}"
    );
}

#[test]
fn issue1656_escaped_semi_amp_terminator_rejected() {
    let (stdout, stderr, code) =
        run_script("case x in a) echo A;\\;& b) echo B;; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_escaped_semi_escaped_amp_terminator_rejected() {
    let (stdout, stderr, code) =
        run_script("case x in a) echo A;\\;\\& b) echo B;; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_bad_terminator_rejected_even_when_clause_matches() {
    // Pre-fix this printed A and then misfired the swallowed clause as
    // commands; GNU rejects the whole case at parse time.
    let (stdout, stderr, code) =
        run_script("case a in a) echo A;\\;& b) echo B;; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_background_amp_is_not_a_terminator() {
    let (stdout, stderr, code) = run_script("case x in a) echo A & b) echo B;; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_stray_close_paren_mid_body_rejected() {
    let (stdout, stderr, code) = run_script("case x in a) echo A);; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_missing_terminator_juxtaposed_clause_rejected() {
    // `b` is just an argument of `echo`; the `)` after it has no
    // production (same missing-terminator shape).
    let (stdout, stderr, code) = run_script("case x in a) echo A b) echo B;; esac\necho AFTER\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1656_valid_terminators_still_work() {
    let (stdout, stderr, code) = run_script("case b in a) echo A;; b) echo B;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("B\n", Some(0)),
        "stderr={stderr:?}"
    );

    let (stdout, stderr, code) = run_script("case a in a) echo A;& b) echo B;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("A\nB\n", Some(0)),
        "stderr={stderr:?}"
    );

    let (stdout, stderr, code) = run_script("case a in a) echo A;;& a) echo A2;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("A\nA2\n", Some(0)),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue1656_paren_bearing_bodies_still_work() {
    // Subshell body: the parens balance inside the clause.
    let (stdout, stderr, code) = run_script("case x in x) (echo A);; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("A\n", Some(0)),
        "stderr={stderr:?}"
    );

    // Function definition body.
    let (stdout, stderr, code) = run_script("case x in x) f() { echo A; }; f;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("A\n", Some(0)),
        "stderr={stderr:?}"
    );

    // Nested case body.
    let (stdout, stderr, code) = run_script("case x in x) case y in y) echo N;; esac;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("N\n", Some(0)),
        "stderr={stderr:?}"
    );

    // Arithmetic command body.
    let (stdout, stderr, code) = run_script("case x in x) n=0; ((n++)); echo $n;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("1\n", Some(0)),
        "stderr={stderr:?}"
    );

    // Extglob pattern with a single close.
    let (stdout, stderr, code) = run_script("shopt -s extglob\ncase x in @(x)) echo A;; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("A\n", Some(0)),
        "stderr={stderr:?}"
    );

    // Quoted `)` is body text, not an operator.
    let (stdout, stderr, code) = run_script("case x in x) echo \"a)b\";; esac\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("a)b\n", Some(0)),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue1656_escaped_semi_amp_before_esac_is_legal() {
    // GNU: `\;&` in the FINAL clause parses as the word `;`
    // backgrounded by `&`; ESAC itself terminates the clause, so
    // there is no syntax error (rc 0; GNU also prints a
    // `;: command not found` runtime complaint, which is not pinned
    // here). The fix must not over-reject this shape.
    let (_stdout, stderr, code) = run_script("case a in a) echo A;\\;& esac\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert!(
        !stderr.contains("syntax error near unexpected token"),
        "final-clause \\;& must not become a parse error: {stderr:?}"
    );
}
