//! rubash#3403 — sequential redirect loop resolves `2>&1` against the
//! live fd 1 target instead of the dup-time snapshot.
//!
//! GNU applies a redirect list left to right (redir.c:237
//! do_redirections; redir.c:1192-1193 `2>&1 means dup2 (1, 2)`), so in
//! `{ ...; } 2>&1 1>out.txt` fd 2 is dup'd from fd 1 as it stands at the
//! moment of the dup (the terminal/pipe) and the later `1>out.txt` does
//! not retroactively move it. The sequential pipeline driver folded a
//! stage's stderr into the pipeline's output stream whenever the fd
//! table held fd 2 on the Stdout endpoint — without checking whether
//! the pipeline's own fd 1 was still that same object. A later
//! `1>out.txt` (or an `exec > file` inside the body) had already
//! retargeted fd 1, so the stage error landed in the file instead of
//! on stdout.
//!
//! Expectations below are the GNU 5.3.20 oracle's, probed from script
//! files. The `ls` cells run on unix only (they need a unix `ls`); the
//! `cd` cells are platform-independent and exercise the same fold
//! through a builtin stage's captured stderr.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const LS_ERROR: &str = "cannot access '/bad'";

fn rubash_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

fn rubash_file(dir: &PathBuf, script: &str) -> (String, String, Option<i32>) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script file");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3403-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn read_back(dir: &PathBuf, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// The issue shape: the `ls` error belongs on stdout (the dup-time
/// snapshot of fd 1), `out.txt` stays empty, `done` follows on stdout.
#[test]
#[cfg(unix)]
fn sequential_dup_snapshot_ls() {
    let dir = test_dir("snapshot-ls");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | ls /bad; }} 2>&1 1>{}\necho done\n",
            out.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "ls error must reach stdout (dup-time snapshot): {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "error leaked to the raw process stderr");
    assert_eq!(read_back(&dir, "out.txt"), "", "out.txt must stay empty");
    cleanup(&dir);
}

/// Mirror order (control): `1>out.txt 2>&1` dups fd 2 from the file, so
/// the error belongs in `out.txt` and stdout shows only `done`.
#[test]
#[cfg(unix)]
fn sequential_dup_mirror_order_ls() {
    let dir = test_dir("mirror-ls");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | ls /bad; }} 1>{} 2>&1\necho done\n",
            out.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "error leaked to the raw process stderr");
    assert!(
        read_back(&dir, "out.txt").contains(LS_ERROR),
        "out.txt must hold the ls error in mirror order: {}",
        read_back(&dir, "out.txt")
    );
    cleanup(&dir);
}

/// Non-final stage: the first stage's stderr is the ambient fold too —
/// `ls /bad | cat` under `2>&1 1>out.txt` still reports on stdout and
/// leaves `out.txt` empty.
#[test]
#[cfg(unix)]
fn sequential_dup_snapshot_nonfinal_ls() {
    let dir = test_dir("snapshot-nonfinal-ls");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!("{{ ls /bad | cat; }} 2>&1 1>{}\necho done\n", out.display()),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "non-final stage error must reach stdout: {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "error leaked to the raw process stderr");
    assert_eq!(read_back(&dir, "out.txt"), "", "out.txt must stay empty");
    cleanup(&dir);
}

/// No-retarget guard (rubash#1552): with no later `1>file`, the fold
/// still merges the stage error into the pipeline output stream —
///
/// both land on the process stdout, so stdout holds error + `done`.
#[test]
#[cfg(unix)]
fn sequential_dup_no_retarget_ls() {
    let dir = test_dir("no-retarget-ls");
    let (stdout, stderr, code) = rubash_file(&dir, "{ echo hi | ls /bad; } 2>&1\necho done\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "ls error must reach stdout without a retarget: {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "error leaked to the raw process stderr");
    cleanup(&dir);
}

/// Body-level retarget guard (rubash#170 nest4 class): an `exec > file`
/// inside the body rebinds fd 1 after the group entry dup, so the stage
/// error still belongs on stdout. GNU's bare `exec` redirections persist
/// in the shell, so the trailing `echo done` lands in the exec target
/// too — the cell asserts the error is NOT folded in with it.
#[test]
#[cfg(unix)]
fn sequential_dup_exec_retarget_ls() {
    let dir = test_dir("exec-retarget-ls");
    let out2 = dir.join("out2.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ exec > {}; echo hi | ls /bad; }} 2>&1\necho done\n",
            out2.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(LS_ERROR),
        "ls error must reach stdout past a body exec retarget: {stdout:?}"
    );
    assert!(
        !stdout.contains("done"),
        "done must follow the exec target, not stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "error leaked to the raw process stderr");
    assert_eq!(
        read_back(&dir, "out2.txt"),
        "done\n",
        "exec target must hold only done (ls writes nothing to stdout)"
    );
    cleanup(&dir);
}

/// Builtin-stage twin of the issue shape (platform-independent): a
/// failing `cd` stage's diagnostic follows the same fold.
#[test]
fn sequential_dup_snapshot_builtin() {
    let dir = test_dir("snapshot-builtin");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | cd /bad; }} 2>&1 1>{}\necho done\n",
            out.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("/bad"),
        "cd diagnostic must reach stdout (dup-time snapshot): {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert_eq!(read_back(&dir, "out.txt"), "", "out.txt must stay empty");
    cleanup(&dir);
}

/// Builtin-stage mirror order (control): `1>out.txt 2>&1` sends the
/// `cd` diagnostic into `out.txt`.
#[test]
fn sequential_dup_mirror_order_builtin() {
    let dir = test_dir("mirror-builtin");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | cd /bad; }} 1>{} 2>&1\necho done\n",
            out.display()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert!(
        read_back(&dir, "out.txt").contains("/bad"),
        "out.txt must hold the cd diagnostic in mirror order: {}",
        read_back(&dir, "out.txt")
    );
    cleanup(&dir);
}
