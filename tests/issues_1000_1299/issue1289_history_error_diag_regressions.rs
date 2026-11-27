//! rubash#1289 — `history -w`/`-a` file-error diagnostics diverged from
//! GNU in presence, wording and count across three failure shapes:
//! HISTFILE is a directory (silent, rc=1), parent directory missing
//! ("cannot create: No such file or directory", rc=1), HISTFILE empty
//! ("HISTFILE: parameter null or not set", rc=1, one per -w/-a call).
//!
//! Probe: clusters/k5/repro/r1289.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1289-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

const SCRIPT: &str = "\
export HOME=$PWD/fakehome; mkdir -p \"$HOME\"
set -o history; history -c; history -s \"cmd1\"
HISTFILE=$PWD/hdir; mkdir -p \"$HISTFILE\"
history -w; echo \"B1-wrc=$?\"; history -a; echo \"B1-arc=$?\"
HISTFILE=$PWD/nonexistdir_xyz/hist
history -w; echo \"B2-wrc=$?\"; history -a; echo \"B2-arc=$?\"
HISTFILE=\"\"
history -w; echo \"B3-wrc=$?\"; history -a; echo \"B3-arc=$?\"
";

#[test]
fn history_write_error_diagnostics_match_gnu() {
    let dir = tempdir("diag");
    let script = dir.join("probe.sh");
    std::fs::write(&script, SCRIPT).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        stdout,
        "B1-wrc=1\nB1-arc=1\nB2-wrc=1\nB2-arc=1\nB3-wrc=1\nB3-arc=1\n"
    );
    // B1 (directory) is silent; B2 reports once (the -w), B3 reports per call.
    assert_eq!(
        stderr
            .matches("cannot create: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .matches("HISTFILE: parameter null or not set")
            .count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(stderr.lines().count(), 3, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
