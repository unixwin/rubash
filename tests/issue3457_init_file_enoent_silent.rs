//! rubash#3457 — a missing `--init-file` / `--rcfile` was diagnosed.
//! GNU sources startup files through builtins/evalfile.c:339
//! maybe_execute_file, whose FEVAL_ENOENTOK flag makes ENOENT silent
//! (evalfile.c:120 `if (((flags & FEVAL_ENOENTOK) == 0) || errno !=
//! ENOENT) file_error (filename);`, return 0 at evalfile.c:130). Every
//! other failure is still reported: a directory is `%s: is a directory`
//! (evalfile.c:133-138, checked on the fstat'd descriptor before any
//! read), EACCES goes through file_error (`%s: Permission denied`). In
//! every case the shell keeps starting and the `-c` string runs (rc 0).
//!
//! Expectations measured with WSL GNU bash 5.3.20 (`bash --rcfile F -i
//! -c 'echo hi' </dev/null`, run as an unprivileged user for EACCES).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3457-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `rubash <option> <file> -i -c 'echo hi'`; return (stdout,
/// startup-file stderr lines, rc). The job-control notices an
/// interactive shell without a terminal prints are not part of the
/// startup-file face and are dropped.
fn run(dir: &Path, option: &str, file: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([option, file, "-i", "-c", "echo hi"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .env("HOME", dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| {
            !line.contains("cannot set terminal process group")
                && !line.contains("no job control in this shell")
        })
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn missing_init_file_is_silently_skipped() {
    let dir = scratch("missing-init");
    let (stdout, stderr, rc) = run(&dir, "--init-file", "nonexistent.rc");
    assert_eq!((stdout.as_str(), stderr.as_str(), rc), ("hi\n", "", 0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_rcfile_is_silently_skipped() {
    let dir = scratch("missing-rc");
    let (stdout, stderr, rc) = run(&dir, "--rcfile", "nonexistent.rc");
    assert_eq!((stdout.as_str(), stderr.as_str(), rc), ("hi\n", "", 0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_rcfile_under_missing_directory_is_silently_skipped() {
    let dir = scratch("missing-dir");
    let (stdout, stderr, rc) = run(&dir, "--rcfile", "nodir/nonexistent.rc");
    assert_eq!((stdout.as_str(), stderr.as_str(), rc), ("hi\n", "", 0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn directory_rcfile_reports_is_a_directory_and_continues() {
    let dir = scratch("dir");
    std::fs::create_dir_all(dir.join("rcdir")).expect("mkdir rcdir");
    let (stdout, stderr, rc) = run(&dir, "--rcfile", "rcdir");
    assert_eq!(stdout, "hi\n");
    assert_eq!(rc, 0);
    assert!(
        stderr.ends_with(": rcdir: is a directory\n") && stderr.lines().count() == 1,
        "stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn existing_rcfile_still_runs() {
    let dir = scratch("control");
    std::fs::write(dir.join("ok.rc"), "echo from-rc\n").expect("write ok.rc");
    let (stdout, stderr, rc) = run(&dir, "--rcfile", "ok.rc");
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), rc),
        ("from-rc\nhi\n", "", 0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// EACCES is not ENOENT: GNU still reports it through file_error. Only
/// meaningful for an unprivileged user (root reads a mode-000 file).
#[cfg(unix)]
#[test]
fn unreadable_rcfile_is_still_reported() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("eacces");
    let rc_path = dir.join("noperm.rc");
    std::fs::write(&rc_path, "echo leaked\n").expect("write noperm.rc");
    std::fs::set_permissions(&rc_path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    if std::fs::read(&rc_path).is_ok() {
        // Privileged runner: the file is readable regardless of mode.
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let (stdout, stderr, rc) = run(&dir, "--rcfile", "noperm.rc");
    assert_eq!(stdout, "hi\n");
    assert_eq!(rc, 0);
    assert!(
        stderr.ends_with(": noperm.rc: Permission denied\n") && stderr.lines().count() == 1,
        "stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
