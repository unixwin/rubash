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
//! NOTE: the `<<-` pin additionally depends on the lexer strip-tabs
//! fix for numbered fds (PR #2269, rubash#2265); on a tree without
//! it the tabbed terminator never matches and that one pin stays
//! red for the lexer's reason, not this fix's.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const READER3: &str = "while IFS= read -r line <&3; do\n  printf 'fd3|%s\\n' \"$line\"\ndone\nif : <&3; then echo 'fd3-open'; else echo 'fd3-closed'; fi\nexit 7\n";
const READER4: &str = "while IFS= read -r line <&4; do\n  printf 'fd4|%s\\n' \"$line\"\ndone\nif : <&4; then echo 'fd4-open'; else echo 'fd4-closed'; fi\n";
const READER0: &str =
    "while IFS= read -r line; do\n  printf 'fd0|%s\\n' \"$line\"\ndone\necho 'fd0-done'\n";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2270-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    for (name, body) in [
        ("reader3.sh", READER3),
        ("reader4.sh", READER4),
        ("reader0.sh", READER0),
    ] {
        let path = dir.join(name);
        fs::write(&path, body).expect("write reader script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("chmod reader script");
        }
    }
    dir
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
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

#[test]
fn issue2270_fd3_heredoc_body_reaches_external_child() {
    let dir = scratch("fd3");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "who=world\n./reader3.sh 3<<EOF\nhello $who\nsecond line\nEOF\n",
    );
    assert_eq!(
        stdout, "fd3|hello world\nfd3|second line\nfd3-open\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_dash_heredoc_strips_tabs() {
    let dir = scratch("fd3dash");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "./reader3.sh 3<<-EOF\n\ttabbed one\n\t\ttabbed two\n\tEOF\n",
    );
    assert_eq!(
        stdout, "fd3|tabbed one\nfd3|tabbed two\nfd3-open\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd4_heredoc_body_reaches_external_child() {
    let dir = scratch("fd4");
    let (stdout, stderr, code) = rubash_in(&dir, "./reader4.sh 4<<EOF\nfour body\nEOF\n");
    assert_eq!(stdout, "fd4|four body\nfd4-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2270_child_exit_code_after_reading_body_to_eof() {
    // The reader consumes the body to EOF and exits 7: the command's
    // status must be the child's own 7 (not a redirect-error 1/2),
    // and the fd must still be open (at EOF) after the last read.
    let dir = scratch("exitcode");
    let (stdout, stderr, code) = rubash_in(&dir, "./reader3.sh 3<<EOF\nonly line\nEOF\n");
    assert_eq!(stdout, "fd3|only line\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_here_string_body_reaches_external_child() {
    let dir = scratch("herestring");
    let (stdout, stderr, code) = rubash_in(&dir, "./reader3.sh 3<<<\"string body\"\n");
    assert_eq!(stdout, "fd3|string body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_last_heredoc_wins() {
    // GNU do_redirections walks the list in order (redir.c:237);
    // the second heredoc's binding is the one the child reads.
    let dir = scratch("lastwins");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "./reader3.sh 3<<ONE 3<<TWO\nfirst body\nONE\nsecond body\nTWO\n",
    );
    assert_eq!(stdout, "fd3|second body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_quoted_delimiter_body_is_literal() {
    let dir = scratch("quoted");
    let (stdout, stderr, code) =
        rubash_in(&dir, "who=world\n./reader3.sh 3<<'EOF'\nhello $who\nEOF\n");
    assert_eq!(stdout, "fd3|hello $who\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}

#[test]
fn issue2270_no_delimiter_file_created() {
    // The pre-fix shape "opened" the delimiter as an output file in
    // the cwd; GNU creates nothing.
    let dir = scratch("nostray");
    let (_stdout, _stderr, _code) = rubash_in(&dir, "./reader3.sh 3<<EOF\nbody\nEOF\n");
    assert!(
        !dir.join("EOF").exists(),
        "delimiter word must not become a file in the cwd"
    );
}

#[test]
fn issue2270_fd0_heredoc_control_still_works() {
    let dir = scratch("fd0");
    let (stdout, stderr, code) = rubash_in(&dir, "./reader0.sh <<EOF\nstdin body\nEOF\n");
    assert_eq!(stdout, "fd0|stdin body\nfd0-done\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2270_fd3_plain_file_redirect_control() {
    let dir = scratch("file");
    fs::write(dir.join("input.txt"), "file body\n").expect("write input");
    let (stdout, stderr, code) = rubash_in(&dir, "./reader3.sh 3<input.txt\n");
    assert_eq!(stdout, "fd3|file body\nfd3-open\n", "stderr: {stderr}");
    assert_eq!(code, Some(7), "stderr: {stderr}");
}
