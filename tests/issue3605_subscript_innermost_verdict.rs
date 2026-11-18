//! Issue unixwin/rubash#3605 (follow-up): the `[`-subscript inner scan
//! of `unclosed_array_subscript_walk` tracked quotes/`[`/`]` depth only
//! and read `$(`/`${`/`` ` `` bytes literally, so an EOF inside a dollar
//! word nested in the subscript was reported as the OUTER `]' verdict
//! instead of the innermost pending construct's.
//!
//! GNU anchor: inside `parse_matched_pair(P_ARRAYSUB)` a `$(`/`${`/`$[`
//! after `$` (LEX_WASDOL) goes to `parse_dollar_word`
//! (third_party/bash/parse.y:4140-4141) — the inner construct owns its
//! own scan, so a `]` inside it never closes the subscript, and an EOF
//! inside it produces the INNER diagnostic (`v[a=$(x` reports `)' at
//! the EOF line; `v[a=$(x])z` still reports `]' because the comsub is
//! balanced). Quotes recurse through the shellquote arm
//! (parse.y:4040-4051).
//!
//! The fix skips balanced `$(`/`${`/`` ` `` units atomically inside the
//! `[` scan (skip.rs `unclosed_array_subscript_walk` + the continuation
//! residual mirror) and defers to the generic close-char path — which
//! names the innermost pending construct — when the inner unit is
//! unclosed.
//!
//! Every message/line below is measured byte-for-byte against GNU
//! bash 5.3.20. Exit codes: the `$(` class already exits 2 like GNU;
//! the word-internal matched-pair statuses (1 per read_token_word
//! EXECUTION_FAILURE) land with the exit-status split in
//! devin/eof-wordpair-exit-status (Refs #3605), so the pins below
//! assert message and line and only pin the exit code where it is
//! already correct without that change.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3605_sub_{}_{}.sh", tag, std::process::id()));
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

/// Unclosed `$(`/`${`/`` ` `` inside the subscript: the verdict names the
/// INNER construct, not `]'. GNU line and closer are pinned; the `$(`
/// class also already matches GNU's exit 2.
#[test]
fn inner_dollar_word_owns_the_eof_verdict() {
    // (script, expected closer, expected report line, expected exit or None)
    for (tag, script, closer, line, code) in [
        ("comsub", "v[a=$(x\n", ')', 2usize, Some(2i32)),
        ("comsub_open", "v[a=$(\n", ')', 2, Some(2)),
        ("brace", "v[a=${x\n", '}', 1, None),
        ("backtick", "v[a=`x\n", '`', 1, None),
        ("arith", "v[a=$((1+\n", ')', 1, None),
        (
            "multiline_comsub",
            "v[a=$(echo y\necho z\n",
            ')',
            3,
            Some(2),
        ),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        if let Some(want) = code {
            assert_eq!(got, want, "{tag}: {script:?} -> {stderr:?}");
        }
        let needle = format!("line {line}: unexpected EOF while looking for matching `{closer}'");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
    }
}

/// Balanced inner units are skipped atomically: a `]` inside `$(...)`
/// does not close the subscript, so the EOF verdict stays `]' at the
/// `[` line.
#[test]
fn balanced_inner_unit_does_not_close_subscript() {
    for (tag, script) in [
        ("comsub_rbracket", "v[a=$(x])z\n"),
        ("comsub_tail", "v[a=$(x)y\n"),
        ("comsub_extra_paren", "v[a=$(x))x\n"),
        ("backtick_rbracket", "v[a=`x]y`\n"),
    ] {
        let (_stdout, stderr, _code) = run_file(script, tag);
        assert!(
            stderr.contains("line 1: unexpected EOF while looking for matching `]'"),
            "{tag}: {stderr:?}"
        );
    }
}
