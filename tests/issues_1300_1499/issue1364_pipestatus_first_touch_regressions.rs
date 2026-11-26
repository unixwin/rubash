//! Issue rubash#1364: `readonly PIPESTATUS` does not freeze
//! pipeline-status updates (GNU leaves the cell empty), and the
//! whole "first touch" family around the PIPESTATUS cell shape:
//! a scalar bind or a bare attribute operand as the variable's
//! FIRST interaction must produce the cell GNU produces.
//!
//! GNU model (variables.c:6291 set_pipestatus_array): PIPESTATUS
//! has no standing cell. The status writer stores only into a cell
//! that currently holds an ARRAY object (array_p); a scalar cell
//! (`PIPESTATUS=99`) or a valueless att_invisible cell (bare
//! `readonly`/`export`/`declare` operand) freezes it, and an absent
//! cell is recreated as a fresh array. `declare -a` creates the
//! valueless cell WITH att_array, so a later scalar bind lands on
//! element 0 and the writer takes the cell over.
//!
//! Rubash models the cell as PipestatusShape (state.rs). Two
//! first-touch defects broke the model: the assignment store let
//! the static ARRAY_VARS mark turn a scalar bind into an element-0
//! array write, and the declaration view was gated off by the
//! scalar dynamic getter (which is None for a valueless cell), so
//! `declare -p` fell back to the env-table render with the static
//! `-a` flag baked in.
//!
//! Expected outputs verified against GNU bash 5.3.20. The sparse
//! element case (`PIPESTATUS[2]=5` keeps index 2) is deliberately
//! NOT pinned here: the status vector is a dense Vec, so that
//! residual (R3) awaits a sparse-representation change.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1364_readonly_first_touch_freezes_pipestatus() {
    // The issue body: readonly as the first interaction leaves a
    // valueless cell; the pipeline must not fill it.
    let (stdout, _, code) = rubash(
        r#"readonly PIPESTATUS; true|false; echo "ps=${PIPESTATUS[@]}"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ps=\ndeclare -r PIPESTATUS\n");
}

#[test]
fn issue1364_readonly_first_renders_without_array_flag() {
    let (stdout, _, code) = rubash(
        r#"readonly PIPESTATUS; echo "val:[$PIPESTATUS] set:[${PIPESTATUS+set}]"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val:[] set:[]\ndeclare -r PIPESTATUS\n");
}

#[test]
fn issue1364_readonly_pipe_update_stays_valueless() {
    let (stdout, _, code) = rubash(r#"readonly PIPESTATUS; false|true; declare -p PIPESTATUS"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -r PIPESTATUS\n");
}

#[test]
fn issue1364_export_first_renders_valueless_export() {
    let (stdout, _, code) = rubash(
        r#"export PIPESTATUS; echo "val:[$PIPESTATUS] set:[${PIPESTATUS+set}]"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val:[] set:[]\ndeclare -x PIPESTATUS\n");
}

#[test]
fn issue1364_declare_first_renders_valueless_plain() {
    let (stdout, _, code) = rubash(
        r#"declare PIPESTATUS; echo "val:[$PIPESTATUS] set:[${PIPESTATUS+set}]"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val:[] set:[]\ndeclare -- PIPESTATUS\n");
}

#[test]
fn issue1364_declare_array_first_renders_valueless_array() {
    let (stdout, _, code) = rubash(
        r#"declare -a PIPESTATUS; echo "val:[$PIPESTATUS] set:[${PIPESTATUS+set}]"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val:[] set:[]\ndeclare -a PIPESTATUS\n");
}

#[test]
fn issue1364_scalar_bind_freezes_against_pipelines() {
    let (stdout, _, code) = rubash(
        r#"PIPESTATUS=99; echo "[$PIPESTATUS]"; false|true; echo "[$PIPESTATUS]"; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[99]\n[99]\ndeclare -- PIPESTATUS=\"99\"\n");
}

#[test]
fn issue1364_scalar_export_print_has_no_array_form() {
    let (stdout, _, code) =
        rubash(r#"PIPESTATUS=99; export PIPESTATUS; export -p | grep PIPESTATUS"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x PIPESTATUS=\"99\"\n");
}

#[test]
fn issue1364_scalar_cell_element_reads() {
    // A scalar cell reads as an array of one for subscripts.
    let (stdout, _, code) = rubash(
        r#"PIPESTATUS=99; echo "${PIPESTATUS[0]}|${PIPESTATUS[1]}|${#PIPESTATUS[@]}|${PIPESTATUS[@]}""#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "99||1|99\n");
}

#[test]
fn issue1364_scalar_bind_on_valueless_cell_stays_scalar() {
    let (stdout, _, code) = rubash(r#"declare PIPESTATUS; PIPESTATUS=99; declare -p PIPESTATUS"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -- PIPESTATUS=\"99\"\n");
}

#[test]
fn issue1364_bind_on_valueless_array_cell_becomes_live() {
    // `declare -a` set att_array on the valueless cell, so the
    // scalar bind writes element 0 and the status writer takes the
    // cell over from that command's completion on.
    let (stdout, _, code) = rubash(
        r#"declare -a PIPESTATUS; PIPESTATUS=99; declare -p PIPESTATUS; false; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"1\")\n"
    );
}

#[test]
fn issue1364_unset_then_readonly_recreates_live_array() {
    // After `unset`, the name is an ordinary variable again: the
    // readonly cell the next command completes into a live array.
    let (stdout, _, code) = rubash(
        r#"unset PIPESTATUS; readonly PIPESTATUS; echo "val:[$PIPESTATUS]"; declare -p PIPESTATUS; false|true; declare -p PIPESTATUS"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "val:[0]\ndeclare -ar PIPESTATUS=([0]=\"0\")\ndeclare -ar PIPESTATUS=([0]=\"1\" [1]=\"0\")\n"
    );
}
