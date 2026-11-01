//! rubash#2384 — PIPESTATUS arithmetic read ownership diverged from
//! GNU: a store inside one evaluation was lost to later reads in the
//! same evaluation (`((PIPESTATUS[0] = 3, PIPESTATUS[0] == 3))` was
//! false), and to reads in the next evaluation of the same word
//! (`$((PIPESTATUS[0]=3)) $((PIPESTATUS[0]))` printed `3 1`).
//!
//! GNU has ONE store: the PIPESTATUS array object that
//! variables.c:6291 set_pipestatus_array writes and every read —
//! ${PIPESTATUS[i]} expansion and expr.c evalexp alike — consults.
//! rubash splits it: pipeline completion writes the live cell in
//! shell_state, arithmetic reads consult a snapshot of that cell
//! taken at evaluation start (rubash#1364 R3), and arithmetic writes
//! land in the env mirror, whose boundary sync updated only the
//! typed variables map. The fix shares the ownership rule with
//! ${PIPESTATUS[i]} expansion: once an evaluation records a
//! PIPESTATUS write, its reads are answered from the mirror (the
//! write's home), and the boundary sync applies PIPESTATUS writes to
//! the live cell as well, so the next snapshot and ${PIPESTATUS[i]}
//! see them.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the exact script below (cluster probes/pins/pin2384.sh). On master,
//! s1/s2/s4/s5/s7/t1/t2/g2/g4 diverge; the plain read (s3) and the
//! unrelated-array-write control (s10) agree in both shells.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let root = std::env::temp_dir().join(format!("issue2384_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let script_path: PathBuf = root.join("pin2384.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2384.sh")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

const SCRIPT: &str = "\
false | true
((PIPESTATUS[0] = 3, PIPESTATUS[0] == 3)); echo \"s1 rc=$?\"
false | true
((PIPESTATUS[0] = 3, x = PIPESTATUS)); echo \"s2 x=$x\"
false | true
((x = PIPESTATUS[0])); echo \"s3 x=$x\"
false | true
((PIPESTATUS[1] = 7, y = PIPESTATUS[1])); echo \"s4 y=$y\"
false | true
((PIPESTATUS[2] = 9, z = PIPESTATUS[2])); echo \"s5 z=$z\"
false | true
((PIPESTATUS[0]++, q = PIPESTATUS[0])); echo \"s7 q=$q\"
false | true
echo \"t1: $((PIPESTATUS[0]=3)) $((PIPESTATUS[0]))\"
false | true
echo \"t2: $((PIPESTATUS[0]=3, PIPESTATUS[0])) ${PIPESTATUS[0]}\"
false | true
((PIPESTATUS=3, r0=PIPESTATUS[0], r1=PIPESTATUS[1])); echo \"g2 r0=$r0 r1=$r1\"
false | true
((PIPESTATUS[0]+=10, s=PIPESTATUS[0])); echo \"g4 s=$s\"
a=(1)
false | true
((a[0]=5, r=PIPESTATUS[0])); echo \"s10 r=$r\"
";

#[test]
fn issue2384_pipestatus_arith_reads_share_one_store() {
    let (stdout, stderr, code) = run_script(SCRIPT);
    assert_eq!(
        stdout,
        "s1 rc=0\n\
         s2 x=3\n\
         s3 x=1\n\
         s4 y=7\n\
         s5 z=9\n\
         s7 q=2\n\
         t1: 3 3\n\
         t2: 3 3\n\
         g2 r0=3 r1=0\n\
         g4 s=11\n\
         s10 r=1\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
