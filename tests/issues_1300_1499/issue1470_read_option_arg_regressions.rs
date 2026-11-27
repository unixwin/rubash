//! rubash#1470 — `read` with a trailing option that requires an
//! argument did not error — it went ahead and read stdin (hanging on
//! a live stdin). GNU: "read: -n: option requires an argument" plus
//! the usage line, rc=2, for -n -N -d -p -t -u -a -i (and combos).
//!
//! Probe: clusters/x2/repro/1470.sh (stdin /dev/null in the probe;
//! the piped form also returns 2 without consuming input as a read).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1470-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn read_trailing_option_without_argument_errors() {
    let dir = tempdir("optarg");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "read -n </dev/null; echo \"rc-n=$?\"\n\
         read -N </dev/null; echo \"rc-N=$?\"\n\
         read -d </dev/null; echo \"rc-d=$?\"\n\
         read -p </dev/null; echo \"rc-p=$?\"\n\
         read -t </dev/null; echo \"rc-t=$?\"\n\
         read -u </dev/null; echo \"rc-u=$?\"\n\
         read -a </dev/null; echo \"rc-a=$?\"\n\
         read -i </dev/null; echo \"rc-i=$?\"\n\
         read -e -n </dev/null; echo \"rc-en=$?\"\n\
         printf 'zz\\n' | read -n; echo \"rc-pipe-n=$?\"\n\
         printf 'zz\\n' | read x; echo \"control x=[$x] rc=$?\"\n",
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
    assert_eq!(
        stdout,
        "rc-n=2\nrc-N=2\nrc-d=2\nrc-p=2\nrc-t=2\nrc-u=2\nrc-a=2\nrc-i=2\nrc-en=2\nrc-pipe-n=2\ncontrol x=[] rc=0\n"
    );
    assert_eq!(
        stderr.matches("option requires an argument").count(),
        10,
        "stderr: {stderr}"
    );
    for opt in ["-n", "-N", "-d", "-p", "-t", "-u", "-a", "-i"] {
        assert!(
            stderr.contains(&format!("read: {opt}: option requires an argument")),
            "missing diagnostic for {opt}, stderr: {stderr}"
        );
    }
    assert!(
        stderr.contains("read: usage: read ["),
        "usage line expected, stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
