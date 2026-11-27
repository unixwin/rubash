//! rubash#2081 — in POSIX mode a pipeline stage named by a special
//! builtin ran a same-named FUNCTION, while GNU runs the builtin.
//!
//! GNU execute_cmd.c:4657-4663 (execute_simple_command): "Posix.2 says
//! special builtins are found before functions" — in posix mode the
//! special-builtin lookup precludes find_function entirely. Every
//! pipeline element reaches that same classification through
//! execute_pipeline (execute_cmd.c:2620), so a stage named `source`,
//! `eval`, ... runs the builtin even when a function shadows the name.
//! The top-level face already declined the function route
//! (execute_function_command_invocation), but the pipeline stage
//! driver's fall-through for unclaimed names entered
//! execute_function_pipeline_stage without the preclusion, so the
//! function won on the stage face for all 16 special names.
//!
//! Fix: execute_function_pipeline_stage itself declines when the
//! expanded stage name is a posix special builtin and posix mode is
//! on — the preclusion lives at the function route, so every stage
//! path (sequential fall-through, shell-writer producer) inherits it.
//!
//! All expectations measured against GNU bash 5.3.20 with script
//! files (self-compiled oracle, 2026-10-29). Default-mode controls
//! pin the opposite verdict (the function wins) so the gate cannot
//! overreach.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2081-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2081_posix_pipeline_source_and_dot_run_builtin_not_function() {
    // The issue's P12 probe: both spellings of the dot builtin.
    let (stdout, stderr, code) = run(
        "source-dot",
        "source() { echo PIPEFUNC-source; }\n.() { echo PIPEFUNC-dot; }\nset -o posix\nsource /dev/null | cat\n. /dev/null | cat\necho \"pipe-done\"\n",
    );
    assert_eq!(stdout, "pipe-done\n", "stdout: {stdout} stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2081_posix_pipeline_eval_runs_builtin_not_function() {
    // The issue's P12b probe: the builtin eval executes its argument
    // (IN-eval reaches the pipe); the shadowing function would print
    // PIPEFUNC-eval instead.
    let (stdout, stderr, code) = run(
        "eval",
        "eval() { echo PIPEFUNC-eval; }\nset -o posix\neval 'echo IN-eval' | cat\necho \"pipe-done\"\n",
    );
    assert_eq!(
        stdout, "IN-eval\npipe-done\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2081_posix_pipeline_export_runs_builtin_not_function() {
    // A second special family member (assignment builtin): the stage
    // must run the builtin (silent), not the echoing function.
    let (stdout, stderr, code) = run(
        "export",
        "export() { echo PIPEFUNC-export; }\nset -o posix\nexport RUBASH2081_X=1 | cat\necho \"pipe-done\"\n",
    );
    assert_eq!(stdout, "pipe-done\n", "stdout: {stdout} stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2081_posix_pipeline_special_as_later_stage() {
    // The preclusion is per-element, not a stage-0 property: a special
    // name in a downstream stage also runs the builtin.
    let (stdout, stderr, code) = run(
        "later-stage",
        "source() { echo PIPEFUNC-source; }\nset -o posix\nprintf 'z\\n' | source /dev/null\necho \"after\"\n",
    );
    assert_eq!(stdout, "after\n", "stdout: {stdout} stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2081_default_mode_pipeline_function_still_shadows() {
    // Control: without posix mode the function keeps winning on the
    // stage face (GNU default classification) — the fix must not leak.
    let (stdout, stderr, code) = run(
        "default-mode",
        "source() { echo PIPEFUNC-source; }\neval() { echo PIPEFUNC-eval; }\nsource /dev/null | cat\neval 'echo IN-eval' | cat\necho \"pipe-done\"\n",
    );
    assert_eq!(
        stdout, "PIPEFUNC-source\nPIPEFUNC-eval\npipe-done\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2081_posix_simple_command_face_unchanged() {
    // The already-aligned simple-command face stays aligned: posix
    // mode, no pipeline — the builtin runs, the function does not.
    let (stdout, stderr, code) = run(
        "simple-face",
        "source() { echo FUNC-source; }\nset -o posix\nsource /dev/null\necho \"done\"\n",
    );
    assert_eq!(stdout, "done\n", "stdout: {stdout} stderr: {stderr}");
    assert_eq!(code, Some(0));
}
