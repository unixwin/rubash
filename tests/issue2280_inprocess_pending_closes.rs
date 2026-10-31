//! rubash#2280 — `{var}` deferred closes never took effect during
//! builtin/function execution. The applier records them
//! (rubash#2272) for the spawn delivery to perform, but an
//! in-process command has no delivery: the records were dropped
//! unapplied at undo, so a function body (or the builtin itself)
//! still saw the descriptor open — GNU runs a builtin/function's
//! do_redirections in the shell's own context
//! (execute_cmd.c:5648 execute_builtin_or_function), so the close
//! is visible for the whole execution and the binding is restored
//! when the command completes.
//!
//! Fix: at dispatch, commands classified in-process (the funnel's
//! existing builtin/function classification, and the function
//! fast path which bypasses the funnel) perform their recorded
//! closes in the table via
//! Executor::apply_fd_var_closes_for_in_process and replay the
//! snapshots when the dispatch returns. The snapshots are kept
//! locally on purpose: a body command's applier run clears the
//! shared fd_var_close_restore list. Close snapshots replay in
//! reverse (stack discipline) — a descriptor closed twice by one
//! command restores to its pre-command binding; the shared undo
//! (undo_child_fd_var_redirects) replays through the same helper.
//!
//! Expectations byte-verified against GNU bash 5.3.20 via
//! probes/p2280.sh and probes/p2280-double.sh (oracle runs in the
//! cluster workdir). The seed carries two lines so "still open"
//! (next line readable) is distinguishable from "closed" (read
//! fails). Observers are nested rubash spawns (never ENOEXEC
//! script children, rubash#2279's harness lesson).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2280-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed"), "SEED1\nSEED2\n").expect("write seed");
    dir
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2280_function_body_sees_the_close_and_parent_survives() {
    let dir = scratch("func");
    let script = "exec {v}<seed; \
                  f() { if read -u \"$v\" line; then echo \"during=OK:$line\"; else echo \"during=FAIL\"; fi; }; \
                  f {v}<&-; \
                  if read -u \"$v\" line; then echo \"after=OK:$line\"; else echo \"after=FAIL\"; fi";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "during=FAIL\nafter=OK:SEED1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2280_builtin_own_read_of_the_closed_fd_fails() {
    let dir = scratch("builtin");
    let script = "exec {v}<seed; \
                  read -u \"$v\" line2 {v}<&-; echo \"rc=$? line2=${line2-UNSET}\"; \
                  read -u \"$v\" line3; echo \"after=$line3\"";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(
        stdout, "rc=1 line2=UNSET\nafter=SEED1\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2280_spawn_face_control_still_closed_for_the_child() {
    let dir = scratch("spawn");
    let inner = "if : <&10; then echo fd10=OPEN; else echo fd10=CLOSED; fi";
    let script = format!("exec {{v}}<seed; echo \"v=$v\"; \"{RUBASH}\" -c '{inner}' {{v}}<&-");
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "v=10\nfd10=CLOSED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2280_double_close_spawn_face_parent_survives() {
    let dir = scratch("double-spawn");
    let inner = "if : <&10; then echo fd10=OPEN; else echo fd10=CLOSED; fi";
    let script = format!(
        "exec {{v}}<seed; \"{RUBASH}\" -c '{inner}' {{v}}<&- {{v}}<&-; echo \"parent=$(cat <&10)\""
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(
        stdout, "fd10=CLOSED\nparent=SEED1\nSEED2\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2280_double_close_function_face_parent_survives() {
    let dir = scratch("double-func");
    let script = "exec {v}<seed; \
                  f() { if read -u \"$v\" line; then echo \"during=OK:$line\"; else echo \"during=FAIL\"; fi; }; \
                  f {v}<&- {v}<&-; \
                  if read -u \"$v\" line; then echo \"after=OK:$line\"; else echo \"after=FAIL\"; fi";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "during=FAIL\nafter=OK:SEED1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}
