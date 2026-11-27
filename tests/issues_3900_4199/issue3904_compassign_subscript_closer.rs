//! Issue unixwin/rubash#3904 (A13): a `[` that follows a non-empty word
//! fragment inside a compound assignment list is literal word text, not
//! the P_ARRAYSUB subscript scan — so EOF reports the list's `)`, not `]`.
//!
//! GNU anchor: parse.y:5635-5639 admits the P_ARRAYSUB matched-pair scan
//! only when (a) the `[` follows a pure identifier at a command position
//! where assignment_acceptable(last_read_token) holds, or (b) the element
//! word starts with `[` (token_index == 0 && PST_COMPASSIGN). Inside
//! parse_compound_assignment's token loop (parse.y:7104-7169) the previous
//! element token is a WORD, never assignment-acceptable, so a mid-element
//! `[` (e.g. the `a[1` in `v=(a[1`) is literal: EOF falls through to the
//! loop's own `parser_error (orig_line_number, "unexpected EOF while
//! looking for matching `)'")` (parse.y:7151-7152) with
//! set_exit_status (EXECUTION_FAILURE) (parse.y:7167-7169), i.e. `)` at
//! the list's line, exit 1. An element-start `[` (`v=([1`, `v=(a [1`,
//! `v=(a\n[1`) DOES open the scan and reports `]` at its own line.
//!
//! The rubash defect was in the text-level subscript walk
//! (`unclosed_array_subscript_walk`, src/lexer/skip.rs): its
//! command-position tracking stayed true from input start through the
//! `name=(` opener, so `a[` inside the list looked like a
//! command-position identifier and opened a hunt that reported `]`.
//! The fix gates that disjunct on being outside any compound assignment
//! and treats a newline (or a comment-terminating newline) inside the
//! list as an element start, exactly like a space does.
//!
//! Every message/line/code below is measured byte-for-byte against GNU
//! bash 5.3.20 (oracle matrix) and re-confirmed against GNU bash 5.3.9.
//! The A29 face of #3904 (a bare `(` / `;` / `|` token inside the list
//! is a yyerror token error, not an EOF diagnostic) is NOT pinned here:
//! its rendering arm belongs in src/script_driver.rs, which open PR
//! #3903 is touching, so it waits for that PR to land.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3904_sub_{}_{}.sh", tag, std::process::id()));
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

/// Mid-element `[` inside a compound assignment reports the list's `)`
/// at the list's line with exit 1; element-start `[` keeps `]`; bare
/// command-position `[` keeps `]` with exit 2.
#[test]
fn compassign_subscript_closer_fidelity() {
    // (tag, script, expected closer, expected report line, expected exit)
    for (tag, script, closer, line, code) in [
        ("mid_elem", "v=(a[1", ')', 1usize, 1i32),
        ("mid_elem_nl", "v=(a[1\n", ')', 1, 1),
        ("mid_elem_semi_prefix", "echo hi; v=(a[1", ')', 1, 1),
        ("mid_elem_two", "v=(a[1 b[2", ')', 1, 1),
        ("mid_elem_nl_mid", "v=(a[1\nb", ')', 1, 1),
        ("mid_elem_nl_mid_two", "v=(a[1\nb[2", ')', 1, 1),
        // Guards: shapes that already reported `)` stay `)`.
        ("mid_elem_subeq", "v=(a[1]=x", ')', 1, 1),
        ("mid_elem_closed_bracket", "v=(a[1]", ')', 1, 1),
        ("mid_elem_declare", "declare -a v=(a[1", ')', 1, 1),
        // Element-start `[` opens the P_ARRAYSUB scan: `]` at its line.
        ("elem_start", "v=([1", ']', 1, 1),
        ("elem_start_space", "v=(a [1", ']', 1, 1),
        ("elem_start_nl_open", "v=(\n[1", ']', 2, 1),
        ("elem_start_nl", "v=(a\n[1", ']', 2, 1),
        ("elem_start_comment", "v=(a # c\n[1", ']', 2, 1),
        ("elem_start_nl_two", "v=(a\nb\n[1", ']', 3, 1),
        // Bare command-position `[` keeps `]` with exit 2.
        ("bare_cmd", "a[1", ']', 1, 2),
        ("after_closed_list", "v=(a) b[1", ']', 1, 2),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, code, "{tag}: {script:?} -> {stderr:?}");
        let needle = format!("line {line}: unexpected EOF while looking for matching `{closer}'");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
    }
}

/// The closed forms still parse: a mid-element `[` stays word text
/// (`a[1]=x` is element 0), an element-start `[sub]=` still assigns,
/// and an argument `[` stays literal.
#[test]
fn compassign_subscript_closed_forms_still_parse() {
    for (tag, script, stdout_needle) in [
        (
            "mid_elem_word_value",
            "v=(a[1]=x); declare -p v",
            "declare -a v=([0]=\"a[1]=x\")",
        ),
        (
            "elem_start_sub_value",
            "v=([1]=x); declare -p v",
            "declare -a v=([1]=\"x\")",
        ),
        ("arg_literal", "echo a[1", "a[1"),
    ] {
        let (stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, 0, "{tag}: {script:?} -> {stderr:?}");
        assert!(
            stdout.contains(stdout_needle),
            "{tag}: stdout {stdout:?} lacks {stdout_needle:?}"
        );
    }
}
