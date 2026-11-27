//! rubash#1473 — under `set -T` (functrace), a function call inside
//! $() fired the DEBUG trap once with $BASH_COMMAND set to the OUTER
//! assignment; GNU fires it for the function invocation (twice: the
//! call and entry) with $BASH_COMMAND == "f", then for the body.
//!
//! Probe: clusters/x2/repro/1473.sh. The pinned byte stream is the
//! full interleaving GNU produces: trap output on the shell's stdout
//! for outer commands, and captured into x for the comsub's commands.
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn debug_trap_in_comsub_function_call_reports_inner_command() {
    let (stdout, stderr, code) = rubash(
        "set -T\n\
         trap 'echo \"DBG:$BASH_COMMAND\"' DEBUG\n\
         f() { echo body; }\n\
         x=$(f)\n\
         echo \"x=[$x]\"\n\
         y=$(echo hi)\n\
         echo \"y=[$y]\"",
    );
    assert_eq!(
        stdout,
        "DBG:x=$(f)\n\
         DBG:echo \"x=[$x]\"\n\
         x=[DBG:f\nDBG:f\nDBG:echo body\nbody]\n\
         DBG:y=$(echo hi)\n\
         DBG:echo \"y=[$y]\"\n\
         y=[DBG:echo hi\nhi]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
