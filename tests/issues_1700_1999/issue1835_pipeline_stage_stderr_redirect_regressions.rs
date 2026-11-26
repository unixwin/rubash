//! rubash#1835: a builtin pipeline stage carrying its own `2>file`
//! lost its diagnostics entirely — the file stayed empty and the real
//! stderr saw nothing. The stage clone kept the stderr redirect, so
//! the builtin's immediate-channel diagnostics were written to the
//! file DURING the stage; the pipeline driver's stream routing then
//! re-applied the same redirect, and its truncating setup open erased
//! the bytes (the stage's captured stderr was empty — the text had
//! never entered the capture). GNU applies a pipeline element's
//! redirections exactly once, in the element's subshell
//! (execute_cmd.c:2620 execute_pipeline -> do_redirections).
//!
//! Fix contract: the builtin stage clone strips the stderr file
//! redirects (fields + ordered list entries) alongside the stdout
//! ones it already stripped; the driver's route_pipeline_stage_streams
//! is the sole applier. Oracle: GNU bash 5.3.20, all shapes below
//! verified byte-for-byte on stdout / err.txt / status.

#![cfg(unix)]

use std::process::{Command, Stdio};

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

/// Run `script` with -c in a scratch cwd; return
/// (stdout, real stderr, err.txt content, pipeline success).
fn run_case(script: &str) -> (String, String, String, bool) {
    let dir = scratch_tempdir();
    let output = Command::new(rubash_bin())
        .arg("-c")
        .arg(script)
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let err_txt = std::fs::read_to_string(dir.0.join("err.txt")).unwrap_or_default();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        err_txt,
        output.status.success(),
    )
}

#[test]
fn issue1835_ulimit_stage_stderr_redirect_keeps_diagnostic() {
    let (stdout, realerr, err_txt, ok) = run_case("ulimit -Z 2>err.txt | cat");
    assert!(ok);
    assert_eq!(stdout, "");
    assert_eq!(realerr, "");
    assert!(
        err_txt.contains("ulimit: -Z: invalid option"),
        "diagnostic must land in err.txt, got [{err_txt}]"
    );
    assert!(err_txt.contains("ulimit: usage:"), "got [{err_txt}]");
}

#[test]
fn issue1835_compgen_stage_stderr_redirect_keeps_diagnostic() {
    let (stdout, realerr, err_txt, ok) = run_case("compgen -Z foo 2>err.txt | cat");
    assert!(ok);
    assert_eq!(stdout, "");
    assert_eq!(realerr, "");
    assert!(
        err_txt.contains("compgen: -Z: invalid option"),
        "diagnostic must land in err.txt, got [{err_txt}]"
    );
    assert!(err_txt.contains("compgen: usage:"), "got [{err_txt}]");
}

#[test]
fn issue1835_stderr_redirect_in_mid_and_last_stage() {
    let (_o, _e, err_txt, _ok) = run_case("printf 'x\\n' | ulimit -Z 2>err.txt | cat");
    assert!(
        err_txt.contains("ulimit: -Z: invalid option"),
        "mid stage: got [{err_txt}]"
    );
    let (_o, _e, err_txt, _ok) = run_case("cat </dev/null | ulimit -Z 2>err.txt");
    assert!(
        err_txt.contains("ulimit: -Z: invalid option"),
        "last stage: got [{err_txt}]"
    );
}

#[test]
fn issue1835_merged_redirect_to_file_keeps_diagnostic() {
    // `>out.txt 2>&1`: the diagnostic belongs to out.txt, in order.
    let dir = scratch_tempdir();
    let output = Command::new(rubash_bin())
        .arg("-c")
        .arg("ulimit -Z >out.txt 2>&1 | cat")
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let out_txt = std::fs::read_to_string(dir.0.join("out.txt")).unwrap_or_default();
    assert!(
        out_txt.contains("ulimit: -Z: invalid option"),
        "merged diagnostic must land in out.txt, got [{out_txt}]"
    );
}

#[test]
fn issue1835_dup_stderr_to_pipe_still_merges() {
    // `2>&1` without a file: the diagnostic rides the pipe (GNU shape
    // that already worked; the strip must not regress it).
    let (stdout, realerr, _err_txt, ok) = run_case("ulimit -Z 2>&1 | cat");
    assert!(ok);
    assert_eq!(realerr, "");
    assert!(
        stdout.contains("ulimit: -Z: invalid option"),
        "diagnostic must ride the pipe, got [{stdout}]"
    );
}

#[test]
fn issue1835_append_form_and_function_stage_unchanged() {
    // `2>>` never truncated, so it worked before the fix; pin it.
    // A function stage applies its redirects in its own frame and was
    // likewise unaffected; pin both against regression.
    let (_o, _e, err_txt, _ok) = run_case("ulimit -Z 2>>err.txt | cat");
    assert!(
        err_txt.contains("ulimit: -Z: invalid option"),
        "append form: got [{err_txt}]"
    );
    let (_o, _e, err_txt, _ok) = run_case("f() { ulimit -Z; }; f 2>err.txt | cat");
    assert!(
        err_txt.contains("ulimit: -Z: invalid option"),
        "function stage: got [{err_txt}]"
    );
}
