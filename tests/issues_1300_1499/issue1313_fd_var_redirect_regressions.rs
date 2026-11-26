//! rubash#1313 — `{var}` redirection did not allocate/assign the fd
//! variable, and `exec {var}>&-` on an unassigned var was not the GNU
//! "ambiguous redirect" error.
//!
//! GNU: `{var}` forms allocate the lowest free fd >= 10 into var for
//! the command (or permanently for `exec`); a bare `{var}` redirect
//! with no command leaves var unset; closing an unset var via
//! `exec {var}>&-` is "var: ambiguous redirect" with rc=1.
//! Probes: clusters/x6/probes/p1313.sh, k6-scratch/repro/1313.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1313-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &PathBuf, text: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn fd_var_forms_allocate_and_close() {
    let dir = tempdir("forms");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{v}<>fa; echo \"bare v=[$v]\"\n\
         exec {w}<>fb; echo \"exec w=[$w]\"\n\
         {u}<>fc echo hi; echo \"cmd u=[$u]\"\n\
         echo data >&$w; echo \"write rc=$?\"\n\
         cat fb\n\
         exec {w}>&-; echo \"close rc=$?\"\n\
         {v2}<>fa; echo \"bare2 v2=[$v2]\"\n\
         exec {v2}>&-; echo \"close2 rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "bare v=[]\nexec w=[10]\nhi\ncmd u=[11]\nwrite rc=0\ndata\nclose rc=0\nbare2 v2=[]\nclose2 rc=1\n"
    );
    assert!(
        stderr.contains("v2: ambiguous redirect"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fd_var_read_write_roundtrip() {
    let dir = tempdir("rw");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec {v}<>f1\n\
         echo \"B v=[$v]\"\n\
         echo hello >&$v\n\
         echo \"write rc=$?\"\n\
         cat f1\n\
         exec {v}>&-\n\
         echo \"close rc=$?\"\n\
         {w}<>f1 echo hi\n\
         echo \"C w=[$w]\"\n\
         : {u}<f1\n\
         echo \"D u=[$u]\"\n\
         read line <&$u\n\
         echo \"line=[$line]\"\n",
    );
    assert_eq!(
        stdout,
        "B v=[10]\nwrite rc=0\nhello\nclose rc=0\nhi\nC w=[10]\nD u=[11]\nline=[hello]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
