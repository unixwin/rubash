//! Issue unixwin/rubash#3003: a script routed to the grouped driver
//! (any earlier line carrying a `. ` token — the foreign-text
//! heuristic in `script_driver::script_runs_foreign_text`, e.g. a
//! comment `# a. b` or a plain `echo a. b`) broke on a here-doc whose
//! command line ends with a binary connector:
//!
//! ```sh
//! echo a. b
//! cat <<EOF &&
//! y
//! EOF
//! echo done
//! ```
//!
//! GNU: `a. b` / `y` / `done`, rc=0. rubash: `a. b`, then
//! `line 3: syntax error: unexpected end of file`, rc=2 — the group
//! gatherer submitted `cat <<EOF &&\ny\nEOF\n` as a complete command.
//!
//! GNU contract: the here-doc body is never a grammar token. When
//! the reader reaches the newline it drains every queued body
//! straight into its REDIRECTION (parse.y:3120
//! `gather_here_documents`, called from the grammar actions at
//! parse.y:1265-1266/1343-1344/1361-1362/1376-1377 over the
//! `redir_stack` queued at parse.y:3116). The grammar's open
//! production is still `list1: list1 AND_AND newline_list list1`
//! (parse.y:1286; OR_OR parse.y:1288, `|` parse.y:1471, `|&`
//! parse.y:1473-1474), so the command counts as read only once the
//! connector's right-hand side has been read — the line after the
//! delimiter line.
//!
//! Root cause: both implementations of the completeness verdict's
//! token-level half let the HereDocBody token overwrite the
//! "last significant token" summary (GroupScanFeeder::fold_token;
//! the tail scan in token_walk_verdict), so once the body drained,
//! the header line's trailing `&&`/`||`/`|` was masked and the
//! verdict reported Complete — contradicting the walk's own
//! doctrine that body tokens are data and fold nothing. The fix
//! keeps body tokens out of the last-significant summary in both
//! implementations.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script
//! files). Eco-chain evidence: git t1300-config aborted at test
//! 198/219 on this shape (eco-2904 REPORT §4 D1).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3003-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue3003_dot_space_line_then_heredoc_and_and() {
    let (stdout, stderr, code) =
        run_script("minimal", "echo a. b\ncat <<EOF &&\ny\nEOF\necho done\n");
    assert_eq!(stdout, "a. b\ny\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_parse_only_form() {
    let dir = tempdir("parseonly");
    let file = dir.join("case.sh");
    std::fs::write(&file, "echo a. b\ncat <<EOF &&\ny\nEOF\necho done\n").expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg(&file)
        .output()
        .expect("run rubash -n");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue3003_comment_line_trigger() {
    // The t1300 shape: the `. ` token sits in a comment, far above
    // the here-doc block — the state that leaks is the driver
    // routing, so distance must not matter.
    let (stdout, stderr, code) = run_script(
        "comment",
        "# GIT_CONFIG_NOSYSTEM. We set it ourselves\necho one\necho two\necho three\ncat <<EOF &&\ny\nEOF\necho done\n",
    );
    assert_eq!(stdout, "one\ntwo\nthree\ny\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_heredoc_or_or() {
    let (stdout, stderr, code) =
        run_script("oror", "echo a. b\ncat <<EOF ||\ny\nEOF\necho fallback\n");
    // cat succeeds, so the right-hand side never runs.
    assert_eq!(stdout, "a. b\ny\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_heredoc_pipe() {
    let (stdout, stderr, code) = run_script("pipe", "echo a. b\ncat <<EOF |\ny\nEOF\ntr a-z A-Z\n");
    assert_eq!(stdout, "a. b\nY\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_two_heredocs_one_connector_line() {
    let (stdout, stderr, code) = run_script(
        "two",
        "echo a. b\ncat <<A && cat <<B\nx\nA\ny\nB\necho done\n",
    );
    assert_eq!(stdout, "a. b\nx\ny\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_heredoc_semicolon_control() {
    // Control: `;` terminates the command before the body, so this
    // shape was complete at the delimiter line even before the fix.
    let (stdout, stderr, code) =
        run_script("semicolon", "echo a. b\ncat <<EOF;\ny\nEOF\necho done\n");
    assert_eq!(stdout, "a. b\ny\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3003_unterminated_heredoc_still_errors() {
    // Control: at real EOF an unterminated body after a dangling
    // connector is still a syntax error, not a silent pass — the
    // needs-more verdict must not over-submit either way.
    let (stdout, stderr, code) = run_script("unterminated", "echo a. b\ncat <<EOF &&\ny\n");
    assert_eq!(stdout, "a. b\n");
    assert!(
        stderr.contains("syntax error"),
        "expected a syntax error diagnostic: {stderr:?}"
    );
    assert_eq!(code, Some(2));
}
