//! Issue rubash#1973: the special-path registry's `fd_alias_target`
//! accepted leading-zero fd spellings (`/dev/fd/00`, `/dev/fd/03`,
//! `/dev/fd/007`, `/proc/self/fd/00`) that GNU bash rejects on every
//! face. GNU 5.3.20 oracle (probe `oracle-fd-leading-zero.sh`, bash
//! 5.3.20(1)-release): cat → ENOENT even with the numbered fd genuinely
//! open, redirect → ENOENT, `test -r` / `[[ -e ]]` → false, `source` →
//! ENOENT — procfs serves only canonical names. The registry now
//! parses the numeric field as canonical decimal (no leading zeros, "0"
//! itself excepted; no sign, no whitespace), and the three faces the R3
//! pass stopped at converge on `fd_alias_target(..) == Some(0)`:
//! `virtual_device_test` (builtins/test.rs), `source` fd-0 (builtins/
//! source.rs) and the exec-redirect fd-0 arm (executor/trap_exec.rs).
//!
//! Assertions follow the GNU 5.3.20 behavior measured with the same
//! scripts; canonical spellings keep working.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1973-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Run rubash with `payload` as the process stdin (the fd-0 binding the
/// leading-zero spellings must NOT reach).
fn rubash_c_with_stdin(script: &str, dir: &Path, payload: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(payload.as_bytes())
        .expect("write stdin payload");
    let output = child.wait_with_output().expect("wait for rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// External-command operand face (dev_fd_operands → registry): a
/// leading-zero spelling with its numbered fd genuinely bound must not
/// materialize that fd's bytes — GNU cats ENOENT even with fd 3 open.
#[test]
fn issue1973_cat_leading_zero_does_not_read_bound_fd() {
    let dir = tempdir("cat");
    for (spelling, fd) in [
        ("/dev/fd/03", 3),
        ("/dev/fd/007", 7),
        ("/proc/self/fd/00", 0),
    ] {
        let script = format!("echo hi | cat {spelling} {fd}<<< hi");
        let run = rubash_c(&script, &dir);
        assert!(
            !run.stdout.contains("hi\n") && !run.stdout.ends_with("hi"),
            "cat {spelling} must not read bound fd {fd}: stdout {:?} stderr {:?}",
            run.stdout,
            run.stderr
        );
        assert_ne!(run.code, Some(0), "cat {spelling} must fail like GNU");
    }
}

/// Pipeline-stage face: `/dev/fd/00` must not alias the stage's fd 0.
#[test]
fn issue1973_pipeline_fd00_is_not_stdin_alias() {
    let dir = tempdir("pipe");
    let run = rubash_c("echo body | cat /dev/fd/00", &dir);
    assert_eq!(
        run.stdout, "",
        "cat /dev/fd/00 must not read the pipe (stderr: {})",
        run.stderr
    );
    assert_ne!(run.code, Some(0));
    // Control: the canonical spelling still aliases fd 0.
    let run = rubash_c("echo body | cat /dev/fd/0", &dir);
    assert_eq!(run.stdout, "body\n");
    assert_eq!(run.code, Some(0));
}

/// `source` face (builtins/source.rs fd-0 arm, now
/// `fd_alias_target == Some(0)`): leading-zero spellings fall through
/// to the filesystem and fail ENOENT instead of sourcing fd 0's bytes.
#[test]
fn issue1973_source_leading_zero_is_filesystem_enoent() {
    let dir = tempdir("source");
    for spelling in ["/dev/fd/00", "/proc/self/fd/00"] {
        let run = rubash_c_with_stdin(&format!(". {spelling}"), &dir, "echo SOURCED00\n");
        assert_eq!(
            run.stdout, "",
            ". {spelling} must not source the stdin payload"
        );
        assert_ne!(run.code, Some(0), ". {spelling} must fail like GNU");
    }
    // Controls: canonical spellings still source fd 0, in both the
    // here-string form and the process-stdin form.
    let run = rubash_c(". /dev/fd/0 <<< 'echo FD0SRC'", &dir);
    assert_eq!(run.stdout, "FD0SRC\n");
    assert_eq!(run.code, Some(0));
    let run = rubash_c(". /proc/self/fd/0 <<< 'echo PROCFD0SRC'", &dir);
    assert_eq!(run.stdout, "PROCFD0SRC\n");
    assert_eq!(run.code, Some(0));
    let run = rubash_c_with_stdin(". /dev/fd/0", &dir, "echo VIASTDIN\n");
    assert_eq!(run.stdout, "VIASTDIN\n");
    assert_eq!(run.code, Some(0));
}

/// exec redirect face (executor/trap_exec.rs fd-0 arm, now
/// `fd_alias_target == Some(0)`): `exec 0</dev/fd/00` must fail with
/// the GNU ENOENT diagnostic instead of silently duplicating fd 0.
/// GNU 5.3.20 (probed): the diagnostic prints and the script continues
/// (non-POSIX exec does not abort on a failed redirect).
#[test]
fn issue1973_exec_redirect_leading_zero_fails_enoent() {
    let dir = tempdir("exec");
    let run = rubash_c_with_stdin("exec 0</dev/fd/00; echo STILL_ALIVE", &dir, "payload\n");
    assert!(
        run.stderr.contains("/dev/fd/00: No such file or directory"),
        "exec 0</dev/fd/00 must fail ENOENT like GNU (stdout {:?} stderr {:?})",
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.stdout, "STILL_ALIVE\n",
        "GNU continues after the failed exec"
    );
    // Control: canonical spellings still bind fd 0.
    let run = rubash_c("exec 0</dev/null; echo EXECK", &dir);
    assert_eq!(run.stdout, "EXECK\n");
    assert_eq!(run.code, Some(0));
}

/// `test`/`[[` face (builtins/test.rs virtual_device_test, now registry
/// queries): leading-zero spellings fail the file tests; canonical
/// spellings keep the virtual-device answers.
#[test]
fn issue1973_test_leading_zero_is_false_canonical_true() {
    let dir = tempdir("test");
    for spelling in ["/dev/fd/00", "/dev/fd/03", "/proc/self/fd/00"] {
        let run = rubash_c(&format!("[[ -e {spelling} ]]; echo rc=$?"), &dir);
        assert_eq!(run.stdout, "rc=1\n", "[[ -e {spelling} ]] must be false");
        let run = rubash_c(&format!("test -r {spelling}; echo rc=$?"), &dir);
        assert_eq!(run.stdout, "rc=1\n", "test -r {spelling} must be false");
    }
    for spelling in ["/dev/fd/0", "/dev/stdin", "/proc/self/fd/0"] {
        let run = rubash_c(&format!("[[ -e {spelling} ]]; echo rc=$?"), &dir);
        assert_eq!(run.stdout, "rc=0\n", "[[ -e {spelling} ]] must stay true");
    }
}
