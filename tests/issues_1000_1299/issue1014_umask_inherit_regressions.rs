//! rubash#1014 — Shell forces umask 022 at startup instead of
//! inheriting.
//!
//! GNU shell.c: bash does not touch the process umask at startup
//! (outside POSIX mode's default-umask rule for a zero mask); a shell
//! started with umask 027 reports 0027 and creates files 640. The
//! rubash bug rewrote the mask to 022 unconditionally.
//!
//! Expectations verified against GNU bash 5.3.20 (child launched with
//! umask 027 pre-set by the parent shell).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1014-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn startup_umask_is_inherited_not_forced() {
    let dir = tempdir("inherit");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let launch = format!("umask 027; exec \"{exe}\" -c 'umask; touch newf; stat -c %a newf'");
    let output = Command::new("/bin/sh")
        .current_dir(&dir)
        .arg("-c")
        .arg(&launch)
        .output()
        .expect("run rubash via umask-setting parent");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "0027\n640\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
