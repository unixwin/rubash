//! rubash#1360 — `hash -p` arity edges: with a pathname but NO name
//! rubash dropped the usage error; with multiple names it stored only
//! the first. GNU: no name is a usage error (rc=2, usage line on
//! stderr); every name is hashed to the pathname.
//!
//! Probes: clusters/z1/z1-evidence/probes/p1360a.sh, p1360b.sh.
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
fn hash_p_without_name_is_usage_error() {
    let (stdout, stderr, code) = rubash("hash -p /some/path; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=2\n");
    assert!(
        stderr.contains("hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn hash_p_stores_every_name() {
    let (stdout, stderr, code) = rubash(
        "hash -p /some/path_xyz n1_xyz n2_xyz; echo \"rc=$?\"\n\
         hash -t n1_xyz; echo \"t1-rc=$?\"\n\
         hash -t n2_xyz; echo \"t2-rc=$?\"",
    );
    assert_eq!(
        stdout,
        "rc=0\n/some/path_xyz\nt1-rc=0\n/some/path_xyz\nt2-rc=0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
