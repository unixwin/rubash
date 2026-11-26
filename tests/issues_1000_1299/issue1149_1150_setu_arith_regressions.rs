//! rubash#1149 / rubash#1150 — `set -u` enforcement gaps in
//! arithmetic-flavored expansions.
//!
//! #1149: GNU subst.c — the offset/length words of `${v:off:len}` are
//! expanded under the same nounset discipline as any other word; an
//! unset `off` is an unbound-variable error that aborts the
//! non-interactive shell (rc=1 from a script file).
//! #1150: GNU expr.c — `((u++))` with `u` unset under `set -u` reports
//! the unbound variable EXACTLY ONCE (the rubash bug emitted a double
//! error) and aborts the shell.
//!
//! All expectations verified against GNU bash 5.3.20 with SCRIPT
//! FILES (the fatal-expansion exit status differs between `-c` and
//! file mode in GNU itself; the filed probes are script files).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1149-{tag}-{}", std::process::id()));
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
fn issue1149_setu_enforced_in_substring_offset() {
    let (stdout, stderr, code) = rubash_file(
        "1149",
        "set -u; unset off; v=hello; echo ${v:$off:2}; echo rc=$?\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("off: unbound variable"), "stderr: {stderr}");
    assert_eq!(
        stderr.matches("unbound variable").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1150_setu_postincrement_reports_once() {
    let (stdout, stderr, code) = rubash_file("1150", "set -u; unset u; ((u++)); echo rc=$?\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("u: unbound variable"), "stderr: {stderr}");
    assert_eq!(
        stderr.matches("unbound variable").count(),
        1,
        "the diagnostic must not be doubled; stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}
