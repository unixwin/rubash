//! rubash#3203 regressions: after a completed `(( ))` arithmetic command
//! or `[[ ]]` conditional command, GNU is in assignment-acceptable
//! position — `reserved_word_acceptable` (parse.y:5899-5940) includes
//! ARITH_CMD and COND_END in the set of previous tokens after which
//! `assignment_acceptable` (parse.y:3163) holds, so `read_token_word`'s
//! `[` span consumption (parse.y:5636) reads `a[1 2]=3` as ONE
//! assignment word, whitespace inside the subscript included.
//!
//! rubash's scanner mirror `reserved_word_position()` had neither arm:
//! the folded arithmetic token (Keyword kind, whole `((…))` text as its
//! value) matched no keyword literal, and a genuine `]]` closer matched
//! no arm at all, so the span broke at the space and the diagnostic
//! named `a[1` instead of the whole word. The `]]` arm must be driven
//! by the reader state bit that `record_token`'s conditional-close arm
//! sets when PST_CONDCMD actually clears — never by the word VALUE
//! `]]`, which would release ordinary argument-position `]]` words
//! into compound-after treatment (see the argc control below).
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle); on the unfixed tree the
//! three defect cells name `a[1` and fail, every control passes.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3203-{}-{}",
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

/// Run a script that must succeed: exact stdout, empty stderr, rc 0.
fn check_ok(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, "", "script: {script:?}");
}

/// Run a script that must stay a syntax error: rc 2, nothing on stdout,
/// the diagnostic names the expected offending token, and the source
/// line is echoed back (both diagnostic lines match GNU's shape).
fn check_syntax_error(script: &str, token: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "script: {script:?}, stderr: {stderr}"
    );
    let first_line = script.lines().next().unwrap_or("");
    assert!(
        stderr.contains(first_line),
        "script: {script:?}, stderr should echo the source line, stderr: {stderr}"
    );
}

#[test]
fn subscript_span_after_arith_command() {
    // Probe cell j7: GNU names the whole assignment word.
    check_syntax_error("((1)) a[1 2]=3\necho after\n", "a[1 2]=3");
}

#[test]
fn subscript_span_after_cond_command() {
    // Probe cell j7b: the COND_END arm — same whole-word naming.
    check_syntax_error("[[ x ]] a[1 2]=3\necho after\n", "a[1 2]=3");
}

#[test]
fn subscript_span_after_arith_command_no_assignment() {
    // Probe cell j7c: no `=3` — the span still swallows the space.
    check_syntax_error("((1)) a[1 2]\necho after\n", "a[1 2]");
}

#[test]
fn adjacent_cond_after_arith_names_cond_opener() {
    // Probe cell j3 (negative scan): the `[[' classification after a
    // compound already agreed with GNU before this fix.
    check_syntax_error("((1)) [[ x ]]\necho after\n", "[[");
}

#[test]
fn adjacent_cond_after_cond_names_cond_opener() {
    // Probe cell j4 (negative scan).
    check_syntax_error("[[ x ]] [[ y ]]\necho after\n", "[[");
}

#[test]
fn adjacent_group_after_arith_names_open_brace() {
    // Probe cell j5 (negative scan).
    check_syntax_error("((1)) { echo in; }\necho after\n", "{");
}

#[test]
fn stray_close_brace_after_arith_names_close_brace() {
    // Probe cell j6 (negative scan).
    check_syntax_error("((1)) }\necho after\n", "}");
}

#[test]
fn ordinary_word_close_brackets_echo() {
    // An argument-position `]]` is an ordinary word, not COND_END.
    check_ok("echo ]]\n", "]]\n");
}

#[test]
fn ordinary_word_close_brackets_assignment_value() {
    // `]]` as an assignment VALUE is ordinary word text too.
    check_ok("x=]]\necho \"[$x]\"\n", "[]]]\n");
}

#[test]
fn argument_word_after_ordinary_close_brackets_stays_split() {
    // The state-bit discriminator: after an ordinary argument word
    // `]]`, a following `a[1 2]=3` is argument text and its `[` span
    // must NOT swallow the space — it arrives as two arguments.
    // A value-based `]]` release in the scanner mirror would merge it
    // into one argument (argc=2) and fail here.
    check_ok(
        "f() { printf \"argc=%s\\n\" \"$#\"; printf \"<%s>\\n\" \"$@\"; }\nf ]] a[1 2]=3\n",
        "argc=3\n<]]>\n<a[1>\n<2]=3>\n",
    );
}

#[test]
fn real_conditional_still_closes_and_runs() {
    // The genuine COND_END path must keep working end to end.
    check_ok("[[ x == x ]] && echo condok\n", "condok\n");
}
