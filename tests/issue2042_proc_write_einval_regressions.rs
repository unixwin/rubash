//! rubash#2042 item ①: a 0644-class synthetic /proc content entry
//! (`/proc/self/status`) opens for write on GNU procfs and every
//! write fails EINVAL — `echo: write error: Invalid argument`
//! (builtins/common.c:320-334 sh_chkwrite renders it in the
//! builtin's name, status 1; measured on the WSL GNU 5.3 oracle).
//! The absorbed temp sink made these writes silently succeed.

use std::process::Command;

fn run(script: &str) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

const ENTRY: &str = "/proc/self/status";

#[test]
fn issue2042_direct_write_fails_einval_in_the_builtins_name() {
    for script in [format!("echo hi > {ENTRY}"), format!("echo hi >> {ENTRY}")] {
        let (stdout, stderr, code) = run(&script);
        // The failed write is the last command, so the -c exit is the
        // builtin's status 1 (GNU: same shape and exit).
        assert_eq!(code, 1, "script: {script} stderr: {stderr}");
        assert!(
            stderr.contains("echo: write error: Invalid argument"),
            "script: {script} stderr: {stderr}"
        );
        assert_eq!(stdout, "");
    }
    let (stdout, stderr, code) = run(&format!("echo hi > {ENTRY}; echo rc=$?"));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=1", "stdout: {stdout} stderr: {stderr}");
}

#[test]
fn issue2042_bound_fd_write_fails_einval_read_serves_snapshot() {
    // The `<>` bind opens (GNU 0644); the read side serves the
    // generated snapshot; the write fails EINVAL in the writer's
    // name; the failed write does not alter what a later read
    // serves (GNU's observable: the failed write never lands).
    let (stdout, stderr, code) = run(&format!(
        "exec 3<>{ENTRY}; head -1 <&3 > /dev/null; echo hi >&3; echo wrc=$?; head -1 <&3 | cut -c1-4"
    ));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: Invalid argument"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("wrc=1"), "stdout: {stdout}");
}

#[test]
fn issue2042_exec_output_bind_fails_einval_no_phantom_file() {
    // `exec 3>ENTRY` binds the symbolic endpoint; writes fail
    // EINVAL; no phantom enclave file appears at the mapped literal
    // path (GNU: the open hits the real procfs entry, never a
    // directory-side file).
    let (stdout, stderr, code) = run(&format!(
        "cd \"$TMPDIR\"; cd - > /dev/null 2>&1; exec 3>{ENTRY}; echo x >&3; echo wrc=$?; test ! -e proc/self/status && echo nophantom"
    ));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("wrc=1"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(stdout.contains("nophantom"), "stdout: {stdout}");
}

#[test]
fn issue2042_dup_of_the_binding_keeps_the_class() {
    let (stdout, stderr, code) = run(&format!(
        "exec 3>{ENTRY}; exec 4>&3; echo hi >&4; echo wrc=$?"
    ));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: Invalid argument"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("wrc=1"), "stdout: {stdout}");
}
