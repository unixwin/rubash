//! rubash#3640 — the session cwd anchor must not occupy a low user fd.
//!
//! GNU keeps its persistent internal descriptors out of the low range
//! with `move_to_high_fd` (general.c:702, ceiling HIGH_FD_MAX=256,
//! general.h:257) and binds a redirect's file onto its redirector fd
//! with dup2 (redir.c:1017/:1120), so `exec 3>file` binds the file to
//! fd 3 itself whatever occupied the number before. Rubash's cwd anchor
//! (`ShellState::cwd_anchor`, the open directory handle that lets a
//! deleted cwd keep spawning and serving reads — rubash#3198/#3268)
//! used to take the lowest free kernel fd, which is 3 in every
//! ordinary session: `readlink /proc/$$/fd/3` named the cwd and
//! `exec 3>out.txt` landed the file on fd 4 while fd 3 still read as
//! the directory.
//!
//! The fix relocates the anchor with a `move_to_high_fd` port the
//! moment it is opened (src/fd/unix.rs `move_file_to_high_fd`,
//! applied at the two CwdAnchor construction sites: the birth capture
//! in executor/init.rs and the cd/refresh funnel in
//! executor/external_inner.rs `refresh_cwd_anchor_for`). These pins
//! assert the observable contract on /proc — unix-only because the
//! whole check is the kernel fd table.
//!
//! Oracle: WSL GNU bash 5.3.20(1)-release, script-file probes.
//! Seed check 3 of the issue (`ls /proc/$$/fd | wc -l` == 4) is NOT
//! pinned here: the count also absorbs rubash's transient spawn
//! socket/pipe fds (visible in the parent while a pipeline sibling
//! is mid-spawn — a separate spawn-lifecycle matter) and the anchor
//! itself, which has no GNU -c counterpart. The deterministic
//! invariants below are the ones this fix owns.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3640-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Spawn rubash on `script` inside `dir`; returns merged output and
/// the exit status. `-c` mode mirrors the issue seed's fresh
/// top-level invocation per check.
fn rubash_c(dir: &Path, script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
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

fn stdout_trimmed(run: &Run) -> &str {
    run.stdout.trim()
}

/// Seed check 1 (rewritten, rubash#3917): no low descriptor names the
/// startup cwd. The original form — `t=$(readlink /proc/$$/fd/3)`
/// classified as a pipe — was vacuous: a free fd 3 makes the probe's
/// own command-substitution capture pipe take (3,4), so `readlink`
/// saw the probe's pipe, not the shell's startup state (and the
/// #3917 capture-pipe relocation unmasks it: the probe's pipe now
/// sits high, so the old form would read an empty target and fail).
/// The rewrite probes the real #3640 invariant (the cwd anchor must
/// not sit on a low fd) without a command substitution: each of
/// fds 3..9 is readlink'd into a scratch file (an external command
/// with a file redirect — no capture pipe), and the file must not
/// contain the cwd path.
#[cfg(unix)]
#[test]
fn issue3640_startup_no_low_fd_is_cwd() {
    let dir = scratch("fd3-startup");
    let run = rubash_c(
        &dir.0,
        "for f in /proc/$$/fd/3 /proc/$$/fd/4 /proc/$$/fd/5 /proc/$$/fd/6 /proc/$$/fd/7 /proc/$$/fd/8 /proc/$$/fd/9; do readlink \"$f\" >>probe.txt 2>/dev/null; done; if grep -qxF \"$PWD\" probe.txt 2>/dev/null; then echo anchor-on-low; else echo anchor-not-on-low; fi",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "anchor-not-on-low",
        "stderr: {}",
        run.stderr
    );
}

/// Seed check 2: `exec 3>file` binds the file to fd 3 itself.
#[cfg(unix)]
#[test]
fn issue3640_exec_3_file_lands_on_fd3() {
    let dir = scratch("exec3");
    let run = rubash_c(
        &dir.0,
        "exec 3>out.txt; [ \"$(readlink /proc/$$/fd/3)\" = \"$PWD/out.txt\" ] && echo fd3-is-file || { echo fd3-not-file; readlink /proc/$$/fd/3; readlink /proc/$$/fd/4; }",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "fd3-is-file",
        "stderr: {}",
        run.stderr
    );
}

/// Seed check 4 (control): `exec 4>file` still lands on fd 4.
#[cfg(unix)]
#[test]
fn issue3640_exec_4_file_lands_on_fd4_control() {
    let dir = scratch("exec4");
    let run = rubash_c(
        &dir.0,
        "exec 4>out2.txt; [ \"$(readlink /proc/$$/fd/4)\" = \"$PWD/out2.txt\" ] && echo fd4-is-file || echo fd4-not-file",
    );
    assert_eq!(
        stdout_trimmed(&run),
        "fd4-is-file",
        "stderr: {}",
        run.stderr
    );
}

/// Seed check 5 (control): the shell's logical fd 3 still reads —
/// `read -u 3` reaches the file bound by `exec 3<file`.
#[cfg(unix)]
#[test]
fn issue3640_parent_read_via_fd3_control() {
    let dir = scratch("read3");
    std::fs::write(dir.0.join("in.txt"), "data\n").expect("seed in.txt");
    let run = rubash_c(
        &dir.0,
        "exec 3<in.txt; read -u 3 x; [ \"$x\" = data ] && echo read3-ok || echo read3-bad",
    );
    assert_eq!(stdout_trimmed(&run), "read3-ok", "stderr: {}", run.stderr);
}

/// Seed check 6 (control): a spawned child inherits the rebound fd 3
/// and writes through it (`cat >&3` is an external command, so this
/// exercises the spawn fd pin pass, not just the in-process table).
#[cfg(unix)]
#[test]
fn issue3640_child_writes_via_fd3_control() {
    let dir = scratch("child3");
    let run = rubash_c(
        &dir.0,
        "exec 3>out3.txt; printf 'hi\\n' | cat >&3; [ \"$(cat out3.txt)\" = hi ] && echo child-write-ok || { echo child-write-bad; cat out3.txt; }",
    );
    assert_eq!(
        stdout_trimmed(&run),
        "child-write-ok",
        "stderr: {}",
        run.stderr
    );
}

/// The relocated anchor must stay OUT of the low user range: after a
/// `cd` (the refresh funnel) no descriptor 3-9 may name the session
/// cwd, and the anchor itself must sit on a high descriptor (>= 10).
/// Before the fix fd 3 itself was the cwd directory.
#[cfg(unix)]
#[test]
fn issue3640_cwd_anchor_never_on_low_fds() {
    let dir = scratch("lowfd");
    let sub = dir.0.join("sub");
    std::fs::create_dir_all(&sub).expect("create sub");
    let script = format!(
        "cd '{}' && bad= && high= && for p in /proc/$$/fd/*; do n=${{p##*/}}; t=$(readlink \"$p\" 2>/dev/null); if [ \"$t\" = \"$PWD\" ]; then if [ \"$n\" -lt 10 ] 2>/dev/null; then bad=\"$bad $n\"; else high=\"$high $n\"; fi; fi; done; [ -z \"$bad\" ] && [ -n \"$high\" ] && echo high-cwd-fd || echo bad:\"$bad\"-high:\"$high\"",
        sub.display()
    );
    let run = rubash_c(&dir.0, &script);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "high-cwd-fd",
        "stderr: {}",
        run.stderr
    );
}

/// The anchor still does its job after relocation: a cwd deleted
/// under the session keeps spawning external commands without failing
/// (the rubash#3198/#3268 surface this fd exists for).
#[cfg(unix)]
#[test]
fn issue3640_anchor_survives_cwd_deletion() {
    let dir = scratch("deadcwd");
    let sub = dir.0.join("gone");
    std::fs::create_dir_all(&sub).expect("create sub");
    let script = format!(
        "cd '{}' && /bin/rm -rf '{}' && /bin/true; echo rc=$?",
        sub.display(),
        sub.display()
    );
    let run = rubash_c(&dir.0, &script);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "rc=0",
        "stdout={} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// Non-unix hosts have no /proc fd table to inspect; the whole pin
/// set is cfg(unix). Keep an explicit marker so the file still
/// compiles to an empty test binary elsewhere.
#[cfg(not(unix))]
#[test]
fn issue3640_unix_only() {}
