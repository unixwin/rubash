//! issue#530 regression: requesting a nameref and an array shape in one
//! declare must drop the -n, not keep it. GNU treats nameref and array/assoc
//! as mutually exclusive with the shape winning: declare.def:962-981 lifts
//! the nameref attribute out of the set mask (onref) and clears it whenever
//! the variable is or becomes an array, declare.def:841 reports the
//! existing-array conflict, and declare.def:206 keeps the chain transform
//! out of commands that toggle -n — so the declared variable stores and
//! routes values as a plain array afterwards.
//!
//! Oracle: GNU bash 5.3.0 (WSL) 2026-10-12. Global `declare -na r` prints
//! `declare -a r` (rc 0, silent); at function scope `local -na r` still
//! reports `local: r: reference variable cannot be an array` once while
//! leaving a plain local array behind; `x=v; declare -na x` moves the value
//! to element 0; a valueless nameref converts silently; assignments after
//! the declaration store into the array (`r=5` -> `[0]="5"`), never follow
//! a nameref.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_c(script: &str) -> Output {
    let output = Command::new(shell_bin())
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

/// The issue's exact repro: -n must be dropped from the global declaration.
#[test]
fn declare_na_drops_nameref_globally() {
    let out = run_c("declare -na r; echo rc=$?; declare -p r");
    assert_eq!(out.stdout, "rc=0\ndeclare -a r\n", "{:?}", out.stderr);
    assert_eq!(out.stderr, "", "no diagnostic at global scope");
}

/// -nA likewise collapses to a plain associative array.
#[test]
fn declare_na_assoc_drops_nameref() {
    let out = run_c("declare -nA y; echo rc=$?; declare -p y");
    assert_eq!(out.stdout, "rc=0\ndeclare -A y\n", "{:?}", out.stderr);
}

/// An existing scalar keeps its value, moved to element 0.
#[test]
fn declare_na_on_scalar_moves_value_to_element_zero() {
    let out = run_c("x=v; declare -na x; echo rc=$?; declare -p x");
    assert_eq!(
        out.stdout, "rc=0\ndeclare -a x=([0]=\"v\")\n",
        "{:?}",
        out.stderr
    );
}

/// Same for the associative shape.
#[test]
fn declare_nA_on_scalar_moves_value_to_element_zero() {
    let out = run_c("s7=hello; declare -nA s7; echo rc=$?; declare -p s7");
    assert_eq!(
        out.stdout, "rc=0\ndeclare -A s7=([0]=\"hello\" )\n",
        "{:?}",
        out.stderr
    );
}

/// Function scope: the local is still created as a plain array, with one
/// diagnostic on the way (GNU `local -na arr8`).
#[test]
fn local_na_reports_conflict_but_creates_plain_array() {
    let out = run_c("f(){ local -na arr8; declare -p arr8; }; f");
    assert_eq!(out.stdout, "declare -a arr8\n");
    assert!(
        out.stderr
            .contains("local: arr8: reference variable cannot be an array"),
        "{:?}",
        out.stderr
    );
}

/// Function scope shadowing a global scalar: local plain array, global
/// untouched, exactly one diagnostic.
#[test]
fn local_na_shadowing_global_scalar() {
    let out = run_c("x=v; f(){ local -na x; declare -p x; }; f; declare -p x");
    assert_eq!(out.stdout, "declare -a x\ndeclare -- x=\"v\"\n");
    assert_eq!(
        out.stderr
            .lines()
            .filter(|l| l.contains("cannot be an array"))
            .count(),
        1,
        "{:?}",
        out.stderr
    );
}

/// An existing indexed array plus -na is the declare.def:841 conflict: one
/// diagnostic, variable untouched.
#[test]
fn declare_na_on_existing_array_conflicts() {
    let out = run_c("declare -a q; declare -na q; echo rc=$?; declare -p q");
    assert_eq!(out.stdout, "rc=1\ndeclare -a q\n");
    assert!(
        out.stderr
            .contains("declare: q: reference variable cannot be an array"),
        "{:?}",
        out.stderr
    );
}

/// And for the associative shape.
#[test]
fn declare_nA_on_existing_assoc_conflicts() {
    let out = run_c("declare -A aq; declare -nA aq; echo rc=$?; declare -p aq");
    assert_eq!(out.stdout, "rc=1\ndeclare -A aq\n");
    assert!(
        out.stderr
            .contains("declare: aq: reference variable cannot be an array"),
        "{:?}",
        out.stderr
    );
}

