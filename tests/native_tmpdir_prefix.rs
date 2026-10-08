#![cfg(windows)]

use std::process::Command;

#[test]
fn prefix_tmpdir_uses_native_child_boundary() {
    for script in [
        "TMPDIR=/c/TempProbeN162 cmd.exe /d /c set TMPDIR",
        "TMPDIR=/c/TempProbeN162 cmd.exe /d /c set TMPDIR | cat",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", script])
            .env_remove("TMPDIR")
            .output()
            .expect("run rubash");
        assert!(output.status.success(), "{script}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "TMPDIR=C:/TempProbeN162",
            "{script}"
        );
    }
}
