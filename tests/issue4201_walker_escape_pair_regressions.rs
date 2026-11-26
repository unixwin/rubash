//! rubash#4201: the second #4195 round. The first round's cooked-word
//! gate skipped the embedded walker's backslash arm for EVERY pair in
//! a protecting walk, on the theory that a surviving raw `\` in a
//! lexer-cooked word is always single-quoted span data. Three faces
//! proved the theory too broad — the pairs below are genuine escape
//! syntax arriving over raw-text channels, and resolving them is the
//! arm's job:
//!
//! 1. A comsub body's re-lexed word carries its double-quoted spans in
//!    escaped form (`"\"q\""` arrives as `\"\\\"q\\\"`); skipping the
//!    \" and \\ pairs turned `0"q"` into `0\q\` (the
//!    compat_issue_regressions pin caught this in CI — the first
//!    round's lane had not run the cli_tests target).
//! 2. An assignment value spliced in DoubleQuoted context keeps a
//!    source-level \` escape: the backquote is DATA, never a group
//!    opener (issue3337_07 / issue3354_05 caught this in the census).
//! 3. A `\\` pair left as a '...' span remnant in a cooked word is
//!    literal data (rubash#2999): collapsing it eats one backslash.
//!    The gate's introducer-only skip reaches the arm again, so the
//!    #2999 protect branch is load-bearing once more.
//!
//! The gate now skips the arm only for the expansion-introducer pairs
//! ($ in any context, a backquote in Unquoted context) — the #4195
//! F5-F8 remnants, whose following byte must stay live.

use std::process::Command;

fn run(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash")
}

#[test]
fn comsub_arith_body_escaped_quote_pairs_resolve() {
    // Face 1: same shape as the cli_tests pin — the word containing
    // $((...)) routes through the mutable embedded walker, and the
    // escaped-quote pairs inside the comsub body must resolve.
    let output = run(r##"echo $(echo $((x))"\"q\"")"##);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\"q\"\n");
}

#[test]
fn escaped_backquote_assignment_value_stays_data() {
    // Face 2: `\` before a backquote in an assignment whose value is
    // spliced in DoubleQuoted context (the procsub forces that route)
    // is an escape: the stored value is the literal backquote plus the
    // materialized /dev/fd path, with NO surviving backslash.
    let dir = std::env::temp_dir().join(format!("rubash-issue4201-bq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("v=\\`>(printf 'hello\\n' > cap.txt)\necho \"V:[${v}]\"\n")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.starts_with("V:[`/dev/fd/"), "stdout: {stdout}");
    let cap = std::fs::read_to_string(dir.join("cap.txt")).unwrap_or_default();
    assert_eq!(cap, "hello\n", "cap.txt: {cap:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn span_remnant_double_backslash_pair_is_preserved() {
    // Face 3: `'a\\b'` is a two-backslash span; cooked into a word with
    // a live substitution after it, the pair must survive verbatim
    // (GNU: the span reaches expansion CTLESC-protected).
    let output = run("x='a\\\\b'$(echo c); echo \"$x\"");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\\\\bc\n");
}
