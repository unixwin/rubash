//! rubash#1296 — `read -u N` ignored the fd opened by a redirect
//! attached to the same command (`read -u 3 x 3< file` read the wrong
//! source); mapfile already honored it.
//!
//! Probe: clusters/k5/repro/r1296.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1296-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn read_u_uses_fd_from_same_command_redirect() {
    let dir = tempdir("readu");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "printf 'l1\\n' > rudata.txt\nread -u 3 x 3< rudata.txt; echo \"rc=$? x=[$x]\"\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "rc=0 x=[l1]\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
