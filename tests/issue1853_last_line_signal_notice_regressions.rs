//! Regression pins for rubash#1853 — a signal death in a script/stdin
//! stream's LAST command never flushed its pending job notice before
//! the shell exited.
//!
//! GNU prints the notice without needing a following command:
//! wait_for calls notify_and_cleanup for the dead foreground job as
//! the wait completes (jobs.c:3385), and
//! maybe_print_job_notifications prints whenever the shell is not
//! interactive (jobs.c:4723-4728), background deaths included. Rubash
//! queued the notice for the next top-level command boundary
//! (report_pending_signal_notices in execute_ast), so a stream that
//! ended right after the death exited silently — exit code 129 on
//! both sides, only the notice missing.
//!
//! Shapes (all byte-checked against GNU bash 5.3.20, the self-built
//! oracle, script-file probes 2026-10-27):
//! - script file whose last line is a foreground child dying by
//!   SIGHUP: GNU prints `<script>: line 1: <pid> Hangup ...`, rc 129.
//! - the same through the stdin stream.
//! - a background child dying while the last foreground command
//!   waits: GNU prints its notice at stream end too.
//! - with an EXIT trap, the notice precedes the trap's output.
//! - control: a followed death already printed at the boundary and
//!   must still print exactly once.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1853-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("file");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash stdin");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const DIE_LAST: &str = "( kill -HUP $BASHPID )\n";

#[test]
fn issue1853_script_last_line_foreground_signal_death_prints_notice() {
    let (_stdout, stderr, code) = rubash_file(DIE_LAST);
    assert!(
        stderr.contains("Hangup") && stderr.contains("( kill -HUP $BASHPID )"),
        "last-line foreground death notice missing: {stderr:?}"
    );
    assert!(
        stderr.contains("line 1:"),
        "notice must carry the line prefix: {stderr:?}"
    );
    assert_eq!(code, Some(129));
}

#[test]
fn issue1853_stdin_last_line_foreground_signal_death_prints_notice() {
    let (_stdout, stderr, code) = rubash_stdin(DIE_LAST);
    assert!(
        stderr.contains("Hangup") && stderr.contains("( kill -HUP $BASHPID )"),
        "stdin last-line foreground death notice missing: {stderr:?}"
    );
    assert_eq!(code, Some(129));
}

#[test]
fn issue1853_background_death_during_last_command_prints_notice() {
    let (_stdout, stderr, code) = rubash_file("( kill -HUP $BASHPID ) &\nsleep 0.5\n");
    assert!(
        stderr.contains("Hangup"),
        "background death notice missing at stream end: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1853_notice_precedes_exit_trap_output() {
    let (_stdout, stderr, code) =
        rubash_file("trap 'echo EXIT-TRAP-RAN >&2' EXIT\n( kill -HUP $BASHPID )\n");
    let notice_at = stderr.find("Hangup").expect("notice missing");
    let trap_at = stderr.find("EXIT-TRAP-RAN").expect("trap output missing");
    assert!(
        notice_at < trap_at,
        "notice must precede EXIT trap output: {stderr:?}"
    );
    assert_eq!(code, Some(129));
}

#[test]
fn issue1853_followed_death_still_prints_exactly_once() {
    let (stdout, stderr, _code) = rubash_file("( kill -HUP $BASHPID )\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(
        stderr.matches("Hangup").count(),
        1,
        "notice must print exactly once: {stderr:?}"
    );
}

#[test]
fn issue1853_command_string_group_final_death_prints_notice() {
    // A -c string whose final unit is a brace group cannot take the
    // exec-replacement path, so GNU's surviving shell prints the
    // notice at string end as well.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("{ ( kill -HUP $BASHPID ); }")
        .output()
        .expect("run rubash -c");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("Hangup"),
        "-c group-final death notice missing: {stderr:?}"
    );
    assert_eq!(output.status.code(), Some(129));
}
