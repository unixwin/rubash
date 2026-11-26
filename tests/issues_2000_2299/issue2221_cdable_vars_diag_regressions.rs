//! rubash#2221 — when a cdable_vars substitution also failed, cd's
//! diagnostic named the substituted VALUE; GNU bash names the OPERAND
//! (builtins/cd.def, cd_builtin error tail: builtin_error prints
//! printable_filename(dirname) — the operand as given; only the errno
//! comes from the last, substituted, attempt).
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! probes in the cluster workdir: probe-gnu-2221.sh A–F rows).

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
fn issue2221_cdable_failure_names_the_operand_not_the_value() {
    let (stdout, stderr, code) =
        rubash("shopt -s cdable_vars; dest=/no/such/dir-xyz; cd dest; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("cd: dest: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("cd: /no/such/dir-xyz:"),
        "stderr must not name the substituted value: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2221_cdable_failure_keeps_the_attempted_values_errno() {
    // The name is the operand's, but the error text is the last
    // attempt's: the value names a file, so GNU reports
    // "Not a directory" — not the operand's own ENOENT.
    let (_stdout, stderr, _code) =
        rubash("shopt -s cdable_vars; dest=/etc/hostname; cd dest; echo \"rc=$?\"");
    assert!(
        stderr.contains("cd: dest: Not a directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2221_control_plain_missing_operand_unchanged() {
    let (_stdout, stderr, _code) = rubash("cd nothere-xyz; echo \"rc=$?\"");
    assert!(
        stderr.contains("cd: nothere-xyz: No such file or directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2221_control_cdable_success_still_prints_the_value() {
    // On success GNU prints the substituted value (cd.def LCD_DOVARS
    // arm: printf temp) — the diagnostic fix must not touch that.
    let dir = std::env::temp_dir();
    let script = format!(
        "shopt -s cdable_vars; dest={}; cd dest; echo \"rc=$? PWD=$PWD\"",
        dir.display()
    );
    let (stdout, stderr, _code) = rubash(&script);
    assert_eq!(
        stdout,
        format!("{}\nrc=0 PWD={}\n", dir.display(), dir.display())
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue2221_control_cd_dash_failure_names_the_oldpwd_value() {
    // `cd -` is the one case where cd.def DOES reassign dirname (to
    // $OLDPWD) before attempting, so the value is the right name.
    let (_stdout, stderr, _code) = rubash("OLDPWD=/no/such/place-xyz; cd -; echo \"rc=$?\"");
    assert!(
        stderr.contains("cd: /no/such/place-xyz: No such file or directory"),
        "stderr: {stderr}"
    );
}
