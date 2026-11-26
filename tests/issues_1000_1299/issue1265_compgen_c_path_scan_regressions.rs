//! rubash#1265 — `compgen -c` PATH scan skips symlinks, lists
//! non-executables (GNU: opposite).
//!
//! GNU builtins/complete.def (command completions): a PATH entry is
//! listed when it is executable — a symlink TO an executable counts
//! (the scan follows the link); a plain non-executable file, a
//! directory, and a dangling symlink do not.
//!
//! The test builds its own fixture directory (mirroring the filed
//! k4 probe `cgtest`) and asserts exactly the two listable names.
//! Expectations verified against GNU bash 5.3.20 with the same
//! fixture shape.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1265-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn compgen_c_lists_executables_and_symlinks_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir("cgtest");
    let fix = dir.join("fix");
    std::fs::create_dir_all(&fix).expect("create fixture dir");
    let realtool = fix.join("realtool");
    std::fs::write(&realtool, "#!/bin/sh\necho real\n").expect("write realtool");
    std::fs::set_permissions(&realtool, std::fs::Permissions::from_mode(0o755))
        .expect("chmod realtool");
    std::os::unix::fs::symlink("realtool", fix.join("linktool")).expect("symlink linktool");
    std::fs::write(fix.join("plainfile"), "x\n").expect("write plainfile");
    std::fs::create_dir_all(fix.join("xdir")).expect("mkdir xdir");
    std::os::unix::fs::symlink("nonexistent", fix.join("deadlink")).expect("symlink deadlink");

    let path = format!("{}:/usr/bin:/bin", fix.display());
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("compgen -c | sort -u | grep -E '^(realtool|linktool|plainfile|xdir|deadlink)$'")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "linktool\nrealtool\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
