//! Issue rubash#1974: the special-path registry classified
//! `/proc/<pid>/fd/N` as a plain filesystem path even when <pid> was
//! the shell's own pid — GNU resolves `/proc/$$/fd/3` through procfs
//! exactly like `/proc/self/fd/3` (the kernel serves the same
//! descriptor directory under both names). Only the cat-operand face
//! (dev_operand_fd) carried a private pid rewrite; the redirect and
//! test faces classified the spelling as Filesystem. The registry's
//! `fd_alias_target` now judges the own-pid spelling itself, and every
//! face that asks the registry answers both spellings alike.
//!
//! GNU 5.3.20 oracle (script-file probes): with fd 3 open on a file,
//! `cat /proc/$$/fd/3`, `cat < /proc/$$/fd/3`, `test -r /proc/$$/fd/3`
//! and command substitution all serve the descriptor. Foreign pids
//! are NOT aliases of the shell's table (documented boundary): on the
//! Windows simulation they have no server and fail loudly.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1974-{tag}-{}", std::process::id()));
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

fn seed_file(dir: &Path) {
    let mut file = std::fs::File::create(dir.join("in.txt")).expect("seed file");
    file.write_all(b"pidfd-content-42\n").expect("seed write");
}

#[test]
fn issue1974_own_pid_fd_operand_reads_like_self() {
    let dir = tempdir("operand");
    seed_file(&dir);
    for spelling in ["/proc/$$/fd/3", "/proc/self/fd/3"] {
        let run = rubash_c(
            &format!("exec 3<in.txt; cat {spelling}"),
            &dir,
        );
        assert_eq!(run.code, Some(0), "{spelling}: stderr={}", run.stderr);
        assert_eq!(run.stdout, "pidfd-content-42\n", "{spelling}");
    }
}

#[test]
fn issue1974_own_pid_fd_redirect_and_comsub_serve() {
    let dir = tempdir("redirect");
    seed_file(&dir);
    // Separate runs per face: both faces dup the same open file
    // description, so they share fd 3's offset.
    let run = rubash_c("exec 3<in.txt; read -r line < /proc/$$/fd/3; echo \"read:$line\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("read:pidfd-content-42"),
        "stdout={}",
        run.stdout
    );
    let run = rubash_c("exec 3<in.txt; x=$(cat /proc/$$/fd/3); echo \"comsub:$x\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("comsub:pidfd-content-42"),
        "stdout={}",
        run.stdout
    );
}

#[test]
fn issue1974_own_pid_fd_pipe_test_face_matches_self() {
    let dir = tempdir("pipetest");
    // A process-substitution fd is pipe-marked; the test face judges
    // the spelling through the registry, so both spellings agree.
    let run = rubash_c(
        "exec 3< <(echo hi); \
         if test -p /proc/$$/fd/3; then echo pid-pipe; fi; \
         if test -p /proc/self/fd/3; then echo self-pipe; fi",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("pid-pipe"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("self-pipe"), "stdout={}", run.stdout);
}

#[cfg(windows)]
#[test]
fn issue1974_foreign_pid_fd_is_not_an_alias() {
    let dir = tempdir("foreign");
    seed_file(&dir);
    // Pid 1 is never the shell under test; its fd directory is not an
    // alias of the shell's table, and the Windows simulation has no
    // other server for it — the open fails loudly (ENOENT-shaped)
    // instead of serving the shell's own fd 3.
    let run = rubash_c("exec 3<in.txt; cat /proc/1/fd/3", &dir);
    assert_ne!(run.code, Some(0), "stdout={}", run.stdout);
    assert!(!run.stdout.contains("pidfd-content-42"));
}
