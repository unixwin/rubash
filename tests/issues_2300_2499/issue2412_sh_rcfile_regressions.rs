//! Issue 2412: an interactive shell invoked as `sh` must ignore
//! --rcfile and source $ENV instead.
//!
//! GNU shell.c run_startup_files: the rcfile override belongs to the
//! bash arm's execute_bashrc_file (shell.c:1235-1237); the interactive
//! `sh` arm (shell.c:1238-1239) and the posix arm (shell.c:1244-1245)
//! run execute_env_file(ENV) and never look at the rcfile. rubash's
//! startup chain already skipped the rcfile for these arms, but the
//! drivers' direct-run fallback (init_file_needs_direct_run, the
//! door for the empty-HOME / non-interactive / login shapes) then ran
//! the --rcfile file anyway, so an interactive `sh --rcfile f` fired
//! f's markers on top of $ENV.
//!
//! Oracle: GNU bash 5.3.20, binary copied as `sh`, `--rcfile rc.sh
//! -i` with ENV=env.sh prints the ENV marker only; the plain-bash
//! `--rcfile rc.sh -i` control prints the rcfile marker; the
//! `--posix --rcfile rc.sh -i` arm prints the ENV marker only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2412-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    write(&dir, "env.sh", "ENV2412=env\n");
    write(&dir, "rc.sh", "RC2412=rc\n");
    write(
        &dir,
        "cmds.txt",
        "echo E=${ENV2412-unset} R=${RC2412-unset}\n",
    );
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

fn run_interactive(dir: &Path, program: &Path, args: &[&str]) -> String {
    let stdin = fs::File::open(dir.join("cmds.txt")).expect("open cmds.txt");
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::from(stdin))
        .env("ENV", dir.join("env.sh"))
        .env("HOME", dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run interactive shell");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue2412_interactive_sh_ignores_rcfile_reads_env() {
    let dir = scratch("sh-rcfile");
    let sh = install_copy(&dir, "sh");
    let rc = dir.join("rc.sh");
    let out = run_interactive(&dir, &sh, &["--rcfile", rc.to_str().unwrap(), "-i"]);
    assert_eq!(out, "E=env R=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2412_interactive_posix_bash_ignores_rcfile_reads_env() {
    let dir = scratch("posix-rcfile");
    let bash = install_copy(&dir, "bash");
    let rc = dir.join("rc.sh");
    let out = run_interactive(
        &dir,
        &bash,
        &["--posix", "--rcfile", rc.to_str().unwrap(), "-i"],
    );
    assert_eq!(out, "E=env R=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2412_interactive_bash_rcfile_still_runs_control() {
    let dir = scratch("bash-rcfile");
    let bash = install_copy(&dir, "bash");
    let rc = dir.join("rc.sh");
    let out = run_interactive(&dir, &bash, &["--rcfile", rc.to_str().unwrap(), "-i"]);
    assert_eq!(out, "E=unset R=rc\n");
    let _ = fs::remove_dir_all(&dir);
}
