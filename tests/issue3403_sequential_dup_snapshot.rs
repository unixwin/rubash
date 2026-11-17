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
//! files. Cell classes:
//!
//! * `_external` — the captured-stderr stage is an external child (the
//!   rubash binary re-entered with `-c`), so these RUN ON BOTH
//!   PLATFORMS and are the discriminating cells: the fold only exists
//!   on the captured-stderr stage paths, and an unpatched tree sends
//!   the child's diagnostic into `out.txt`.
//! * `_builtin` — controls. GNU's dup2 model is the same, but a builtin
//!   stage's diagnostic bypasses the captured-stderr accumulator on
//!   every tree, so these are green before and after the fix; they
//!   pin the shape without claiming discrimination.
//! * the `ls` cells run on unix only (they need a unix `ls`).
//!
//! Every path interpolated into a script is single-quoted: an unquoted
//! Windows temp path let the parser read `\U...` as an escape and `:`
//! as a marker, so the redirect target became a different (PUA) file
//! name and the cell failed for the wrong reason.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
const LS_ERROR: &str = "cannot access '/bad'";
const CD_ERROR: &str = "/bad";

fn rubash_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

/// Single-quote a path for the shell under test. `sh_single_quote`
/// shape (`parse.y:5305 read_token_word` keeps a single-quoted run
/// literal), so backslashes and `:` reach the redirect word verbatim.
fn sq(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The external stage used by the platform-independent cells: the shell
/// under test re-entered on a failing `cd`. Any external child owns the
/// captured-stderr stage path, which is where the fold lives.
fn cd_stage() -> String {
    format!("{} -c 'cd {}'", sq(Path::new(rubash_bin())), CD_ERROR)
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

fn read_back(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// The issue shape with an external stage (discriminating): the child's
/// diagnostic belongs on stdout (the dup-time snapshot of fd 1),
/// `out.txt` stays empty, `done` follows on stdout.
#[test]
fn sequential_dup_snapshot_external() {
    let dir = test_dir("snapshot-external");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | {}; }} 2>&1 1>{}\necho done\n",
            cd_stage(),
            sq(&out)
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(CD_ERROR),
        "diagnostic must reach stdout (dup-time snapshot): {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert_eq!(read_back(&dir, "out.txt"), "", "out.txt must stay empty");
    cleanup(&dir);
}

/// Mirror order with an external stage (control): `1>out.txt 2>&1`
/// dups fd 2 from the file, so the diagnostic belongs in `out.txt` and
/// stdout shows only `done`.
#[test]
fn sequential_dup_mirror_order_external() {
    let dir = test_dir("mirror-external");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ echo hi | {}; }} 1>{} 2>&1\necho done\n",
            cd_stage(),
            sq(&out)
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert!(
        read_back(&dir, "out.txt").contains(CD_ERROR),
        "out.txt must hold the diagnostic in mirror order: {}",
        read_back(&dir, "out.txt")
    );
    cleanup(&dir);
}

/// Body-level retarget with an external stage (discriminating): an
/// `exec > file` inside the body rebinds fd 1 after the group entry
/// dup, so the diagnostic still belongs on stdout (redir.c:237 list
/// order). GNU's bare `exec` redirections persist, so the trailing
/// `echo done` lands in the exec target too.
#[test]
fn sequential_dup_exec_retarget_external() {
    let dir = test_dir("exec-retarget-external");
    let out = dir.join("out2.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!(
            "{{ exec > {}; echo hi | {}; }} 2>&1\necho done\n",
            sq(&out),
            cd_stage()
        ),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(CD_ERROR),
        "diagnostic must reach stdout past a body exec retarget: {stdout:?}"
    );
    assert!(
        !stdout.contains("done"),
        "done must follow the exec target, not stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert_eq!(
        read_back(&dir, "out2.txt"),
        "done\n",
        "exec target must hold only done (the pipeline stage writes nothing to stdout)"
    );
    cleanup(&dir);
}

/// No-retarget guard (rubash#1552, control): with no later `1>file`
/// the fold still merges the stage diagnostic into the pipeline output
/// stream — both land on the process stdout.
#[test]
fn sequential_dup_no_retarget_external() {
    let dir = test_dir("no-retarget-external");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!("{{ echo hi | {}; }} 2>&1\necho done\n", cd_stage()),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(CD_ERROR),
        "diagnostic must reach stdout without a retarget: {stdout:?}"
    );
    assert!(
        stdout.contains("done\n"),
        "done must follow on stdout: {stdout:?}"
    );
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    cleanup(&dir);
}

/// Builtin-stage control of the issue shape: a failing `cd` builtin's
/// diagnostic follows the same GNU fd model (green on every tree — the
/// builtin path never accumulates captured stderr, so this cell pins
/// the shape but does not discriminate the fold).
#[test]
fn sequential_dup_snapshot_builtin() {
    let dir = test_dir("snapshot-builtin");
    let out = dir.join("out.txt");
    let (stdout, stderr, code) = rubash_file(
        &dir,
        &format!("{{ echo hi | cd /bad; }} 2>&1 1>{}\necho done\n", sq(&out)),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(CD_ERROR),
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
        &format!("{{ echo hi | cd /bad; }} 1>{} 2>&1\necho done\n", sq(&out)),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "diagnostic leaked to the raw process stderr");
    assert!(
        read_back(&dir, "out.txt").contains(CD_ERROR),
        "out.txt must hold the cd diagnostic in mirror order: {}",
        read_back(&dir, "out.txt")
    );
    cleanup(&dir);
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
        &format!("{{ echo hi | ls /bad; }} 2>&1 1>{}\necho done\n", sq(&out)),
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
        &format!("{{ echo hi | ls /bad; }} 1>{} 2>&1\necho done\n", sq(&out)),
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
        &format!("{{ ls /bad | cat; }} 2>&1 1>{}\necho done\n", sq(&out)),
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
            sq(&out2)
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
