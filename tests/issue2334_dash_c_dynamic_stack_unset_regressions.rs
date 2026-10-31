//! rubash#2334 — `-c` mode seeded BASH_SOURCE as set-but-empty instead
//! of UNSET (residual of #1306: the stored array was correctly emptied,
//! but the operator fetches fabricated a `Some("")` scalar for the
//! empty dynamic stack arrays, so the non-colon `-`/`+`/`?` tests saw
//! a SET parameter).
//!
//! GNU ground: subst.c:10131 (parameter_brace_expand) — var_is_set is
//! (temp != NULL) where temp is the get_string_value fetch, i.e.
//! element 0 of the ARRAY object (variables.c:1892
//! init_dynamic_array_var). An empty array has no element 0, so under
//! `bash -c` all five dynamic stack arrays (BASH_SOURCE, BASH_LINENO,
//! FUNCNAME, BASH_ARGV — BASH_ARGC's element 0 is the count "0") read
//! as UNSET for the operator tests while `declare -p` still shows the
//! empty array. Expectations hand-measured against GNU bash 5.3.20.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2334-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_args(args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    run_args(&["-c", script])
}

#[test]
fn issue_2334_dash_c_bash_source_minus_and_plus() {
    // The issue's regression seed, both shapes.
    let (stdout, _stderr, code) = run_c("echo src=[${BASH_SOURCE-UNSET}]");
    assert_eq!(stdout, "src=[UNSET]\n");
    assert_eq!(code, Some(0));
    let (stdout, _stderr, code) =
        run_args(&["-c", "echo src=[${BASH_SOURCE-UNSET}] zero=[$0]", "myname"]);
    assert_eq!(stdout, "src=[UNSET] zero=[myname]\n");
    assert_eq!(code, Some(0));
    // The `+` twin: an unset parameter substitutes nothing.
    let (stdout, _stderr, code) = run_c("echo plus=[${BASH_SOURCE+SET}]");
    assert_eq!(stdout, "plus=[]\n");
    assert_eq!(code, Some(0));
    // Colon forms were already right (set-but-null also substitutes);
    // pin them so the fix cannot trade one face for the other.
    let (stdout, _stderr, _code) = run_c("echo cm=[${BASH_SOURCE:-DEF}] cp=[${BASH_SOURCE:+ALT}]");
    assert_eq!(stdout, "cm=[DEF] cp=[]\n");
}

#[test]
fn issue_2334_dash_c_dynamic_stack_class_minus_and_plus() {
    // The whole dynamic-stack class shares the empty-array shape under
    // -c (GNU 5.3.20): BASH_LINENO, FUNCNAME and BASH_ARGV are equally
    // unset for the operator tests.
    for name in ["BASH_LINENO", "FUNCNAME", "BASH_ARGV"] {
        let script = format!("echo m=[${{{name}-UNSET}}] p=[${{{name}+SET}}]");
        let (stdout, _stderr, code) = run_c(&script);
        assert_eq!(stdout, "m=[UNSET] p=[]\n", "name: {name}");
        assert_eq!(code, Some(0), "name: {name}");
    }
    // Control: BASH_ARGC's element 0 is the argument count — set, "0".
    let (stdout, _stderr, _code) = run_c("echo m=[${BASH_ARGC-UNSET}] p=[${BASH_ARGC+SET}]");
    assert_eq!(stdout, "m=[0] p=[SET]\n");
    // Control: with real arguments the BASH_ARGV bottom frame exists —
    // element 0 is the LAST argument (the stack is reversed).
    let (stdout, _stderr, _code) = run_args(&[
        "-c",
        "echo a=[${BASH_ARGV-UNSET}] c=[${BASH_ARGC-UNSET}]",
        "nm",
        "arg1",
        "arg2",
    ]);
    assert_eq!(stdout, "a=[arg2] c=[2]\n");
}

#[test]
fn issue_2334_dash_c_bash_source_error_operator_fires() {
    // `${BASH_SOURCE?err}` on an unset parameter is a fatal expansion
    // error in GNU; the set-but-empty misread passed silently.
    let (stdout, stderr, code) = run_c("echo q=[${BASH_SOURCE?boom}] after");
    assert!(!stdout.contains("after"), "stdout: {stdout}");
    assert_ne!(code, Some(0));
    assert!(stderr.contains("BASH_SOURCE"), "stderr: {stderr}");
}

#[test]
fn issue_2334_dash_c_in_function_frames_are_set() {
    // Inside a function the frames exist: BASH_SOURCE[0] is the $0
    // operand, FUNCNAME[0] the function, BASH_LINENO[0] the call line;
    // BASH_ARGV stays unmaterialized without extdebug (GNU 5.3.20).
    let (stdout, _stderr, code) = run_args(&[
        "-c",
        "f() { echo \"src=[${BASH_SOURCE-UNSET}] fn=[${FUNCNAME-UNSET}] ln=[${BASH_LINENO-UNSET}] argv=[${BASH_ARGV-UNSET}]\"; }; f",
        "myname",
    ]);
    assert_eq!(stdout, "src=[myname] fn=[f] ln=[1] argv=[UNSET]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2334_script_mode_controls_stay_set() {
    // Script mode: the main frame gives BASH_LINENO element 0 = "0" and
    // BASH_SOURCE element 0 = the script path — both SET for `-`.
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(
        &path,
        "echo ln=[${BASH_LINENO-UNSET}] src=[${BASH_SOURCE:+SET}]\n",
    )
    .expect("write script");
    let (stdout, _stderr, code) = run_args(&[path.to_str().expect("utf8 path")]);
    assert_eq!(stdout, "ln=[0] src=[SET]\n");
    assert_eq!(code, Some(0));
}
