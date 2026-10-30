//! Regression pins for the two product divergences found by the
//! rubash#1871 executor_tests triage (the other eight were pin
//! corrections or pin-shape races, fixed in place in
//! tests/executor_command_chaining/part_077.rs and part_080.rs).
//!
//! 1. Lexer: a reserved word read inside the `time' option window
//!    (`time -p coproc C { :; }`) was demoted to a plain Word in the
//!    reader history — record_token recomputed
//!    reserved_word_position() AFTER rewriting last_was_time_option
//!    for the current token, so the history lost (Keyword, "coproc")
//!    and the `{` after the coproc NAME never became a group opener
//!    (exit 2). GNU CHECK_FOR_RESERVED_WORD (parse.y:3168) judges the
//!    token by the state in which it was read.
//! 2. A bare assignment's `>(...)` fd outlived the assignment
//!    command: the finish removed the temp carrier but left the
//!    fd-table entry, so a later `printf ... > "$p"` resolved
//!    in-shell to the dead carrier and silently swallowed the write.
//!    GNU closes a command's procsub fds when the command finishes
//!    (the #545 input-side rule); the stored word dangles and the
//!    open fails ENOENT (printf status 1, target left empty by the
//!    assignment's own consumer run).
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1871-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("case.sh");
    fs::write(&script_path, script).expect("write script");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let out = Command::new(bin)
        .arg(&script_path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        dir,
    )
}

#[test]
fn issue1871_time_option_window_keeps_coproc_keyword_position() {
    let (stdout, stderr, code, dir) =
        run_script("time -p coproc TIMEDC { :; }\necho \"pid:${TIMEDC_PID:+set}\"\n");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "pid:set\n", "stderr: {stderr}");
}

#[test]
fn issue1871_alias_time_option_window_keeps_coproc_keyword_position() {
    let (stdout, stderr, code, dir) = run_script(
        "shopt -s expand_aliases\nalias t=time\n\
         t -p coproc ATIMEDC { :; }\necho \"pid:${ATIMEDC_PID:+set}\"\n",
    );
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "pid:set\n", "stderr: {stderr}");
}

#[test]
fn issue1871_assignment_output_procsub_fd_dies_with_assignment() {
    let (stdout, stderr, code, dir) =
        run_script("p=>(cat > out)\nprintf alpha > \"$p\"\necho \"rc=$?\"\n");
    let out_path = dir.join("out");
    let out_contents = fs::read_to_string(&out_path).unwrap_or_else(|_| "<missing>".to_string());
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(out_contents, "", "stderr: {stderr}");
}
