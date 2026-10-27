//! rubash#1807: nvm's `nvm alias default 20.11.1` wrote an EMPTY alias
//! file because the write path — `nvm_echo "$VERSION" | tee file`
//! inside the `nvm` function, itself called with stdout redirected —
//! lost the pipeline's first-stage output: a pipeline element executed
//! in-process (function or compound stage) inherited the enclosing
//! context's fd-1 binding (the call's `>file`/`>/dev/null` target), so
//! the stage body's writes followed that binding and never reached the
//! pipe; tee still created/truncated its file and read EOF.
//!
//! GNU: execute_pipeline passes pipe_out = fildes[1] for every
//! non-rightmost element (execute_cmd.c:2702-2708), and the element's
//! child installs it with do_piping's dup2(pipe_out, 1)
//! (execute_cmd.c:6373, dup2 at :6392) — the pipe REPLACES whatever
//! fd 1 the enclosing context bound. The rightmost element gets
//! NO_PIPE: its fd 1 stays the context binding. The command-
//! substitution child performs the identical rebind
//! (subst.c:7320 dup2(fildes[1], 1)); rubash's comsub path already
//! mirrors it, the pipeline stage clones were missing it.
//!
//! Oracle: GNU bash 5.3.20, script file.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `script` as a file under rubash in a scratch cwd and return
/// (stdout, stderr, success).
fn run_script(script: &str) -> (String, String, bool) {
    let dir = scratch_tempdir();
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn issue1807_function_stage_inside_redirected_function() {
    // The nvm shape reduced: an inner pipeline whose first stage is a
    // function, inside a function call whose stdout is redirected.
    let (stdout, stderr, ok) = run_script(
        "e() { echo VAL; };\n\
         f() { e | tee o1 >/dev/null; };\n\
         f >/dev/null;\n\
         echo \"tee:[$(cat o1)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_function_stage_redirect_to_file_not_hijacked() {
    // With the call redirected to a real file, the stage output must
    // reach the pipe (tee's file) and NOT the call's redirect target.
    let (stdout, stderr, ok) = run_script(
        "e() { echo VAL; };\n\
         f() { e | tee o1 >/dev/null; };\n\
         f >outer;\n\
         echo \"tee:[$(cat o1)] outer:[$(cat outer)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL] outer:[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_function_stage_external_body_inside_redirected_group() {
    // The leak is in the stage's whole stdio, not just in-process
    // writes: an external command inside the function stage body also
    // wrote to the enclosing binding.
    let (stdout, stderr, ok) = run_script(
        "e() { /bin/echo VAL; };\n\
         { e | tee o1 >/dev/null; } >outer;\n\
         echo \"tee:[$(cat o1)] outer:[$(cat outer)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL] outer:[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_compound_stage_inside_redirected_group() {
    let (stdout, stderr, ok) = run_script(
        "{ { echo VAL; } | tee o1 >/dev/null; } >outer;\n\
         echo \"tee:[$(cat o1)] outer:[$(cat outer)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL] outer:[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_function_stage_dup_bound_fd1() {
    // The enclosing binding need not be a file: fd 1 dup'd onto fd 2's
    // endpoint (`>&2` on the group) hijacked the stage the same way.
    let (stdout, stderr, ok) = run_script(
        "e() { echo VAL; };\n\
         { e | tee o1 >/dev/null; } 2>err >&2;\n\
         echo \"tee:[$(cat o1)] err:[$(cat err)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL] err:[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_final_function_stage_keeps_context_fd1() {
    // Control: the RIGHTMOST element gets NO_PIPE in GNU — its fd 1
    // stays the enclosing binding, so a final function stage inside a
    // redirected group still writes to the group's target.
    let (stdout, stderr, ok) = run_script(
        "e() { echo VAL; };\n\
         { true | e; } >outer;\n\
         echo \"outer:[$(cat outer)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "outer:[VAL]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_builtin_stage_control() {
    // Control: driver-built stages (builtin here) already had their
    // stdio constructed by the driver and were never hijacked.
    let (stdout, stderr, ok) = run_script(
        "f() { echo VAL | tee o1 >/dev/null; };\n\
         f >outer;\n\
         echo \"tee:[$(cat o1)] outer:[$(cat outer)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "tee:[VAL] outer:[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1807_nvm_alias_write_path_seed() {
    // The issue's regression seed in miniature: nvm's alias creation is
    // a dispatcher function that resolves the target and calls a
    // maker whose body pipes a helper FUNCTION (nvm's `nvm_echo`)
    // into tee: `my_echo "$target" | tee "$alias_dir/$name"`;
    // the user-level call conventionally redirects the dispatcher's
    // stdout away. Both the bare form (target needs resolution) and
    // the v-prefixed form must land in the alias file.
    let (stdout, stderr, ok) = run_script(
        "mkdir -p alias;\n\
         my_echo() { echo \"$1\"; };\n\
         make_alias() { my_echo \"$2\" | tee \"alias/$1\" >/dev/null; };\n\
         resolve() { case \"$1\" in v*) echo \"$1\";; *) echo \"$1\";; esac; };\n\
         dispatch() { local t; t=\"$(resolve \"$2\")\"; make_alias \"$1\" \"$t\"; };\n\
         dispatch default 20.11.1 >/dev/null 2>&1;\n\
         echo \"bare:[$(cat alias/default)]\";\n\
         dispatch default v18.20.0 >/dev/null 2>&1;\n\
         echo \"vform:[$(cat alias/default)]\"\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "bare:[20.11.1]\nvform:[v18.20.0]\n");
    assert_eq!(stderr, "");
}
