//! rubash#1484 — `${param@P}`: the \s prompt escape expanded to a
//! hardcoded "bash" instead of the shell's invocation name (argv[0]
//! basename), which is what GNU puts there.
//!
//! Probe: clusters/x3/repro/1484.sh (shell invoked through a symlink
//! named `shell`, output is that name).
//! Expectations verified against GNU bash 5.3.20 (script files).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1484-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn prompt_s_escape_uses_argv0_basename() {
    let dir = tempdir("prompts");
    let link = dir.join("w3bshell");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rubash"), &link).expect("symlink shell");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "p='\\s'\necho \"${p@P}\"\n").expect("write script");
    let output = Command::new(&link)
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash via symlink");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "w3bshell\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
