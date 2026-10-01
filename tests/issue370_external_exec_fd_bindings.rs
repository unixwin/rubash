//! rubash#370: fds copied persistently by `exec N>&M` must be honored by
//! EXTERNAL-command redirects (`cmd >&N`), matching GNU's fork-time dup2.
//!
//! GNU contract (C source is the specification):
//! - redir.c:1170 `do_redirection_internal`: `exec 3>&2` is
//!   `dup2 (2, 3)` — fd 3 becomes a real descriptor sharing fd 2's open
//!   file description; redir.c:1188-1192 keeps it open across exec (no
//!   close-on-exec for a user dup of an std fd).
//! - `execute_cmd.c` forks the disk-command child with those descriptors
//!   in place; the child's `dup2 (3, 1)` (`cmd >&3`) then lands its stdout
//!   on fd 3's object — stderr.
//!
//! Root cause on the rubash side (regression introduced by ba69f0d9,
//! verified good at its parent c70d109f with clean worktree builds):
//! the deleted ordered capture-replay family
//! (`command_needs_ordered_output_capture` -> `write_ordered_command_output`)
//! was the only consumer that resolved a `>&N` dup through the command's
//! ordered redirect state. Its replacements missed the class twice:
//! (1) the stdio planner's `resolve_dup_source` only consulted fd 1/2
//!     slots and the command's OWN fd>=3 redirects — never the ambient
//!     fd-table entries an `exec N>&M` leaves — so `>&3` declined;
//! (2) the legacy per-fd branch piped the child's stdout and drained it
//!     to fd 1's AMBIENT endpoint (the live stdout alias), so the bytes
//!     landed on the real stdout instead of fd 3's stderr object.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes (m370x matrix, e1-e10).

use std::{fs, path::Path, path::PathBuf, process::Command};

fn bin_dir() -> PathBuf {
    Path::new("target").join(format!("rubash-issue370-bin-{}", std::process::id()))
}

fn helper_path(dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        dir.join(format!("{name}.cmd"))
    }

    #[cfg(not(windows))]
    {
        dir.join(name)
    }
}

fn write_helper(path: &Path, body: &str) {
    #[cfg(windows)]
    {
        // cmd.exe echoes the space before a trailing `>&2` unless the
        // redirection hugs the word (same normalization the fd_redirects
        // fixture helper does).
        let body = body.replace(" >&2", ">&2");
        fs::write(path, format!("@echo off\r\n{body}\r\n")).unwrap();
    }

    #[cfg(not(windows))]
    {
        fs::write(path, format!("{body}\n")).unwrap();
        make_executable(path);
    }
}

#[cfg(not(windows))]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn stream_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Runs `script` with a PATH that resolves `emitout` (external child whose
/// stdout writes `marker`) and `emiterr` (writes to its stderr).
fn run(script: &str) -> (String, String) {
    let dir = bin_dir();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    write_helper(&helper_path(&dir, "emitout"), "echo external-via-fd");
    write_helper(
        &helper_path(&dir, "emiterr"),
        "echo external-error-via-fd >&2",
    );
    let path = std::env::join_paths([&dir, Path::new(".")]).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    let _ = fs::remove_dir_all(&dir);
    (stream_text(&output.stdout), stream_text(&output.stderr))
}

#[test]
fn external_stdout_through_exec_copied_stderr_fd() {
    // The #370 reproducer: fd 3 was dup'd from stderr by exec; the child's
    // `>&3` must land on stderr (redir.c:1170 dup2 semantics).
    let (stdout, stderr) = run("exec 3>&2; emitout >&3");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "external-via-fd\n");
}

#[test]
fn external_stderr_through_exec_copied_stdout_fd() {
    // Dual case: fd 3 copied from stdout carries the child's STDERR
    // (`2>&3`) onto stdout.
    let (stdout, stderr) = run("exec 3>&1; emiterr 2>&3");
    assert_eq!(stdout, "external-error-via-fd\n");
    assert_eq!(stderr, "");
}

#[test]
fn builtin_control_through_exec_copied_stderr_fd() {
    // Control: the BUILTIN writer keeps honoring fd 3's stderr binding
    // (this path did not regress; it must not regress back either).
    let (stdout, stderr) = run("exec 3>&2; echo builtin-via-fd >&3");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "builtin-via-fd\n");
}

#[test]
fn multiple_exec_copied_fds_route_independently() {
    // `exec 5>&2 6>&1` binds two persistent fds; each child redirect
    // resolves its OWN fd's object.
    let (stdout, stderr) = run("exec 5>&2 6>&1; emitout >&5; emitout >&6");
    assert_eq!(stdout, "external-via-fd\n");
    assert_eq!(stderr, "external-via-fd\n");
}

#[test]
fn chained_exec_dups_follow_the_chain() {
    // `exec 4>&3` dups fd 3's binding (itself stderr); `>&4` sees stderr.
    let (stdout, stderr) = run("exec 3>&2; exec 4>&3; emitout >&4");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "external-via-fd\n");
}

#[test]
fn exec_copied_fd_inside_command_substitution() {
    // Capture chain: inside $( ) fd 1 is the capture pipe, so `exec 3>&1`
    // binds fd 3 to THAT object and `>&3` feeds the substitution.
    let (stdout, stderr) = run("v=$(exec 3>&1; emitout >&3); echo \"captured=[$v]\"");
    assert_eq!(stdout, "captured=[external-via-fd]\n");
    assert_eq!(stderr, "");
}

#[test]
fn high_numbered_exec_copied_fd_and_close() {
    // fd 10 (past the dynamic-fd base) and an `exec 10>&-` afterwards —
    // the binding works until it is explicitly closed.
    let (stdout, stderr) = run("exec 10>&2; emitout >&10; exec 10>&-");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "external-via-fd\n");
}
