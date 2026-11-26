//! rubash#1102 — exec-installed permanent stdin redirect ignored
//! inside pipeline components.
//!
//! GNU execute_cmd.c / redir.c: an `exec <&-` (or `exec <file`)
//! inside a pipeline stage rebinds the stage's fd 0 for the rest of
//! the stage; the pipeline's own stdin buffer must not leak back in.
//! Here the stage closes stdin with exec and `head` must see the
//! closed descriptor (rc=1), not the piped line.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).
//! (The workspace source file also carries the #1103/#1107 forms;
//! those belong to their own issues' pins.)

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn exec_closed_stdin_shadows_pipeline_buffer() {
    let (stdout, _, code) = rubash("printf 'pipe\\n' | { exec <&-; head -c 10; echo \"rc=$?\"; }");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}
