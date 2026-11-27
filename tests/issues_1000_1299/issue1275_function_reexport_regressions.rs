//! rubash#1275 — Imported functions lose the export attribute:
//! the shell never re-exports BASH_FUNC_* to grandchildren.
//!
//! GNU variables.c: a function imported from the environment keeps
//! its export attribute, so a child shell that received it passes it
//! on to ITS children. The chain below — define + export, child
//! imports and runs it, grandchild must still see it — only closes
//! if the middle shell re-exports.
//!
//! Expectations verified against GNU bash 5.3.20 with the same
//! two-stage script files (SELF pointing at the shell under test).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1275-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn imported_function_is_reexported_to_grandchildren() {
    let dir = tempdir("chain");
    std::fs::write(
        dir.join("stage1.sh"),
        "f() { echo F-RAN; }\nexport -f f\n\"$SELF\" stage2.sh\n",
    )
    .expect("write stage1");
    std::fs::write(dir.join("stage2.sh"), "f\n\"$SELF\" -c 'f'\n").expect("write stage2");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(exe)
        .current_dir(&dir)
        .env("SELF", exe)
        .arg("stage1.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "F-RAN\nF-RAN\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
