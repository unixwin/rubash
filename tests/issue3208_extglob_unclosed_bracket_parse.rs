//! rubash#3208 — unquoted unclosed `[` inside an extglob group must not
//! be reported as a parse-time `]` EOF error.
//!
//! Root cause: the unclosed-subscript oracle
//! (`skip::unclosed_array_subscript_walk`, src/lexer/skip.rs) models a
//! `(` that follows a word character as a subshell/grouping boundary
//! (flipping `command_position` to true), so the `a` of `@(a[b)` becomes
//! a command-position identifier and the `[` opens a P_ARRAYSUB hunt
//! that runs to EOF. GNU never treats that `(` as a boundary: with
//! extglob live it is consumed as part of the group (parse.y:5466-5489
//! read_token_word → parse_matched_pair), and with extglob off it is a
//! syntax-error token (`syntax error near unexpected token '('`).
//! The oracle's text-derived extglob mirror cannot see a `shopt -s
//! extglob` that already executed in an earlier group (scripts run
//! group-by-group), so the misreport fires exactly in the live-extglob
//! case-pattern shape too (#3189 residual E6).
//!
//! GNU anchors: parse.y:5466-5489 (extglob arm), parse.y:3877
//! parse_matched_pair (`[` is ordinary inside a group), lib/glob/sm_loop.c
//! PATSCAN/BRACKMATCH (unclosed `[` is literal at match time, never a
//! parse error), parse.y:3901-3912 (unclosed group reports `)`).
//!
//! Harness: script files (extglob must be enabled at parse time). Same
//! `rubash_file` shape as issue3189. Expectations verified byte-for-byte
//! against GNU bash 5.3.20 script-file probes (~/workspace/fix-3208/probes/).
//! Cells are marked RED (fail on the 6ace5616 base, must turn green with
//! the fix) or LOCK (already correct on that base; guards against
//! overcorrection). Error cells assert rc + a stderr substring (rubash's
//! diagnostic prefix differs from GNU's filename prefix).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3208-{}-{}",
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
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

fn check_err(script: &str, expected_rc: i32, stderr_needle: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_rc),
        "script rc mismatch: {script}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(stderr_needle),
        "script stderr missing {stderr_needle:?}: {script}\nstderr: {stderr}"
    );
    assert_eq!(
        stdout, "",
        "error script wrote to stdout: {script}\nstdout: {stdout}"
    );
}

#[test]
fn extglob_unclosed_bracket_case_pattern() {
    // RED — the #3189 E6 original: extglob group with an unquoted
    // unterminated `[` as a case pattern. GNU: NOMATCH, script survives.
    check(
        "shopt -s extglob\n\
         case 'a[b' in @(a[b)) echo E6_MATCH;; *) echo E6_NOMATCH;; esac\n\
         echo E6_SURVIVED\n",
        "E6_NOMATCH\nE6_SURVIVED\n",
    );
}

#[test]
fn extglob_unclosed_bracket_five_operators() {
    // RED — all five group operators in command position, extglob live:
    // the group body is consumed whole, the unclosed `[` is literal at
    // match time, no file matches, the word echoes verbatim.
    check(
        "shopt -s extglob\n\
         echo @(a[b)\n\
         echo *(a[b)\n\
         echo +(a[b)\n\
         echo ?(a[b)\n\
         echo !(a[b)\n\
         echo alive\n",
        "@(a[b)\n*(a[b)\n+(a[b)\n?(a[b)\n!(a[b)\nalive\n",
    );
}

#[test]
fn extglob_unclosed_bracket_assignment() {
    // RED — assignment position: the group is word text, stored verbatim.
    check(
        "shopt -s extglob\n\
         x=@(a[b)\n\
         echo \"x=$x\"\n\
         echo alive\n",
        "x=@(a[b)\nalive\n",
    );
}

#[test]
fn extglob_unclosed_bracket_conditional() {
    // RED — [[ == ]] RHS: a group with an unclosed `[` never matches
    // (PATSCAN group scan fails; contrast the bare `a[b` literal
    // fallback, which DOES match — see the lock below).
    check(
        "shopt -s extglob\n\
         [[ 'a[b' == @(a[b) ]] && echo H3_MATCH || echo H3_NOMATCH\n\
         [[ 'ab' == @([a]b) ]] && echo H5_MATCH || echo H5_NOMATCH\n\
         echo alive\n",
        "H3_NOMATCH\nH5_MATCH\nalive\n",
    );
    // LOCK — bare unclosed `[` in a [[ == ]] pattern is a literal.
    check(
        "shopt -s extglob\n\
         [[ 'a[b' == a[b ]] && echo H2_MATCH || echo H2_NOMATCH\n\
         echo alive\n",
        "H2_MATCH\nalive\n",
    );
}

#[test]
fn extglob_unclosed_bracket_unclosed_group_is_paren_eof() {
    // RED — an unclosed GROUP (not bracket) reports `)`, never `]`.
    check_err(
        "shopt -s extglob\n\
         echo @(a[b\n\
         echo alive\n",
        2,
        "unexpected EOF while looking for matching `)'",
    );
}

#[test]
fn extglob_off_paren_is_syntax_error() {
    // RED — with extglob off, `(` after a word character is a syntax
    // error token, not a `]` EOF: four shapes, GNU rc=2 each.
    for script in [
        "echo @(a[b)\necho alive\n",
        "echo *(a[b)\necho alive\n",
        "echo foo(bar)\necho alive\n",
        "echo @(a[b\necho alive\n",
    ] {
        check_err(script, 2, "syntax error near unexpected token `('");
        let (_, stderr, _) = rubash_file(script);
        assert!(
            !stderr.contains("matching `]'"),
            "extglob-off shape misreported as `]` EOF: {script}\nstderr: {stderr}"
        );
    }
}

#[test]
fn extglob_unclosed_bracket_locks() {
    // LOCK — a genuinely unclosed subscript still reports `]`: bare
    // command-position `a[1`, negation+subshell `!(a[1)`, and the
    // pipe-position hunt from the oracle's own unit test.
    check_err(
        "a[1\necho alive\n",
        2,
        "unexpected EOF while looking for matching `]'",
    );
    check_err(
        "!(a[1)\necho alive\n",
        2,
        "unexpected EOF while looking for matching `]'",
    );
    check_err(
        "a | b[c\necho alive\n",
        2,
        "unexpected EOF while looking for matching `]'",
    );
    // LOCK — an unclosed `[` in a case pattern / [[ == ]] RHS is literal
    // NOMATCH, never an error (GNU g7).
    check(
        "shopt -s extglob\n\
         case ab in [a) echo G7_MATCH;; *) echo G7_NOMATCH;; esac\n\
         [[ ab == [a ]] && echo G7B_MATCH || echo G7B_NOMATCH\n\
         echo alive\n",
        "G7_NOMATCH\nG7B_NOMATCH\nalive\n",
    );
}
