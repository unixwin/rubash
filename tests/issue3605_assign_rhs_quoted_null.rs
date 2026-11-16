//! Issue rubash#3605 regression: `${var=rhs}` / `${var:=rhs}` must not
//! retain a quoted-null field where the `-`/`+` alternates anchor one.
//!
//! GNU anchors:
//! - subst.c:11843 / subst.c:11905: CTLNUL is emitted only when
//!   (W_NOSPLIT|W_EXPANDRHS|W_ASSIGNRHS) == W_EXPANDRHS. The `=` rhs word
//!   carries W_ASSIGNRHS (subst.c:4495), so the gate fails and an empty
//!   `""`/`''`/null-expansion span is discarded by the NOSPLIT2 branch
//!   (subst.c:11853, subst.c:11921).
//! - subst.c:3181-3192 list_string: a quoted null spliced in from a nested
//!   `${x- ""}` dies in remove_quoted_nulls.
//! - subst.c:8085: `t1 = temp ? dequote_string (temp) : savestring ("")` —
//!   the `=` result is dequoted before bind_variable and substitution, so
//!   the value that lands in the word is CTLNUL-free. ` "$x" ` with x=''
//!   yields "  ", which field-splits to zero words.
//! - Contrast: `-`/`+` return temp raw (subst.c:8080-8083), so their
//!   CTLNUL anchors an empty field (subst.c:3181-3186 QUOTED_NULL branch).
//!
//! rubash's `=` rhs reuses the alternate walker
//! (expand_embedded_parameters_alternate_mut), which emits
//! QUOTED_NULL_MARKER (U+E002, the CTLNUL port) for empty `""`/`''` spans
//! (embedded_mutations.rs). The `-`/`+` word-boundary splitter consumes
//! the marker; the `=` path had no consumer, so `recho ${v= "$x" }`
//! emitted one empty arg where GNU emits zero.
//!
//! Fix: expand_assignment_alternate_mut restores bare QUOTED_NULL
//! carriers from the `=` rhs result (E400-aware — a typed `$''`
//! is user data, rubash#2882).
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probe /tmp/probe_eq5.sh
//! 2026-11-16).

use std::process::Command;

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605qnull-{}-{}",
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

/// `${v= "$x" }` / `${v= '' }` / `${v= "$*"|"$@" }` with empty operands:
/// the quoted null is discarded, the value is the whitespace around it,
/// and that whitespace splits to zero words. GNU prints nothing for the
/// recho call but assigns the spaces to v.
#[test]
fn assign_rhs_quoted_empty_spans_split_to_zero_words() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "x=''\n",
        "set -- ''\n",
        "unset v; recho ${v= \"$x\" }\n",
        "unset v; recho ${v= \"$*\" }\n",
        "unset v; recho ${v= '' }\n",
        "unset v; recho ${v= \"$@\" }\n",
        "unset v; recho ${v:= \"$x\" }\n",
        "unset v; recho ${v=   }\n",
        "echo done\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "done\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The `=` result is both assigned and substituted: v gets the
/// whitespace-only value, and a quoted outer context keeps it verbatim.
#[test]
fn assign_rhs_value_is_whitespace_only() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "x=''\n",
        "unset v; recho ${v= \"$x\" }; recho \"v=<$v>\"\n",
        "unset v; recho \"${v= \"$x\" }\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <v=<  >>\nargv[1] = <  >\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A nested `${x- ""}` inside the `=` rhs contributes its quoted null to
/// the rhs word list, where GNU's remove_quoted_nulls drops it: the
/// nested alternate does NOT anchor an empty field here.
#[test]
fn nested_alternate_quoted_null_dies_in_assign_rhs() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "unset v x2; recho ${v= ${x2- \"\"} }; recho \"v=<$v>\"\n",
        "unset v x2; x2=1; recho ${v= ${x2+ \"\"} }\n",
        "echo done\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <v=<   >>\ndone\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Control: the `-`/`+` alternates still anchor their quoted null as one
/// empty field — only the `=`/`:=` rhs discards it.
#[test]
fn minus_plus_rhs_still_anchors_quoted_null() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "x=''\n",
        "unset v; recho ${v- \"$x\" }\n",
        "unset v; recho ${v- \"\" }\n",
        "v=AB; recho ${v+ \"$x\" }\n",
        "unset v; recho ${v+ \"$x\" }\n",
        "echo done\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <>\nargv[1] = <>\nargv[1] = <>\ndone\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `${v= a"$x"b }`: the quoted-null discard happens INSIDE the rhs, the
/// surrounding literal text still concatenates — GNU yields `<ab>`.
#[test]
fn assign_rhs_quoted_empty_concatenates_literal_halves() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "x=''\n",
        "unset v; recho ${v= a\"$x\"b }\n",
        "echo done\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <ab>\ndone\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// IFS='' keeps the whitespace value unsplit: `${v= "$*" }` yields
/// `<  >` (two spaces, no marker bytes).
#[test]
fn assign_rhs_under_null_ifs_keeps_spaces() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "unset v; IFS=''; set -- '' ''; recho ${v= \"$*\" }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <  >\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A user-typed `$''` in the `=` rhs is E400-escaped data, not a
/// bare carrier: the byte sequence survives into argv and the assigned
/// value (rubash#2882).
#[test]
fn typed_e002_in_assign_rhs_is_user_data() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "unset v; recho ${v= $'' }; recho \"v=<$v>\"\n",
        "unset v; recho ${v= x$''y }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        "argv[1] = <\u{E002}>\nargv[1] = <v=< \u{E002} >>\nargv[1] = <x\u{E002}y>\n"
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
