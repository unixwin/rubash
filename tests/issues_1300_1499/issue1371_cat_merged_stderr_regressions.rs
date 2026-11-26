//! rubash#1371 — in-process `cat` with `>f 2>&1`: the stderr
//! diagnostic was silently LOST (f stayed empty); neighboring forms
//! (`2>e`, `command cat`, `1>f 2>&1`) agreed with GNU.
//!
//! GNU: with stdout and stderr merged into f, the diagnostic lands in
//! f. Probe: clusters/z4/probes/final/1371.sh (+ gnu-1371.out).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1371-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn cat_diagnostic_survives_merged_redirect() {
    let dir = tempdir("merged");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "rm -f f e; cat /nonexistent_xyz >f 2>&1; echo \"rc=$?\"; echo \"f:[$(cat f 2>&1)]\"\n\
         cat /nonexistent_xyz >f 2>e; echo \"rc2=$?\"; echo \"f2:[$(cat f)]\"; echo \"e:[$(cat e)]\"\n\
         echo data > src; cat src >f 2>&1; echo \"ok_rc=$? f3:[$(cat f)]\"\n\
         command cat /nonexistent_xyz >f 2>&1; echo \"rc3=$?\"; echo \"f4:[$(cat f 2>&1)]\"\n\
         cat /nonexistent_xyz 1>f 2>&1; echo \"rc4=$?\"; echo \"f5:[$(cat f 2>&1)]\"\n\
         rm -f f e src\n",
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
        "rc=1\nf:[cat: /nonexistent_xyz: No such file or directory]\n\
         rc2=1\nf2:[]\ne:[cat: /nonexistent_xyz: No such file or directory]\n\
         ok_rc=0 f3:[data]\n\
         rc3=1\nf4:[cat: /nonexistent_xyz: No such file or directory]\n\
         rc4=1\nf5:[cat: /nonexistent_xyz: No such file or directory]\n"
    );
    assert_eq!(stderr, "");
    let _ = std::fs::remove_dir_all(&dir);
}
