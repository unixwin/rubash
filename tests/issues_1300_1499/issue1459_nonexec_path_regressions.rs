//! rubash#1459 — a NON-executable file found via PATH was executed
//! as a shell script; GNU refuses with "Permission denied", rc=126,
//! and never runs the content (the exec fallback may only interpret
//! ENOEXEC files, not EACCES ones). The `./file` form agrees.
//!
//! Probe: clusters/x1/issues/issue-1459.md repro.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1459-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn nonexecutable_file_is_permission_denied_126() {
    let dir = tempdir("noexec");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "printf 'echo hi\\n' > notexec.sh\n\
         PATH=\"$PWD:$PATH\"\n\
         notexec.sh; echo \"rc=$?\"\n\
         ./notexec.sh; echo \"rc=$?\"\n",
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
    assert_eq!(stdout, "rc=126\nrc=126\n", "content must not run: {stdout}");
    assert_eq!(
        stderr.matches("notexec.sh: Permission denied").count(),
        2,
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
