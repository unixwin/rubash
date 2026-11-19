//! Issue rubash#3605 regression: the rhs of `${param OP word}` keeps its
//! own separator semantics — GNU expands it via expand_string_for_rhs
//! (subst.c:4462-4501, W_NOSPLIT2), so literal IFS bytes stay raw and
//! delimit the deferred split, while whitespace separators glue onto the
//! boundary fields of the produced word list.
//!
//! GNU anchors:
//! - subst.c:7840-7860 param_expand -> parameter_brace_expand_rhs
//!   (subst.c:7960-8070): the `-`/`+`/`=` rhs produces a word list; a
//!   `$@`-class source inside it sets *qdollaratp so the deferred split
//!   fires even inside a quoted outer word.
//! - add_ifs_character (subst.c:~11950): under W_NOSPLIT2 literal IFS
//!   characters are NOT CTLESC-protected, so a `:` separator delimits at
//!   the deferred split (subst.c:12098-12160); whitespace separators are
//!   literal text that glues to boundary fields.
//! - The deferred split is gated on has_dollar_at: a rhs producing no
//!   `$@`-class list keeps its literal IFS bytes unsplit.
//!
//! Fix: expand_alternate_word_fragment emits a Text/WordBoundary/
//! DelimBoundary stream — whitespace separators are literal text (or a
//! word boundary at unquoted outer), non-whitespace IFS separators
//! delimit exactly when the fragment carries a `$@`-class source — and
//! merge_alternate_emits folds it into GNU's slot model (leading and
//! interior `:` produce empty fields, a trailing one is dropped).
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probes /tmp/probe_final.sh and
//! /tmp/pinfix.sh 2026-11-16).

use std::process::Command;

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605fragsep-{}-{}",
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

/// Default IFS: separators are all IFS whitespace, so they split — the
/// pre-existing flat behaviour was already GNU-correct here and must not
/// regress.
#[test]
fn default_ifs_separators_still_split() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v\n",
        "unset IFS\n",
        "recho ${v- \"$@\" }\n",
        "recho ${v- \"$@\" x }\n",
        "recho ${v- \"$@\" \"$@\" }\n",
        "recho ${v- x \"$@\" y }\n",
        "v=1\n",
        "recho ${v+ x \"$@\" y }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = <a b>\nargv[2] = <c>\n",
            "argv[1] = <a b>\nargv[2] = <c>\nargv[3] = <x>\n",
            "argv[1] = <a b>\nargv[2] = <c>\nargv[3] = <a b>\nargv[4] = <c>\n",
            "argv[1] = <x>\nargv[2] = <a b>\nargv[3] = <c>\nargv[4] = <y>\n",
            "argv[1] = <x>\nargv[2] = <a b>\nargv[3] = <c>\nargv[4] = <y>\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// IFS='': no separator is a delimiter, so edge/interior whitespace glues
/// verbatim onto the $@ boundary fields.
#[test]
fn null_ifs_glues_separators_to_boundary_fields() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v\n",
        "IFS=\n",
        "recho ${v- \"$@\" }\n",
        "recho ${v- \"$@\" \"$@\" }\n",
        "recho ${v- x \"$@\" y }\n",
        "recho ${v- \"$x\" \"$@\" }\n",
        "recho \"${v- \"$@\" }\"\n",
        "recho \"${v- x y }\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = < a b>\nargv[2] = <c >\n",
            "argv[1] = < a b>\nargv[2] = <c a b>\nargv[3] = <c >\n",
            "argv[1] = < x a b>\nargv[2] = <c y >\n",
            "argv[1] = <  a b>\nargv[2] = <c >\n",
            "argv[1] = < a b>\nargv[2] = <c >\n",
            "argv[1] = < x y >\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A non-whitespace IFS leaves the space separators as literal affix
/// text — same gluing as IFS=''. (Whitespace separators are never IFS
/// delimiters here; the IFS char itself delimits — next test.)
#[test]
fn non_whitespace_ifs_keeps_whitespace_separators_literal() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v x\n",
        "IFS=:\n",
        "recho ${v- \"$@\" }\n",
        "recho ${v- x \"$@\" y }\n",
        "recho ${v- \"$x\" \"$@\" }\n",
        "recho \"${v- \"$@\" }\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = < a b>\nargv[2] = <c >\n",
            "argv[1] = < x a b>\nargv[2] = <c y >\n",
            "argv[1] = <  a b>\nargv[2] = <c >\n",
            "argv[1] = < a b>\nargv[2] = <c >\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A literal non-whitespace IFS separator DELIMITS the rhs fields when
/// the fragment carries a `$@`-class source — quoted outer included —
/// and `'...'` inside `"${...}"` is literal text (GNU: the quote was
/// already consumed by the outer word's parse). Empty slots materialize
/// for leading/interior `:`; a trailing one is dropped.
#[test]
fn non_whitespace_ifs_separators_delimit_list_bearing_rhs() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v\n",
        "IFS=:\n",
        "recho ${v- x:\"$@\" }\n",
        "recho \"${v- x:\"$@\" }\"\n",
        "recho \"pre${v- x:\"$@\" }post\"\n",
        "recho \"${v- x:'$@' }\"\n",
        "recho ${v- x::\"$@\" }\n",
        "recho \"${v- :x:\"$@\" }\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = < x>\nargv[2] = <a b>\nargv[3] = <c >\n",
            "argv[1] = < x>\nargv[2] = <a b>\nargv[3] = <c >\n",
            "argv[1] = <pre x>\nargv[2] = <a b>\nargv[3] = <c post>\n",
            "argv[1] = < x>\nargv[2] = <'a b>\nargv[3] = <c' >\n",
            "argv[1] = < x>\nargv[2] = <>\nargv[3] = <a b>\nargv[4] = <c >\n",
            "argv[1] = < >\nargv[2] = <x>\nargv[3] = <a b>\nargv[4] = <c >\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
/// A fragment word that expands to zero fields does not break the glue:
/// the separators on both sides of it still concatenate through.
#[test]
fn empty_expanding_word_glues_through() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v e\n",
        "unset IFS\n",
        "recho ${v- \"$e\" mid \"$e\" }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <>\nargv[2] = <mid>\nargv[3] = <>\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Inside a double-quoted outer word every separator is literal text —
/// the rhs never field-splits, whatever the ambient IFS.
#[test]
fn double_quoted_outer_word_never_splits() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- 'a b' c\n",
        "unset v\n",
        "unset IFS\n",
        "recho \"${v- x y }\"\n",
        "IFS=:\n",
        "recho \"${v- \"$@\" }\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!("argv[1] = < x y >\n", "argv[1] = < a b>\nargv[2] = <c >\n",)
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
