//! Issue rubash#1653: `coproc time CMD | CAT` sets COPROC in the
//! parent; GNU leaves it unset.
//!
//! GNU model (parse.y:1125-1175 coproc productions, parse.y:1416
//! timespec on pipeline_command): after `coproc`, `time` is never
//! the time keyword — it is the coproc NAME when a compound command
//! follows, and otherwise the first word of the simple-command body
//! (`coproc time echo hi` runs a child that reports `time: command
//! not found`). A `|` after that body belongs to the OUTER pipeline,
//! so the coproc lives in the pipeline subshell and the parent never
//! sees COPROC_PID.
//!
//! Two fixes are pinned here:
//! 1. Parser: no `time` arm in the coproc body parse, and `time`
//!    no longer counts as a coproc compound-command start, so the
//!    body falls through to the simple-command collection that
//!    stops at `|`.
//! 2. Coproc child spawn: the simple body is respawned through
//!    `rubash -c <text>`, where a bare head word `time` would regain
//!    keyword status on re-parse. The spawn quotes a bare head
//!    `time` (`'time'`) so the child keeps it an ordinary command
//!    name and reports `time: command not found` like GNU's forked
//!    child.
//!
//! Harness notes: scripts run from files (never `-c` strings), under
//! an isolated PATH holding only `cat` and `sleep`, so `time` can
//! never resolve through PATH on any machine (GNU would run a
//! `/usr/bin/time` the same way, so the `command not found`
//! assertions need that isolation). The child diagnostic is async
//! (the coproc child is a respawned process, not a fork), so every
//! diagnostic assertion sleeps first; the COPROC_PID assertions
//! need no sleep and are timing-independent.
#![cfg(unix)]

use std::process::Command;

struct IsolatedPath {
    dir: std::path::PathBuf,
}

impl IsolatedPath {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue1653-bin-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir(&dir).expect("create isolated bin dir");
        for tool in ["cat", "sleep"] {
            let target = format!("/usr/bin/{tool}");
            if std::path::Path::new(&target).exists() {
                std::os::unix::fs::symlink(&target, dir.join(tool)).expect("symlink tool");
            }
        }
        IsolatedPath { dir }
    }
}

impl Drop for IsolatedPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run a script file under the rubash binary; return (status, stdout, stderr).
fn run(script: &str, path: &IsolatedPath) -> (i32, String, String) {
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1653-probe-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env_clear()
        .env("PATH", path.dir.as_os_str())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&script_path);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1653_pipe_leaves_coproc_unset_in_parent() {
    let path = IsolatedPath::new();
    let (code, stdout, _) = run(
        "coproc time echo hi | cat\ndeclare -p COPROC_PID 2>&1\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("declare: COPROC_PID: not found"),
        "parent must not see COPROC_PID, stdout: {stdout}"
    );
    assert!(!stdout.contains("COPROC_PID=\""), "stdout: {stdout}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
}

#[test]
fn issue1653_pipe_child_reports_time_not_found() {
    let path = IsolatedPath::new();
    let (code, _, stderr) = run(
        "coproc time echo hi | cat\ndeclare -p COPROC_PID 2>&1\nsleep 2\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("time: command not found"),
        "coproc child must report time, stderr: {stderr}"
    );
}

#[test]
fn issue1653_nopipe_sets_coproc_and_reports_time() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc time echo hi\ndeclare -p COPROC_PID 2>&1\nsleep 2\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(
        stdout.contains("COPROC_PID=\""),
        "plain coproc time sets COPROC_PID, stdout: {stdout}"
    );
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1653_time_as_name_before_brace_group() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc time { echo hi; }\ndeclare -p COPROC_PID 2>&1\ncat <&${time[0]}\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(
        stdout.contains("declare: COPROC_PID: not found"),
        "named coproc leaves COPROC_PID unset, stdout: {stdout}"
    );
    assert!(stdout.contains("hi"), "stdout: {stdout}");
}

#[test]
fn issue1653_time_as_name_before_for_loop() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc time for i in 1 2; do echo $i; done\ndeclare -p COPROC_PID 2>&1\ncat <&${time[0]}\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(
        stdout.contains("declare: COPROC_PID: not found"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("1\n2"), "stdout: {stdout}");
}

#[test]
fn issue1653_first_word_reports_its_own_name() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc FOO time echo hi\ndeclare -p COPROC_PID 2>&1\nsleep 2\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.contains("COPROC_PID=\""), "stdout: {stdout}");
    assert!(
        stderr.contains("FOO: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1653_time_dash_p_is_plain_word_after_coproc() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc time -p echo hi\ndeclare -p COPROC_PID 2>&1\nsleep 2\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.contains("COPROC_PID=\""), "stdout: {stdout}");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1653_time_outside_coproc_still_times_pipeline() {
    // Control for rubash#1528: `time` OUTSIDE the coproc still
    // times the pipeline, and the coproc inside it still lives in
    // the pipeline subshell (parent COPROC_PID unset).
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "time coproc echo hi | cat\ndeclare -p COPROC_PID 2>&1\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
    assert!(
        stdout.contains("declare: COPROC_PID: not found"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1653_myc_time_for_is_syntax_error() {
    // GNU: `coproc MYC time -p for ...` is `syntax error near
    // 'do'` — the `for` never opens a loop because it is a plain
    // word inside the simple command.
    let path = IsolatedPath::new();
    let (code, _, stderr) = run(
        "coproc MYC time -p for x in a; do echo $x; done\necho done\n",
        &path,
    );
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("syntax error"), "stderr: {stderr}");
    assert!(stderr.contains("do"), "stderr: {stderr}");
}

#[test]
fn issue1653_plain_coproc_pipe_control() {
    let path = IsolatedPath::new();
    let (code, stdout, stderr) = run(
        "coproc echo hi | cat\ndeclare -p COPROC_PID 2>&1\necho done\n",
        &path,
    );
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
    assert!(
        stdout.contains("declare: COPROC_PID: not found"),
        "stdout: {stdout}"
    );
}
