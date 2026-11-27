//! rubash#1372 — the in-process `chmod`/`mkdir` builtins ignored
//! output redirections (`>f` was never created and diagnostics
//! bypassed `2>&1`); mkdir also reported the wrong errno text.
//!
//! GNU (external commands under bash): the redirection creates f and
//! the diagnostic lands in it. Probe: clusters/k9/probes/p1372.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1372-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn chmod_mkdir_honor_output_redirections() {
    let dir = tempdir("redir");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "rm -f f; chmod 644 /nonexistent_xyz >f 2>&1; echo \"rc=$?\"; ls f 2>&1; cat f 2>/dev/null; rm -f f\n\
         rm -f f; mkdir /nonexistent_dir_xyz/sub >f 2>&1; echo \"rc=$?\"; ls f 2>&1; cat f 2>/dev/null; rm -f f\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout,
        "rc=1\nf\nchmod: cannot access '/nonexistent_xyz': No such file or directory\n\
         rc=1\nf\nmkdir: cannot create directory '/nonexistent_dir_xyz/sub': No such file or directory\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
