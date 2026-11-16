//! niubash#993 S2 — `cd /c; pwd` renders the bare drive root `/c`,
//! not `/c/`.
//!
//! Git Bash 5.3.15:
//!   cd /c            -> `/c`   (was `/c/` on rubash)
//!   cd /c/           -> `/c`
//!   cd /             -> `/`    (the one root that keeps its slash)
//!   cd /c; cd ..     -> `/`
//!   dirs / PWD / cd- -> the same `/c` rendering everywhere
//!
//! GNU anchor: sh_canonpath strips the trailing separator of every
//! non-`/` root, so the drive root's canonical spelling has no slash;
//! the display funnel is `shell_pwd_display_path`
//! (src/executor/path.rs), the one producer for PWD, pwd output,
//! DIRSTACK and the startup PWD (rubash#537).

#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(tag: &str, script: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!("rubash-993s2-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path: PathBuf = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(&dir)
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "OS",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    let out = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn cd_bare_drive_root_pwd_has_no_trailing_slash() {
    let drive = std::env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".to_string())
        .chars()
        .next()
        .unwrap()
        .to_ascii_lowercase();
    let (stdout, stderr, code) = run_script(
        "bare",
        &format!(
            "cd /{drive}; pwd\ncd /{drive}/; pwd\ncd /{drive}/Windows/System32/../.. 2>/dev/null; pwd\n"
        ),
    );
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(
        stdout,
        format!("/{drive}\n/{drive}\n/{drive}\n"),
        "stdout: {stdout:?}"
    );
}

#[test]
fn drive_root_pwd_var_dirs_and_parent() {
    let drive = std::env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".to_string())
        .chars()
        .next()
        .unwrap()
        .to_ascii_lowercase();
    let (stdout, _stderr, _code) = run_script(
        "vars",
        &format!("cd /{drive}; echo \"[$PWD]\"; dirs; cd ..; pwd\n"),
    );
    assert_eq!(
        stdout,
        format!("[/{drive}]\n/{drive}\n/\n"),
        "stdout: {stdout:?}"
    );
}
