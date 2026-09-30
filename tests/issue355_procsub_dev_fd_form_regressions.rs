//! Issue rubash#355 regressions: process-substitution words use the GNU
//! `/dev/fd/N` form, not a Windows temp path.
//!
//! GNU owner: subst.c:6362 process_substitute() — the parent's pipe end is
//! moved to a high fd below 64 (move_to_high_fd) and the substituted word
//! is make_dev_fd_filename(fd) = `/dev/fd/N`. Scripts pattern-match and
//! echo this value (`case <(echo c) in /dev/fd/*`), so a temp-file word
//! takes the wrong branch script-visibly.
//!
//! Windows has no /dev/fd filesystem, so rubash carries the stream in the
//! virtual fd table (executor/external_setup.rs
//! register_process_substitution_fd): every `/dev/fd/N` consumer resolves
//! through it — redirects via execution_misc::dev_stdio_redirect_fd ->
//! open_fd_read_endpoint, output targets via redirection.rs
//! open_command_output_target, external argv via
//! dev_fd_operands::materialize_dev_fd_operands, in-process file builtins
//! via dev_fd_operand_bytes. The temp file remains the backing store for
//! Windows child processes that cannot resolve a foreign fd.
//!
//! Documented residuals (platform-bound, in the issue's decision record):
//! - An external child that PRINTS its argv file name (`wc -c <(echo x)`)
//!   shows the materialized temp path in its output, not `/dev/fd/N` —
//!   the child cannot open the virtual fd, so argv must be a real path.
//! - GNU closes every procsub fd at command completion
//!   (execute_cmd.c:460-466 unlink_fifo_list when variable_context==0);
//!   rubash keeps the fd resolvable (a superset: `p=<(echo hi); wc -c <
//!   $p` succeeds here, GNU reports ENOENT). Fds allocate highest-free
//!   below 64 like move_to_high_fd.
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script files 2026-09-28; probes under
//! target/issue-suites/results/resid21/i355/ and tmp/ in the lane
//! worktree).

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn case_pattern_matches_dev_fd_form() {
    let (stdout, stderr, code) =
        rubash_stdin("case <(echo c) in /dev/fd/*) echo fdpath;; *) echo other;; esac\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fdpath\n");
}

#[test]
fn echo_procsub_prints_dev_fd_word() {
    let (stdout, stderr, code) = rubash_stdin("echo <(echo x)\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("/dev/fd/") && stdout.ends_with('\n'),
        "stdout: {stdout}"
    );
    let number = stdout.trim().strip_prefix("/dev/fd/").unwrap();
    assert!(
        !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()),
        "stdout: {stdout}"
    );
}

#[test]
fn assignment_rhs_carries_dev_fd_word() {
    let (stdout, stderr, code) = rubash_stdin("p=<(echo hello)\necho \"$p\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("/dev/fd/"),
        "assignment word form: {stdout}"
    );
}

#[test]
fn in_process_cat_reads_the_stream() {
    let (stdout, stderr, code) = rubash_stdin("cat <(echo payload)\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "payload\n");
}

#[test]
fn external_child_reads_the_stream() {
    // wc is a real external child: argv materializes the fd to a temp the
    // child opens (byte count is the GNU-observable contract).
    let (stdout, stderr, code) = rubash_stdin("wc -c <(printf abc) | tr -d ' \\t\\r'\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("3"),
        "wc output should start with the byte count: {stdout}"
    );
}

#[test]
fn redirect_procsub_still_feeds_read() {
    let (stdout, stderr, code) =
        rubash_stdin("read line < <(echo from-redirect)\necho \"read=[$line]\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "read=[from-redirect]\n");
}

#[test]
fn multiple_words_count_down_from_63() {
    // move_to_high_fd band: first procsub takes 63 (when free), second 62
    // (GNU allocates the same way within one command).
    let (stdout, stderr, code) = rubash_stdin("echo <(echo a) <(echo b)\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let words: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(words.len(), 2, "stdout: {stdout}");
    assert_eq!(words[0], "/dev/fd/63", "stdout: {stdout}");
    assert_eq!(words[1], "/dev/fd/62", "stdout: {stdout}");
}

#[test]
fn dev_fd_word_reopens_through_fd_table() {
    // A variable-carried /dev/fd/N reopens through the virtual fd table
    // (GNU keeps the pipe; rubash keeps the endpoint).
    let (stdout, stderr, code) = rubash_stdin("p=<(echo hi)\nwc -c < $p\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.starts_with("3"), "stdout: {stdout}");
}

#[test]
fn output_procsub_word_form() {
    let (stdout, stderr, code) = rubash_stdin("q=>(cat)\necho \"$q\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.starts_with("/dev/fd/"), "output word form: {stdout}");
}

#[test]
fn diff_procsub_operands_compare_equal() {
    let (stdout, stderr, code) =
        rubash_stdin("diff <(printf 'a\\nb\\n') <(printf 'a\\nb\\n') && echo same\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "same\n");
}
