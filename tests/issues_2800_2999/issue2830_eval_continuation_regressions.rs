//! rubash#2830 — a backslash inside a single-quoted segment disables the
//! quoted-IFS protection of the following byte (and, for expansion-trigger
//! bytes, desynchronizes the whole marking simulation), so a compound word
//! such as `'( a | b \<newline>| c)'${V:+}` field-splits inside the quoted
//! span. When that word is an `eval` argument, the manufactured `\ ` token
//! makes the eval'd text a syntax error — the modernish `is()` death point.
//!
//! GNU treats a single-quoted span as data per character:
//! `string_extract_single_quoted` is called with `allowesc == 0`
//! (subst.c:1088, subst.c:11887), so a backslash cannot quote or consume
//! the next byte; the extracted span then goes through
//! `add_quoted_string`/`quote_string` (subst.c:11862, subst.c:4773),
//! which CTLESC-prefixes every byte of the span. Every IFS byte in the
//! span is therefore split-proof regardless of its predecessor.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.

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
fn single_quoted_backslash_ifs_bytes_stay_one_field_with_unquoted_suffixes() {
    let (stdout, stderr, code) = rubash(
        "E=\n\
         printf '[%s]\\n' '( a | b \\\n| c)'${E:+}\n\
         printf '[%s]\\n' 'a\\\nb'$E\n\
         printf '[%s]\\n' 'a\\ b'${E}\n\
         printf '[%s]\\n' 'a\\\tb'$(printf '')\n\
         ( IFS=:; printf '[%s]\\n' 'a\\:b'`printf ''` )\n\
         for x in 'a\\\nb'$E; do printf 'for:[%s]\\n' \"$x\"; done",
    );
    assert_eq!(
        stdout,
        "[( a | b \\\n| c)]\n\
         [a\\\nb]\n\
         [a\\ b]\n\
         [a\\\tb]\n\
         [a\\:b]\n\
         for:[a\\\nb]\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn single_quoted_backslash_trigger_bytes_do_not_poison_later_quoted_ifs() {
    // A `\` before `$`, a backtick or `"` inside a single-quoted span is
    // still just two data bytes; it must not desynchronize the marking of
    // the quoted space that follows in the same span.
    let (stdout, stderr, code) = rubash(
        "E=\n\
         printf '[%s]\\n' 'a\\$b c'$E\n\
         printf '[%s]\\n' 'a\\`b c'${E}\n\
         printf '[%s]\\n' 'a\\\"b c'$E",
    );
    assert_eq!(
        stdout,
        "[a\\$b c]\n\
         [a\\`b c]\n\
         [a\\\"b c]\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_case_pattern_continuation_with_compound_argument_defines_function() {
    // The minimal modernish shape: the eval argument is one compound word
    // whose first single-quoted segment carries a case pattern table with
    // a backslash-newline continuation, followed by an unquoted expansion
    // segment and a closing quoted segment.
    let (stdout, stderr, code) = rubash(
        "E=\n\
         eval 'f2830() {\n\
         \tcase b in\n\
         \t( a | b \\\n\
         \t| c ) echo case-ok ;;\n\
         \tesac\n\
         }\n\
         \tx=1'${E:+}'\n\
         '\n\
         declare -F f2830 >/dev/null && echo DEFINED || echo MISSING\n\
         f2830",
    );
    assert_eq!(stdout, "DEFINED\ncase-ok\n", "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_modernish_pattern_table_shape_runs() {
    let (stdout, stderr, code) = rubash(
        "E=\n\
         eval 'is2830() {\n\
         \tcase $1 in\n\
         \t( present | reg | charspecial \\\n\
         \t| setuid | mine ) echo yes ;;\n\
         \t*) echo no ;;\n\
         \tesac\n\
         }\n\
         \tx=1'${E:+}'\n\
         '\n\
         is2830 setuid\n\
         is2830 absent",
    );
    assert_eq!(stdout, "yes\nno\n", "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn single_quoted_backslash_controls_stay_intact() {
    // Shapes that already agree with GNU: a purely single-quoted word, an
    // assignment RHS (no field splitting), and a quoted segment without a
    // backslash before its IFS bytes.
    let (stdout, stderr, code) = rubash(
        "E=\n\
         printf '[%s]\\n' 'a\\\nb'\n\
         x='a\\\nb'${E:+}; printf '[%s]\\n' \"$x\"\n\
         printf '[%s]\\n' 'ab'${E:+}",
    );
    assert_eq!(
        stdout,
        "[a\\\nb]\n\
         [a\\\nb]\n\
         [ab]\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
