//! rubash#1479 — the standalone rubash binary never ran $BASH_ENV
//! for non-interactive shells (the niubash host did — drift). GNU
//! sources $BASH_ENV before a `-c` command in a non-interactive
//! shell.
//!
//! Probe: clusters/x3/repro/1479.sh.
//! Expectations verified against GNU bash 5.3.20.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1479-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn bash_env_runs_for_dash_c() {
    let dir = tempdir("benv");
    let benv = dir.join("benv.sh");
    std::fs::write(&benv, "echo BASHENV-MARKER\n").expect("write benv");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo body"])
        .env("BASH_ENV", &benv)
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "BASHENV-MARKER\nbody\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
