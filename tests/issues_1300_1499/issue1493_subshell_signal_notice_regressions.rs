//! rubash#1493 — no signal-death notice when a (foreground)
//! subshell was killed by a signal; GNU prints the notice
//! ("File size limit exceeded" for SIGXFSZ) on the shell's stderr
//! and $? is 128+sig.
//!
//! Probe: clusters/x4/repro/1493.sh (dd under `ulimit -f 0` dies by
//! SIGXFSZ; notice text is the shell's, dd's own stderr is nulled).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1493-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn subshell_signal_death_is_noticed() {
    let dir = tempdir("signotice");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "( ulimit -f 0; exec dd if=/dev/zero of=$PWD/big bs=1024 count=100 2>/dev/null )\n\
         echo \"rc=$?\"\n\
         rm -f $PWD/big\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=153\n");
    assert!(
        stderr.contains("File size limit exceeded"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
