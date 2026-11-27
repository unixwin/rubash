//! Issue 2436: --init-file/--rcfile must stay closed behind GNU's
//! four startup doors.
//!
//! GNU shell.c run_startup_files (shell.c:1156-1245): bashrc_file is
//! executed by exactly one arm — execute_bashrc_file, reached only by
//! an interactive, non-login, non-posix shell invoked as bash with
//! no_rc clear (shell.c:1235-1237). A non-interactive shell returns
//! after the $BASH_ENV step (shell.c:1214-1221), a login shell bumps
//! no_rc and reads the profile chain (shell.c:1196-1206, 1223-1233),
//! --norc sets no_rc at parse time, and the sh/posix arms read $ENV
//! (shell.c:1238-1245). rubash's startup chain already kept those
//! doors, but the drivers' direct-run fallback
//! (init_file_needs_direct_run) then ran the file anyway for the
//! non-interactive / login / --norc shapes.
//!
//! Oracle: GNU bash 5.3.20 — every closed-door shape below prints
//! `unset` (the login shape still reads the profile chain); the
//! plain interactive bash control prints `seed`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2436-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    write(&dir, "rc.sh", "IFV=seed\n");
    write(&dir, "cmds.txt", "echo ${IFV-unset}\n");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).expect("write scratch file");
}

fn install_copy(dir: &Path, name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = dir.join(exe);
    #[cfg(unix)]
    std::os::unix::fs::symlink(rubash_bin(), &path).expect("link rubash binary");
    #[cfg(not(unix))]
    fs::copy(rubash_bin(), &path).expect("copy rubash binary");
    path
}

fn run_with_stdin_file(dir: &Path, program: &Path, args: &[&str]) -> String {
    let stdin = fs::File::open(dir.join("cmds.txt")).expect("open cmds.txt");
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::from(stdin))
        .env("HOME", dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run(dir: &Path, program: &Path, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .env("HOME", dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run shell");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue2436_norc_gate_keeps_init_file_closed() {
    let dir = scratch("norc");
    let bin = rubash_bin();
    let rc = dir.join("rc.sh");
    let rc = rc.to_str().unwrap();
    // Both flag orders, and the --rcfile spelling (issue matrix).
    for args in [
        vec!["--init-file", rc, "--norc", "-i"],
        vec!["--norc", "--rcfile", rc, "-i"],
    ] {
        let out = run_with_stdin_file(&dir, &bin, &args);
        assert_eq!(out, "unset\n", "args: {args:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2436_noninteractive_dash_c_gate() {
    let dir = scratch("dash-c");
    let bin = rubash_bin();
    let rc = dir.join("rc.sh");
    let rc = rc.to_str().unwrap();
    for flag in ["--init-file", "--rcfile"] {
        let out = run(&dir, &bin, &[flag, rc, "-c", "echo ${IFV-unset}"]);
        assert_eq!(out, "unset\n", "flag: {flag}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2436_noninteractive_script_gate() {
    let dir = scratch("script");
    let bin = rubash_bin();
    write(&dir, "main.sh", "echo ${IFV-unset}\n");
    let rc = dir.join("rc.sh");
    let main = dir.join("main.sh");
    let out = run(
        &dir,
        &bin,
        &["--init-file", rc.to_str().unwrap(), main.to_str().unwrap()],
    );
    assert_eq!(out, "unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(not(windows))]
#[test]
fn issue2436_login_gate_reads_profile_not_init_file() {
    let dir = scratch("login");
    write(&dir, ".bash_profile", "PROF=prof\n");
    write(
        &dir,
        "cmds.txt",
        "echo prof=${PROF-unset} init=${IFV-unset}\n",
    );
    let bin = rubash_bin();
    let rc = dir.join("rc.sh");
    let rc = rc.to_str().unwrap();
    for flag in ["--init-file", "--rcfile"] {
        let out = run_with_stdin_file(&dir, &bin, &[flag, rc, "-l", "-i"]);
        assert_eq!(out, "prof=prof init=unset\n", "flag: {flag}");
    }
    // --noprofile closes the profile chain too; the init file must
    // still stay closed.
    let out = run_with_stdin_file(&dir, &bin, &["--init-file", rc, "--noprofile", "-l", "-i"]);
    assert_eq!(out, "prof=unset init=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2436_sh_noninteractive_gate() {
    let dir = scratch("sh");
    let sh = install_copy(&dir, "sh");
    let rc = dir.join("rc.sh");
    let rc = rc.to_str().unwrap();
    let out = run(&dir, &sh, &["--init-file", rc, "-c", "echo ${IFV-unset}"]);
    assert_eq!(out, "unset\n");
    write(&dir, "main.sh", "echo ${IFV-unset}\n");
    let main = dir.join("main.sh");
    let out = run(&dir, &sh, &["--init-file", rc, main.to_str().unwrap()]);
    assert_eq!(out, "unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2436_interactive_bash_control_still_runs_init_file() {
    let dir = scratch("control");
    let bin = rubash_bin();
    let rc = dir.join("rc.sh");
    let rc = rc.to_str().unwrap();
    for flag in ["--init-file", "--rcfile"] {
        let out = run_with_stdin_file(&dir, &bin, &[flag, rc, "-i"]);
        assert_eq!(out, "seed\n", "flag: {flag}");
    }
    let _ = fs::remove_dir_all(&dir);
}
