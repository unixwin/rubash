//! Issue rubash#1557: descriptors inherited open from the parent
//! process (fd >= 3 — CI log channels, socket activation, a parent
//! shell's `3>file`) were unusable: every reference reported
//! `3: Bad file descriptor` even though the descriptor was open in
//! the process. GNU keeps no fd registry to consult — redir.c
//! validates a dup source against the kernel itself
//! (`fcntl (redirector, F_GETFD, 0) != -1`, redir.c:1160) — so an
//! inherited descriptor is simply usable. Rubash's virtual fd table
//! must be seeded from kernel state at Executor::new: each open
//! fd >= 3 becomes a File endpoint over its own raw descriptor,
//! with read/write arms following the F_GETFL access mode.
//!
//! Every test installs REAL inherited descriptors into the spawned
//! rubash process (dup2 in pre_exec, which clears CLOEXEC so the
//! descriptors survive the exec exactly as a parent's would) and
//! asserts the GNU bash 5.3.20 behavior measured with script files.

#![cfg(unix)]

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1557-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Spawn rubash with `args`, installing each `(target, src_fd)` pair
/// as an inherited descriptor in the child (dup2 onto `target` just
/// before exec — the descriptor a parent process would have handed
/// the shell). The caller keeps the source descriptors alive for the
/// duration of the call.
fn rubash_with_fds(args: &[&str], fds: &[(u32, i32)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let raw: Vec<(i32, i32)> = fds
        .iter()
        .map(|(target, src)| (*target as i32, *src))
        .collect();
    unsafe {
        cmd.pre_exec(move || {
            for &(target, src) in &raw {
                if src == target {
                    // The source already sits on the target slot, but
                    // a std File carries O_CLOEXEC and would die at
                    // exec — clear the flag so the descriptor is
                    // genuinely inherited. (os_pipe descriptors are
                    // born inheritable; Files are not.)
                    if libc::fcntl(src, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else if libc::dup2(src, target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let output = cmd.output().expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn rubash_c_with_fds(script: &str, fds: &[(u32, i32)]) -> Run {
    rubash_with_fds(&["-c", script], fds)
}

fn file_contents(path: &Path) -> String {
    let mut text = String::new();
    File::open(path)
        .expect("open result file")
        .read_to_string(&mut text)
        .expect("read result file");
    text
}

fn seed_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write seed file");
    path
}

#[test]
fn issue1557_inherited_fd_write_to_file() {
    // The issue's first form: `rubash -c 'echo INHERITED >&3' 3>file`.
    let dir = tempdir("write");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds("echo INHERITED >&3", &[(3, file.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stderr, "");
    drop(file);
    assert_eq!(file_contents(&out), "INHERITED\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_write_to_pipe() {
    // The issue's second form: fd 3 is the write end of a pipe
    // (`3>&1 | cat` in a pipeline) — bytes must land in the pipe.
    let (mut reader, writer) = os_pipe::pipe().expect("os_pipe");
    let run = rubash_c_with_fds("echo PIPEFORM >&3", &[(3, writer.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(writer);
    let mut text = String::new();
    reader.read_to_string(&mut text).expect("read pipe");
    assert_eq!(text, "PIPEFORM\n");
}

#[test]
fn issue1557_inherited_fd_read() {
    let dir = tempdir("read");
    let seed = seed_file(&dir, "seed.txt", "seed-line\n");
    let file = File::open(&seed).expect("open seed");
    let run = rubash_c_with_fds(
        "IFS= read -r x <&3; echo \"got:$x\"",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "got:seed-line\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_external_cat_read() {
    let dir = tempdir("extcat");
    let seed = seed_file(&dir, "seed.txt", "seed-line\n");
    let file = File::open(&seed).expect("open seed");
    let run = rubash_c_with_fds("/bin/cat <&3", &[(3, file.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "seed-line\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_readwrite_both_directions() {
    // O_RDWR inheritance (3<>file): one open file description serves
    // both arms, offsets shared.
    let dir = tempdir("rdwr");
    let path = seed_file(&dir, "rw.txt", "rdwr-line\n");
    let read_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open rw file");
    let run = rubash_c_with_fds(
        "IFS= read -r x <&3; echo \"got:$x\"",
        &[(3, read_file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "got:rdwr-line\n");
    drop(read_file);
    let write_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("reopen rw file");
    let run = rubash_c_with_fds("echo WROTE >&3", &[(3, write_file.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(write_file);
    // "WROTE\n" (6 bytes) at offset 0 overwrites "rdwr-l", leaving
    // "ine\n" — byte-identical to GNU's result for the same form.
    assert_eq!(file_contents(&path), "WROTE\nine\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_subshell_and_function() {
    let dir = tempdir("subfn");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "( echo SUBSHELL >&3 ); f() { echo FUNC >&3; }; f",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file);
    assert_eq!(file_contents(&out), "SUBSHELL\nFUNC\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_comsub_body() {
    let dir = tempdir("comsub");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "x=$(echo COMSUB >&3; echo out); echo \"x=$x\"",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "x=out\n");
    drop(file);
    assert_eq!(file_contents(&out), "COMSUB\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_procsub_body() {
    // The issue's third form: the redirect rides the assignment
    // command itself; the procsub body writes to the inherited fd.
    // (The assigned VALUE of a bare whole-RHS >(…) was rubash#1561,
    // pinned by issue1557_inherited_fd_procsub_value below.)
    let dir = tempdir("procsub");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "v=>(echo BODY >&3); sleep 0.5; echo done",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    assert_eq!(run.stderr, "");
    drop(file);
    assert_eq!(file_contents(&out), "BODY\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_procsub_value() {
    // GNU: v becomes the substitution's /dev/fd path. The value
    // materialization gap was rubash#1561 (the assignment channels
    // spliced by the re-printed target and missed verbatim spans
    // whose bodies re-print differently, like this `>&3` body);
    // fixed there, so this pin now runs un-ignored.
    let dir = tempdir("procsubval");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "v=>(echo BODY >&3); sleep 0.5; echo \"v=$v\"",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("v=/dev/fd/"),
        "stdout: {}",
        run.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_dup_snapshot_survives_close() {
    // dup shares the open file description: after `exec 4>&3` the
    // snapshot on fd 4 keeps writing when fd 3 itself is closed.
    let dir = tempdir("dupsnap");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "exec 4>&3; exec 3>&-; echo SNAP >&4",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file);
    assert_eq!(file_contents(&out), "SNAP\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_close_then_use_fails() {
    // `exec 3>&-` really closes the descriptor (GNU closes the kernel
    // fd); a later reference is the ordinary Bad file descriptor.
    let dir = tempdir("closeuse");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_c_with_fds(
        "exec 3>&-; echo X >&3; echo rc=$?",
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("3: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    drop(file);
    assert_eq!(file_contents(&out), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_sparse_and_high_numbers() {
    // Sparse inheritance (only fd 5 open, 4 closed) and a high
    // descriptor number both work — validity is kernel state, not a
    // contiguous range.
    let dir = tempdir("sparse");
    let out5 = dir.join("five.txt");
    let file5 = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out5)
        .expect("open five file");
    let run = rubash_c_with_fds("echo FIVE >&5", &[(5, file5.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file5);
    assert_eq!(file_contents(&out5), "FIVE\n");
    let out200 = dir.join("high.txt");
    let file200 = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out200)
        .expect("open high file");
    let run = rubash_c_with_fds("echo HIGH >&200", &[(200, file200.as_raw_fd())]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file200);
    assert_eq!(file_contents(&out200), "HIGH\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_wrong_direction_fails_like_gnu() {
    // Direction follows the F_GETFL access mode: writing to a
    // read-only inherited descriptor fails at the write (rc 1, write
    // error), reading from a write-only one fails the read — the same
    // outcomes shell-opened descriptors produce.
    let dir = tempdir("wrongdir");
    let seed = seed_file(&dir, "seed.txt", "seed-line\n");
    let ro = File::open(&seed).expect("open seed ro");
    let run = rubash_c_with_fds("echo X >&3; echo rc=$?", &[(3, ro.as_raw_fd())]);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    let out = dir.join("wo.txt");
    let wo = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open wo file");
    let run = rubash_c_with_fds("IFS= read -r x <&3; echo rc=$?", &[(3, wo.as_raw_fd())]);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1557_inherited_fd_script_file_form() {
    // Script-file invocation (not -c): the descriptor handed to the
    // interpreter process is inherited the same way.
    let dir = tempdir("script");
    let script = seed_file(&dir, "s.sh", "echo SCRIPT >&3\n");
    let out = dir.join("out.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let run = rubash_with_fds(
        &[script.to_str().expect("script path")],
        &[(3, file.as_raw_fd())],
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file);
    assert_eq!(file_contents(&out), "SCRIPT\n");
    let _ = std::fs::remove_dir_all(&dir);
}
