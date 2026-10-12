//! Issue rubash#543 — unterminated `$[` legacy-arithmetic parse guards.
//!
//! GNU read_token_word's shellexp arm hands a `$` whose peek is `[` to
//! `parse_matched_pair (cd, '[', ']', P_ARITH)` (parse.y:5494-5527), a
//! grouping scan that reads across lines until the matching `]`; EOF reports
//! `unexpected EOF while looking for matching `X'' at the innermost failed
//! scan's opener (parse.y:3901-3912, start_lineno) and exits 2. Rubash
//! folded the unterminated span into the token silently: `echo $[1+2`
//! echoed the literal text with rc=0 where GNU dies with rc=2.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.2.21 and 5.3.0
//! (script-file probes, 2026-10-12; both versions identical on every shape):
//!
//! - unquoted hunt: `]'` at the `$[` line; complete commands before that
//!   line still run (probe A);
//! - a quote/backtick recursion inside the hunt names ITSELF at its own
//!   line (a5/q11/[6]/[14] — parse.y:4040-4051 shellquote arm);
//! - a hunt bounded by a double-quoted span names the DQUOTE at the span's
//!   line (q2/w1-w4: `echo "$[1+2"` -> `"');
//! - the hunt owns the EOF inside `${` (`echo ${x[$[1}` -> `]', not the `}'
//!   close-char verdict) and inside `$(` (`echo $(echo $[1+2` -> `]');
//! - `$((`/`((` unclosed arithmetic keeps its `)' family verdict (a1/a3);
//! - heredoc bodies stay raw text (make_cmd.c:512) and quoted/escaped `$`
//!   stays literal (rc 0);
//! - balanced `$[...]` keeps evaluating (cell 1's control), and the
//!   HEREDOC_MAX=16 pending-count guard (shell.h:172, parse.y:3108-3116
//!   push_heredoc -> `maximum here-document count exceeded`, the issue's
//!   cell 2) stays at rc=2 — pinned here because the audit's baseline
//!   predated it.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a SCRIPT FILE (the byte-exact baseline shape) and return
/// (stdout+stderr text, exit code).
fn rubash_file(script: &str) -> (String, Option<i32>) {
    let path = std::env::temp_dir().join(format!(
        "rubash_i543_{}_{}.sh",
        std::process::id(),
        script.len()
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (text, output.status.code())
}

/// Feed a script through `rubash -s` stdin (the audit's stdin-shape oracle).
fn rubash_stdin(script: &str) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (text, output.status.code())
}

/// The audited cell: an unterminated `$[` is a parse error, rc 2 — GNU:
/// `line 1: unexpected EOF while looking for matching `]'`.
#[test]
fn unterminated_dollar_bracket_is_parse_error() {
    let (text, code) = rubash_file("echo $[1+2\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
    // The failed line never runs: the literal echo is gone.
    assert!(!text.contains("$[1+2"), "text={text:?}");
}

/// No trailing newline: the EOF still closes the parse with the same
/// verdict (GNU: identical for `printf '%s' '$[1+2'`).
#[test]
fn unterminated_dollar_bracket_without_newline_still_errors() {
    let (text, code) = rubash_file("echo $[1+2");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
}

/// GNU's incremental reader executes the COMPLETE commands before the
/// construct's line, then dies at the `$[` line (probe A).
#[test]
fn complete_lines_before_construct_still_run() {
    let (text, code) = rubash_file("echo one\necho $[1+2\necho three\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(text.contains("one\n"), "text={text:?}");
    assert!(
        text.contains("line 2: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
    assert!(!text.contains("three"), "text={text:?}");
}

/// The stdin (-s) shape of the audited cell: the reader keeps pulling lines
/// while the hunt is open, complete lines run, the verdict lands at the
/// `$[` line with rc 2.
#[test]
fn stdin_stream_reports_at_construct_line() {
    let (text, code) = rubash_stdin("echo before\necho $[1+2\necho after\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(text.contains("before\n"), "text={text:?}");
    assert!(
        text.contains("line 2: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
    assert!(!text.contains("after"), "text={text:?}");
}

/// A quote recursion inside the hunt reports the QUOTE at its own line
/// (a5: `echo $[\n'q` -> `'` line 2, nothing else).
#[test]
fn quote_inside_hunt_reports_itself_at_its_line() {
    let (text, code) = rubash_file("echo $[\n'q\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 2: unexpected EOF while looking for matching `''"),
        "text={text:?}"
    );
    assert!(
        !text.contains("unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
}

/// A backtick recursion inside the hunt names the backtick ([14]).
#[test]
fn backtick_inside_hunt_reports_itself() {
    let (text, code) = rubash_file("echo $[ `echo 1\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching ``'"),
        "text={text:?}"
    );
}

/// Every `[` counts in the P_ARITH scan (parse.y:3979), so `echo $[a[1`
/// still names the hunt's `]'` (probe [18]) while the balanced `[19]`
/// closes and evaluates.
#[test]
fn nested_brackets_count_in_hunt() {
    let (text, code) = rubash_file("echo $[a[1\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
    let (text, code) = rubash_file("echo $[a[1]]\n");
    assert_eq!(code, Some(0), "text={text:?}");
}

/// A hunt opened inside an unclosed `${` owns the EOF: `echo ${x[$[1}`
/// names `]'` where the close-char family would say `}' (probe H).
#[test]
fn hunt_inside_dolbrace_owns_eof() {
    let (text, code) = rubash_file("echo ${x[$[1}\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
    assert!(!text.contains("bad substitution"), "text={text:?}");
}

/// A hunt opened inside an unclosed `$( ` names `]'` at the `$[` line (t4:
/// `echo $(echo $[1+2` — GNU line 1, not the comsub's `)' at EOF).
#[test]
fn hunt_inside_comsub_owns_eof() {
    let (text, code) = rubash_file("echo $(echo $[1+2\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `]'"),
        "text={text:?}"
    );
}

/// A hunt bounded by a double-quoted span names the DQUOTE at the span's
/// line (q2/w1: `echo "$[1+2"` and `echo "$[1+2" ]` both say `"').
#[test]
fn hunt_inside_dquote_names_the_dquote() {
    for script in [
        "echo \"$[1+2\"\n",
        "echo \"$[1+2\" ]\n",
        "echo \"$[x[0]\"\n",
    ] {
        let (text, code) = rubash_file(script);
        assert_eq!(code, Some(2), "script={script:?} text={text:?}");
        assert!(
            text.contains("line 1: unexpected EOF while looking for matching `\"'"),
            "script={script:?} text={text:?}"
        );
    }
}

/// Unclosed `$((`/`((` arithmetic keeps its own family's `)' verdict
/// (a1/a3) — the `$[` inside is arithmetic text to this guard.
#[test]
fn dparen_arith_unclosed_keeps_paren_verdict() {
    let (text, code) = rubash_file("echo $(( $[1\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `)'"),
        "text={text:?}"
    );
    let (text, code) = rubash_file("(( x = $[1\n");
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: unexpected EOF while looking for matching `)'"),
        "text={text:?}"
    );
}

/// Control: the balanced form still evaluates (the audit's termination
/// control), single-quoted and escaped `$` stay literal, and a bare-word
/// `[` subscript stays literal data (rc 0).
#[test]
fn balanced_and_literal_shapes_unchanged() {
    let (text, code) = rubash_file("echo $[1+2]\n");
    assert_eq!((code.as_ref(), text.trim()), (Some(&0), "3"));
    let (text, code) = rubash_file("x=3; echo $[x]\n");
    assert_eq!((code.as_ref(), text.trim()), (Some(&0), "3"));
    let (text, code) = rubash_file("echo '$[1+2'\n");
    assert_eq!((code.as_ref(), text.trim()), (Some(&0), "$[1+2"));
    let (text, code) = rubash_file("echo \\$[1+2\n");
    assert_eq!((code.as_ref(), text.trim()), (Some(&0), "$[1+2"));
    let (text, code) = rubash_file("echo a[b\n");
    assert_eq!((code.as_ref(), text.trim()), (Some(&0), "a[b"));
}

/// Heredoc bodies are raw text (make_cmd.c:512): a `$[` inside never opens
/// a hunt (probe a8 — GNU runs the script through with rc 0).
#[test]
fn heredoc_body_stays_raw_text() {
    let (text, code) = rubash_file("cat <<E\n$[1+2\nE\necho ok\n");
    assert_eq!(code, Some(0), "text={text:?}");
    assert!(text.contains("ok"), "text={text:?}");
    assert!(!text.contains("unexpected EOF"), "text={text:?}");
}

/// Cell 2 of the issue: the HEREDOC_MAX=16 pending-heredoc guard
/// (shell.h:172; parse.y:3108-3116 push_heredoc -> report_syntax_error +
/// exit 2). Seventeen `<<` redirections on one command are fatal; sixteen
/// parse. The audit's baseline predated this guard; pinned so the limit
/// stays at GNU's boundary.
#[test]
fn heredoc_pending_count_limit_stays_at_seventeen() {
    let over = format!("cat{}\n", " <<E".repeat(17));
    let (text, code) = rubash_file(&over);
    assert_eq!(code, Some(2), "text={text:?}");
    assert!(
        text.contains("line 1: maximum here-document count exceeded"),
        "text={text:?}"
    );
    let at = format!("cat{}\n", " <<E".repeat(16));
    let (text, code) = rubash_file(&at);
    assert_eq!(code, Some(0), "text={text:?}");
    assert!(
        !text.contains("maximum here-document count exceeded"),
        "text={text:?}"
    );
}
