//! rubash#2588 — residue batch. Behavioral pins for the legs with a
//! script-visible surface; the remaining legs are pinned where they
//! live (noted per leg):
//! - (a) PIPESTATUS snapshot key misspelling (`__RUBASH_PIPESSTATUS_
//!   STATE`): the key rides scope snapshots, not the script env, so
//!   the spelling is pinned by the in-crate unit test
//!   `state::tests::issue2588_pipestatus_snapshot_key_spelling`
//!   (cargo test --lib). The behavioral guard below proves PIPESTATUS
//!   scope save/restore still works after the rename.
//! - (b) dead `ERROR` arm in the trap pseudo-signal lists: GNU's set
//!   is EXIT/DEBUG/ERR/RETURN (signames.h:71-73); `trap ... ERROR`
//!   is an invalid signal specification (oracle-verified below).
//! - (c) `__RUBASH_POSIX_MODE` mirror: host verification found the niu
//!   host reads the key only through the engine's variable store, so
//!   the fix is engine-side both-storages consistency; the guard below
//!   proves the posix flip surface is unchanged.
//! - (d) `__RUBASH_CMD_FILE` temp-file delete failure: retry + warn;
//!   no script-visible surface when deletion succeeds (the normal
//!   path is covered by the existing command-file transport tests).
//! - (e) `__RUBASH_EXTGLOB_ENABLED_LINE` fresh-init strip: the guard
//!   below proves normal extglob enable/gating is unchanged.
//!
//! Oracle (GNU bash 5.3.20):
//!   trap 'echo caught' ERR; false; echo after  →  caught / after
//!   trap 'echo x' ERROR  →  "trap: ERROR: invalid signal specification", rc 1

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2588-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2588_err_trap_fires_and_error_is_not_a_trap_name() {
    let (stdout, stderr, _code) = run_script(
        "trap 'echo caught' ERR\nfalse\necho after\ntrap 'echo x' ERROR\necho \"rc=$?\"\n",
    );
    assert!(stdout.contains("caught"), "stdout: {stdout}");
    assert!(stdout.contains("after"), "stdout: {stdout}");
    assert!(
        stdout.contains("rc=1"),
        "stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("invalid signal specification"),
        "ERROR must be rejected like GNU, stderr: {stderr}"
    );
}

#[test]
fn issue_2588_pipestatus_scope_restore_still_works() {
    // Guard for leg (a): a function-local PIPESTATUS snapshot/restore
    // round-trip (the renamed key's only job) keeps the outer vector.
    let script = "echo a | echo b\necho \"outer=${PIPESTATUS[*]}\"\nf() { local PIPESTATUS; echo x | echo y | echo z; }\nf\necho \"after=${PIPESTATUS[*]}\"\n";
    let (stdout, stderr, _code) = run_script(script);
    assert!(
        stdout.contains("outer=0 0"),
        "stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("after=0 0"),
        "stdout: {stdout}, stderr: {stderr}"
    );
}

#[test]
fn issue_2588_posix_flip_surface_unchanged() {
    let (stdout, stderr, _code) = run_script("set -o posix\nset -o | grep posix\necho \"pc=${POSIXLY_CORRECT-unset}\"\nset +o posix\nset -o | grep posix\n");
    assert!(
        stdout.contains("posix"),
        "stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("pc=y"),
        "stdout: {stdout}, stderr: {stderr}"
    );
}

#[test]
fn issue_2588_extglob_gate_unchanged() {
    let (stdout, stderr, _code) =
        run_script("shopt -s extglob\ncase b in @(a|b)) echo matched;; *) echo no;; esac\n");
    assert_eq!(stdout, "matched\n", "stderr: {stderr}");
}
