//! Issue 2394: an interactive shell invoked as `sh` must read $ENV.
//!
//! GNU shell.c run_startup_files: in the posixly_correct == 0 arm
//! the interactive `sh` branch (shell.c:1238-1239) sources $ENV
//! instead of the bashrc chain, and the posix arm (shell.c:1244-1245)
//! sources $ENV for any interactive shell. A plain interactive bash
//! ignores $ENV (control), and a non-interactive sh reads no env
//! file at all (control).
//!
//! Oracle: GNU bash 5.3.20, script-file probes (see the issue's
//! Regression seed: `ENV=... exec -a sh bash -i < cmds.txt` prints
//! M=env26). Here the shell under test is a copy of the test binary
//! named `sh` (resp. `bash` for the control).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2394-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
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
    // A symlink where the platform allows (a debug binary is far too
    // large to copy into a small tmpfs); a real copy on Windows. The
    // invocation name comes from argv[0]'s path either way.
    #[cfg(unix)]
    std::os::unix::fs::symlink(rubash_bin(), &path).expect("link rubash binary");
    #[cfg(not(unix))]
    fs::copy(rubash_bin(), &path).expect("copy rubash binary");
    path
}

#[test]
fn issue2394_interactive_sh_reads_env_file() {
    let dir = scratch("sh-i");
    let sh = install_copy(&dir, "sh");
    write(&dir, "env26.sh", "ENVMARK=env26\n");
    write(&dir, "cmds.txt", "echo M=${ENVMARK-unset}\n");
    let stdin = fs::File::open(dir.join("cmds.txt")).expect("open cmds.txt");
    let output = Command::new(sh)
        .arg("-i")
        .stdin(Stdio::from(stdin))
        .env("ENV", dir.join("env26.sh"))
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("run sh -i");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "M=env26\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2394_interactive_bash_ignores_env_control() {
    let dir = scratch("bash-i");
    let bash = install_copy(&dir, "bash");
    write(&dir, "env26.sh", "ENVMARK=env26\n");
    write(&dir, "cmds.txt", "echo M=${ENVMARK-unset}\n");
    let stdin = fs::File::open(dir.join("cmds.txt")).expect("open cmds.txt");
    let output = Command::new(bash)
        .arg("-i")
        .stdin(Stdio::from(stdin))
        .env("ENV", dir.join("env26.sh"))
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("run bash -i");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "M=unset\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2394_noninteractive_sh_reads_no_env_file_control() {
    let dir = scratch("sh-c");
    let sh = install_copy(&dir, "sh");
    write(&dir, "env26.sh", "ENVMARK=env26\n");
    let output = Command::new(sh)
        .arg("-c")
        .arg("echo M=${ENVMARK-unset}")
        .env("ENV", dir.join("env26.sh"))
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("run sh -c");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "M=unset\n"
    );
    let _ = fs::remove_dir_all(&dir);
}
