//! rubash#1390 — `break N`/`continue N` with N deeper than the loop
//! nesting ABORTED when the unwind crossed a C-style `for ((...))`
//! frame; GNU clamps to the available depth and returns 0. A bare
//! `break` outside a loop and `break 0` keep their GNU diagnostics.
//!
//! Probe: clusters/z4/probes/final/1390.sh (+ gnu-1390.out/.err).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn break_continue_beyond_depth_clamps() {
    let (stdout, stderr, code) = rubash(
        "for ((i=0;i<5;i++)); do break 2; done; echo \"rc=$?\"\n\
         for ((i=0;i<5;i++)); do continue 2; done; echo \"rc1b=$?\"\n\
         for i in 1 2; do for j in a b; do break 2; done; done; echo \"rc2=$?\"\n\
         for ((i=0;i<3;i++)); do for ((j=0;j<3;j++)); do break 3; done; done; echo \"rc3=$?\"\n\
         for ((i=0;i<3;i++)); do for ((j=0;j<3;j++)); do break 2; done; echo \"inner_done=$?\"; done; echo \"rc4=$?\"\n\
         for i in 1; do break 2; done; echo \"rc5=$?\"\n\
         while true; do break 2; done; echo \"rc6=$?\"\n\
         for ((i=0;i<3;i++)); do break; done; echo \"rc7=$?\"\n\
         break; echo \"top_break_rc=$?\"\n\
         for ((i=0;i<3;i++)); do break 0; done; echo \"rc8=$?\"\n\
         echo end",
    );
    assert_eq!(
        stdout,
        "rc=0\nrc1b=0\nrc2=0\nrc3=0\nrc4=0\nrc5=0\nrc6=0\nrc7=0\ntop_break_rc=0\nrc8=1\nend\n"
    );
    assert!(
        stderr.contains("break: only meaningful in a `for', `while', or `until' loop"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("break: 0: loop count out of range"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
