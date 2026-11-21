//! Issue rubash#4045: an async (background) subshell respawn leaked
//! the __RUBASH_FD_* env-stamp carrier handle and the runtime's own
//! descriptors into the respawned child. `exec 3>real.txt;
//! ( ls /proc/self/fd ) &` showed `fd 12 -> real.txt` plus pipe fds
//! 254/255 where GNU bash 5.3 has exactly 0/1/2/3 in the child
//! (execute_cmd.c:1576 execute_in_subshell runs in the make_child
//! fork, which copies the parent's fd table wholesale and closes
//! only the redirect bitmap — execute_cmd.c:1709/:377).
//!
//! Root cause: the carrier arrived open at its stamped number and
//! stayed open after the table installed its own descriptor; the
//! process-inherited adoption then installed its number as a user
//! slot and the non-CLOEXEC handle rode every spawned child raw.
//! The fix restores each carrier into an executor-owned CLOEXEC
//! duplicate, retires the staging descriptors after the restore
//! loop, and skips unclaimed process-fd adoption for validated
//! internal respawns only when the stamp delivery provably covers
//! the whole live birth-fd set.
//!
//! Each assertion pins the GNU bash 5.3.20 behavior measured with
//! script files (target/issue-4045/probe*.sh).

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4045-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Spawn rubash with `args`, installing each `(target, src_fd)` pair
/// as an inherited descriptor in the child (dup2 onto `target` just
/// before exec). The caller keeps the source descriptors alive for
/// the duration of the call.
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

/// Snapshot the fds this test process can pass to `Command::new(RUBASH)`.
/// Cargo/CI harnesses sometimes hold perfectly legitimate inheritable
/// descriptors (the shape hardened in rubash#4054); those are user-inherited
/// fds under rubash#1557 and must be subtracted from fd-enumeration
/// expectations rather than misreported as respawn transport leaks.
fn snapshot_inheritable_fds() -> BTreeSet<u32> {
    let mut fds = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        fds.extend([0, 1, 2]);
        return fds;
    };
    for entry in entries.flatten() {
        let Some(fd) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        // Only fds without FD_CLOEXEC survive Command::spawn. This also
        // excludes read_dir's own directory fd and cloexec fds opened by
        // other test threads.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags != -1 && (flags & libc::FD_CLOEXEC) == 0 && fd >= 0 {
            fds.insert(fd as u32);
        }
    }
    fds
}

/// Assert the exact fd set listed by a child `ls /proc/self/fd`: the
/// inheritable birth set, the standard descriptors, the explicit user
/// bindings this test installed, and ls's own lowest-free opendir fd.
/// Anything beyond that is respawn/runtime residue; anything missing is
/// a delivery regression.
fn assert_listed_fd_set(
    stdout: &str,
    stderr: &str,
    inheritable: &BTreeSet<u32>,
    bound_slots: &[u32],
) {
    assert_eq!(stderr, "");
    let actual: BTreeSet<u32> = listed_fds(stdout).into_iter().collect();
    let mut expected = inheritable.clone();
    expected.extend([0, 1, 2]);
    expected.extend(bound_slots.iter().copied());
    let glob_fd = (0u32..)
        .find(|fd| !expected.contains(fd))
        .expect("find lowest free fd for ls opendir");
    expected.insert(glob_fd);
    assert_eq!(
        actual, expected,
        "child fd set mismatch (inheritable={inheritable:?}, bound_slots={bound_slots:?}, stdout={stdout:?})"
    );
}

