//! rubash#3613 — `declare -a b=([${x}]=v)` with a triple-expanding
//! subscript word must report the GNU error token and leave `b` unset.
//!
//! Chain (`y='`echo 1`'; x='$y'`): the element word expands `${x}` ->
//! `$y`; array_expand_index's `expand_arith_string` (arrayfunc.c:1368)
//! expands again -> `` `echo 1` ``; `evalexp` then fails with error
//! token `` `echo 1` `` -> `evalerror` -> DISCARD, so `b` keeps the `-a`
//! mark but stays unset (`declare -a b`).
//!
//! rubash kept pass-1's data carriers (E30A) into `evalexp` — the token
//! leaked `\u{E30A}y` and the operand bound `b=()`. The carriers now
//! decode at the eval entry (expand_arithmetic_subscript_mut), and the
//! declare rewrite abandons the operand on evalerror while keeping the
//! attribute marks.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3613-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir("declare");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, output.status.code())
}

#[test]
fn declare_compound_triple_expansion_reports_expanded_token() {
    let (out, err, _rc) = run_script(
        "y='`echo 1`'; x='$y'; declare -a b=([${x}]=v)\necho \"rc=$?\"\ndeclare -p b 2>&1\n",
    );
    assert!(err.contains("error token is \"`echo 1`\""), "err={err}");
    assert_eq!(out, "rc=1\ndeclare -a b\n", "out={out} err={err}");
}

#[test]
fn declare_compound_evalerror_leaves_name_declared_unset() {
    // Same verdict through a second probe: `declare -p` exit status for a
    // declared-but-unset name is 0 with the `declare -a` line, never
    // `declare: b: not found`.
    let (out, err, _rc) = run_script(
        "y='`echo 1`'; x='$y'; declare -a b=([${x}]=v)\ndeclare -p b\necho \"p_rc=$?\"\n",
    );
    assert_eq!(out, "declare -a b\np_rc=0\n", "out={out} err={err}");
    assert!(err.contains("`echo 1`"), "err={err}");
    assert!(!err.contains("not found"), "err={err}");
}

#[test]
fn plain_compound_evalerror_still_binds_empty_array() {
    // GNU: `b=([${x}]=v)` (no declare) converts b to an array BEFORE the
    // element loop — the evalerror still leaves `b=()` bound.
    let (out, err, _rc) = run_script("y='`echo 1`'; x='$y'\nb=([${x}]=v)\ndeclare -p b\n");
    assert_eq!(out, "declare -a b=()\n", "out={out} err={err}");
    assert!(err.contains("error token is \"`echo 1`\""), "err={err}");
}

#[test]
fn indexed_element_assign_keeps_produced_dollar_literal() {
    // `a[${x}]=v`: word expansion produces `$y` CTLESC-protected, so
    // evalexp's junk token is the literal `$y` — GNU reports `$y` here
    // where the compound-declare arm reports `` `echo 1` `` (verified
    // byte-identical).
    let (out, err, _rc) = run_script("y='`echo 1`'; x='$y'\na[${x}]=v\ndeclare -p a 2>&1\n");
    assert_eq!(
        out, "probe.sh: line 3: declare: a: not found\n",
        "out={out} err={err}"
    );
    assert!(err.contains("error token is \"$y\""), "err={err}");
}
