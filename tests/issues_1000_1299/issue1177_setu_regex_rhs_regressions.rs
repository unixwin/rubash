//! rubash#1177 — set -u skipped for =~ RHS expansion.
//!
//! GNU subst.c / execute_cmd.c (cond): nounset applies to the words
//! of a `[[ ]]` command like anywhere else — expanding an unset
//! variable as the =~ right-hand side (quoted or not) is an
//! unbound-variable error that aborts the non-interactive shell.
//!
//! Expectations verified against GNU bash 5.3.20 with SCRIPT FILES
//! (the fatal-expansion exit status is 1 in file mode). GNU dies on
//! the FIRST form of the multi-form probe, which is itself the pin:
//! the error must exist, cite line 2, and nothing may print.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1177-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn setu_applies_to_regex_rhs() {
    let (stdout, stderr, code) = rubash_file("main", "set -u; [[ x =~ $u ]]; echo rc=$?\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("u: unbound variable"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn setu_regex_multiform_probe_dies_on_first_form() {
    let (stdout, stderr, code) = rubash_file(
        "multi",
        "set -u\n\
         [[ x =~ \"$u\" ]]; echo \"quoted rc=$?\"\n\
         [[ $u =~ x ]]; echo \"lhs rc=$?\"\n\
         [[ x = $u ]]; echo \"eq rc=$?\"\n",
    );
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("line 2: u: unbound variable"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn setu_plain_expansion_still_errors() {
    let (stdout, stderr, code) = rubash_file("plain", "set -u\necho \"plain: $u\"; echo rc=$?\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("u: unbound variable"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}
