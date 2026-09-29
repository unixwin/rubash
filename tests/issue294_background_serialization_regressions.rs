//! Issue rubash#294 regressions (wt12/ctrlcfix lane).
//!
//! A background command inside a function body killed every respawned
//! async child: `background_command_source`
//! (executor/compound_exec.rs) serializes ALL defined functions into the
//! child's `-c` script and joined body commands with `"; "`, so a body
//! command serialized as `true &` became `true &; while ...` — invalid
//! under GNU's list grammar. The child died with `syntax error near
//! unexpected token `;'' before running anything, so the canonical
//! trap-terminated async construct hung forever:
//!
//!   trap 'echo TERM; return' TERM
//!   f() { ( sleep 1; kill -TERM $$ ) & until (exit 42); do (exit 42); done; }
//!   f; printf 'status:%s\n' "$?"
//!
//! — the async `kill -TERM $$` never ran, the TERM trap never fired, and
//! the infinite `until` loop (condition `exit 42` is never true) spun
//! with no exit path: the CI 30-minute timeout hang (rubash#294) and the
//! wedged rubash.exe processes seen across worktrees.
//!
//! GNU ownership: there is no serialization upstream — GNU forks
//! (execute_cmd.c make_child, execute_in_subshell). The respawn stand-in
//! must emit text that re-parses under GNU's grammar: parse.y:1275/1290
//! `list1 '&' newline_list [list1]` admit only newline_list after `&`,
//! and print_cmd.c:1520-1529 `semicolon()` suppresses the `;' separator
//! after a command printed as `... &` (the exact rule this fix ports).
//! The same rule was missing in `exported_function_env_value`
//! (executor/function_env.rs), whose exportstr value is re-imported by
//! child shells (variables.c:3989-4034 push_exported_function is the
//! upstream export path).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probe 2026-09-28: `TERM`,
//! `status:42`, `DONE`, exit 0).

use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The minimal parse shape: `&` then a compound command inside a function
/// body. Before the fix this printed `syntax error near unexpected token
/// `;'' (with exit 2); GNU runs it silently with exit 0.
#[test]
fn background_then_compound_in_function_body_parses() {
    let (stdout, stderr, code) = rubash_c("f() { true & while false; do :; done; }; f");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "unexpected syntax error: {stderr}"
    );
}

/// The canonical rubash#294 shape: an infinite `until` loop whose only
/// exit is a TERM trap fired by an async `kill -TERM $$`, with `return`
/// unwinding the function. GNU 5.3.0 prints `TERM` then `status:42` and
/// exits 0; before the fix this hung forever (child never ran its kill).
#[test]
fn async_kill_self_terminates_trap_until_loop() {
    let (stdout, stderr, code) = rubash_c(
        "trap 'echo TERM; return' TERM; \
         f() { ( sleep 1; kill -TERM $$ ) & until (exit 42); do (exit 42); done; }; \
         f; printf 'status:%s\n' \"$?\"",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "TERM\nstatus:42\n", "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// A body ENDING in `&` serializes with `&\n}` (background_command_source
/// tail); the loop after the background command serializes via the newline
/// joiner. Whole-script byte parity with the GNU probe output.
#[test]
fn background_tail_and_function_roundtrip_script_file() {
    let script = "trap 'echo TERM; return' TERM\nf() {\n  ( sleep 1; kill -TERM $$ ) &\n  until (exit 42); do (exit 42); done\n}\nf\nprintf 'status:%s\\n' \"$?\"\necho DONE\n";
    let dir = std::env::temp_dir().join(format!(
        "rubash-i294-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("repro294.sh");
    std::fs::write(&path, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "TERM\nstatus:42\nDONE\n", "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// The exportstr half: an exported function whose body contains a
/// background command must re-import into a child shell intact. Before
/// the function_env.rs fix the value carried `true &;` and the child died
/// with a syntax error instead of defining the function.
#[test]
fn exported_function_with_background_body_reimports() {
    let (stdout, stderr, code) = rubash_c(concat!(
        "f() { echo bg-ok & wait; }; ",
        "export -f f; ",
        "\"${THIS_SH}\" -c 'declare -F f && f' | tail -1",
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("bg-ok"),
        "child did not run the imported function; stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("syntax error"),
        "exportstr re-import syntax error: {stderr}"
    );
}
