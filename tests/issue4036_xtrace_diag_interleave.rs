use std::{env, process::Command};

// rubash#4036: under `set -x`, a command's xtrace line (`+ echo`) must be
// emitted before its failing redirection diagnostic (`3: Bad file descriptor`),
// matching GNU `execute_cmd.c` `execute_simple_command`.

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
        output.status.code(),
    )
}

#[test]
fn xtrace_line_precedes_failing_redirect_diagnostic() {
    let (out, err, code) = run_script("set -x\necho $(echo in 1>&2) 1>&3\n");
    assert_eq!(code, Some(1));
    assert_eq!(out, "");
    let lines: Vec<&str> = err.lines().collect();
    assert!(lines.len() >= 4, "unexpected stderr: {err:?}");
    let xtrace_idx = lines
        .iter()
        .position(|line| line.starts_with("+ echo"))
        .expect("missing `+ echo` xtrace line");
    let bad_fd_idx = lines
        .iter()
        .position(|line| line.contains("3: Bad file descriptor"))
        .expect("missing `3: Bad file descriptor` diagnostic");
    assert!(
        xtrace_idx < bad_fd_idx,
        "expected `+ echo` before `3: Bad file descriptor`, got:\n{err}"
    );
}
