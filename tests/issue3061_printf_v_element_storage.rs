//! Regression pins for rubash#3061 — a `printf -v 'x[0]'` element
//! bind stored the compound storage render (`\x1d([0]="bogus")`)
//! without recording the variable's array attribute in the VarTable's
//! structured flag map, so a scalar read-back of `x` returned the raw
//! storage string instead of projecting element 0.
//!
//! GNU ownership: builtins/printf.def:112 binds the -v operand with
//! `builtin_bind_variable (vname, vbuf, bindflags)` — the same
//! variables.c bind path as a plain `x[0]=bogus` assignment
//! (bind_variable -> bind_array_variable / assign_array_element), so
//! the variable becomes a real array cell and an unsubscripted read
//! is element 0 (subst.c array read of a bare array name). rubash's
//! printf path formatted the same storage render as the plain path
//! but marked the variable by raw string surgery on the serialized
//! `__RUBASH_ARRAY_VARS` entry; since the structured attribute map
//! became authoritative (no serialized string is maintained for the
//! ten attribute keys), the mark was invisible to `attr_of` and the
//! scalar projection in `shell_variable_value` fell through to the
//! raw bytes.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3061-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run a script file and return raw stdout bytes (the leak is a raw
/// 0x1D byte, so lossy string comparison is not good enough).
fn run_stdout(tag: &str, body: &str) -> Vec<u8> {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(BIN)
        .arg(&script)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    output.stdout
}

#[test]
fn issue3061_printf_v_element_scalar_readback_projects_element_zero() {
    let out = run_stdout("repro", "printf -v 'x[0]' bogus; printf '[%s]' \"$x\"\n");
    assert_eq!(
        out, b"[bogus]",
        "scalar read of a printf-bound array must be element 0, not the storage render: {out:?}"
    );
}

#[test]
fn issue3061_printf_v_sparse_index_scalar_readback_is_empty() {
    // Only element 2 is bound; GNU's bare-name read is element 0,
    // which is unset, so the expansion is empty.
    let out = run_stdout("sparse", "printf -v 'x[2]' two; printf '[%s]' \"$x\"\n");
    assert_eq!(
        out, b"[]",
        "sparse printf bind must not leak the storage render on scalar read: {out:?}"
    );
}

#[test]
fn issue3061_printf_v_two_elements_read_back() {
    let out = run_stdout(
        "append",
        "printf -v 'x[0]' a; printf -v 'x[1]' b; printf '[%s|%s|%s]' \"$x\" \"${x[0]}\" \"${x[1]}\"\n",
    );
    assert_eq!(out, b"[a|a|b]");
}

#[test]
fn issue3061_printf_v_overwrite_element_reads_back_new_value() {
    let out = run_stdout(
        "overwrite",
        "printf -v 'x[0]' a; printf -v 'x[0]' b; printf '[%s]' \"$x\"\n",
    );
    assert_eq!(out, b"[b]");
}

#[test]
fn issue3061_printf_v_element_over_existing_scalar_converts_to_array() {
    let out = run_stdout(
        "over-scalar",
        "x=scalar; printf -v 'x[0]' el; printf '[%s]' \"$x\"; declare -p x\n",
    );
    assert_eq!(out, b"[el]declare -a x=([0]=\"el\")\n");
}

#[test]
fn issue3061_printf_v_element_over_existing_array_keeps_siblings() {
    let out = run_stdout(
        "over-array",
        "x=(p q); printf -v 'x[1]' Q; printf '[%s|%s]' \"$x\" \"${x[1]}\"; declare -p x\n",
    );
    assert_eq!(out, b"[p|Q]declare -a x=([0]=\"p\" [1]=\"Q\")\n");
}

#[test]
fn issue3061_printf_v_element_reads_and_declare_stay_correct() {
    let out = run_stdout(
        "controls",
        "printf -v 'x[0]' bogus; printf '[%s|%s|%s]' \"${x[0]}\" \"${x[@]}\" \"${#x[@]}\"; declare -p x\n",
    );
    assert_eq!(out, b"[bogus|bogus|1]declare -a x=([0]=\"bogus\")\n");
}

#[test]
fn issue3061_printf_v_assoc_element_after_declare_stays_correct() {
    let out = run_stdout(
        "assoc",
        "declare -A m; printf -v 'm[k]' v; printf '[%s|%s]' \"${m[k]}\" \"$m\"; declare -p m\n",
    );
    assert_eq!(out, b"[v|]declare -A m=([k]=\"v\" )\n");
}

#[test]
fn issue3061_plain_element_assignment_control() {
    let out = run_stdout("plain", "y[0]=bogus; printf '[%s]' \"$y\"\n");
    assert_eq!(out, b"[bogus]");
}