/// Sorted fd numbers one `ls /proc/self/fd` reports.
fn listed_fds(stdout: &str) -> Vec<u32> {
    let mut fds: Vec<u32> = stdout
        .lines()
        .map(|line| line.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .expect("ls /proc/self/fd output is fd numbers");
    fds.sort_unstable();
    fds
}

#[test]
fn issue4045_async_subshell_external_child_sees_only_user_fds() {
    // The issue's reproducer: `exec 3>real.txt; ( ls /proc/self/fd ) &`.
    // GNU 5.3.20: 0 1 2 3 4 (4 is ls's own directory fd; no carrier).
    let dir = tempdir("asyncls");
    let inheritable = snapshot_inheritable_fds();
    let out = dir.join("real.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    // `exec 3>real.txt` resolves in the child's cwd — cd into the
    // tempdir first so the file exists at a literal name.
    let script = format!(
        "cd {:?}; exec 3>real.txt; ( ls /proc/self/fd | sort -n ) & wait",
        dir
    );
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    // GNU has no descriptor beyond the inherited user table in the
    // async child — the leaked carrier sat at 12, the signal self-pipe
    // at 254/255. Harness-owned birth fds are part of that user table.
    assert_listed_fd_set(&run.stdout, &run.stderr, &inheritable, &[3]);
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4045_foreground_subshell_fd_table_is_control() {
    // The foreground (non-respawn) subshell never leaked; pin it as
    // the control so a future regression in EITHER branch is caught.
    let dir = tempdir("fgls");
    let inheritable = snapshot_inheritable_fds();
    let out = dir.join("real.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let script = format!(
        "cd {:?}; exec 3>real.txt; ( ls /proc/self/fd | sort -n )",
        dir
    );
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_listed_fd_set(&run.stdout, &run.stderr, &inheritable, &[3]);
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4045_async_subshell_delivered_fd3_still_writes() {
    // Retiring the carrier must not break delivery: fd 3 in the
    // respawned child writes real.txt just like GNU's forked child.
    let dir = tempdir("asyncwrite");
    let out = dir.join("real.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let script = format!("cd {:?}; exec 3>real.txt; ( echo ASYNC >&3 ) & wait", dir);
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    drop(file);
    assert_eq!(file_contents(&out), "ASYNC\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4045_async_subshell_no_spurious_adopted_slots() {
    // The early inherited snapshot also manufactured user slots for
    // runtime plumbing (the signal self-pipe appeared as 254/255).
    // GNU's forked child has no such slots: `>&200` is Bad file
    // descriptor.
    let dir = tempdir("nospurious");
    let inheritable = snapshot_inheritable_fds();
    let out = dir.join("real.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    // A harness may legitimately own fd 200 (the #4054 shape), so probe
    // the first high fd outside the birth set. Runtime plumbing must still
    // never manufacture a user slot there.
    let probe_fd = (200u32..)
        .find(|fd| !inheritable.contains(fd))
        .expect("find unclaimed high probe fd");
    let script = format!(
        "cd {:?}; exec 3>real.txt; ( echo X >&{probe_fd}; echo rc=$? ) 2>/dev/null & wait",
        dir
    );
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1\n");
    drop(file);
    assert_eq!(file_contents(&out), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4045_input_procsub_respawn_retires_pin_carrier() {
    // The pin-transport respawn (`<(...)` body) has the same leak
    // shape: its pin pass installs the carrier at the slot but
    // leaves it open at the stamped number (a rubash-child pin pass
    // keeps the carrier as the restore fallback). GNU: the procsub
    // child's `ls` sees 0 1 2 3 + its own dir fd — no carrier.
    let dir = tempdir("procsub");
    let inheritable = snapshot_inheritable_fds();
    let out = dir.join("real.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let script = format!(
        "cd {:?}; exec 3>real.txt; cat <( ls /proc/self/fd | sort -n )",
        dir
    );
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_listed_fd_set(&run.stdout, &run.stderr, &inheritable, &[3]);
    drop(file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue4045_nested_async_and_second_bound_fd() {
    // A second bound fd travels too, and a nested async respawn sees
    // the same clean table: GNU shows 0 1 2 3 4 5 (5 = ls's dir fd).
    let dir = tempdir("nested");
    let inheritable = snapshot_inheritable_fds();
    let out = dir.join("real.txt");
    let out2 = dir.join("real2.txt");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out)
        .expect("open out file");
    let file2 = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out2)
        .expect("open out2 file");
    let script = format!(
        "cd {:?}; exec 3>real.txt; exec 4>real2.txt; ( ( ls /proc/self/fd | sort -n ) & wait ) & wait",
        dir
    );
    let run = rubash_c_with_fds(&script, &[]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_listed_fd_set(&run.stdout, &run.stderr, &inheritable, &[3, 4]);
    drop(file);
    drop(file2);
    let _ = std::fs::remove_dir_all(&dir);
}
