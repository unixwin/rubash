//! rubash#2432 — DIRSTACK element assignment: negative subscripts must
//! resolve against the live stack, and a subscript that stays negative
//! (or is empty / an all-elements spelling) must print the standard
//! `bad array subscript` diagnostic instead of failing silently.
//!
//! GNU: arrayfunc.c:420-434 assign_array_element_internal evaluates the
//! subscript with array_expand_index, resolves a negative result as
//! `array_max_index (array_cell (entry)) + 1 + ind` — for DIRSTACK the
//! entry's cell is get_dirstack's rebuild of the live stack
//! (variables.c:1619-1630 get_dirstack, invoked from find_variable) —
//! and calls err_badarraysub when the result is still negative. The
//! ALL_ELEMENT_SUB and empty-subscript rejections sit at
//! arrayfunc.c:355-360 / the sublen sanity check in the same function.
//! The bind then reaches variables.c:1612 assign_dirstack ->
//! builtins/pushd.def:657 set_dirstack_element, which silently drops
//! slot 0 and out-of-range slots with a successful status.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes; the scratch path is normalized to WORK and the
//! script path to SCRIPT, mirroring tests/issue2407_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2432-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["a", "b"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr) with the scratch path normalized to WORK and the
/// script path normalized to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
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
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn issue2432_negative_subscript_resolves_against_live_stack() {
    let dir = scratch("resolve");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
pushd "$1/a" >/dev/null
DIRSTACK[-1]=NEG
echo "rc=$? last=${DIRSTACK[-1]} all=${DIRSTACK[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "rc=0 last=NEG all=WORK/a WORK/b NEG\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2432_negative_subscript_on_fresh_stack_is_successful_noop() {
    // One-element stack: -1 resolves to slot 0, which
    // set_dirstack_element ignores — GNU reports success.
    let dir = scratch("fresh");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
DIRSTACK[-1]=NEG
echo "rc=$? top=${DIRSTACK[0]} count=${#DIRSTACK[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "rc=0 top=WORK/a count=1\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2432_bad_subscript_diagnostic_plain_and_declare() {
    // Repro B in script shape: the plain failure discards the rest of
    // its own line (GNU's assignment-error class), the declare failure
    // is an ordinary builtin failure with its diagnostic shown.
    let dir = scratch("diag");
    let (out, err) = run_script(
        &dir,
        r#"DIRSTACK[-2]=x; echo "plain rc=$?"
declare DIRSTACK[-2]=x; echo "declare rc=$?"
echo survived
"#,
    );
    assert_eq!(out, "declare rc=1\nsurvived\n");
    assert_eq!(
        err,
        "SCRIPT: line 1: DIRSTACK[-2]: bad array subscript\n\
         SCRIPT: line 2: DIRSTACK[-2]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2432_arithmetic_append_and_name_subscripts() {
    let dir = scratch("arith");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
pushd "$1/a" >/dev/null
DIRSTACK[1+1]=Q
echo "arith rc=$? all=${DIRSTACK[@]}"
DIRSTACK[1]+=X
echo "append rc=$? e1=${DIRSTACK[1]}"
DIRSTACK[abc]=x
echo "name rc=$?"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        "arith rc=0 all=WORK/a WORK/b Q\nappend rc=0 e1=WORK/bX\nname rc=0\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2432_all_element_and_empty_subscript_diagnostics() {
    let dir = scratch("at");
    let (out, err) = run_script(
        &dir,
        r#"DIRSTACK[@]=x
echo "after plain"
declare DIRSTACK[@]=x; echo "declare rc=$?"
DIRSTACK[]=x
echo "after empty"
"#,
    );
    assert_eq!(out, "after plain\ndeclare rc=1\nafter empty\n");
    assert_eq!(
        err,
        "SCRIPT: line 1: DIRSTACK[@]: bad array subscript\n\
         SCRIPT: line 3: DIRSTACK[@]: bad array subscript\n\
         SCRIPT: line 4: DIRSTACK[]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}
