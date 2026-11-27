//! Issue rubash#1966: a NUMBERED here-string was invisible to the
//! `read` builtin — `read x 0<<< hello` read EOF (empty value,
//! status 1) while the unnumbered `read x <<< hello` and the external
//! `cat 0<<< hello` both served the word. The parser registers the
//! two forms in different stores (unnumbered in cmd.here_string,
//! numbered in heredoc_redirects with fd == Some(0)); the resolver
//! `stdin_string_for_command_mut` reads both, but the read funnel's
//! admission check consulted only the unnumbered field, so the
//! numbered body never reached the builtin and the read fell through
//! to the process stdin.
//!
//! GNU 5.3.20 oracle (script-file probes): `read x 0<<< hello` binds
//! hello with status 0; the same holds for `0<<EOF` heredocs, for
//! words with spaces, and inside while-read loops.
//!
//! The fix touches registration/visibility only (the funnel's
//! admission predicate), never the here-string carrier bytes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1966-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue1966_numbered_herestring_reaches_read() {
    let dir = tempdir("basic");
    let run = rubash_c("read x 0<<< hello; echo \"rc=$? x=[$x]\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("rc=0"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("x=[hello]"), "stdout={}", run.stdout);
}

#[test]
fn issue1966_numbered_herestring_word_with_spaces() {
    let dir = tempdir("spaces");
    let run = rubash_c("read w 0<<< 'a b c'; echo \"w=[$w]\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("w=[a b c]"), "stdout={}", run.stdout);
}

#[test]
fn issue1966_numbered_herestring_in_while_loop() {
    let dir = tempdir("loop");
    let run = rubash_c(
        "while IFS= read -r l; do echo \"[$l]\"; done 0<<< 'only-line'",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("[only-line]"), "stdout={}", run.stdout);
}

#[test]
fn issue1966_numbered_heredoc_reaches_read() {
    let dir = tempdir("heredoc");
    let run = rubash_c("read x 0<<EOF; echo \"x=[$x]\"\nbody-line\nEOF", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("x=[body-line]"),
        "stdout={}",
        run.stdout
    );
}

#[test]
fn issue1966_unnumbered_and_fd3_forms_unchanged() {
    let dir = tempdir("controls");
    let run = rubash_c(
        "read a <<< plain; read -u 3 z 3<<< world; echo \"a=[$a] z=[$z]\"",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("a=[plain]"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("z=[world]"), "stdout={}", run.stdout);
}

#[test]
fn issue1966_later_file_redirect_still_wins() {
    let dir = tempdir("winner");
    std::fs::write(dir.join("f.txt"), "from-file\n").expect("seed");
    // GNU redir.c: redirections apply left to right; the LAST fd-0
    // input redirect wins, so the file outranks the earlier 0<<<.
    let run = rubash_c("read x 0<<< from-string < f.txt; echo \"x=[$x]\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("x=[from-file]"),
        "stdout={}",
        run.stdout
    );
}
