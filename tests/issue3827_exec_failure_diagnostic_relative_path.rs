//! rubash#3827: an exec-failure diagnostic for a command found via a
//! RELATIVE PATH entry must name the PATH-joined relative form, not the
//! session-cwd-anchored absolute path. GNU prints the string its walk
//! built (findcmd.c `sh_makepath`) and hands that same string to execve
//! and to shell_execve's file_error (execute_cmd.c:6126-6159), so
//! `PATH=sub:...` reports `sub/mycmd: Permission denied` (rc 126),
//! `PATH=.` reports `./mycmd: Permission denied`, and an absolute PATH
//! entry reports the absolute form (control). A command named with a
//! slash directly is never PATH-searched (findcmd.c:385-387) and is
//! reported as typed (control); a missing command is reported by bare
//! name (control). The same rule applies to a pipeline member's spawn
//! failure (pipeline_exec.rs `member_spawn_failure`, the same
//! `report_external_spawn_error` rule on the simple-command path).
//!
//! Regression seed: R133-D (issue #3827 body). GNU bash 5.3.20: 13/13
//! checks; rubash base 969230b9: 11/13 (the two relative-entry
//! diagnostics fail). Unix-only: the as-written join is the unix
//! PATH-walk contract; on Windows `spawn_exec_name` declines and the
//! anchored display is the #2302 status quo.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3827-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run_in(cwd: &Path, body: &str) -> Outcome {
    std::fs::write(cwd.join("driver.sh"), body).expect("driver script");
    let out = Command::new(BIN)
        .arg("driver.sh")
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn run(root: &Path, body: &str) -> Outcome {
    run_in(root, body)
}

/// 1: a non-executable file found via a relative PATH entry `sub`
/// reports the PATH-joined form `sub/mycmd`, not `<cwd>/sub/mycmd`.
#[test]
fn rel_sub_path_entry_reports_joined_form() {
    let root = scratch("rel-sub");
    let out = run(
        &root,
        "mkdir -p sub\nprintf 'x' > sub/mycmd\nchmod 644 sub/mycmd\nPATH=sub:/usr/bin:/bin\nmycmd\n",
    );
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "driver.sh: line 5: sub/mycmd: Permission denied\n"
    );
}

/// 2: a non-executable file found via the `.` PATH entry reports
/// `./mycmd`, not `<cwd>/./mycmd`.
#[test]
fn rel_dot_path_entry_reports_dot_slash_form() {
    let root = scratch("rel-dot");
    let out = run(
        &root,
        "printf 'x' > mycmd\nchmod 644 mycmd\nPATH=.:/usr/bin:/bin\nmycmd\n",
    );
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "driver.sh: line 4: ./mycmd: Permission denied\n"
    );
}

/// 3: a non-executable file found via a `..` PATH entry reports the
/// `../sib/mycmd` form, not the absolute path.
#[test]
fn rel_dotdot_path_entry_reports_dotdot_form() {
    let root = scratch("rel-dotdot");
    let work = root.join("work");
    let sib = root.join("sib");
    std::fs::create_dir_all(&work).expect("work dir");
    std::fs::create_dir_all(&sib).expect("sib dir");
    let out = run_in(
        &work,
        "printf 'x' > ../sib/mycmd\nchmod 644 ../sib/mycmd\nPATH=../sib:/usr/bin:/bin\nmycmd\n",
    );
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "driver.sh: line 4: ../sib/mycmd: Permission denied\n"
    );
}

/// 4: an empty PATH element normalizes to `.` (GNU findcmd.c:322
/// `get_next_path_element`), so the diagnostic is `./mycmd`.
#[test]
fn empty_path_element_reports_dot_slash_form() {
    let root = scratch("empty-elem");
    let out = run(
        &root,
        "printf 'x' > mycmd\nchmod 644 mycmd\nPATH=:/usr/bin:/bin\nmycmd\n",
    );
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "driver.sh: line 4: ./mycmd: Permission denied\n"
    );
}

/// 5 (control): an absolute PATH entry reports the absolute form on
/// both shells.
#[test]
fn abs_path_entry_reports_absolute_form() {
    let root = scratch("abs-entry");
    let out = run(
        &root,
        "mkdir -p sub\nprintf 'x' > sub/mycmd\nchmod 644 sub/mycmd\nPATH=\"$PWD/sub:/usr/bin:/bin\"\nmycmd\n",
    );
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        format!(
            "driver.sh: line 5: {}/sub/mycmd: Permission denied\n",
            root.display()
        )
    );
}

/// 6 (control): a command named with a slash directly is never
/// PATH-searched and is reported as typed.
#[test]
fn direct_slash_name_reports_typed_form() {
    let root = scratch("direct-slash");
    let out = run(&root, "printf 'x' > mycmd\nchmod 644 mycmd\n./mycmd\n");
    assert_eq!(out.rc, 126);
    assert_eq!(
        out.stderr,
        "driver.sh: line 3: ./mycmd: Permission denied\n"
    );
}

/// 7 (control): a missing command is reported by bare name.
#[test]
fn missing_command_reports_bare_name() {
    let root = scratch("missing");
    let out = run(&root, "PATH=sub:/usr/bin:/bin\nnosuchcmd123\n");
    assert_eq!(out.rc, 127);
    assert_eq!(
        out.stderr,
        "driver.sh: line 2: nosuchcmd123: command not found\n"
    );
}

/// 8 (control): an executable file found via a relative PATH entry
/// runs fine.
#[test]
fn rel_sub_executable_runs() {
    let root = scratch("rel-exec");
    let out = run(
        &root,
        "mkdir -p sub\nprintf '#!/bin/sh\\necho OK\\n' > sub/mycmd\nchmod +x sub/mycmd\nPATH=sub:/usr/bin:/bin\nmycmd\n",
    );
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "OK\n");
    assert_eq!(out.stderr, "");
}

/// 9: a pipeline member that fails to exec via a relative PATH entry
/// reports the joined form too (the `member_spawn_failure` face of
/// the same rule), and its slot in PIPESTATUS is 126.
#[test]
fn rel_sub_pipeline_member_reports_joined_form() {
    let root = scratch("rel-pipe");
    let out = run(
        &root,
        "mkdir -p sub\nprintf 'x' > sub/mycmd\nchmod 644 sub/mycmd\nPATH=sub:/usr/bin:/bin\nmycmd | cat\necho \"ps0=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "ps0=126\n");
    assert_eq!(
        out.stderr,
        "driver.sh: line 5: sub/mycmd: Permission denied\n"
    );
}
