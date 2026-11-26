//! Issue unixwin/rubash#4132: an `exit` later in a sourced file was
//! routed as a top-level exit (killing the parent script) when an
//! earlier line of the file executed a backquote substitution whose
//! body carries a quoted `<<`:
//!
//! ```sh
//! # g.sh:  r=`echo "<<x"`\nexit 0
//! ( . ./g.sh )
//! echo AFTER          # GNU prints AFTER; rubash printed nothing.
//! ```
//!
//! GNU contract: the backquote body is re-lexed as its own input
//! (parse.y:4451 parse_comsub), where a `<<` inside the body's quotes
//! is word data (parse.y:5305 read_token_word) and declares no
//! heredoc; the sourced file's lines are complete commands one by one
//! (builtins/evalstring.c:348 parse_and_execute), so the subshell's
//! `exit` ends only the subshell.
//!
//! Root cause: the group-completeness scans judged the line
//! incomplete — the close-char scans' heredoc arm fired on the quoted
//! `<<` inside the backquote body (a backquote frame is operator-live,
//! but the scans tracked no quote state inside the body, so
//! skip_heredoc_top_level consumed the closing quote and backquote as
//! delimiter/body text and the backquote never closed), and the
//! comsub residual machine's backquote-heredoc arm did the same while
//! its double-quote state was open. The gather glued the whole file
//! into one incomplete group, which runs through the whole-text
//! source path whose exit handling is the top-level one.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script files,
//! cloud oracle): every case below produced identical stdout, stderr
//! and rc.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4132-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_main(
    dir: &PathBuf,
    g_name: &str,
    g_text: &str,
    main_text: &str,
) -> (String, String, Option<i32>) {
    std::fs::write(dir.join(g_name), g_text).expect("write sourced file");
    let main = dir.join("main.sh");
    std::fs::write(&main, main_text).expect("write main script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&main)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue4132_subshell_source_backquote_dquote_ltlt_then_parent_continues() {
    let dir = tempdir("dq");
    let (stdout, stderr, code) = run_main(
        &dir,
        "g.sh",
        "r=`echo \"<<x\"`\nexit 0\n",
        "( . ./g.sh )\necho AFTER\n",
    );
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4132_subshell_source_backquote_squote_ltlt_then_parent_continues() {
    let dir = tempdir("sq");
    let (stdout, stderr, code) = run_main(
        &dir,
        "g.sh",
        "r=`printf '%s' '<<x'`\nexit 0\n",
        "( . ./g.sh )\necho AFTER\n",
    );
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4132_subshell_source_bare_backquote_ltlt_then_parent_continues() {
    // The substitution's output becomes the command name (and fails,
    // exactly as GNU), then the subshell's exit must still be local.
    let dir = tempdir("bare");
    let (stdout, stderr, code) = run_main(
        &dir,
        "g.sh",
        "`echo \"<<x\"`\nexit 0\n",
        "( . ./g.sh )\necho AFTER\n",
    );
    assert_eq!(stdout, "AFTER\n");
    assert!(
        stderr.contains("<<x: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4132_subshell_source_backquote_ltlt_middle_line_still_runs() {
    // A line between the trigger and the exit executes as its own
    // command (its diagnostic names its own line), and the exit is
    // still the subshell's.
    let dir = tempdir("mid");
    let (stdout, stderr, code) = run_main(
        &dir,
        "g.sh",
        "r=`echo \"<<x\"`\nx\nexit 0\n",
        "( . ./g.sh )\necho AFTER\n",
    );
    assert_eq!(stdout, "AFTER\n");
    assert!(
        stderr.contains("line 2: x: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue4132_subshell_source_real_heredoc_in_backquote_body_still_gathers() {
    // Guard: an UNQUOTED `<<` inside a backquote body is a real
    // heredoc of the inner command — its body lines still gather and
    // the value lands.
    let dir = tempdir("realhd");
    let (stdout, stderr, code) = run_main(
        &dir,
        "g.sh",
        "r=`cat <<EOF\nhello\nEOF\n`\necho \"r=$r\"\nexit 0\n",
        "( . ./g.sh )\necho AFTER\n",
    );
    assert_eq!(stdout, "r=hello\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4132_source_without_subshell_exit_still_kills_script() {
    // Guard (control): sourced at top level, the same file's exit is
    // the script's exit on both engines — nothing prints.
    let dir = tempdir("noparen");
    let (stdout, _stderr, code) = run_main(
        &dir,
        "g.sh",
        "r=`echo \"<<x\"`\nexit 0\n",
        ". ./g.sh\necho AFTER\n",
    );
    assert_eq!(stdout, "");
    assert_eq!(code, Some(0));
}
