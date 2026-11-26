//! Issue unixwin/rubash#1290 (residual): the two-line diagnostic for a
//! `[[ ... =~ (` regex whose parse_matched_pair group never closes.
//!
//! GNU owner: parse.y:5443-5461 read_token_word under PST_REGEXP — an
//! unquoted `(' in the regex word opens parse_matched_pair, which scans
//! the RAW input stream to EOF (parse.y:3901-3912 reports
//! `unexpected EOF while looking for matching `)'' at the `(' line and
//! sets PST_NOERROR); the cond parser then reports
//! `unexpected argument to conditional binary operator` at the stream's
//! EOF line (parse.y:5223-5227). Quotes inside the scan are nested pairs
//! and never count (parse.y:4040-4051); a quoted `(' never opens a pair
//! at all (shellquote runs first, parse.y:5417).
//!
//! The residual had three faces, each pinned below:
//!  1. File mode merged the swallowed following lines into the RHS, so
//!     raw != value defeated the unclosed-paren check and the generic
//!     `` `]]' `` EOF shape printed instead of the pair of lines.
//!  2. The EOF line ignored the parse input's line offset and the
//!     unterminated-final-line rule (GNU reports one past the last line
//!     either way: `-c '[[ abc =~ ([a-z]]]'` reports line 2).
//!  3. The stdin reader submitted the `[[` line as a complete group, so
//!     the scan never saw the rest of the stream GNU consumes.
//!
//! Verified against GNU Bash 5.3.20 (script-file and stdin probes):
//! v1290c reports lines 3 + 7 in both modes, p1290b lines 3 + 5.

use std::io::Write;
use std::process::{Command, Stdio};

const V1290C: &str = "x=1\n\n[[ abc =~ ([a-z]]]\necho \"after $x\"\ny=\"span1\nspan2\"\n";
const P1290B: &str = "x=1\n\n[[ abc =~ ([a-z] ]]\necho \"after rc=$?\"\n";

fn run_rubash(args: &[&str], stdin_text: Option<&str>) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_text.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(text) = stdin_text {
        child
            .stdin
            .as_mut()
            .expect("stdin pipe")
            .write_all(text.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1290_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let result = run_rubash(&[path.to_str().expect("utf8 path")], None);
    let _ = std::fs::remove_file(&path);
    result
}

/// Diagnostic lines with the program-name prefix stripped: every line
/// reduces to its `line N: ...` body, the part GNU pins.
fn diag_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find("line ") {
            Some(at) => line[at..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

fn expect_pair(stderr: &str, paren_line: usize, eof_line: usize) {
    assert_eq!(
        diag_lines(stderr),
        vec![
            format!("line {paren_line}: unexpected EOF while looking for matching `)'"),
            format!("line {eof_line}: unexpected argument to conditional binary operator"),
        ],
        "stderr was: {stderr}"
    );
}

#[test]
fn file_v1290c_reports_paren_line_and_stream_eof_line() {
    let (stdout, stderr, code) = run_rubash_file(V1290C, "v1290c");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 3, 7);
}

#[test]
fn file_p1290b_reports_paren_line_and_stream_eof_line() {
    let (stdout, stderr, code) = run_rubash_file(P1290B, "p1290b");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 3, 5);
}

#[test]
fn stdin_v1290c_consumes_stream_like_gnu() {
    let (stdout, stderr, code) = run_rubash(&[], Some(V1290C));
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 3, 7);
}

#[test]
fn stdin_p1290b_consumes_stream_like_gnu() {
    let (stdout, stderr, code) = run_rubash(&[], Some(P1290B));
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 3, 5);
}

#[test]
fn dash_c_unclosed_regex_reports_line_two() {
    for script in ["[[ abc =~ ([a-z]]]", "[[ a =~ (b ]]", "[[ a =~ (b"] {
        let (stdout, stderr, code) = run_rubash(&["-c", script], None);
        assert_eq!(code, Some(2), "script: {script}");
        assert_eq!(stdout, "");
        expect_pair(&stderr, 1, 2);
    }
}

#[test]
fn unterminated_final_line_still_reports_line_two() {
    let (stdout, stderr, code) = run_rubash_file("[[ abc =~ ([a-z]]]", "nonl");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 1, 2);
}

#[test]
fn quoted_paren_never_opens_a_pair() {
    // The `(' is quoted in both: no scan opens, the word is a (quoted,
    // hence literal) pattern that simply does not match.
    for script in [
        "[[ a =~ \"x(unclosed\" ]]; echo rc=$?",
        "[[ a =~ \"(x)\" ]]; echo rc=$?",
    ] {
        let (stdout, stderr, code) = run_rubash(&["-c", script], None);
        assert_eq!(code, Some(0), "script: {script}");
        assert_eq!(stdout, "rc=1\n");
        assert_eq!(stderr, "");
    }
}

#[test]
fn closed_group_still_evaluates() {
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ abc =~ ([a-z]) ]]; echo rc=$?"], None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn bracket_close_inside_open_scan_is_data() {
    // The first `]]' sits inside the still-open pair scan, so the
    // conditional is well-formed and evaluates (no match -> rc 1).
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (x ]] y) ]]; echo rc=$?"], None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn quoted_close_paren_does_not_end_the_scan() {
    // GNU consumes the `echo ")"` line as scan data (the quoted `)'
    // cannot close the pair), so the scan dies at the stream EOF.
    let script = "[[ a =~ (x\necho \")\"\necho never\n";
    let (stdout, stderr, code) = run_rubash(&[], Some(script));
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    expect_pair(&stderr, 1, 4);
}

#[test]
fn multiline_regex_group_closes_on_a_later_line() {
    let script = "[[ a =~ (x\ny) ]]\necho done\n";
    let (stdout, stderr, code) = run_rubash(&[], Some(script));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
}
