//! Issue 2411: an interactive login shell in posix mode must NOT run
//! the profile chain — GNU's posix arm sources $ENV only.
//!
//! GNU shell.c run_startup_files gates both login profile blocks on
//! posixly_correct == 0: the argv[0]-dash/--login block (shell.c:1196)
//! and the interactive arm split (shell.c:1223), whose posix arm
//! (shell.c:1244-1245) runs execute_env_file(ENV) for every
//! interactive shell, login or not. rubash's login branch ran
//! /etc/profile + ~/.bash_profile even with posix mode on, so a posix
//! login shell fired profile markers GNU never runs (the env-file
//! step already selected $ENV correctly — both fired).
//!
//! Oracle: GNU bash 5.3.20 with a scratch HOME holding marker files:
//! `--posix -l -i` and argv0=`-bash` `--posix -i` print the ENV marker
//! only; the non-posix `--login -i` control prints the profile marker
//! only (no $ENV, no .bashrc — a login shell never takes the rc arm).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2411-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    // The scratch dir IS the scratch HOME: profile/rc markers live in
    // it, the $ENV file beside them.
    write(&dir, ".bash_profile", "PROFILE2411=prof\n");
    write(&dir, ".bashrc", "RC2411=rc\n");
    write(&dir, "env.sh", "ENV2411=env\n");
    write(
        &dir,
        "cmds.txt",
        "echo P=${PROFILE2411-unset} E=${ENV2411-unset} R=${RC2411-unset}\n",
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
fn issue2411_posix_login_flag_interactive_reads_env_only() {
    let dir = scratch("posix-login-flag");
    let bash = install_copy(&dir, "bash");
    let out = run_interactive(&dir, &bash, &["--posix", "--login", "-i"]);
    assert_eq!(out, "P=unset E=env R=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2411_posix_login_argv0_interactive_reads_env_only() {
    let dir = scratch("posix-login-argv0");
    let bash = install_copy(&dir, "-bash");
    let out = run_interactive(&dir, &bash, &["--posix", "-i"]);
    assert_eq!(out, "P=unset E=env R=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2411_login_interactive_nonposix_reads_profile_only_control() {
    let dir = scratch("login-nonposix");
    let bash = install_copy(&dir, "bash");
    let out = run_interactive(&dir, &bash, &["--login", "-i"]);
    assert_eq!(out, "P=prof E=unset R=unset\n");
    let _ = fs::remove_dir_all(&dir);
}
