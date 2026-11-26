//! rubash#1650 — `shift` with surplus operands printed
//! `shift: too many arguments` but kept executing the rest of the
//! line, and a function body continued past it so the function's
//! status was overwritten by the next command.
//!
//! GNU ownership: builtins/common.c no_args() — the diagnostic,
//! set_exit_status (EX_BADUSAGE), then jump_to_top_level (DISCARD).
//! The discard abandons the rest of the current list: a function
//! body unwinds with status 2 and its caller continues, a subshell
//! ends with 2, the next script line runs, and under `-c` the whole
//! string is abandoned with status 1 (shell.c:1477 run_one_command
//! DISCARD catch). Only the surplus class discards: a non-numeric
//! operand returns plain EX_USAGE and execution continues.
//!
//! The fix reuses the `exit 1 2` no_args channel
//! (command_dispatch_primary.rs): the shift executor arms
//! raise_evalerror_abort() plus the exit-too-many marker instead of
//! returning a plain status.
//!
//! Expectations verified against the issue's GNU bash 5.3.20
//! transcript (local bash here is 5.2 and reports after=1 for the
//! function case — a version difference in the DISCARD catch, not
//! the 5.3 behaviour this repo targets). Line semantics need script
//! files: under `-c` a DISCARD abandons the whole string.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let path = std::env::temp_dir().join(format!("rubash-1650-{tag}.sh"));
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
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
fn shift_surplus_abandons_function_body_with_status_2() {
    let (stdout, _stderr, code) =
        rubash_file("f() { shift 1 2; echo \"n=$# rc=$?\"; }\nf a b c\necho \"after=$?\"\n");
    assert_eq!(stdout, "after=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn shift_surplus_skips_rest_of_line_only() {
    let (stdout, _stderr, code) = rubash_file("shift 1 2; echo same\necho next\n");
    assert_eq!(stdout, "next\n");
    assert_eq!(code, Some(0));
}

#[test]
fn shift_surplus_ends_subshell_with_status_2() {
    let (stdout, _stderr, code) = rubash_file("(shift 1 2; echo inner)\necho after=$?\n");
    assert_eq!(stdout, "after=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn shift_nonnumeric_does_not_discard() {
    let (stdout, _stderr, code) = rubash_file("set -- a b\nshift x; echo \"continues rc=$?\"\n");
    assert_eq!(stdout, "continues rc=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn shift_surplus_under_c_abandons_whole_string_with_status_1() {
    let (stdout, _stderr, code) = rubash_c("shift 1 2; echo after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}
