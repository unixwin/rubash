//! rubash#1660 — `history` with surplus operands printed
//! `history: too many arguments` but kept executing the rest of
//! the line, and a function body continued past it so the
//! function's status was overwritten.
//!
//! Same GNU mechanism as rubash#1650 (`shift`): builtins/common.c
//! no_args() — the diagnostic, set_exit_status (EX_BADUSAGE),
//! then jump_to_top_level (DISCARD). The fix arms the same
//! no_args channel (raise_evalerror_abort + the exit-too-many
//! marker) from both history implementations: the session path
//! (history_exec.rs, the script path) and the provider path
//! (builtins/history.rs, via a surplus out-flag — the other
//! EX_USAGE paths there keep a plain status with no discard).
//!
//! Expectations verified against the issue's GNU bash 5.3.20
//! transcript. Line semantics need script files: under `-c` a
//! DISCARD abandons the whole string with status 1.

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
    let path = std::env::temp_dir().join(format!("rubash-1660-{tag}.sh"));
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
fn history_surplus_skips_rest_of_line_only() {
    let (stdout, _stderr, code) = rubash_file("history 1 2; echo \"ran=$?\"\necho next\n");
    assert_eq!(stdout, "next\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_surplus_on_own_line_leaves_next_line_running() {
    let (stdout, _stderr, code) = rubash_file("history 1 2\necho \"ran=$?\"\n");
    assert_eq!(stdout, "ran=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_surplus_abandons_function_body_with_status_2() {
    let (stdout, _stderr, code) =
        rubash_file("f() { history 1 2; echo \"inner=$?\"; }\nf\necho \"after=$?\"\n");
    assert_eq!(stdout, "after=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_surplus_abandons_multiline_function_body() {
    let (stdout, _stderr, code) =
        rubash_file("f() {\n  history 1 2\n  echo inner\n}\nf\necho \"after=$?\"\n");
    assert_eq!(stdout, "after=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_nonnumeric_does_not_discard() {
    let (stdout, _stderr, code) = rubash_file("history x; echo continues\n");
    assert_eq!(stdout, "continues\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_surplus_under_c_abandons_whole_string_with_status_1() {
    let (stdout, _stderr, code) = rubash_c("history 1 2; echo after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
}
