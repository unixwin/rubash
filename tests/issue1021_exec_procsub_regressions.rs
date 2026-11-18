//! rubash#1021: `exec` with a redirection fed by process substitution
//! must not block until the producer finishes.
//!
//! GNU anchor: `execute_cmd.c` runs the `exec` builtin's redirections
//! over the /dev/fd entry the substitution already opened, so the shell
//! replaces itself / continues at once; only a later read waits for
//! data. rubash materialized the substitution by draining it first, so
//! `exec 3< <(sleep 2; echo hi)` stalled for the full sleep and an
//! unbounded producer (`tail -f`) hung the shell forever.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::time::Duration;

/// `exec` over a slow producer must reach the next command promptly:
/// GNU prints `after-exec` within ~0.1 s although the producer sleeps
/// 30 s. The read runs under a deadline so a blocking build fails the
/// test instead of hanging the suite.
#[test]
#[cfg(unix)]
fn exec_over_procsub_does_not_wait_for_producer() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "exec 3< <(sleep 30; echo hi); echo after-exec"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    let stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(Duration::from_secs(15)).unwrap_or_default();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(line, "after-exec\n");
}

/// The redirected fd stays readable once data arrives: GNU reads
/// `hi` back from fd 3.
#[test]
fn exec_procsub_fd_remains_readable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            "exec 3< <(echo hi); read -r line <&3; echo \"got:$line\"",
        ])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "got:hi\n");
}
