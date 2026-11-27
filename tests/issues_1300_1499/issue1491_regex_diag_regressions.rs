//! rubash#1491 — the `[[ =~ ]]` invalid-regex diagnostic passed the
//! raw regerror() text through; GNU formats it as
//! "[[: invalid regular expression `<pat>': <regerror text>", rc=2.
//!
//! Probe: clusters/x4/repro/1491.sh (+ gnu-1489-1491.txt).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1491-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn invalid_regex_diagnostic_names_the_pattern() {
    let dir = tempdir("regex");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "[[ x =~ [ ]]; echo \"rc=$?\"\nre='('\n[[ x =~ $re ]]; echo \"rc2=$?\"\n[[ abc =~ b ]]; echo \"rc3=$?\"\n",
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
    assert_eq!(stdout, "rc=2\nrc2=2\nrc3=0\n");
    assert!(
        stderr.contains("[[: invalid regular expression `[': Invalid regular expression"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("[[: invalid regular expression `(': Unmatched ( or \\("),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
