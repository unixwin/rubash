//! Issue unixwin/rubash#3605 (class B/D follow-ups): two related bugs
//! pinned against GNU bash 5.3.20 (local oracle, 2026-11-15).
//!
//! 1. ANSI-C `$'...'` inside a `$(`/`(` unit: several parenthesis /
//! quote walkers tracked `'` as a plain toggle, so an escaped quote
//! inside the ANSI-C string closed the span early — `\'` then let the
//! `)` byte inside the string balance the enclosing `$(`. The fix
//! feeds the ANSI-C span through the escape-aware close scan in
//! `skip_parenthesized_unit_ex`,
//! `skip_parenthesized_unit_corrected_ex`, `advance_quote_state`,
//! and the alias_stream paren walkers (GNU anchor:
//! parse.y:5476-5480 -> parse_matched_pair P_ALLOWESC; `\'` is an
//! escaped quote, never the close).
//!
//! 2. EOF exit status of the unmatched-delimiter diagnostics: GNU
//! splits the family on the innermost construct. Word-internal
//! matched pairs — `'`, `"`, backquote, `${`'s `}`, `[`'s `]`, and
//! the `$((`/`((` arithmetic `)` — die inside read_token_word at
//! EXECUTION_FAILURE = 1, while command-position constructs and the
//! `$(` hunt reported at the EOF line go through `error yacc_EOF`
//! -> EX_BADUSAGE = 2 (parse.y:489-490). error.c:324-327 forces 2
//! under live errexit. Every case below is measured byte-for-byte
//! against GNU 5.3.20.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3605_eof_rc_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Word-internal matched-pair EOF -> exit 2 in GNU bash 5.3.20.
#[test]
fn word_internal_eof_exits_two() {
    for (tag, script, closer) in [
        ("squote", "echo 'abc\n", '\''),
        ("dquote", "echo \"abc\n", '"'),
        ("backtick", "echo `abc\n", '`'),
        ("dollarbrace", "echo ${x\n", '}'),
        ("dollarbracket", "echo $[1+\n", ']'),
        ("subscript_cmd", "a[\n", ']'),
        ("subscript_assign", "v[a=\n", ']'),
        ("arith_exp", "echo $((1+\n", ')'),
        ("arith_cmd", "((x++\n", ')'),
    ] {
        let (_stdout, stderr, code) = run_file(script, tag);
        assert_eq!(code, 2, "{tag}: {script:?} -> {stderr:?}");
        let needle = format!("unexpected EOF while looking for matching `{closer}'");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
    }
}

/// Command-position constructs and the `$(` hunt keep exit 2.
#[test]
fn command_position_eof_exits_two() {
    for (tag, script) in [
        ("comsub", "echo $(echo x\n"),
        ("brace_group", "{ echo x;\n"),
        ("cond_cmd", "[[ -n x\n"),
        ("case", "case x in\n"),
        ("if_then", "if true; then\n"),
    ] {
        let (_stdout, stderr, code) = run_file(script, tag);
        assert_eq!(code, 2, "{tag}: {script:?} -> {stderr:?}");
        assert!(
            stderr.contains("unexpected EOF") || stderr.contains("unexpected end of file"),
            "{tag}: {stderr:?}"
        );
    }
}

/// ANSI-C `$'...\')...'` inside `$(` must not truncate the unit:
/// GNU runs the body and prints the decoded word.
#[test]
fn ansi_c_escaped_quote_inside_comsub_runs() {
    let (stdout, _stderr, code) = run_file("echo $(echo $'a\\')b')\n", "ansi_comsub");
    assert_eq!(code, 0);
    assert_eq!(stdout, "a')b\n");
}

/// The same shape without the escape leaves the trailing `'` open —
/// GNU 5.3.20 reports the quote EOF at line 1 with exit 2.
#[test]
fn unclosed_quote_after_comsub_exits_two() {
    let (_stdout, stderr, code) = run_file("echo $(echo $'a')b'\n", "stray_quote");
    assert_eq!(code, 2, "{stderr:?}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `''"),
        "{stderr:?}"
    );
}
