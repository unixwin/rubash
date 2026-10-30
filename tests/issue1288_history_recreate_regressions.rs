//! rubash#1288 — `history -w`/`-a` failed on a chmod-000 HISTFILE; GNU
//! recreates the file with mode 600 and returns 0.
//!
//! Probe: clusters/k5/repro/r1288.sh (HISTFILE chmod 000, then -w/-a).
//! Expectations verified against GNU bash 5.3.20 (script files).

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1288-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[cfg(unix)]
const SCRIPT: &str = "\
export HISTFILE=$PWD/hist.test; export HOME=$PWD/fakehome
mkdir -p \"$HOME\"; rm -f \"$HISTFILE\"; touch \"$HISTFILE\"; chmod 000 \"$HISTFILE\"
set -o history
history -c
history -s \"cmd1\"
history -w; echo \"wrc=$?\"
history -a; echo \"arc=$?\"
ls -l \"$HISTFILE\"
cat \"$HISTFILE\"
";

// The face under test is POSIX mode semantics end to end (chmod 000 on
// the HISTFILE, recreation at mode 600, the `ls -l` mode string, and the
// mode-bit readback), so the probe only exists on unix.
#[cfg(unix)]
#[test]
fn history_write_recreates_unreadable_histfile_mode_600() {
    let dir = tempdir("recreate");
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
    assert!(
        stdout.contains("wrc=0"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("arc=0"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("-rw-------"),
        "ls line should show mode 600, stdout: {stdout}"
    );
    let hist = dir.join("hist.test");
    let mode = std::fs::metadata(&hist)
        .expect("histfile exists")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "recreated HISTFILE mode");
    let contents = std::fs::read_to_string(&hist).expect("read histfile");
    assert!(
        contents.lines().any(|l| l == "cmd1"),
        "histfile contents: {contents}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
