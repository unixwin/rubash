//! rubash#2270 — an external command's NUMBERED-fd heredoc
//! (`./reader.sh 3<<EOF`) never installed the body: the child's fd 3
//! was unbound (reads failed EBADF), while the same body on fd 0 and
//! a plain `3<file` both worked.
//!
//! GNU installs a heredoc body independently of the redirector
//! number: do_redirection_internal's r_reading_until /
//! r_deblank_reading_until / r_reading_string arm calls
//! here_document_to_fd (redir.c:423), which places the body in a
//! temporary file, and dup2's the resulting descriptor onto the
//! redirector (redir.c:1065-1072) — fd 0, 3, or {var} alike. Rubash's
//! spawn loop classified the entry as a WRITE file redirect and
//! "opened" the delimiter word as an output file, so nothing ever
//! reached the child. The fix materializes the canonical body text
//! (external_fd_heredoc_input) into a carrier file inside that same
//! loop and delivers it through the ordinary input arm — the same
//! env / pin channels as a file input redirect, one installation
//! contract for every target fd.
//!
//! Expectations byte-verified against GNU bash 5.3.20 via
//! probes/probe-2270.sh (oracle run in the cluster workdir).
//!
//! OBSERVER NOTE: the observing child is a real spawned process —
//! the rubash binary itself re-invoked — never a `./reader.sh`
//! script: rubash runs ENOEXEC scripts in-process, where
//! spawn-delivered fd bindings do not appear. The original pins
//! observed through `./reader3.sh` / `./reader4.sh` / `./reader0.sh`
//! and read 2 passed / 8 failed on a tree whose product face was
//! verified correct with foreign ELF observers (the script child
//! reported `3: Bad file descriptor` and printed `fd3-closed` even
//! for the plain `3<file` control). The harness repair changed the
//! observer mechanism only; every redirect shape, expected body,
//! and exit-code expectation is unchanged.
//!
//! NOTE: the `<<-` pin additionally depends on the lexer strip-tabs
//! fix for numbered fds (PR #2269, rubash#2265); on a tree without
//! it the tabbed terminator never matches and that one pin stays
//! red for the lexer's reason, not this fix's.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2270-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The observing child for a numbered fd: a nested rubash (a real
/// spawn) whose read loop echoes each line the delivered descriptor
/// carries, then reports whether the descriptor is still open at
/// EOF, then exits with `exit_code` when one is given — the exact
/// observations the old `./readerN.sh` scripts made, through a
/// process boundary the spawn delivery actually crosses.
fn reader_child(fd: u32, exit_code: Option<i32>) -> String {
    let mut inner = format!(
        "while IFS= read -r line <&{fd}; do printf \"fd{fd}|%s\\n\" \"$line\"; done; \
         if : <&{fd}; then echo fd{fd}-open; else echo fd{fd}-closed; fi"
    );
    if let Some(code) = exit_code {
        inner.push_str(&format!("; exit {code}"));
    }
    format!("\"{RUBASH}\" -c '{inner}'")
}

/// The observing child for the fd 0 control: a nested rubash whose
/// read loop echoes each stdin line, then prints the done marker.
fn reader0_child() -> String {
    let inner = "while IFS= read -r line; do printf \"fd0|%s\\n\" \"$line\"; done; echo fd0-done";
    format!("\"{RUBASH}\" -c '{inner}'")
}

#[test]
fn issue2270_fd3_heredoc_body_reaches_external_child() {
    let dir = scratch("fd3");
    let script = format!(
        "who=world\n{} 3<<EOF\nhello $who\nsecond line\nEOF\n",
        reader_child(3, Some(7))
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(
        stdout, "fd3|hello world\nfd3|second line\nfd3-open\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_dash_heredoc_strips_tabs() {
    let dir = scratch("fd3dash");
    let script = format!(
        "{} 3<<-EOF\n\ttabbed one\n\t\ttabbed two\n\tEOF\n",
        reader_child(3, Some(7))
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(
        stdout, "fd3|tabbed one\nfd3|tabbed two\nfd3-open\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd4_heredoc_body_reaches_external_child() {
    let dir = scratch("fd4");
    let script = format!("{} 4<<EOF\nfour body\nEOF\n", reader_child(4, None));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd4|four body\nfd4-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2270_child_exit_code_after_reading_body_to_eof() {
    // The reader consumes the body to EOF and exits 7: the command's
    // status must be the child's own 7 (not a redirect-error 1/2),
    // and the fd must still be open (at EOF) after the last read.
    let dir = scratch("exitcode");
    let script = format!("{} 3<<EOF\nonly line\nEOF\n", reader_child(3, Some(7)));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3|only line\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_here_string_body_reaches_external_child() {
    let dir = scratch("herestring");
    let script = format!("{} 3<<<\"string body\"\n", reader_child(3, Some(7)));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3|string body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_last_heredoc_wins() {
    // GNU do_redirections walks the list in order (redir.c:237);
    // the second heredoc's binding is the one the child reads.
    let dir = scratch("lastwins");
    let script = format!(
        "{} 3<<ONE 3<<TWO\nfirst body\nONE\nsecond body\nTWO\n",
        reader_child(3, Some(7))
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3|second body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_quoted_delimiter_body_is_literal() {
    let dir = scratch("quoted");
    let script = format!(
        "who=world\n{} 3<<'EOF'\nhello $who\nEOF\n",
        reader_child(3, Some(7))
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3|hello $who\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_no_delimiter_file_created() {
    // The pre-fix shape "opened" the delimiter as an output file in
    // the cwd; GNU creates nothing.
    let dir = scratch("nostray");
    let script = format!("{} 3<<EOF\nbody\nEOF\n", reader_child(3, Some(7)));
    let (_stdout, _stderr, _code) = rubash_in(&dir, &script);
    assert!(
        !dir.join("EOF").exists(),
        "delimiter word must not become a file in the cwd"
    );
}

#[test]
fn issue2270_fd0_heredoc_control_still_works() {
    let dir = scratch("fd0");
    let script = format!("{} <<EOF\nstdin body\nEOF\n", reader0_child());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd0|stdin body\nfd0-done\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_plain_file_redirect_control() {
    let dir = scratch("file");
    fs::write(dir.join("input.txt"), "file body\n").expect("write input");
    let script = format!("{} 3<input.txt\n", reader_child(3, Some(7)));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3|file body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}
