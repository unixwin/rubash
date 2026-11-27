//! rubash#2482 (face b): the ERR trap inside pipeline elements.
//! A pipeline element is a subshell in GNU (execute_cmd.c
//! execute_pipeline forks it), so its ERR inheritance follows
//! errtrace exactly like a `( )` child (trap.c:1480
//! reset_signal_handlers + the errtrace exception): with `set -E`
//! the parent's ERR trap fires for a failing command inside the
//! element — once, inside the element. The element's own completion
//! takes NO wait-side adjudication: execute_pipeline has no
//! wait_for frame for an element (the execute_cmd.c:769 site belongs
//! to a frame that forked a subshell COMMAND), so `( f ) | cat`
//! fires once, while the same `( f )` standalone fires twice.
//!
//! Expectations are GNU bash 5.3.20 outputs, probed from script
//! files on 2026-11-02. Before the fix, rubash reset the ERR trap
//! unconditionally at element entry (zero fires inside), and — once
//! inheritance was restored — double-fired for `( )` elements whose
//! body failed through a function call, because execute_command's
//! subshell tail applied the standalone wait-side adjudication to
//! the stage root.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2482b-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

// 01 — function element: the ERR trap fires once, inside the
// element, for the body's failing command; the pipeline's status
// is the last stage's, so the script continues.
#[test]
fn issue2482b_01_function_element_fires_once() {
    let dir = test_dir("01");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         f(){ false; }\n\
         f | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ERRFIRE\nafter\n");
    let _ = fs::remove_dir_all(&dir);
}

// 02 — subshell element with a function-call body (bats cell 133's
// shape): one fire inside, NOT a second one for the element's own
// completion.
#[test]
fn issue2482b_02_subshell_element_fires_once() {
    let dir = test_dir("02");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         f(){ false; }\n\
         ( f ) | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ERRFIRE\nafter\n");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — subshell element with a simple-command body, and a brace
// group element: one fire each.
#[test]
fn issue2482b_03_simple_and_group_elements() {
    let dir = test_dir("03");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         ( false ) | cat\n\
         { false; } | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ERRFIRE\nERRFIRE\nafter\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 04 — a nested `( )` command INSIDE the element body keeps its
// own wait-side fire (it is a real subshell command in the element
// child): two fires total — the inner `false` and the inner
// subshell's completion. The element's own completion adds none.
#[test]
fn issue2482b_04_nested_subshell_in_element_keeps_wait_fire() {
    let dir = test_dir("04");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         ( ( false ); echo x ) | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ERRFIRE\nERRFIRE\nafter\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 05 — control: the same `( f )` STANDALONE fires twice (inside
// the body, and the wait-side adjudication for the subshell
// command in the parent frame), and errexit then ends the run.
#[test]
fn issue2482b_05_standalone_subshell_fires_twice() {
    let dir = test_dir("05");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         f(){ false; }\n\
         ( f )\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ERRFIRE\nERRFIRE\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 06 — without errtrace there is nothing to inherit: no fire
// inside the element (the trap was armed in the parent only).
#[test]
fn issue2482b_06_no_errtrace_no_inheritance() {
    let dir = test_dir("06");
    let output = run_script(
        &dir,
        "case.sh",
        "set -e\n\
         trap \"echo ERRFIRE\" ERR\n\
         f(){ false; }\n\
         ( f ) | cat\n\
         f | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after\n");
    let _ = fs::remove_dir_all(&dir);
}

// 07 — errtrace without errexit: GNU fires twice for `( f ) | cat`,
// both times for `false` — once inside `f` (FUNCNAME "errh f
// main"), and once for the `f` call itself in the element child's
// list (FUNCNAME "errh main", BASH_COMMAND frozen at the body's
// last command). With `-e` live (02) the child exits at the call
// and only the first fire remains. Verified with a tagged handler.
#[test]
fn issue2482b_07_errtrace_without_errexit() {
    let dir = test_dir("07");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         trap \"echo ERRFIRE\" ERR\n\
         f(){ false; }\n\
         ( f ) | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ERRFIRE\nERRFIRE\nafter\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 08 — a simple command as a pipeline element never fires (GNU
// execute_cmd.c:1000 requires NO_PIPE on both ends).
#[test]
fn issue2482b_08_simple_element_never_fires() {
    let dir = test_dir("08");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         false | cat\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after\n");
    let _ = fs::remove_dir_all(&dir);
}
