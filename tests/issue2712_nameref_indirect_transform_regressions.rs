//! rubash#2712 (owner decision D2, 2026-11-04) — replicate GNU's
//! `${!nr@A}` nameref rendering quirk: the target NAME as the value.
//!
//! GNU subst.c: parameter_brace_transform (8923) receives the
//! expansion's computed word as its value input, while
//! get_var_and_type (8529) independently resolves the variable
//! identity (name + attribute flags). For a nameref holder,
//! parameter_brace_expand_indir's ksh93 shortcut (subst.c:7896-7903)
//! makes that word the nameref cell's stored string — the target
//! NAME — so `${!nr@A}` renders `tgt='tgt'`, never `tgt='hello'`.
//! The same plumbing feeds every value-consuming transform (@Q,
//! @K/@k, the case/quote family); @a reads identity only and was
//! never wrong. Array/assoc targets render from their cells
//! (VT_ARRAYVAR / VT_ARRAYMEMBER) and ignore the word.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-04).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_stdout(script: &str, expected: &str) {
    let run = run_c(script);
    assert_eq!(run.stdout, expected, "script: {script}");
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// The headline face: @A through a nameref renders the target's
/// NAME as the value.
#[test]
fn issue2712_nameref_at_a_renders_target_name_as_value() {
    assert_stdout(
        "tgt=hello; declare -n nr=tgt; echo \"<${!nr@A}>\"",
        "<tgt='tgt'>\n",
    );
}

/// The rendered value is the cell string even when the target's
/// value would need quoting — it is the NAME that is quoted.
#[test]
fn issue2712_nameref_at_a_ignores_target_value_text() {
    assert_stdout(
        "tgt=\"a b\"; declare -n nr=tgt; echo \"<${!nr@A}>\"",
        "<tgt='tgt'>\n",
    );
}

/// The attribute flags come from the TARGET's cell (readonly here),
/// the value from the nameref cell string — the two inputs stay
/// independent, exactly as in get_var_and_type.
#[test]
fn issue2712_nameref_at_a_flags_come_from_target_cell() {
    assert_stdout(
        "declare -r tgt=hello; declare -n nr=tgt; echo \"<${!nr@A}>\"",
        "<declare -r tgt='tgt'>\n",
    );
}

/// A nameref CHAIN resolves to the final target for both inputs
/// (GNU find_variable_last_nameref).
#[test]
fn issue2712_nameref_chain_at_a() {
    assert_stdout(
        "tgt=hello; declare -n nr2=tgt; declare -n nr=nr2; echo \"<${!nr@A}>\"",
        "<tgt='tgt'>\n",
    );
}

/// The same value-input plumbing feeds @Q and @K: both render the
/// cell string, not the target's value.
#[test]
fn issue2712_nameref_at_q_and_at_k_use_cell_string() {
    assert_stdout(
        "tgt=hello; declare -n nr=tgt; echo \"<${!nr@Q}>\"",
        "<'tgt'>\n",
    );
    assert_stdout(
        "tgt=hello; declare -n nr=tgt; echo \"<${!nr@K}>\"",
        "<'tgt'>\n",
    );
}

/// @a reads the resolved variable's attributes only — no value
/// input — and stays empty for an attribute-less target.
#[test]
fn issue2712_nameref_at_a_attrs_unchanged() {
    assert_stdout("tgt=hello; declare -n nr=tgt; echo \"<${!nr@a}>\"", "<>\n");
}

/// A nameref whose target does not exist renders empty: @A is
/// the one transform that consults the target CELL
/// (string_transform's 'A' case returns NULL for a missing cell),
/// even though the plain expansion and @Q still yield the name.
#[test]
fn issue2712_nameref_to_unset_target_at_a_is_empty() {
    assert_stdout("declare -n nr=missing; echo \"<${!nr@A}>\"", "<>\n");
}

/// A declared-but-unset target renders its attributes with NO
/// value body (subst.c:8652: var_isset == 0 makes the rendered
/// value NULL) — the cell string must not leak in as a body.
#[test]
fn issue2712_nameref_to_declared_unset_target_at_a_has_no_body() {
    assert_stdout(
        "declare -rx tgt; declare -n nr=tgt; echo \"<${!nr@A}>\"",
        "<declare -rx tgt>\n",
    );
    assert_stdout(
        "declare -r tgt; declare -n nr=tgt; echo \"<${!nr@A}>\"",
        "<declare -r tgt>\n",
    );
}

/// Guard: plain indirection through a NON-nameref holder keeps the
/// classic rendering — the word there IS the target's value.
#[test]
fn issue2712_plain_indirection_at_a_unchanged() {
    assert_stdout("n=z; z=deep; echo \"<${!n@A}>\"", "<z='deep'>\n");
}

/// Guard: an ARRAY target renders from its cell (VT_ARRAYVAR) —
/// the nameref word does not leak into the element rendering.
#[test]
fn issue2712_nameref_to_array_at_a_renders_from_cell() {
    assert_stdout(
        "arr=(x y); declare -n nr=arr; echo \"<${!nr@A}>\"",
        "<declare -a arr='x'>\n",
    );
}

/// Guard: the direct (non-indirect) @A of the target itself is
/// untouched.
#[test]
fn issue2712_direct_at_a_unchanged() {
    assert_stdout("tgt=hello; echo \"<${tgt@A}>\"", "<tgt='hello'>\n");
}
