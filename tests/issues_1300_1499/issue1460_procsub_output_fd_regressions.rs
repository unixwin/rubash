//! rubash#1460 — output process substitution `>(...)` handed an
//! external command a dangling /dev/fd/N: the data was silently lost
//! (tee_out.txt stayed empty, tee reported the fd missing). GNU keeps
//! the substitution fd alive into the external child.
//!
//! Probe: clusters/x1/issues/issue-1460.md repro.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1460-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn output_procsub_delivers_data_to_file() {
    let dir = tempdir("tee");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "tee >(cat > tee_out.txt) <<< \"hello\"\nsleep 0.5\ncat tee_out.txt\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "hello\nhello\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("tee_out.txt")).expect("tee_out.txt"),
        "hello\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
