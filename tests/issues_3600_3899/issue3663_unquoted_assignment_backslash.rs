//! rubash#3663 — unquoted assignment `x=a\\*e` drops the backslash
//! before `*` and `?` (value becomes `a*e`), breaking the modernish
//! DBLBRACKET guard.
//!
//! Root cause (one fold site, one mechanism): the assignment value
//! reaches `expand_assignment_value_inner`
//! (`src/executor/assignment_expansion.rs`) with the lexer's
//! DATA_BACKSLASH carrier (U+0014) in place of a source `\\`
//! (`src/lexer/quotes.rs`: `\\` -> DATA_BACKSLASH, `\\*` -> CTLESC +
//! `*`). The assignment-RHS walker
//! (`expand_embedded_parameters_assignment_rhs_mut`,
//! `src/executor/embedded_mutations.rs`) restored that carrier to a raw
//! `\` in its tail, and the assignment path's final
//! `unescape_remaining_shell_escapes` pass then folded the raw `\` as
//! live source escape syntax (`\*` -> `*`, `\?` -> `?`). `\\[` and
//! `\\b` survived only because `[`/`b` are outside that pass's escape
//! set — the distinguishing table proves the fold site, not the matcher:
//! literal-source patterns (`[[ abcde == a\*e ]]`) and single-quoted
//! assignments were already correct on both shells.
//!
//! GNU anchor: `expand_string_assignment` (subst.c:4357-4392) runs ONE
//! `expand_word_internal` pass and then `dequote_list`; `dequote_string`
//! (subst.c:4807-4838) strips only CTLESC bytes, so a literal `\` in the
//! expanded value is data and is never re-read as an escape. The lexer
//! quotes the char after `\` once (parse.y:5368-5397 `read_token_word`,
//! parse.y:5694-5706 `got_escaped_character` CTLESC-marks it).
//!
//! The fix defers the DATA_BACKSLASH restore past the assignment path's
//! final unescape pass (the rubash#3088 defer-and-restore-later
//! discipline for the operator-word walker): the walker keeps the
//! carrier alive, `expand_assignment_value_inner` restores it after
//! quote removal. The unescape pass itself is untouched, so the
//! `${x:+a\*b}` -> `a*b` behavior it exists for (rubash#209) is
//! preserved, as is expansion-result protection (rubash#218).
//!
//! Fail-first matrix (GNU bash 5.3.20 vs rubash 27d3483a, verified with
//! the probe scripts under `~/workspace/fix-3663/`): the `*`/`?`
//! double-backslash shapes, the mixed `${y}`/append/star-plus-ques
//! shapes, and the DBLBRACKET `[[ ]]`/`case` guards fail on the old
//! binary; every other pin below is a control that was already green
//! and must stay green.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

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
fn issue3663_unquoted_double_backslash_star_keeps_backslash() {
    expect(
        "_Msh_test=a\\\\*e\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\*e>\n",
        "x=a\\\\*e must store a\\*e",
    );
}

#[test]
fn issue3663_unquoted_double_backslash_ques_keeps_backslash() {
    expect(
        "_Msh_test=a\\\\?b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\?b>\n",
        "x=a\\\\?b must store a\\?b",
    );
}

#[test]
fn issue3663_unquoted_double_backslash_bracket_control() {
    expect(
        "_Msh_test=a\\\\[b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\[b>\n",
        "x=a\\\\[b already stores a\\[b",
    );
}

#[test]
fn issue3663_unquoted_double_backslash_plain_control() {
    expect(
        "_Msh_test=a\\\\b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\b>\n",
        "x=a\\\\b already stores a\\b",
    );
}

#[test]
fn issue3663_single_quoted_assignment_control() {
    expect(
        "_Msh_test='a\\*e'\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\*e>\n",
        "quoted x='a\\*e' already stores a\\*e",
    );
}

#[test]
fn issue3663_non_assignment_word_control() {
    expect(
        "echo a\\\\*e\n",
        b"a\\*e\n",
        "echo a\\\\*e already prints a\\*e",
    );
}

#[test]
fn issue3663_triple_backslash_star_control() {
    expect(
        "_Msh_test=a\\\\\\*e\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\*e>\n",
        "x=a\\\\\\*e already stores a\\*e (\\\\ + \\* collapse once)",
    );
}

#[test]
fn issue3663_triple_backslash_ques_control() {
    expect(
        "_Msh_test=a\\\\\\?b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a\\?b>\n",
        "x=a\\\\\\?b already stores a\\?b",
    );
}

#[test]
fn issue3663_single_backslash_star_control() {
    expect(
        "_Msh_test=a\\*e\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a*e>\n",
        "x=a\\*e stores a*e (backslash quotes the star)",
    );
}

