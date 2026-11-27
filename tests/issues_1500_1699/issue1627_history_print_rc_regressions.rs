//! rubash#1627 — `history -p` returned rc=0 even when an operand failed
//! history expansion. GNU (builtins/history.def
//! expand_and_print_history): every operand is processed in order and
//! successful expansions still print, but any failure flips the overall
//! result to EXECUTION_FAILURE (1).
//!
//! Expectations verified against GNU bash 5.3.20 (script file):
//! mixed good/bad operands print the good ones and exit 1; a lone failing
//! operand (`!999`, quick substitution) exits 1; a good operand and the
//! no-operand form exit 0.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1627-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

const SCRIPT: &str = "\
history -p 'echo good1' '!999' 'echo good2'; echo \"mixed rc=$?\"
history -p '!999'; echo \"bad rc=$?\"
history -p '^alpha^ALPHA^'; echo \"quicksub rc=$?\"
history -p 'echo hello'; echo \"good rc=$?\"
history -p; echo \"noargs rc=$?\"
";

#[test]
fn history_print_failure_rc_matches_gnu() {
    let dir = tempdir("rc");
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
        "echo good1\necho good2\nmixed rc=1\nbad rc=1\nquicksub rc=1\necho hello\ngood rc=0\nnoargs rc=0\n",
        "stderr: {stderr}"
    );
    // One diagnostic per failing operand: one in the mixed line, the lone
    // `!999`, and the quick substitution.
    assert_eq!(
        stderr.matches("history expansion failed").count(),
        3,
        "stderr: {stderr}"
    );
}
