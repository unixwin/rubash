//! rubash#2396 — interactive autocd: a bare directory name at the
//! prompt must `cd` into it instead of failing "command not found".
//!
//! GNU contract (bash 5.3 C source): execute_cmd.c:4909-4917
//! (execute_simple_command) — with the autocd shopt set and the
//! shell interactive, after function and builtin resolution fall
//! through and before the disk-command path, a first word accepted
//! by is_dirname (execute_cmd.c:4451-4462: the findcmd.c
//! search_for_command result when the PATH search finds one, else
//! the raw cwd-relative/slashed word, tested with file_isdir)
//! rewrites the whole word list to `cd -- <words...>`, prints it
//! via xtrace_print_word_list (words, 0), and dispatches through
//! run_builtin with find_function ("cd") taking precedence over
//! the cd builtin.
//!
//! Harness: `rubash -i` over pipes (the session interactive marker
//! is set for `-i` with or without a tty, the same harness shape as
//! tests/issue2214_job_start_notice_regressions.rs). Expectations
//! were verified byte-for-byte against GNU bash 5.3.20 driven the
//! same way (its stderr additionally carries the pipe-only
//! "cannot set terminal process group / no job control" banner,
//! which rubash does not print).

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2396-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("autodir")).expect("create autodir");
    fs::write(dir.join("afile"), b"not a directory").expect("create afile");
    fs::canonicalize(&dir).expect("canonicalize sandbox")
}

/// Run one interactive pipe session; return (stdout, stderr).
fn run_interactive(home: &PathBuf, lines: &[&str]) -> (String, String) {
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut script = lines.join("\n");
    script.push_str("\nexit\n");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });
    let (tx, rx) = mpsc::channel();
    let mut out = child.stdout.take().expect("child stdout");
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (tx2, rx2) = mpsc::channel();
    let mut err = child.stderr.take().expect("child stderr");
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx2.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let _ = writer.join();
    let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx2.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();
    assert!(
        status.is_some_and(|s| s.success()),
        "interactive session did not exit cleanly: {status:?}"
    );
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

#[test]
fn autocd_on_bare_dirname_cds() {
    let home = sandbox("on");
    let (stdout, stderr) = run_interactive(
        &home,
        &["shopt -s autocd", "cd \"$HOME\"", "autodir", "pwd"],
    );
    let target = format!("{}/autodir", home.display());
    assert!(
        stdout.contains(&target),
        "pwd should print the autocd target {target}:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("cd -- autodir"),
        "the transform prints the rewritten command to stderr:\nstderr={stderr}"
    );
    assert!(
        !stderr.contains("command not found"),
        "no command-not-found expected:\nstderr={stderr}"
    );
}

#[test]
fn autocd_off_bare_dirname_is_command_not_found() {
    // Control (GNU SAME): without the shopt, a bare directory name
    // is an ordinary command word.
    let home = sandbox("off");
    let (stdout, stderr) = run_interactive(&home, &["cd \"$HOME\"", "autodir", "pwd"]);
    assert!(stderr.contains("command not found"), "stderr={stderr}");
    assert!(
        stdout
            .lines()
            .any(|line| line == home.display().to_string()),
        "pwd should stay at HOME:\nstdout={stdout}"
    );
    assert!(
        !stdout.contains("autodir"),
        "cwd must not change:\nstdout={stdout}"
    );
}

#[test]
fn autocd_on_regular_file_is_command_not_found() {
    // Control (GNU SAME): is_dirname is false for a regular file.
    let home = sandbox("file");
    let (stdout, stderr) =
        run_interactive(&home, &["shopt -s autocd", "cd \"$HOME\"", "afile", "pwd"]);
    assert!(stderr.contains("command not found"), "stderr={stderr}");
    assert!(
        stdout
            .lines()
            .any(|line| line == home.display().to_string()),
        "pwd should stay at HOME:\nstdout={stdout}"
    );
}

#[test]
fn autocd_slash_form_cds() {
    let home = sandbox("slash");
    let (stdout, stderr) = run_interactive(
        &home,
        &["shopt -s autocd", "cd \"$HOME\"", "./autodir", "pwd"],
    );
    assert!(stderr.contains("cd -- ./autodir"), "stderr={stderr}");
    assert!(
        stdout.contains(&format!("{}/autodir", home.display())),
        "stdout={stdout}"
    );
}

#[test]
fn autocd_extra_arguments_ride_along() {
    // The FULL word list is rewritten (execute_cmd.c:4911-4913
    // prepends to the existing list), so cd itself reports the
    // extra operand.
    let home = sandbox("extra");
    let (stdout, stderr) = run_interactive(
        &home,
        &["shopt -s autocd", "cd \"$HOME\"", "autodir extra", "pwd"],
    );
    assert!(stderr.contains("cd -- autodir extra"), "stderr={stderr}");
    assert!(stderr.contains("too many arguments"), "stderr={stderr}");
    assert!(
        stdout
            .lines()
            .any(|line| line == home.display().to_string()),
        "cwd must not change:\nstdout={stdout}"
    );
}

#[test]
fn autocd_prefers_cd_function() {
    // execute_cmd.c:4916: func = find_function ("cd") — a cd
    // function receives the rewritten words (`-- autodir`).
    let home = sandbox("func");
    let (stdout, stderr) = run_interactive(
        &home,
        &[
            "shopt -s autocd",
            "cd() { echo \"CD-FUNC $*\"; builtin cd \"$@\"; }",
            "cd \"$HOME\"",
            "autodir",
            "pwd",
        ],
    );
    assert!(
        stdout.contains("CD-FUNC -- autodir"),
        "the cd function must run for the transform:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains(&format!("{}/autodir", home.display())),
        "stdout={stdout}"
    );
}

#[test]
fn autocd_not_in_script_mode() {
    // GNU's condition includes `interactive`: a script never
    // autocds even with the shopt set inside it.
    let home = sandbox("script");
    let script = home.join("case.sh");
    fs::write(&script, "shopt -s autocd\ncd \"$HOME\"\nautodir\npwd\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .output()
        .expect("run script");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("command not found"), "stderr={stderr}");
    assert!(
        stdout
            .lines()
            .any(|line| line == home.display().to_string()),
        "cwd must not change:\nstdout={stdout}"
    );
}