#[test]
fn issue3663_single_backslash_ques_control() {
    expect(
        "_Msh_test=a\\?b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a?b>\n",
        "x=a\\?b stores a?b (backslash quotes the ques)",
    );
}

#[test]
fn issue3663_single_backslash_bracket_control() {
    expect(
        "_Msh_test=a\\[b\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"value=<a[b>\n",
        "x=a\\[b stores a[b (backslash quotes the bracket)",
    );
}

#[test]
fn issue3663_alternate_word_star_control() {
    expect(
        "x=1; v=${x:+a\\*b}\nprintf 'value=<%s>\\n' \"$v\"\n",
        b"value=<a*b>\n",
        "v=${x:+a\\*b} stores a*b: the unescape pass must keep resolving live escapes (rubash#209)",
    );
}

#[test]
fn issue3663_alternate_word_double_backslash_star_control() {
    expect(
        "x=1; v=${x:+a\\\\*b}\nprintf 'value=<%s>\\n' \"$v\"\n",
        b"value=<a\\*b>\n",
        "v=${x:+a\\\\*b} stores a\\*b",
    );
}

#[test]
fn issue3663_parameter_result_backslash_control() {
    expect(
        "u='a\\*e'; v=${u}\nprintf 'value=<%s>\\n' \"$v\"\n",
        b"value=<a\\*e>\n",
        "v=${u} keeps expansion-result backslashes (rubash#218)",
    );
}

#[test]
fn issue3663_parameter_result_double_backslash_control() {
    expect(
        "u='a\\\\b'; v=${u}\nprintf 'value=<%s>\\n' \"$v\"\n",
        b"value=<a\\\\b>\n",
        "v=${u} keeps expansion-result double backslashes (rubash#218)",
    );
}

#[test]
fn issue3663_command_substitution_suffix_control() {
    expect(
        "x=$(echo hi)\\\\*e\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<hi\\*e>\n",
        "x=$(echo hi)\\\\*e already stores hi\\*e",
    );
}

#[test]
fn issue3663_mixed_parameter_prefix_keeps_backslash() {
    expect(
        "y=Z; x=${y}a\\\\*e\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<Za\\*e>\n",
        "x=${y}a\\\\*e must store Za\\*e",
    );
}

#[test]
fn issue3663_mixed_parameter_suffix_keeps_backslash() {
    expect(
        "y=Z; x=a\\\\*e${y}\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<a\\*eZ>\n",
        "x=a\\\\*e${y} must store a\\*eZ",
    );
}

#[test]
fn issue3663_append_assignment_keeps_backslash() {
    expect(
        "x=pre; x+=a\\\\*e\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<prea\\*e>\n",
        "x+=a\\\\*e must store prea\\*e",
    );
}

#[test]
fn issue3663_star_and_ques_in_one_word_keep_backslashes() {
    expect(
        "x=a\\\\*e\\?f\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<a\\*e?f>\n",
        "x=a\\\\*e\\?f must store a\\*e?f",
    );
}

#[test]
fn issue3663_dblbracket_guard_seed() {
    expect(
        "_Msh_test=a\\\\*e\n! eval '[[ abcde == ${_Msh_test} ]]'\necho \"guard-rc=$?\"\nprintf 'value=<%s>\\n' \"$_Msh_test\"\n",
        b"guard-rc=0\nvalue=<a\\*e>\n",
        "modernish DBLBRACKET guard: no match, value a\\*e",
    );
}

#[test]
fn issue3663_case_form_no_match() {
    expect(
        "_Msh_test=a\\\\*e\ncase abcde in (${_Msh_test}) echo MATCH;; (*) echo no-match;; esac\n",
        b"no-match\n",
        "case abcde in (a\\\\*e) must not match",
    );
}

#[test]
fn issue3663_literal_pattern_control() {
    expect(
        "if [[ abcde == a\\*e ]]; then echo MATCH; else echo no-match; fi\n",
        b"no-match\n",
        "literal [[ abcde == a\\*e ]] already answers no-match (matcher is not at fault)",
    );
}

#[test]
fn issue3663_array_element_control() {
    expect(
        "a=(x\\\\*e)\nprintf 'value=<%s>\\n' \"${a[0]}\"\n",
        b"value=<x\\*e>\n",
        "compound a=(x\\\\*e) already stores x\\*e",
    );
}

#[test]
fn issue3663_declare_assignment_control() {
    expect(
        "declare dx=a\\\\*e\nprintf 'value=<%s>\\n' \"$dx\"\n",
        b"value=<a\\*e>\n",
        "declare x=a\\\\*e already stores a\\*e",
    );
}

#[test]
fn issue3663_double_quoted_assignment_control() {
    expect(
        "x=\"a\\\\*e\"\nprintf 'value=<%s>\\n' \"$x\"\n",
        b"value=<a\\*e>\n",
        "x=\"a\\\\*e\" already stores a\\*e",
    );
}