/// A valueless nameref converts to a plain array silently (rc 0).
#[test]
fn declare_na_on_valueless_nameref_converts_silently() {
    let out = run_c("declare -n ref3; declare -na ref3; echo rc=$?; declare -p ref3");
    assert_eq!(out.stdout, "rc=0\ndeclare -a ref3\n", "{:?}", out.stderr);
}

/// A nameref carrying a target cell converts with the cell as element 0,
/// without following the chain (declare.def:206).
#[test]
fn declare_na_on_chain_nameref_converts_cell() {
    let out = run_c("declare -n ref=vv; declare -na ref; echo rc=$?; declare -p ref");
    assert_eq!(
        out.stdout, "rc=0\ndeclare -a ref=([0]=\"vv\")\n",
        "{:?}",
        out.stderr
    );
}

/// An assignment under -na binds plainly into the array, never through a
/// nameref (declare.def:979 onref=0 keeps ASS_NAMEREF off).
#[test]
fn assignment_with_na_binds_into_array_not_nameref() {
    let out = run_c("declare -na rv=tgt; tgt=zz; echo rc=$?; declare -p rv; echo v=${rv[@]}");
    assert_eq!(
        out.stdout, "rc=0\ndeclare -a rv=([0]=\"tgt\")\nv=tgt\n",
        "{:?}",
        out.stderr
    );
}

/// Later plain assignments route into the array (the observable routing
/// corruption the kept -n caused: GNU `declare -na r; r=5` stores [0]="5").
#[test]
fn later_assignment_stores_into_array() {
    let out = run_c("declare -na r4; r4=5; echo rc=$?; declare -p r4; echo at=${r4[@]}");
    assert_eq!(
        out.stdout, "rc=0\ndeclare -a r4=([0]=\"5\")\nat=5\n",
        "{:?}",
        out.stderr
    );
}

/// A scalar whose current value cannot be a nameref target still errors and
/// stays untouched (GNU :857 fires before the shape wins).
#[test]
fn declare_na_on_invalid_scalar_value_errors() {
    let out = run_c("w=5v; declare -na w; echo rc=$?; declare -p w");
    assert_eq!(out.stdout, "rc=1\ndeclare -- w=\"5v\"\n");
    assert!(
        out.stderr
            .contains("invalid variable name for name reference"),
        "{:?}",
        out.stderr
    );
}

/// A fresh operand with an invalid assignment value errors and creates
/// nothing.
#[test]
fn declare_na_with_invalid_assignment_value_creates_nothing() {
    let out = run_c("declare -na z5=5x; echo rc=$?; declare -p z5");
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr
            .contains("invalid variable name for name reference"),
        "{:?}",
        out.stderr
    );
    assert!(out.stderr.contains("z5: not found"), "{:?}", out.stderr);
}

/// The compound-assignment conflict keeps its existing behavior: one
/// diagnostic and the plain array still carries the elements (regression
/// guard).
#[test]
fn compound_assignment_still_conflicts_and_assigns() {
    let out = run_c("declare -na r9=(1 2); echo rc=$?; declare -p r9");
    assert_eq!(out.stdout, "rc=1\ndeclare -a r9=([0]=\"1\" [1]=\"2\")\n");
    assert!(
        out.stderr
            .contains("r9: reference variable cannot be an array"),
        "{:?}",
        out.stderr
    );
}

/// Readonly interplay: with a valid nameref value the readonly refusal wins
/// over the shape drop (GNU declare.def:864); with an invalid value the
/// invalid-name error fires first (declare.def:857).
#[test]
fn readonly_variable_still_refuses_na() {
    let out = run_c("readonly y=t; declare -na y; echo rc=$?; declare -p y");
    assert_eq!(out.stdout, "rc=1\ndeclare -r y=\"t\"\n");
    assert!(
        out.stderr.contains("declare: y: readonly variable"),
        "{:?}",
        out.stderr
    );

    let out = run_c("readonly x=1; declare -na x; echo rc=$?; declare -p x");
    assert_eq!(out.stdout, "rc=1\ndeclare -r x=\"1\"\n");
    assert!(
        out.stderr
            .contains("invalid variable name for name reference"),
        "{:?}",
        out.stderr
    );
}
