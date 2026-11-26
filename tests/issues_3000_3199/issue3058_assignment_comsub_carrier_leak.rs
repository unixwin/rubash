//! rubash#3058 — an assignment RHS combining a command substitution with
//! `${v:+":${v}"}` leaks a 0x1C carrier byte (IFS_GLUE) in front of every
//! space of the nested expansion's result.
//!
//! Root cause: the value takes the mixed-substitution assignment path
//! (`expand_mixed_command_substitution_assignment`), which splits out the
//! top-level `$(...)` spans and expands the remaining prefix/suffix
//! fragments with the embedded-parameter walker. A fragment expansion whose
//! site was quoted carries the QUOTED_EXPANSION_SITE verdict, and the walker
//! IFS-protects that expansion's result span — every IFS char of the result
//! is preceded by IFS_GLUE so a later field split cannot break it. That
//! protection is transport for word splitting; an assignment value never
//! field-splits (GNU subst.c:4357 `expand_string_assignment` expands the
//! RHS as ONE word under W_ASSIGNRHS and dequotes it into storage), so the
//! glue must be dropped at the store boundary. The sibling assignment paths
//! do drop it (the quoted path and the unquoted path in
//! `expand_assignment_value_inner` both strip IFS_GLUE at their tails), and
//! this same function already drops the bare ANSI_C_IFS_GUARD from its
//! fragments (rubash#796) — but the IFS_GLUE on the fragments was never
//! stripped, so it rode the payload splice into storage verbatim:
//! `v="$(echo x)/z${v:+":${v}"}"` stored `x/z:/c/aa\x1c bb`.
//!
//! GNU anchors: subst.c:4357 `expand_string_assignment` (RHS = one
//! W_ASSIGNRHS word, `dequote_list` into storage), subst.c:11552 (a quoted
//! portion "will be split, but this portion should not be" — the CTLESC
//! protection rationale), subst.c:11862 `add_quoted_string` →
//! subst.c:4773 `quote_string` (quoted results are protected during the
//! walk and dequoted at the boundary — protection never reaches storage).
//!
//! Oracle evidence: byte tables in the lane REPORT, GNU bash 5.3.20 vs
//! rubash master 873b30e6. Controls that are clean on BOTH shells and must
//! stay clean: no-cmdsub, no-nested-quotes, argument position, split
//! statements, a literal-space operator word, `export`/`readonly`
//! assignment statements, and a genuine 0x1C DATA byte delivered by a pure
//! command substitution (data bytes are payload, not glue).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the pin
//! is hermetic under any ambient locale. Script texts are raw strings so
//! what is pinned is byte-for-byte the probed shape.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(out, expected, "{ctx}: got {out:?}, want {expected:?}");
}

#[test]
fn comsub_plus_nested_quoted_param_stores_no_glue() {
    expect(
        r#"v='/c/aa bb'; v="$(echo x)/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "repro: comsub + ${v:+\":${v}\"} must store x/z:/c/aa bb, no 0x1C",
    );
}

#[test]
fn comsub_at_end_still_stores_no_glue() {
    expect(
        r#"v='/c/aa bb'; v="/z${v:+":${v}"}$(echo x)"; printf '[%s]' "$v""#,
        b"[/z:/c/aa bbx]",
        "cmdsub at the end: the braced expansion rides the prefix fragment",
    );
}

#[test]
fn conda_shaped_path_splice_stores_no_glue() {
    expect(
        r#"P='/c/Program Files/A:/c/Program Files/B'; P="$(echo /opt)/condabin${P:+":${P}"}"; printf '[%s]' "$P""#,
        b"[/opt/condabin:/c/Program Files/A:/c/Program Files/B]",
        "conda hook shape: every space in the spliced PATH stays a plain space",
    );
}

#[test]
fn unquoted_rhs_stores_no_glue() {
    expect(
        r#"v='/c/aa bb'; v=$(echo x)/z${v:+":${v}"}; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "unquoted RHS with the same trigger shape",
    );
}

#[test]
fn backtick_substitution_stores_no_glue() {
    expect(
        r#"v='/c/aa bb'; v="`echo x`/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "backtick span instead of $(...) takes the same mixed path",
    );
}

#[test]
fn two_comsubs_store_no_glue() {
    expect(
        r#"v='/c/aa bb'; v="$(echo x)/z${v:+":${v}"}$(echo y)"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bby]",
        "two top-level comsub spans around the braced expansion",
    );
}

#[test]
fn append_assignment_stores_no_glue() {
    expect(
        r#"v='/c/aa bb'; v+="$(echo x)/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[/c/aa bbx/z:/c/aa bb]",
        "+= appends the expanded RHS; the appended span carries no glue",
    );
}

#[test]
fn local_assignment_stores_no_glue() {
    expect(
        r#"f() { local v='/c/aa bb'; v="$(echo x)/z${v:+":${v}"}"; printf '[%s]' "$v"; }; f"#,
        b"[x/z:/c/aa bb]",
        "local assignment inside a function takes the same store boundary",
    );
}

#[test]
fn no_cmdsub_control_stays_clean() {
    expect(
        r#"v='/c/aa bb'; v="x/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "control: no comsub in the RHS (general quoted path)",
    );
}

#[test]
fn no_nested_quotes_control_stays_clean() {
    expect(
        r#"v='/c/aa bb'; v="$(echo x)/z${v}"; printf '[%s]' "$v""#,
        b"[x/z/c/aa bb]",
        "control: comsub + plain ${v}, no nested quoted operator word (no colon: it came from the \":${v}\" literal)",
    );
}

#[test]
fn argument_position_control_stays_clean() {
    expect(
        r#"v='/c/aa bb'; printf '[%s]' "$(echo x)/z${v:+":${v}"}""#,
        b"[x/z:/c/aa bb]",
        "control: same expansion in argument position (field split consumes glue)",
    );
}

#[test]
fn split_statements_control_stays_clean() {
    expect(
        r#"v='/c/aa bb'; c="$(echo x)"; v="$c/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "control: comsub split into its own statement first",
    );
}

#[test]
fn literal_space_operator_word_control_stays_clean() {
    expect(
        r#"w=1; v="$(echo x)${w:+a b}"; printf '[%s]' "$v""#,
        b"[xa b]",
        "control: operator word is a literal with a space, no nested expansion",
    );
}

#[test]
fn export_and_readonly_controls_stay_clean() {
    expect(
        r#"v='/c/aa bb'; export v="$(echo x)/z${v:+":${v}"}"; printf '[%s]' "$v""#,
        b"[x/z:/c/aa bb]",
        "control: export assignment statement",
    );
    expect(
        r#"v='/c/aa bb'; readonly v2="$(echo x)/z${v:+":${v}"}"; printf '[%s]' "$v2""#,
        b"[x/z:/c/aa bb]",
        "control: readonly assignment statement",
    );
}

#[test]
fn data_fs_byte_from_comsub_is_preserved() {
    expect(
        r#"v="$(printf 'a\034 b')"; printf '[%s]' "$v""#,
        b"[a\x1c b]",
        "a genuine 0x1C data byte in comsub output is payload, not glue",
    );
}

#[test]
fn paren_scalar_gate_3034_unaffected() {
    expect(
        r#"x=v1.0; y="($x)"; printf '[%s]' "$y""#,
        b"[(v1.0)]",
        "guard: the #3034 compound-claim gate shape stays fixed",
    );
}
