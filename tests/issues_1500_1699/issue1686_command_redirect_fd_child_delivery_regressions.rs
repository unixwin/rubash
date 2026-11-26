//! Issue rubash#1686: a numbered fd opened by a command-level
//! redirection (`cmd 9>file`) was created in the parent but never
//! delivered to a foreign child — the child saw EBADF on the fd. The
//! binding only traveled the __RUBASH_FD_* env channel, which just
//! rubash children restore; GNU's fork simply hands the child the
//! descriptor at its number (redir.c do_redirections runs before the
//! fork for disk commands). The spawn path now pins the command's own
//! redirect descriptors onto their slots next to the ambient pins
//! (rubash#1626), with the command's binding winning any slot an
//! ambient `exec` fd already occupies.
//!
//! Every test drives a REAL foreign child (/usr/bin/bash) so the env
//! channel cannot mask the result, and asserts the GNU bash 5.3.20
//! behavior measured with the same scripts.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1686-{tag}-{}", std::process::id()));
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

fn file_contents(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[test]
fn issue1686_command_fd_write_reaches_external_child() {
    let dir = tempdir("write");
    let run = rubash_c("/usr/bin/bash -c 'echo own >&9' 9>own.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(file_contents(&dir, "own.txt"), "own\n");
}

#[test]
fn issue1686_command_fd_read_reaches_external_child() {
    let dir = tempdir("read");
    std::fs::write(dir.join("seed.txt"), "seed-line\n").expect("seed input");
    let run = rubash_c(
        "/usr/bin/bash -c 'read -u 3 line; echo \"read=[$line]\"' 3<seed.txt",
        &dir,
    );
    assert_eq!(run.stdout, "read=[seed-line]\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1686_command_fd_append_reaches_external_child() {
    let dir = tempdir("append");
    std::fs::write(dir.join("app.txt"), "a\n").expect("seed append target");
    let run = rubash_c("/usr/bin/bash -c 'echo b >&9' 9>>app.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(file_contents(&dir, "app.txt"), "a\nb\n");
}

#[test]
fn issue1686_command_fd_listed_in_external_child() {
    let dir = tempdir("listing");
    let run = rubash_c("/usr/bin/bash -c 'ls /proc/$$/fd' 9>own.txt", &dir);
    assert!(
        run.stdout.lines().any(|fd| fd.trim() == "9"),
        "child fd listing lacks 9: {}",
        run.stdout
    );
}

#[test]
fn issue1686_command_fd_wins_over_ambient_binding() {
    let dir = tempdir("collision");
    // The command's own `9>own.txt` is the more local binding: the child
    // must write own.txt, and the ambient target must stay empty (the
    // rubash#1626 safety invariant — never the wrong file).
    let run = rubash_c(
        "exec 9>amb.txt; /usr/bin/bash -c 'echo own >&9' 9>own.txt",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(file_contents(&dir, "own.txt"), "own\n");
    assert_eq!(file_contents(&dir, "amb.txt"), "");
}
