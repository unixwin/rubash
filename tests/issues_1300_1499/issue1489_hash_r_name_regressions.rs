//! rubash#1489 — `hash -r <name>` silently deleted that one entry;
//! GNU's -r clears the whole table and then REHASHES the named
//! command (so it reappears with 0 hits, rc=0); an unknown name is
//! diagnosed "hash: <name>: not found", rc=1.
//!
//! Probe: clusters/x4/repro/1489.sh (+ gnu-1489-1491.txt).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1489-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn hash_r_with_name_rehashes_it_with_zero_hits() {
    let dir = tempdir("hashr");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "mkdir -p bin\n\
         printf '#!/bin/sh\\necho myecho-ran\\n' > bin/myecho\n\
         chmod +x bin/myecho\n\
         export PATH=\"$PWD/bin:$PATH\"\n\
         myecho\n\
         echo \"-- table before:\"\n\
         hash\n\
         hash -r myecho; echo \"rc1=$?\"\n\
         echo \"-- table after -r myecho:\"\n\
         hash\n\
         hash -r nosuchname; echo \"rc2=$?\"\n",
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
    let exe = format!("{}/bin/myecho", dir.display());
    let expected = format!(
        "myecho-ran\n-- table before:\nhits\tcommand\n   1\t{exe}\nrc1=0\n-- table after -r myecho:\nhits\tcommand\n   0\t{exe}\nrc2=1\n"
    );
    assert_eq!(stdout, expected);
    assert!(
        stderr.contains("hash: nosuchname: not found"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
