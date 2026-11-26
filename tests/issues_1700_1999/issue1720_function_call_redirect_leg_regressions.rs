//! rubash#1720 (bats scenario chain), function-call redirect layer:
//! a body command's own `> file` lost to the function CALL's
//! `> target` — the call's bytes landed in the call target and the
//! body's file stayed empty. Root: apply_function_call_redirects
//! classified the call's plain fd-1 entry as a STDERR leg too
//! (is_stderr_write_redirect defaults a missing fd to 2, which serves
//! the redirect_err FIELD contexts, not ordered-list entries), and
//! the stderr applicator pushed that clone into body leaves after
//! their own fd-1 entries, so the ordered walk resolved fd 1 to the
//! call's target. GNU: a command's own do_redirections inside
//! execute_function's frame is the last word for its descriptors
//! (execute_cmd.c execute_function; redir.c do_redirections).
//!
//! Oracle: GNU bash 5.3.20. This is the layer that emptied the
//! ecosuite bats chain's setup_file markers; the chain's remaining
//! blocker after this fix is the here-string carrier layer recorded
//! in the pipe-fd cluster REPORT (not covered here).

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

fn run(script: &str) -> ScratchDir {
    let dir = scratch_tempdir();
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

fn content(dir: &ScratchDir, name: &str) -> String {
    std::fs::read_to_string(dir.0.join(name)).unwrap_or_default()
}

#[test]
fn issue1720_body_own_stdout_redirect_beats_call_redirect() {
    let dir = run("f() { printf 'x\\n' > body.txt; }\nf > call.txt\n");
    assert_eq!(content(&dir, "body.txt"), "x\n");
    assert_eq!(content(&dir, "call.txt"), "");
}

#[test]
fn issue1720_body_without_own_redirect_inherits_call_target() {
    // The inheritance itself is the applicator's purpose: a body
    // command with no redirect of its own writes to the call's target.
    let dir = run("f() { printf 'y\\n'; }\nf > call.txt\n");
    assert_eq!(content(&dir, "call.txt"), "y\n");
}

#[test]
fn issue1720_body_own_redirect_for_echo_and_external() {
    let dir = run(
        "f() { echo e > body1.txt; }\nf > call.txt\ng() { /bin/echo x > body2.txt; }\ng > call.txt\n",
    );
    assert_eq!(content(&dir, "body1.txt"), "e\n");
    assert_eq!(content(&dir, "body2.txt"), "x\n");
    assert_eq!(content(&dir, "call.txt"), "");
}

#[test]
fn issue1720_body_own_stderr_redirect_beats_call_stderr() {
    let dir = run("f() { printf '%z\\n' 2> bodyerr.txt; }\nf 2> callerr.txt\ntrue\n");
    assert!(
        content(&dir, "bodyerr.txt").contains("invalid format character"),
        "body's own 2> must win: [{}]",
        content(&dir, "bodyerr.txt")
    );
    assert_eq!(content(&dir, "callerr.txt"), "");
}

#[test]
fn issue1720_combined_call_redirect_still_routes_both_streams() {
    // `f &> both.txt`: the combined entry's fd-2 leg is split out by
    // the caller (fd set explicitly) and must keep working.
    let dir = run("f() { echo out; echo err >&2; }\nf &> both.txt\n");
    let both = content(&dir, "both.txt");
    assert!(both.contains("out"), "got [{both}]");
    assert!(both.contains("err"), "got [{both}]");
}

#[test]
fn issue1720_append_call_redirect_and_nested_calls() {
    let dir = run(
        "td() { printf 'teardown:1\\n' > marker.txt; }\ntd >> cap.txt 2>&1\nb() { printf 'inner\\n' > inner.txt; }\na() { b; }\na > outer.txt\n",
    );
    assert_eq!(content(&dir, "marker.txt"), "teardown:1\n");
    assert_eq!(content(&dir, "cap.txt"), "");
    assert_eq!(content(&dir, "inner.txt"), "inner\n");
    assert_eq!(content(&dir, "outer.txt"), "");
}
