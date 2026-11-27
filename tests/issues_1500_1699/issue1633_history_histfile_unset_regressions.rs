//! rubash#1633 — with HISTFILE unset, the file modes of `history`
//! printed `history: filename not specified` and returned rc=2. GNU
//! (builtins/history.def history_builtin file branch): the filename
//! falls back to $HISTFILE, and a null-or-unset $HISTFILE draws
//! `history: HISTFILE: parameter null or not set` with
//! EXECUTION_FAILURE (1) — for -a/-n/-r/-w alike. An explicitly empty
//! operand is the separate `history: empty filename` diagnostic.
//!
//! Expectations verified against GNU bash 5.3.20 (script file).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1633-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

const SCRIPT: &str = "\
export HOME=$PWD/fakehome; mkdir -p \"$HOME\"
unset HISTFILE
history -n; echo \"n rc=$?\"
history -r; echo \"r rc=$?\"
history -w; echo \"w rc=$?\"
history -a; echo \"a rc=$?\"
HISTFILE=''
history -n; echo \"n-empty rc=$?\"
history -n ''; echo \"n-emptyop rc=$?\"
";

#[test]
fn histfile_unset_diagnostic_matches_gnu() {
    let dir = tempdir("unset");
    let script = dir.join("probe.sh");
    std::fs::write(&script, SCRIPT).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("HISTFILE")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        stdout, "n rc=1\nr rc=1\nw rc=1\na rc=1\nn-empty rc=1\nn-emptyop rc=1\n",
        "stderr: {stderr}"
    );
    // Four unset modes + the set-but-empty variable.
    assert_eq!(
        stderr
            .matches("HISTFILE: parameter null or not set")
            .count(),
        5,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("history: empty filename").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("filename not specified"),
        "old diagnostic must be gone; stderr: {stderr}"
    );
}
