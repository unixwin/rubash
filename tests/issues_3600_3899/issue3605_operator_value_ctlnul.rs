//! issue#3605: a `QUOTED_NULL_MARKER` sitting inside a stored variable value
//! is field-internal data, not an empty-field anchor. GNU
//! `parameter_brace_expand_rhs` (subst.c:8096-8173) returns the `=`/`:=`
//! result after `quote_escapes(t1)` (subst.c:8172) so the embedded CTLNUL
//! survives dequote as data, and `W_HASQUOTEDNULL` is forgotten whenever the
//! word is not exactly one CTLNUL (subst.c:8174-8179). The `-`/`:-` value
//! side returns the stored text verbatim (subst.c:8090 `w->word = temp`) —
//! the same dequote-and-drop rule. Only a marker freshly emitted by the
//! CURRENT word's RHS expansion anchors `<>`, so `${v- "$@" }` and
//! `${v+ "$@" }` keep theirs.
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probes 2026-11-16).

use std::process::Command;

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605opctl-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn assign_operator_unquoted_result_is_field_data() {
    // `sa ${v= "$*" }` unquoted: GNU yields ZERO arguments — the embedded
    // CTLNUL dequotes to nothing and the empty field drops.
    let (stdout, _, code) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; sa ${v= \"$*\" }");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(0));
}

#[test]
fn assign_result_does_not_anchor_empty_arg() {
    // `sa X${v= "$@" }` → GNU argv = `X` only (no `<>` anchor).
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; sa X${v= \"$@\" }");
    assert_eq!(stdout, "argv[1] = <X>\n");
}

#[test]
fn colon_assign_result_drops_embedded_marker() {
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; sa ${n:= \"$@\" }");
    assert_eq!(stdout, "");
}

#[test]
fn quoted_assign_result_keeps_inner_spaces() {
    // `"${v= "$@" }"` → one field ` ` … ` ` (GNU `quote_string` keeps the
    // dequoted spaces as data).
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; sa \"${v= \"$@\" }\"");
    assert_eq!(stdout, "argv[1] = <  >\n");
}

#[test]
fn dash_value_side_drops_marker() {
    // `${n- "$@" }` with `n` bound to ` \uE002 `: the stored value is verbatim
    // field data → GNU prints `CD`, never `C\uE002D` and never `C <> D`.
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; n=${v= \"$@\" }; sa C${n- \"$@\" }D");
    assert_eq!(stdout, "argv[1] = <C>\nargv[2] = <D>\n");
}

#[test]
fn colon_dash_value_side_drops_marker() {
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; n=${v= \"$@\" }; sa C${n:- \"$@\" }D");
    assert_eq!(stdout, "argv[1] = <C>\nargv[2] = <D>\n");
}

#[test]
fn plain_var_unquoted_read_drops_marker() {
    // `S $n T` with n bound to ` ␂ `: the stored marker dequotes and the
    // empty field drops — GNU `S`,`T`, never `S <E002> T`.
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; n=${v= \"$@\" }; sa S $n T");
    assert_eq!(stdout, "argv[1] = <S>\nargv[2] = <T>\n");
}

#[test]
fn plain_var_quoted_read_keeps_spaces() {
    // `"$n"` → the dequoted `  ` survives as one quoted field.
    let (stdout, _, _) = rubash_file("set -- ''; sa() { local i=1; for a in \"$@\"; do echo \"argv[$i] = <$a>\"; i=$((i+1)); done; }; n=${v= \"$@\" }; sa S \"$n\" T");
    assert_eq!(stdout, "argv[1] = <S>\nargv[2] = <  >\nargv[3] = <T>\n");
}

// Remaining residual (next family, NOT fixed here): the RHS-emitted quoted
// empty `"$@"` segment should GLUE to neighbouring fields — GNU
// `M${v- "$@" }N` → `MN` while rubash still splits it to `M`,`N`. That is the
// missing-`<>`/empty-arg-glue family on the emission side, orthogonal to the
// stored-value dequote fixed above.
