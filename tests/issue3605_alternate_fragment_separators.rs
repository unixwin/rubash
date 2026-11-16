//! Issue rubash#3605 regression: the word list inside `${param OP word}`
//! must keep the whitespace BETWEEN its fragment words — GNU carries the
//! whole alternate as ONE word, so affix whitespace glues to boundary
//! fields and splits only at ambient-IFS whitespace.
//!
//! GNU anchors:
//! - subst.c:7840-7860 param_expand: the `-`/`+`/`=` rhs is expanded as a
//!   single word (the operator's `word` is one WORD_DESC from the lexer),
//!   so `${0+  "$@"  }` is affix ` ` + `"$@"` + affix ` ` — the whitespace
//!   concatenates onto the $@ boundary fields, it is not a word separator.
//! - expand_word_list_internal then field-splits ONLY the unquoted-IFS-
//!   whitespace runs of the assembled result: under a null or non-
//!   whitespace IFS the edge spaces survive (` < a b>` / ` <c >`), under
//!   the default IFS they split away (`<a b>` / `<c>`).
//! - Inside a double-quoted outer word (`"${v- "$@" }"`) nothing splits —
//!   every separator byte is literal affix text.
//!
//! rubash re-parses the fragment as `__rubash_parameter_alternate__
//! <fragment>` so its words can go through expand_command_word, and the
//! lexer discarded the inter-word whitespace: ` "$@" ` tokenized to a
//! bare `"$@"`, yielding `<a b>`/`<c>` where GNU yields ` < a b>`/` <c >`
//! under IFS='' or IFS=':'.
//!
//! Fix: expand_alternate_word_fragment recovers each separator as the
//! source gap between consecutive word raws and merge_fragment_fields
//! re-assembles GNU's single-word model — separator text glues to
//! boundary fields and splits only at ambient-IFS-whitespace bytes
//! (never inside a double-quoted outer word).
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probe /tmp/probe_final.sh
//! 2026-11-16).

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
/// text — same gluing as IFS=''.
#[test]
fn non_whitespace_ifs_keeps_separators_literal() {
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
