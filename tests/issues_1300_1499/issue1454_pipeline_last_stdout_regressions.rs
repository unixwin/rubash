//! rubash#1454 — the LAST command of a pipeline never got the
//! console (the shell's own stdout) as its stdout, so pagers did not
//! page and /proc/self/fd/1 showed a pipe where GNU shows the shell's
//! stdout target.
//!
//! GNU: only non-last pipeline stages write into a pipe; the last
//! stage inherits the shell's stdout unchanged.
//! Probe: clusters/w2/repro/p1454.sh (Linux /proc form).
//! Expectations verified against GNU bash 5.3.20 (script files,
//! stdout redirected to a file).

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1454-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
#[ignore = "issue#1454 residual: the last pipeline stage gets a fresh pipe as fd1 instead of inheriting the shell's stdout (readlink /proc/self/fd/1 in a last-stage brace group prints pipe:[...], GNU prints the stdout file path); see bf-w3-b REPORT.md"]
fn last_pipeline_stage_inherits_shell_stdout() {
    let dir = tempdir("lastout");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "out=$( { echo hello | cat; } )\n\
         echo \"captured=[$out]\"\n\
         echo hello | { readlink /proc/self/fd/1 || echo nolink; }\n\
         { readlink /proc/self/fd/1; }\n",
    )
    .expect("write script");
    let out_path = dir.join("out.txt");
    let out_file = std::fs::File::create(&out_path).expect("create out file");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(0));
    let bytes = std::fs::read(&out_path).expect("read out file");
    let stdout = String::from_utf8_lossy(&bytes).into_owned();
    let canon = std::fs::canonicalize(&out_path)
        .expect("canonicalize")
        .display()
        .to_string();
    assert_eq!(
        stdout,
        format!("captured=[hello]\n{canon}\n{canon}\n"),
        "last stage fd1 must be the shell's stdout file, not a pipe"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
