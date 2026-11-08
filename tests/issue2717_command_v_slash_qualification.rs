//! Regression pins for rubash#2717 — `command -v` / `type` reported
//! non-executable slash-named files as found commands (and one walk
//! absolutized them), where GNU qualifies a slash name by
//! file_status alone before any describe consumer reports it.
//!
//! GNU ownership: builtins/type.def describe_command — for an
//! absolute_program (slash-bearing) name, `f = file_status
//! (command)` and the name is reported (verbatim) only when
//! `f & FS_EXECABLE`; findcmd.c file_status never sets FS_EXECABLE
//! for a directory and sets it only when the file is executable.
//! A slash-free name keeps the PATH walk's semantics, including the
//! file_to_lose_on report of an existing non-executable file
//! (findcmd.c:591/695) — that face is a guard here, not a defect.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2717-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env("FIX", root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn write(path: &Path, content: &str, mode: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, content).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    write(&root.join("seed.txt"), "data\n", 0o644);
    write(&root.join("rel1").join("foo"), "#!/bin/sh\necho FOO-RAN\n", 0o755);
    std::fs::create_dir_all(root.join("somedir")).expect("somedir");
    write(&root.join("noexecdir").join("bar"), "x\n", 0o644);
    root
}

#[test]
fn slash_names_must_be_executable_to_be_reported() {
    let root = fixture("matrix");
    let out = run(
        &root,
        "command -v ./seed.txt; echo \"rc=$?\"\n\
         command -v ./rel1/foo; echo \"rc=$?\"\n\
         command -v ./somedir; echo \"rc=$?\"\n\
         command -V ./seed.txt; echo \"rc=$?\"\n\
         type ./seed.txt; echo \"rc=$?\"\n\
         type ./rel1/foo; echo \"rc=$?\"\n",
    );
    assert_eq!(
        out.stdout,
        "rc=1\n./rel1/foo\nrc=0\nrc=1\nrc=1\nrc=1\n./rel1/foo is ./rel1/foo\nrc=0\n"
    );
    assert_eq!(
        out.stderr,
        "case.sh: line 4: command: ./seed.txt: not found\n\
         case.sh: line 5: type: ./seed.txt: not found\n"
    );
    assert_eq!(out.rc, 0);
}

#[test]
fn absolute_slash_names_are_qualified_too() {
    let root = fixture("absolute");
    let out = run(
        &root,
        "command -v \"$FIX/seed.txt\"; echo \"rc=$?\"\n\
         command -v \"$FIX/rel1/foo\"; echo \"rc=$?\"\n\
         command -v /nonexistent/x; echo \"rc=$?\"\n",
    );
    let abs = format!("{}/rel1/foo", root.display());
    assert_eq!(out.stdout, format!("rc=1\n{abs}\nrc=0\nrc=1\n"));
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn path_found_non_executable_keeps_lose_on_report() {
    let root = fixture("loseon");
    let out = run(
        &root,
        "PATH=./noexecdir:/usr/bin:/bin\ncommand -v bar; echo \"rc=$?\"\n",
    );
    // GNU reports the file_to_lose_on hit (rc 0). The reported
    // spelling is the walk's join; only the qualification (found,
    // rc 0) is pinned here.
    assert!(out.stdout.contains("noexecdir/bar"), "stdout={:?}", out.stdout);
    assert!(out.stdout.ends_with("rc=0\n"), "stdout={:?}", out.stdout);
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}
